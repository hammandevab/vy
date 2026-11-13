//! Ready-made controls for pages: [`TextBox`] (one line or several),
//! [`Button`], [`Checkbox`], [`RadioGroup`], [`Slider`], a virtualised
//! [`List`], a [`ScrollView`], and the layout stacks [`Column`] and [`Row`].
//!
//! The components are ordinary module code on top of [`Frame`] and the
//! typed imports: nothing here needs anything the ABI lacks, every one is
//! deterministic (same state, same rectangle, same display list) and none
//! allocates more per frame than the text it draws. They follow the
//! `layout()` pattern of the reference pages: the page computes each
//! component's rectangle from the window, `draw` paints into it and
//! registers the semantic node(s) with the right role (SPEC-10.1.1), and
//! `event` hit-tests against the same rectangle.
//!
//! - **Events** go through an [`Input`]: `App::event` wraps the event, hands
//!   `&mut ui` to every component (inner before outer), and returns
//!   [`Input::redraw`]. A component that uses a pointer, scroll, key or text
//!   event *takes* it, so components after it never see it; a `FOCUS` event
//!   is never taken, because every component must learn where the host's
//!   focus went (SPEC-10.3.1). What no component took is
//!   [`Input::unhandled`]. Each `event` returns a [`Response`].
//! - **Signatures** are the same for every component:
//!   `event(&mut self, ui: &mut Input, r: Rect, theme: &Theme, ..) -> Response`
//!   and `draw(&mut self, f: &mut Frame, r: Rect, theme: &Theme, ..)`, with the
//!   same rectangle in both.
//! - **Node ids.** A focusable component takes a fixed, negative node id at
//!   construction (SPEC-5.2.13: a page's own ids are negative), so the
//!   host's focus, which follows node ids across frames (SPEC-10.3.2), stays
//!   on it however the rest of the tree changes.
//! - **Focus** is the host's (SPEC-10.3.1): a component learns it from
//!   `Event::Focus` with its id and draws the focus mark itself
//!   (SPEC-10.3.3). A [`List`] or [`ScrollView`] has no focusable role
//!   (SPEC-10.3.2); it takes keys while *active*, after a press inside it
//!   that gave no node focus.
//! - **Style** comes from a [`Theme`], built once in `App::init` (it
//!   registers its text styles with the host).
//!
//! ```
//! use vy_sdk::prelude::*;
//! use vy_sdk::ui::{Button, Input, Label, TextBox, Theme};
//!
//! struct Page { window: Window, theme: Theme, name: TextBox, send: Button, sent: bool }
//!
//! impl Page {
//!     /// Every rectangle from the window: draw and event use the same.
//!     fn layout(&self) -> (Rect, Rect) {
//!         let w = (self.window.width - 48.0).max(0.0);
//!         (Rect::new(24.0, 24.0, w, 40.0), Rect::new(24.0, 76.0, 120.0, 40.0))
//!     }
//! }
//!
//! impl App for Page {
//!     fn init(window: Window, _: i32) -> Result<Self, VyError> {
//!         let theme = Theme::new()?; // registers the styles once
//!         Ok(Page { window, theme, name: TextBox::new(-1), send: Button::new(-2), sent: false })
//!     }
//!     fn frame(&mut self, f: &mut Frame, _: i32) -> Result<(), VyError> {
//!         let (field, button) = self.layout();
//!         self.name.draw(f, field, &self.theme, Label::hidden("Namn"), "Ditt namn")?;
//!         self.send.draw(f, button, &self.theme, "Skicka")
//!     }
//!     fn event(&mut self, event: Event<'_>) -> bool {
//!         let (field, button) = self.layout();
//!         let mut ui = Input::new(event);
//!         let enter = self.name.event(&mut ui, field, &self.theme).activated();
//!         if self.send.event(&mut ui, button, &self.theme).activated() || enter {
//!             self.sent = true;
//!         }
//!         ui.redraw()
//!     }
//!     fn resize(&mut self, window: Window) { self.window = window; }
//! }
//! # vy_sdk::mock::reset();
//! # let mut page = Page::init(Window { width: 400.0, height: 300.0, scale: 1.0 }, 0).unwrap();
//! # assert!(vy_sdk::mock::run_frame(&mut page, 0).is_ok());
//! # vy_sdk::mock::with(|h| assert_eq!(h.sem.len(), 3, "the field's name, the field, the button"));
//! ```

mod button;
mod checkbox;
mod input;
mod layout;
mod list;
mod radio;
mod scroll;
mod slider;
mod textbox;
mod theme;

pub use button::{Button, ButtonKind};
pub use checkbox::Checkbox;
pub use input::{Input, Response};
pub use layout::{split, Column, Row};
pub use list::{List, Rows};
pub use radio::RadioGroup;
pub use scroll::{ScrollView, BAR_W, LINE_PX};
pub use slider::Slider;
pub use textbox::{Label, Lines, Multiline, OneLine, TextBox, MAX_LINE_BYTES, MAX_MULTILINE_BYTES};
pub use theme::{Palette, Theme, Words};

use crate::event::Event;
use crate::frame::Frame;
use crate::{Rect, VyError};

/// Height of a one-line control (text field, button) in logical pixels;
/// a suggestion for the page's layout, not a rule.
pub const CONTROL_H: f32 = 40.0;
/// Side of a checkbox and diameter of a radio button.
pub const BOX: f32 = 20.0;
/// Corner radius of fields and buttons.
pub const RADIUS: f32 = 6.0;
/// Horizontal padding inside a text field.
pub const PAD: f32 = 12.0;
/// Gap between a checkbox or radio button and its label.
pub const LABEL_GAP: f32 = 10.0;
/// How far outside a control its focus ring lies.
pub const RING: f32 = 3.0;

/// `Some(gained)` when `event` is a `FOCUS` event for node `id`.
pub(crate) fn focus_of(event: &Event<'_>, id: i32) -> Option<bool> {
    match *event {
        Event::Focus { node, gained, .. } if node.0 == id => Some(gained),
        _ => None,
    }
}

/// `text` cut to at most `max` bytes at a character boundary.
pub(crate) fn clip_str(text: &str, max: usize) -> &str {
    &text[..text.floor_char_boundary(max)]
}

/// The focus ring around `r` (corners `radius`) in the theme's ring colour.
pub(crate) fn ring(f: &mut Frame, r: Rect, radius: f32, theme: &Theme) -> Result<(), VyError> {
    f.rect(r.inset(-RING), radius + RING, theme.palette.ring)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
