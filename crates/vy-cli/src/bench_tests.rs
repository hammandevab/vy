//! Performance budget tests (SPEC §13) and later conformance gates. Enabled milestone by milestone.
//! M1 tests live in `lib.rs` and `conformance.rs`.

/// SPEC-1.1.3, SPEC-13.1.2: Enter to first frame on cache hit <= 100 ms.
#[test]
#[ignore = "M3: bench"]
fn spec_13_1_2_first_frame_cache_hit() {
    todo!()
}

/// SPEC-13.1.3: cold fetch of a 300 KB module to first frame <= 400 ms.
#[test]
#[ignore = "M3: bench"]
fn spec_13_1_3_first_frame_cold() {
    todo!()
}

/// SPEC-13.2.2: input to pixel <= 16 ms.
#[test]
#[ignore = "M3: bench"]
fn spec_13_2_2_input_latency() {
    todo!()
}

/// SPEC-13.2.3: scrolling a text page holds 60 fps with > 95 % glyph cache hits.
#[test]
#[ignore = "M3: bench"]
fn spec_13_2_3_scroll_fps() {
    todo!()
}

/// SPEC-13.4.1: SDK refuses to publish a module over budget without --allow-slow.
#[test]
#[ignore = "M2: sdk"]
fn spec_13_4_1_sdk_refuses_slow_module() {
    todo!()
}

/// SPEC-10.4.2: a page with input fields but no `ime` capability fails conformance.
#[test]
#[ignore = "M2: conformance"]
fn spec_10_4_2_ime_required_for_input() {
    todo!()
}

/// SPEC-12.1.2: a client passing the conformance suite is a Vy client.
#[test]
#[ignore = "M2: conformance"]
fn spec_12_1_2_conformance_defines_client() {
    todo!()
}

/// SPEC-12.2.3: no performance regression over 5 %.
#[test]
#[ignore = "M3: bench"]
fn spec_12_2_3_perf_regression_gate() {
    todo!()
}

/// SPEC-12.3.1: client binary <= 25 MB; whole codebase < 400 000 tokens.
#[test]
#[ignore = "M5: release"]
fn spec_12_3_1_size_budget() {
    todo!()
}
