//! A small immediate-mode layout helper: vertical and horizontal stacks with
//! padding and gap, and text that measures itself.
//!
//! There is no constraint solver and no flexbox; layout lives in the module
//! (SPEC-2.1.3) and this is the least that makes a page readable. A
//! [`Column`] hands out rectangles top to bottom, a [`Row`] left to right;
//! both only do arithmetic, except [`Column::text`] and
//! [`Column::measure`], which ask the host to shape text (SPEC-6.4.1).

use crate::api::text_measure;
use crate::error::VyError;
use crate::frame::Frame;
use crate::{Rect, Size, StyleId};

/// A vertical stack inside an area.
#[derive(Clone, Copy, Debug)]
pub struct Column {
    area: Rect,
    pad: f32,
    gap: f32,
    cursor: f32,
    items: u32,
}

impl Column {
    /// A column filling `area`, with no padding and no gap.
    pub fn new(area: Rect) -> Self {
        Self {
            area,
            pad: 0.0,
            gap: 0.0,
            cursor: area.y,
            items: 0,
        }
    }

    /// Inset the content by `pad` on every side.
    pub fn padding(mut self, pad: f32) -> Self {
        self.pad = pad;
        self.cursor = self.area.y + pad;
        self
    }

    /// Space between consecutive items.
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Left edge of the content.
    pub fn x(&self) -> f32 {
        self.area.x + self.pad
    }

    /// Width available to the content.
    pub fn width(&self) -> f32 {
        (self.area.w - 2.0 * self.pad).max(0.0)
    }

    /// Top of the next item.
    pub fn cursor(&self) -> f32 {
        if self.items == 0 {
            self.cursor
        } else {
            self.cursor + self.gap
        }
    }

    /// Height used so far including padding on both sides: what the column
    /// needs to show everything placed in it.
    pub fn height(&self) -> f32 {
        self.cursor - self.area.y + self.pad
    }

    /// Height left before the area's bottom edge.
    pub fn remaining(&self) -> f32 {
        self.area.bottom() - self.pad - self.cursor()
    }

    /// The area the column was given.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// Place the next item, `h` tall and full width.
    pub fn next(&mut self, h: f32) -> Rect {
        let y = self.cursor();
        self.cursor = y + h;
        self.items += 1;
        Rect::new(self.x(), y, self.width(), h)
    }

    /// Leave `h` of empty space (no gap added before or after).
    pub fn space(&mut self, h: f32) {
        self.cursor += h;
    }

    /// Measure `text` wrapped at the content width without drawing it.
    pub fn measure(&self, style: StyleId, text: &str) -> Result<Size, VyError> {
        text_measure(style, self.width(), text)
    }

    /// Draw `text` as the next item, wrapped at the content width, and
    /// return the rectangle it took (height from the host, SPEC-6.4.3).
    pub fn text(&mut self, f: &mut Frame, style: StyleId, text: &str) -> Result<Rect, VyError> {
        let y = self.cursor();
        let h = f.text(self.x(), y, self.width(), style, text)?;
        Ok(self.next(h))
    }

    /// Place the next item, `h` tall, and return a [`Row`] over it.
    pub fn row(&mut self, h: f32) -> Row {
        Row::new(self.next(h)).gap(self.gap)
    }
}

/// A horizontal stack inside an area.
#[derive(Clone, Copy, Debug)]
pub struct Row {
    area: Rect,
    gap: f32,
    cursor: f32,
    items: u32,
}

impl Row {
    /// A row filling `area` with no gap.
    pub fn new(area: Rect) -> Self {
        Self {
            area,
            gap: 0.0,
            cursor: area.x,
            items: 0,
        }
    }

    /// Space between consecutive items.
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Left edge of the next item.
    pub fn cursor(&self) -> f32 {
        if self.items == 0 {
            self.cursor
        } else {
            self.cursor + self.gap
        }
    }

    /// Width left before the right edge.
    pub fn remaining(&self) -> f32 {
        (self.area.right() - self.cursor()).max(0.0)
    }

    /// The area the row was given.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// Place the next item, `w` wide and full height.
    pub fn next(&mut self, w: f32) -> Rect {
        let x = self.cursor();
        self.cursor = x + w;
        self.items += 1;
        Rect::new(x, self.area.y, w, self.area.h)
    }

    /// Place an item taking everything that is left.
    pub fn fill(&mut self) -> Rect {
        let w = self.remaining();
        self.next(w)
    }

    /// Leave `w` of empty space.
    pub fn space(&mut self, w: f32) {
        self.cursor += w;
    }
}
