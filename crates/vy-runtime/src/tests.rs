//! Runtime tests. Every module here is built from WAT so the tests do not
//! depend on `modules/hello` being compiled first.

use super::*;
use host::LogSource;
use vy_abi::err;

/// The exports a test module does not care about.
const STUBS: &str = r#"
    (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
    (func (export "vy_resize") (param i32 i32 f32))
    (func (export "vy_suspend") (result i32) i32.const 0)
    (func (export "vy_alloc") (param i32) (result i32) i32.const 1024)
"#;

const INIT_OK: &str =
    r#"(func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)"#;

fn wasm(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).unwrap_or_else(|e| panic!("bad test WAT: {e}\n{wat}"))
}

fn load(bytes: &[u8]) -> Runtime {
    Runtime::load(bytes, &sha256(bytes), Options::default()).unwrap()
}

fn module_logs(rt: &Runtime) -> Vec<String> {
    rt.logs()
        .iter()
        .filter(|l| l.source == LogSource::Module)
        .map(|l| l.message.clone())
        .collect()
}

fn host_logs(rt: &Runtime) -> Vec<String> {
    rt.logs()
        .iter()
        .filter(|l| l.source == LogSource::Host)
        .map(|l| l.message.clone())
        .collect()
}

/// SPEC-5.2.1..5.2.13: every import in `vy_abi::IMPORTS` is linked with its exact signature,
/// and nothing else is (SPEC-2.1.3).
#[test]
fn spec_5_2_all_imports_linked() {
    let mut linker: Linker<Host> = Linker::new(engine());
    host::link_imports(&mut linker).unwrap();
    let limits = StoreLimitsBuilder::new().build();
    let mut store = Store::new(
        engine(),
        Host::new(limits, Capabilities::default(), Vec::new()),
    );
    let mut linked = 0;
    for sig in vy_abi::IMPORTS {
        let ext = linker
            .get(&mut store, vy_abi::IMPORT_MODULE, sig.name)
            .unwrap_or_else(|e| panic!("import {} not linked: {e}", sig.name));
        let func = ext.into_func().unwrap();
        let ty = func.ty(&store);
        let params: Vec<String> = ty.params().map(|p| format!("{p}")).collect();
        let expect: Vec<String> = sig
            .params
            .iter()
            .map(|p| {
                match p {
                    vy_abi::Ty::I32 => "i32",
                    vy_abi::Ty::I64 => "i64",
                    vy_abi::Ty::F32 => "f32",
                }
                .to_string()
            })
            .collect();
        assert_eq!(params, expect, "{} params", sig.name);
        assert_eq!(
            ty.results().len(),
            usize::from(sig.result.is_some()),
            "{} result",
            sig.name
        );
        linked += 1;
    }
    assert_eq!(linked, vy_abi::IMPORTS.len());
    assert_eq!(linker.iter(&mut store).count(), vy_abi::IMPORTS.len());
}

/// SPEC-5.2.2, SPEC-6.2.1: a frame is wanted only after request_frame, an event
/// returning 1, a window change, or init itself.
#[test]
fn spec_6_2_1_frame_only_on_request() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "request_frame" (func $request_frame))
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 1)) (then (call $request_frame)))
                i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) local.get 1)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
    ));
    let mut rt = load(&bytes);
    assert!(!rt.wants_frame(), "nothing wanted before init");
    assert!(matches!(rt.frame_with_dt(0), Err(Error::NotInitialized)));
    rt.init(100, 100, 1.0).unwrap();
    assert!(rt.wants_frame(), "init is a window change");
    rt.frame_with_dt(0).unwrap();
    assert!(!rt.wants_frame(), "frame consumed the request");
    rt.frame_with_dt(1).unwrap();
    assert!(rt.wants_frame(), "request_frame during the frame");
    rt.frame_with_dt(0).unwrap();
    assert!(!rt.wants_frame());
    assert!(!rt.event(event::POINTER_MOVE, 0, 0, 0, 0).unwrap());
    assert!(!rt.wants_frame(), "event returning 0 wants nothing");
    assert!(rt.event(event::POINTER_MOVE, 1, 0, 0, 0).unwrap());
    assert!(rt.wants_frame(), "event returning 1 wants a frame");
    rt.frame_with_dt(0).unwrap();
    rt.resize(200, 100, 1.0).unwrap();
    assert!(rt.wants_frame(), "resize wants a frame");
}

/// SPEC-6.2.2: an idle page consumes 0 module CPU.
#[test]
#[ignore = "M3: shell scheduling"]
fn spec_6_2_2_idle_is_free() {
    todo!()
}

/// SPEC-5.3.1, SPEC-2.1.4: a call without its capability returns E_DENIED, is logged,
/// and the module keeps running.
#[test]
fn spec_5_3_1_denied_call_is_harmless() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "fetch_begin" (func $fetch_begin (param i32 i32 i32 i32 i32) (result i32)))
            (import "vy" "state_put" (func $state_put (param i32 i32 i32 i32) (result i32)))
            (import "vy" "clipboard_read" (func $clipboard_read (param i32 i32) (result i32)))
            (import "vy" "navigate" (func $navigate (param i32 i32 i32) (result i32)))
            (import "vy" "dl_begin" (func $dl_begin (result i32)))
            (import "vy" "dl_end" (func $dl_end (result i32)))
            (import "vy" "dl_rect" (func $dl_rect (param f32 f32 f32 f32 f32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "vy://example.test/")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (local $sum i32)
                (local.set $sum (call $fetch_begin (i32.const 0) (i32.const 0) (i32.const 18) (i32.const 0) (i32.const 0)))
                (local.set $sum (i32.add (local.get $sum) (call $state_put (i32.const 0) (i32.const 1) (i32.const 0) (i32.const 1))))
                (local.set $sum (i32.add (local.get $sum) (call $clipboard_read (i32.const 0) (i32.const 16))))
                (drop (call $navigate (i32.const 0) (i32.const 18) (i32.const 0)))
                (drop (call $dl_begin))
                (drop (call $dl_rect (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (f32.const 0) (i32.const 0)))
                (drop (call $dl_end))
                local.get $sum)
            {STUBS})"#
    ));
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    for _ in 0..2 {
        let dl = rt.frame_with_dt(16).unwrap().to_vec();
        assert_eq!(dl.len(), 26, "the frame still produced its display list");
        assert_eq!(rt.last_frame().ops_reported, 3 * err::E_DENIED);
    }
    let denied: Vec<String> = host_logs(&rt)
        .into_iter()
        .filter(|m| m.contains("E_DENIED"))
        .collect();
    assert_eq!(denied.len(), 2 * 4, "{denied:?}");
    assert!(denied.iter().any(|m| m.starts_with("fetch_begin:")));
    assert!(denied.iter().any(|m| m.starts_with("state_put:")));
    assert!(denied.iter().any(|m| m.starts_with("clipboard_read:")));
    assert!(denied.iter().any(|m| m.starts_with("navigate:")));
    assert!(!rt.is_suspended());
    assert_eq!(rt.exit_code(), None);
}

/// SPEC-5.2.8: fetch_begin to an unlisted origin returns E_ORIGIN with no network call.
#[test]
#[ignore = "M3: network"]
fn spec_5_2_8_origin_gate() {
    todo!()
}

fn budget_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module
            (import "vy" "log" (func $log (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "budget")
            (data (i32.const 16) "suspend")
            (data (i32.const 32) "final")
            {INIT_OK}
            ;; Spin dt_ms times; -1 spins until the hard limit.
            (func (export "vy_frame") (param i32) (result i32)
                (local $i i32)
                (local.set $i (local.get 0))
                (block (loop
                    (br_if 1 (i32.eqz (local.get $i)))
                    (local.set $i (i32.sub (local.get $i) (i32.const 1)))
                    (br 0)))
                i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 31)) (then
                    (drop (call $log (i32.const 0) (i32.const 0) (i32.const 6)))
                    (if (i32.eq (local.get 3) (i32.const 1)) (then
                        (drop (call $log (i32.const 0) (i32.const 32) (i32.const 5)))))))
                i32.const 0)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32)
                (drop (call $log (i32.const 0) (i32.const 16) (i32.const 7)))
                i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
    ))
}

/// SPEC-5.3.3, SPEC-9.3.1: two frames over 8 ms of logical time in a row yield KIND_BUDGET
/// and then a forced vy_suspend; a frame within budget resets the streak.
#[test]
fn spec_5_3_3_frame_budget() {
    // Soft breach: more than FRAME_BUDGET_FUEL but under the hard limit.
    let slow = 3_000_000;
    let mut rt = load(&budget_module());
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(0).unwrap();
    assert!(!rt.last_frame().over_budget);
    assert!(rt.last_frame().fuel < FRAME_BUDGET_FUEL);

    rt.frame_with_dt(slow).unwrap();
    assert!(rt.last_frame().over_budget, "{:?}", rt.last_frame());
    assert_eq!(module_logs(&rt), vec!["budget"], "first breach: a warning");
    rt.frame_with_dt(0).unwrap();
    assert!(!rt.last_frame().over_budget);
    rt.frame_with_dt(slow).unwrap();
    assert_eq!(
        module_logs(&rt),
        vec!["budget", "budget"],
        "streak was reset"
    );
    assert!(!rt.is_suspended());

    rt.frame_with_dt(slow).unwrap();
    assert_eq!(
        module_logs(&rt),
        vec!["budget", "budget", "budget", "final", "suspend"],
        "second consecutive breach: final warning then forced suspend"
    );
    assert!(rt.is_suspended());
    assert!(matches!(rt.frame_with_dt(0), Err(Error::Suspended)));

    // Hard limit: the frame is aborted and counts as a breach.
    let mut rt = load(&budget_module());
    rt.init(10, 10, 1.0).unwrap();
    let e = rt.frame_with_dt(-1).unwrap_err();
    assert!(
        matches!(e, Error::OverBudget { used_ms } if used_ms >= 8 * 8),
        "{e}"
    );
    assert_eq!(module_logs(&rt), vec!["budget"]);
    assert!(matches!(
        rt.frame_with_dt(-1),
        Err(Error::OverBudget { .. })
    ));
    assert!(rt.is_suspended());
    assert_eq!(
        module_logs(&rt),
        vec!["budget", "budget", "final", "suspend"]
    );
}

/// A module whose frame does `dt_ms` text measurements of 64 KB each when
/// `dt_ms > 0`, and asks `random` for `-dt_ms` bytes when `dt_ms < 0`. Both
/// are a few wasm instructions; all the work is in the host.
fn host_work_module() -> Vec<u8> {
    let text = "a".repeat(limits::MAX_TEXT_BYTES as usize);
    wasm(&format!(
        r#"(module
            (import "vy" "log" (func $log (param i32 i32 i32) (result i32)))
            (import "vy" "text_style" (func $text_style (param i32 f32 i32 i32) (result i32)))
            (import "vy" "text_measure" (func $text_measure (param i32 f32 i32 i32) (result i64)))
            (import "vy" "random" (func $random (param i32 i32) (result i32)))
            ;; page 0: the text; page 1: the log message; pages 2..: random
            (memory (export "memory") 34)
            (global $style (mut i32) (i32.const 0))
            (data (i32.const 0) "{text}")
            (data (i32.const 65536) "budget")
            (func (export "vy_init") (param i32 i32 i32 f32) (result i32)
                (global.set $style (call $text_style (i32.const -1) (f32.const 16) (i32.const 400) (i32.const 0)))
                i32.const 0)
            (func (export "vy_frame") (param i32) (result i32)
                (local $i i32)
                (if (i32.lt_s (local.get 0) (i32.const 0)) (then
                    (drop (call $random (i32.const 131072) (i32.sub (i32.const 0) (local.get 0))))
                    (return (i32.const 0))))
                (local.set $i (local.get 0))
                (block (loop
                    (br_if 1 (i32.eqz (local.get $i)))
                    (drop (call $text_measure (global.get $style) (f32.const 0) (i32.const 0) (i32.const {len})))
                    (local.set $i (i32.sub (local.get $i) (i32.const 1)))
                    (br 0)))
                i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 31)) (then
                    (drop (call $log (i32.const 0) (i32.const 65536) (i32.const 6)))))
                i32.const 0)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
        len = limits::MAX_TEXT_BYTES,
    ))
}

/// SPEC-5.3.3, SPEC-9.3.1: host work inside imports is charged to the frame
/// budget at FUEL_PER_HOST_BYTE, so a frame of a few wasm instructions that
/// shapes four 64 KB runs breaches the 8 ms budget and gets KIND_BUDGET; a
/// large enough charge exhausts the hard limit and aborts the frame.
#[test]
fn spec_5_3_3_host_work_costs_fuel() {
    let per_measure = u64::from(limits::MAX_TEXT_BYTES) * FUEL_PER_HOST_BYTE;
    let over = (FRAME_BUDGET_FUEL / per_measure + 1) as i32;
    assert_eq!(over, 4, "the documented example: four 64 KB runs");

    let mut rt = load(&host_work_module());
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(1).unwrap();
    let one = rt.last_frame();
    assert!(!one.over_budget, "{one:?}");
    assert_eq!(
        one.host_fuel, per_measure,
        "exactly len * FUEL_PER_HOST_BYTE"
    );
    assert!(one.fuel >= one.host_fuel, "host fuel is part of the total");
    assert!(
        one.fuel - one.host_fuel < 10_000,
        "the wasm side is cheap: {one:?}"
    );

    rt.frame_with_dt(over).unwrap();
    let four = rt.last_frame();
    assert!(four.over_budget, "{four:?}");
    assert_eq!(four.host_fuel, u64::from(over as u32) * per_measure);
    assert_eq!(module_logs(&rt), vec!["budget"], "KIND_BUDGET delivered");
    rt.frame_with_dt(over).unwrap();
    assert!(
        rt.is_suspended(),
        "second consecutive breach: forced suspend"
    );

    // The hard limit: 2 MB of random bytes is 64 M fuel, so the instruction
    // after the import traps with OutOfFuel and the frame is aborted.
    let bytes = (CALL_HARD_LIMIT_FUEL / FUEL_PER_HOST_BYTE) as i32;
    let mut rt = load(&host_work_module());
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(-(bytes / 16)).unwrap();
    assert!(!rt.last_frame().over_budget, "{:?}", rt.last_frame());
    assert_eq!(rt.last_frame().host_fuel, CALL_HARD_LIMIT_FUEL / 16);
    let r = rt.frame_with_dt(-bytes).map(|_| ());
    assert!(
        matches!(r, Err(Error::OverBudget { .. })),
        "{r:?} {:?}",
        rt.last_frame()
    );
    assert_eq!(module_logs(&rt), vec!["budget"]);
}

/// SPEC-5.3.4, SPEC-4.2.1: the host never calls the module outside the six exports.
/// Structurally: `Runtime` wraps exactly those six, a module exporting more is
/// refused before it is compiled, and an import never re-enters the module.
#[test]
fn spec_5_3_4_no_callbacks() {
    let extra = wasm(&format!(
        r#"(module
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) i32.const 0)
            (func (export "vy_tick"))
            {STUBS})"#
    ));
    match Runtime::load(&extra, &sha256(&extra), Options::default()) {
        Err(Error::Abi(report)) => assert!(
            report
                .problems
                .iter()
                .any(|p| p.contains("extra function export vy_tick")),
            "{report}"
        ),
        other => panic!("expected ABI rejection, got {:?}", other.map(|_| ())),
    }
    assert_eq!(vy_abi::EXPORTS.len(), 6);

    // request_frame inside vy_frame does not re-enter vy_frame: one call, one frame.
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "request_frame" (func $request_frame))
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (call $request_frame) (call $request_frame)
                i32.const 7)
            {STUBS})"#
    ));
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(0).unwrap();
    assert_eq!(rt.frames(), 1);
    assert_eq!(rt.now_ms(), host::HEADLESS_FRAME_MS);
    assert!(rt.wants_frame());
}

fn drawing_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module
            (import "vy" "dl_begin" (func $dl_begin (result i32)))
            (import "vy" "dl_end" (func $dl_end (result i32)))
            (import "vy" "dl_rect" (func $dl_rect (param f32 f32 f32 f32 f32 i32) (result i32)))
            (import "vy" "dl_text" (func $dl_text (param f32 f32 f32 i32 i32 i32) (result i64)))
            (import "vy" "text_style" (func $text_style (param i32 f32 i32 i32) (result i32)))
            (import "vy" "random" (func $random (param i32 i32) (result i32)))
            (import "vy" "now_ms" (func $now_ms (result i64)))
            (memory (export "memory") 1)
            (global $style (mut i32) (i32.const 0))
            (data (i32.const 0) "Hej från Vy")
            (func (export "vy_init") (param i32 i32 i32 f32) (result i32)
                (global.set $style (call $text_style (i32.const -1) (f32.const 18) (i32.const 700) (i32.const 0x202020FF)))
                i32.const 0)
            (func (export "vy_frame") (param i32) (result i32)
                (drop (call $random (i32.const 64) (i32.const 4)))
                (drop (call $dl_begin))
                (drop (call $dl_rect (f32.const 0) (f32.const 0) (f32.const 120) (f32.const 60) (f32.const 0) (i32.const 0xF0F0F0FF)))
                ;; a rectangle placed by the random source and the clock
                (drop (call $dl_rect
                    (f32.convert_i32_u (i32.rem_u (i32.load8_u (i32.const 64)) (i32.const 40)))
                    (f32.convert_i64_s (i64.rem_s (call $now_ms) (i64.const 40)))
                    (f32.const 10) (f32.const 10) (f32.const 2) (i32.const 0xFF0000FF)))
                (drop (call $dl_text (f32.const 4) (f32.const 30) (f32.const 110) (global.get $style) (i32.const 0) (i32.const 12)))
                (drop (call $dl_end))
                i32.const 3)
            {STUBS})"#
    ))
}

/// SPEC-2.1.5: same module, same input, same window gives the same display list and the
/// same pixels, byte for byte; time and randomness come from the host.
#[test]
fn spec_2_1_5_determinism() {
    let bytes = drawing_module();
    let run = || {
        let mut rt = load(&bytes);
        rt.init(120, 60, 1.0).unwrap();
        let mut dls = Vec::new();
        for _ in 0..3 {
            dls.push(rt.frame_with_dt(16).unwrap().to_vec());
        }
        let images = vy_render::ImageCache::new(Vec::new());
        let img = vy_render::render(&dls[2], 120, 60, 1.0, rt.text_engine_mut(), &images).unwrap();
        (dls, img.encode_png().unwrap())
    };
    let (a_dls, a_png) = run();
    let (b_dls, b_png) = run();
    assert_eq!(a_dls, b_dls, "display lists differ between runs");
    assert_eq!(a_png, b_png, "pixels differ between runs");
    assert_ne!(a_dls[0], a_dls[1], "the clock and random source do advance");
}

/// SPEC-2.1.5: a module cannot observe which CPU it runs on through floating
/// point. `inf - inf` yields the default NaN, whose sign bit differs between
/// x86_64 (0xFFC00000) and AArch64 (0x7FC00000); the engine canonicalises it.
/// The operands come from `dt_ms` so the compiler cannot fold the expression.
#[test]
fn spec_2_1_5_nan_is_canonical() {
    let bytes = wasm(&format!(
        r#"(module
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (local $inf f32)
                (local.set $inf (f32.div (f32.convert_i32_s (local.get 0)) (f32.const 0)))
                (i32.reinterpret_f32 (f32.sub (local.get $inf) (local.get $inf))))
            {STUBS})"#
    ));
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(1).unwrap();
    assert_eq!(
        rt.last_frame().ops_reported as u32,
        0x7FC0_0000,
        "NaN must be canonical regardless of host CPU"
    );
}

/// SPEC-2.1.5: relaxed SIMD instructions may give different results on
/// different CPUs, so a module using them is refused before it runs; the ABI
/// check and the engine agree.
#[test]
fn spec_2_1_5_relaxed_simd_refused() {
    let bytes = wasm(&format!(
        r#"(module
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (drop (f32x4.relaxed_madd (v128.const i32x4 0 0 0 0) (v128.const i32x4 0 0 0 0) (v128.const i32x4 0 0 0 0)))
                i32.const 0)
            {STUBS})"#
    ));
    assert!(matches!(check_abi_bytes(&bytes), Err(Error::Invalid(_))));
    assert!(matches!(
        Runtime::load(&bytes, &sha256(&bytes), Options::default()),
        Err(Error::Invalid(_))
    ));
    // Plain SIMD is still fine: its semantics are fixed by the spec.
    let plain = wasm(&format!(
        r#"(module
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (i32x4.extract_lane 0 (i32x4.add (v128.const i32x4 1 0 0 0) (v128.const i32x4 2 0 0 0))))
            {STUBS})"#
    ));
    let mut rt = load(&plain);
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(0).unwrap();
    assert_eq!(rt.last_frame().ops_reported, 3);
}

/// SPEC-5.2.4, SPEC-5.2.5, SPEC-6.4.3: `dl_text` returns the laid-out width
/// × 100 in the high 32 bits and the height × 100 in the low ones, packed
/// exactly like `text_measure`; wrapping at `w` raises the height and
/// bounds the width, `w <= 0` wraps nothing, and a refused call is a
/// negative `i64`.
#[test]
fn spec_5_2_4_dl_text_size() {
    let bytes = wasm(
        r#"(module
            (import "vy" "dl_begin" (func $dl_begin (result i32)))
            (import "vy" "dl_end" (func $dl_end (result i32)))
            (import "vy" "dl_text" (func $dl_text (param f32 f32 f32 i32 i32 i32) (result i64)))
            (import "vy" "text_style" (func $text_style (param i32 f32 i32 i32) (result i32)))
            (import "vy" "text_measure" (func $text_measure (param i32 f32 i32 i32) (result i64)))
            (memory (export "memory") 1)
            (global $style (mut i32) (i32.const 0))
            (global $last (mut i64) (i64.const 0))
            (data (i32.const 0) "ett två tre fyra fem sex sju åtta nio tio")
            (func (export "vy_init") (param i32 i32 i32 f32) (result i32)
                (global.set $style (call $text_style (i32.const -1) (f32.const 16) (i32.const 400) (i32.const 0)))
                i32.const 0)
            ;; dt_ms is the wrap width; -1 asks for a bad style. Returns the low half.
            (func (export "vy_frame") (param i32) (result i32)
                (drop (call $dl_begin))
                (global.set $last (call $dl_text (f32.const 0) (f32.const 0) (f32.convert_i32_s (local.get 0))
                    (if (result i32) (i32.lt_s (local.get 0) (i32.const 0)) (then (i32.const 99)) (else (global.get $style)))
                    (i32.const 0) (i32.const 46)))
                (drop (call $dl_end))
                (i32.wrap_i64 (global.get $last)))
            ;; kind 0: low half of text_measure at width b; kind 1: high half of the
            ;; last dl_text; kind 2: 1 if the last dl_text was negative.
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 1)) (then
                    (return (i32.wrap_i64 (i64.shr_u (global.get $last) (i64.const 32))))))
                (if (i32.eq (local.get 0) (i32.const 2)) (then
                    (return (i64.lt_s (global.get $last) (i64.const 0)))))
                (i32.wrap_i64 (call $text_measure (global.get $style) (f32.convert_i32_s (local.get 1)) (i32.const 0) (i32.const 46))))
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
    );
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(1000).unwrap();
    let one_line = rt.last_frame().ops_reported;
    assert_eq!(one_line, 2000, "16 px * 1.25 line height * 100");
    // The high half is the width: one line is wider than 100 px.
    let r = rt.event(1, 0, 0, 0, 0);
    assert!(matches!(r, Ok(false)), "{r:?}");
    let wide_w = rt.last_event_result();
    assert!(wide_w > 100 * 100, "width x100: {wide_w}");
    assert!(!rt.event(2, 0, 0, 0, 0).unwrap(), "non-negative i64");

    rt.frame_with_dt(100).unwrap();
    let wrapped = rt.last_frame().ops_reported;
    assert!(wrapped > one_line, "{wrapped} > {one_line}");
    assert_eq!(wrapped % 2000, 0);
    rt.event(1, 0, 0, 0, 0).unwrap();
    let narrow_w = rt.last_event_result();
    assert!(
        narrow_w <= 100 * 100 && narrow_w > 0,
        "wrapped width x100: {narrow_w}"
    );
    let dl = rt.frame_with_dt(100).unwrap().to_vec();
    let ops = vy_dl::decode(&dl).unwrap();
    assert!(matches!(ops[0], vy_dl::Op::Text { max_w, .. } if max_w == 100.0));
    // text_measure's low 32 bits are the same height (SPEC-5.2.5); vy_event here
    // returns them, so "redraw" is reported exactly when they equal 1, which they never do.
    assert!(!rt.event(0, 100, 0, 0, 0).unwrap());
    let m = rt
        .text_engine_mut()
        .measure(1, 100.0, "ett två tre fyra fem sex sju åtta nio tio")
        .unwrap();
    assert_eq!(
        vy_render::pack_x100(m.w, m.h),
        ((i64::from(narrow_w)) << 32) | i64::from(wrapped)
    );

    // w <= 0: no wrapping, same as the wide layout (SPEC-6.4.3).
    rt.frame_with_dt(0).unwrap();
    assert_eq!(rt.last_frame().ops_reported, one_line);
    rt.event(1, 0, 0, 0, 0).unwrap();
    assert_eq!(rt.last_event_result(), wide_w);

    // A refused call is the status code sign-extended to i64: negative as a whole.
    rt.frame_with_dt(-1).unwrap();
    assert_eq!(
        rt.last_frame().ops_reported,
        err::E_STYLE,
        "low half is the code"
    );
    assert!(
        rt.event(2, 0, 0, 0, 0).unwrap(),
        "the whole i64 is negative"
    );
    assert!(host_logs(&rt).contains(&"dl_text: E_STYLE".to_string()));
}

/// SPEC-4.2.4, SPEC-8.1.3: memory may not grow past memory_max.
#[test]
fn spec_4_2_4_memory_cap() {
    let bytes = wasm(&format!(
        r#"(module
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) (memory.grow (local.get 0)))
            {STUBS})"#
    ));
    let opts = Options {
        memory_max_pages: 4,
        ..Options::default()
    };
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts.clone()).unwrap();
    rt.init(10, 10, 1.0).unwrap();
    assert_eq!(rt.memory_pages(), 1);
    rt.frame_with_dt(2).unwrap();
    assert_eq!(rt.last_frame().ops_reported, 1, "grow returns the old size");
    assert_eq!(rt.memory_pages(), 3);
    rt.frame_with_dt(2).unwrap();
    assert_eq!(
        rt.last_frame().ops_reported,
        -1,
        "3 + 2 > 4 pages is refused"
    );
    assert_eq!(rt.memory_pages(), 3);
    rt.frame_with_dt(1).unwrap();
    assert_eq!(rt.memory_pages(), 4);
    rt.frame_with_dt(1).unwrap();
    assert_eq!(rt.last_frame().ops_reported, -1);
    assert_eq!(rt.memory_pages(), 4);

    // A module whose initial memory is already above the cap never instantiates.
    let big = wasm(&format!(
        r#"(module (memory (export "memory") 8) {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) i32.const 0) {STUBS})"#
    ));
    assert!(matches!(
        Runtime::load(&big, &sha256(&big), opts),
        Err(Error::Instantiate(_))
    ));
    for pages in [0, limits::MAX_MEMORY_PAGES + 1] {
        let opts = Options {
            memory_max_pages: pages,
            ..Options::default()
        };
        assert!(matches!(
            Runtime::load(&bytes, &sha256(&bytes), opts),
            Err(Error::MemoryMax(_))
        ));
    }
}

/// SPEC-9.3.1: resource limits enforced by the host.
#[test]
#[ignore = "M3: limits"]
fn spec_9_3_1_resource_limits() {
    todo!()
}

/// A module whose frame calls one import with a `len` one byte over its cap,
/// selected by `dt_ms`, and returns the import's status code (0 for imports
/// without one). Case 20 is the in-range control: a 64 KB `dl_text`.
fn caps_module() -> Vec<u8> {
    let text = "a".repeat(limits::MAX_TEXT_BYTES as usize + 1);
    wasm(&format!(
        r#"(module
            (import "vy" "log" (func $log (param i32 i32 i32) (result i32)))
            (import "vy" "dl_begin" (func $dl_begin (result i32)))
            (import "vy" "dl_end" (func $dl_end (result i32)))
            (import "vy" "dl_text" (func $dl_text (param f32 f32 f32 i32 i32 i32) (result i64)))
            (import "vy" "dl_path" (func $dl_path (param i32 i32 i32 f32) (result i32)))
            (import "vy" "text_style" (func $text_style (param i32 f32 i32 i32) (result i32)))
            (import "vy" "text_measure" (func $text_measure (param i32 f32 i32 i32) (result i64)))
            (import "vy" "text_caret" (func $text_caret (param i32 i32 i32 i32) (result i64)))
            (import "vy" "sem_node" (func $sem_node (param i32 i32 i32 f32 f32 f32 f32 i32 i32) (result i32)))
            (import "vy" "link_register" (func $link_register (param i32 f32 f32 f32 f32 i32 i32) (result i32)))
            (import "vy" "navigate" (func $navigate (param i32 i32 i32) (result i32)))
            (import "vy" "fetch_begin" (func $fetch_begin (param i32 i32 i32 i32 i32) (result i32)))
            (import "vy" "state_put" (func $state_put (param i32 i32 i32 i32) (result i32)))
            (import "vy" "state_get" (func $state_get (param i32 i32 i32 i32) (result i32)))
            (import "vy" "state_delete" (func $state_delete (param i32 i32) (result i32)))
            (import "vy" "clipboard_write" (func $clipboard_write (param i32 i32) (result i32)))
            ;; 17 pages: every oversized range below lies inside memory, so the
            ;; only reason to refuse it is the cap.
            (memory (export "memory") 17)
            (global $style (mut i32) (i32.const 0))
            (data (i32.const 0) "{text}")
            (func (export "vy_init") (param i32 i32 i32 f32) (result i32)
                (global.set $style (call $text_style (i32.const -1) (f32.const 16) (i32.const 400) (i32.const 0)))
                i32.const 0)
            (func $case (param $n i32) (result i32)
                (local $r i32)
                (block $done
                    (if (i32.eq (local.get $n) (i32.const 1)) (then
                        (drop (call $log (i32.const 0) (i32.const 0) (i32.const {log1})))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 2)) (then
                        (drop (call $dl_begin))
                        (local.set $r (i32.wrap_i64 (call $dl_text (f32.const 0) (f32.const 0) (f32.const 0) (global.get $style) (i32.const 0) (i32.const {text1}))))
                        (drop (call $dl_end))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 3)) (then
                        (local.set $r (i32.wrap_i64 (call $text_measure (global.get $style) (f32.const 0) (i32.const 0) (i32.const {text1}))))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 4)) (then
                        (local.set $r (i32.wrap_i64 (call $text_caret (global.get $style) (i32.const 0) (i32.const {text1}) (i32.const 0))))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 5)) (then
                        (drop (call $dl_begin))
                        (drop (call $dl_path (i32.const 0) (i32.const {path1}) (i32.const 0) (f32.const 0)))
                        (drop (call $dl_end))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 6)) (then
                        (drop (call $sem_node (i32.const 1) (i32.const 0) (i32.const 1) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const {sem1})))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 7)) (then
                        (drop (call $link_register (i32.const 1) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const {url1})))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 8)) (then
                        (drop (call $navigate (i32.const 0) (i32.const {url1}) (i32.const 0)))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 9)) (then
                        (local.set $r (call $fetch_begin (i32.const 0) (i32.const 0) (i32.const {url1}) (i32.const 0) (i32.const 0)))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 10)) (then
                        (local.set $r (call $fetch_begin (i32.const 0) (i32.const 0) (i32.const 10) (i32.const 0) (i32.const {body1})))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 11)) (then
                        (local.set $r (call $state_put (i32.const 0) (i32.const {key1}) (i32.const 0) (i32.const 1)))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 12)) (then
                        (local.set $r (call $state_put (i32.const 0) (i32.const 1) (i32.const 0) (i32.const {val1})))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 13)) (then
                        (local.set $r (call $state_get (i32.const 0) (i32.const {key1}) (i32.const 0) (i32.const 0)))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 14)) (then
                        (drop (call $state_delete (i32.const 0) (i32.const {key1})))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 15)) (then
                        (drop (call $clipboard_write (i32.const 0) (i32.const {clip1})))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 20)) (then
                        (drop (call $log (i32.const 0) (i32.const 0) (i32.const {log0})))
                        (drop (call $dl_begin))
                        (local.set $r (i32.wrap_i64 (call $dl_text (f32.const 0) (f32.const 0) (f32.const 0) (global.get $style) (i32.const 0) (i32.const {text0}))))
                        (drop (call $dl_end))
                        (br $done))))
                local.get $r)
            (func (export "vy_frame") (param i32) (result i32) (call $case (local.get 0)))
            {STUBS})"#,
        log0 = limits::MAX_LOG_BYTES,
        log1 = limits::MAX_LOG_BYTES + 1,
        text0 = limits::MAX_TEXT_BYTES,
        text1 = limits::MAX_TEXT_BYTES + 1,
        path1 = limits::MAX_PATH_BYTES + vy_dl::PATH_CMD_BYTES as u32,
        sem1 = limits::MAX_SEM_TEXT_BYTES + 1,
        url1 = limits::MAX_URL_BYTES + 1,
        body1 = limits::MAX_FETCH_BODY_BYTES + 1,
        key1 = limits::MAX_STATE_KEY_BYTES + 1,
        val1 = limits::MAX_STATE_VALUE_BYTES + 1,
        clip1 = limits::MAX_CLIPBOARD_BYTES + 1,
    ))
}

/// SPEC-9.3.1: every copy out of module memory is capped per call. One byte
/// over the cap is E_ARG, logged with the cap, and nothing is copied; the
/// capability gate (SPEC-5.3.1) still comes first for net, storage and
/// clipboard; the cap itself is accepted.
#[test]
fn spec_9_3_1_byte_caps_per_call() {
    // (case, import, what, cap)
    let cases: [(i32, &str, &str, u32); 15] = [
        (1, "log", "message", limits::MAX_LOG_BYTES),
        (2, "dl_text", "text", limits::MAX_TEXT_BYTES),
        (3, "text_measure", "text", limits::MAX_TEXT_BYTES),
        (4, "text_caret", "text", limits::MAX_TEXT_BYTES),
        (5, "dl_path", "path", limits::MAX_PATH_BYTES),
        (6, "sem_node", "text", limits::MAX_SEM_TEXT_BYTES),
        (7, "link_register", "url", limits::MAX_URL_BYTES),
        (8, "navigate", "url", limits::MAX_URL_BYTES),
        (9, "fetch_begin", "url", limits::MAX_URL_BYTES),
        (10, "fetch_begin", "body", limits::MAX_FETCH_BODY_BYTES),
        (11, "state_put", "key", limits::MAX_STATE_KEY_BYTES),
        (12, "state_put", "value", limits::MAX_STATE_VALUE_BYTES),
        (13, "state_get", "key", limits::MAX_STATE_KEY_BYTES),
        (14, "state_delete", "key", limits::MAX_STATE_KEY_BYTES),
        (15, "clipboard_write", "text", limits::MAX_CLIPBOARD_BYTES),
    ];
    let returns_code = |case: i32| matches!(case, 2 | 3 | 4 | 9 | 10 | 11 | 12 | 13);
    let gated = |case: i32| case >= 9;

    let bytes = caps_module();
    let all = Capabilities {
        net: true,
        storage: true,
        clipboard: true,
        hires_time: false,
        ime: false,
        persist_frame: false,
    };
    for caps in [Capabilities::default(), all] {
        let opts = Options {
            capabilities: caps,
            ..Options::default()
        };
        let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
        rt.init(10, 10, 1.0).unwrap();
        for (case, name, what, cap) in cases {
            let before = rt.logs().len();
            rt.frame_with_dt(case).unwrap();
            let stats = rt.last_frame();
            let new: Vec<String> = rt.logs()[before..]
                .iter()
                .map(|l| format!("{:?} {}", l.source, l.message))
                .collect();
            let expect_cap = !gated(case) || caps == all;
            if expect_cap {
                // One byte over, or one whole command for the path buffer.
                let over = if case == 5 {
                    vy_dl::PATH_CMD_BYTES as u32
                } else {
                    1
                };
                let line = format!(
                    "Host {name}: E_ARG ({what} is {} bytes, cap is {cap}; nothing copied)",
                    cap + over
                );
                assert_eq!(new, vec![line], "case {case}");
                if returns_code(case) {
                    assert_eq!(stats.ops_reported, err::E_ARG, "case {case}");
                }
                // A refused call is free; cases 2 and 5 also pay for dl_end's END byte.
                let end = u64::from(matches!(case, 2 | 5)) * FUEL_PER_HOST_BYTE;
                assert_eq!(stats.host_fuel, end, "case {case}: a refused call is free");
            } else {
                assert_eq!(new.len(), 1, "case {case}: {new:?}");
                assert!(
                    new[0].starts_with(&format!("Host {name}: E_DENIED")),
                    "case {case}: capability gate first (SPEC-5.3.1): {new:?}"
                );
                if returns_code(case) {
                    assert_eq!(stats.ops_reported, err::E_DENIED, "case {case}");
                }
            }
            assert!(rt.display_list().len() <= 1, "case {case}: nothing drawn");
            assert!(rt.semantic_tree().is_empty(), "case {case}");
            assert!(rt.links().is_empty(), "case {case}");
            assert!(
                rt.logs()
                    .iter()
                    .all(|l| l.source == LogSource::Host || l.message.is_empty()),
                "nothing was copied out of the module"
            );
        }
        assert!(!rt.is_suspended());
    }

    // The caps themselves pass: a 4 KB log line and a 64 KB text run.
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(20).unwrap();
    let stats = rt.last_frame();
    assert!(
        stats.ops_reported > 0,
        "dl_text returned a height: {stats:?}"
    );
    assert!(host_logs(&rt).is_empty(), "{:?}", host_logs(&rt));
    let logs = module_logs(&rt);
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].len(), limits::MAX_LOG_BYTES as usize);
    let ops = vy_dl::decode(rt.display_list()).unwrap();
    assert!(
        matches!(&ops[0], vy_dl::Op::Text { text, .. } if text.len() == limits::MAX_TEXT_BYTES as usize)
    );
    assert_eq!(
        stats.host_fuel,
        FUEL_PER_HOST_BYTE
            * (u64::from(limits::MAX_LOG_BYTES) + 2 * u64::from(limits::MAX_TEXT_BYTES) + 17 + 1),
        "log bytes, shaped bytes, TEXT op bytes, END"
    );
}

/// A module that builds the semantic tree and display list from `vy_event`:
/// kind 0 adds `a` nodes with `b` bytes of text each (ids from a running
/// counter, so no two nodes of one frame share an id, SPEC-5.2.13), kind 1
/// draws two text runs of `a` and `b` bytes and returns the status of the
/// first.
fn frame_caps_module() -> Vec<u8> {
    let text = "a".repeat(limits::MAX_SEM_TEXT_BYTES as usize);
    wasm(&format!(
        r#"(module
            (import "vy" "dl_begin" (func $dl_begin (result i32)))
            (import "vy" "dl_end" (func $dl_end (result i32)))
            (import "vy" "dl_text" (func $dl_text (param f32 f32 f32 i32 i32 i32) (result i64)))
            (import "vy" "text_style" (func $text_style (param i32 f32 i32 i32) (result i32)))
            (import "vy" "sem_node" (func $sem_node (param i32 i32 i32 f32 f32 f32 f32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (global $style (mut i32) (i32.const 0))
            (global $next (mut i32) (i32.const 1))
            (data (i32.const 0) "{text}")
            (func (export "vy_init") (param i32 i32 i32 f32) (result i32)
                (global.set $style (call $text_style (i32.const -1) (f32.const 16) (i32.const 400) (i32.const 0)))
                i32.const 0)
            (func (export "vy_frame") (param i32) (result i32)
                (global.set $next (i32.const 1))
                i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (local $i i32)
                (local $r i32)
                (if (i32.eq (local.get 0) (i32.const 0)) (then
                    (block (loop
                        (br_if 1 (i32.ge_s (local.get $i) (local.get 1)))
                        (drop (call $sem_node (global.get $next) (i32.const 0) (i32.const 1) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (local.get 2)))
                        (global.set $next (i32.add (global.get $next) (i32.const 1)))
                        (local.set $i (i32.add (local.get $i) (i32.const 1)))
                        (br 0)))))
                (if (i32.eq (local.get 0) (i32.const 1)) (then
                    (drop (call $dl_begin))
                    (local.set $r (i32.wrap_i64 (call $dl_text (f32.const 0) (f32.const 0) (f32.const 0) (global.get $style) (i32.const 0) (local.get 1))))
                    (drop (call $dl_text (f32.const 0) (f32.const 0) (f32.const 0) (global.get $style) (i32.const 0) (local.get 2)))
                    (drop (call $dl_end))))
                local.get $r)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
    ))
}

/// SPEC-9.3.1, SPEC-10.1.1: per frame the semantic tree holds at most 65 536
/// nodes, and the tree and the display list together hold at most 16 MB of
/// text; the call that would exceed either is E_DL_FULL and copies nothing.
/// The tree lives until the next `vy_frame`, so it is filled from events,
/// each of which has its own fuel.
#[test]
fn spec_9_3_1_frame_caps() {
    let nodes = limits::MAX_SEM_NODES_PER_FRAME as i32;
    let mut rt = load(&frame_caps_module());
    rt.init(10, 10, 1.0).unwrap();
    rt.event(0, nodes, 0, 0, 0).unwrap();
    assert_eq!(rt.semantic_tree().len(), nodes as usize);
    rt.event(0, 2, 0, 0, 0).unwrap();
    assert_eq!(rt.semantic_tree().len(), nodes as usize, "65 537th dropped");
    assert_eq!(host_logs(&rt), vec!["sem_node: E_DL_FULL"], "logged once");
    rt.frame_with_dt(0).unwrap();
    assert!(rt.semantic_tree().is_empty(), "a frame starts a new tree");

    // 16 MB of text in the tree, 256 nodes (1 MB) per event call.
    let mut rt = load(&frame_caps_module());
    rt.init(10, 10, 1.0).unwrap();
    let sem_len = limits::MAX_SEM_TEXT_BYTES as i32;
    let per_call = 256;
    let calls = limits::MAX_FRAME_TEXT_BYTES / (per_call as u32 * sem_len as u32);
    for _ in 0..calls {
        rt.event(0, per_call, sem_len, 0, 0).unwrap();
    }
    let full = limits::MAX_FRAME_TEXT_BYTES as usize;
    assert_eq!(
        rt.semantic_tree().len() * sem_len as usize,
        full,
        "exactly 16 MB fits"
    );
    assert!(host_logs(&rt).is_empty());
    rt.event(0, 1, 1, 0, 0).unwrap();
    assert_eq!(host_logs(&rt), vec!["sem_node: E_DL_FULL"], "one more byte");
    assert_eq!(rt.semantic_tree().len() * sem_len as usize, full);
    rt.event(0, 1, 0, 0, 0).unwrap();
    assert_eq!(
        rt.semantic_tree().len(),
        full / sem_len as usize + 1,
        "an empty node adds no text and fits"
    );

    // The display list shares the budget: with 16 MB in the tree, a one-byte
    // text run does not fit; an empty one does.
    assert!(!rt.event(1, 1, 0, 0, 0).unwrap());
    assert_eq!(
        host_logs(&rt),
        vec!["sem_node: E_DL_FULL", "dl_text: E_DL_FULL"]
    );
    let ops = vy_dl::decode(rt.display_list()).unwrap();
    assert_eq!(ops.len(), 1, "only the empty run was written: {ops:?}");
    assert!(matches!(&ops[0], vy_dl::Op::Text { text, .. } if text.is_empty()));

    // After a frame the tree is empty and both fit again.
    rt.frame_with_dt(0).unwrap();
    rt.event(1, 1, 1, 0, 0).unwrap();
    assert_eq!(vy_dl::decode(rt.display_list()).unwrap().len(), 2);
    assert_eq!(host_logs(&rt).len(), 2, "no new complaint");
}

/// SPEC-9.4.1, SPEC-2.1.2: a module whose hash mismatches is never run, or even parsed.
#[test]
fn spec_9_4_1_hash_mismatch_never_runs() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "log" (func $log (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (start $boom)
            (func $boom (drop (call $log (i32.const 3) (i32.const 0) (i32.const 0))))
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) i32.const 0)
            {STUBS})"#
    ));
    let mut wrong = sha256(&bytes);
    wrong[0] ^= 0xFF;
    match Runtime::load(&bytes, &wrong, Options::default()) {
        Err(Error::HashMismatch { expected, actual }) => {
            assert_eq!(expected, wrong);
            assert_eq!(actual, sha256(&bytes));
        }
        other => panic!("expected hash mismatch, got {:?}", other.map(|_| ())),
    }
    // The hash is checked before the bytes are parsed: garbage with a wrong hash
    // is a hash mismatch, not an invalid module.
    assert!(matches!(
        Runtime::load(b"garbage", &wrong, Options::default()),
        Err(Error::HashMismatch { .. })
    ));
    // With the right hash the bytes are parsed, and a start function is a hidden
    // entry point outside the six exports (SPEC-5.3.4), so the module is refused.
    match Runtime::load(&bytes, &sha256(&bytes), Options::default()) {
        Err(Error::Abi(report)) => assert!(
            report.problems.iter().any(|p| p.contains("start function")),
            "{report}"
        ),
        other => panic!("expected ABI rejection, got {:?}", other.map(|_| ())),
    }
}

/// SPEC-9.5.1, SPEC-9.5.2: storage and state are per origin and never shared.
#[test]
#[ignore = "M3: storage"]
fn spec_9_5_1_per_origin_isolation() {
    todo!()
}

/// SPEC-10.1.1, SPEC-10.1.2, SPEC-5.2.13: the semantic tree is rebuilt every frame via
/// sem_node; nodes with a role outside 1..=24 are ignored (and logged as E_ARG).
#[test]
fn spec_10_1_1_semantic_tree() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "sem_node" (func $sem_node (param i32 i32 i32 f32 f32 f32 f32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "Hej \"där\"")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (drop (call $sem_node (i32.const 1) (i32.const 0) (i32.const 15) (f32.const 0) (f32.const 0) (f32.const 100) (f32.const 50) (i32.const 0) (i32.const 0)))
                (drop (call $sem_node (i32.const 2) (i32.const 1) (i32.const 2) (f32.const 4) (f32.const 4.5) (f32.const 90) (f32.const 20) (i32.const 0) (i32.const 10)))
                (drop (call $sem_node (i32.const 3) (i32.const 1) (i32.const 25) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const 0)))
                (drop (call $sem_node (i32.const 4) (i32.const 1) (i32.const 0) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const 0)))
                (drop (call $sem_node (i32.const 5) (i32.const 1) (i32.const 1) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 65530) (i32.const 100)))
                i32.const 0)
            {STUBS})"#
    ));
    let mut rt = load(&bytes);
    rt.init(100, 50, 1.0).unwrap();
    assert!(rt.semantic_tree().is_empty());
    for _ in 0..2 {
        rt.frame_with_dt(16).unwrap();
        let tree = rt.semantic_tree();
        assert_eq!(tree.len(), 2, "{tree:?}");
        assert_eq!(
            tree[0],
            SemNode {
                id: 1,
                parent_id: 0,
                role: vy_abi::role::MAIN,
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 50.0,
                text: String::new()
            }
        );
        assert_eq!(tree[1].role, vy_abi::role::HEADING);
        assert_eq!(tree[1].text, "Hej \"där\"");
    }
    let bad: Vec<_> = host_logs(&rt)
        .into_iter()
        .filter(|m| m.starts_with("sem_node: E_ARG"))
        .collect();
    assert_eq!(
        bad.len(),
        2 * 3,
        "two bad roles and one bad pointer per frame"
    );
    let json = rt.semantic_tree_json();
    assert!(json.contains(r#""role_name": "main""#), "{json}");
    assert!(json.contains(r#""text": "Hej \"där\"""#), "{json}");
    assert!(json.contains(r#""y": 4.5"#), "{json}");
}

/// SPEC-5.2.11: now_ms is monotonic with 1 ms resolution; headless it advances per frame.
#[test]
fn spec_5_2_11_time_resolution() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "now_ms" (func $now_ms (result i64)))
            (memory (export "memory") 1)
            {INIT_OK}
            ;; returns now_ms, or -1 if two reads in one frame disagree
            (func (export "vy_frame") (param i32) (result i32)
                (local $a i64)
                (local.set $a (call $now_ms))
                (if (i64.ne (local.get $a) (call $now_ms)) (then (return (i32.const -1))))
                (i32.wrap_i64 (local.get $a)))
            {STUBS})"#
    ));
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    let mut last = -1;
    for i in 0..5 {
        rt.frame_with_dt(16).unwrap();
        let now = rt.last_frame().ops_reported;
        assert!(now > last, "monotonic: {now} after {last}");
        assert_eq!(
            i64::from(now),
            i * host::HEADLESS_FRAME_MS,
            "whole milliseconds, no wall clock"
        );
        last = now;
    }
}

/// SPEC-5.1.3, SPEC-5.1.6: committed text reaches the module through vy_alloc and vy_event.
#[test]
fn spec_5_1_6_text_input_via_alloc() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "log" (func $log (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 7)) (then
                    (drop (call $log (i32.const 0) (local.get 1) (local.get 2)))))
                i32.const 1)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 4096))"#
    ));
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    assert!(rt.text_input("Hej från Vy").unwrap());
    assert_eq!(module_logs(&rt), vec!["Hej från Vy"]);
    assert!(rt.wants_frame());
}

/// A module whose frame returns `dt_ms` and whose `vy_event` returns 1 for
/// kind 1; `key_down` of the code given as `a` to kind 2 decides the frame's
/// return value instead when kind 2 was the last event.
fn lifecycle_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module
            (import "vy" "request_frame" (func $request_frame))
            (import "vy" "key_down" (func $key_down (param i32) (result i32)))
            (memory (export "memory") 1)
            (global $probe (mut i32) (i32.const -1))
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (if (i32.ge_s (global.get $probe) (i32.const 0)) (then
                    (return (call $key_down (global.get $probe)))))
                local.get 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 2)) (then
                    (global.set $probe (local.get 1))))
                (if (i32.eq (local.get 0) (i32.const 3)) (then (call $request_frame)))
                (i32.eq (local.get 0) (i32.const 1)))
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
    ))
}

/// SPEC-5.1.2: `dt_ms` is the host clock's advance since the previous
/// `vy_frame`, 0 in the first frame after `vy_init` and in the first after
/// `VISIBILITY` shown, so a page never integrates a pause.
#[test]
fn spec_5_1_2_dt_ms_is_time_since_previous_frame() {
    let mut rt = load(&lifecycle_module());
    rt.init(10, 10, 1.0).unwrap();
    rt.frame().unwrap();
    assert_eq!(rt.last_frame().ops_reported, 0, "first frame after init");
    rt.frame().unwrap();
    assert_eq!(
        rt.last_frame().ops_reported,
        host::HEADLESS_FRAME_MS as i32,
        "the headless clock advanced one frame"
    );
    rt.event(0, 0, 0, 0, 0).unwrap();
    rt.event(0, 0, 0, 0, 0).unwrap();
    rt.frame().unwrap();
    assert_eq!(
        rt.last_frame().ops_reported,
        host::HEADLESS_FRAME_MS as i32,
        "events do not advance the headless clock"
    );
    rt.event(event::VISIBILITY, 0, 0, 0, 0).unwrap();
    rt.event(event::VISIBILITY, 1, 0, 0, 0).unwrap();
    rt.frame().unwrap();
    assert_eq!(rt.last_frame().ops_reported, 0, "first frame after shown");
    rt.frame().unwrap();
    assert_eq!(rt.last_frame().ops_reported, host::HEADLESS_FRAME_MS as i32);
}

/// SPEC-5.2.6: `key_down` is 1 from `KEY_DOWN` to `KEY_UP`; the host
/// clears every held key when the tab is hidden, so a key held across a
/// `VISIBILITY` cycle is released as far as the module can tell.
#[test]
fn spec_5_2_6_key_state_held_until_up_or_hidden() {
    let mut rt = load(&lifecycle_module());
    rt.init(10, 10, 1.0).unwrap();
    let probe = |rt: &mut Runtime, code: i32| {
        rt.event(2, code, 0, 0, 0).unwrap();
        rt.frame().unwrap();
        rt.last_frame().ops_reported
    };
    assert_eq!(probe(&mut rt, 80), 0);
    rt.event(event::KEY_DOWN, 80, 0, 0, 0).unwrap();
    rt.event(event::KEY_DOWN, 80, 0, 1, 0).unwrap();
    assert_eq!(probe(&mut rt, 80), 1, "held, repeats do not matter");
    assert_eq!(probe(&mut rt, 79), 0, "another key is not");
    rt.event(event::KEY_UP, 80, 0, 0, 0).unwrap();
    assert_eq!(probe(&mut rt, 80), 0, "released");
    rt.event(event::KEY_DOWN, 80, 0, 0, 0).unwrap();
    rt.event(event::KEY_DOWN, 79, 0, 0, 0).unwrap();
    assert_eq!(probe(&mut rt, 80), 1);
    rt.event(event::VISIBILITY, 0, 0, 0, 0).unwrap();
    rt.event(event::VISIBILITY, 1, 0, 0, 0).unwrap();
    assert_eq!(probe(&mut rt, 80), 0, "hidden clears the held keys");
    assert_eq!(probe(&mut rt, 79), 0);
}

/// SPEC-6.2.1, SPEC-9.3.1, SPEC-13.3.2: no `vy_frame` while the tab is
/// hidden without `persist_frame`, whatever the module asked for; the
/// request is kept and served when the tab is shown again. With
/// `persist_frame` frames go on while hidden.
#[test]
fn spec_6_2_1_no_frame_while_hidden() {
    let mut rt = load(&lifecycle_module());
    rt.init(10, 10, 1.0).unwrap();
    rt.frame().unwrap();
    assert!(!rt.wants_frame());
    rt.event(event::VISIBILITY, 0, 0, 0, 0).unwrap();
    assert!(!rt.is_visible());
    assert!(
        rt.event(1, 0, 0, 0, 0).unwrap(),
        "the module asks for a redraw"
    );
    assert!(!rt.wants_frame(), "but hidden means no frame");
    rt.event(3, 0, 0, 0, 0).unwrap();
    assert!(!rt.wants_frame(), "request_frame neither");
    assert!(matches!(rt.frame(), Err(Error::Hidden)));
    assert_eq!(rt.frames(), 1, "nothing ran");
    rt.event(event::VISIBILITY, 1, 0, 0, 0).unwrap();
    assert!(rt.is_visible());
    assert!(rt.wants_frame(), "the request survived the pause");
    rt.frame().unwrap();
    assert_eq!(rt.frames(), 2);

    let bytes = lifecycle_module();
    let opts = Options {
        capabilities: Capabilities::from_names(["persist_frame"]).unwrap(),
        ..Options::default()
    };
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    rt.init(10, 10, 1.0).unwrap();
    rt.frame().unwrap();
    rt.event(event::VISIBILITY, 0, 0, 0, 0).unwrap();
    assert!(rt.event(1, 0, 0, 0, 0).unwrap());
    assert!(rt.wants_frame(), "persist_frame: frames continue hidden");
    rt.frame().unwrap();
    assert_eq!(rt.frames(), 2);
}

/// SPEC-5.1.6: when `vy_alloc` returns 0 (or a pointer with no room) the
/// host logs it and drops the event; the module never sees a `TEXT_INPUT`
/// it has no buffer for, and the page goes on.
#[test]
fn spec_5_1_6_alloc_failure_drops_the_event() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "log" (func $log (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (global $mode (mut i32) (i32.const 0))
            {INIT_OK}
            ;; dt_ms picks what vy_alloc answers: 0 => 0, 1 => past memory, 2 => 4096
            (func (export "vy_frame") (param i32) (result i32)
                (global.set $mode (local.get 0))
                i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 7)) (then
                    (drop (call $log (i32.const 0) (local.get 1) (local.get 2)))))
                i32.const 1)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32)
                (if (i32.eq (global.get $mode) (i32.const 0)) (then (return (i32.const 0))))
                (if (i32.eq (global.get $mode) (i32.const 1)) (then (return (i32.const 65530))))
                i32.const 4096))"#
    ));
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(0).unwrap();
    assert!(!rt.text_input("Hej").unwrap(), "dropped, no redraw asked");
    assert_eq!(
        host_logs(&rt),
        vec!["TEXT_INPUT dropped: vy_alloc(3) returned 0 (SPEC-5.1.6)"]
    );
    assert!(module_logs(&rt).is_empty(), "the module saw nothing");
    rt.frame_with_dt(1).unwrap();
    assert!(!rt.text_input("Hej från Vy").unwrap());
    assert_eq!(host_logs(&rt).len(), 2, "{:?}", host_logs(&rt));
    assert!(host_logs(&rt)[1].contains("returned 65530"));
    rt.frame_with_dt(2).unwrap();
    assert!(rt.text_input("Hej").unwrap(), "a real buffer: delivered");
    assert_eq!(module_logs(&rt), vec!["Hej"]);
    assert!(!rt.is_suspended());
}

/// A module whose frame draws one `IMAGE` with `resource_id = dt_ms / 10`
/// and `fit = dt_ms % 10`.
fn image_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module
            (import "vy" "dl_begin" (func $dl_begin (result i32)))
            (import "vy" "dl_end" (func $dl_end (result i32)))
            (import "vy" "dl_image" (func $dl_image (param f32 f32 f32 f32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (drop (call $dl_begin))
                (drop (call $dl_image (f32.const 0) (f32.const 0) (f32.const 4) (f32.const 2)
                    (i32.div_s (local.get 0) (i32.const 10))
                    (i32.rem_s (local.get 0) (i32.const 10))))
                (drop (call $dl_end))
                i32.const 1)
            {STUBS})"#
    ))
}

fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut img = vy_render::RgbaImage::new(w, h);
    for p in img.data.chunks_exact_mut(4) {
        p.copy_from_slice(&rgba);
    }
    img.encode_png().unwrap()
}

/// SPEC-5.2.3, SPEC-6.5.1, SPEC-4.5.1: `dl_image` takes `fit` 0..=2 (else
/// `E_ARG`, nothing written); the resource is decoded at the first call that
/// names it and the bytes read are charged as host work (SPEC-5.3.3); id 0,
/// an unknown id and an image over 8192 px are `E_RESOURCE`; the decoded
/// image reaches the pixels through the runtime's cache.
#[test]
fn spec_5_2_3_dl_image_fit_and_resources() {
    let red = png(2, 1, [255, 0, 0, 255]);
    let huge = png(limits::MAX_IMAGE_SIDE_PX + 1, 1, [0, 255, 0, 255]);
    let bad = b"not an image".to_vec();
    let red_len = red.len();
    let opts = Options {
        resources: vec![red, huge, bad],
        ..Options::default()
    };
    let bytes = image_module();
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    rt.init(4, 2, 1.0).unwrap();
    assert_eq!(rt.image_cache().decodes(), 0, "nothing decoded before use");

    // fit 3 with resource 1: E_ARG, no decode, no op.
    let dl = rt.frame_with_dt(13).unwrap().to_vec();
    assert_eq!(vy_dl::decode(&dl).unwrap().len(), 0);
    assert_eq!(host_logs(&rt), vec!["dl_image: E_ARG"]);
    assert_eq!(rt.image_cache().decodes(), 0);

    // fit 0..=2 with resource 1: decoded once, on the first call.
    for (i, fit_mode) in [0, 1, 2].into_iter().enumerate() {
        let dl = rt.frame_with_dt(10 + fit_mode).unwrap().to_vec();
        let ops = vy_dl::decode(&dl).unwrap();
        assert!(
            matches!(ops[0], vy_dl::Op::Image { resource_id: 1, fit, .. } if fit == fit_mode as u8),
            "{ops:?}"
        );
        assert_eq!(rt.image_cache().decodes(), 1);
        let expect_fuel = if i == 0 { red_len as u64 } else { 0 } + 20 + 1;
        assert_eq!(
            rt.last_frame().host_fuel,
            expect_fuel * FUEL_PER_HOST_BYTE,
            "resource bytes charged once, IMAGE op and END every frame"
        );
    }
    let images = rt.image_cache();
    let img =
        vy_render::render(rt.display_list(), 4, 2, 1.0, &mut TextEngine::new(), images).unwrap();
    assert_eq!(
        img.pixel(3, 1).unwrap(),
        [255, 0, 0, 255],
        "stretched over 4x2"
    );

    // Refusals: id 0 (the module), id 4 (none), id 2 (too large), id 3 (not a PNG).
    for (dt, log) in [
        (0, "dl_image: E_RESOURCE"),
        (40, "dl_image: E_RESOURCE"),
        (
            20,
            "dl_image: resource 2: 8193x1 px exceeds 8192 px per side (SPEC-6.5.1)",
        ),
        (30, "dl_image: resource 3: cannot decode image: "),
    ] {
        let before = rt.logs().len();
        let dl = rt.frame_with_dt(dt).unwrap().to_vec();
        assert_eq!(
            vy_dl::decode(&dl).unwrap().len(),
            0,
            "dt {dt}: nothing written"
        );
        let new = host_logs(&rt)[before..].to_vec();
        assert!(new[0].starts_with(log), "dt {dt}: {new:?}");
        assert_eq!(
            new.last().unwrap(),
            "dl_image: E_RESOURCE",
            "dt {dt}: {new:?}"
        );
    }
    assert_eq!(
        rt.image_cache().decodes(),
        3,
        "each resource decoded at most once"
    );
    assert!(!rt.is_suspended());
}

/// SPEC-8.2.1, SPEC-8.2.2: the headless host grants capabilities by their
/// six canonical names and nothing else; a granted `ime` makes `ime_begin`
/// succeed where it was `E_DENIED` (SPEC-5.2.6, SPEC-5.3.1).
#[test]
fn spec_8_2_1_capabilities_from_names() {
    let caps = Capabilities::from_names(["ime", "storage"]).unwrap();
    assert!(caps.ime && caps.storage && !caps.net && !caps.clipboard);
    assert!(caps.has(vy_abi::cap::IME) && !caps.has(vy_abi::cap::NET));
    assert!(!caps.has(0) && !caps.has(7));
    assert_eq!(
        Capabilities::from_names(["ime", "file"]),
        Err("file".into())
    );
    assert_eq!(
        Capabilities::from_names(vy_abi::cap::NAMES).unwrap(),
        Capabilities {
            net: true,
            storage: true,
            clipboard: true,
            hires_time: true,
            ime: true,
            persist_frame: true,
        }
    );
    assert_eq!(
        Capabilities::from_names([]).unwrap(),
        Capabilities::default()
    );

    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "ime_begin" (func $ime_begin (param f32 f32 f32 f32) (result i32)))
            (import "vy" "ime_end" (func $ime_end (result i32)))
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (drop (call $ime_begin (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1)))
                (drop (call $ime_end))
                i32.const 0)
            {STUBS})"#
    ));
    for (names, denied) in [(vec![], 2), (vec!["ime"], 0)] {
        let opts = Options {
            capabilities: Capabilities::from_names(names).unwrap(),
            ..Options::default()
        };
        let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
        rt.init(10, 10, 1.0).unwrap();
        rt.frame_with_dt(0).unwrap();
        let logs = host_logs(&rt);
        assert_eq!(logs.len(), denied, "{logs:?}");
        assert!(logs.iter().all(|l| l.contains("E_DENIED")));
    }
}

/// SPEC-5.2.1: exit stops the module; later calls report the code.
#[test]
fn spec_5_2_1_exit() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "exit" (func $exit (param i32)))
            (import "vy" "abi_version" (func $abi_version (result i32)))
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 1)) (then (call $exit (i32.const 42))))
                (call $abi_version))
            {STUBS})"#
    ));
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(0).unwrap();
    assert_eq!(rt.last_frame().ops_reported, vy_abi::ABI_VERSION);
    assert!(matches!(rt.frame_with_dt(1), Err(Error::Exited(42))));
    assert_eq!(rt.exit_code(), Some(42));
    assert!(matches!(rt.frame_with_dt(0), Err(Error::Exited(42))));
}

/// A module whose frame runs one import call selected by `dt_ms` and returns
/// the status code the import gave back.
fn status_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module
            (import "vy" "log" (func $log (param i32 i32 i32) (result i32)))
            (import "vy" "dl_begin" (func $dl_begin (result i32)))
            (import "vy" "dl_end" (func $dl_end (result i32)))
            (import "vy" "dl_rect" (func $dl_rect (param f32 f32 f32 f32 f32 i32) (result i32)))
            (import "vy" "dl_image" (func $dl_image (param f32 f32 f32 f32 i32 i32) (result i32)))
            (import "vy" "dl_path" (func $dl_path (param i32 i32 i32 f32) (result i32)))
            (import "vy" "dl_clip_push" (func $dl_clip_push (param f32 f32 f32 f32 f32) (result i32)))
            (import "vy" "dl_clip_pop" (func $dl_clip_pop (result i32)))
            (import "vy" "ime_begin" (func $ime_begin (param f32 f32 f32 f32) (result i32)))
            (import "vy" "ime_end" (func $ime_end (result i32)))
            (import "vy" "fetch_cancel" (func $fetch_cancel (param i32) (result i32)))
            (import "vy" "state_delete" (func $state_delete (param i32 i32) (result i32)))
            (import "vy" "navigate" (func $navigate (param i32 i32 i32) (result i32)))
            (import "vy" "link_register" (func $link_register (param i32 f32 f32 f32 f32 i32 i32) (result i32)))
            (import "vy" "random" (func $random (param i32 i32) (result i32)))
            (import "vy" "clipboard_write" (func $clipboard_write (param i32 i32) (result i32)))
            (import "vy" "sem_node" (func $sem_node (param i32 i32 i32 f32 f32 f32 f32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "vy://example.test/")
            (data (i32.const 32) "ftp://nej")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (local $r i32)
                (block $done
                    (if (i32.eq (local.get 0) (i32.const 1)) (then
                        (local.set $r (call $log (i32.const 1) (i32.const 0) (i32.const 18))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 2)) (then
                        (local.set $r (call $log (i32.const 1) (i32.const 0) (i32.const -1))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 3)) (then
                        (local.set $r (call $dl_rect (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (f32.const 0) (i32.const 0))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 4)) (then
                        (drop (call $dl_begin))
                        (local.set $r (call $dl_begin))
                        (drop (call $dl_end))
                        (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 5)) (then
                        (drop (call $dl_begin))
                        (local.set $r (call $dl_clip_pop))
                        (drop (call $dl_end))
                        (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 6)) (then
                        (drop (call $dl_begin))
                        (drop (call $dl_clip_push (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (f32.const 0)))
                        (local.set $r (call $dl_end))
                        (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 7)) (then
                        (drop (call $dl_begin))
                        (local.set $r (call $dl_image (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 1) (i32.const 0)))
                        (drop (call $dl_end))
                        (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 8)) (then
                        (drop (call $dl_begin))
                        (local.set $r (call $dl_path (i32.const 0) (i32.const 5) (i32.const 0) (f32.const 0)))
                        (drop (call $dl_end))
                        (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 9)) (then
                        (local.set $r (call $ime_begin (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 10)) (then
                        (local.set $r (call $ime_end)) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 11)) (then
                        (local.set $r (call $fetch_cancel (i32.const 1))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 12)) (then
                        (local.set $r (call $state_delete (i32.const 0) (i32.const 1))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 13)) (then
                        (local.set $r (call $navigate (i32.const 32) (i32.const 9) (i32.const 0))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 14)) (then
                        (local.set $r (call $navigate (i32.const 0) (i32.const 18) (i32.const 0))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 15)) (then
                        (local.set $r (call $link_register (i32.const 1) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const 18))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 16)) (then
                        (local.set $r (call $random (i32.const 0) (i32.const 8))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 17)) (then
                        (local.set $r (call $random (i32.const 65530) (i32.const 8))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 18)) (then
                        (local.set $r (call $clipboard_write (i32.const 0) (i32.const 18))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 19)) (then
                        (local.set $r (call $sem_node (i32.const 1) (i32.const 0) (i32.const 99) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const 0))) (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 20)) (then
                        (drop (call $sem_node (i32.const 7) (i32.const 0) (i32.const 1) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const 0)))
                        (local.set $r (call $sem_node (i32.const 7) (i32.const 0) (i32.const 2) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const 0)))
                        (br $done)))
                    (if (i32.eq (local.get 0) (i32.const 21)) (then
                        (drop (call $dl_begin))
                        (local.set $r (call $dl_end))
                        (br $done))))
                local.get $r)
            {STUBS})"#
    ))
}

/// SPEC-5.3.1, SPEC-5.3.2, SPEC-6.1.4, SPEC-5.2.1, SPEC-5.2.3, SPEC-5.2.6,
/// SPEC-5.2.7, SPEC-5.2.9, SPEC-5.2.10, SPEC-5.2.11, SPEC-5.2.12,
/// SPEC-5.2.13: every import that can refuse a call returns the status code
/// of appendix A to the module, the same code the host logs, and `OK` when
/// the call went through.
#[test]
fn spec_5_3_1_status_codes_reach_the_module() {
    let cases: [(i32, &str, i32); 21] = [
        (1, "log ok", err::OK),
        (2, "log negative len", err::E_ARG),
        (3, "dl_rect outside begin/end", err::E_DL_STATE),
        (4, "dl_begin while open", err::E_DL_STATE),
        (5, "dl_clip_pop without push", err::E_DL_CLIP),
        (6, "dl_end with an open clip", err::E_DL_CLIP),
        (7, "dl_image without resources", err::E_RESOURCE),
        (8, "dl_path with 5 bytes", err::E_ARG),
        (9, "ime_begin without ime", err::E_DENIED),
        (10, "ime_end without ime", err::E_DENIED),
        (11, "fetch_cancel without net", err::E_DENIED),
        (12, "state_delete without storage", err::E_DENIED),
        (13, "navigate ftp://", err::E_ARG),
        (14, "navigate headless", err::E_DENIED),
        (15, "link_register ok", err::OK),
        (16, "random ok", err::OK),
        (17, "random past memory", err::E_ARG),
        (18, "clipboard_write without clipboard", err::E_DENIED),
        (19, "sem_node bad role", err::E_ARG),
        (20, "sem_node duplicate id", err::E_ARG),
        (21, "dl_end ok", err::OK),
    ];
    let mut rt = load(&status_module());
    rt.init(10, 10, 1.0).unwrap();
    for (case, what, expect) in cases {
        let before = rt.logs().len();
        rt.frame_with_dt(case).unwrap();
        assert_eq!(rt.last_frame().ops_reported, expect, "case {case}: {what}");
        let logged = rt.logs()[before..]
            .iter()
            .filter(|l| l.source == LogSource::Host)
            .any(|l| l.message.contains(host::err_name(expect)));
        assert_eq!(
            logged,
            expect != err::OK,
            "case {case}: {what} logged as refused"
        );
    }
    assert!(!rt.is_suspended());
}

/// SPEC-5.2.13: a node id already registered in the same frame is `E_ARG`
/// and the second node is dropped; the next frame starts with a clean slate.
#[test]
fn spec_5_2_13_duplicate_node_id_refused() {
    let mut rt = load(&status_module());
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(20).unwrap();
    assert_eq!(rt.last_frame().ops_reported, err::E_ARG);
    let tree = rt.semantic_tree();
    assert_eq!(tree.len(), 1, "{tree:?}");
    assert_eq!(
        (tree[0].id, tree[0].role),
        (7, vy_abi::role::TEXT),
        "the first one stays"
    );
    assert_eq!(host_logs(&rt), vec!["sem_node: E_ARG"]);
    rt.frame_with_dt(20).unwrap();
    assert_eq!(
        rt.semantic_tree().len(),
        1,
        "a new frame, the same id is free again"
    );
}

/// A module that registers `a` links with ids from `b` in `vy_event` and
/// draws an empty frame in `vy_frame`.
fn links_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module
            (import "vy" "dl_begin" (func $dl_begin (result i32)))
            (import "vy" "dl_end" (func $dl_end (result i32)))
            (import "vy" "link_register" (func $link_register (param i32 f32 f32 f32 f32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "vy://example.test/")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (drop (call $dl_begin))
                (drop (call $link_register (i32.const 1) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const 18)))
                (drop (call $dl_end))
                i32.const 1)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (local $i i32)
                (local $r i32)
                (block (loop
                    (br_if 1 (i32.ge_s (local.get $i) (local.get 1)))
                    (local.set $r (call $link_register (i32.add (local.get 2) (local.get $i)) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const 18)))
                    (local.set $i (i32.add (local.get $i) (i32.const 1)))
                    (br 0)))
                local.get $r)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
    ))
}

/// SPEC-5.2.10, SPEC-9.3.2: links live per frame: `dl_begin` empties the
/// list, the same id in one frame replaces the earlier rectangle, and the
/// 4 097th link of a frame is `E_DL_FULL`.
#[test]
fn spec_5_2_10_links_per_frame() {
    let mut rt = load(&links_module());
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(0).unwrap();
    assert_eq!(rt.links().len(), 1, "the frame's own link");
    rt.event(0, 10, 100, 0, 0).unwrap();
    assert_eq!(rt.last_event_result(), err::OK);
    assert_eq!(rt.links().len(), 11, "links from events add to the frame's");
    rt.event(0, 1, 100, 0, 0).unwrap();
    assert_eq!(rt.links().len(), 11, "same id replaces, never duplicates");
    rt.frame_with_dt(0).unwrap();
    assert_eq!(rt.links().len(), 1, "dl_begin emptied the list");
    let cap = host::MAX_LINKS_PER_FRAME as i32;
    rt.event(0, cap - 1, 1000, 0, 0).unwrap();
    assert_eq!(rt.last_event_result(), err::OK);
    assert_eq!(rt.links().len(), cap as usize);
    rt.event(0, 1, 999_999, 0, 0).unwrap();
    assert_eq!(rt.last_event_result(), err::E_DL_FULL, "the 4 097th link");
    assert_eq!(rt.links().len(), cap as usize);
    assert_eq!(host_logs(&rt), vec!["link_register: E_DL_FULL"]);
    rt.frame_with_dt(0).unwrap();
    assert_eq!(rt.links().len(), 1, "and the next frame is free again");
}

/// SPEC-2.1.6, SPEC-4.1.4: a trapping module is reported, not interpreted around.
#[test]
fn spec_4_1_4_trap_is_reported() {
    let bytes = wasm(&format!(
        r#"(module
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) unreachable)
            {STUBS})"#
    ));
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    assert!(matches!(rt.frame_with_dt(0), Err(Error::Trap(_))));
    assert!(matches!(
        Runtime::load(b"garbage", &sha256(b"garbage"), Options::default()),
        Err(Error::Invalid(_))
    ));
}
