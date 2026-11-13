//! A horizontal slider over a range of whole numbers.

use super::{focus_of, ring, Input, Response, Theme};
use crate::api::text_measure;
use crate::event::{button, key, Event};
use crate::frame::Frame;
use crate::num::push_int;
use crate::sem::{NodeId, Role};
use crate::{math, Rect, VyError};
use alloc::string::String;

/// Diameter of the knob.
pub const KNOB: f32 = 20.0;
/// Height of the track.
pub const TRACK_H: f32 = 6.0;
/// Width kept at the right of the rectangle for the value.
pub const VALUE_W: f32 = 48.0;

/// A slider: a track with a knob, the value right-aligned beside it, and a
/// `slider` node of a fixed id whose text is `"<label>: <value>"`.
///
/// The pointer drags it (press anywhere in the rectangle, move, release);
/// with the host's focus the arrow keys move it by a step, Page Up/Down by
/// ten steps, Home/End to the ends. Focused, the knob has a ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slider {
    id: i32,
    min: i32,
    max: i32,
    step: i32,
    value: i32,
    focused: bool,
    dragging: bool,
}

impl Slider {
    /// A slider with node id `id` (negative, SPEC-5.2.13) over
    /// `min..=max` in steps of 1, at `value` (clamped).
    pub fn new(id: i32, min: i32, max: i32, value: i32) -> Self {
        let max = max.max(min);
        Self {
            id,
            min,
            max,
            step: 1,
            value: value.clamp(min, max),
            focused: false,
            dragging: false,
        }
    }

    /// The same slider moving in steps of `step` (at least 1) from `min`.
    pub fn with_step(mut self, step: i32) -> Self {
        self.step = step.max(1);
        self.value = self.snap(self.value);
        self
    }

    /// The node id `Event::Focus` names.
    pub fn id(&self) -> NodeId {
        NodeId(self.id)
    }

    /// The value.
    pub fn value(&self) -> i32 {
        self.value
    }

    /// Set the value (clamped and snapped to a step); true when it changed.
    pub fn set_value(&mut self, value: i32) -> bool {
        let v = self.snap(value);
        let changed = v != self.value;
        self.value = v;
        changed
    }

    /// True while the slider has the host's focus.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// True while the pointer drags the knob.
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    fn snap(&self, v: i32) -> i32 {
        let (min, step) = (i64::from(self.min), i64::from(self.step));
        let v = i64::from(v.clamp(self.min, self.max));
        let steps = (v - min + step / 2) / step;
        (min + steps * step).min(i64::from(self.max)) as i32
    }

    /// The track inside the slider's rectangle `r`: the knob's centre runs
    /// from its left to its right end.
    pub fn track(r: Rect) -> Rect {
        let x0 = r.x + KNOB / 2.0;
        let x1 = (r.right() - VALUE_W - KNOB / 2.0).max(x0);
        Rect::new(x0, r.y + (r.h - TRACK_H) / 2.0, x1 - x0, TRACK_H)
    }

    /// The knob's centre x for the current value.
    pub fn knob_x(&self, r: Rect) -> f32 {
        let t = Self::track(r);
        let span = (self.max - self.min).max(1) as f32;
        t.x + t.w * (self.value - self.min) as f32 / span
    }

    /// The value under pointer x, snapped to a step.
    pub fn value_at(&self, r: Rect, x: f32) -> i32 {
        let t = Self::track(r);
        let frac = if t.w > 0.0 {
            ((x - t.x) / t.w).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let raw = self.min as f32 + frac * (self.max - self.min) as f32;
        self.snap(math::round(raw) as i32)
    }

    fn moved(&mut self, ui: &mut Input<'_>, value: i32) -> Response {
        ui.take();
        if self.set_value(value) {
            ui.respond(Response::Changed)
        } else {
            Response::Nothing
        }
    }

    /// Handle an event for the slider in `r`; `Changed` when the value moved.
    pub fn event(&mut self, ui: &mut Input<'_>, r: Rect, _theme: &Theme) -> Response {
        let event = ui.event();
        if let Some(gained) = focus_of(&event, self.id) {
            self.focused = gained;
            return ui.respond(Response::Redraw);
        }
        match ui.unhandled() {
            Some(Event::PointerDown { x, y, button, .. })
                if button == button::PRIMARY && r.contains(x, y) =>
            {
                self.dragging = true;
                let v = self.value_at(r, x);
                self.moved(ui, v)
            }
            Some(Event::PointerMove { x, buttons, .. }) if self.dragging => {
                if buttons & button::mask(button::PRIMARY) == 0 {
                    // Released outside the page: the drag is over.
                    self.dragging = false;
                    return Response::Nothing;
                }
                let v = self.value_at(r, x);
                self.moved(ui, v)
            }
            Some(Event::PointerUp { x, button, .. })
                if self.dragging && button == button::PRIMARY =>
            {
                self.dragging = false;
                let v = self.value_at(r, x);
                self.moved(ui, v)
            }
            Some(Event::KeyDown { code, .. }) if self.focused => {
                let page = self.step.saturating_mul(10);
                let target = match code {
                    key::LEFT | key::DOWN => self.value.saturating_sub(self.step),
                    key::RIGHT | key::UP => self.value.saturating_add(self.step),
                    key::PAGE_DOWN => self.value.saturating_sub(page),
                    key::PAGE_UP => self.value.saturating_add(page),
                    key::HOME => self.min,
                    key::END => self.max,
                    _ => return Response::Nothing,
                };
                self.moved(ui, target)
            }
            _ => Response::Nothing,
        }
    }

    /// Draw the slider in `r` and register its node, named `label`.
    pub fn draw(&self, f: &mut Frame, r: Rect, theme: &Theme, label: &str) -> Result<(), VyError> {
        let p = &theme.palette;
        let t = Self::track(r);
        let kx = self.knob_x(r);
        f.rect(t, TRACK_H / 2.0, p.track)?;
        f.rect(Rect::new(t.x, t.y, kx - t.x, t.h), TRACK_H / 2.0, p.accent)?;
        let knob = Rect::new(
            kx - KNOB / 2.0,
            t.y + TRACK_H / 2.0 - KNOB / 2.0,
            KNOB,
            KNOB,
        );
        if self.focused {
            ring(f, knob, KNOB / 2.0, theme)?;
        }
        f.rect(knob, KNOB / 2.0, p.accent)?;
        f.rect(knob.inset(3.0), KNOB / 2.0 - 3.0, p.surface)?;
        // By hand, so that a slider does not bring `core::fmt` into a page.
        let mut text = String::with_capacity(label.len() + 13);
        text.push_str(label);
        text.push_str(": ");
        let at = text.len();
        push_int(&mut text, i64::from(self.value));
        let value = &text[at..];
        let vw = text_measure(theme.text, 0.0, value)?.w;
        f.text(
            r.right() - vw,
            r.y + (r.h - theme.line_h) / 2.0,
            0.0,
            theme.text,
            value,
        )?;
        f.sem.node_with_id(self.id, Role::Slider, r, &text)?;
        Ok(())
    }
}
