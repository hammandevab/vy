//! The dev API's agent pipeline end to end, with `tools/fake_agent.sh`
//! (`tools/fake_agent.ps1` on Windows) in place of Claude Code: no network,
//! no credentials. A mission posted to `/_dev/agent` reaches the agent on
//! stdin with the fixed headless flags, its `stream-json` output comes back
//! from `/_dev/agent/events` as assistant text and tool names, one agent
//! runs at a time, a stop ends it and what it started, and the agent's
//! exit triggers the build that serves its edit on the page's origin.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vy_dev::dev::BuildStatus;
use vy_dev::serve::{start, ServeConfig};
use vy_manifest::{Package, MEDIA_PACKAGE};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn target_dir() -> PathBuf {
    root().join("target").join("vy-dev-tests")
}

const BUILD_TIMEOUT: Duration = Duration::from_secs(600);

/// The fake agent as an `--agent-cmd` line.
fn fake_agent() -> String {
    let tools = std::path::absolute(root().join("tools")).unwrap();
    if cfg!(windows) {
        format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -File \"{}\"",
            tools.join("fake_agent.ps1").display()
        )
    } else {
        format!("\"{}\"", tools.join("fake_agent.sh").display())
    }
}

/// This run's dev API token, set once the daemon is up.
static TOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// One HTTP/1.1 request to `localhost:<port>`, the token added to a
/// `/_dev/` path as Studio sends it; status and body.
fn http(port: u16, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
    let path = match (path.starts_with("/_dev/"), TOKEN.get()) {
        (true, Some(t)) => format!(
            "{path}{}token={t}",
            if path.contains('?') { '&' } else { '?' }
        ),
        _ => path.to_string(),
    };
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(60))).unwrap();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost:{port}\r\nAccept: {MEDIA_PACKAGE}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).unwrap();
    s.write_all(body).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let status = String::from_utf8_lossy(&raw[..split])
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, raw[split + 4..].to_vec())
}

fn json(port: u16, method: &str, path: &str, body: &[u8]) -> (u16, serde_json::Value) {
    let (status, body) = http(port, method, path, body);
    (status, serde_json::from_slice(&body).unwrap())
}

/// Long-poll the events (without raw lines) until `done` holds for the
/// texts seen so far; returns every entry seen.
fn events_until(
    port: u16,
    after: &mut u64,
    done: impl Fn(&[serde_json::Value]) -> bool,
) -> Vec<serde_json::Value> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut seen = Vec::new();
    while !done(&seen) {
        assert!(Instant::now() < deadline, "events so far: {seen:#?}");
        let (status, v) = json(
            port,
            "GET",
            &format!("/_dev/agent/events?after={after}&wait=5000&raw=0"),
            b"",
        );
        assert_eq!(status, 200);
        *after = v["next"].as_u64().unwrap();
        seen.extend(v["entries"].as_array().unwrap().iter().cloned());
    }
    seen
}

fn has(entries: &[serde_json::Value], kind: &str, text: &str) -> bool {
    entries
        .iter()
        .any(|e| e["kind"] == kind && e["text"].as_str().unwrap_or("").contains(text))
}

#[test]
fn agent_runs_streams_stops_and_triggers_a_build() {
    let tmp = std::env::temp_dir().join(format!("vy-dev-agent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let project = tmp.join("agent-sida");
    vy_dev::scaffold::Scaffold {
        dir: project.clone(),
        name: "agent-sida".into(),
        sdk: Some(root()),
    }
    .create()
    .unwrap();
    let handle = start(ServeConfig {
        port: 0,
        studio_port: 0,
        key_path: Some(tmp.join("dev.key")),
        target_dir: Some(target_dir()),
        agent_cmd: Some(fake_agent()),
        ..ServeConfig::new(&project)
    })
    .unwrap();
    let (a, b) = (handle.port, handle.studio_port);
    let dev = &handle.dev;
    TOKEN.set(dev.access().token.clone()).unwrap();
    let settled = |builds: u32| {
        assert!(
            dev.wait_until(BUILD_TIMEOUT, |s| s.builds >= builds
                && s.build != BuildStatus::Building
                && !s.build_requested),
            "build {builds} did not finish"
        );
    };
    settled(1);
    assert_eq!(dev.with(|s| s.build), BuildStatus::Ok);
    let before = Package::unpack(&http(a, "GET", "/agent-sida", b"").1).unwrap();

    // Missions the API refuses.
    assert_eq!(json(b, "POST", "/_dev/agent", b"  \n").0, 400, "empty");
    assert_eq!(
        json(b, "POST", "/_dev/agent", &[0xff, 0xfe]).0,
        400,
        "not UTF-8"
    );
    let long = vec![b'x'; vy_dev::dev::MAX_BODY_BYTES + 1];
    assert_eq!(json(b, "POST", "/_dev/agent", &long).0, 413);
    assert_eq!(json(b, "GET", "/_dev/agent", b"").0, 405);
    assert_eq!(
        json(b, "POST", "/_dev/agent/stop", b"").1["stopped"],
        false,
        "nothing to stop"
    );

    // A slow run: one at a time, and a stop ends it and its child.
    let mut after = json(b, "GET", "/_dev/state", b"").1["seq"]
        .as_u64()
        .unwrap();
    assert_eq!(json(b, "POST", "/_dev/agent", b"SLOW: ta tid").0, 202);
    let seen = events_until(b, &mut after, |e| has(e, "tool", "Read src/lib.rs"));
    assert!(has(&seen, "agent", "uppdrag: SLOW: ta tid"), "{seen:#?}");
    assert!(has(&seen, "agent", "Claude Code startade (fake-agent)"));
    assert!(has(&seen, "assistant", "Jag läser sidan"));
    assert_eq!(
        json(b, "GET", "/_dev/state", b"").1["agent"]["running"],
        true
    );
    let (status, busy) = json(b, "POST", "/_dev/agent", b"en till");
    assert_eq!(status, 409, "{busy}");
    let pid_file = project.join("fake-agent-sleep.pid");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !pid_file.is_file() {
        assert!(Instant::now() < deadline, "the fake agent never slept");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(json(b, "POST", "/_dev/agent/stop", b"").1["stopped"], true);
    let seen = events_until(b, &mut after, |e| has(e, "agent", "agenten stoppades"));
    assert!(!has(&seen, "result", ""), "a stopped run has no result");
    let state = json(b, "GET", "/_dev/state", b"").1;
    assert_eq!(state["agent"]["running"], false);
    assert!(state["agent"]["last_exit"].is_null());
    if !cfg!(windows) {
        // Gone, or a zombie waiting for init to reap it, within the grace
        // period and the SIGKILL after it.
        let pid = std::fs::read_to_string(&pid_file).unwrap();
        let pid = pid.trim();
        let deadline = Instant::now() + vy_dev::agent::STOP_GRACE + Duration::from_secs(5);
        loop {
            let out = std::process::Command::new("ps")
                .args(["-o", "stat=", "-p", pid])
                .output()
                .unwrap();
            let stat = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if stat.is_empty() || stat.starts_with('Z') {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the agent's child {pid} is still running ({stat})"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    // The stop triggered a build too; nothing was edited.
    settled(2);
    after = json(b, "GET", "/_dev/state", b"").1["seq"]
        .as_u64()
        .unwrap();

    // A run to the end: the mission arrives on stdin with the headless
    // flags, the stream comes back as text and tool names, the exit builds.
    let mission = "gör en sida som\nhälsar från agenten";
    assert_eq!(json(b, "POST", "/_dev/agent", mission.as_bytes()).0, 202);
    let seen = events_until(b, &mut after, |e| has(e, "build", "klar:"));
    assert!(
        has(&seen, "agent", "uppdrag: gör en sida som…"),
        "{seen:#?}"
    );
    assert!(has(&seen, "tool", "Edit src/lib.rs"));
    assert!(has(&seen, "assistant", "Klart: sidan hälsar från agenten."));
    assert!(has(&seen, "result", "Sidan är ändrad."));
    assert!(has(&seen, "agent", "agenten är klar"));
    assert!(
        seen.iter().all(|e| e["kind"] != "raw"),
        "raw=0 leaves them out"
    );
    let order: Vec<&str> = seen.iter().map(|e| e["kind"].as_str().unwrap()).collect();
    let result_at = order.iter().position(|k| *k == "result").unwrap();
    let build_at = order.iter().rposition(|k| *k == "build").unwrap();
    assert!(
        result_at < build_at,
        "the build comes after the agent: {order:?}"
    );
    assert_eq!(
        std::fs::read_to_string(project.join("fake-agent-mission.txt")).unwrap(),
        mission
    );
    let args = std::fs::read_to_string(project.join("fake-agent-args.txt")).unwrap();
    let args: Vec<&str> = args.lines().map(str::trim_end).collect();
    if cfg!(windows) {
        // PowerShell's binder may treat `-p` and `--x` tokens on their way
        // to `$args` its own way; what matters reached the script.
        for a in ["stream-json", "acceptEdits", vy_dev::agent::ALLOWED_TOOLS] {
            assert!(args.contains(&a), "{a} in {args:?}");
        }
    } else {
        assert_eq!(args, vy_dev::agent::fixed_args());
    }
    let state = json(b, "GET", "/_dev/state", b"").1;
    assert_eq!(state["agent"]["last_exit"], 0);
    assert_eq!(state["build"]["status"], "ok");
    let after_run = Package::unpack(&http(a, "GET", "/agent-sida", b"").1).unwrap();
    assert_ne!(after_run.manifest.module.hash, before.manifest.module.hash);
    assert!(
        after_run
            .module
            .windows(b"Agenten var har.".len())
            .any(|w| w == b"Agenten var har."),
        "the agent's edit is in the served module"
    );

    // Raw lines are there for the devtools; a long-poll with nothing new
    // returns empty after its wait, with `next` unchanged.
    let (_, all) = json(b, "GET", "/_dev/agent/events?after=0&raw=1&wait=0", b"");
    let entries = all["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["kind"] == "raw"
                && e["text"].as_str().unwrap().contains("\"type\":\"result\""))
    );
    let started = Instant::now();
    let (_, empty) = json(
        b,
        "GET",
        &format!("/_dev/agent/events?after={after}&wait=300&raw=0"),
        b"",
    );
    assert!(started.elapsed() >= Duration::from_millis(250));
    assert_eq!(empty["entries"].as_array().unwrap().len(), 0);
    assert_eq!(empty["next"].as_u64(), Some(after));
    let (_, limited) = json(b, "GET", "/_dev/agent/events?after=0&raw=0&limit=2", b"");
    let limited = limited["entries"].as_array().unwrap();
    assert_eq!(limited.len(), 2);
    assert_eq!(limited[1]["seq"].as_u64(), Some(after), "the newest two");

    // A failing run reports its exit code.
    let mut after2 = after;
    assert_eq!(json(b, "POST", "/_dev/agent", b"FAIL nu").0, 202);
    let seen = events_until(b, &mut after2, |e| has(e, "agent", "avslutades med kod 3"));
    assert!(has(&seen, "result", "fel: Något gick fel."));
    assert_eq!(
        json(b, "GET", "/_dev/state", b"").1["agent"]["last_exit"],
        3
    );
    handle.stop();
    let _ = std::fs::remove_dir_all(&tmp);
}
