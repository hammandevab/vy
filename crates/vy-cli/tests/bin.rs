//! The built binary: `abi-dump` prints the frozen listing (SPEC-5.3.5),
//! `conformance --stats` what a case's frames cost (SPEC-12.2.4), and
//! `--tab` serves as a tab process. This integration test is also
//! what makes `cargo test` build the binary the unit tests spawn for the
//! shell cases of the conformance suite.

use std::io::Write;
use std::process::{Command, Stdio};

/// SPEC-5.3.5: the binary's `abi-dump` is `docs/ABI-1.0.md`.
#[test]
fn abi_dump_matches_the_frozen_listing() {
    let out = Command::new(env!("CARGO_BIN_EXE_vy-cli"))
        .arg("abi-dump")
        .output()
        .unwrap();
    assert!(out.status.success());
    let expected = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/ABI-1.0.md"
    ))
    .unwrap();
    assert_eq!(String::from_utf8(out.stdout).unwrap(), expected);
}

/// SPEC-9.1.1: `vy-cli --tab` is a tab process: it answers a tick on an
/// unloaded page with `Idle` and leaves when its input ends.
#[test]
fn tab_mode_serves_the_protocol() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_vy-cli"))
        .arg("--tab")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let tick = vy_shell::ipc::ToTab::Tick { clock_ms: 0 }.encode();
    {
        let stdin = child.stdin.as_mut().unwrap();
        vy_shell::ipc::write_frame(stdin, &tick).unwrap();
        stdin.flush().unwrap();
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    let mut cursor = std::io::Cursor::new(out.stdout);
    let body = vy_shell::ipc::read_frame(&mut cursor).unwrap().unwrap();
    assert_eq!(
        vy_shell::ipc::ToShell::decode(&body).unwrap(),
        vy_shell::ipc::ToShell::Idle
    );
    assert_eq!(vy_shell::ipc::read_frame(&mut cursor).unwrap(), None);
}

/// SPEC-12.2.4: `vy-cli conformance --stats <prefix>` runs the cases whose
/// names start with the prefix and prints, under each, what its frames
/// cost, and last the costliest frame of the run against the budget.
#[test]
fn conformance_stats_prints_the_costliest_frame() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let out = Command::new(env!("CARGO_BIN_EXE_vy-cli"))
        .args(["conformance", "--stats", "--root", root, "hello-800x600@"])
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(out.status.success(), "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines.len(),
        5,
        "two cases, their stats, the summary: {text}"
    );
    assert!(lines[0].starts_with("ok    hello-800x600@1 "), "{text}");
    assert!(
        lines[1].starts_with("      frames   1  costliest ") && lines[1].contains("(frame 1, "),
        "{text}"
    );
    assert!(lines[2].starts_with("ok    hello-800x600@2 "), "{text}");
    assert!(
        lines[4].starts_with("costliest frame: hello-800x600@")
            && lines[4].contains(" fuel of 8000000 ("),
        "{text}"
    );
    // Without the flag the output is as it was: one line a case.
    let out = Command::new(env!("CARGO_BIN_EXE_vy-cli"))
        .args(["conformance", "--root", root, "hello-800x600@1"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8(out.stdout).unwrap().lines().count(), 1);
}
