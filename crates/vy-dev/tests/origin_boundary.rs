//! The boundary between the page under development and the dev API,
//! through the client's own request path (`vy_proto::Fetcher::request`, the
//! call the shell's loader makes for a page's `fetch_begin`) and its own
//! policy (`Fetched::grants`), against a running `vy-dev serve`:
//!
//! - a page from a remote origin never reaches a loopback origin over
//!   plain http, whatever its manifest lists and whatever the user allowed:
//!   the transport refuses before a connection is made;
//! - the page under development on `localhost:<port>` that lists the
//!   Studio origin gets a question, never an automatic grant; and even
//!   allowed, its requests reach the API without the run's token, which it
//!   cannot read, and are refused;
//! - Studio's own origin reaches the API with the token.

use std::path::{Path, PathBuf};
use std::time::Duration;
use vy_dev::dev::BuildStatus;
use vy_dev::serve::{start, ServeConfig};
use vy_manifest::Grant;
use vy_proto::{Address, Error, Fetcher, HttpUrl, Options};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const LIMIT: u64 = 1 << 20;

#[test]
fn pages_cannot_drive_the_dev_api() {
    let tmp = std::env::temp_dir().join(format!("vy-dev-boundary-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let project = tmp.join("grans-sida");
    vy_dev::scaffold::Scaffold {
        dir: project.clone(),
        name: "grans-sida".into(),
        sdk: Some(root()),
    }
    .create()
    .unwrap();
    let fake = std::path::absolute(root().join("tools/fake_agent.sh")).unwrap();
    let handle = start(ServeConfig {
        port: 0,
        studio_port: 0,
        key_path: Some(tmp.join("dev.key")),
        target_dir: Some(root().join("target").join("vy-dev-tests")),
        agent_cmd: Some(format!("\"{}\"", fake.display())),
        ..ServeConfig::new(&project)
    })
    .unwrap();
    let (a, b) = (handle.port, handle.studio_port);
    let dev = &handle.dev;
    let token = dev.access().token.clone();
    let settled = |builds: u32| {
        assert!(
            dev.wait_until(Duration::from_secs(600), |s| s.builds >= builds
                && s.build != BuildStatus::Building
                && !s.build_requested)
        );
    };
    settled(1);

    // The page under development asks for `net` to the Studio origin.
    let manifest = project.join("manifest.json");
    let draft = std::fs::read_to_string(&manifest).unwrap();
    let studio_origin = format!("http://localhost:{b}");
    std::fs::write(
        &manifest,
        draft.replace(
            "\"memory_max\": 32,",
            &format!("\"memory_max\": 32,\n  \"capabilities\": {{\"net\": {{\"origins\": [\"{studio_origin}\"]}}}},"),
        ),
    )
    .unwrap();
    settled(2);
    assert_eq!(
        dev.with(|s| s.build),
        BuildStatus::Ok,
        "{}",
        dev.with(|s| s.build_log.clone())
    );

    // The client fetches it like any page: signed with the developer key,
    // pinned on first use, trusted. Its `net` to another origin is a
    // question (SPEC-8.2.3), never granted without one.
    let mut fetcher = Fetcher::new(Options::new(tmp.join("cache"))).unwrap();
    let page = fetcher
        .fetch(&Address::parse(&format!("vy://localhost:{a}/grans-sida")).unwrap())
        .unwrap();
    assert!(page.trusted());
    assert!(
        page.grants().contains(&("net", Grant::Ask)),
        "{:?}",
        page.grants()
    );
    // The rule the shell's main process can apply, knowing the address.
    assert!(!vy_runtime::plain_http_allowed(
        &studio_origin,
        &format!("localhost:{a}")
    ));
    assert!(vy_runtime::plain_http_allowed(
        &studio_origin,
        &format!("localhost:{b}")
    ));

    // Allowed anyway, the page's requests carry no token and are refused.
    let api = |path: &str| format!("{studio_origin}{path}");
    let (status, body) = fetcher
        .request("POST", &api("/_dev/agent"), b"gor nagot", true, LIMIT)
        .unwrap();
    assert_eq!(status, 403, "{}", String::from_utf8_lossy(&body));
    let (status, _) = fetcher
        .request("GET", &api("/_dev/state"), b"", true, LIMIT)
        .unwrap();
    assert_eq!(status, 403);
    let (status, _) = fetcher
        .request(
            "GET",
            &api("/_dev/state?token=00000000000000000000000000000000"),
            b"",
            true,
            LIMIT,
        )
        .unwrap();
    assert_eq!(status, 403, "the placeholder token of an unpatched Studio");
    // What a page can fetch from the Studio origin does not hold the token:
    // without the Vy media types in `Accept` (a page cannot set headers) the
    // server answers HTML, and blobs need a hash the page never sees.
    for path in ["/", "/index", "/.well-known/vy-key"] {
        let (_, body) = fetcher
            .request("GET", &api(path), b"", true, LIMIT)
            .unwrap();
        assert!(!String::from_utf8_lossy(&body).contains(&token), "{path}");
    }

    // A remote page never reaches loopback over plain http, even with the
    // token and with `net` allowed: refused before any connection.
    let with_token = api(&format!("/_dev/agent?token={token}"));
    assert!(!vy_proto::transport_allowed(
        false,
        &HttpUrl::parse(&with_token).unwrap()
    ));
    match fetcher.request("POST", &with_token, b"gor nagot", false, LIMIT) {
        Err(Error::TlsRequired { .. }) => {}
        other => panic!("a remote page reached the dev API: {other:?}"),
    }
    assert!(!vy_runtime::plain_http_allowed(
        &studio_origin,
        "page.example"
    ));

    // Nothing above started the agent.
    assert!(!dev.with(|s| s.agent_running));
    assert!(dev
        .wait_events(0, Duration::ZERO, false, 1000)
        .iter()
        .all(|e| !e.text.starts_with("uppdrag")));
    assert!(!project.join("fake-agent-mission.txt").exists());

    // Studio's own origin, with the token, is let in.
    let (status, body) = fetcher
        .request(
            "GET",
            &api(&format!("/_dev/state?token={token}")),
            b"",
            true,
            LIMIT,
        )
        .unwrap();
    assert_eq!(status, 200);
    let state: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(state["name"], "grans-sida");
    handle.stop();
    let _ = std::fs::remove_dir_all(&tmp);
}
