//! SPEC-9.1.3: the tab process, started the way the shell starts it, runs
//! without filesystem and network access after startup, and a probe shows
//! it. Each platform asserts what its sandbox guarantees and says so when
//! the facility is missing on the machine running the test.

use std::path::Path;

fn refused(text: &str, what: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with(&format!("{what}: refused")))
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
