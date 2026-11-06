//! A group of radio buttons, one of which is selected.

use super::{ring, Input, Response, Theme, BOX, LABEL_GAP};
use crate::event::{button, key, Event};
use crate::frame::Frame;
use crate::sem::{NodeId, Role};
use crate::{Rect, VyError};
use alloc::string::String;

/// `len` radio buttons stacked in equal rows, exactly one selected. Each
/// option is a `radio` node of its own fixed id – `first_id`,
/// `first_id − 1`, … – and so its own Tab stop (SPEC-10.3.2); the selected
/// one's text ends in [`super::Words::selected`].
///
/// A primary press on a row selects it, Space selects the focused one.
/// Arrow keys do nothing: the host owns focus (SPEC-10.3.1), and moving the
/// selection away from the focused option would leave the ring behind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RadioGroup {
    first_id: i32,
    len: usize,
    /// Index of the selected option.
    pub selected: usize,
    focused: Option<usize>,
}

impl RadioGroup {
    /// `len` options with node ids `first_id`, `first_id − 1`, … (all
    /// negative, SPEC-5.2.13) and option `selected` selected.
    pub fn new(first_id: i32, len: usize, selected: usize) -> Self {
        Self {
            first_id,
            len,
            selected: selected.min(len.saturating_sub(1)),
            focused: None,
        }
    }

    /// Number of options.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True for a group without options.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The node id of option `i`.
    pub fn id(&self, i: usize) -> NodeId {
        NodeId(self.first_id - i as i32)
    }

    /// The option that has the host's focus.
    pub fn focused(&self) -> Option<usize> {
        self.focused
    }

    /// Row `i` of the group in `r`: `r` cut into `len` equal rows.
    pub fn option_rect(&self, r: Rect, i: usize) -> Rect {
        let h = r.h / self.len.max(1) as f32;
        Rect::new(r.x, r.y + i as f32 * h, r.w, h)
    }

    fn select(&mut self, ui: &mut Input<'_>, i: usize) -> Response {
        ui.take();
        if i == self.selected {
            return Response::Nothing;
        }
        self.selected = i;
        ui.respond(Response::Changed)
    }

    /// Handle an event for the group in `r`; `Changed` when another option
    /// was selected.
    pub fn event(&mut self, ui: &mut Input<'_>, r: Rect, _theme: &Theme) -> Response {
        if let Event::Focus { node, gained, .. } = ui.event() {
            let Some(i) = (0..self.len).find(|&i| self.id(i) == node) else {
                return Response::Nothing;
            };
            if gained {
                self.focused = Some(i);
            } else if self.focused == Some(i) {
                self.focused = None;
            }
            return ui.respond(Response::Redraw);
        }
        match ui.unhandled() {
            Some(Event::PointerDown { x, y, button, .. })
                if button == button::PRIMARY && r.contains(x, y) =>
            {
                match (0..self.len).find(|&i| self.option_rect(r, i).contains(x, y)) {
                    Some(i) => self.select(ui, i),
                    None => Response::Nothing,
                }
            }
            Some(Event::KeyDown {
                code: key::SPACE,
                repeat: false,
                ..
            }) => match self.focused {
                Some(i) => self.select(ui, i),
                None => Response::Nothing,
            },
            _ => Response::Nothing,
        }
    }

    /// Draw the options in `r` with `labels` (one per option), and register
    /// one `radio` node per option.
    pub fn draw(
        &self,
        f: &mut Frame,
        r: Rect,
        theme: &Theme,
        labels: &[&str],
    ) -> Result<(), VyError> {
        let p = &theme.palette;
        for i in 0..self.len {
            let row = self.option_rect(r, i);
            let label = labels.get(i).copied().unwrap_or("");
            let selected = i == self.selected;
            let c = Rect::new(row.x, row.y + (row.h - BOX) / 2.0, BOX, BOX);
            if self.focused == Some(i) {
                ring(f, c, BOX / 2.0, theme)?;
            }
            f.rect(c, BOX / 2.0, if selected { p.accent } else { p.border })?;
            f.rect(c.inset(2.0), BOX / 2.0 - 2.0, p.surface)?;
            if selected {
                f.rect(c.inset(6.0), BOX / 2.0 - 6.0, p.accent)?;
            }
            let lx = c.right() + LABEL_GAP;
            f.text(
                lx,
                row.y + (row.h - theme.line_h) / 2.0,
                (row.right() - lx).max(1.0),
                theme.text,
                label,
            )?;
            let mut text = String::from(label);
            if selected {
                text.push_str(theme.words.selected);
            }
            f.sem.node_with_id(self.id(i).0, Role::Radio, row, &text)?;
        }
        Ok(())
    }
}
