//! A vertical scroll area with wheel, keys and a scroll bar.

use super::{Input, Response, Theme, RING};
use crate::event::{button, key, Event};
use crate::frame::Frame;
use crate::{Rect, VyError};

/// Pixels one wheel line or arrow key scrolls.
pub const LINE_PX: f32 = 40.0;
/// Width of the scroll bar.
pub const BAR_W: f32 = 6.0;
/// Shortest scroll-bar thumb.
const MIN_THUMB: f32 = 24.0;

/// Keyboard activity of a component without a focusable role (a list, a
/// scroll area; SPEC-10.3.2): active after a press inside it that gave no
/// node the host's focus, until a node gains focus or a press lands
/// outside.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Activity {
    /// The node the host's focus is on, as the `FOCUS` events said.
    host_focus: Option<i32>,
    /// Takes keys now.
    pub active: bool,
}

impl Activity {
    /// Follow a `FOCUS` event; true when the activity changed.
    pub fn focus(&mut self, node: i32, gained: bool) -> bool {
        let was = self.active;
        if gained {
            self.host_focus = Some(node);
            self.active = false;
        } else if self.host_focus == Some(node) {
            self.host_focus = None;
        }
        was != self.active
    }

    /// Follow a press (the host's `FOCUS` events for it came first,
    /// SPEC-10.3.1); true when the activity changed.
    pub fn press(&mut self, inside: bool) -> bool {
        let was = self.active;
        self.active = inside && self.host_focus.is_none();
        was != self.active
    }
}

/// A vertical scroll area over content `content_h` high, shown in a
/// viewport. The page lays its content out with [`ScrollView::offset`]
/// subtracted (`y = viewport.y + content_y - scroll.offset()`), so hit
/// tests and drawing agree, and draws it inside [`ScrollView::draw`],
/// which clips to the viewport and adds the scroll bar.
///
/// - The wheel scrolls it while the pointer is over the viewport (or its
///   position is unknown); at either end the wheel is left for an outer
///   area.
/// - The scroll bar's thumb can be dragged; a press on the track pages.
/// - Keys while *active* (a press inside that gave no node the host's
///   focus): arrows by [`LINE_PX`], Page Up/Down and Space (Shift: up) by a
///   viewport, Home/End to the ends. Active, it has a ring.
/// - It registers no node of its own; the content registers its nodes,
///   off-screen ones too (SPEC-10.1.1). When the host moves keyboard focus
///   to a control inside, the page calls [`ScrollView::reveal`] with its
///   rectangle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollView {
    offset: f32,
    pub(crate) activity: Activity,
    /// Whether the pointer was last seen over the viewport; `None` when
    /// unknown.
    hover: Option<bool>,
    /// While the thumb is dragged: where in it the pointer grabbed it.
    grab: Option<f32>,
}

impl ScrollView {
    /// A scroll area at the top, not active.
    pub fn new() -> Self {
        Self::default()
    }

    /// How far the content is scrolled up, in logical pixels.
    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// True while the area takes keys.
    pub fn active(&self) -> bool {
        self.activity.active
    }

    /// Make the area take keys (the page's main area, say) or stop.
    pub fn set_active(&mut self, active: bool) {
        self.activity.active = active;
    }

    /// True while the scroll bar's thumb is dragged.
    pub fn dragging(&self) -> bool {
        self.grab.is_some()
    }

    /// The largest offset for content `content_h` high in `viewport`.
    pub fn max_offset(viewport: Rect, content_h: f32) -> f32 {
        (content_h - viewport.h).max(0.0)
    }

    /// Scroll to `y` (clamped); true when the offset changed.
    pub fn scroll_to(&mut self, y: f32, viewport: Rect, content_h: f32) -> bool {
        let next = y.clamp(0.0, Self::max_offset(viewport, content_h));
        let changed = next != self.offset;
        self.offset = next;
        changed
    }

    /// Scroll by `dy` (positive scrolls the content up); true when it moved.
    pub fn scroll_by(&mut self, dy: f32, viewport: Rect, content_h: f32) -> bool {
        self.scroll_to(self.offset + dy, viewport, content_h)
    }

    /// Keep the offset inside the content, after a resize or when the
    /// content shrank.
    pub fn clamp(&mut self, viewport: Rect, content_h: f32) {
        self.scroll_to(self.offset, viewport, content_h);
    }

    /// Scroll the least that brings `r` (in window coordinates, as laid out
    /// with the current offset) fully into the viewport; true when it moved.
    pub fn reveal(&mut self, r: Rect, viewport: Rect, content_h: f32) -> bool {
        let dy = if r.y < viewport.y {
            r.y - viewport.y
        } else if r.bottom() > viewport.bottom() {
            (r.bottom() - viewport.bottom()).min(r.y - viewport.y)
        } else {
            0.0
        };
        self.scroll_by(dy, viewport, content_h)
    }

    /// The scroll bar's track, when the content is taller than the viewport.
    pub fn track(viewport: Rect, content_h: f32) -> Option<Rect> {
        (content_h > viewport.h).then(|| {
            Rect::new(
                viewport.right() - BAR_W - 2.0,
                viewport.y + 2.0,
                BAR_W,
                (viewport.h - 4.0).max(0.0),
            )
        })
    }

    /// The thumb inside the track.
    pub fn thumb(&self, viewport: Rect, content_h: f32) -> Option<Rect> {
        let t = Self::track(viewport, content_h)?;
        let h = (t.h * viewport.h / content_h).clamp(MIN_THUMB.min(t.h), t.h);
        let max = Self::max_offset(viewport, content_h);
        let y = t.y + (t.h - h) * if max > 0.0 { self.offset / max } else { 0.0 };
        Some(Rect::new(t.x, y, t.w, h))
    }

    /// True when `(x, y)` is on the scroll bar (with a little slack).
    pub(crate) fn on_bar(viewport: Rect, content_h: f32, x: f32, y: f32) -> bool {
        Self::track(viewport, content_h).is_some_and(|t| viewport.contains(x, y) && x >= t.x - 4.0)
    }

    fn page(viewport: Rect) -> f32 {
        (viewport.h - LINE_PX).max(LINE_PX)
    }

    /// Handle an event for the area `viewport` over content `content_h`
    /// high; give it to the components inside first.
    pub fn event(
        &mut self,
        ui: &mut Input<'_>,
        viewport: Rect,
        _theme: &Theme,
        content_h: f32,
    ) -> Response {
        let event = ui.event();
        if let Event::Focus { node, gained, .. } = event {
            let changed = self.activity.focus(node.0, gained);
            return ui.respond(if changed {
                Response::Redraw
            } else {
                Response::Nothing
            });
        }
        let taken = ui.taken();
        let response = match event {
            Event::PointerDown { x, y, button, .. } => {
                let inside = viewport.contains(x, y);
                let was = self.activity.active;
                if !taken
                    && inside
                    && button == button::PRIMARY
                    && Self::on_bar(viewport, content_h, x, y)
                {
                    ui.take();
                    self.activity.press(true);
                    let thumb = self.thumb(viewport, content_h).unwrap_or_default();
                    if y >= thumb.y && y < thumb.bottom() {
                        self.grab = Some(y - thumb.y);
                        Response::Redraw
                    } else {
                        let dy = if y < thumb.y {
                            -Self::page(viewport)
                        } else {
                            Self::page(viewport)
                        };
                        self.scroll_by(dy, viewport, content_h);
                        Response::Redraw
                    }
                } else {
                    // A press an inner component took was not the area's.
                    self.activity.press(inside && !taken);
                    if was != self.activity.active {
                        Response::Redraw
                    } else {
                        Response::Nothing
                    }
                }
            }
            Event::PointerMove { x, y, buttons, .. } => {
                self.hover = Some(viewport.contains(x, y));
                match self.grab {
                    Some(_) if buttons & button::mask(button::PRIMARY) == 0 => {
                        self.grab = None;
                        Response::Nothing
                    }
                    Some(grab) if !taken => {
                        ui.take();
                        let (Some(t), Some(thumb)) = (
                            Self::track(viewport, content_h),
                            self.thumb(viewport, content_h),
                        ) else {
                            return Response::Nothing;
                        };
                        let room = (t.h - thumb.h).max(1.0);
                        let frac = ((y - grab - t.y) / room).clamp(0.0, 1.0);
                        let max = Self::max_offset(viewport, content_h);
                        if self.scroll_to(frac * max, viewport, content_h) {
                            Response::Redraw
                        } else {
                            Response::Nothing
                        }
                    }
                    _ => Response::Nothing,
                }
            }
            Event::PointerUp { .. } if self.grab.is_some() => {
                self.grab = None;
                if !taken {
                    ui.take();
                }
                Response::Redraw
            }
            Event::PointerLeave => {
                self.hover = None;
                Response::Nothing
            }
            Event::Scroll { dy, lines, .. } if !taken && self.hover != Some(false) => {
                let px = if lines { dy * LINE_PX } else { dy };
                if self.scroll_by(px, viewport, content_h) {
                    ui.take();
                    Response::Redraw
                } else {
                    Response::Nothing
                }
            }
            Event::KeyDown {
                code, modifiers, ..
            } if !taken && self.activity.active => {
                let page = Self::page(viewport);
                let dy = match code {
                    key::UP => -LINE_PX,
                    key::DOWN => LINE_PX,
                    key::PAGE_UP => -page,
                    key::PAGE_DOWN => page,
                    key::SPACE if modifiers.shift() => -page,
                    key::SPACE => page,
                    key::HOME => -self.offset,
                    key::END => Self::max_offset(viewport, content_h),
                    _ => return Response::Nothing,
                };
                ui.take();
                if self.scroll_by(dy, viewport, content_h) {
                    Response::Redraw
                } else {
                    Response::Nothing
                }
            }
            _ => Response::Nothing,
        };
        ui.respond(response)
    }

    /// Draw the content with `body` clipped to `viewport`, then the scroll
    /// bar and, while active, the ring. Returns what `body` returned.
    pub fn draw<R>(
        &mut self,
        f: &mut Frame,
        viewport: Rect,
        theme: &Theme,
        content_h: f32,
        body: impl FnOnce(&mut Frame) -> Result<R, VyError>,
    ) -> Result<R, VyError> {
        self.clamp(viewport, content_h);
        let out = f.clipped(viewport, 0.0, body)?;
        self.draw_bar(f, viewport, theme, content_h)?;
        if self.activity.active {
            ring_outline(f, viewport, theme)?;
        }
        Ok(out)
    }

    pub(crate) fn draw_bar(
        &self,
        f: &mut Frame,
        viewport: Rect,
        theme: &Theme,
        content_h: f32,
    ) -> Result<(), VyError> {
        if let Some(thumb) = self.thumb(viewport, content_h) {
            let color = if self.grab.is_some() {
                theme.palette.muted
            } else {
                theme.palette.track
            };
            f.rect(thumb, BAR_W / 2.0, color)?;
        }
        Ok(())
    }
}

/// The focus ring as an outline around `r`, for an area without a
/// background of its own: four bars in the ring colour.
pub(crate) fn ring_outline(f: &mut Frame, r: Rect, theme: &Theme) -> Result<(), VyError> {
    let c = theme.palette.ring;
    let o = r.inset(-RING);
    let t = RING - 1.0;
    f.rect(Rect::new(o.x, o.y, o.w, t), 0.0, c)?;
    f.rect(Rect::new(o.x, o.bottom() - t, o.w, t), 0.0, c)?;
    f.rect(Rect::new(o.x, o.y + t, t, o.h - 2.0 * t), 0.0, c)?;
    f.rect(Rect::new(o.right() - t, o.y + t, t, o.h - 2.0 * t), 0.0, c)
}
