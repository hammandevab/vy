//! SPEC-9.1.3: the tab process, started the way the shell starts it, runs
//! without filesystem and network access after startup, and a probe shows
//! it. Each platform asserts what its sandbox guarantees and says so when
//! the facility is missing on the machine running the test.

use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;
use vy_shell::ipc::{read_frame, ToShell, ToTab};
use vy_shell::process::{channel_sink, spawn_process, Inbound, TabEvent};

fn refused(text: &str, what: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with(&format!("{what}: refused")))
}

/// The next thing tab `id` reports.
fn next(inbox: &mpsc::Receiver<TabEvent>, id: u32) -> Inbound {
    let (tab, inbound) = inbox
        .recv_timeout(Duration::from_secs(30))
        .expect("the tab answers");
    assert_eq!(tab, id);
    inbound
}

/// A line of `/proc/<pid>/status` as its words, e.g. `["Seccomp:", "2"]`.
fn has_status_line(status: &str, line: [&str; 2]) -> bool {
    status.lines().any(|l| l.split_whitespace().eq(line))
}

/// SPEC-9.1.3, SPEC-9.1.1, SPEC-11.2.2: `vy-shell --tab --sandbox-probe`
/// started by `process::probe_process` (the shell's own spawn path) tries
/// to read a file, write one, open a socket and start a process after its
/// lockdown; on Linux all four are refused (seccomp, Landlock when the
/// kernel has it), on Windows writing and starting a process (low
/// integrity, job object; reading and sockets only in `VY_SANDBOX=strict`),
/// on macOS all four when `sandbox_init` takes the profile. `VY_SANDBOX=off`
/// really is off, so the probe can see a difference.
#[test]
fn spec_9_1_3_tab_sandbox() {
    let exe = Path::new(env!("CARGO_BIN_EXE_vy-shell"));
    let text = vy_shell::process::probe_process(exe).unwrap();
    eprintln!("{text}");
    assert!(text.starts_with("sandbox: "), "{text}");
    let all = ["file-read", "file-write", "socket", "process"];
    if cfg!(target_os = "linux") {
        if text.contains("seccomp allowlist") {
            for what in all {
                assert!(refused(&text, what), "{what}: {text}");
            }
        } else {
            eprintln!("skipped: seccomp is not available here");
        }
    } else if cfg!(windows) {
        if text.contains("low integrity") {
            assert!(refused(&text, "file-write"), "{text}");
            assert!(
                refused(&text, "process"),
                "the job allows one process: {text}"
            );
        } else {
            eprintln!("skipped: the token could not be lowered here");
        }
    } else if cfg!(target_os = "macos") {
        if text.contains("sandbox_init deny-default") {
            for what in all {
                assert!(refused(&text, what), "{what}: {text}");
            }
        } else {
            eprintln!("skipped: sandbox_init refused the profile here");
        }
    } else {
        eprintln!("skipped: no sandbox on this platform");
    }

    // Off is off: the same probe reaches the file system.
    let off = std::process::Command::new(exe)
        .args(["--tab", "--sandbox-probe"])
        .env("VY_SANDBOX", "off")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&off.stdout);
    assert!(!off.status.success(), "{text}");
    assert!(text.contains("file-read: allowed"), "{text}");
}

/// SPEC-9.1.3, SPEC-11.4.1 (granskning 0007:1): a tab that is recorded or
/// replayed runs in the same sandbox as every other. The tab process takes
/// no file: `--tab` with `--record` or `--replay` is refused before the
/// probe can run and makes no file, so no flag starts a tab unlocked. A
/// tab the shell records (`spawn_process` with a file) is the plain
/// locked-down `--tab`: on Linux its process is in seccomp filter mode
/// under no_new_privs while it records, and the main process writes the
/// file, exactly the frames the tab was sent up to its shutdown. Fed that
/// file (`spawn_process` with it to replay), a new tab answers the same,
/// and what the shell sends it meanwhile is not delivered.
#[test]
fn spec_9_1_3_recording_and_replay_are_sandboxed() {
    let exe = Path::new(env!("CARGO_BIN_EXE_vy-shell"));
    let dir = std::env::temp_dir().join(format!("vy-sandbox-record-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("session.vyrec");
    for flag in ["--record", "--replay"] {
        let out = Command::new(exe)
            .args(["--tab", flag])
            .arg(&file)
            .arg("--sandbox-probe")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.code(), Some(2), "{flag} is refused: {text}");
        assert!(!text.contains("allowed"), "{flag}: {text}");
        assert!(!file.exists(), "{flag} made no file");
    }

    let seccomp = vy_shell::process::probe_process(exe)
        .unwrap()
        .contains("seccomp allowlist");
    let (sink, inbox) = mpsc::channel::<TabEvent>();
    let conn = spawn_process(exe, 1, channel_sink(sink), Some(&file), None).unwrap();
    assert!(conn.send(&ToTab::Tick { clock_ms: 16 }));
    assert!(matches!(next(&inbox, 1), Inbound::Message(ToShell::Idle)));
    if cfg!(target_os = "linux") && seccomp {
        let pid = conn.pid().unwrap();
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
        assert!(has_status_line(&status, ["Seccomp:", "2"]), "{status}");
        assert!(has_status_line(&status, ["NoNewPrivs:", "1"]), "{status}");
    } else {
        eprintln!("skipped the process status: not Linux, or no seccomp here");
    }
    assert!(conn.send(&ToTab::Shutdown));
    assert!(matches!(
        next(&inbox, 1),
        Inbound::Message(ToShell::Storage(e)) if e.is_empty()
    ));
    conn.shutdown();
    let mut r = std::io::BufReader::new(std::fs::File::open(&file).unwrap());
    let mut frames = Vec::new();
    while let Some(body) = read_frame(&mut r).unwrap() {
        frames.push(ToTab::decode(&body).unwrap());
    }
    assert_eq!(frames, [ToTab::Tick { clock_ms: 16 }, ToTab::Shutdown]);

    let (sink, inbox) = mpsc::channel::<TabEvent>();
    let conn = spawn_process(exe, 2, channel_sink(sink), None, Some(&file)).unwrap();
    assert!(
        !conn.send(&ToTab::Tick { clock_ms: 99 }),
        "a replaying tab is fed its recording"
    );
    assert!(matches!(next(&inbox, 2), Inbound::Message(ToShell::Idle)));
    assert!(matches!(
        next(&inbox, 2),
        Inbound::Message(ToShell::Storage(e)) if e.is_empty()
    ));
    assert!(matches!(next(&inbox, 2), Inbound::Gone { .. }));
    conn.shutdown();
    let _ = std::fs::remove_dir_all(&dir);
}
