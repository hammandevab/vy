//! The components against the mock host: what they draw, which semantic
//! nodes they register and how they answer events. The mock's text model
//! is fixed (16 px text: 9.6 px per character, 20 px lines).

use super::*;
use crate::event::{key, Modifiers};
use crate::frame::Frame;
use crate::mock::{self, Op, SemNode};
use crate::sem::{NodeId, Role};
use crate::Color;
use alloc::vec::Vec;

/// Run `draw` inside a frame and return what the mock recorded.
pub(super) fn drawn(
    draw: impl FnOnce(&mut Frame) -> Result<(), VyError>,
) -> (Vec<Op>, Vec<SemNode>) {
    let mut f = Frame::begin();
    draw(&mut f).unwrap();
    f.end().unwrap();
    mock::with(|h| (h.frame_ops().to_vec(), h.sem.clone()))
}

pub(super) fn theme() -> Theme {
    mock::reset();
    Theme::new().unwrap()
}

pub(super) fn down(x: f32, y: f32) -> Event<'static> {
    Event::PointerDown {
        x,
        y,
        button: 1,
        modifiers: Modifiers(0),
    }
}

pub(super) fn up(x: f32, y: f32) -> Event<'static> {
    Event::PointerUp {
        x,
        y,
        button: 1,
        modifiers: Modifiers(0),
    }
}

pub(super) fn drag(x: f32, y: f32) -> Event<'static> {
    Event::PointerMove {
        x,
        y,
        buttons: 1,
        modifiers: Modifiers(0),
    }
}

pub(super) fn key_with(code: i32, modifiers: i32) -> Event<'static> {
    Event::KeyDown {
        code,
        modifiers: Modifiers(modifiers),
        repeat: false,
    }
}

pub(super) fn key(code: i32) -> Event<'static> {
    key_with(code, 0)
}

pub(super) fn focus(id: i32, gained: bool) -> Event<'static> {
    Event::Focus {
        node: NodeId(id),
        gained,
        keyboard: true,
    }
}

fn rects(ops: &[Op]) -> Vec<(f32, f32, f32, f32, u32)> {
    ops.iter()
        .filter_map(|o| match *o {
            Op::Rect {
                x, y, w, h, color, ..
            } => Some((x, y, w, h, color)),
            _ => None,
        })
        .collect()
}

fn texts(ops: &[Op]) -> Vec<(f32, f32, &str)> {
    ops.iter()
        .filter_map(|o| match o {
            Op::Text { x, y, text, .. } => Some((*x, *y, text.as_str())),
            _ => None,
        })
        .collect()
}

/// SPEC-5.2.5: a theme registers its four styles and takes the line height
/// from the host's measurement; colours come from the palette.
#[test]
fn spec_5_2_5_theme_registers_styles() {
    let t = theme();
    assert_eq!(t.line_h, 20.0, "16 px × 1.25 in the mock");
    mock::with(|h| {
        assert_eq!(h.styles.len(), 4);
        assert_eq!(h.styles[0].color, Palette::LIGHT.ink.0);
        assert_eq!(h.styles[1].color, Palette::LIGHT.placeholder.0);
        assert_eq!(h.styles[2].weight, 600);
        assert_eq!(h.styles[2].color, Palette::LIGHT.on_accent.0);
        assert_eq!(h.styles[3].color, Palette::LIGHT.accent.0);
    });
    let dark = Theme::custom(Palette::DARK, crate::Font::Serif, 20.0).unwrap();
    assert_eq!(dark.line_h, 25.0);
    assert_eq!(dark.words, Words::SV);
}

/// SPEC-10.3.1, SPEC-5.1.3: an input event is used by one component only –
/// a later one sees it taken – while a `FOCUS` event is never taken, so
/// every component learns where the host's focus went; the redraw answer is
/// the union of the responses.
#[test]
fn spec_10_3_1_input_takes_input_but_never_focus() {
    let mut ui = Input::new(down(1.0, 1.0));
    assert!(!ui.taken() && ui.unhandled().is_some() && !ui.redraw());
    ui.take();
    assert!(ui.taken() && ui.unhandled().is_none());
    assert_eq!(ui.respond(Response::Changed), Response::Changed);
    assert!(ui.redraw());

    let mut ui = Input::new(focus(-1, true));
    ui.take();
    assert!(!ui.taken(), "focus is for everyone");
    assert!(ui.unhandled().is_some());
    ui.respond(Response::Nothing);
    assert!(!ui.redraw());
    ui.set_redraw();
    assert!(ui.redraw());

    assert!(Response::Activated.redraw() && Response::Activated.activated());
    assert!(Response::Changed.changed() && !Response::Redraw.changed());
    assert!(!Response::Nothing.redraw());
}

/// SPEC-10.1.1, SPEC-10.3.3, SPEC-10.3.1: a button draws its fill and
/// centred label and registers a `button` node with its fixed id; focused
/// (from the host's `FOCUS`) it draws a ring first; a primary press inside,
/// or Enter/Space while focused, activates it and takes the event.
#[test]
fn spec_10_3_3_button() {
    let t = theme();
    let r = Rect::new(10.0, 20.0, 120.0, 40.0);
    let mut b = Button::new(-5);
    assert_eq!(b.id(), NodeId(-5));
    let (ops, sem) = drawn(|f| b.draw(f, r, &t, "OK"));
    assert_eq!(
        rects(&ops),
        [(10.0, 20.0, 120.0, 40.0, Palette::LIGHT.accent.0)]
    );
    // "OK" is 19.2 px wide: centred in 120, and (40 − 20) / 2 down.
    assert_eq!(texts(&ops), [(10.0 + (120.0 - 19.2) / 2.0, 30.0, "OK")]);
    assert_eq!(sem.len(), 1);
    assert_eq!((sem[0].id, sem[0].role), (-5, Role::Button as i32));
    assert_eq!(
        (sem[0].x, sem[0].w, sem[0].text.as_str()),
        (10.0, 120.0, "OK")
    );

    // Keys do nothing until the host focuses the button.
    let mut ui = Input::new(key(key::ENTER));
    assert_eq!(b.event(&mut ui, r, &t), Response::Nothing);
    assert!(!ui.taken());
    let mut ui = Input::new(focus(-6, true));
    assert_eq!(b.event(&mut ui, r, &t), Response::Nothing, "another node");
    let mut ui = Input::new(focus(-5, true));
    assert_eq!(b.event(&mut ui, r, &t), Response::Redraw);
    assert!(b.focused() && ui.redraw());
    let (ops, _) = drawn(|f| b.draw(f, r, &t, "OK"));
    let rs = rects(&ops);
    assert_eq!(
        rs[0],
        (7.0, 17.0, 126.0, 46.0, Palette::LIGHT.ring.0),
        "ring first"
    );
    assert_eq!(rs.len(), 2);
    for code in [key::ENTER, key::SPACE] {
        let mut ui = Input::new(key(code));
        assert_eq!(b.event(&mut ui, r, &t), Response::Activated);
        assert!(ui.taken());
    }
    let mut ui = Input::new(Event::KeyDown {
        code: key::SPACE,
        modifiers: Modifiers(0),
        repeat: true,
    });
    assert_eq!(b.event(&mut ui, r, &t), Response::Nothing, "auto-repeat");

    // The pointer: a primary press inside; not outside, not another button,
    // not an event an earlier component took.
    let mut ui = Input::new(down(11.0, 21.0));
    assert_eq!(b.event(&mut ui, r, &t), Response::Activated);
    assert!(ui.taken());
    let mut ui = Input::new(down(5.0, 21.0));
    assert_eq!(b.event(&mut ui, r, &t), Response::Nothing);
    let mut ui = Input::new(Event::PointerDown {
        x: 11.0,
        y: 21.0,
        button: 2,
        modifiers: Modifiers(0),
    });
    assert_eq!(b.event(&mut ui, r, &t), Response::Nothing);
    let mut ui = Input::new(down(11.0, 21.0));
    ui.take();
    assert_eq!(b.event(&mut ui, r, &t), Response::Nothing);

    let mut ui = Input::new(focus(-5, false));
    b.event(&mut ui, r, &t);
    assert!(!b.focused());

    // Secondary: an outline (accent, then surface inside), accent label.
    let s = Button::secondary(-7);
    let (ops, sem) = drawn(|f| s.draw(f, r, &t, "Avbryt"));
    let rs = rects(&ops);
    assert_eq!(rs[0].4, Palette::LIGHT.accent.0);
    assert_eq!(rs[1], (11.0, 21.0, 118.0, 38.0, Palette::LIGHT.surface.0));
    assert!(matches!(ops[2], Op::Text { style_id, .. } if style_id == t.button_secondary.0));
    assert_eq!(sem[0].text, "Avbryt");
}

/// SPEC-10.1.1, SPEC-10.3.3: a checkbox toggles on a press anywhere in its
/// row and on Space while focused; its node is a `checkbox` over the whole
/// row whose text says the state; checked, it draws a tick; focused, a ring
/// around the box.
#[test]
fn spec_10_1_1_checkbox() {
    let t = theme();
    let r = Rect::new(0.0, 0.0, 300.0, 28.0);
    let mut c = Checkbox::new(-3, false);
    let (ops, sem) = drawn(|f| c.draw(f, r, &t, "Nyheter"));
    let rs = rects(&ops);
    assert_eq!(rs[0], (0.0, 4.0, 20.0, 20.0, Palette::LIGHT.border.0));
    assert_eq!(rs[1], (1.0, 5.0, 18.0, 18.0, Palette::LIGHT.surface.0));
    assert!(!ops.iter().any(|o| matches!(o, Op::Path { .. })));
    assert_eq!(texts(&ops), [(30.0, 4.0, "Nyheter")]);
    assert_eq!((sem[0].id, sem[0].role), (-3, Role::Checkbox as i32));
    assert_eq!(
        (sem[0].w, sem[0].h, sem[0].text.as_str()),
        (300.0, 28.0, "Nyheter")
    );

    // A press on the label toggles too, and takes the event.
    let mut ui = Input::new(down(200.0, 10.0));
    assert_eq!(c.event(&mut ui, r, &t), Response::Changed);
    assert!(c.checked && ui.taken());
    let (ops, sem) = drawn(|f| c.draw(f, r, &t, "Nyheter"));
    assert_eq!(rects(&ops)[0].4, Palette::LIGHT.accent.0);
    assert!(ops
        .iter()
        .any(|o| matches!(o, Op::Path { stroke_w, .. } if *stroke_w == 2.2)));
    assert_eq!(sem[0].text, "Nyheter (markerad)");

    let mut ui = Input::new(key(key::SPACE));
    assert_eq!(c.event(&mut ui, r, &t), Response::Nothing, "not focused");
    c.event(&mut Input::new(focus(-3, true)), r, &t);
    assert_eq!(
        c.event(&mut Input::new(key(key::SPACE)), r, &t),
        Response::Changed
    );
    assert!(!c.checked);
    let (ops, _) = drawn(|f| c.draw(f, r, &t, "Nyheter"));
    assert_eq!(
        rects(&ops)[0],
        (-3.0, 1.0, 26.0, 26.0, Palette::LIGHT.ring.0)
    );
    assert_eq!(
        c.event(&mut Input::new(key(key::ENTER)), r, &t),
        Response::Nothing
    );

    let mut en = t;
    en.words = Words::EN;
    c.checked = true;
    let (_, sem) = drawn(|f| c.draw(f, r, &en, "News"));
    assert_eq!(sem[0].text, "News (checked)");
}

/// SPEC-10.1.1, SPEC-10.3.2: a radio group registers one `radio` node per
/// option with ids `first, first − 1, …` (each its own Tab stop); a press
/// on a row selects it, Space selects the focused option, and the
/// selected option's text says so.
#[test]
fn spec_10_3_2_radio_group() {
    let t = theme();
    let r = Rect::new(0.0, 100.0, 200.0, 96.0);
    let labels = ["Liten", "Mellan", "Stor"];
    let mut g = RadioGroup::new(-20, 3, 1);
    assert_eq!((g.len(), g.id(2)), (3, NodeId(-22)));
    assert_eq!(g.option_rect(r, 2), Rect::new(0.0, 164.0, 200.0, 32.0));
    let (ops, sem) = drawn(|f| g.draw(f, r, &t, &labels));
    let ids: Vec<i32> = sem.iter().map(|n| n.id).collect();
    assert_eq!(ids, [-20, -21, -22]);
    assert!(sem
        .iter()
        .all(|n| n.role == Role::Radio as i32 && n.h == 32.0));
    assert_eq!(sem[1].text, "Mellan (vald)");
    assert_eq!(sem[0].text, "Liten");
    // The selected circle has the accent ring and dot; the others do not.
    let dots = rects(&ops)
        .iter()
        .filter(|r| r.2 == 8.0 && r.4 == Palette::LIGHT.accent.0)
        .count();
    assert_eq!(dots, 1);

    let mut ui = Input::new(down(150.0, 170.0));
    assert_eq!(g.event(&mut ui, r, &t), Response::Changed);
    assert!(ui.taken());
    assert_eq!(g.selected, 2);
    let mut ui = Input::new(down(150.0, 170.0));
    assert_eq!(
        g.event(&mut ui, r, &t),
        Response::Nothing,
        "already selected"
    );
    assert!(ui.taken());
    assert_eq!(
        g.event(&mut Input::new(down(250.0, 170.0)), r, &t),
        Response::Nothing
    );

    assert_eq!(
        g.event(&mut Input::new(focus(-20, true)), r, &t),
        Response::Redraw
    );
    assert_eq!(g.focused(), Some(0));
    assert_eq!(
        g.event(&mut Input::new(key(key::DOWN)), r, &t),
        Response::Nothing
    );
    assert_eq!(
        g.event(&mut Input::new(key(key::SPACE)), r, &t),
        Response::Changed
    );
    assert_eq!(g.selected, 0);
    let (ops, _) = drawn(|f| g.draw(f, r, &t, &labels));
    assert_eq!(
        rects(&ops)[0],
        (-3.0, 103.0, 26.0, 26.0, Palette::LIGHT.ring.0)
    );
    g.event(&mut Input::new(focus(-20, false)), r, &t);
    assert_eq!(g.focused(), None);
    assert_eq!(
        g.event(&mut Input::new(focus(-99, true)), r, &t),
        Response::Nothing
    );
}

/// SPEC-10.1.1, SPEC-A.2.2: a slider follows a primary-button drag (press,
/// moves with the button held, release), moves by a step on the arrow
/// keys, ten on Page Up/Down and to the ends on Home/End while focused,
/// snaps to its step, and its node's text is "label: value".
#[test]
fn spec_a_2_2_slider_drag_and_keys() {
    let t = theme();
    // Track from x 10 to 10 + 100: the knob's centre runs over 100 px.
    let r = Rect::new(0.0, 0.0, 100.0 + 20.0 + slider::VALUE_W, 30.0);
    let mut s = Slider::new(-4, 0, 100, 40);
    assert_eq!(Slider::track(r), Rect::new(10.0, 12.0, 100.0, 6.0));
    assert_eq!(s.knob_x(r), 50.0);
    let (ops, sem) = drawn(|f| s.draw(f, r, &t, "Volym"));
    assert_eq!((sem[0].id, sem[0].role), (-4, Role::Slider as i32));
    assert_eq!(sem[0].text, "Volym: 40");
    let value = texts(&ops)[0];
    assert_eq!((value.0, value.2), (r.right() - 19.2, "40"));

    let mut ui = Input::new(down(85.0, 15.0));
    assert_eq!(s.event(&mut ui, r, &t), Response::Changed);
    assert!(s.dragging() && ui.taken());
    assert_eq!(s.value(), 75);
    assert_eq!(
        s.event(&mut Input::new(drag(200.0, 40.0)), r, &t),
        Response::Changed
    );
    assert_eq!(s.value(), 100, "clamped at the end, the pointer may leave");
    assert_eq!(
        s.event(&mut Input::new(drag(300.0, 40.0)), r, &t),
        Response::Nothing
    );
    assert_eq!(
        s.event(&mut Input::new(up(-50.0, 0.0)), r, &t),
        Response::Changed
    );
    assert_eq!((s.value(), s.dragging()), (0, false));
    // A move after release does nothing; neither does a move without the
    // button, which ends a drag the page never saw released.
    assert_eq!(
        s.event(&mut Input::new(drag(60.0, 15.0)), r, &t),
        Response::Nothing
    );
    s.event(&mut Input::new(down(60.0, 15.0)), r, &t);
    let released = Event::PointerMove {
        x: 90.0,
        y: 15.0,
        buttons: 0,
        modifiers: Modifiers(0),
    };
    assert_eq!(s.event(&mut Input::new(released), r, &t), Response::Nothing);
    assert!(!s.dragging());
    assert_eq!(s.value(), 50);

    assert_eq!(
        s.event(&mut Input::new(key(key::RIGHT)), r, &t),
        Response::Nothing
    );
    s.event(&mut Input::new(focus(-4, true)), r, &t);
    let steps = [
        (key::RIGHT, 51),
        (key::UP, 52),
        (key::LEFT, 51),
        (key::DOWN, 50),
        (key::PAGE_UP, 60),
        (key::PAGE_DOWN, 50),
        (key::END, 100),
        (key::HOME, 0),
    ];
    for (code, want) in steps {
        assert_eq!(
            s.event(&mut Input::new(key(code)), r, &t),
            Response::Changed
        );
        assert_eq!(s.value(), want, "key {code}");
    }
    assert_eq!(
        s.event(&mut Input::new(key(key::HOME)), r, &t),
        Response::Nothing
    );
    let (ops, _) = drawn(|f| s.draw(f, r, &t, "Volym"));
    assert!(rects(&ops).iter().any(|r| r.4 == Palette::LIGHT.ring.0));

    let mut stepped = Slider::new(-8, 0, 100, 42).with_step(5);
    assert_eq!(stepped.value(), 40);
    assert!(stepped.set_value(48));
    assert_eq!(stepped.value(), 50);
    assert_eq!(stepped.value_at(r, 10.0 + 33.0), 35);
    assert!(stepped.set_value(500));
    assert_eq!(stepped.value(), 100);
    assert_eq!(
        Slider::new(-9, 10, 5, 0).value(),
        10,
        "an empty range is its min"
    );
}

/// SPEC-2.1.3: the stacks only add: rectangles one after another with gap
/// and padding, the height a card around them needs, what is left, and
/// equal columns.
#[test]
fn spec_2_1_3_stacks() {
    let area = Rect::new(10.0, 20.0, 300.0, 400.0);
    let mut c = Column::new(area, 16.0, 8.0);
    assert_eq!(c.width(), 268.0);
    assert_eq!(c.rest(), Rect::new(26.0, 36.0, 268.0, 368.0));
    assert_eq!(c.next(30.0), Rect::new(26.0, 36.0, 268.0, 30.0));
    c.space(4.0);
    assert_eq!(c.next(40.0), Rect::new(26.0, 78.0, 268.0, 40.0));
    assert_eq!(c.bottom(), 118.0);
    assert_eq!(c.height(), 118.0 + 16.0 - 20.0);
    assert_eq!(c.rest().y, 126.0);
    assert_eq!(c.next(-5.0).h, 0.0);

    let mut r = Row::new(area, 10.0, 6.0);
    assert_eq!(r.height(), 380.0);
    assert_eq!(r.next(100.0), Rect::new(20.0, 30.0, 100.0, 380.0));
    assert_eq!(r.next(50.0).x, 126.0);
    assert_eq!(r.right(), 176.0);
    assert_eq!(r.width(), 176.0 + 10.0 - 10.0);
    assert_eq!(r.rest(), Rect::new(182.0, 30.0, 118.0, 380.0));

    let [a, b, c3] = split::<3>(Rect::new(0.0, 5.0, 320.0, 50.0), 10.0);
    assert_eq!(
        (a.x, a.w, b.x, c3.x, c3.right()),
        (0.0, 100.0, 110.0, 220.0, 320.0)
    );
    assert_eq!(split::<1>(area, 10.0)[0], area);
    let _: Color = Palette::DARK.accent;
}
