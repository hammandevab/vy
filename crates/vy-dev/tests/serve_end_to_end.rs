//! `vy-dev serve` end to end on loopback: a scaffolded project in a
//! temporary directory is built at start, served signed on origin A, its
//! state and a build trigger answered on origin B, rebuilt when a source
//! changes, and a broken build is reported without taking the daemon or
//! the last good page down. The two origins serve different things: the
//! page is not on B and the dev API is not on A.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;
use vy_dev::dev::BuildStatus;
use vy_dev::serve::{start, ServeConfig};
use vy_manifest::{Package, MEDIA_PACKAGE};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The target directory scaffolded test projects share.
fn target_dir() -> PathBuf {
    root().join("target").join("vy-dev-tests")
}

/// The first build may compile `vy-sdk` for wasm32 from scratch.
const BUILD_TIMEOUT: Duration = Duration::from_secs(600);

struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}

/// This run's dev API token, set once the daemon is up.
static TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// [`http_raw`] with the token added to a `/_dev/` path, as Studio sends it.
fn http(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Response {
    let path = match (path.starts_with("/_dev/"), TOKEN.get()) {
        (true, Some(t)) => format!(
            "{path}{}token={t}",
            if path.contains('?') { '&' } else { '?' }
        ),
        _ => path.to_string(),
    };
    http_raw(port, method, &path, headers, body)
}

/// One HTTP/1.1 request with `Connection: close`; `Host` is
/// `localhost:<port>` unless `headers` names one.
fn http_raw(
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Response {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\n");
    if !headers.iter().any(|(n, _)| n.eq_ignore_ascii_case("host")) {
        req.push_str(&format!("Host: localhost:{port}\r\n"));
    }
    for (n, v) in headers {
        req.push_str(&format!("{n}: {v}\r\n"));
    }
    req.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    s.write_all(req.as_bytes()).unwrap();
    s.write_all(body).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a header block");
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_string(), v.trim().to_string()))
        .collect();
    Response {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

const VY_ACCEPT: (&str, &str) = ("Accept", MEDIA_PACKAGE);

/// `n` as unsigned LEB128, as wasm writes sizes.
fn leb128(mut n: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return out;
        }
        out.push(byte | 0x80);
    }
}

/// A Studio package as `tools/build_studio.sh` makes one, with the
/// smallest module that has Studio's config block: one memory and a custom
/// section holding the block with the placeholder, signed with a throwaway
/// key. `vy-dev serve` fits it to the run like the real one.
fn studio_package(path: &Path) {
    let mut block = vy_dev::serve::STUDIO_CONFIG_MARKER.to_vec();
    block.extend_from_slice(b"http://localhost:8081 00000000000000000000000000000000");
    block.resize(vy_dev::serve::STUDIO_CONFIG_LEN, b' ');
    let name = b"vy-dev-test";
    let mut custom = leb128(name.len());
    custom.extend_from_slice(name);
    custom.extend_from_slice(&block);
    let mut module = b"\0asm\x01\0\0\0".to_vec();
    // Memory section: one memory of one page.
    module.extend_from_slice(&[5, 3, 1, 0, 1]);
    module.push(0);
    module.extend_from_slice(&leb128(custom.len()));
    module.extend_from_slice(&custom);
    let draft = vy_manifest::Draft::from_json(
        &std::fs::read_to_string(root().join("conformance/manifests/studio.json")).unwrap(),
    )
    .unwrap();
    let throwaway = vy_manifest::SigningKey::from_seed(&[5; 32]);
    let package = vy_publish::build(&draft, module, vec![], &throwaway).unwrap();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, package.pack().unwrap()).unwrap();
}

fn holds(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|w| w == needle.as_bytes())
}

#[test]
fn serve_builds_and_serves_two_origins() {
    let tmp = std::env::temp_dir().join(format!("vy-dev-serve-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let project = tmp.join("e2e-sida");
    vy_dev::scaffold::Scaffold {
        dir: project.clone(),
        name: "e2e-sida".into(),
        sdk: Some(root()),
    }
    .create()
    .unwrap();
    let key_path = tmp.join("keys").join("dev.key");
    let studio_vyp = tmp.join("studio.vyp");
    studio_package(&studio_vyp);

    let handle = start(ServeConfig {
        port: 0,
        studio_port: 0,
        key_path: Some(key_path.clone()),
        target_dir: Some(target_dir()),
        studio_package: Some(studio_vyp),
        ..ServeConfig::new(&project)
    })
    .unwrap();
    assert!(handle.key_created, "the key is made on first run");
    let (a, b) = (handle.port, handle.studio_port);
    assert_ne!(a, b);
    TOKEN.set(handle.dev.access().token.clone()).unwrap();
    assert_eq!(handle.address(), format!("vy://localhost:{a}/e2e-sida"));
    let dev = &handle.dev;
    let settled = |builds: u32| {
        assert!(
            dev.wait_until(BUILD_TIMEOUT, |s| s.builds >= builds
                && s.build != BuildStatus::Building
                && !s.build_requested),
            "build {builds} did not finish"
        );
    };

    // The first build runs at start.
    settled(1);
    let state = http(b, "GET", "/_dev/state", &[], b"").json();
    assert_eq!(state["build"]["status"], "ok", "{state}");
    assert_eq!(state["name"], "e2e-sida");
    assert_eq!(state["address"], handle.address());
    assert!(state["build"]["log"]
        .as_str()
        .unwrap()
        .contains("ABI 1.0: OK"));
    assert_eq!(state["agent"]["running"], false);

    // Origin A: the signed package, never cached past a rebuild.
    let page = http(a, "GET", "/e2e-sida", &[VY_ACCEPT], b"");
    assert_eq!(page.status, 200);
    assert_eq!(page.header("Content-Type"), Some(MEDIA_PACKAGE));
    assert_eq!(page.header("Cache-Control"), Some("no-cache"));
    let first = Package::unpack(&page.body).unwrap();
    let key = vy_publish::read_key(&key_path).unwrap();
    assert!(first.manifest.verify(&key.public()).is_some());
    assert!(first.validate().is_empty());
    // Both origins publish the key; only A has the page, only B the API.
    let key_a = http(a, "GET", "/.well-known/vy-key", &[], b"");
    let key_b = http(b, "GET", "/.well-known/vy-key", &[], b"");
    assert_eq!(key_a.status, 200);
    assert_eq!(key_a.body, key_b.body);
    assert_eq!(
        vy_manifest::parse_key_file(std::str::from_utf8(&key_a.body).unwrap()).unwrap(),
        vec![key.public()]
    );
    assert_eq!(http(b, "GET", "/e2e-sida", &[VY_ACCEPT], b"").status, 404);
    assert_eq!(http(a, "GET", "/_dev/state", &[], b"").status, 404);
    assert_eq!(http(a, "POST", "/_dev/build", &[], b"").status, 405);

    // The guard: a browser's request, one addressed to another name or
    // port, one without this run's token.
    let other_port = format!("localhost:{a}");
    let localhost = format!("localhost:{b}");
    for headers in [
        &[("Origin", "https://evil.example")][..],
        &[("Sec-Fetch-Site", "cross-site")][..],
        &[("Referer", "https://evil.example/")][..],
        &[("Host", "evil.example")][..],
        &[("Host", other_port.as_str())][..],
        &[
            ("Host", "evil.example"),
            ("X-Forwarded-Host", localhost.as_str()),
        ][..],
    ] {
        let r = http(b, "POST", "/_dev/build", headers, b"");
        assert_eq!(r.status, 403, "{headers:?}");
        assert_eq!(r.header("Access-Control-Allow-Origin"), None);
    }
    for path in [
        "/_dev/state",
        "/_dev/state?token=",
        "/_dev/state?token=0000",
    ] {
        let r = http_raw(b, "GET", path, &[], b"");
        assert_eq!(r.status, 403, "{path}");
        assert!(r.json()["error"].as_str().unwrap().contains("token"));
    }
    let with_127 = format!("127.0.0.1:{b}");
    assert_eq!(
        http(b, "GET", "/_dev/state", &[("Host", with_127.as_str())], b"").status,
        200
    );
    assert_eq!(http(b, "GET", "/_dev/build", &[], b"").status, 405);
    assert_eq!(http(b, "GET", "/_dev/nothing", &[], b"").status, 404);

    // The Studio package and its module carry this run's token (review
    // 0005, finding 1): under Studio's own name they are served, under a
    // rebound name or to a browser nothing on the origin is, the API with
    // the token neither.
    let token = TOKEN.get().unwrap().as_str();
    let index = http(b, "GET", "/index", &[VY_ACCEPT], b"");
    assert_eq!(index.status, 200);
    let studio = Package::unpack(&index.body).unwrap();
    let blob = format!("/_vy/{}", studio.manifest.module.hash.hex());
    let module = http(b, "GET", &blob, &[], b"");
    assert_eq!(module.status, 200);
    assert!(holds(&module.body, token), "the module carries the token");
    assert_eq!(http(b, "GET", "/", &[VY_ACCEPT], b"").status, 200);
    let rebound = format!("rebind.example:{b}");
    let api_with_token = format!("/_dev/state?token={token}");
    for path in [
        "/",
        "/index",
        blob.as_str(),
        "/.well-known/vy-key",
        api_with_token.as_str(),
    ] {
        for headers in [
            &[("Host", rebound.as_str()), VY_ACCEPT][..],
            &[("Host", "rebind.example"), VY_ACCEPT][..],
            &[("Host", other_port.as_str()), VY_ACCEPT][..],
            &[("Origin", "https://rebind.example"), VY_ACCEPT][..],
            &[("Sec-Fetch-Mode", "no-cors"), VY_ACCEPT][..],
            &[("Referer", "https://rebind.example/"), VY_ACCEPT][..],
        ] {
            let r = http_raw(b, "GET", path, headers, b"");
            assert_eq!(r.status, 403, "{path} {headers:?}");
            assert!(!holds(&r.body, token), "{path} {headers:?}");
            assert_eq!(r.header("Access-Control-Allow-Origin"), None);
        }
        let own = http_raw(
            b,
            "GET",
            path,
            &[("Host", with_127.as_str()), VY_ACCEPT],
            b"",
        );
        assert_eq!(own.status, 200, "{path} under 127.0.0.1");
    }
    // The page origin answers only under its own name too; it holds no
    // token, so a browser (the HTML fallback) is let in there.
    let page_rebound = format!("rebind.example:{a}");
    assert_eq!(
        http(
            a,
            "GET",
            "/e2e-sida",
            &[("Host", page_rebound.as_str()), VY_ACCEPT],
            b""
        )
        .status,
        403
    );
    assert_eq!(
        http(
            a,
            "GET",
            "/.well-known/vy-key",
            &[("Host", localhost.as_str())],
            b""
        )
        .status,
        403,
        "the Studio port's name is not the page origin's"
    );
    let page_127 = format!("127.0.0.1:{a}");
    assert_eq!(
        http(
            a,
            "GET",
            "/e2e-sida",
            &[("Host", page_127.as_str()), VY_ACCEPT],
            b""
        )
        .status,
        200
    );
    assert_eq!(
        http(
            a,
            "GET",
            "/e2e-sida",
            &[("Sec-Fetch-Mode", "navigate")],
            b""
        )
        .status,
        200
    );

    // A build trigger: the same sources build to the same package.
    let r = http(b, "POST", "/_dev/build", &[], b"");
    assert_eq!(r.status, 202);
    assert_eq!(r.header("Cache-Control"), Some("no-store"));
    settled(2);
    let again = Package::unpack(&http(a, "GET", "/e2e-sida", &[VY_ACCEPT], b"").body).unwrap();
    assert_eq!(again.identity(), first.identity(), "deterministic build");

    // A broken source: the watcher rebuilds, the error is in the state, the
    // last good page is still served and the daemon still answers.
    let lib = project.join("src/lib.rs");
    let good = std::fs::read_to_string(&lib).unwrap();
    std::fs::write(&lib, good.replace("fn press(", "fn press(,")).unwrap();
    settled(3);
    let state = http(b, "GET", "/_dev/state", &[], b"").json();
    assert_eq!(state["build"]["status"], "error", "{state}");
    let log = state["build"]["log"].as_str().unwrap();
    assert!(log.contains("error"), "{log}");
    assert!(log.contains("cargo build failed"), "{log}");
    let still = http(a, "GET", "/e2e-sida", &[VY_ACCEPT], b"");
    assert_eq!(still.status, 200);
    assert_eq!(Package::unpack(&still.body).unwrap(), first);

    // Fixed with a new title: rebuilt, a new module on A.
    std::fs::write(&lib, good.replace("Hej från Vy", "Hej igen")).unwrap();
    settled(4);
    assert_eq!(dev.with(|s| s.build), BuildStatus::Ok);
    let fixed = Package::unpack(&http(a, "GET", "/e2e-sida", &[VY_ACCEPT], b"").body).unwrap();
    assert_ne!(fixed.manifest.module.hash, first.manifest.module.hash);
    assert!(fixed.manifest.verify(&key.public()).is_some());
    handle.stop();

    // `vy-dev build` is the same pipeline from the command line.
    let out = tmp.join("cli.vyp");
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_vy-dev"))
        .args(["build"])
        .arg(&project)
        .arg("--key")
        .arg(&key_path)
        .arg("--out")
        .arg(&out)
        .env("CARGO_TARGET_DIR", target_dir())
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let cli = Package::unpack(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(cli.identity(), fixed.identity());
    let _ = std::fs::remove_dir_all(&tmp);
}
