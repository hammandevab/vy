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

/// One HTTP/1.1 request with `Connection: close`; `Host` is
/// `localhost:<port>` unless `headers` names one.
fn http(port: u16, method: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Response {
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

    let handle = start(ServeConfig {
        port: 0,
        studio_port: 0,
        key_path: Some(key_path.clone()),
        target_dir: Some(target_dir()),
        ..ServeConfig::new(&project)
    })
    .unwrap();
    assert!(handle.key_created, "the key is made on first run");
    let (a, b) = (handle.port, handle.studio_port);
    assert_ne!(a, b);
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

    // The guard: a browser's request, or one addressed to another name.
    for headers in [
        &[("Origin", "https://evil.example")][..],
        &[("Sec-Fetch-Site", "cross-site")][..],
        &[("Referer", "https://evil.example/")][..],
        &[("Host", "evil.example")][..],
        &[("Host", "evil.example"), ("X-Forwarded-Host", "localhost")][..],
    ] {
        let r = http(b, "POST", "/_dev/build", headers, b"");
        assert_eq!(r.status, 403, "{headers:?}");
        assert_eq!(r.header("Access-Control-Allow-Origin"), None);
    }
    assert_eq!(http(b, "GET", "/_dev/build", &[], b"").status, 405);
    assert_eq!(http(b, "GET", "/_dev/nothing", &[], b"").status, 404);

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
