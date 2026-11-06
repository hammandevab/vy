//! `TextBox` against the mock host. Text is 16 px: 9.6 px per character,
//! 20 px lines; a field at x 0 puts its text at x 12 (the padding).

use super::super::tests::{down, drag, drawn, focus, key, key_with, theme, up};
use super::super::{Input, Palette, Response, Theme};
use super::*;
use crate::event::{key, Modifiers};
use crate::mock::{self, Op};
use crate::sem::Role;
use crate::VyError;

const CW: f32 = 9.6;
const SHIFT: i32 = 1;
const CTRL: i32 = 2;

/// `a` and `b` agree to a thousandth of a pixel (the ABI carries lengths
/// × 100, so `3.0 * CW` and the host's 28.8 differ in the last bit).
fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

fn send(tb: &mut TextBox, e: Event<'_>, r: Rect, t: &Theme) -> (Response, bool) {
    let mut ui = Input::new(e);
    let resp = tb.event(&mut ui, r, t);
    (resp, ui.taken())
}

fn typed(text: &str) -> Event<'_> {
    Event::TextInput { text }
}

fn focused(r: Rect, t: &Theme) -> TextBox {
    let mut tb = TextBox::new(-1);
    send(&mut tb, focus(-1, true), r, t);
    tb
}

/// SPEC-5.1.6, SPEC-5.2.6: committed text is inserted at the caret only
/// while the field has the host's focus; the editing keys move the caret by
/// character and (with Ctrl) by word, delete before and after it, Enter
/// activates a one-line field, Escape and unknown shortcuts are left to the
/// page, a printable key's `KEY_DOWN` is taken (its text comes as
/// `TEXT_INPUT`); control characters are dropped, a line break becomes a
/// space, and what passes `max_len` is cut at a character boundary.
#[test]
fn spec_5_1_6_textbox_typing_and_editing() {
    let t = theme();
    let r = Rect::new(0.0, 0.0, 300.0, 40.0);
    let mut tb = TextBox::new(-1);
    assert_eq!(send(&mut tb, typed("x"), r, &t), (Response::Nothing, false));
    assert_eq!(tb.text(), "", "not focused: not for this field");
    send(&mut tb, focus(-1, true), r, &t);
    assert!(tb.focused());
    assert_eq!(
        send(&mut tb, typed("Hej då"), r, &t),
        (Response::Changed, true)
    );
    assert_eq!((tb.text(), tb.caret()), ("Hej då", 7));
    assert_eq!(send(&mut tb, key(key::LEFT), r, &t).0, Response::Redraw);
    assert_eq!(tb.caret(), 5, "back over the two-byte å");
    send(&mut tb, typed("x"), r, &t);
    assert_eq!(tb.text(), "Hej dxå");
    send(&mut tb, key(key::BACKSPACE), r, &t);
    assert_eq!(tb.text(), "Hej då");
    assert_eq!(send(&mut tb, key(key::DELETE), r, &t).0, Response::Changed);
    assert_eq!(tb.text(), "Hej d");
    assert_eq!(
        send(&mut tb, key(key::DELETE), r, &t),
        (Response::Nothing, true)
    );
    send(&mut tb, key(key::HOME), r, &t);
    assert_eq!(tb.caret(), 0);
    assert_eq!(
        send(&mut tb, key(key::LEFT), r, &t),
        (Response::Nothing, true)
    );
    send(&mut tb, key_with(key::RIGHT, CTRL), r, &t);
    assert_eq!(tb.caret(), 4, "after the word and its space");
    send(&mut tb, key(key::END), r, &t);
    send(&mut tb, key_with(key::LEFT, CTRL), r, &t);
    assert_eq!(tb.caret(), 4);
    send(&mut tb, key(key::END), r, &t);
    send(&mut tb, key_with(key::BACKSPACE, CTRL), r, &t);
    assert_eq!(tb.text(), "Hej ");
    // Meta (Cmd) is the shortcut modifier too; Ctrl+Alt (AltGr) is not.
    send(&mut tb, key_with(key::HOME, 8), r, &t);
    assert_eq!(tb.caret(), 0);
    assert!(send(&mut tb, key_with(key::A, CTRL | 4), r, &t).1);
    assert!(
        tb.selection().is_empty(),
        "AltGr+A is a character, not select-all"
    );

    assert_eq!(
        send(&mut tb, key(key::ENTER), r, &t),
        (Response::Activated, true)
    );
    assert_eq!(
        send(&mut tb, key(key::ESCAPE), r, &t),
        (Response::Nothing, false)
    );
    assert_eq!(
        send(&mut tb, key_with(0x16, CTRL), r, &t),
        (Response::Nothing, false),
        "Ctrl+S is the page's"
    );
    assert_eq!(
        send(&mut tb, key(0x16), r, &t),
        (Response::Nothing, true),
        "S: text follows"
    );

    tb.set_text("a\tb\nc\u{7}d\r");
    assert_eq!(tb.text(), "a b cd");
    let mut short = TextBox::new(-2).with_max_len(5).with_text("åäöå");
    assert_eq!(short.text(), "åä", "cut at a character boundary");
    send(&mut short, focus(-2, true), r, &t);
    assert_eq!(send(&mut short, typed("x"), r, &t).0, Response::Changed);
    assert_eq!(
        send(&mut short, typed("y"), r, &t),
        (Response::Nothing, true),
        "full"
    );
    assert_eq!(short.text(), "åäx");
    assert_eq!(short.max_len(), 5);

    send(&mut tb, focus(-1, false), r, &t);
    assert!(!tb.focused());
    assert_eq!(
        send(&mut tb, key(key::LEFT), r, &t),
        (Response::Nothing, false)
    );
}

/// SPEC-10.2.2: the field's own selection – Shift with any movement, Ctrl+A,
/// a press and a drag with the primary button, Shift+press – is a byte
/// range of the value; typing replaces it, and an arrow collapses it.
#[test]
fn spec_10_2_2_textbox_selection() {
    let t = theme();
    let r = Rect::new(0.0, 0.0, 300.0, 40.0);
    let mut tb = focused(r, &t);
    send(&mut tb, typed("abc def"), r, &t);
    send(&mut tb, key_with(key::LEFT, SHIFT), r, &t);
    send(&mut tb, key_with(key::LEFT, SHIFT), r, &t);
    assert_eq!((tb.selection(), tb.selected_text()), (5..7, "ef"));
    send(&mut tb, key_with(key::LEFT, SHIFT | CTRL), r, &t);
    assert_eq!(tb.selected_text(), "def");
    assert_eq!(send(&mut tb, key(key::RIGHT), r, &t).0, Response::Redraw);
    assert_eq!(
        (tb.caret(), tb.selection()),
        (7, 7..7),
        "collapsed to the end"
    );
    assert_eq!(
        send(&mut tb, key_with(key::A, CTRL), r, &t),
        (Response::Redraw, true)
    );
    assert_eq!(tb.selected_text(), "abc def");
    send(&mut tb, typed("x"), r, &t);
    assert_eq!(tb.text(), "x");

    tb.set_text("abcdef");
    // A press between b and c (nearer c's left edge) puts the caret at 2.
    let (resp, taken) = send(&mut tb, down(12.0 + 2.0 * CW + 1.0, 20.0), r, &t);
    assert_eq!((resp, taken, tb.caret()), (Response::Redraw, true, 2));
    assert_eq!(
        send(&mut tb, drag(12.0 + 4.0 * CW + 4.0, 22.0), r, &t).0,
        Response::Redraw
    );
    assert_eq!(tb.selected_text(), "cd");
    assert_eq!(
        send(&mut tb, up(0.0, 0.0), r, &t),
        (Response::Nothing, true)
    );
    assert_eq!(
        send(&mut tb, drag(12.0, 22.0), r, &t),
        (Response::Nothing, false),
        "not dragging"
    );
    let shift_press = Event::PointerDown {
        x: 12.0 + 6.0 * CW + 30.0,
        y: 20.0,
        button: 1,
        modifiers: Modifiers(SHIFT),
    };
    send(&mut tb, shift_press, r, &t);
    assert_eq!(tb.selected_text(), "cdef", "past the end clamps to it");
    // Outside the field: not ours, and the drag is over.
    assert_eq!(
        send(&mut tb, down(400.0, 20.0), r, &t),
        (Response::Nothing, false)
    );
    tb.select(1..3);
    assert_eq!(tb.selected_text(), "bc");
}

/// SPEC-8.2.4, SPEC-5.2.12: Ctrl+C writes the selection with
/// `clipboard_write`, which succeeds without capability `clipboard` only
/// while the page handles the copy gesture's `KEY_DOWN`; Ctrl+X does the
/// same and removes the selection only when the write succeeded, so a
/// refused write never loses text; Ctrl+V is never the field's (the host
/// delivers the clipboard as `TEXT_INPUT`).
#[test]
fn spec_8_2_4_textbox_copy_and_cut() {
    let t = theme();
    let r = Rect::new(0.0, 0.0, 300.0, 40.0);
    let mut tb = focused(r, &t);
    send(&mut tb, typed("kopiera mig"), r, &t);
    assert_eq!(tb.last_copy(), None);
    assert_eq!(
        send(&mut tb, key_with(key::C, CTRL), r, &t),
        (Response::Nothing, true),
        "no selection"
    );
    assert_eq!(tb.last_copy(), None);
    tb.select(8..11);

    // Outside the gesture and without the capability the host refuses.
    assert_eq!(
        send(&mut tb, key_with(key::X, CTRL), r, &t).0,
        Response::Redraw
    );
    assert_eq!(tb.last_copy(), Some(Err(VyError::Denied)));
    assert_eq!(tb.text(), "kopiera mig", "a refused cut keeps the text");

    let (resp, taken) = mock::as_copy_gesture(|| send(&mut tb, key_with(key::C, CTRL), r, &t));
    assert_eq!((resp, taken), (Response::Redraw, true));
    assert_eq!(tb.last_copy(), Some(Ok(())));
    mock::with(|h| assert_eq!(h.clipboard, "mig"));
    tb.select(0..7);
    let resp = mock::as_copy_gesture(|| send(&mut tb, key_with(key::X, CTRL), r, &t).0);
    assert_eq!(resp, Response::Changed);
    assert_eq!(tb.text(), " mig");
    mock::with(|h| {
        assert_eq!(h.clipboard, "kopiera");
        assert!(!h.copy_gesture, "the gesture ends with the event");
    });
    assert_eq!(
        send(&mut tb, key_with(key::V, CTRL), r, &t),
        (Response::Nothing, false)
    );
    send(&mut tb, typed("klistrat"), r, &t);
    assert_eq!(
        tb.text(),
        "klistrat mig",
        "pasted at the caret the cut left"
    );
}

/// SPEC-10.3.3, SPEC-10.1.1, SPEC-9.3.2: unfocused, a field draws a 1 px
/// border, its surface and (clipped to it) the placeholder or the value;
/// focused, a 2 px accent border and the caret, with the selection behind
/// the text. Its `textbox` node carries the value, at most 4 KB of it. A
/// long one-line value scrolls sideways to keep the caret in view.
#[test]
fn spec_10_3_3_textbox_draw() {
    let t = theme();
    let p = Palette::LIGHT;
    let r = Rect::new(10.0, 20.0, 200.0, 40.0);
    let mut tb = TextBox::new(-7);
    let (ops, sem) = drawn(|f| tb.draw(f, r, &t, "Namn"));
    assert_eq!(
        ops[..3],
        [
            Op::Rect {
                x: 10.0,
                y: 20.0,
                w: 200.0,
                h: 40.0,
                radius: 6.0,
                color: p.border.0
            },
            Op::Rect {
                x: 11.0,
                y: 21.0,
                w: 198.0,
                h: 38.0,
                radius: 5.0,
                color: p.surface.0
            },
            Op::ClipPush {
                x: 11.0,
                y: 21.0,
                w: 198.0,
                h: 38.0,
                radius: 0.0
            },
        ]
    );
    assert!(matches!(&ops[3], Op::Text { x, y, style_id, text, .. }
        if (*x, *y, *style_id, text.as_str()) == (22.0, 30.0, t.placeholder.0, "Namn")));
    assert_eq!(ops[4], Op::ClipPop);
    assert_eq!(ops.len(), 5);
    assert_eq!(
        (sem[0].id, sem[0].role, sem[0].text.as_str()),
        (-7, Role::Textbox as i32, "")
    );

    send(&mut tb, focus(-7, true), r, &t);
    send(&mut tb, typed("Anna"), r, &t);
    tb.select(1..3);
    let (ops, sem) = drawn(|f| tb.draw(f, r, &t, "Namn"));
    assert!(matches!(ops[0], Op::Rect { color, .. } if color == p.accent.0));
    assert!(
        matches!(ops[1], Op::Rect { x, w, radius, .. } if (x, w, radius) == (12.0, 196.0, 4.0))
    );
    assert!(
        matches!(ops[3], Op::Rect { x, y: 30.0, w, h: 20.0, color, .. }
            if near(x, 22.0 + CW) && near(w, 2.0 * CW) && color == p.selection.0),
        "the selection, behind the text: {:?}",
        ops[3]
    );
    assert!(matches!(&ops[4], Op::Text { x, text, .. } if *x == 22.0 && text == "Anna"));
    assert!(
        matches!(ops[5], Op::Rect { x, y: 31.0, w: 1.0, h: 18.0, color, .. }
            if near(x, 22.0 + 3.0 * CW) && color == p.ink.0),
        "the caret at the selection's end: {:?}",
        ops[5]
    );
    assert_eq!(sem[0].text, "Anna");

    // 30 characters (288 px) in 176: End scrolls the text left so the
    // caret stays inside; Home scrolls back.
    tb.set_text(&"x".repeat(30));
    let (ops, _) = drawn(|f| tb.draw(f, r, &t, ""));
    let text_x = |ops: &[Op]| {
        ops.iter()
            .find_map(|o| match o {
                Op::Text { x, .. } => Some(*x),
                _ => None,
            })
            .unwrap()
    };
    let scrolled = text_x(&ops);
    assert!(near(scrolled, 22.0 - (30.0 * CW - 175.0)), "{scrolled}");
    let caret = ops.iter().rev().find_map(|o| match *o {
        Op::Rect { x, w: 1.0, .. } => Some(x),
        _ => None,
    });
    assert!(caret.is_some_and(|x| near(x, 22.0 + 175.0)), "{caret:?}");
    send(&mut tb, key(key::HOME), r, &t);
    let (ops, _) = drawn(|f| tb.draw(f, r, &t, ""));
    assert_eq!(text_x(&ops), 22.0);

    let big = TextBox::multiline(-8)
        .with_max_len(10_000)
        .with_text(&"é".repeat(3000));
    let mut big = big;
    let (_, sem) = drawn(|f| big.draw(f, Rect::new(0.0, 0.0, 300.0, 100.0), &t, ""));
    assert_eq!(
        sem[0].text.len(),
        4096,
        "the node's text is capped (SPEC-9.3.2)"
    );
}

/// SPEC-5.2.6: focus opens composition over the field (`ime_begin`) and
/// losing it closes it (`ime_end`); a field that moves while focused moves
/// the composition with it; `IME_COMPOSE` shows the composing text
/// underlined at the caret without changing the value, and the committed
/// `TEXT_INPUT` replaces it. Without `ime` the field still works.
#[test]
fn spec_5_2_6_textbox_ime() {
    let t = theme();
    mock::with(|h| h.caps.ime = true);
    let r = Rect::new(0.0, 0.0, 300.0, 40.0);
    let mut tb = TextBox::new(-1).with_text("Hej");
    send(&mut tb, focus(-1, true), r, &t);
    mock::with(|h| assert_eq!(h.ime_rect, Some((0.0, 0.0, 300.0, 40.0))));
    let moved = r.offset(0.0, 50.0);
    drawn(|f| tb.draw(f, moved, &t, ""));
    mock::with(|h| assert_eq!(h.ime_rect, Some((0.0, 50.0, 300.0, 40.0))));

    let compose = Event::ImeCompose {
        text: "かな",
        caret: 3,
    };
    assert_eq!(send(&mut tb, compose, moved, &t), (Response::Redraw, true));
    assert_eq!(tb.text(), "Hej");
    let (ops, _) = drawn(|f| tb.draw(f, moved, &t, ""));
    assert!(ops
        .iter()
        .any(|o| matches!(o, Op::Text { text, .. } if text == "Hejかな")));
    assert!(
        ops.iter()
            .any(|o| matches!(*o, Op::Rect { x, y: 78.0, w, h: 1.0, .. }
            if near(x, 12.0 + 3.0 * CW) && near(w, 2.0 * CW))),
        "the composition is underlined: {ops:?}"
    );
    send(&mut tb, typed("かな"), moved, &t);
    assert_eq!(tb.text(), "Hejかな");
    let (ops, _) = drawn(|f| tb.draw(f, moved, &t, ""));
    assert!(
        !ops.iter().any(|o| matches!(o, Op::Rect { h: 1.0, .. })),
        "no underline after the commit"
    );

    send(&mut tb, focus(-1, false), moved, &t);
    mock::with(|h| assert_eq!(h.ime_rect, None));

    // Without `ime` the refusal is the host's to log, once, and no
    // `ime_end` follows a composition that never began.
    mock::with(|h| {
        h.caps.ime = false;
        h.refusals.clear();
    });
    send(&mut tb, focus(-1, true), r, &t);
    send(&mut tb, typed("!"), r, &t);
    send(&mut tb, focus(-1, false), r, &t);
    mock::with(|h| assert_eq!(h.refusals, ["ime_begin: E_DENIED"]));
    assert_eq!(tb.text(), "Hejかな!");
}

/// SPEC-6.4.3, SPEC-5.2.5: a multi-line field wraps at word boundaries
/// (spaces stay at the end of their line, a word wider than the line is
/// broken), keeps line breaks, inserts one on Enter, moves between lines
/// at the same x with Up/Down, and scrolls down to the caret and with the
/// wheel under the pointer; only lines in view are drawn.
#[test]
fn spec_6_4_3_textbox_multiline() {
    let t = theme();
    // 10 characters per line: 2 × 12 padding + 96.
    let r = Rect::new(0.0, 0.0, 24.0 + 10.0 * CW, 80.0);
    let mut tb = TextBox::multiline(-3).with_text("aaa bbb ccc ddd\n\nabcdefghijklmnop");
    assert!(tb.is_multiline());
    let (ops, _) = drawn(|f| tb.draw(f, r, &t, ""));
    assert_eq!(
        tb.wrap.lines,
        [(0, 8), (8, 15), (16, 16), (17, 27), (27, 33)]
    );
    // 80 high: 60 px of text, three lines; the caret at the end scrolled
    // the last line into view.
    let lines: Vec<&str> = ops
        .iter()
        .filter_map(|o| match o {
            Op::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(lines, ["abcdefghij", "klmnop"], "the empty line has no op");
    assert_eq!(tb.scroll_y, 40.0);

    send(&mut tb, focus(-3, true), r, &t);
    send(&mut tb, key_with(key::HOME, CTRL), r, &t);
    send(&mut tb, key(key::RIGHT), r, &t);
    send(&mut tb, key(key::DOWN), r, &t);
    assert_eq!(tb.caret(), 9, "same x on the next line");
    send(&mut tb, key(key::DOWN), r, &t);
    assert_eq!(tb.caret(), 16, "the empty line");
    send(&mut tb, key(key::DOWN), r, &t);
    assert_eq!(tb.caret(), 18, "x remembered across the empty line");
    send(&mut tb, key(key::UP), r, &t);
    send(&mut tb, key(key::UP), r, &t);
    send(&mut tb, key(key::UP), r, &t);
    assert_eq!(tb.caret(), 1);
    send(&mut tb, key(key::UP), r, &t);
    assert_eq!(tb.caret(), 0, "up from the first line: the start");
    send(&mut tb, key(key::END), r, &t);
    assert_eq!(
        tb.caret(),
        7,
        "end of a soft line: before its trailing space"
    );
    send(&mut tb, key(key::END), r, &t);
    send(&mut tb, key_with(key::DOWN, SHIFT), r, &t);
    assert_eq!(tb.selected_text(), " ccc ddd");
    send(&mut tb, key(key::RIGHT), r, &t);
    assert_eq!(send(&mut tb, key(key::ENTER), r, &t).0, Response::Changed);
    assert_eq!(tb.text(), "aaa bbb ccc ddd\n\n\nabcdefghijklmnop");

    // A press on the second visual line (scrolled to the top first).
    send(&mut tb, key_with(key::HOME, CTRL), r, &t);
    drawn(|f| tb.draw(f, r, &t, ""));
    assert_eq!(tb.scroll_y, 0.0);
    send(&mut tb, down(12.0 + 2.0 * CW, 10.0 + 20.0 + 5.0), r, &t);
    assert_eq!(tb.caret(), 10);
    send(&mut tb, down(12.0 + 9.0 * CW, 15.0), r, &t);
    assert_eq!(tb.caret(), 7, "past the soft end: before the space");

    // The wheel scrolls only under the pointer, and never past the end.
    let wheel = Event::Scroll {
        dx: 0.0,
        dy: 1.0,
        lines: true,
        modifiers: Modifiers(0),
    };
    assert_eq!(
        send(&mut tb, wheel, r, &t),
        (Response::Nothing, false),
        "pointer elsewhere"
    );
    send(
        &mut tb,
        Event::PointerMove {
            x: 5.0,
            y: 5.0,
            buttons: 0,
            modifiers: Modifiers(0),
        },
        r,
        &t,
    );
    assert_eq!(send(&mut tb, wheel, r, &t), (Response::Redraw, true));
    assert_eq!(tb.scroll_y, 20.0);
    let far = Event::Scroll {
        dx: 0.0,
        dy: 500.0,
        lines: false,
        modifiers: Modifiers(0),
    };
    send(&mut tb, far, r, &t);
    assert_eq!(tb.scroll_y, 6.0 * 20.0 - 60.0);
    assert_eq!(
        send(&mut tb, far, r, &t),
        (Response::Nothing, false),
        "at the end: the page may scroll"
    );
    send(&mut tb, Event::PointerLeave, r, &t);
    assert!(!send(&mut tb, wheel, r, &t).1);
    let (ops, _) = drawn(|f| tb.draw(f, r, &t, ""));
    assert!(
        ops.iter()
            .any(|o| matches!(o, Op::Rect { w: 3.0, color, .. } if *color == t.palette.track.0)),
        "a scroll thumb when the text overflows"
    );
}

/// SPEC-6.4.3: paragraphs keep their wrapped lines across edits – the same
/// text wraps the same way whether it was wrapped fresh or reused after
/// typing, Enter, joining two paragraphs, or with repeated paragraphs.
#[test]
fn spec_6_4_3_textbox_rewrap_matches_fresh_wrap() {
    let t = theme();
    let r = Rect::new(0.0, 0.0, 24.0 + 10.0 * CW, 400.0);
    let fresh = |text: &str| {
        let mut tb = TextBox::multiline(-1).with_text(text);
        drawn(|f| tb.draw(f, r, &t, ""));
        tb.wrap.lines.clone()
    };
    let mut tb = TextBox::multiline(-1).with_text("aaa bbb ccc\nsame\nsame\nddd eee fff ggg");
    send(&mut tb, focus(-1, true), r, &t);
    drawn(|f| tb.draw(f, r, &t, ""));
    assert_eq!(
        tb.wrap.lines,
        fresh("aaa bbb ccc\nsame\nsame\nddd eee fff ggg")
    );
    let edits: [(Event<'static>, &str); 4] = [
        (typed("!"), "aaa bbb ccc\nsame\nsame\nddd eee fff ggg!"),
        (
            key(key::ENTER),
            "aaa bbb ccc\nsame\nsame\nddd eee fff ggg!\n",
        ),
        (
            typed("x y z"),
            "aaa bbb ccc\nsame\nsame\nddd eee fff ggg!\nx y z",
        ),
        (
            key_with(key::HOME, CTRL),
            "aaa bbb ccc\nsame\nsame\nddd eee fff ggg!\nx y z",
        ),
    ];
    for (e, want) in edits {
        send(&mut tb, e, r, &t);
        drawn(|f| tb.draw(f, r, &t, ""));
        assert_eq!(tb.text(), want);
        assert_eq!(tb.wrap.lines, fresh(want), "after {e:?}");
    }
    // End of the soft line "aaa bbb ": before its space; Delete takes the
    // space (one paragraph shrinks), Enter splits it in two.
    send(&mut tb, key(key::END), r, &t);
    send(&mut tb, key(key::DELETE), r, &t);
    drawn(|f| tb.draw(f, r, &t, ""));
    assert_eq!(tb.wrap.lines, fresh(tb.text()));
    send(&mut tb, key(key::ENTER), r, &t);
    drawn(|f| tb.draw(f, r, &t, ""));
    assert_eq!(tb.wrap.lines, fresh(tb.text()));
    assert_eq!(
        tb.text(),
        "aaa bbb\nccc\nsame\nsame\nddd eee fff ggg!\nx y z"
    );
}
