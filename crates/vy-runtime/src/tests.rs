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
    let mut store = Store::new(engine(), Host::new(limits, Capabilities::default()));
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
    assert!(matches!(rt.frame(0), Err(Error::NotInitialized)));
    rt.init(100, 100, 1.0).unwrap();
    assert!(rt.wants_frame(), "init is a window change");
    rt.frame(0).unwrap();
    assert!(!rt.wants_frame(), "frame consumed the request");
    rt.frame(1).unwrap();
    assert!(rt.wants_frame(), "request_frame during the frame");
    rt.frame(0).unwrap();
    assert!(!rt.wants_frame());
    assert!(!rt.event(event::POINTER_MOVE, 0, 0, 0, 0).unwrap());
    assert!(!rt.wants_frame(), "event returning 0 wants nothing");
    assert!(rt.event(event::POINTER_MOVE, 1, 0, 0, 0).unwrap());
    assert!(rt.wants_frame(), "event returning 1 wants a frame");
    rt.frame(0).unwrap();
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
            (import "vy" "navigate" (func $navigate (param i32 i32 i32)))
            (import "vy" "dl_begin" (func $dl_begin))
            (import "vy" "dl_end" (func $dl_end))
            (import "vy" "dl_rect" (func $dl_rect (param f32 f32 f32 f32 f32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "vy://example.test/")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (local $sum i32)
                (local.set $sum (call $fetch_begin (i32.const 0) (i32.const 0) (i32.const 18) (i32.const 0) (i32.const 0)))
                (local.set $sum (i32.add (local.get $sum) (call $state_put (i32.const 0) (i32.const 1) (i32.const 0) (i32.const 1))))
                (local.set $sum (i32.add (local.get $sum) (call $clipboard_read (i32.const 0) (i32.const 16))))
                (call $navigate (i32.const 0) (i32.const 18) (i32.const 0))
                (call $dl_begin)
                (call $dl_rect (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (f32.const 0) (i32.const 0))
                (call $dl_end)
                local.get $sum)
            {STUBS})"#
    ));
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    for _ in 0..2 {
        let dl = rt.frame(16).unwrap().to_vec();
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
            (import "vy" "log" (func $log (param i32 i32 i32)))
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
                    (call $log (i32.const 0) (i32.const 0) (i32.const 6))
                    (if (i32.eq (local.get 3) (i32.const 1)) (then
                        (call $log (i32.const 0) (i32.const 32) (i32.const 5))))))
                i32.const 0)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32)
                (call $log (i32.const 0) (i32.const 16) (i32.const 7))
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
    rt.frame(0).unwrap();
    assert!(!rt.last_frame().over_budget);
    assert!(rt.last_frame().fuel < FRAME_BUDGET_FUEL);

    rt.frame(slow).unwrap();
    assert!(rt.last_frame().over_budget, "{:?}", rt.last_frame());
    assert_eq!(module_logs(&rt), vec!["budget"], "first breach: a warning");
    rt.frame(0).unwrap();
    assert!(!rt.last_frame().over_budget);
    rt.frame(slow).unwrap();
    assert_eq!(
        module_logs(&rt),
        vec!["budget", "budget"],
        "streak was reset"
    );
    assert!(!rt.is_suspended());

    rt.frame(slow).unwrap();
    assert_eq!(
        module_logs(&rt),
        vec!["budget", "budget", "budget", "final", "suspend"],
        "second consecutive breach: final warning then forced suspend"
    );
    assert!(rt.is_suspended());
    assert!(matches!(rt.frame(0), Err(Error::Suspended)));

    // Hard limit: the frame is aborted and counts as a breach.
    let mut rt = load(&budget_module());
    rt.init(10, 10, 1.0).unwrap();
    let e = rt.frame(-1).unwrap_err();
    assert!(
        matches!(e, Error::OverBudget { used_ms } if used_ms >= 8 * 8),
        "{e}"
    );
    assert_eq!(module_logs(&rt), vec!["budget"]);
    assert!(matches!(rt.frame(-1), Err(Error::OverBudget { .. })));
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
            (import "vy" "log" (func $log (param i32 i32 i32)))
            (import "vy" "text_style" (func $text_style (param i32 f32 i32 i32) (result i32)))
            (import "vy" "text_measure" (func $text_measure (param i32 f32 i32 i32) (result i64)))
            (import "vy" "random" (func $random (param i32 i32)))
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
                    (call $random (i32.const 131072) (i32.sub (i32.const 0) (local.get 0)))
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
                    (call $log (i32.const 0) (i32.const 65536) (i32.const 6))))
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
    rt.frame(1).unwrap();
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

    rt.frame(over).unwrap();
    let four = rt.last_frame();
    assert!(four.over_budget, "{four:?}");
    assert_eq!(four.host_fuel, u64::from(over as u32) * per_measure);
    assert_eq!(module_logs(&rt), vec!["budget"], "KIND_BUDGET delivered");
    rt.frame(over).unwrap();
    assert!(
        rt.is_suspended(),
        "second consecutive breach: forced suspend"
    );

    // The hard limit: 2 MB of random bytes is 64 M fuel, so the instruction
    // after the import traps with OutOfFuel and the frame is aborted.
    let bytes = (CALL_HARD_LIMIT_FUEL / FUEL_PER_HOST_BYTE) as i32;
    let mut rt = load(&host_work_module());
    rt.init(10, 10, 1.0).unwrap();
    rt.frame(-(bytes / 16)).unwrap();
    assert!(!rt.last_frame().over_budget, "{:?}", rt.last_frame());
    assert_eq!(rt.last_frame().host_fuel, CALL_HARD_LIMIT_FUEL / 16);
    let r = rt.frame(-bytes).map(|_| ());
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
    rt.frame(0).unwrap();
    assert_eq!(rt.frames(), 1);
    assert_eq!(rt.now_ms(), host::HEADLESS_FRAME_MS);
    assert!(rt.wants_frame());
}

fn drawing_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module
            (import "vy" "dl_begin" (func $dl_begin))
            (import "vy" "dl_end" (func $dl_end))
            (import "vy" "dl_rect" (func $dl_rect (param f32 f32 f32 f32 f32 i32)))
            (import "vy" "dl_text" (func $dl_text (param f32 f32 f32 i32 i32 i32) (result i32)))
            (import "vy" "text_style" (func $text_style (param i32 f32 i32 i32) (result i32)))
            (import "vy" "random" (func $random (param i32 i32)))
            (import "vy" "now_ms" (func $now_ms (result i64)))
            (memory (export "memory") 1)
            (global $style (mut i32) (i32.const 0))
            (data (i32.const 0) "Hej från Vy")
            (func (export "vy_init") (param i32 i32 i32 f32) (result i32)
                (global.set $style (call $text_style (i32.const -1) (f32.const 18) (i32.const 700) (i32.const 0x202020FF)))
                i32.const 0)
            (func (export "vy_frame") (param i32) (result i32)
                (call $random (i32.const 64) (i32.const 4))
                (call $dl_begin)
                (call $dl_rect (f32.const 0) (f32.const 0) (f32.const 120) (f32.const 60) (f32.const 0) (i32.const 0xF0F0F0FF))
                ;; a rectangle placed by the random source and the clock
                (call $dl_rect
                    (f32.convert_i32_u (i32.rem_u (i32.load8_u (i32.const 64)) (i32.const 40)))
                    (f32.convert_i64_s (i64.rem_s (call $now_ms) (i64.const 40)))
                    (f32.const 10) (f32.const 10) (f32.const 2) (i32.const 0xFF0000FF))
                (drop (call $dl_text (f32.const 4) (f32.const 30) (f32.const 110) (global.get $style) (i32.const 0) (i32.const 12)))
                (call $dl_end)
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
            dls.push(rt.frame(16).unwrap().to_vec());
        }
        let img = vy_render::render(&dls[2], 120, 60, 1.0, rt.text_engine_mut()).unwrap();
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
    rt.frame(1).unwrap();
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
    rt.frame(0).unwrap();
    assert_eq!(rt.last_frame().ops_reported, 3);
}

/// SPEC-5.2.4, SPEC-6.4.3: dl_text returns the layout height times 100, and text_measure
/// packs width and height the same way (SPEC-5.2.5).
#[test]
fn spec_5_2_4_dl_text_height() {
    let bytes = wasm(
        r#"(module
            (import "vy" "dl_begin" (func $dl_begin))
            (import "vy" "dl_end" (func $dl_end))
            (import "vy" "dl_text" (func $dl_text (param f32 f32 f32 i32 i32 i32) (result i32)))
            (import "vy" "text_style" (func $text_style (param i32 f32 i32 i32) (result i32)))
            (import "vy" "text_measure" (func $text_measure (param i32 f32 i32 i32) (result i64)))
            (memory (export "memory") 1)
            (global $style (mut i32) (i32.const 0))
            (data (i32.const 0) "ett två tre fyra fem sex sju åtta nio tio")
            (func (export "vy_init") (param i32 i32 i32 f32) (result i32)
                (global.set $style (call $text_style (i32.const -1) (f32.const 16) (i32.const 400) (i32.const 0)))
                i32.const 0)
            ;; dt_ms is used as the wrap width
            (func (export "vy_frame") (param i32) (result i32)
                (local $h i32)
                (call $dl_begin)
                (local.set $h (call $dl_text (f32.const 0) (f32.const 0) (f32.convert_i32_s (local.get 0)) (global.get $style) (i32.const 0) (i32.const 46)))
                (call $dl_end)
                local.get $h)
            ;; returns the low 32 bits of text_measure: height * 100
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (i32.wrap_i64 (call $text_measure (global.get $style) (f32.convert_i32_s (local.get 1)) (i32.const 0) (i32.const 46))))
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
    );
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    rt.frame(1000).unwrap();
    let one_line = rt.last_frame().ops_reported;
    assert_eq!(one_line, 2000, "16 px * 1.25 line height * 100");
    rt.frame(100).unwrap();
    let wrapped = rt.last_frame().ops_reported;
    assert!(wrapped > one_line, "{wrapped} > {one_line}");
    assert_eq!(wrapped % 2000, 0);
    let dl = rt.frame(100).unwrap().to_vec();
    let ops = vy_dl::decode(&dl).unwrap();
    assert!(matches!(ops[0], vy_dl::Op::Text { max_w, .. } if max_w == 100.0));
    // text_measure's low 32 bits are the same height (SPEC-5.2.5); vy_event here
    // returns them, so "redraw" is reported exactly when they equal 1, which they never do.
    assert!(!rt.event(0, 100, 0, 0, 0).unwrap());
    let m = rt
        .text_engine_mut()
        .measure(1, 100.0, "ett två tre fyra fem sex sju åtta nio tio")
        .unwrap();
    assert_eq!(vy_render::height_x100(m.h), wrapped);
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
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    rt.init(10, 10, 1.0).unwrap();
    assert_eq!(rt.memory_pages(), 1);
    rt.frame(2).unwrap();
    assert_eq!(rt.last_frame().ops_reported, 1, "grow returns the old size");
    assert_eq!(rt.memory_pages(), 3);
    rt.frame(2).unwrap();
    assert_eq!(
        rt.last_frame().ops_reported,
        -1,
        "3 + 2 > 4 pages is refused"
    );
    assert_eq!(rt.memory_pages(), 3);
    rt.frame(1).unwrap();
    assert_eq!(rt.memory_pages(), 4);
    rt.frame(1).unwrap();
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
            (import "vy" "log" (func $log (param i32 i32 i32)))
            (import "vy" "dl_begin" (func $dl_begin))
            (import "vy" "dl_end" (func $dl_end))
            (import "vy" "dl_text" (func $dl_text (param f32 f32 f32 i32 i32 i32) (result i32)))
            (import "vy" "dl_path" (func $dl_path (param i32 i32 i32 f32)))
            (import "vy" "text_style" (func $text_style (param i32 f32 i32 i32) (result i32)))
            (import "vy" "text_measure" (func $text_measure (param i32 f32 i32 i32) (result i64)))
            (import "vy" "text_caret" (func $text_caret (param i32 i32 i32 i32) (result i64)))
            (import "vy" "sem_node" (func $sem_node (param i32 i32 i32 f32 f32 f32 f32 i32 i32)))
            (import "vy" "link_register" (func $link_register (param i32 f32 f32 f32 f32 i32 i32)))
            (import "vy" "navigate" (func $navigate (param i32 i32 i32)))
            (import "vy" "fetch_begin" (func $fetch_begin (param i32 i32 i32 i32 i32) (result i32)))
            (import "vy" "state_put" (func $state_put (param i32 i32 i32 i32) (result i32)))
            (import "vy" "state_get" (func $state_get (param i32 i32 i32 i32) (result i32)))
            (import "vy" "state_delete" (func $state_delete (param i32 i32)))
            (import "vy" "clipboard_write" (func $clipboard_write (param i32 i32)))
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
                        (call $log (i32.const 0) (i32.const 0) (i32.const {log1}))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 2)) (then
                        (call $dl_begin)
                        (local.set $r (call $dl_text (f32.const 0) (f32.const 0) (f32.const 0) (global.get $style) (i32.const 0) (i32.const {text1})))
                        (call $dl_end)
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 3)) (then
                        (local.set $r (i32.wrap_i64 (call $text_measure (global.get $style) (f32.const 0) (i32.const 0) (i32.const {text1}))))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 4)) (then
                        (local.set $r (i32.wrap_i64 (call $text_caret (global.get $style) (i32.const 0) (i32.const {text1}) (i32.const 0))))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 5)) (then
                        (call $dl_begin)
                        (call $dl_path (i32.const 0) (i32.const {path1}) (i32.const 0) (f32.const 0))
                        (call $dl_end)
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 6)) (then
                        (call $sem_node (i32.const 1) (i32.const 0) (i32.const 1) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const {sem1}))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 7)) (then
                        (call $link_register (i32.const 1) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const {url1}))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 8)) (then
                        (call $navigate (i32.const 0) (i32.const {url1}) (i32.const 0))
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
                        (call $state_delete (i32.const 0) (i32.const {key1}))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 15)) (then
                        (call $clipboard_write (i32.const 0) (i32.const {clip1}))
                        (br $done)))
                    (if (i32.eq (local.get $n) (i32.const 20)) (then
                        (call $log (i32.const 0) (i32.const 0) (i32.const {log0}))
                        (call $dl_begin)
                        (local.set $r (call $dl_text (f32.const 0) (f32.const 0) (f32.const 0) (global.get $style) (i32.const 0) (i32.const {text0})))
                        (call $dl_end)
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
            rt.frame(case).unwrap();
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
    rt.frame(20).unwrap();
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
/// kind 0 adds `a` nodes with `b` bytes of text each, kind 1 draws two text
/// runs of `a` and `b` bytes and returns the status of the first.
fn frame_caps_module() -> Vec<u8> {
    let text = "a".repeat(limits::MAX_SEM_TEXT_BYTES as usize);
    wasm(&format!(
        r#"(module
            (import "vy" "dl_begin" (func $dl_begin))
            (import "vy" "dl_end" (func $dl_end))
            (import "vy" "dl_text" (func $dl_text (param f32 f32 f32 i32 i32 i32) (result i32)))
            (import "vy" "text_style" (func $text_style (param i32 f32 i32 i32) (result i32)))
            (import "vy" "sem_node" (func $sem_node (param i32 i32 i32 f32 f32 f32 f32 i32 i32)))
            (memory (export "memory") 1)
            (global $style (mut i32) (i32.const 0))
            (data (i32.const 0) "{text}")
            (func (export "vy_init") (param i32 i32 i32 f32) (result i32)
                (global.set $style (call $text_style (i32.const -1) (f32.const 16) (i32.const 400) (i32.const 0)))
                i32.const 0)
            (func (export "vy_frame") (param i32) (result i32) i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (local $i i32)
                (local $r i32)
                (if (i32.eq (local.get 0) (i32.const 0)) (then
                    (block (loop
                        (br_if 1 (i32.ge_s (local.get $i) (local.get 1)))
                        (call $sem_node (local.get $i) (i32.const 0) (i32.const 1) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (local.get 2))
                        (local.set $i (i32.add (local.get $i) (i32.const 1)))
                        (br 0)))))
                (if (i32.eq (local.get 0) (i32.const 1)) (then
                    (call $dl_begin)
                    (local.set $r (call $dl_text (f32.const 0) (f32.const 0) (f32.const 0) (global.get $style) (i32.const 0) (local.get 1)))
                    (drop (call $dl_text (f32.const 0) (f32.const 0) (f32.const 0) (global.get $style) (i32.const 0) (local.get 2)))
                    (call $dl_end)))
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
    rt.frame(0).unwrap();
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
    rt.frame(0).unwrap();
    rt.event(1, 1, 1, 0, 0).unwrap();
    assert_eq!(vy_dl::decode(rt.display_list()).unwrap().len(), 2);
    assert_eq!(host_logs(&rt).len(), 2, "no new complaint");
}

/// SPEC-9.4.1, SPEC-2.1.2: a module whose hash mismatches is never run, or even parsed.
#[test]
fn spec_9_4_1_hash_mismatch_never_runs() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "log" (func $log (param i32 i32 i32)))
            (memory (export "memory") 1)
            (start $boom)
            (func $boom (call $log (i32.const 3) (i32.const 0) (i32.const 0)))
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
            (import "vy" "sem_node" (func $sem_node (param i32 i32 i32 f32 f32 f32 f32 i32 i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "Hej \"där\"")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (call $sem_node (i32.const 1) (i32.const 0) (i32.const 15) (f32.const 0) (f32.const 0) (f32.const 100) (f32.const 50) (i32.const 0) (i32.const 0))
                (call $sem_node (i32.const 2) (i32.const 1) (i32.const 2) (f32.const 4) (f32.const 4.5) (f32.const 90) (f32.const 20) (i32.const 0) (i32.const 10))
                (call $sem_node (i32.const 3) (i32.const 1) (i32.const 25) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const 0))
                (call $sem_node (i32.const 4) (i32.const 1) (i32.const 0) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 0) (i32.const 0))
                (call $sem_node (i32.const 5) (i32.const 1) (i32.const 1) (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (i32.const 65530) (i32.const 100))
                i32.const 0)
            {STUBS})"#
    ));
    let mut rt = load(&bytes);
    rt.init(100, 50, 1.0).unwrap();
    assert!(rt.semantic_tree().is_empty());
    for _ in 0..2 {
        rt.frame(16).unwrap();
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
        rt.frame(16).unwrap();
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
            (import "vy" "log" (func $log (param i32 i32 i32)))
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 7)) (then
                    (call $log (i32.const 0) (local.get 1) (local.get 2))))
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
    rt.frame(0).unwrap();
    assert_eq!(rt.last_frame().ops_reported, vy_abi::ABI_VERSION);
    assert!(matches!(rt.frame(1), Err(Error::Exited(42))));
    assert_eq!(rt.exit_code(), Some(42));
    assert!(matches!(rt.frame(0), Err(Error::Exited(42))));
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
    assert!(matches!(rt.frame(0), Err(Error::Trap(_))));
    assert!(matches!(
        Runtime::load(b"garbage", &sha256(b"garbage"), Options::default()),
        Err(Error::Invalid(_))
    ));
}
