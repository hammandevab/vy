//! A checkbox with its label.

use super::{focus_of, ring, Input, Response, Theme, BOX, LABEL_GAP};
use crate::event::{button, key, Event};
use crate::frame::{Frame, Path};
use crate::sem::{NodeId, Role};
use crate::{Rect, VyError};
use alloc::string::String;

/// A checkbox: a box with a tick, its label to the right, and a `checkbox`
/// node of a fixed id over the whole rectangle whose text is the label
/// plus [`super::Words::checked`] while checked.
///
/// Toggled by a primary press anywhere in its rectangle (the label too) and
/// by Space while it has the host's focus; focused, the box has a ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkbox {
    id: i32,
    /// True while checked.
    pub checked: bool,
    focused: bool,
}

impl Checkbox {
    /// A checkbox with node id `id` (negative, SPEC-5.2.13).
    pub fn new(id: i32, checked: bool) -> Self {
        Self {
            id,
            checked,
            focused: false,
        }
    }

    /// The node id `Event::Focus` names.
    pub fn id(&self) -> NodeId {
        NodeId(self.id)
    }

    /// True while the checkbox has the host's focus.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// The box inside the checkbox's rectangle `r`.
    pub fn box_rect(r: Rect) -> Rect {
        Rect::new(r.x, r.y + (r.h - BOX) / 2.0, BOX, BOX)
    }

    /// Handle an event for the checkbox in `r`; `Changed` when toggled.
    pub fn event(&mut self, ui: &mut Input<'_>, r: Rect, _theme: &Theme) -> Response {
        let event = ui.event();
        if let Some(gained) = focus_of(&event, self.id) {
            self.focused = gained;
            return ui.respond(Response::Redraw);
        }
        let toggle = match ui.unhandled() {
            Some(Event::PointerDown { x, y, button, .. }) => {
                button == button::PRIMARY && r.contains(x, y)
            }
            Some(Event::KeyDown {
                code: key::SPACE,
                repeat: false,
                ..
            }) => self.focused,
            _ => false,
        };
        if !toggle {
            return Response::Nothing;
        }
        ui.take();
        self.checked = !self.checked;
        ui.respond(Response::Changed)
    }

    /// Draw the checkbox in `r` with `label`, and register its node.
    pub fn draw(&self, f: &mut Frame, r: Rect, theme: &Theme, label: &str) -> Result<(), VyError> {
        let p = &theme.palette;
        let b = Self::box_rect(r);
        if self.focused {
            ring(f, b, 4.0, theme)?;
        }
        f.rect(b, 4.0, if self.checked { p.accent } else { p.border })?;
        f.rect(
            b.inset(1.0),
            3.0,
            if self.checked { p.accent } else { p.surface },
        )?;
        if self.checked {
            let mut tick = Path::new();
            tick.move_to(b.x + 5.0, b.y + 10.5)
                .line_to(b.x + 8.5, b.y + 14.0)
                .line_to(b.x + 15.0, b.y + 6.5);
            f.path(&tick, p.on_accent, 2.2)?;
        }
        let lx = b.right() + LABEL_GAP;
        f.text(
            lx,
            r.y + (r.h - theme.line_h) / 2.0,
            (r.right() - lx).max(1.0),
            theme.text,
            label,
        )?;
        let mut text = String::from(label);
        if self.checked {
            text.push_str(theme.words.checked);
        }
        f.sem.node_with_id(self.id, Role::Checkbox, r, &text)?;
        Ok(())
    }
}
