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
/// No name: the tests that are not about the field's label register the
/// field's own node and nothing else.
const BARE: Label<'static> = Label::hidden("");

/// `a` and `b` agree to a thousandth of a pixel (the ABI carries lengths
/// × 100, so `3.0 * CW` and the host's 28.8 differ in the last bit).
fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

fn send<L: Lines>(tb: &mut TextBox<L>, e: Event<'_>, r: Rect, t: &Theme) -> (Response, bool) {
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

    tb.set_text("a\tb\nc\u{7}d\r\ne\rf");
    assert_eq!(
        tb.text(),
        "a b cd e f",
        "line breaks are spaces on one line"
    );
    let lines = TextBox::multiline(-9).with_text("a\r\nb\rc\n\u{7}d");
    assert_eq!(lines.text(), "a\nb\nc\nd", "CRLF and CR are one break");
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
    let (ops, sem) = drawn(|f| tb.draw(f, r, &t, BARE, "Namn"));
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
    let (ops, sem) = drawn(|f| tb.draw(f, r, &t, BARE, "Namn"));
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
    let (ops, _) = drawn(|f| tb.draw(f, r, &t, BARE, ""));
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
    let (ops, _) = drawn(|f| tb.draw(f, r, &t, BARE, ""));
    assert_eq!(text_x(&ops), 22.0);

    let big = TextBox::multiline(-8)
        .with_max_len(10_000)
        .with_text(&"é".repeat(3000));
    let mut big = big;
    let (_, sem) = drawn(|f| big.draw(f, Rect::new(0.0, 0.0, 300.0, 100.0), &t, BARE, ""));
    assert_eq!(
        sem[0].text.len(),
        4096,
        "the node's text is capped (SPEC-9.3.2)"
    );
}

/// SPEC-10.1.1 (granskning 0010:4): a field has a name in the tree also
/// while it is empty. The `textbox` node's text is the value and the
/// placeholder is only drawn, so the field registers its [`Label`] as a
/// `text` node right before its own: over the rectangle the page drew the
/// label in, or, for a name that is drawn nowhere, at the field's corner
/// without a size, where no pointer finds it. The name is there with and
/// without a value, is cut to the node's 4 KB, and draws nothing.
#[test]
fn spec_10_1_1_textbox_label_is_in_the_tree() {
    let t = theme();
    let r = Rect::new(10.0, 40.0, 200.0, 40.0);
    let mut tb = TextBox::new(-7);
    let (bare_ops, sem) = drawn(|f| tb.draw(f, r, &t, BARE, "Nytt kort"));
    assert_eq!(sem.len(), 1, "an empty label registers nothing");

    let (ops, sem) = drawn(|f| tb.draw(f, r, &t, Label::hidden("Nytt kort i Pågår"), "Nytt kort"));
    assert_eq!(ops, bare_ops, "the label draws nothing");
    let nodes: Vec<(i32, i32, &str)> = sem
        .iter()
        .map(|n| (n.id, n.role, n.text.as_str()))
        .collect();
    assert_eq!(
        nodes,
        [
            (1, Role::Text as i32, "Nytt kort i Pågår"),
            (-7, Role::Textbox as i32, "")
        ],
        "the name, then the field"
    );
    assert_eq!(
        (sem[0].x, sem[0].y, sem[0].w, sem[0].h),
        (10.0, 40.0, 0.0, 0.0),
        "the field's corner, no size"
    );
    assert_eq!(sem[0].parent_id, sem[1].parent_id);

    // A label the page draws above the field is registered where it is.
    let above = Rect::new(10.0, 16.0, 200.0, 20.0);
    send(&mut tb, focus(-7, true), r, &t);
    send(&mut tb, typed("Anna"), r, &t);
    let (_, sem) = drawn(|f| tb.draw(f, r, &t, Label::shown(above, "Namn"), "Förnamn"));
    assert_eq!(
        (
            sem[0].role,
            sem[0].text.as_str(),
            sem[0].x,
            sem[0].y,
            sem[0].w,
            sem[0].h
        ),
        (Role::Text as i32, "Namn", 10.0, 16.0, 200.0, 20.0)
    );
    assert_eq!(
        (sem[1].id, sem[1].text.as_str()),
        (-7, "Anna"),
        "with a value too"
    );

    // The same for a multi-line field, and a name longer than a node takes.
    let mut note = TextBox::multiline(-8);
    let long = "n".repeat(5000);
    let (_, sem) = drawn(|f| note.draw(f, r, &t, Label::hidden(&long), ""));
    assert_eq!((sem[0].text.len(), sem[1].id), (4096, -8));
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
    drawn(|f| tb.draw(f, moved, &t, BARE, ""));
    mock::with(|h| assert_eq!(h.ime_rect, Some((0.0, 50.0, 300.0, 40.0))));

    let compose = Event::ImeCompose {
        text: "かな",
        caret: 3,
    };
    assert_eq!(send(&mut tb, compose, moved, &t), (Response::Redraw, true));
    assert_eq!(tb.text(), "Hej");
    let (ops, _) = drawn(|f| tb.draw(f, moved, &t, BARE, ""));
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
    let (ops, _) = drawn(|f| tb.draw(f, moved, &t, BARE, ""));
    assert!(
        !ops.iter().any(|o| matches!(o, Op::Rect { h: 1.0, .. })),
        "no underline after the commit"
    );

    send(&mut tb, focus(-1, false), moved, &t);
    mock::with(|h| assert_eq!(h.ime_rect, None));

    // Without `ime` the host refuses both calls and logs them; the field
    // goes on working.
    mock::with(|h| {
        h.caps.ime = false;
        h.refusals.clear();
    });
    send(&mut tb, focus(-1, true), r, &t);
    send(&mut tb, typed("!"), r, &t);
    send(&mut tb, focus(-1, false), r, &t);
    mock::with(|h| assert_eq!(h.refusals, ["ime_begin: E_DENIED", "ime_end: E_DENIED"]));
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
    let (ops, _) = drawn(|f| tb.draw(f, r, &t, BARE, ""));
    assert_eq!(
        tb.field.wrap.lines,
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
    assert_eq!(tb.field.scroll_y, 40.0);

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
    drawn(|f| tb.draw(f, r, &t, BARE, ""));
    assert_eq!(tb.field.scroll_y, 0.0);
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
    assert_eq!(tb.field.scroll_y, 20.0);
    let far = Event::Scroll {
        dx: 0.0,
        dy: 500.0,
        lines: false,
        modifiers: Modifiers(0),
    };
    send(&mut tb, far, r, &t);
    assert_eq!(tb.field.scroll_y, 6.0 * 20.0 - 60.0);
    assert_eq!(
        send(&mut tb, far, r, &t),
        (Response::Nothing, false),
        "at the end: the page may scroll"
    );
    send(&mut tb, Event::PointerLeave, r, &t);
    assert!(!send(&mut tb, wheel, r, &t).1);
    let (ops, _) = drawn(|f| tb.draw(f, r, &t, BARE, ""));
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
        drawn(|f| tb.draw(f, r, &t, BARE, ""));
        tb.field.wrap.lines.clone()
    };
    let mut tb = TextBox::multiline(-1).with_text("aaa bbb ccc\nsame\nsame\nddd eee fff ggg");
    send(&mut tb, focus(-1, true), r, &t);
    drawn(|f| tb.draw(f, r, &t, BARE, ""));
    assert_eq!(
        tb.field.wrap.lines,
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
        drawn(|f| tb.draw(f, r, &t, BARE, ""));
        assert_eq!(tb.text(), want);
        assert_eq!(tb.field.wrap.lines, fresh(want), "after {e:?}");
    }
    // End of the soft line "aaa bbb ": before its space; Delete takes the
    // space (one paragraph shrinks), Enter splits it in two.
    send(&mut tb, key(key::END), r, &t);
    send(&mut tb, key(key::DELETE), r, &t);
    drawn(|f| tb.draw(f, r, &t, BARE, ""));
    assert_eq!(tb.field.wrap.lines, fresh(tb.text()));
    send(&mut tb, key(key::ENTER), r, &t);
    drawn(|f| tb.draw(f, r, &t, BARE, ""));
    assert_eq!(tb.field.wrap.lines, fresh(tb.text()));
    assert_eq!(
        tb.text(),
        "aaa bbb\nccc\nsame\nsame\nddd eee fff ggg!\nx y z"
    );
}

/// The wrapping `wrap_with` replaced, kept as the definition of where
/// lines break: for a word wider than the line it bisects the whole rest
/// of the word, measuring a prefix of that length at each step, for every
/// line (quadratic in the word, granskning 0010:1).
fn wrap_exhaustive(
    width: f32,
    p: &str,
    measure: &impl Fn(&str) -> Result<f32, VyError>,
) -> Vec<(usize, usize)> {
    let mut lines = Vec::new();
    if p.is_empty() || width <= 0.0 {
        lines.push((0, p.len()));
        return lines;
    }
    let (mut start, mut line_w, mut i) = (0usize, 0.0f32, 0usize);
    while i < p.len() {
        let word_end = p[i..].find(' ').map_or(p.len(), |n| i + n);
        let rest = &p[word_end..];
        let token_end = word_end + rest.len() - rest.trim_start_matches(' ').len();
        let word_w = if word_end > i {
            measure(&p[i..word_end]).unwrap()
        } else {
            0.0
        };
        if i > start && line_w + word_w > width {
            lines.push((start, i));
            start = i;
            line_w = 0.0;
        }
        if i == start && word_w > width {
            let bounds: Vec<usize> = p[i..word_end]
                .char_indices()
                .skip(1)
                .map(|(o, _)| i + o)
                .chain(core::iter::once(word_end))
                .collect();
            let (mut lo, mut hi) = (0, bounds.len());
            while lo < hi {
                let mid = (lo + hi) / 2;
                if measure(&p[i..bounds[mid]]).unwrap() <= width {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            let k = bounds[lo.saturating_sub(1)];
            lines.push((i, k));
            start = k;
            i = k;
            continue;
        }
        line_w += if token_end == word_end {
            word_w
        } else {
            measure(&p[i..token_end]).unwrap()
        };
        i = token_end;
    }
    lines.push((start, p.len()));
    lines
}

/// xorshift64*, as the headless host's `random`: the same paragraphs in
/// every run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    state.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// SPEC-6.4.3 (granskning 0010:1): the lines break exactly where the
/// exhaustive search breaks them, for words far wider than the line, in
/// characters of one to four bytes and of every width (none, narrow,
/// wide), at widths from less than a character to more than the
/// paragraph.
#[test]
fn spec_6_4_3_textbox_wrap_breaks_where_the_exhaustive_search_does() {
    // Width models: the mock's (every character the same), and one where
    // a character's width depends on the character, some having none.
    let fixed = |s: &str| -> Result<f32, VyError> { Ok(s.chars().count() as f32 * CW) };
    let varied = |s: &str| -> Result<f32, VyError> {
        Ok(s.chars()
            .map(|c| match c {
                'i' | 'l' | '.' => 3.25,
                'W' | 'M' => 14.5,
                '\u{301}' | '\u{200B}' => 0.0,
                c if c.len_utf8() >= 3 => 16.0,
                ' ' => 4.0,
                _ => 8.75,
            })
            .sum())
    };
    let alphabet: Vec<char> = "ilW.Mabcåäö\u{301}\u{200B}語\u{10348} ".chars().collect();
    let mut seed = 0x5EED_5EED_0000_0001u64;
    let mut paragraphs: Vec<String> = [
        "",
        " ",
        "a",
        "abc",
        "   abc   ",
        "aaa bbb ccc ddd",
        "abcdefghijklmnop",
        "ab  abcdefghijklmnopqrstuvwxyz  yz ",
        "語語語語語語語語語語語語語語語語語語語語",
    ]
    .iter()
    .map(|s| String::from(*s))
    .collect();
    paragraphs.push("W".repeat(700));
    paragraphs.push("i".repeat(300) + &"W".repeat(300) + &"i".repeat(300));
    paragraphs.push("\u{10348}".repeat(200));
    paragraphs.push("e\u{301}".repeat(150));
    for n in 0..120 {
        // Mostly long words: a space once in forty characters or never.
        let len = 1 + (next(&mut seed) % 400) as usize;
        let spaces = n % 3 > 0;
        let p: String = (0..len)
            .map(|_| {
                let c = alphabet[(next(&mut seed) % alphabet.len() as u64) as usize];
                if c == ' ' && (!spaces || !next(&mut seed).is_multiple_of(3)) {
                    'W'
                } else {
                    c
                }
            })
            .collect();
        paragraphs.push(p);
    }
    let widths = [1.0, 9.6, 10.0, 28.8, 50.0, 96.0, 100.5, 333.0, 2000.0, 1e7];
    for p in &paragraphs {
        for width in widths {
            let got = wrap_with(width, p, &fixed).unwrap();
            assert_eq!(
                got,
                wrap_exhaustive(width, p, &fixed),
                "fixed widths, {width} px: {p:?}"
            );
            let got = wrap_with(width, p, &varied).unwrap();
            assert_eq!(
                got,
                wrap_exhaustive(width, p, &varied),
                "varied widths, {width} px: {p:?}"
            );
        }
    }
    // A width no character fits: one character a line, and (as before) an
    // empty line after a paragraph that ends with such a character.
    assert_eq!(
        wrap_with(5.0, "ab c", &fixed).unwrap(),
        [(0, 1), (1, 2), (2, 3), (3, 4), (4, 4)]
    );
    // A failing measure is the caller's error, not a panic.
    let refused = |_: &str| -> Result<f32, VyError> { Err(VyError::Arg) };
    assert_eq!(wrap_with(50.0, "abc", &refused), Err(VyError::Arg));
}

/// SPEC-5.3.3, SPEC-6.4.3 (granskning 0010:1): the host charges a frame
/// for every byte of text it shapes, so wrapping must not measure a long
/// word over and over. A word of 4 096 bytes without a space is measured
/// a few times its own length, however many lines it breaks into and
/// whatever its characters' widths, and a field holding it draws, takes a
/// press and is resized at a cost in the order of its text.
#[test]
fn spec_5_3_3_textbox_wrap_measures_in_the_order_of_the_text() {
    use core::cell::Cell;
    let measured = Cell::new(0usize);
    let fixed = |s: &str| -> Result<f32, VyError> {
        measured.set(measured.get() + s.len());
        Ok(s.chars().count() as f32 * CW)
    };
    // Lines of a few characters and of many, and one-, two- and four-byte
    // characters: the search starts where the line before ended.
    for (word, width) in [
        ("W".repeat(4096), 48.0),
        ("W".repeat(4096), 480.0),
        ("å".repeat(2048), 100.0),
        ("\u{10348}".repeat(1024), 300.0),
    ] {
        measured.set(0);
        let lines = wrap_with(width, &word, &fixed).unwrap();
        assert!(lines.len() > 8, "{} lines", lines.len());
        assert!(
            measured.get() <= 4 * word.len(),
            "{} bytes measured for a word of {} at {width} px",
            measured.get(),
            word.len()
        );
    }
    // Widths that change along the word: narrow, then wide, then narrow
    // characters, so the guess from the line before is wrong twice.
    let varied = |s: &str| -> Result<f32, VyError> {
        measured.set(measured.get() + s.len());
        Ok(s.chars().map(|c| if c == 'i' { 3.0 } else { 15.0 }).sum())
    };
    let word = "i".repeat(1500) + &"W".repeat(1096) + &"i".repeat(1500);
    measured.set(0);
    wrap_with(300.0, &word, &varied).unwrap();
    assert!(measured.get() <= 4 * word.len(), "{}", measured.get());
    // Ordinary text: every word and every token once, as before.
    let prose = "ord och mellanslag ".repeat(200);
    measured.set(0);
    wrap_with(300.0, &prose, &fixed).unwrap();
    assert!(measured.get() <= 2 * prose.len(), "{}", measured.get());

    // The field itself against the mock, which counts the bytes of every
    // `text_measure`, `text_caret` and `dl_text`.
    let t = theme();
    let r = Rect::new(0.0, 0.0, 24.0 + 40.0 * CW, 100.0);
    let word = "W".repeat(4096);
    let mut tb = TextBox::multiline(-1).with_text(&word);
    let cost = |f: &mut dyn FnMut()| {
        let before = mock::with(|h| h.text_bytes);
        f();
        mock::with(|h| h.text_bytes) - before
    };
    let first = cost(&mut || {
        drawn(|f| tb.draw(f, r, &t, BARE, ""));
    });
    assert_eq!(tb.field.wrap.lines.len(), 103, "40 characters a line");
    assert!(first <= 4 * 4096, "the first frame shaped {first} bytes");
    let again = cost(&mut || {
        drawn(|f| tb.draw(f, r, &t, BARE, ""));
    });
    assert!(again <= 400, "an unchanged field wraps nothing: {again}");
    send(&mut tb, focus(-1, true), r, &t);
    let press = cost(&mut || {
        send(&mut tb, down(100.0, 50.0), r, &t);
    });
    assert!(press <= 400, "a press reads one line: {press}");
    let narrower = Rect::new(0.0, 0.0, 24.0 + 25.0 * CW, 100.0);
    let resized = cost(&mut || {
        drawn(|f| tb.draw(f, narrower, &t, BARE, ""));
    });
    assert!(resized <= 4 * 4096, "a resize shaped {resized} bytes");
    let typed_cost = cost(&mut || {
        send(&mut tb, typed("x"), narrower, &t);
        drawn(|f| tb.draw(f, narrower, &t, BARE, ""));
    });
    assert!(typed_cost <= 4 * 4096, "a keystroke shaped {typed_cost}");
}
