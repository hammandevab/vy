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
