//! A push button.

use super::{focus_of, ring, Input, Response, Theme, RADIUS};
use crate::api::text_measure;
use crate::event::{button, key, Event};
use crate::frame::Frame;
use crate::sem::{NodeId, Role};
use crate::{Rect, VyError};

/// How a [`Button`] looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// Filled with the accent colour: the page's main action.
    Primary,
    /// Outlined in the accent colour on the surface.
    Secondary,
}

/// A push button with a `button` node of a fixed id.
///
/// Activated by a primary press inside it (on `POINTER_DOWN`, as in every
/// reference page) and by Enter or Space while it has the host's focus.
/// Focused, it has a ring (SPEC-10.3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Button {
    id: i32,
    kind: ButtonKind,
    focused: bool,
}

impl Button {
    /// A primary button with node id `id` (negative, SPEC-5.2.13).
    pub fn new(id: i32) -> Self {
        Self {
            id,
            kind: ButtonKind::Primary,
            focused: false,
        }
    }

    /// A secondary button with node id `id`.
    pub fn secondary(id: i32) -> Self {
        Self {
            kind: ButtonKind::Secondary,
            ..Self::new(id)
        }
    }

    /// The node id `Event::Focus` names.
    pub fn id(&self) -> NodeId {
        NodeId(self.id)
    }

    /// True while the button has the host's focus.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// Handle an event for the button in `r`.
    pub fn event(&mut self, ui: &mut Input<'_>, r: Rect, _theme: &Theme) -> Response {
        let event = ui.event();
        if let Some(gained) = focus_of(&event, self.id) {
            self.focused = gained;
            return ui.respond(Response::Redraw);
        }
        let activated = match ui.unhandled() {
            Some(Event::PointerDown { x, y, button, .. }) => {
                button == button::PRIMARY && r.contains(x, y)
            }
            Some(Event::KeyDown {
                code: key::ENTER | key::SPACE,
                repeat: false,
                ..
            }) => self.focused,
            _ => false,
        };
        if !activated {
            return Response::Nothing;
        }
        ui.take();
        ui.respond(Response::Activated)
    }

    /// Draw the button in `r` with `label` centred, and register its node.
    pub fn draw(&self, f: &mut Frame, r: Rect, theme: &Theme, label: &str) -> Result<(), VyError> {
        let p = &theme.palette;
        if self.focused {
            ring(f, r, RADIUS, theme)?;
        }
        f.rect(r, RADIUS, p.accent)?;
        let style = match self.kind {
            ButtonKind::Primary => theme.button,
            ButtonKind::Secondary => {
                f.rect(r.inset(1.0), RADIUS - 1.0, p.surface)?;
                theme.button_secondary
            }
        };
        let tw = text_measure(style, 0.0, label)?.w;
        f.text(
            r.x + (r.w - tw) / 2.0,
            r.y + (r.h - theme.line_h) / 2.0,
            0.0,
            style,
            label,
        )?;
        f.sem.node_with_id(self.id, Role::Button, r, label)?;
        Ok(())
    }
}
