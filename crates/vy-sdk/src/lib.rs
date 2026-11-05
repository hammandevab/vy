//! Module-side SDK for Vy pages.
//!
//! A page is a `struct` that implements [`App`]; `vy_sdk::export!(MyApp)`
//! generates the six exports of SPEC-5.1 around it. Everything the module
//! can ask the host for goes through the typed functions in this crate,
//! which wrap the 32 imports of SPEC-5.2 (one function per import, SPEC-2.1.1)
//! and turn status codes into [`VyError`].
//!
//! The crate is `no_std` and builds for `wasm32-unknown-unknown`, where the
//! bindings are real imports from namespace `vy`, and natively, where they
//! dispatch to a [`mock::MockHost`] so SDK and page logic can be unit-tested
//! on the host without a wasm engine.
//!
//! The SDK checks locally what the host would refuse anyway (SPEC-6.1.4,
//! SPEC-6.1.6, SPEC-9.3.2), so a page sees the error before the host logs it;
//! the host remains the authority.
#![no_std]
#![deny(missing_docs)]

extern crate alloc;
#[cfg(not(target_arch = "wasm32"))]
extern crate std;

pub mod api;
pub mod app;
pub mod arena;
pub mod error;
pub mod event;
pub mod frame;
#[cfg(not(target_arch = "wasm32"))]
pub mod mock;
mod raw;
pub mod sem;

pub mod heap;

pub use api::{
    abi_version, clipboard_read, clipboard_write, exit, fetch_begin, fetch_cancel, fetch_poll,
    fetch_read, ime_begin, ime_end, key_down, link_register, log, navigate, now_ms, random,
    request_frame, state_delete, state_get, state_put, text_caret, text_measure, text_style, Level,
    Method,
};
pub use app::{App, Window};
pub use error::VyError;
pub use event::{Capability, Event, Modifiers};
pub use frame::{Fit, Frame, Path};
pub use sem::{NodeId, Role, Sem};

/// Fixed-point helper: the ABI carries lengths as integers × 100.
pub(crate) fn from_x100(v: i32) -> f32 {
    v as f32 / 100.0
}

/// A rectangle in logical pixels (SPEC-6.3.1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// Rectangle from position and size.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    /// Right edge.
    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    /// Bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    /// True when `(px, py)` lies inside (left and top edges inclusive).
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.right() && py < self.bottom()
    }

    /// The rectangle shrunk by `d` on every side (never below zero size).
    pub fn inset(&self, d: f32) -> Self {
        Self {
            x: self.x + d,
            y: self.y + d,
            w: (self.w - 2.0 * d).max(0.0),
            h: (self.h - 2.0 * d).max(0.0),
        }
    }

    /// Same rectangle moved by `(dx, dy)`.
    pub fn offset(&self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            ..*self
        }
    }
}

/// A measured size in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

/// A colour as the ABI carries it: `0xRRGGBBAA` in sRGB (SPEC-6.1.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    /// Opaque colour from 8-bit channels.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 255)
    }

    /// Colour from 8-bit channels with alpha.
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self(((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | a as u32)
    }

    /// The `i32` the imports take.
    pub const fn as_i32(self) -> i32 {
        self.0 as i32
    }

    /// Same colour with another alpha.
    pub const fn with_alpha(self, a: u8) -> Self {
        Self((self.0 & 0xFFFF_FF00) | a as u32)
    }
}

/// A font to shape text with: one of the five built-in roles (SPEC-6.4.2) or
/// a WOFF2 resource of the package (SPEC-4.1.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    /// Proportional sans-serif (resource id −1).
    Sans,
    /// Proportional serif (−2).
    Serif,
    /// Monospace (−3).
    Mono,
    /// Interface font (−4).
    Ui,
    /// Emoji (−5).
    Emoji,
    /// A font resource of the package by index (SPEC-4.5.1).
    Resource(u16),
}

impl Font {
    /// The `font_resource_id` the imports take.
    pub const fn resource_id(self) -> i32 {
        match self {
            Font::Sans => -1,
            Font::Serif => -2,
            Font::Mono => -3,
            Font::Ui => -4,
            Font::Emoji => -5,
            Font::Resource(id) => id as i32,
        }
    }
}

/// A text style registered with [`text_style`] (SPEC-5.2.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleId(pub i32);

/// A resource of the package by index; 0 is the module itself (SPEC-4.5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceId(pub u16);

/// Everything a page normally needs, for `use vy_sdk::prelude::*`.
pub mod prelude {
    pub use crate::api::{log, request_frame, text_measure, text_style, Level};
    pub use crate::app::{App, Window};
    pub use crate::error::VyError;
    pub use crate::event::{Capability, Event, Modifiers};
    pub use crate::frame::{Fit, Frame, Path};
    pub use crate::sem::{NodeId, Role, Sem};
    pub use crate::{Color, Font, Rect, ResourceId, Size, StyleId};
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests;
