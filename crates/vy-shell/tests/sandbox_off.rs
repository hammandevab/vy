//! SPEC-9.1.3 (granskning 0007:5), end to end: a tab process whose
//! lockdown did not apply says so before its page runs, and the shell
//! shows it in the chrome and the tab's log. `VY_SANDBOX` is read by every
//! tab process this binary starts, so the test has a binary of its own.

use std::path::Path;
use std::time::Duration;
use vy_shell::sandbox::{MARKER_OFF, MARKER_PARTIAL};

/// SPEC-9.1.3, SPEC-9.6.1: with `VY_SANDBOX=off` a real tab process,
/// started by the headless shell as a window starts one, reports no
/// layer; the chrome shows "sandlåda: av" (on Windows "sandlåda: delvis":
/// the shell's job object still stands) and the log has the detail.
#[test]
fn spec_9_1_3_sandbox_off_is_shown() {
    std::env::set_var("VY_SANDBOX", "off");
    let exe = Path::new(env!("CARGO_BIN_EXE_vy-shell")).to_path_buf();
    let mut shell = vy_shell::headless::headless_shell(Some(exe), 800.0, 600.0, 1.0).unwrap();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/wasm/hello.wasm");
    let tab = shell.open_tab(Some(vy_shell::page::Source::File { path, draft: None }));
    vy_shell::headless::settle(&mut shell, 1, Duration::from_secs(60)).unwrap();
    assert_eq!(shell.tab_status(tab), "running");
    let report = shell.tab_sandbox(tab).expect("the tab process reported");
    assert!(
        report.warnings.iter().any(|w| w == "VY_SANDBOX=off"),
        "{report}"
    );
    let marker = if cfg!(windows) {
        MARKER_PARTIAL
    } else {
        MARKER_OFF
    };
    assert_eq!(report.marker(), Some(marker));
    let chrome: Vec<String> = vy_dl::decode(&shell.chrome_display_list())
        .unwrap()
        .iter()
        .filter_map(|op| match op {
            vy_dl::Op::Text { text, .. } => Some(String::from_utf8_lossy(text).into_owned()),
            _ => None,
        })
        .collect();
    assert!(chrome.contains(&marker.to_string()), "{chrome:?}");
    let detail = format!("[host warn] sandbox: {report}");
    assert!(
        shell.tab_logs(tab).contains(&detail),
        "{:?}",
        shell.tab_logs(tab)
    );
    shell.shutdown();
}
