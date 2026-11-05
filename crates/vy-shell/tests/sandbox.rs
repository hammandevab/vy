//! SPEC-9.1.3: the tab process runs without filesystem and network access
//! after startup, and a probe shows it.

/// SPEC-9.1.3, SPEC-9.1.1, SPEC-11.2.2: `vy-shell --tab --sandbox-probe`.
#[test]
#[ignore = "M4: the tab process has no OS sandbox yet"]
fn spec_9_1_3_tab_sandbox() {
    todo!()
}
