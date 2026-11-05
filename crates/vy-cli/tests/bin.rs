//! The built binary: `abi-dump` prints the frozen listing (SPEC-5.3.5)
//! and `--tab` serves as a tab process. This integration test is also
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
