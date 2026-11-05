//! SDK tests against the mock host. Each test cites the spec sentence whose
//! rule the SDK mirrors; the host-side tests of the same rules live in
//! `vy-runtime` and `vy-dl`.

use crate::event::key;
use crate::heap::{Grow, Heap, ALIGN, HEADER, PAGE};
use crate::mock::{self, Op};
use crate::prelude::*;
use crate::{api, arena, math};
use alloc::string::String;
use alloc::vec::Vec;
use core::alloc::Layout;
use vy_abi::{err, event as kind, limits};

fn style() -> StyleId {
    text_style(Font::Sans, 16.0, 400, Color::rgb(0, 0, 0)).unwrap()
}

/// SPEC-5.2.1..5.2.13, SPEC-2.1.3: the SDK binds every import of namespace
/// `vy` exactly once and binds nothing the ABI does not have.
#[test]
fn spec_5_2_all_imports_bound() {
    const BOUND: [&str; 32] = [
        "abi_version",
        "log",
        "request_frame",
        "exit",
        "dl_begin",
        "dl_end",
        "dl_rect",
        "dl_text",
        "dl_image",
        "dl_path",
        "dl_clip_push",
        "dl_clip_pop",
        "text_style",
        "text_measure",
        "text_caret",
        "key_down",
        "ime_begin",
        "ime_end",
        "fetch_begin",
        "fetch_poll",
        "fetch_read",
        "fetch_cancel",
        "state_put",
        "state_get",
        "state_delete",
        "navigate",
        "link_register",
        "now_ms",
        "random",
        "clipboard_read",
        "clipboard_write",
        "sem_node",
    ];
    assert_eq!(BOUND.len(), vy_abi::IMPORTS.len());
    for name in BOUND {
        assert!(vy_abi::import(name).is_some(), "{name} is not an import");
    }
    // The native stand-ins carry the ABI's Rust signatures (SPEC-5.3.1:
    // every fallible import returns its status).
    let _: unsafe fn(i32, *const u8, i32) -> i32 = crate::raw::log;
    let _: unsafe fn() -> i32 = crate::raw::dl_begin;
    let _: unsafe fn(f32, f32, f32, i32, *const u8, i32) -> i64 = crate::raw::dl_text;
    let _: unsafe fn(f32, f32, f32, f32, i32, i32) -> i32 = crate::raw::dl_image;
    let _: unsafe fn(i32, f32, *const u8, i32) -> i64 = crate::raw::text_measure;
    type SemNodeFn = unsafe fn(i32, i32, i32, f32, f32, f32, f32, *const u8, i32) -> i32;
    let _: SemNodeFn = crate::raw::sem_node;
    let _: unsafe fn(f32, f32, f32, f32) -> i32 = crate::raw::ime_begin;
    let _: unsafe fn(*mut u8, i32) -> i32 = crate::raw::random;
    mock::reset();
    assert_eq!(api::abi_version(), vy_abi::ABI_VERSION);
}

/// SPEC-A.1.1: every status code maps to one error and back; `OK` and other
/// non-negative results are not errors.
#[test]
fn spec_a_1_1_error_codes_roundtrip() {
    for code in [
        err::E_DENIED,
        err::E_ARG,
        err::E_OOM,
        err::E_QUOTA,
        err::E_RATE,
        err::E_DL_STATE,
        err::E_DL_FULL,
        err::E_DL_CLIP,
        err::E_RESOURCE,
        err::E_STYLE,
        err::E_ORIGIN,
        err::E_NET,
        err::E_TOO_LARGE,
        err::E_GESTURE,
    ] {
        let e = VyError::from_code(code).unwrap();
        assert_eq!(e.code(), code);
        assert!(!matches!(e, VyError::Unknown(_)), "{code}");
        assert_eq!(e.name(), crate::alloc::format!("{e}"));
    }
    assert_eq!(VyError::from_code(err::OK), None);
    assert_eq!(VyError::from_code(2000), None);
    assert_eq!(VyError::from_code(-99), Some(VyError::Unknown(-99)));
    assert_eq!(VyError::Unknown(-99).code(), -99);
}

struct TestApp {
    window: Window,
    abi_minor: i32,
    events: Vec<String>,
    suspended: u32,
}

impl App for TestApp {
    fn init(window: Window, abi_minor: i32) -> Self {
        log(Level::Info, "test: init").unwrap();
        Self {
            window,
            abi_minor,
            events: Vec::new(),
            suspended: 0,
        }
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let s = style();
        let area = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(area, 0.0, Color::rgb(255, 255, 255))?;
        f.group(Role::Main, area, "", |f| {
            let h = f.text(10.0, 10.0, area.w - 20.0, s, "Hej")?.h;
            f.sem.node(
                Role::Heading,
                Rect::new(10.0, 10.0, area.w - 20.0, h),
                "Hej",
            )?;
            Ok(())
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        self.events.push(crate::alloc::format!("{event:?}"));
        matches!(event, Event::PointerDown { .. } | Event::TextInput { .. })
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
    }

    fn suspend(&mut self) -> i32 {
        self.suspended += 1;
        7
    }
}

crate::export!(TestApp);

/// SPEC-5.1.1..5.1.6, SPEC-4.2.1: `export!` produces the six exports; init
/// builds the app, frame opens and closes the list and returns the opcode
/// count, event returns 1 only when the app wants a redraw, resize and
/// suspend forward, alloc serves a buffer the host can write to. Calls
/// before init are ignored.
#[test]
fn spec_5_1_exports_lifecycle() {
    mock::reset();
    assert_eq!(vy_frame(16), 0, "before init: ignored");
    assert_eq!(vy_event(kind::POINTER_DOWN, 0, 0, 1, 0), 0);
    assert_eq!(vy_suspend(), 0);
    vy_resize(1, 1, 1.0);
    assert!(mock::with(|h| h.ops.is_empty()));

    assert_eq!(vy_init(0, 800, 600, 2.0), 0);
    mock::with(|h| assert_eq!(h.logs, [(1, "test: init".into())]));
    let app = __VY_APP.get().unwrap();
    assert_eq!(
        app.window,
        Window {
            width: 800.0,
            height: 600.0,
            scale: 2.0
        }
    );
    assert_eq!(app.abi_minor, 0);

    let ops = vy_frame(16);
    assert_eq!(ops, 2, "a rect and a text");
    mock::with(|h| {
        assert_eq!(h.ops.first(), Some(&Op::Begin));
        assert_eq!(h.ops.last(), Some(&Op::End));
        assert_eq!(h.frame_ops().len(), 2);
        assert!(!h.dl_open());
        assert_eq!(h.sem.len(), 2);
        assert_eq!((h.sem[0].id, h.sem[0].parent_id, h.sem[0].role), (1, 0, 15));
        assert_eq!((h.sem[1].id, h.sem[1].parent_id, h.sem[1].role), (2, 1, 2));
        assert!(h.refusals.is_empty(), "{:?}", h.refusals);
    });

    assert_eq!(vy_event(kind::POINTER_MOVE, 1050, 2000, 0, 0), 0);
    assert_eq!(
        vy_event(kind::POINTER_DOWN, 1050, 2000, 1, 0),
        1,
        "redraw wanted"
    );
    let (ptr, len) = arena::write("Hej då".as_bytes()).unwrap();
    assert_eq!(vy_event(kind::TEXT_INPUT, ptr, len, 0, 0), 1);
    let app = __VY_APP.get().unwrap();
    assert_eq!(app.events.len(), 3);
    assert!(app.events[0].starts_with("PointerMove { x: 10.5, y: 20.0"));
    assert_eq!(app.events[2], "TextInput { text: \"Hej då\" }");

    vy_resize(400, 300, 1.0);
    assert_eq!(app.window.width, 400.0);
    assert_eq!(vy_suspend(), 7);
    assert_eq!(app.suspended, 1);

    let p = vy_alloc(16);
    assert_ne!(p, 0);
    assert_eq!(arena::bytes(p, 16).unwrap().len(), 16);
    assert_eq!(vy_alloc(-1), 0, "bad size is 0");
}

/// SPEC-6.1.4: the SDK refuses the 33rd clip push and a pop without push
/// before the host sees them; an unbalanced frame is still ended.
#[test]
fn spec_6_1_4_clip_depth_checked_locally() {
    mock::reset();
    let mut f = Frame::begin();
    assert_eq!(f.clip_pop(), Err(VyError::DlClip));
    for _ in 0..limits::MAX_CLIP_DEPTH {
        f.clip_push(Rect::new(0.0, 0.0, 1.0, 1.0), 0.0).unwrap();
    }
    assert_eq!(
        f.clip_push(Rect::new(0.0, 0.0, 1.0, 1.0), 0.0),
        Err(VyError::DlClip)
    );
    assert_eq!(f.clip_depth(), 32);
    for _ in 0..31 {
        f.clip_pop().unwrap();
    }
    assert_eq!(f.ops(), 63);
    assert_eq!(f.end(), Err(VyError::DlClip), "one push left open");
    mock::with(|h| {
        assert!(!h.dl_open(), "dl_end was still called");
        assert_eq!(
            h.refusals,
            ["dl_end: E_DL_CLIP"],
            "the host saw only the end"
        );
    });
    // `clipped` pops even when the body fails.
    let mut f = Frame::begin();
    let r: Result<(), VyError> =
        f.clipped(Rect::new(0.0, 0.0, 1.0, 1.0), 4.0, |_| Err(VyError::Arg));
    assert_eq!(r, Err(VyError::Arg));
    assert_eq!(f.clip_depth(), 0);
    assert_eq!(f.end(), Ok(2));
    mock::with(|h| {
        assert!(
            matches!(h.frame_ops()[0], Op::ClipPush { radius, .. } if radius == 4.0),
            "the radius reaches the host (SPEC-5.2.3)"
        );
    });
}

/// SPEC-6.1.6: the 65 537th opcode is refused by the SDK, not the host.
#[test]
fn spec_6_1_6_op_count_checked_locally() {
    mock::reset();
    let mut f = Frame::begin();
    let r = Rect::new(0.0, 0.0, 1.0, 1.0);
    for _ in 0..limits::MAX_OPS_PER_FRAME {
        f.rect(r, 0.0, Color(0)).unwrap();
    }
    assert_eq!(f.rect(r, 0.0, Color(0)), Err(VyError::DlFull));
    assert_eq!(f.text(0.0, 0.0, 1.0, style(), "x"), Err(VyError::DlFull));
    assert_eq!(f.clip_push(r, 0.0), Err(VyError::DlFull));
    assert_eq!(f.end(), Ok(limits::MAX_OPS_PER_FRAME));
    mock::with(|h| {
        assert_eq!(h.frame_ops().len(), limits::MAX_OPS_PER_FRAME as usize);
        assert!(h.refusals.is_empty(), "{:?}", h.refusals);
    });
}

/// SPEC-9.3.2: every byte cap is checked by the SDK before the call, so an
/// over-long argument is `Arg` here and the host never copies anything.
#[test]
fn spec_9_3_2_byte_caps_checked_locally() {
    mock::reset();
    mock::with(|h| {
        h.caps.storage = true;
        h.caps.clipboard = true;
    });
    let over = |n: u32| "a".repeat(n as usize + 1);
    let s = style();
    assert_eq!(
        log(Level::Info, &over(limits::MAX_LOG_BYTES)),
        Err(VyError::Arg)
    );
    assert_eq!(
        text_measure(s, 0.0, &over(limits::MAX_TEXT_BYTES)),
        Err(VyError::Arg)
    );
    assert_eq!(
        api::text_caret(s, &over(limits::MAX_TEXT_BYTES), 0),
        Err(VyError::Arg)
    );
    assert_eq!(
        api::state_put(&over(limits::MAX_STATE_KEY_BYTES), b""),
        Err(VyError::Arg)
    );
    assert_eq!(
        api::state_put(
            "k",
            &alloc::vec![0; limits::MAX_STATE_VALUE_BYTES as usize + 1]
        ),
        Err(VyError::Arg)
    );
    assert_eq!(
        api::state_get(&over(limits::MAX_STATE_KEY_BYTES), &mut []),
        Err(VyError::Arg)
    );
    assert_eq!(
        api::state_delete(&over(limits::MAX_STATE_KEY_BYTES)),
        Err(VyError::Arg)
    );
    let url = alloc::format!("vy://x/{}", over(limits::MAX_URL_BYTES));
    assert_eq!(api::navigate(&url, false), Err(VyError::Arg));
    assert_eq!(
        api::link_register(1, Rect::default(), &url),
        Err(VyError::Arg)
    );
    assert_eq!(
        api::fetch_begin(api::Method::Get, &url, b""),
        Err(VyError::Arg)
    );
    assert_eq!(
        api::fetch_begin(
            api::Method::Post,
            "vy://x/",
            &alloc::vec![0; limits::MAX_FETCH_BODY_BYTES as usize + 1]
        ),
        Err(VyError::Arg)
    );
    assert_eq!(
        api::clipboard_write(&over(limits::MAX_CLIPBOARD_BYTES)),
        Err(VyError::Arg)
    );
    let mut f = Frame::begin();
    assert_eq!(
        f.text(0.0, 0.0, 0.0, s, &over(limits::MAX_TEXT_BYTES)),
        Err(VyError::Arg)
    );
    assert_eq!(
        f.sem.node(
            Role::Text,
            Rect::default(),
            &over(limits::MAX_SEM_TEXT_BYTES)
        ),
        Err(VyError::Arg)
    );
    let mut p = Path::new();
    for _ in 0..=u16::MAX {
        p.line_to(0.0, 0.0);
    }
    assert_eq!(f.path(&p, Color(0), 0.0), Err(VyError::Arg));
    assert_eq!(
        f.image(Rect::default(), ResourceId(0), Fit::Cover),
        Err(VyError::Resource)
    );
    assert_eq!(f.end(), Ok(0));
    mock::with(|h| {
        assert!(
            h.refusals.is_empty(),
            "the host saw nothing: {:?}",
            h.refusals
        );
        assert!(h.logs.is_empty() && h.sem.is_empty() && h.links.is_empty());
    });

    // The caps themselves pass, and the host enforces its own rules too:
    // a style the SDK cannot know about is refused by the host, as `Style`.
    assert!(log(Level::Info, &"a".repeat(limits::MAX_LOG_BYTES as usize)).is_ok());
    assert_eq!(text_measure(StyleId(99), 0.0, "x"), Err(VyError::Style));
    mock::with(|h| assert_eq!(h.refusals, ["text_measure: E_STYLE"]));
}

/// SPEC-A.2.1, SPEC-5.1.3: every event kind decodes with the argument order
/// of appendix A; text events resolve their buffer; unknown kinds and
/// unresolvable buffers are delivered as `Unknown`.
#[test]
fn spec_a_2_1_event_decoding() {
    let none = |_, _| None;
    let dec = |k, a, b, c, d| Event::decode(k, a, b, c, d, none);
    assert_eq!(
        dec(kind::POINTER_MOVE, 1050, 2000, 1, 3),
        Event::PointerMove {
            x: 10.5,
            y: 20.0,
            buttons: 1,
            modifiers: Modifiers(3)
        }
    );
    let Event::PointerMove { modifiers, .. } = dec(kind::POINTER_MOVE, 0, 0, 0, 3) else {
        panic!()
    };
    assert!(modifiers.shift() && modifiers.ctrl() && !modifiers.alt() && !modifiers.meta());
    assert_eq!(
        dec(kind::POINTER_DOWN, 100, 200, 2, 0),
        Event::PointerDown {
            x: 1.0,
            y: 2.0,
            button: 2,
            modifiers: Modifiers(0)
        }
    );
    assert!(matches!(
        dec(kind::POINTER_UP, 0, 0, 1, 0),
        Event::PointerUp { button: 1, .. }
    ));
    assert_eq!(
        dec(kind::SCROLL, 0, -12000, 0, 0),
        Event::Scroll {
            dx: 0.0,
            dy: -120.0,
            lines: false,
            modifiers: Modifiers(0)
        }
    );
    assert!(matches!(
        dec(kind::SCROLL, 0, 300, 1, 0),
        Event::Scroll {
            dy: 3.0,
            lines: true,
            ..
        }
    ));
    assert_eq!(
        dec(kind::KEY_DOWN, key::ENTER, 1, 1, 0),
        Event::KeyDown {
            code: 0x28,
            modifiers: Modifiers(1),
            repeat: true
        }
    );
    assert_eq!(
        dec(kind::KEY_UP, key::TAB, 0, 0, 0),
        Event::KeyUp {
            code: 0x2B,
            modifiers: Modifiers(0)
        }
    );
    assert_eq!(dec(kind::POINTER_LEAVE, 0, 0, 0, 0), Event::PointerLeave);
    assert_eq!(
        dec(kind::FOCUS, 4, 1, 1, 0),
        Event::Focus {
            node: NodeId(4),
            gained: true,
            keyboard: true
        }
    );
    assert_eq!(
        dec(kind::FETCH_DONE, 3, 200, 512, 0),
        Event::FetchDone {
            request: 3,
            status: 200,
            bytes: 512
        }
    );
    assert_eq!(
        dec(kind::VISIBILITY, 0, 0, 0, 0),
        Event::Visibility { visible: false }
    );
    assert_eq!(
        dec(kind::BUDGET, 9, 8, 1, 0),
        Event::Budget {
            used_ms: 9,
            allowed_ms: 8,
            last_warning: true
        }
    );
    assert_eq!(
        dec(kind::CAPABILITY, vy_abi::cap::IME, 0, 0, 0),
        Event::Capability {
            cap: Capability::Ime,
            granted: false
        }
    );
    assert_eq!(
        dec(kind::CAPABILITY, 1, 1, 0, 0),
        Event::Capability {
            cap: Capability::Net,
            granted: true
        }
    );
    for c in Capability::ALL {
        assert_eq!(Capability::from_id(c.id()), Some(c));
        assert_eq!(vy_abi::cap::name(c.id()), Some(c.name()), "{c:?}");
    }
    assert_eq!(Capability::PersistFrame.id(), vy_abi::cap::PERSIST_FRAME);
    assert_eq!(Capability::Net.name(), "net");
    assert!(
        matches!(dec(kind::CAPABILITY, 7, 1, 0, 0), Event::Unknown { .. }),
        "a 1.1 capability id is unknown to a 1.0 SDK (SPEC-A.4.1)"
    );
    assert_eq!(
        dec(99, 1, 2, 3, 4),
        Event::Unknown {
            kind: 99,
            a: 1,
            b: 2,
            c: 3,
            d: 4
        }
    );
    // Text events through the arena (SPEC-5.1.6).
    let (ptr, len) = arena::write("åäö".as_bytes()).unwrap();
    let resolve = |p, l| arena::bytes(p, l).and_then(|b| core::str::from_utf8(b).ok());
    assert_eq!(
        Event::decode(kind::TEXT_INPUT, ptr, len, 0, 0, resolve),
        Event::TextInput { text: "åäö" }
    );
    assert_eq!(
        Event::decode(kind::IME_COMPOSE, ptr, len, 2, 0, resolve),
        Event::ImeCompose {
            text: "åäö",
            caret: 2
        }
    );
    assert!(matches!(
        Event::decode(kind::TEXT_INPUT, ptr, len + 1_000_000, 0, 0, resolve),
        Event::Unknown { .. }
    ));
    let (ptr, len) = arena::write(&[0xFF, 0xFE]).unwrap();
    assert!(matches!(
        Event::decode(kind::TEXT_INPUT, ptr, len, 0, 0, resolve),
        Event::Unknown { .. }
    ));
}

/// SPEC-5.1.6: `vy_alloc` hands out 8-byte aligned buffers from the arena,
/// wraps when full, and refuses sizes it cannot serve.
#[test]
fn spec_5_1_6_arena() {
    let a = arena::alloc(10);
    let b = arena::alloc(10);
    assert_ne!(a, 0);
    assert_eq!(b - a, 16, "8-byte aligned bump");
    assert_eq!(arena::alloc(arena::ARENA_BYTES as i32 + 1), 0);
    assert_eq!(arena::alloc(-1), 0);
    let big = arena::alloc(arena::ARENA_BYTES as i32);
    assert_eq!(
        arena::last(),
        (0, arena::ARENA_BYTES),
        "wrapped to the start"
    );
    assert_eq!(
        arena::bytes(big, arena::ARENA_BYTES as i32).unwrap().len(),
        arena::ARENA_BYTES
    );
    assert!(arena::bytes(big, arena::ARENA_BYTES as i32 + 1).is_none());
    assert!(arena::bytes(-1, 1).is_none());
    assert!(arena::bytes(big, -1).is_none());
}

/// SPEC-5.2.4, SPEC-5.2.5, SPEC-6.4.3: `Frame::text` returns the host's
/// width and height in logical pixels, unpacked from the same `i64` layout
/// as `text_measure`, which agrees; `max_w <= 0` wraps nothing; a refused
/// text is not counted; the page places the next element from the size it
/// got back (SPEC-2.1.3: layout lives in the module).
#[test]
fn spec_5_2_4_text_size() {
    mock::reset();
    let s = style();
    let m = text_measure(s, 100.0, "ett två tre fyra").unwrap();
    assert_eq!(m.h, 40.0, "two lines of 16 px * 1.25 under the mock model");
    assert!(m.w <= 100.0);
    let mut f = Frame::begin();
    assert_eq!(f.text(0.0, 0.0, 100.0, s, "ett två tre fyra").unwrap(), m);
    let one_line = f.text(0.0, 0.0, 0.0, s, "ett två tre fyra").unwrap();
    assert_eq!(one_line.h, 20.0, "no wrapping at max_w 0");
    assert_eq!(one_line.w, 16.0 * 0.6 * 16.0);
    assert_eq!(
        f.text(0.0, 0.0, -5.0, s, "ett två tre fyra").unwrap(),
        one_line
    );
    assert_eq!(f.text(0.0, 0.0, 0.0, StyleId(42), "x"), Err(VyError::Style));
    assert_eq!(f.ops(), 3, "a refused text is not counted");
    // The next element follows from the returned height.
    let below = f.text(20.0, 30.0 + m.h, 100.0, s, "fem").unwrap();
    assert_eq!(below.h, 20.0);
    assert_eq!(f.end(), Ok(4));
    mock::with(|h| {
        assert!(
            matches!(&h.frame_ops()[3], Op::Text { x, y, max_w, .. } if (*x, *y, *max_w) == (20.0, 70.0, 100.0))
        );
    });
    assert_eq!(api::text_caret(s, "ab\ncd", 4).unwrap(), (9.6, 20.0));
}

/// SPEC-10.1.1, SPEC-5.2.13: `Sem` numbers nodes in call order from 1,
/// nests parents with `begin`/`end`/`group`; a page's own ids are negative
/// and leave the automatic numbering alone, and a non-negative one is
/// refused, so no id is ever used twice in a frame.
#[test]
fn spec_10_1_1_sem_ids_and_parents() {
    mock::reset();
    let mut sem = Sem::new();
    let r = Rect::new(0.0, 0.0, 10.0, 10.0);
    let main = sem.begin(Role::Main, r, "").unwrap();
    assert_eq!(main, NodeId(1));
    assert_eq!(sem.node(Role::Heading, r, "H").unwrap(), NodeId(2));
    sem.group(Role::List, r, "", |sem| {
        assert_eq!(sem.node(Role::ListItem, r, "a")?, NodeId(4));
        assert_eq!(sem.node_with_id(-10, Role::ListItem, r, "b")?, NodeId(-10));
        assert_eq!(
            sem.next_id(),
            NodeId(5),
            "fixed ids do not move the counter"
        );
        assert_eq!(
            sem.node_with_id(10, Role::ListItem, r, "c"),
            Err(VyError::Arg),
            "fixed ids are negative"
        );
        assert_eq!(
            sem.node_with_id(0, Role::ListItem, r, "c"),
            Err(VyError::Arg)
        );
        Ok(())
    })
    .unwrap();
    assert_eq!(sem.parent(), 1);
    sem.end();
    sem.end();
    assert_eq!(sem.parent(), 0, "extra end is harmless");
    assert_eq!(sem.node(Role::Button, r, "Ok").unwrap(), NodeId(5));
    assert_eq!(sem.count(), 6);
    mock::with(|h| {
        let tree: Vec<(i32, i32, i32)> =
            h.sem.iter().map(|n| (n.id, n.parent_id, n.role)).collect();
        assert_eq!(
            tree,
            [
                (1, 0, 15),
                (2, 1, 2),
                (3, 1, 8),
                (4, 3, 9),
                (-10, 3, 9),
                (5, 0, 4)
            ]
        );
        assert_eq!(h.sem[5].text, "Ok");
        assert!(h.refusals.is_empty(), "the host never saw the refused ids");
    });
}

/// SPEC-5.2.3, SPEC-C.1.2: `Path` encodes 9-byte commands, `quad_to` as two;
/// `Frame::image` passes the fit mode; `Frame::path` sends the bytes.
#[test]
fn spec_c_1_2_path_and_image_encoding() {
    mock::reset();
    let mut p = Path::new();
    p.move_to(1.0, 2.0)
        .line_to(3.0, 4.0)
        .quad_to(5.0, 6.0, 7.0, 8.0)
        .close();
    assert_eq!(p.len(), 5);
    assert_eq!(p.bytes().len(), 45);
    assert_eq!(p.bytes()[0], 0);
    assert_eq!(p.bytes()[9], 1);
    assert_eq!(&p.bytes()[19..23], &7.0f32.to_le_bytes(), "end point first");
    assert_eq!(
        &p.bytes()[28..32],
        &5.0f32.to_le_bytes(),
        "then the control point"
    );
    assert_eq!(p.bytes()[36], 3);
    let mut f = Frame::begin();
    f.path(&p, Color::rgb(1, 2, 3), 2.0).unwrap();
    f.image(Rect::new(1.0, 2.0, 3.0, 4.0), ResourceId(7), Fit::Contain)
        .unwrap();
    assert_eq!(f.end(), Ok(2));
    mock::with(|h| {
        assert_eq!(
            h.frame_ops()[0],
            Op::Path {
                bytes: p.bytes().to_vec(),
                color: 0x0102_03FF,
                stroke_w: 2.0
            }
        );
        assert!(matches!(
            h.frame_ops()[1],
            Op::Image {
                resource_id: 7,
                fit: 1,
                ..
            }
        ));
    });
}

/// SPEC-5.2.6..5.2.12: the remaining bindings pass their arguments through
/// and turn the host's codes into errors (SPEC-5.3.1: a denied capability is
/// `Denied`, the page goes on).
#[test]
fn spec_5_2_6_bindings_pass_through() {
    mock::reset();
    mock::with(|h| {
        h.keys_down.push(key::SPACE);
        h.now_ms = 42;
    });
    assert!(api::key_down(key::SPACE) && !api::key_down(key::ENTER));
    assert_eq!(api::now_ms(), 42);
    let mut buf = [0u8; 12];
    api::random(&mut buf).unwrap();
    assert_ne!(buf, [0u8; 12]);
    assert_eq!(
        api::ime_begin(Rect::new(1.0, 2.0, 3.0, 4.0)),
        Err(VyError::Denied)
    );
    assert_eq!(api::ime_end(), Err(VyError::Denied));
    assert_eq!(api::state_put("k", b"v"), Err(VyError::Denied));
    assert_eq!(api::state_delete("k"), Err(VyError::Denied));
    assert_eq!(api::clipboard_read(&mut buf), Err(VyError::Denied));
    assert_eq!(api::clipboard_write("x"), Err(VyError::Denied));
    assert_eq!(api::fetch_poll(api::Request(1)), Err(VyError::Denied));
    assert_eq!(api::fetch_cancel(api::Request(1)), Err(VyError::Denied));
    mock::with(|h| {
        assert_eq!(h.refusals.len(), 8);
        h.refusals.clear();
        h.caps.ime = true;
        h.caps.storage = true;
        h.caps.clipboard = true;
    });
    api::ime_begin(Rect::new(1.0, 2.0, 3.0, 4.0)).unwrap();
    mock::with(|h| assert_eq!(h.ime_rect, Some((1.0, 2.0, 3.0, 4.0))));
    api::ime_end().unwrap();
    api::state_put("k", "värde".as_bytes()).unwrap();
    let mut small = [0u8; 3];
    assert_eq!(
        api::state_get("k", &mut small).unwrap(),
        6,
        "full length reported"
    );
    assert_eq!(&small, b"v\xc3\xa4");
    assert_eq!(api::state_get("nej", &mut small), Err(VyError::Resource));
    api::state_delete("k").unwrap();
    assert_eq!(api::state_get("k", &mut small), Err(VyError::Resource));
    api::clipboard_write("hej").unwrap();
    let mut out = [0u8; 8];
    assert_eq!(api::clipboard_read(&mut out).unwrap(), 3);
    assert_eq!(api::navigate("ftp://x", false), Err(VyError::Arg));
    api::navigate("vy://exempel.se/", true).unwrap();
    api::link_register(3, Rect::new(0.0, 0.0, 1.0, 1.0), "https://exempel.se/").unwrap();
    api::request_frame();
    api::exit(3);
    mock::with(|h| {
        assert!(h.refusals.is_empty(), "{:?}", h.refusals);
        assert_eq!(h.navigations, [("vy://exempel.se/".into(), true)]);
        assert_eq!(h.links[0].5, "https://exempel.se/");
        assert_eq!(h.frame_requests, 1);
        assert_eq!(h.exit_code, Some(3));
    });
}

/// SPEC-5.2.9: `state_get` reports the value's full length even when `dst`
/// is shorter, and a key with no value is `Resource`, not an empty value;
/// `state_delete` makes the key missing again.
#[test]
fn spec_5_2_9_state_get_missing_key_is_resource() {
    mock::reset();
    mock::with(|h| h.caps.storage = true);
    let mut buf = [0u8; 2];
    assert_eq!(api::state_get("saknas", &mut buf), Err(VyError::Resource));
    api::state_put("k", b"abcd").unwrap();
    assert_eq!(
        api::state_get("k", &mut buf),
        Ok(4),
        "full length, two copied"
    );
    assert_eq!(&buf, b"ab");
    api::state_put("tom", b"").unwrap();
    assert_eq!(
        api::state_get("tom", &mut buf),
        Ok(0),
        "an empty value exists"
    );
    api::state_delete("k").unwrap();
    assert_eq!(api::state_get("k", &mut buf), Err(VyError::Resource));
    mock::with(|h| assert!(h.refusals.is_empty(), "{:?}", h.refusals));
}

/// SPEC-5.2.13, SPEC-5.2.10: a node id used twice in one frame is refused
/// as `Arg` and not counted, like the host does; links are forgotten at
/// `dl_begin` and the same id replaces the earlier rectangle.
#[test]
fn spec_5_2_13_duplicate_ids_and_links_per_frame() {
    mock::reset();
    let r = Rect::new(0.0, 0.0, 1.0, 1.0);
    let mut f = Frame::begin();
    f.sem.node_with_id(-5, Role::Button, r, "a").unwrap();
    assert_eq!(
        f.sem.node_with_id(-5, Role::Button, r, "b"),
        Err(VyError::Arg)
    );
    assert_eq!(f.sem.count(), 1);
    api::link_register(1, r, "vy://x/").unwrap();
    api::link_register(1, r, "vy://y/").unwrap();
    mock::with(|h| {
        assert_eq!(h.links.len(), 1);
        assert_eq!(h.links[0].5, "vy://y/");
        assert_eq!(h.refusals, ["sem_node: E_ARG"]);
    });
    f.end().unwrap();
    let f = Frame::begin();
    mock::with(|h| assert!(h.links.is_empty(), "dl_begin forgets the links"));
    assert_eq!(f.end(), Ok(0));
}

/// A memory source over a static arena for testing the heap natively.
struct ArenaGrow {
    base: usize,
    pages: usize,
    cap: usize,
}

impl Grow for ArenaGrow {
    fn grow(&mut self, pages: usize) -> Option<usize> {
        if self.pages + pages > self.cap {
            return None;
        }
        let addr = self.base + self.pages * PAGE;
        self.pages += pages;
        Some(addr)
    }
}

/// SPEC-4.2.4: the module heap grows page by page, reuses and merges freed
/// blocks, and fails cleanly (null, never a wild write) at the memory cap.
#[test]
fn spec_4_2_4_heap_within_cap() {
    let backing: Vec<u8> = alloc::vec![0; 4 * PAGE + ALIGN];
    let base = (backing.as_ptr() as usize).div_ceil(ALIGN) * ALIGN;
    let mut heap = Heap::new(ArenaGrow {
        base,
        pages: 0,
        cap: 4,
    });
    let small = Layout::from_size_align(24, 8).unwrap();
    // SAFETY: single-threaded test over memory `backing` owns.
    unsafe {
        let a = heap.alloc(small);
        let b = heap.alloc(small);
        assert!(!a.is_null() && !b.is_null());
        assert_eq!(a as usize % ALIGN, 0);
        assert_eq!(
            b as usize - a as usize,
            (24 + HEADER).div_ceil(ALIGN) * ALIGN
        );
        assert_eq!(heap.free_bytes(), PAGE - 2 * 48);
        heap.dealloc(a);
        heap.dealloc(b);
        assert_eq!(heap.free_blocks(), 1, "neighbours merged");
        assert_eq!(heap.free_bytes(), PAGE);
        let c = heap.alloc(small);
        assert_eq!(c, a, "freed memory is reused first");
        heap.dealloc(c);

        let big = Layout::from_size_align(2 * PAGE, 16).unwrap();
        let d = heap.alloc(big);
        assert!(!d.is_null(), "grew by three pages");
        assert!(heap.alloc(big).is_null(), "cap reached: 4 pages total");
        assert!(
            heap.alloc(Layout::from_size_align(8, 64).unwrap())
                .is_null(),
            "alignment above 16"
        );
        heap.dealloc(d);
        assert_eq!(heap.free_bytes(), 4 * PAGE);
        assert_eq!(heap.free_blocks(), 1);
    }
    drop(backing);
}

/// SPEC-6.3.1: `math` rounds like `std` does, with casts only, and `snap`
/// lands on physical-pixel boundaries at every scale: a 1 px rule at scale
/// 1.5 becomes two whole device pixels, never one and a half.
#[test]
fn spec_6_3_1_math_rounds_like_std_and_snaps_to_device_pixels() {
    let samples = [
        0.0,
        0.25,
        0.5,
        0.75,
        1.0,
        1.5,
        2.5,
        -0.25,
        -0.5,
        -0.75,
        -1.5,
        -2.5,
        76.5,
        120.3,
        1e7,
        -1e7,
        0.499_999_97,
        16_777_217.0,
        f32::MAX,
        f32::MIN,
    ];
    for v in samples {
        assert_eq!(math::floor(v), v.floor(), "floor {v}");
        assert_eq!(math::ceil(v), v.ceil(), "ceil {v}");
        assert_eq!(math::round(v), v.round(), "round {v}");
    }
    assert!(math::floor(f32::NAN).is_nan() && math::round(f32::NAN).is_nan());
    assert_eq!(math::ceil(f32::INFINITY), f32::INFINITY);

    for scale in [1.0, 1.5, 2.0, 0.75, 3.0] {
        for v in [0.0, 0.3, 76.5, 100.2, 1.0, 999.49] {
            let s = math::snap(v, scale);
            let device = s * scale;
            assert_eq!(device, device.round(), "snap({v}, {scale}) = {s}");
            assert!((s - v).abs() <= 0.5 / scale + 1e-4);
        }
    }
    assert_eq!(math::snap(76.5, 1.0), 77.0);
    assert_eq!(math::snap(76.4, 2.0), 76.5);
    assert_eq!(
        math::snap(1.0, 1.5) * 1.5,
        2.0,
        "a hairline is two device pixels at 1.5"
    );
    assert_eq!(math::snap(1.0, 2.0), 1.0);
    assert_eq!(math::snap(3.3, 0.0), 3.3, "no scale: unchanged");
    assert_eq!(math::snap(3.3, f32::NAN), 3.3);
}
