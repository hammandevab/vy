//! Runtime tests. Every module here is built from WAT so the tests do not
//! depend on `modules/hello` being compiled first.

use super::*;
use host::LogSource;
use vy_abi::{err, key, modifier};

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

/// The host's log lines, without the one note about a `vy_event` result
/// other than 0 or 1 (SPEC-5.1.3): the test modules here use that return
/// value as a channel out of the module, so nearly every test would carry
/// the note; `spec_5_1_3_event_result_other_than_0_or_1` checks it alone.
fn host_logs(rt: &Runtime) -> Vec<String> {
    rt.logs()
        .iter()
        .filter(|l| l.source == LogSource::Host && !l.message.starts_with("vy_event returned "))
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
    let mut store = Store::new(engine(), Host::new(limits, Options::default()));
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

/// A module driven by `vy_event`: kind 200 calls `fetch_begin` with URL
/// `a` (0 the listed API, 1 a foreign origin, 2 not https) and returns
/// the result; 201 returns `fetch_poll(a)`; 202 returns `fetch_read(a,
/// 2048, b)`; 203 returns `fetch_cancel(a)`; 204 calls `fetch_begin` on
/// the listed API with method `a` and returns the result. Every
/// `FETCH_DONE` is recorded as three `i32`s at 4096; kind 100 returns
/// field `a` of the records, 101 clears them, 102 returns the count.
/// `FETCH_DONE` answers 1.
fn fetch_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module
            (import "vy" "fetch_begin" (func $fetch_begin (param i32 i32 i32 i32 i32) (result i32)))
            (import "vy" "fetch_poll" (func $fetch_poll (param i32) (result i32)))
            (import "vy" "fetch_read" (func $fetch_read (param i32 i32 i32) (result i32)))
            (import "vy" "fetch_cancel" (func $fetch_cancel (param i32) (result i32)))
            (memory (export "memory") 1)
            (global $n (mut i32) (i32.const 0))
            (data (i32.const 0) "https://chat.example/api/messages")
            (data (i32.const 64) "https://evil.example/x")
            (data (i32.const 128) "ftp://x")
            (data (i32.const 256) "{{\"text\":\"hej\"}}")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) i32.const 0)
            (func $begin (param $which i32) (result i32)
                (if (i32.eq (local.get $which) (i32.const 1)) (then
                    (return (call $fetch_begin (i32.const 1) (i32.const 64) (i32.const 22) (i32.const 256) (i32.const 14)))))
                (if (i32.eq (local.get $which) (i32.const 2)) (then
                    (return (call $fetch_begin (i32.const 1) (i32.const 128) (i32.const 7) (i32.const 256) (i32.const 14)))))
                (call $fetch_begin (i32.const 1) (i32.const 0) (i32.const 33) (i32.const 256) (i32.const 14)))
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (local $p i32)
                (if (i32.eq (local.get 0) (i32.const 200)) (then (return (call $begin (local.get 1)))))
                (if (i32.eq (local.get 0) (i32.const 201)) (then (return (call $fetch_poll (local.get 1)))))
                (if (i32.eq (local.get 0) (i32.const 202)) (then
                    (return (call $fetch_read (local.get 1) (i32.const 2048) (local.get 2)))))
                (if (i32.eq (local.get 0) (i32.const 203)) (then (return (call $fetch_cancel (local.get 1)))))
                (if (i32.eq (local.get 0) (i32.const 204)) (then
                    (return (call $fetch_begin (local.get 1) (i32.const 0) (i32.const 33) (i32.const 256) (i32.const 14)))))
                (if (i32.eq (local.get 0) (i32.const 100)) (then
                    (return (i32.load (i32.add (i32.const 4096) (i32.mul (local.get 1) (i32.const 4)))))))
                (if (i32.eq (local.get 0) (i32.const 101)) (then
                    (global.set $n (i32.const 0))
                    (return (i32.const 0))))
                (if (i32.eq (local.get 0) (i32.const 102)) (then (return (global.get $n))))
                (if (i32.eq (local.get 0) (i32.const 20)) (then
                    (local.set $p (i32.add (i32.const 4096) (i32.mul (global.get $n) (i32.const 12))))
                    (i32.store (local.get $p) (local.get 1))
                    (i32.store (i32.add (local.get $p) (i32.const 4)) (local.get 2))
                    (i32.store (i32.add (local.get $p) (i32.const 8)) (local.get 3))
                    (global.set $n (i32.add (global.get $n) (i32.const 1)))
                    (return (i32.const 1))))
                i32.const 0)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
    ))
}

/// The `FETCH_DONE` events `fetch_module` recorded since the last call, as
/// `(req_id, status, bytes)`.
fn fetch_done_events(rt: &mut Runtime) -> Vec<(i32, i32, i32)> {
    rt.event(102, 0, 0, 0, 0).unwrap();
    let n = rt.last_event_result();
    let field = |rt: &mut Runtime, i: i32| {
        rt.event(100, i, 0, 0, 0).unwrap();
        rt.last_event_result()
    };
    let out = (0..n)
        .map(|i| (field(rt, i * 3), field(rt, i * 3 + 1), field(rt, i * 3 + 2)))
        .collect();
    rt.event(101, 0, 0, 0, 0).unwrap();
    out
}

fn fetch_runtime(canned: Vec<CannedResponse>, origins: &[&str]) -> Runtime {
    let bytes = fetch_module();
    let opts = Options {
        capabilities: Capabilities::from_names(["net"]).unwrap(),
        fetch: canned,
        net_origins: origins.iter().map(|o| o.to_string()).collect(),
        ..Options::default()
    };
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    rt.init(10, 10, 1.0).unwrap();
    rt
}

const API: &str = "https://chat.example/api/messages";

/// SPEC-5.2.7, SPEC-A.2.1: the host buffers the whole response before
/// `FETCH_DONE`, which carries the status and the body's length;
/// `fetch_read` copies the buffered body in order and returns 0 only at
/// its end, which closes the request and frees the body; `fetch_cancel`
/// closes it early; a request is not readable before its `FETCH_DONE`;
/// the 65th open request is `E_RATE`; a body over 64 MB is `E_TOO_LARGE`
/// in `FETCH_DONE` with no body; `fetch_poll` is 0 until `FETCH_DONE`.
#[test]
fn spec_5_2_7_fetch_read_zero_only_at_end() {
    let canned = CannedResponse {
        url: API.into(),
        status: 200,
        body: b"hello world!".to_vec(),
    };
    let mut rt = fetch_runtime(vec![canned.clone()], &["https://chat.example"]);
    let call = |rt: &mut Runtime, kind: i32, a: i32, b: i32| {
        rt.event(kind, a, b, 0, 0).unwrap();
        rt.last_event_result()
    };
    assert_eq!(call(&mut rt, 200, 0, 0), 1, "first request id");
    assert_eq!((rt.open_fetches(), rt.pending_fetches()), (1, 1));
    assert_eq!(call(&mut rt, 201, 1, 0), 0, "poll: on its way");
    assert_eq!(
        call(&mut rt, 202, 1, 64),
        err::E_ARG,
        "not readable before FETCH_DONE"
    );
    assert_eq!(fetch_done_events(&mut rt), vec![]);

    assert_eq!(rt.fetch_done().unwrap(), Some(true), "the module redraws");
    assert_eq!(fetch_done_events(&mut rt), vec![(1, 200, 12)]);
    assert_eq!((rt.open_fetches(), rt.pending_fetches()), (1, 0));
    assert_eq!(call(&mut rt, 201, 1, 0), 1, "poll: body available");
    assert_eq!(rt.fetch_done().unwrap(), None, "nothing else waiting");

    assert_eq!(call(&mut rt, 202, 1, 5), 5);
    assert_eq!(rt.read_memory(2048, 5), b"hello");
    assert_eq!(call(&mut rt, 202, 1, 5), 5);
    assert_eq!(rt.read_memory(2048, 5), b" worl");
    assert_eq!(call(&mut rt, 202, 1, 5), 2, "the rest, not a full chunk");
    assert_eq!(rt.read_memory(2048, 2), b"d!");
    assert_eq!(rt.open_fetches(), 1, "still open until the 0");
    assert_eq!(call(&mut rt, 202, 1, 5), 0, "the end");
    assert_eq!(rt.open_fetches(), 0, "closed and freed by the 0");
    assert_eq!(call(&mut rt, 202, 1, 5), err::E_ARG, "no such request now");
    assert_eq!(call(&mut rt, 203, 1, 0), err::E_ARG);
    assert_eq!(
        call(&mut rt, 201, 1, 0),
        err::E_ARG,
        "poll: 0 pending, 1 done, else a negative code"
    );
    assert_eq!(call(&mut rt, 202, 1, 0), err::E_ARG, "max must be positive");

    // Cancel before FETCH_DONE: nothing is delivered for it afterwards.
    assert_eq!(call(&mut rt, 200, 0, 0), 2);
    assert_eq!(call(&mut rt, 203, 2, 0), err::OK);
    assert_eq!(rt.open_fetches(), 0);
    assert_eq!(rt.fetch_done().unwrap(), None);
    assert_eq!(fetch_done_events(&mut rt), vec![]);

    // 64 open requests at once; the 65th is E_RATE (SPEC-9.3.1).
    for i in 0..limits::MAX_OPEN_FETCHES as i32 {
        assert_eq!(call(&mut rt, 200, 0, 0), 3 + i);
    }
    assert_eq!(call(&mut rt, 200, 0, 0), err::E_RATE);
    assert_eq!(call(&mut rt, 203, 3, 0), err::OK, "closing one makes room");
    assert_eq!(
        call(&mut rt, 200, 0, 0),
        3 + limits::MAX_OPEN_FETCHES as i32
    );
    assert!(host_logs(&rt).iter().any(|l| l == "fetch_begin: E_RATE"));

    // FETCH_DONE goes to the oldest waiting request first.
    assert_eq!(rt.fetch_done().unwrap(), Some(true));
    assert_eq!(fetch_done_events(&mut rt), vec![(4, 200, 12)]);

    // A body over 64 MB: E_TOO_LARGE, no body, request closed.
    let huge = CannedResponse {
        url: API.into(),
        status: 200,
        body: vec![0; limits::MAX_FETCH_RESPONSE_BYTES as usize + 1],
    };
    let mut rt = fetch_runtime(vec![huge], &[]);
    assert_eq!(call(&mut rt, 200, 0, 0), 1);
    assert_eq!(rt.fetch_done().unwrap(), Some(true));
    assert_eq!(fetch_done_events(&mut rt), vec![(1, err::E_TOO_LARGE, 0)]);
    assert_eq!(rt.open_fetches(), 0);
    assert_eq!(call(&mut rt, 202, 1, 5), err::E_ARG);
    assert!(host_logs(&rt)
        .iter()
        .any(|l| l.starts_with("fetch 1: E_TOO_LARGE")));

    // An empty body: the first read is already the end.
    let empty = CannedResponse {
        url: API.into(),
        status: 204,
        body: Vec::new(),
    };
    let mut rt = fetch_runtime(vec![empty], &[]);
    assert_eq!(call(&mut rt, 200, 0, 0), 1);
    rt.fetch_done().unwrap();
    assert_eq!(fetch_done_events(&mut rt), vec![(1, 204, 0)]);
    assert_eq!(call(&mut rt, 202, 1, 5), 0);
    assert_eq!(rt.open_fetches(), 0);
}

/// SPEC-5.2.8, SPEC-9.5.1: `fetch_begin` to an origin the manifest does
/// not list is `E_ORIGIN` with no request made, even with a canned
/// response for it; origins compare as scheme, host and port; a URL that
/// is not `https://` is `E_ARG`; with no manifest (no origins) the gate is
/// not applied.
#[test]
fn spec_5_2_8_origin_gate() {
    let canned = |url: &str| CannedResponse {
        url: url.into(),
        status: 200,
        body: b"ok".to_vec(),
    };
    let responses = vec![canned(API), canned("https://evil.example/x")];
    let call = |rt: &mut Runtime, which: i32| {
        rt.event(200, which, 0, 0, 0).unwrap();
        rt.last_event_result()
    };
    let mut rt = fetch_runtime(responses.clone(), &["https://chat.example"]);
    assert_eq!(call(&mut rt, 0), 1, "listed origin");
    assert_eq!(call(&mut rt, 1), err::E_ORIGIN, "foreign origin");
    assert_eq!(rt.open_fetches(), 1, "no request was made for it");
    assert!(host_logs(&rt)
        .iter()
        .any(|l| l.starts_with("fetch_begin: E_ORIGIN (https://evil.example")));
    assert_eq!(
        call(&mut rt, 2),
        err::E_ARG,
        "ftp:// is not a fetchable URL"
    );

    let mut rt = fetch_runtime(
        responses.clone(),
        &["https://CHAT.example:443/ignored/path"],
    );
    assert_eq!(
        call(&mut rt, 0),
        err::E_ORIGIN,
        "the port is part of the origin"
    );
    let mut rt = fetch_runtime(responses.clone(), &["https://Chat.Example"]);
    assert_eq!(call(&mut rt, 0), 1, "hosts compare case-insensitively");
    let mut rt = fetch_runtime(responses, &[]);
    assert_eq!(call(&mut rt, 1), 1, "no manifest, no gate");

    assert_eq!(
        origin_of("https://Chat.example:8443/a?b#c").as_deref(),
        Some("https://chat.example:8443")
    );
    assert_eq!(
        origin_of("https://chat.example").as_deref(),
        Some("https://chat.example")
    );
    assert_eq!(origin_of("http://chat.example/"), None);
    assert_eq!(origin_of("https:///x"), None);
}

/// SPEC-5.2.7: `method` is 0 GET, 1 POST, 2 PUT or 3 DELETE; any other
/// value is `E_ARG`, logged, and no request is opened. The capability gate
/// still comes first (SPEC-5.3.1).
#[test]
fn spec_5_2_7_fetch_method_values() {
    let canned = CannedResponse {
        url: API.into(),
        status: 200,
        body: b"ok".to_vec(),
    };
    let mut rt = fetch_runtime(vec![canned], &["https://chat.example"]);
    let begin = |rt: &mut Runtime, method: i32| {
        rt.event(204, method, 0, 0, 0).unwrap();
        rt.last_event_result()
    };
    for (m, id) in [
        (vy_abi::method::GET, 1),
        (vy_abi::method::POST, 2),
        (vy_abi::method::PUT, 3),
        (vy_abi::method::DELETE, 4),
    ] {
        assert_eq!(begin(&mut rt, m), id, "method {m} opens a request");
    }
    assert_eq!(rt.open_fetches(), 4);
    for bad in [4, -1, 7, i32::MAX] {
        assert_eq!(begin(&mut rt, bad), err::E_ARG, "method {bad}");
    }
    assert_eq!(rt.open_fetches(), 4, "a refused method opens nothing");
    let refused: Vec<String> = host_logs(&rt)
        .into_iter()
        .filter(|l| l.starts_with("fetch_begin: E_ARG (method"))
        .collect();
    assert_eq!(refused.len(), 4, "{refused:?}");
    assert!(refused[0].contains("method 4 is not 0 GET, 1 POST, 2 PUT or 3 DELETE"));

    // Without `net` the method is never looked at: E_DENIED (SPEC-5.3.1).
    let bytes = fetch_module();
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), Options::default()).unwrap();
    rt.init(10, 10, 1.0).unwrap();
    assert_eq!(begin(&mut rt, 99), err::E_DENIED);
}

/// SPEC-5.2.1: `log.level` is 0 debug, 1 info, 2 warn, 3 error; the host
/// records any other value as 3 and the call still succeeds.
#[test]
fn spec_5_2_1_log_level_clamps() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "log" (func $log (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "lvl")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (call $log (local.get 0) (i32.const 0) (i32.const 3)))
            {STUBS})"#
    ));
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    for lvl in [0, 1, 2, 3, 4, -1, 1000] {
        rt.frame_with_dt(lvl).unwrap();
        assert_eq!(rt.last_frame().ops_reported, err::OK, "level {lvl}");
    }
    let levels: Vec<i32> = rt
        .logs()
        .iter()
        .filter(|l| l.source == LogSource::Module)
        .map(|l| l.level)
        .collect();
    assert_eq!(levels, [0, 1, 2, 3, 3, 3, 3]);
    assert_eq!(vy_abi::level::name(levels[4]), "error");
    assert!(host_logs(&rt).is_empty(), "nothing was refused");
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

    // A forced breach (the conformance suite's simulation) takes the same
    // path: warning, then final warning and suspend on the second in a row.
    let mut rt = load(&budget_module());
    rt.init(10, 10, 1.0).unwrap();
    rt.force_budget_breach();
    rt.frame_with_dt(0).unwrap();
    assert!(rt.last_frame().over_budget);
    assert!(rt.last_frame().fuel < FRAME_BUDGET_FUEL, "it cost nothing");
    assert_eq!(module_logs(&rt), vec!["budget"]);
    rt.frame_with_dt(0).unwrap();
    assert!(!rt.last_frame().over_budget, "one frame only");
    rt.force_budget_breach();
    rt.frame_with_dt(0).unwrap();
    rt.force_budget_breach();
    rt.frame_with_dt(0).unwrap();
    assert!(rt.is_suspended());
    assert_eq!(
        module_logs(&rt),
        vec!["budget", "budget", "budget", "final", "suspend"]
    );

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

/// SPEC-5.3.3: a frame whose rasterization the host had to cut short is a
/// budget breach like a slow `vy_frame`: logged, `KIND_BUDGET` with `c = 0`
/// and `a` past 8, counted in the same streak so a second one in a row is
/// the final warning and a forced `vy_suspend`, and a frame within both
/// budgets resets the streak. A finished rasterization changes nothing, a
/// frame that already breached by fuel is not counted twice, and nothing
/// reaches a suspended module.
#[test]
fn spec_5_3_3_truncated_raster_is_a_budget_breach() {
    let truncated = |units: u64| RasterReport {
        truncated: true,
        ops_done: 65,
        units,
        budget: 1 << 26,
    };
    let finished = RasterReport {
        truncated: false,
        ops_done: 10,
        units: 1000,
        budget: 1 << 26,
    };
    let mut rt = load(&budget_module());
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(0).unwrap();
    rt.raster_report(&finished).unwrap();
    assert!(!rt.last_frame().over_budget);
    assert!(module_logs(&rt).is_empty(), "a finished frame is no breach");

    rt.frame_with_dt(0).unwrap();
    rt.raster_report(&truncated(1 << 26)).unwrap();
    assert!(rt.last_frame().over_budget);
    assert_eq!(module_logs(&rt), vec!["budget"], "first breach: a warning");
    assert!(host_logs(&rt)
        .iter()
        .any(|l| l
            .starts_with("rasterization stopped after 65 ops at 67108864 of 67108864 work units")));
    assert!(!rt.is_suspended());

    // Within budget in between: the streak starts over.
    rt.frame_with_dt(0).unwrap();
    rt.raster_report(&finished).unwrap();
    rt.frame_with_dt(0).unwrap();
    rt.raster_report(&truncated(1 << 26)).unwrap();
    assert_eq!(module_logs(&rt), vec!["budget", "budget"]);
    assert!(!rt.is_suspended());

    // Two in a row: final warning and forced suspend; a report after that
    // is logged but delivered to nobody.
    rt.frame_with_dt(0).unwrap();
    rt.raster_report(&truncated(3 << 26)).unwrap();
    assert_eq!(
        module_logs(&rt),
        vec!["budget", "budget", "budget", "final", "suspend"]
    );
    assert!(rt.is_suspended());
    rt.raster_report(&truncated(1)).unwrap();
    assert_eq!(module_logs(&rt).len(), 5);

    // Fuel and raster breaches share the streak: a slow frame whose list
    // was also cut short counts once, and the next truncated one suspends.
    let mut rt = load(&budget_module());
    rt.init(10, 10, 1.0).unwrap();
    rt.frame_with_dt(3_000_000).unwrap();
    assert!(rt.last_frame().over_budget);
    rt.raster_report(&truncated(1 << 26)).unwrap();
    assert_eq!(module_logs(&rt), vec!["budget"], "counted once");
    rt.frame_with_dt(0).unwrap();
    rt.raster_report(&truncated(1 << 26)).unwrap();
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
        let images = vy_render::ImageCache::from_bytes(Vec::new());
        let img = vy_render::render(&dls[2], 120, 60, 1.0, rt.text_engine_mut(), &images)
            .unwrap()
            .image;
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
    // The high half is the width: one line is wider than 100 px (the
    // width comes back through vy_event's result, which the host then
    // takes as a redraw request, SPEC-5.1.3).
    let r = rt.event(1, 0, 0, 0, 0);
    assert!(matches!(r, Ok(true)), "{r:?}");
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
    // text_measure's low 32 bits are the same height (SPEC-5.2.5); vy_event
    // here returns them, a value other than 0 or 1, which the host takes
    // as a redraw request (SPEC-5.1.3).
    assert!(rt.event(0, 100, 0, 0, 0).unwrap());
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

    // A module whose initial memory leaves no page to grow is refused before
    // it is compiled (SPEC-8.1.3): 8 initial pages need memory_max >= 9.
    let big = wasm(&format!(
        r#"(module (memory (export "memory") 8) {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) i32.const 0) {STUBS})"#
    ));
    assert!(matches!(
        Runtime::load(&big, &sha256(&big), opts),
        Err(Error::MemoryMax(4))
    ));
    for pages in [8, 9] {
        let opts = Options {
            memory_max_pages: pages,
            ..Options::default()
        };
        let r = Runtime::load(&big, &sha256(&big), opts).map(|_| ());
        assert_eq!(r.is_ok(), pages == 9, "memory_max {pages}: {r:?}");
    }
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
    rt.event(1, 1, 0, 0, 0).unwrap();
    assert_eq!(rt.last_event_result(), err::E_DL_FULL);
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

/// SPEC-5.2.7, SPEC-5.3.1: without a canned response the headless host has
/// no transport, so `fetch_begin` is `E_DENIED` even with `net` granted
/// (noted at debug level, since the capability is there: it is the host
/// that lacks a network), the other `fetch_*` calls name no open request
/// (`E_ARG`), and the page runs on. Storage is served (SPEC-5.2.9): a key
/// nobody wrote is `E_RESOURCE`.
#[test]
fn spec_5_2_7_fetch_and_storage_denied_headless() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "fetch_begin" (func $fetch_begin (param i32 i32 i32 i32 i32) (result i32)))
            (import "vy" "fetch_poll" (func $fetch_poll (param i32) (result i32)))
            (import "vy" "fetch_read" (func $fetch_read (param i32 i32 i32) (result i32)))
            (import "vy" "fetch_cancel" (func $fetch_cancel (param i32) (result i32)))
            (import "vy" "state_get" (func $state_get (param i32 i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "https://chat.example/api")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (local $r i32)
                (if (i32.eq (local.get 0) (i32.const 1)) (then
                    (local.set $r (call $fetch_begin (i32.const 1) (i32.const 0) (i32.const 24) (i32.const 0) (i32.const 0)))))
                (if (i32.eq (local.get 0) (i32.const 2)) (then
                    (local.set $r (call $fetch_poll (i32.const 1)))))
                (if (i32.eq (local.get 0) (i32.const 3)) (then
                    (local.set $r (call $fetch_read (i32.const 1) (i32.const 1024) (i32.const 64)))))
                (if (i32.eq (local.get 0) (i32.const 4)) (then
                    (local.set $r (call $fetch_cancel (i32.const 1)))))
                (if (i32.eq (local.get 0) (i32.const 5)) (then
                    (local.set $r (call $state_get (i32.const 0) (i32.const 5) (i32.const 1024) (i32.const 64)))))
                local.get $r)
            {STUBS})"#
    ));
    let opts = Options {
        capabilities: Capabilities::from_names(["net", "storage"]).unwrap(),
        ..Options::default()
    };
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    rt.init(10, 10, 1.0).unwrap();
    for (case, name, code, level) in [
        (1, "fetch_begin", err::E_DENIED, 0),
        (2, "fetch_poll", err::E_ARG, 2),
        (3, "fetch_read", err::E_ARG, 2),
        (4, "fetch_cancel", err::E_ARG, 2),
        (5, "state_get", err::E_RESOURCE, 0),
    ] {
        let before = rt.logs().len();
        rt.frame_with_dt(case).unwrap();
        assert_eq!(rt.last_frame().ops_reported, code, "{name}");
        let new: Vec<&LogEntry> = rt.logs()[before..]
            .iter()
            .filter(|l| l.source == LogSource::Host)
            .collect();
        assert_eq!(new.len(), 1, "{name}: {new:?}");
        assert!(
            new[0]
                .message
                .starts_with(&format!("{name}: {}", host::err_name(code))),
            "{new:?}"
        );
        assert_eq!(
            new[0].level, level,
            "{name}: debug for the host's own limits"
        );
    }
    assert!(!rt.is_suspended());
}

/// A module driven by `vy_event`: kind 200 returns `clipboard_read(2048,
/// a)`; 201 returns `clipboard_write(0, a)` of the digits at offset 0;
/// every other kind returns `a`, so a test can hand the host any
/// `vy_event` result it likes.
fn clipboard_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module
            (import "vy" "clipboard_read" (func $clipboard_read (param i32 i32) (result i32)))
            (import "vy" "clipboard_write" (func $clipboard_write (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "0123456789")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 200)) (then
                    (return (call $clipboard_read (i32.const 2048) (local.get 1)))))
                (if (i32.eq (local.get 0) (i32.const 201)) (then
                    (return (call $clipboard_write (i32.const 0) (local.get 1)))))
                local.get 1)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
    ))
}

fn clipboard_runtime(clipboard: Option<&[u8]>, granted: bool) -> Runtime {
    let bytes = clipboard_module();
    let opts = Options {
        capabilities: if granted {
            Capabilities::from_names(["clipboard"]).unwrap()
        } else {
            Capabilities::default()
        },
        clipboard: clipboard.map(<[u8]>::to_vec),
        ..Options::default()
    };
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    rt.init(10, 10, 1.0).unwrap();
    rt
}

/// SPEC-5.2.12, SPEC-5.3.1: `clipboard_read` is `E_DENIED` without the
/// capability, `E_GESTURE` without a pointer press, key or text input
/// within the last second of the host's clock, returns the clipboard's
/// length without copying when `max` is 0, copies the whole clipboard and
/// returns its length when it fits, and is `E_ARG` without a copy when it
/// does not; `clipboard_write` replaces the clipboard.
#[test]
fn spec_5_2_12_clipboard_read_returns_length_or_code() {
    let call = |rt: &mut Runtime, kind: i32, a: i32| {
        rt.event(kind, a, 0, 0, 0).unwrap();
        rt.last_event_result()
    };
    let mut rt = clipboard_runtime(Some(b"hej du"), false);
    assert_eq!(call(&mut rt, 200, 64), err::E_DENIED, "capability first");
    assert!(host_logs(&rt)[0].starts_with("clipboard_read: E_DENIED"));

    let mut rt = clipboard_runtime(Some(b"hej du"), true);
    // Kind 7 is TEXT_INPUT; the key events are gestures, pointer moves are not.
    assert_eq!(call(&mut rt, event::POINTER_MOVE, 64), 64);
    assert_eq!(call(&mut rt, 200, 64), err::E_GESTURE, "no gesture yet");
    assert_eq!(host_logs(&rt), vec!["clipboard_read: E_GESTURE"]);
    rt.event(event::KEY_DOWN, 0x04, 0, 0, 0).unwrap();
    assert_eq!(
        call(&mut rt, 200, 0),
        6,
        "max = 0: the length, nothing copied"
    );
    assert_eq!(rt.read_memory(2048, 6), [0; 6]);
    assert_eq!(
        call(&mut rt, 200, 5),
        err::E_ARG,
        "too small: nothing copied"
    );
    assert!(host_logs(&rt)
        .last()
        .unwrap()
        .starts_with("clipboard_read: E_ARG (clipboard is 6 bytes, buffer is 5"));
    assert_eq!(rt.read_memory(2048, 6), [0; 6]);
    assert_eq!(call(&mut rt, 200, 6), 6, "fits exactly: bytes written");
    assert_eq!(rt.read_memory(2048, 6), b"hej du");
    assert_eq!(
        call(&mut rt, 200, 64),
        6,
        "a bigger buffer: still the length"
    );
    assert_eq!(call(&mut rt, 200, -1), err::E_ARG);

    // The gesture window is one second of the host's clock: 16 ms per
    // frame headless, so 62 frames keep it open and the 63rd closes it.
    for _ in 0..62 {
        rt.frame_with_dt(0).unwrap();
    }
    assert_eq!(
        call(&mut rt, 200, 64),
        6,
        "1 s after the key: still allowed"
    );
    rt.frame_with_dt(0).unwrap();
    assert_eq!(call(&mut rt, 200, 64), err::E_GESTURE, "1 016 ms: closed");
    rt.event(event::POINTER_UP, 100, 100, 1, 0).unwrap();
    assert_eq!(call(&mut rt, 200, 64), 6, "a release reopens it");

    // Writing replaces the clipboard; reading it back needs no new gesture
    // within the window.
    assert_eq!(call(&mut rt, 201, 10), err::OK);
    assert_eq!(call(&mut rt, 200, 0), 10);
    assert_eq!(call(&mut rt, 200, 10), 10);
    assert_eq!(rt.read_memory(2048, 10), b"0123456789");
    assert_eq!(call(&mut rt, 201, 0), err::OK, "an empty clipboard");
    assert_eq!(call(&mut rt, 200, 0), 0);
    assert_eq!(call(&mut rt, 200, 8), 0, "nothing to copy, nothing refused");

    // A host without a clipboard denies after the gates, at debug level.
    let mut rt = clipboard_runtime(None, true);
    rt.event(event::POINTER_DOWN, 0, 0, 1, 0).unwrap();
    assert_eq!(call(&mut rt, 200, 64), err::E_DENIED);
    assert_eq!(call(&mut rt, 201, 3), err::E_DENIED);
    let unavailable: Vec<&LogEntry> = rt
        .logs()
        .iter()
        .filter(|l| l.message.contains("not available in the headless host"))
        .collect();
    assert_eq!(unavailable.len(), 2);
    assert!(unavailable.iter().all(|l| l.level == vy_abi::level::DEBUG));
}

/// SPEC-5.1.3: `vy_event` returning 1 asks for a redraw and 0 does not;
/// any other value is treated as 1, and the host notes it once per
/// instance at debug level.
#[test]
fn spec_5_1_3_event_result_other_than_0_or_1() {
    let mut rt = clipboard_runtime(None, false);
    rt.frame_with_dt(0).unwrap();
    assert!(!rt.event(1, 0, 0, 0, 0).unwrap(), "0: no redraw");
    assert!(!rt.wants_frame());
    assert!(rt.event(1, 1, 0, 0, 0).unwrap(), "1: redraw");
    assert!(rt.wants_frame());
    rt.frame_with_dt(0).unwrap();
    let notes = |rt: &Runtime| {
        rt.logs()
            .iter()
            .filter(|l| l.source == LogSource::Host && l.message.starts_with("vy_event returned "))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert!(notes(&rt).is_empty(), "0 and 1 are not noted");
    for odd in [2, -1, i32::MAX, i32::MIN] {
        assert!(rt.event(1, odd, 0, 0, 0).unwrap(), "{odd} counts as 1");
        assert!(rt.wants_frame(), "{odd}");
        rt.frame_with_dt(0).unwrap();
        assert_eq!(rt.last_event_result(), odd, "the raw value is kept");
    }
    let notes = notes(&rt);
    assert_eq!(notes.len(), 1, "noted once per instance: {notes:?}");
    assert_eq!(
        notes[0].message,
        "vy_event returned 2; treated as 1 (redraw) (SPEC-5.1.3)"
    );
    assert_eq!(notes[0].level, vy_abi::level::DEBUG);
    assert!(host_logs(&rt).is_empty(), "nothing was refused");
}

/// A module driven by `vy_event`: kind 200 puts key `k` with `a` bytes of
/// value from offset 16 (the digits "0123456789" repeated); 201 gets `k`
/// into offset 2048 with `max = a` and returns the code; 202 deletes `k`;
/// 203 puts key `big` with `a` bytes. The result of the call is the
/// event's return value.
fn storage_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module
            (import "vy" "state_put" (func $state_put (param i32 i32 i32 i32) (result i32)))
            (import "vy" "state_get" (func $state_get (param i32 i32 i32 i32) (result i32)))
            (import "vy" "state_delete" (func $state_delete (param i32 i32) (result i32)))
            (memory (export "memory") 2)
            (data (i32.const 0) "k")
            (data (i32.const 4) "big")
            (data (i32.const 16) "0123456789012345678901234567890123456789")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 200)) (then
                    (return (call $state_put (i32.const 0) (i32.const 1) (i32.const 16) (local.get 1)))))
                (if (i32.eq (local.get 0) (i32.const 201)) (then
                    (return (call $state_get (i32.const 0) (i32.const 1) (i32.const 2048) (local.get 1)))))
                (if (i32.eq (local.get 0) (i32.const 202)) (then
                    (return (call $state_delete (i32.const 0) (i32.const 1)))))
                (if (i32.eq (local.get 0) (i32.const 203)) (then
                    (return (call $state_put (i32.const 4) (i32.const 3) (i32.const 16) (local.get 1)))))
                i32.const 0)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
    ))
}

/// SPEC-5.2.9, SPEC-9.3.1, SPEC-5.1.5: with `storage` granted the headless
/// host keeps a key space in memory: `state_get` on a key nobody wrote is
/// `E_RESOURCE`; `state_put` stores; `state_get` reports the full length
/// and copies at most `max` bytes; `state_delete` makes the key missing
/// again; the quota is enforced as `E_QUOTA`; and the storage taken from
/// one instance seeds the next, which is how values survive a suspend and
/// a fresh `vy_init`.
#[test]
fn spec_5_2_9_headless_storage() {
    let bytes = storage_module();
    let opts = Options {
        capabilities: Capabilities::from_names(["storage"]).unwrap(),
        storage_quota_bytes: 40,
        ..Options::default()
    };
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    rt.init(10, 10, 1.0).unwrap();
    let call = |rt: &mut Runtime, kind: i32, a: i32| {
        rt.event(kind, a, 0, 0, 0).unwrap();
        rt.last_event_result()
    };
    assert_eq!(call(&mut rt, 201, 64), err::E_RESOURCE, "nothing saved yet");
    assert_eq!(call(&mut rt, 200, 10), err::OK);
    assert_eq!(rt.storage().get(b"k".as_slice()).unwrap(), b"0123456789");
    assert_eq!(call(&mut rt, 201, 4), 10, "full length, four copied");
    assert_eq!(rt.read_memory(2048, 6), b"0123\0\0");
    assert_eq!(call(&mut rt, 201, 64), 10);
    assert_eq!(rt.read_memory(2048, 10), b"0123456789");
    assert_eq!(call(&mut rt, 200, 0), err::OK, "an empty value");
    assert_eq!(call(&mut rt, 201, 64), 0, "exists, zero bytes");
    assert_eq!(call(&mut rt, 202, 0), err::OK);
    assert_eq!(call(&mut rt, 201, 64), err::E_RESOURCE, "deleted");
    assert_eq!(
        call(&mut rt, 202, 0),
        err::OK,
        "deleting a missing key is fine"
    );

    // Quota: 40 bytes for keys and values together (SPEC-9.3.1).
    assert_eq!(call(&mut rt, 200, 20), err::OK, "k + 20 = 21 bytes");
    assert_eq!(call(&mut rt, 203, 20), err::E_QUOTA, "big + 20 would be 44");
    assert_eq!(call(&mut rt, 203, 16), err::OK, "big + 16 = 19: exactly 40");
    assert_eq!(
        call(&mut rt, 200, 21),
        err::E_QUOTA,
        "replacing k: 22 + 19 = 41"
    );
    assert_eq!(call(&mut rt, 200, 5), err::OK, "replacing k with less");
    assert!(host_logs(&rt).iter().any(|l| l == "state_put: E_QUOTA"));

    // Persistence: the next instance starts with what this one saved.
    rt.suspend().unwrap();
    let saved = rt.take_storage();
    assert_eq!(saved.len(), 2);
    assert!(rt.storage().is_empty(), "taken, not copied");
    let opts = Options {
        capabilities: Capabilities::from_names(["storage"]).unwrap(),
        storage: saved,
        ..Options::default()
    };
    let mut again = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    again.init(10, 10, 1.0).unwrap();
    assert_eq!(call(&mut again, 201, 64), 5, "k came back");
    assert_eq!(again.read_memory(2048, 5), b"01234");

    // Without the capability nothing is stored, whatever was handed in.
    let opts = Options {
        storage: again.take_storage(),
        ..Options::default()
    };
    let mut denied = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    denied.init(10, 10, 1.0).unwrap();
    assert_eq!(call(&mut denied, 201, 64), err::E_DENIED);
    assert_eq!(call(&mut denied, 200, 1), err::E_DENIED);
    assert_eq!(denied.storage().len(), 2, "untouched");
}

/// A module with a fixed tree (main 1; button 2 at y 0, textbox 3 at y 20,
/// text 4 at y 30, link 5 at y 40, all 50 wide and 10 tall) that records
/// every event it receives as `kind * 1e6 + (a % 1000) * 1000 + (b % 10) *
/// 10 + c % 10`. Kind 100 returns record `a`, 101 clears, 102 returns the
/// count, 103 toggles whether the button (node 2) is registered at all;
/// those four are not recorded.
fn focus_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module
            (import "vy" "sem_node" (func $sem_node (param i32 i32 i32 f32 f32 f32 f32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (global $n (mut i32) (i32.const 0))
            (global $hide (mut i32) (i32.const 0))
            {INIT_OK}
            (func $node (param $id i32) (param $role i32) (param $y f32)
                (drop (call $sem_node (local.get $id) (i32.const 1) (local.get $role) (f32.const 0) (local.get $y) (f32.const 50) (f32.const 10) (i32.const 0) (i32.const 0))))
            (func (export "vy_frame") (param i32) (result i32)
                (drop (call $sem_node (i32.const 1) (i32.const 0) (i32.const 15) (f32.const 0) (f32.const 0) (f32.const 100) (f32.const 100) (i32.const 0) (i32.const 0)))
                (if (i32.eqz (global.get $hide)) (then
                    (call $node (i32.const 2) (i32.const 4) (f32.const 0))))
                (call $node (i32.const 3) (i32.const 5) (f32.const 20))
                (call $node (i32.const 4) (i32.const 1) (f32.const 30))
                (call $node (i32.const 5) (i32.const 3) (f32.const 40))
                i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 100)) (then
                    (return (i32.load (i32.add (i32.const 1024) (i32.mul (local.get 1) (i32.const 4)))))))
                (if (i32.eq (local.get 0) (i32.const 101)) (then
                    (global.set $n (i32.const 0))
                    (return (i32.const 0))))
                (if (i32.eq (local.get 0) (i32.const 102)) (then
                    (return (global.get $n))))
                (if (i32.eq (local.get 0) (i32.const 103)) (then
                    (global.set $hide (i32.sub (i32.const 1) (global.get $hide)))
                    (return (i32.const 0))))
                (i32.store (i32.add (i32.const 1024) (i32.mul (global.get $n) (i32.const 4)))
                    (i32.add
                        (i32.mul (local.get 0) (i32.const 1000000))
                        (i32.add
                            (i32.mul (i32.rem_u (local.get 1) (i32.const 1000)) (i32.const 1000))
                            (i32.add
                                (i32.mul (i32.rem_u (local.get 2) (i32.const 10)) (i32.const 10))
                                (i32.rem_u (local.get 3) (i32.const 10))))))
                (global.set $n (i32.add (global.get $n) (i32.const 1)))
                i32.const 1)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
    ))
}

/// The events `focus_module` recorded since the last call, decoded as
/// `(kind, a, b, c)` with `a` modulo 1000.
fn recorded(rt: &mut Runtime) -> Vec<(i32, i32, i32, i32)> {
    rt.event(102, 0, 0, 0, 0).unwrap();
    let n = rt.last_event_result();
    let mut out = Vec::new();
    for i in 0..n {
        rt.event(100, i, 0, 0, 0).unwrap();
        let v = rt.last_event_result();
        out.push((v / 1_000_000, v / 1000 % 1000, v / 10 % 10, v % 10));
    }
    rt.event(101, 0, 0, 0, 0).unwrap();
    out
}

/// SPEC-10.3.1, SPEC-10.3.2, SPEC-10.1.1: the host owns focus. Tab and
/// Shift+Tab never reach the module; they move focus through the focusable
/// nodes of the whole tree in document order (button, textbox, link; the
/// text node is skipped), wrapping, and arrive as `FOCUS` out then in with
/// `c = 1`. Enter and Escape are ordinary `KEY_DOWN`s. A pointer press
/// focuses the node under it (`c = 0`) before the press is delivered, and
/// a press on nothing focusable clears focus. A scripted `FOCUS` moves the
/// host's focus as well.
#[test]
fn spec_10_3_1_host_owns_focus() {
    use vy_abi::event::{FOCUS, KEY_DOWN, POINTER_DOWN};
    let mut rt = load(&focus_module());
    rt.init(100, 100, 1.0).unwrap();
    rt.frame().unwrap();
    assert_eq!(rt.focus(), None);
    recorded(&mut rt);

    assert!(
        rt.event(KEY_DOWN, key::TAB, 0, 0, 0).unwrap(),
        "focus moved: redraw"
    );
    assert_eq!(
        recorded(&mut rt),
        vec![(FOCUS, 2, 1, 1)],
        "first focusable, no KEY_DOWN 43"
    );
    assert_eq!(rt.focus(), Some(2));
    assert!(!rt.event(vy_abi::event::KEY_UP, key::TAB, 0, 0, 0).unwrap());
    assert!(
        recorded(&mut rt).is_empty(),
        "KEY_UP of Tab is swallowed too"
    );
    rt.event(KEY_DOWN, key::TAB, 0, 0, 0).unwrap();
    assert_eq!(recorded(&mut rt), vec![(FOCUS, 2, 0, 1), (FOCUS, 3, 1, 1)]);
    rt.event(KEY_DOWN, key::TAB, 0, 0, 0).unwrap();
    assert_eq!(
        recorded(&mut rt),
        vec![(FOCUS, 3, 0, 1), (FOCUS, 5, 1, 1)],
        "text skipped"
    );
    rt.event(KEY_DOWN, key::TAB, 0, 0, 0).unwrap();
    assert_eq!(
        recorded(&mut rt),
        vec![(FOCUS, 5, 0, 1), (FOCUS, 2, 1, 1)],
        "wraps"
    );
    rt.event(KEY_DOWN, key::TAB, modifier::SHIFT, 0, 0).unwrap();
    assert_eq!(
        recorded(&mut rt),
        vec![(FOCUS, 2, 0, 1), (FOCUS, 5, 1, 1)],
        "Shift+Tab backwards"
    );

    rt.event(KEY_DOWN, key::ENTER, 0, 0, 0).unwrap();
    rt.event(KEY_DOWN, key::ESCAPE, 0, 0, 0).unwrap();
    assert_eq!(
        recorded(&mut rt),
        vec![(KEY_DOWN, key::ENTER, 0, 0), (KEY_DOWN, key::ESCAPE, 0, 0)],
        "Enter and Escape are the module's"
    );

    // Pointer: press on the textbox (y 25), then on the text node (y 35).
    rt.event(POINTER_DOWN, 1000, 2500, 1, 0).unwrap();
    assert_eq!(
        recorded(&mut rt),
        vec![(FOCUS, 5, 0, 0), (FOCUS, 3, 1, 0), (POINTER_DOWN, 0, 0, 1)],
        "focus before the press, c = 0 for the pointer"
    );
    assert_eq!(rt.focus(), Some(3));
    rt.event(POINTER_DOWN, 1000, 3500, 1, 0).unwrap();
    assert_eq!(
        recorded(&mut rt),
        vec![(FOCUS, 3, 0, 0), (POINTER_DOWN, 0, 0, 1)]
    );
    assert_eq!(rt.focus(), None, "nothing focusable under the pointer");
    rt.event(POINTER_DOWN, 1000, 3500, 1, 0).unwrap();
    assert_eq!(
        recorded(&mut rt),
        vec![(POINTER_DOWN, 0, 0, 1)],
        "no change, no FOCUS"
    );

    // A scripted FOCUS (what cases.toml sends) is the host's focus too.
    rt.event(FOCUS, 5, 1, 1, 0).unwrap();
    assert_eq!(rt.focus(), Some(5));
    rt.event(KEY_DOWN, key::TAB, 0, 0, 0).unwrap();
    assert_eq!(
        recorded(&mut rt),
        vec![(FOCUS, 5, 1, 1), (FOCUS, 5, 0, 1), (FOCUS, 2, 1, 1)]
    );

    // Keys the host consumes are never "held" for `key_down`.
    assert!(!rt.host().keys_down.contains(&key::TAB));
}

/// SPEC-10.3.2, SPEC-10.3.1: focus follows node ids across frames. When the
/// focused node is left out of a frame's tree the host delivers `FOCUS`
/// out for it (`b = 0`, `c = 0`) after that frame and nothing has focus;
/// the next Tab starts from the beginning of document order, which is how
/// a page moves focus without an import: by what it registers, and in what
/// order. A node that comes back does not regain focus by itself.
#[test]
fn spec_10_3_2_focus_lost_when_node_disappears() {
    use vy_abi::event::{FOCUS, KEY_DOWN};
    let mut rt = load(&focus_module());
    rt.init(100, 100, 1.0).unwrap();
    rt.frame().unwrap();
    rt.event(KEY_DOWN, key::TAB, 0, 0, 0).unwrap();
    assert_eq!(rt.focus(), Some(2));
    recorded(&mut rt);

    // The page stops registering the button; the next frame drops focus.
    rt.event(103, 0, 0, 0, 0).unwrap();
    rt.frame().unwrap();
    assert_eq!(rt.focus(), None, "the node is gone, so is focus");
    assert_eq!(
        recorded(&mut rt),
        vec![(FOCUS, 2, 0, 0)],
        "FOCUS out after the frame, neither keyboard nor pointer"
    );
    assert!(
        rt.wants_frame(),
        "the module answered 1 to the FOCUS out: it redraws without the ring"
    );
    rt.frame().unwrap();
    assert!(
        recorded(&mut rt).is_empty(),
        "reported once; nothing has focus so nothing more is lost"
    );

    // From nothing, Tab goes to the first focusable node now registered.
    rt.event(KEY_DOWN, key::TAB, 0, 0, 0).unwrap();
    assert_eq!(recorded(&mut rt), vec![(FOCUS, 3, 1, 1)]);

    // The button comes back: focus stays where it is, Shift+Tab reaches it.
    rt.event(103, 0, 0, 0, 0).unwrap();
    rt.frame().unwrap();
    assert_eq!(rt.focus(), Some(3));
    assert!(recorded(&mut rt).is_empty());
    rt.event(KEY_DOWN, key::TAB, modifier::SHIFT, 0, 0).unwrap();
    assert_eq!(recorded(&mut rt), vec![(FOCUS, 3, 0, 1), (FOCUS, 2, 1, 1)]);
}

/// SPEC-10.3.1, SPEC-10.3.2: without a single focusable node Tab is consumed
/// and nothing happens; the module never sees the key.
#[test]
fn spec_10_3_2_tab_with_nothing_focusable() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "log" (func $log (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "key")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (drop (call $log (i32.const 0) (i32.const 0) (i32.const 3)))
                i32.const 1)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
    ));
    let mut rt = load(&bytes);
    rt.init(10, 10, 1.0).unwrap();
    rt.frame().unwrap();
    assert!(!rt.event(event::KEY_DOWN, key::TAB, 0, 0, 0).unwrap());
    assert!(!rt
        .event(event::KEY_DOWN, key::TAB, modifier::SHIFT, 0, 0)
        .unwrap());
    assert!(module_logs(&rt).is_empty(), "the module saw no event");
    assert!(!rt.wants_frame());
    assert!(rt.event(event::KEY_DOWN, key::ENTER, 0, 0, 0).unwrap());
    assert_eq!(module_logs(&rt), vec!["key"]);
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
/// names it, and that decoding costs the frame nothing (SPEC-5.3.3: only
/// the opcode's bytes are charged, the same on the first frame as on the
/// next); id 0, an unknown id and an image over 8192 px are `E_RESOURCE`;
/// the decoded image reaches the pixels through the runtime's cache.
#[test]
fn spec_5_2_3_dl_image_fit_and_resources() {
    let red = png(2, 1, [255, 0, 0, 255]);
    let huge = png(limits::MAX_IMAGE_SIDE_PX + 1, 1, [0, 255, 0, 255]);
    let bad = b"not an image".to_vec();
    let opts = Options {
        resources: vec![
            vy_render::Resource::with_size(red, 2, 1),
            vy_render::Resource::new(huge),
            vy_render::Resource::new(bad),
        ],
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

    // fit 0..=2 with resource 1: decoded once, on the first call, for free.
    for fit_mode in [0, 1, 2] {
        let dl = rt.frame_with_dt(10 + fit_mode).unwrap().to_vec();
        let ops = vy_dl::decode(&dl).unwrap();
        assert!(
            matches!(ops[0], vy_dl::Op::Image { resource_id: 1, fit, .. } if fit == fit_mode as u8),
            "{ops:?}"
        );
        assert_eq!(rt.image_cache().decodes(), 1);
        assert_eq!(
            rt.last_frame().host_fuel,
            (20 + 1) * FUEL_PER_HOST_BYTE,
            "only the IMAGE op and END are charged, decoding is not"
        );
    }
    let images = rt.image_cache();
    let img = vy_render::render(rt.display_list(), 4, 2, 1.0, &mut TextEngine::new(), images)
        .unwrap()
        .image;
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

/// SPEC-8.1.2, SPEC-6.5.1: an image whose decoded size differs from the
/// size the manifest declares for it is `E_RESOURCE` at its first
/// `dl_image`, logged with both sizes, and never drawn; the right size and
/// an undeclared size both decode.
#[test]
fn spec_8_1_2_declared_image_size_is_checked() {
    let two = png(2, 1, [255, 0, 0, 255]);
    let opts = Options {
        resources: vec![
            vy_render::Resource::with_size(two.clone(), 2, 1),
            vy_render::Resource::with_size(two.clone(), 1, 2),
            vy_render::Resource::new(two),
        ],
        ..Options::default()
    };
    let bytes = image_module();
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    rt.init(4, 2, 1.0).unwrap();
    for (id, ok) in [(1, true), (2, false), (3, true)] {
        let before = rt.logs().len();
        let dl = rt.frame_with_dt(id * 10).unwrap().to_vec();
        assert_eq!(
            vy_dl::decode(&dl).unwrap().len(),
            usize::from(ok),
            "resource {id}"
        );
        assert_eq!(rt.last_frame().ops_reported, 1);
        let new = host_logs(&rt)[before..].to_vec();
        if ok {
            assert!(new.is_empty(), "{new:?}");
        } else {
            assert_eq!(
                new,
                vec![
                    "dl_image: resource 2: 2x1 px but the manifest says 1x2 (SPEC-8.1.2)"
                        .to_string(),
                    "dl_image: E_RESOURCE".to_string()
                ]
            );
        }
    }
    assert!(matches!(
        rt.image_cache().get(2),
        Err(vy_render::ImageError::SizeMismatch { .. })
    ));
    assert_eq!(
        rt.image_cache().decoded_bytes(),
        2 * 8,
        "the refused image holds no pixels"
    );
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

/// SPEC-5.3.3, SPEC-9.3.2, SPEC-5.2.13: a frame of 65 536 nodes is legal,
/// and the host's own work for it must stay linear in the node count. The
/// duplicate-id check used to scan the whole tree per call, which made the
/// legal maximum cost the host seconds (3 s in release, 10 s in debug)
/// that no fuel paid for; a set lookup keeps it well under a second even in
/// debug. The bound here is loose on purpose: it only has to tell linear
/// from quadratic on a slow CI machine.
#[test]
fn spec_5_3_3_full_tree_costs_linear_host_time() {
    let nodes = limits::MAX_SEM_NODES_PER_FRAME as i32;
    let mut rt = load(&frame_caps_module());
    rt.init(10, 10, 1.0).unwrap();
    let start = std::time::Instant::now();
    rt.event(0, nodes, 0, 0, 0).unwrap();
    let took = start.elapsed();
    assert_eq!(rt.semantic_tree().len(), nodes as usize);
    assert!(host_logs(&rt).is_empty(), "{:?}", host_logs(&rt));
    assert!(
        took < std::time::Duration::from_secs(5),
        "65 536 sem_node calls took {took:?}: the duplicate check is not linear (SPEC-5.3.3)"
    );
    // The set is per frame like the tree: the same ids are free again.
    rt.frame_with_dt(0).unwrap();
    rt.event(0, nodes, 0, 0, 0).unwrap();
    assert_eq!(rt.semantic_tree().len(), nodes as usize);
    assert!(host_logs(&rt).is_empty());
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
    let cap = limits::MAX_LINKS_PER_FRAME as i32;
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

/// SPEC-5.2.7, SPEC-9.1.1: under `Transport::External` a `fetch_begin`
/// that passed every gate is queued for the host process with its
/// method, URL and body, `fetch_poll` is 0 meanwhile, and
/// `fetch_complete` brings the response: `FETCH_DONE` with the status
/// and length, the body readable to its end; a cancelled request's
/// response is dropped; a transport failure is a negative status with
/// no body; nothing is queued for a refused call.
#[test]
fn spec_5_2_7_external_transport_queues_requests() {
    let bytes = fetch_module();
    let opts = Options {
        capabilities: Capabilities::from_names(["net"]).unwrap(),
        net_origins: vec!["https://chat.example".into()],
        transport: Transport::External,
        ..Options::default()
    };
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    rt.init(10, 10, 1.0).unwrap();
    let call = |rt: &mut Runtime, kind: i32, a: i32, b: i32| {
        rt.event(kind, a, b, 0, 0).unwrap();
        rt.last_event_result()
    };
    assert_eq!(call(&mut rt, 200, 0, 0), 1, "first request id");
    assert_eq!(call(&mut rt, 200, 1, 0), err::E_ORIGIN, "foreign origin");
    assert_eq!(call(&mut rt, 200, 0, 0), 2);
    let queued = rt.take_fetch_requests();
    assert_eq!(
        queued,
        vec![
            FetchRequest {
                id: 1,
                method: vy_abi::method::POST,
                url: API.into(),
                body: br#"{"text":"hej"}"#.to_vec(),
            },
            FetchRequest {
                id: 2,
                method: vy_abi::method::POST,
                url: API.into(),
                body: br#"{"text":"hej"}"#.to_vec(),
            },
        ],
        "only calls that passed every gate are queued"
    );
    assert!(rt.take_fetch_requests().is_empty(), "taken once");
    assert_eq!(call(&mut rt, 201, 1, 0), 0, "poll: on its way");
    assert_eq!(rt.fetch_done().unwrap(), None, "nothing answered yet");
    assert_eq!((rt.open_fetches(), rt.pending_fetches()), (2, 2));

    // The host process answers request 2 first; order of arrival rules.
    assert_eq!(
        rt.fetch_complete(2, 201, b"created".to_vec()).unwrap(),
        Some(true)
    );
    assert_eq!(fetch_done_events(&mut rt), vec![(2, 201, 7)]);
    assert_eq!(call(&mut rt, 201, 2, 0), 1, "poll: readable");
    assert_eq!(call(&mut rt, 201, 1, 0), 0, "the other still on its way");
    assert_eq!(call(&mut rt, 202, 2, 64), 7);
    assert_eq!(rt.read_memory(2048, 7), b"created");
    assert_eq!(call(&mut rt, 202, 2, 64), 0, "the end closes it");
    assert_eq!(rt.open_fetches(), 1);

    // Cancelled before the answer: the answer is dropped, no event.
    assert_eq!(call(&mut rt, 203, 1, 0), err::OK);
    assert_eq!(rt.fetch_complete(1, 200, b"late".to_vec()).unwrap(), None);
    assert_eq!(fetch_done_events(&mut rt), vec![]);
    assert_eq!(rt.open_fetches(), 0);

    // A transport failure: E_NET as the status, no body (SPEC-5.2.7).
    assert_eq!(call(&mut rt, 200, 0, 0), 3);
    assert_eq!(rt.take_fetch_requests().len(), 1);
    assert_eq!(
        rt.fetch_complete(3, err::E_NET, Vec::new()).unwrap(),
        Some(true)
    );
    assert_eq!(fetch_done_events(&mut rt), vec![(3, err::E_NET, 0)]);
    assert_eq!(call(&mut rt, 202, 3, 64), 0);
    // An answer twice, or for an unknown id, is nothing.
    assert_eq!(rt.fetch_complete(3, 200, Vec::new()).unwrap(), None);
    assert_eq!(rt.fetch_complete(99, 200, Vec::new()).unwrap(), None);
    // The canned transport never queues anything.
    let mut canned = fetch_runtime(vec![], &[]);
    assert_eq!(call(&mut canned, 200, 0, 0), err::E_DENIED);
    assert!(canned.take_fetch_requests().is_empty());
}

/// SPEC-5.2.10, SPEC-2.1.7: with navigation on, `navigate` is `OK` for
/// `vy://` and `https://` and the shell takes the addresses in order with
/// their `replace` flags; anything else stays `E_ARG`; the headless host
/// still denies and queues nothing.
#[test]
fn spec_5_2_10_navigate_queues_for_the_shell() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "navigate" (func $navigate (param i32 i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "vy://example.test/a")
            (data (i32.const 32) "https://example.test/b")
            (data (i32.const 64) "ftp://nej")
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 100)) (then
                    (return (call $navigate (i32.const 0) (i32.const 19) (local.get 1)))))
                (if (i32.eq (local.get 0) (i32.const 101)) (then
                    (return (call $navigate (i32.const 32) (i32.const 22) (local.get 1)))))
                (call $navigate (i32.const 64) (i32.const 9) (i32.const 0)))
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
    ));
    let call = |rt: &mut Runtime, kind: i32, replace: i32| {
        rt.event(kind, replace, 0, 0, 0).unwrap();
        rt.last_event_result()
    };
    let opts = Options {
        navigation: true,
        ..Options::default()
    };
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    rt.init(10, 10, 1.0).unwrap();
    assert_eq!(call(&mut rt, 100, 0), err::OK);
    assert_eq!(call(&mut rt, 101, 1), err::OK);
    assert_eq!(call(&mut rt, 102, 0), err::E_ARG, "ftp:// is not navigable");
    assert_eq!(
        rt.take_navigations(),
        vec![
            Navigation {
                url: "vy://example.test/a".into(),
                replace: false
            },
            Navigation {
                url: "https://example.test/b".into(),
                replace: true
            },
        ]
    );
    assert!(rt.take_navigations().is_empty());
    let mut headless = load(&bytes);
    headless.init(10, 10, 1.0).unwrap();
    assert_eq!(call(&mut headless, 100, 0), err::E_DENIED);
    assert!(headless.take_navigations().is_empty());
}

/// SPEC-9.2.1, SPEC-A.2.1, SPEC-5.3.1: a capability granted or revoked
/// while the page runs reaches the module as `CAPABILITY` (`a` the id,
/// `b` 1 or 0) and takes effect at the next import call; an id outside
/// 1.0 is refused and delivers nothing.
#[test]
fn spec_9_2_1_capability_change_at_runtime() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "clipboard_write" (func $clipboard_write (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            (data (i32.const 0) "kopia")
            (global $last (mut i32) (i32.const -1))
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32) i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const 32)) (then
                    (global.set $last (i32.add (i32.mul (local.get 1) (i32.const 10)) (local.get 2)))
                    (return (i32.const 1))))
                (if (i32.eq (local.get 0) (i32.const 100)) (then
                    (return (call $clipboard_write (i32.const 0) (i32.const 5)))))
                global.get $last)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
    ));
    let opts = Options {
        clipboard: Some(Vec::new()),
        ..Options::default()
    };
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    rt.init(10, 10, 1.0).unwrap();
    rt.event(100, 0, 0, 0, 0).unwrap();
    assert_eq!(rt.last_event_result(), err::E_DENIED, "not granted yet");
    assert!(!rt.capabilities().clipboard);
    assert!(rt.set_capability(vy_abi::cap::CLIPBOARD, true).unwrap());
    assert!(rt.capabilities().clipboard);
    rt.event(101, 0, 0, 0, 0).unwrap();
    assert_eq!(rt.last_event_result(), 31, "CAPABILITY a=3 b=1");
    rt.event(100, 0, 0, 0, 0).unwrap();
    assert_eq!(rt.last_event_result(), err::OK);
    assert_eq!(rt.clipboard(), Some(&b"kopia"[..]));
    assert!(rt.set_capability(vy_abi::cap::CLIPBOARD, false).unwrap());
    rt.event(101, 0, 0, 0, 0).unwrap();
    assert_eq!(rt.last_event_result(), 30, "CAPABILITY a=3 b=0");
    rt.event(100, 0, 0, 0, 0).unwrap();
    assert_eq!(rt.last_event_result(), err::E_DENIED, "revoked");
    assert!(matches!(rt.set_capability(7, true), Err(Error::Trap(_))));
    rt.event(101, 0, 0, 0, 0).unwrap();
    assert_eq!(rt.last_event_result(), 30, "nothing delivered for id 7");
    assert_eq!(rt.capabilities().names(), vec![] as Vec<&str>);
    let mut caps = Capabilities::default();
    assert!(caps.set(vy_abi::cap::NET, true));
    assert!(caps.set(vy_abi::cap::IME, true));
    assert!(!caps.set(0, true));
    assert_eq!(caps.names(), vec!["net", "ime"]);
    rt.set_clipboard(Some(b"ny".to_vec()));
    assert_eq!(rt.clipboard(), Some(&b"ny"[..]));
}

/// SPEC-5.2.11, SPEC-5.1.2: with `ClockSource::External` the module's
/// clock is what the host process set last, never goes backwards, and
/// feeds `dt_ms`; `random` under `RandomSource::ChaCha20` gives a
/// different stream from the headless seed and the same stream for the
/// same key, so a recorded session replays exactly (SPEC-11.4.1).
#[test]
fn spec_5_2_11_external_clock_and_keyed_random() {
    let bytes = wasm(&format!(
        r#"(module
            (import "vy" "now_ms" (func $now_ms (result i64)))
            (import "vy" "random" (func $random (param i32 i32) (result i32)))
            (memory (export "memory") 1)
            {INIT_OK}
            (func (export "vy_frame") (param i32) (result i32)
                (if (i32.eq (local.get 0) (i32.const -7)) (then
                    (drop (call $random (i32.const 0) (i32.const 8)))
                    (return (i32.load (i32.const 0)))))
                (i32.wrap_i64 (call $now_ms)))
            {STUBS})"#
    ));
    let opts = Options {
        clock: ClockSource::External,
        random: RandomSource::ChaCha20([7; 32]),
        ..Options::default()
    };
    let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
    rt.init(10, 10, 1.0).unwrap();
    rt.set_clock_ms(1_000);
    rt.frame().unwrap();
    assert_eq!(rt.last_frame().ops_reported, 1_000);
    rt.frame().unwrap();
    assert_eq!(rt.now_ms(), 1_000, "no advance without the host");
    rt.set_clock_ms(900);
    assert_eq!(rt.now_ms(), 1_000, "never backwards");
    rt.set_clock_ms(1_016);
    rt.frame().unwrap();
    assert_eq!(rt.last_frame().ops_reported, 1_016);
    let draw = |source: RandomSource| {
        let opts = Options {
            random: source,
            ..Options::default()
        };
        let mut rt = Runtime::load(&bytes, &sha256(&bytes), opts).unwrap();
        rt.init(10, 10, 1.0).unwrap();
        rt.frame_with_dt(-7).unwrap();
        rt.last_frame().ops_reported
    };
    let a = draw(RandomSource::ChaCha20([7; 32]));
    assert_eq!(
        a,
        draw(RandomSource::ChaCha20([7; 32])),
        "same key, same stream"
    );
    assert_ne!(a, draw(RandomSource::ChaCha20([8; 32])));
    assert_ne!(a, draw(RandomSource::default()));
    assert_eq!(draw(RandomSource::default()), draw(RandomSource::default()));
    // The headless clock is untouched by set_clock_ms.
    let mut headless = load(&bytes);
    headless.init(10, 10, 1.0).unwrap();
    headless.set_clock_ms(5_000);
    headless.frame().unwrap();
    assert_eq!(headless.last_frame().ops_reported, 0);
}
