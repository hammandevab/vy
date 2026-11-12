//! Stacks for `layout()`: rectangles one after another with gaps and
//! padding. There is no layout engine (SPEC-2.1.3); these only add.

use crate::Rect;

/// Rectangles stacked down a column, `gap` apart, inside `area` shrunk by
/// `pad` on every side.
///
/// ```
/// use vy_sdk::Rect;
/// use vy_sdk::ui::Column;
/// let mut c = Column::new(Rect::new(0.0, 0.0, 300.0, 500.0), 20.0, 8.0);
/// let title = c.next(32.0);
/// let field = c.next(40.0);
/// assert_eq!(field, Rect::new(20.0, 60.0, 260.0, 40.0));
/// assert_eq!(c.height(), 120.0); // what a card around both needs
/// # let _ = title;
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Column {
    area: Rect,
    pad: f32,
    gap: f32,
    y: f32,
    started: bool,
}

impl Column {
    /// A column at the top of `area`, inset by `pad`, with `gap` between rectangles.
    pub fn new(area: Rect, pad: f32, gap: f32) -> Self {
        Self {
            area,
            pad,
            gap,
            y: area.y + pad,
            started: false,
        }
    }

    /// Width of every rectangle: the area's width less both pads.
    pub fn width(&self) -> f32 {
        (self.area.w - 2.0 * self.pad).max(0.0)
    }

    /// The next rectangle, `h` high, a gap below the previous one.
    pub fn next(&mut self, h: f32) -> Rect {
        if self.started {
            self.y += self.gap;
        }
        self.started = true;
        let r = Rect::new(self.area.x + self.pad, self.y, self.width(), h.max(0.0));
        self.y += r.h;
        r
    }

    /// Extra space before the next rectangle.
    pub fn space(&mut self, h: f32) {
        self.y += h;
    }

    /// Bottom edge of the last rectangle (the top pad before the first).
    pub fn bottom(&self) -> f32 {
        self.y
    }

    /// Height from the area's top to the last rectangle plus the bottom
    /// pad: what a card around the column needs.
    pub fn height(&self) -> f32 {
        self.y + self.pad - self.area.y
    }

    /// What is left below, from where the next rectangle would start to
    /// the area's bottom pad (zero height when nothing is left).
    pub fn rest(&self) -> Rect {
        let y = if self.started {
            self.y + self.gap
        } else {
            self.y
        };
        let bottom = self.area.bottom() - self.pad;
        Rect::new(
            self.area.x + self.pad,
            y,
            self.width(),
            (bottom - y).max(0.0),
        )
    }
}

/// Rectangles side by side along a row, `gap` apart, inside `area` shrunk
/// by `pad` on every side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    area: Rect,
    pad: f32,
    gap: f32,
    x: f32,
    started: bool,
}

impl Row {
    /// A row at the left of `area`, inset by `pad`, with `gap` between rectangles.
    pub fn new(area: Rect, pad: f32, gap: f32) -> Self {
        Self {
            area,
            pad,
            gap,
            x: area.x + pad,
            started: false,
        }
    }

    /// Height of every rectangle: the area's height less both pads.
    pub fn height(&self) -> f32 {
        (self.area.h - 2.0 * self.pad).max(0.0)
    }

    /// The next rectangle, `w` wide, a gap right of the previous one.
    pub fn next(&mut self, w: f32) -> Rect {
        if self.started {
            self.x += self.gap;
        }
        self.started = true;
        let r = Rect::new(self.x, self.area.y + self.pad, w.max(0.0), self.height());
        self.x += r.w;
        r
    }

    /// Extra space before the next rectangle.
    pub fn space(&mut self, w: f32) {
        self.x += w;
    }

    /// Right edge of the last rectangle (the left pad before the first).
    pub fn right(&self) -> f32 {
        self.x
    }

    /// Width from the area's left to the last rectangle plus the right pad.
    pub fn width(&self) -> f32 {
        self.x + self.pad - self.area.x
    }

    /// What is left to the right, to the area's right pad.
    pub fn rest(&self) -> Rect {
        let x = if self.started {
            self.x + self.gap
        } else {
            self.x
        };
        let right = self.area.right() - self.pad;
        Rect::new(
            x,
            self.area.y + self.pad,
            (right - x).max(0.0),
            self.height(),
        )
    }
}

/// `area` cut into `N` equal columns side by side, `gap` apart: two
/// columns on a wide window, say.
///
/// ```
/// use vy_sdk::Rect;
/// let [left, right] = vy_sdk::ui::split::<2>(Rect::new(0.0, 0.0, 410.0, 100.0), 10.0);
/// assert_eq!((left.w, right.x), (200.0, 210.0));
/// ```
pub fn split<const N: usize>(area: Rect, gap: f32) -> [Rect; N] {
    let n = N.max(1) as f32;
    let w = ((area.w - gap * (n - 1.0)) / n).max(0.0);
    core::array::from_fn(|i| Rect::new(area.x + i as f32 * (w + gap), area.y, w, area.h))
}
