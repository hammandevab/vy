//! The M3 rasterizer, kept byte for byte as it drew the goldens, for the
//! tests only: `raster.rs` must give the same pixels and the same
//! [`RasterReport`] for every list (SPEC-2.1.5, SPEC-13.2.1). Nothing here
//! is used outside `#[cfg(test)]`.
#![allow(dead_code)]

use crate::image::{ImageCache, RgbaImage};
use crate::text::{Glyph, TextEngine};
use cosmic_text::SwashContent;
use tiny_skia::{
    Color, FillRule, IntSize, Mask, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke,
    Transform,
};
use vy_abi::fit;
use vy_dl::{DecodeError, Op, PathCmd};

/// Colour of the placeholder drawn for an `IMAGE` whose resource is not in
/// the cache: never decoded (a display list rendered outside a runtime) or
/// refused (`E_RESOURCE` was logged when `dl_image` ran).
pub const IMAGE_PLACEHOLDER_COLOR: u32 = 0x8080_80FF;
/// Largest canvas side `render` produces, in physical pixels; also the
/// largest side [`RgbaImage::decode_png`] accepts, since what it reads back
/// (goldens, SPEC-12.2.2) is what `render` wrote.
pub const MAX_CANVAS_SIDE_PX: u32 = 16_384;
/// Largest magnitude accepted for a coordinate, size, radius or stroke
/// width, in logical pixels. The canvas is at most 16384 physical pixels, so a
/// larger value cannot be visible; it only serves to overflow the fixed-point
/// arithmetic in the rasterizer. An op carrying a non-finite or larger value
/// is skipped; a `CLIP_PUSH` carrying one clips everything, so the clip stack
/// stays balanced.
pub const MAX_MAGNITUDE: f32 = 16_777_216.0;

/// Work units the rasterizer may spend on one frame on a canvas of up to a
/// million pixels: 64 × 2²⁰ (SPEC-5.3.3). A unit is one pixel written by a
/// plain opaque fill; pixels that go through the raster pipeline (translucent
/// colour, a clip mask, a glyph, an image, a path) cost [`PIPELINE_WEIGHT`]
/// each, and building a clip mask costs [`MASK_WEIGHT`] per pixel of the
/// clip's bounds. Sized so that a legal frame of 2 000 ops at 1080p — a few
/// full-window layers of fills plus text — fits with room to spare, while
/// 65 000 full-screen rectangles at 1280 × 800 (66 G units) are cut off
/// after 64 of them. On larger canvases the budget grows with the area
/// ([`raster_budget_units`]), so a 4K window gets the same 64 layers.
pub const RASTER_BUDGET_UNITS: u64 = 64 << 20;
/// Layers of the whole canvas the budget allows at weight 1:
/// [`RASTER_BUDGET_UNITS`] is this many pixels on a 1024 × 1024 canvas.
pub const RASTER_BUDGET_OVERDRAWS: u64 = 64;
/// Units per pixel for work that runs the raster pipeline rather than a
/// memset: measured at 15–20 × a plain fill in tiny-skia, release and debug.
pub const PIPELINE_WEIGHT: u64 = 16;
/// Units per pixel of a rounded clip's bounds when its mask is built: a
/// zeroed mask, an anti-aliased fill and a multiply by the parent.
pub const MASK_WEIGHT: u64 = 4;
/// Units every op costs besides its pixels, for decoding and setup.
pub const OP_UNITS: u64 = 64;

/// The work budget of one frame on a `width` × `height` canvas (SPEC-5.3.3):
/// [`RASTER_BUDGET_UNITS`], or [`RASTER_BUDGET_OVERDRAWS`] times the canvas
/// area when that is more.
pub fn raster_budget_units(width: u32, height: u32) -> u64 {
    (u64::from(width) * u64::from(height) * RASTER_BUDGET_OVERDRAWS).max(RASTER_BUDGET_UNITS)
}

/// What a frame cost and whether it was finished (SPEC-5.3.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RasterReport {
    /// True when the budget ran out: the ops after `ops_done` were not
    /// drawn, the canvas holds what was drawn before.
    pub truncated: bool,
    /// Ops processed before the list ended or the budget ran out.
    pub ops_done: usize,
    /// Work units charged.
    pub units: u64,
    /// The budget the frame had, from [`raster_budget_units`].
    pub budget: u64,
}

/// A rasterized frame.
#[derive(Debug)]
pub struct Rendered {
    /// The pixels.
    pub image: RgbaImage,
    /// What it cost and whether the list was finished.
    pub report: RasterReport,
}

/// True when every value is finite and within [`MAX_MAGNITUDE`].
fn sane(values: &[f32]) -> bool {
    values
        .iter()
        .all(|v| v.is_finite() && v.abs() <= MAX_MAGNITUDE)
}

fn sane_path(cmds: &[PathCmd], stroke_w: f32) -> bool {
    sane(&[stroke_w])
        && cmds.iter().all(|c| match *c {
            PathCmd::MoveTo(x, y) | PathCmd::LineTo(x, y) => sane(&[x, y]),
            PathCmd::QuadTo { ctrl, to } => sane(&[ctrl.0, ctrl.1, to.0, to.1]),
            PathCmd::Close => true,
        })
}

/// Why rendering failed.
#[derive(Debug, PartialEq, Eq)]
pub enum RenderError {
    /// The display list bytes did not decode.
    Decode(DecodeError),
    /// Zero or absurd output size.
    BadSize,
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "display list does not decode: {e:?}"),
            Self::BadSize => {
                write!(
                    f,
                    "output size must be at least 1x1 and at most {0}x{0}",
                    MAX_CANVAS_SIDE_PX
                )
            }
        }
    }
}

impl std::error::Error for RenderError {}

fn color(rgba: u32) -> Color {
    Color::from_rgba8(
        (rgba >> 24) as u8,
        (rgba >> 16) as u8,
        (rgba >> 8) as u8,
        rgba as u8,
    )
}

fn paint(rgba: u32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(color(rgba));
    p.anti_alias = true;
    p
}

/// Rounded rectangle path in logical coordinates; `None` if degenerate.
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<Path> {
    if !(w > 0.0 && h > 0.0) || !x.is_finite() || !y.is_finite() {
        return None;
    }
    let r = radius.max(0.0).min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        return Some(PathBuilder::from_rect(Rect::from_xywh(x, y, w, h)?));
    }
    // Cubic approximation of a quarter circle.
    const K: f32 = 0.552_284_8;
    let k = r * K;
    let (l, t, rt, b) = (x, y, x + w, y + h);
    let mut pb = PathBuilder::new();
    pb.move_to(l + r, t);
    pb.line_to(rt - r, t);
    pb.cubic_to(rt - r + k, t, rt, t + r - k, rt, t + r);
    pb.line_to(rt, b - r);
    pb.cubic_to(rt, b - r + k, rt - r + k, b, rt - r, b);
    pb.line_to(l + r, b);
    pb.cubic_to(l + r - k, b, l, b - r + k, l, b - r);
    pb.line_to(l, t + r);
    pb.cubic_to(l, t + r - k, l + r - k, t, l + r, t);
    pb.close();
    pb.finish()
}

/// `premultiply_u8` of tiny-skia: how `Mask::intersect_path` combines two
/// coverages, reproduced so that a nested rounded clip gives the same bytes
/// as rebuilding the whole stack did.
fn multiply_coverage(a: u8, b: u8) -> u8 {
    let prod = u32::from(a) * u32::from(b) + 128;
    ((prod + (prod >> 8)) >> 8) as u8
}

/// A rectangle of whole pixels on the canvas: columns `x0..x1`, rows `y0..y1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Pixels {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Pixels {
    fn area(&self) -> u64 {
        u64::try_from(self.x1 - self.x0).unwrap_or(0)
            * u64::try_from(self.y1 - self.y0).unwrap_or(0)
    }

    fn intersect(&self, o: &Pixels) -> Option<Pixels> {
        let p = Pixels {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        };
        (p.x1 > p.x0 && p.y1 > p.y0).then_some(p)
    }
}

/// `SCALAR_NEARLY_ZERO` of tiny-skia: a path whose bounds are this thin in
/// either direction is not filled at all.
const NEARLY_ZERO: f32 = 1.0 / 4096.0;

/// The pixel boundary a rectangle edge at `v` physical pixels rounds to in
/// tiny-skia's non-anti-aliased scan converter: 26.6 fixed point, truncated,
/// then rounded half up (`LineEdge::new` and `walk_edges`). A pixel is
/// covered when its centre is inside, except that an edge exactly on a
/// centre rounds up. `v` must already lie within the canvas.
fn pixel_edge(v: f32) -> i32 {
    ((v * 64.0) as i32 + 32) >> 6
}

/// The pixels a non-anti-aliased fill of `r` covers on a `w` × `h` canvas:
/// what a mask built from a plain clip rectangle holds, computed instead of
/// rasterized. `None` when nothing is covered.
fn exact_pixels(r: &Rect, w: u32, h: u32) -> Option<Pixels> {
    if r.width().abs() <= NEARLY_ZERO || r.height().abs() <= NEARLY_ZERO {
        return None;
    }
    // The edge clipper moves edges outside the canvas onto its border
    // before rounding; clamping first does the same.
    let (l, t) = (r.left().max(0.0), r.top().max(0.0));
    let (rt, b) = (r.right().min(w as f32), r.bottom().min(h as f32));
    if !(rt > l && b > t) {
        return None;
    }
    let p = Pixels {
        x0: pixel_edge(l),
        y0: pixel_edge(t),
        x1: pixel_edge(rt),
        y1: pixel_edge(b),
    };
    (p.x1 > p.x0 && p.y1 > p.y0).then_some(p)
}

/// Every pixel an anti-aliased fill of `r` may touch on a `w` × `h` canvas.
fn covering_pixels(r: &Rect, w: u32, h: u32) -> Option<Pixels> {
    let p = Pixels {
        x0: r.left().max(0.0).floor() as i32,
        y0: r.top().max(0.0).floor() as i32,
        x1: r.right().min(w as f32).ceil() as i32,
        y1: r.bottom().min(h as f32).ceil() as i32,
    };
    (p.x1 > p.x0 && p.y1 > p.y0).then_some(p)
}

/// One `CLIP_PUSH` in physical pixels: a rectangle with a corner radius, or
/// nothing at all when the rectangle was degenerate or absurd.
#[derive(Clone, Copy)]
struct ClipShape {
    rect: Option<Rect>,
    radius: f32,
}

/// One level of the clip stack.
struct Level {
    shape: ClipShape,
    /// Analytic intersection of every rectangle up to this level; `None`
    /// when nothing is visible under it.
    rect: Option<Rect>,
    /// The pixels a plain-rectangle clip covers: the mask's contents for a
    /// level without rounded shapes, exactly.
    exact: Option<Pixels>,
    /// Every pixel an op under this clip may touch.
    bounds: Option<Pixels>,
    /// True when this or a lower level has a corner radius: ops under it go
    /// through [`Level::mask`].
    rounded: bool,
    /// The mask, built on first use and kept until the level is popped.
    mask: Option<Mask>,
}

/// The frame's work budget.
struct Budget {
    limit: u64,
    used: u64,
}

/// The budget ran out: the op in progress is not drawn.
struct Exhausted;

impl Budget {
    /// Charge `units`; `Err` when they do not fit, in which case nothing is
    /// charged and the caller draws nothing.
    fn charge(&mut self, units: u64) -> Result<(), Exhausted> {
        let after = self.used.saturating_add(units);
        if after > self.limit {
            return Err(Exhausted);
        }
        self.used = after;
        Ok(())
    }

    /// Charge work already done (a text run's glyphs, known only after
    /// shaping); may push past the limit, which stops the next op.
    fn charge_done(&mut self, units: u64) {
        self.used = self.used.saturating_add(units);
    }

    fn exhausted(&self) -> bool {
        self.used >= self.limit
    }
}

/// Clip stack in physical pixels (SPEC-5.2.3, SPEC-6.1.4).
///
/// Plain rectangles are intersected analytically; the pixels they cover
/// are computed with the scan converter's own rounding ([`exact_pixels`]),
/// so their mask is a memset. A rounded shape materializes an anti-aliased
/// mask over its own bounds, multiplied by the parent level's coverage, the
/// way `Mask::intersect_path` would; each level keeps its mask until it is
/// popped, so the cost of a push is at most the shape's area and a pop
/// costs nothing.
struct Clip {
    width: u32,
    height: u32,
    levels: Vec<Level>,
}

impl Clip {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            levels: Vec::new(),
        }
    }

    fn push(&mut self, shape: ClipShape) {
        let (parent_rect, parent_rounded, parent_bounds) = match self.levels.last() {
            None => (None, false, None),
            Some(p) => (Some(p.rect), p.rounded, p.bounds),
        };
        let rect = match (parent_rect, shape.rect) {
            (None, r) => r,
            (Some(Some(p)), Some(r)) => p.intersect(&r),
            _ => None,
        };
        let rounded = shape.radius > 0.0 || parent_rounded;
        let exact = rect.and_then(|r| exact_pixels(&r, self.width, self.height));
        let bounds = if rounded {
            let own = rect.and_then(|r| covering_pixels(&r, self.width, self.height));
            match (own, parent_bounds) {
                (Some(o), Some(p)) if parent_rect.is_some() => o.intersect(&p),
                (o, _) => o,
            }
        } else {
            exact
        };
        self.levels.push(Level {
            shape,
            rect,
            exact,
            bounds,
            rounded,
            mask: None,
        });
    }

    fn pop(&mut self) {
        self.levels.pop();
    }

    fn top(&self) -> Option<&Level> {
        self.levels.last()
    }

    /// Pixels an op may touch under the current clip: the whole canvas with
    /// no clip, `None` when the clip hides everything.
    fn bounds(&self) -> Option<Pixels> {
        match self.top() {
            None => Some(Pixels {
                x0: 0,
                y0: 0,
                x1: self.width as i32,
                y1: self.height as i32,
            }),
            Some(level) => level.bounds,
        }
    }

    /// The mask ops under the current clip draw through: `None` without a
    /// clip; built on first use. `Err` when the budget cannot pay for it.
    fn mask(&mut self, budget: &mut Budget) -> Result<Option<&Mask>, Exhausted> {
        let Some(top) = self.levels.len().checked_sub(1) else {
            return Ok(None);
        };
        self.materialize(top, budget)?;
        Ok(self.levels[top].mask.as_ref())
    }

    fn materialize(&mut self, k: usize, budget: &mut Budget) -> Result<(), Exhausted> {
        if self.levels[k].mask.is_some() {
            return Ok(());
        }
        let (w, h) = (self.width, self.height);
        let Some(bounds) = self.levels[k].bounds else {
            // Nothing visible: an all-zero mask, and nothing is ever drawn
            // through it because callers check `bounds` first.
            self.levels[k].mask = Mask::new(w, h);
            return Ok(());
        };
        if !self.levels[k].rounded {
            budget.charge(bounds.area())?;
            let mut mask = Mask::new(w, h).ok_or(Exhausted)?;
            fill_pixels(&mut mask, &bounds, w);
            self.levels[k].mask = Some(mask);
            return Ok(());
        }
        // Rounded: this shape's own coverage, multiplied by the parent's
        // (a mask, or an exact rectangle) over every pixel the shape
        // touched, so the result is zero outside the parent too.
        let parent_rounded = k > 0 && self.levels[k - 1].rounded;
        if parent_rounded {
            self.materialize(k - 1, budget)?;
        }
        let shape = self.levels[k].shape;
        let region = shape
            .rect
            .and_then(|r| covering_pixels(&r, w, h))
            .unwrap_or(bounds);
        budget.charge(region.area().saturating_mul(MASK_WEIGHT))?;
        let mut mask = Mask::new(w, h).ok_or(Exhausted)?;
        match shape.rect {
            Some(r) if shape.radius > 0.0 => {
                if let Some(path) = rounded_rect(r.x(), r.y(), r.width(), r.height(), shape.radius)
                {
                    mask.fill_path(&path, FillRule::Winding, true, Transform::identity());
                }
            }
            Some(r) => {
                if let Some(p) = exact_pixels(&r, w, h) {
                    fill_pixels(&mut mask, &p, w);
                }
            }
            None => {}
        }
        if k > 0 {
            let parent = &self.levels[k - 1];
            if parent_rounded {
                let parent_mask = parent.mask.as_ref().expect("materialized above");
                multiply_within(&mut mask, parent_mask, &region, w);
            } else {
                // A plain parent keeps its exact pixels and nothing else.
                match parent.exact {
                    Some(keep) => keep_within(&mut mask, &keep, &region, w),
                    None => mask.clear(),
                }
            }
        }
        self.levels[k].mask = Some(mask);
        Ok(())
    }
}

/// Set `p` to full coverage in a `w` pixels wide mask.
fn fill_pixels(mask: &mut Mask, p: &Pixels, w: u32) {
    let data = mask.data_mut();
    let (x0, x1) = (p.x0 as usize, p.x1 as usize);
    for y in p.y0..p.y1 {
        let row = y as usize * w as usize;
        data[row + x0..row + x1].fill(255);
    }
}

/// `child[i] = multiply(child[i], parent[i])` over `region`.
fn multiply_within(child: &mut Mask, parent: &Mask, region: &Pixels, w: u32) {
    let (c, p) = (child.data_mut(), parent.data());
    let (x0, x1) = (region.x0 as usize, region.x1 as usize);
    for y in region.y0..region.y1 {
        let row = y as usize * w as usize;
        for i in row + x0..row + x1 {
            c[i] = multiply_coverage(c[i], p[i]);
        }
    }
}

/// Zero every pixel of `region` that is outside `keep`.
fn keep_within(mask: &mut Mask, keep: &Pixels, region: &Pixels, w: u32) {
    let data = mask.data_mut();
    let (x0, x1) = (region.x0 as usize, region.x1 as usize);
    for y in region.y0..region.y1 {
        let row = y as usize * w as usize;
        if y < keep.y0 || y >= keep.y1 {
            data[row + x0..row + x1].fill(0);
            continue;
        }
        let (kx0, kx1) = (keep.x0 as usize, keep.x1 as usize);
        if x0 < kx0 {
            data[row + x0..row + kx0.min(x1)].fill(0);
        }
        if x1 > kx1 {
            data[row + kx1.max(x0)..row + x1].fill(0);
        }
    }
}

struct Canvas<'a> {
    pixmap: Pixmap,
    scale: f32,
    clip: Clip,
    budget: Budget,
    text: &'a mut TextEngine,
    images: &'a ImageCache,
}

/// Where an image lands on the canvas, in physical pixels: the drawn region
/// (the whole scaled image, which may overflow the destination under
/// `cover` or underfill it under `contain`) and the destination rectangle
/// that clips it.
struct Placement {
    /// Left of the scaled image.
    rx: f32,
    /// Top of the scaled image.
    ry: f32,
    /// Physical pixels per source pixel, horizontally.
    kx: f32,
    /// Physical pixels per source pixel, vertically.
    ky: f32,
    /// Pixel columns to fill: `px0..px1`.
    px0: i32,
    /// See `px0`.
    px1: i32,
    /// Pixel rows to fill: `py0..py1`.
    py0: i32,
    /// See `py0`.
    py1: i32,
}

impl Placement {
    fn area(&self) -> u64 {
        u64::try_from(self.px1 - self.px0).unwrap_or(0)
            * u64::try_from(self.py1 - self.py0).unwrap_or(0)
    }
}

/// Fit a `iw` × `ih` image into the physical rectangle `(dx, dy, dw, dh)`
/// (SPEC-5.2.3: 0 cover, 1 contain, 2 stretch), limited to a `cw` × `ch`
/// canvas. `None` when nothing is visible or `fit` is unknown.
#[allow(clippy::too_many_arguments)]
fn place(
    fit_mode: u8,
    iw: u32,
    ih: u32,
    dx: f32,
    dy: f32,
    dw: f32,
    dh: f32,
    cw: u32,
    ch: u32,
) -> Option<Placement> {
    if iw == 0 || ih == 0 || !(dw > 0.0 && dh > 0.0) {
        return None;
    }
    let (iw, ih) = (iw as f32, ih as f32);
    let (kx, ky) = match i32::from(fit_mode) {
        fit::COVER => {
            let k = (dw / iw).max(dh / ih);
            (k, k)
        }
        fit::CONTAIN => {
            let k = (dw / iw).min(dh / ih);
            (k, k)
        }
        fit::STRETCH => (dw / iw, dh / ih),
        _ => return None,
    };
    let (rw, rh) = (iw * kx, ih * ky);
    let (rx, ry) = (dx + (dw - rw) / 2.0, dy + (dh - rh) / 2.0);
    // Visible span: destination ∩ drawn region ∩ canvas. A pixel is filled
    // when its centre lies in the half-open span.
    let x0 = dx.max(rx).max(0.0);
    let y0 = dy.max(ry).max(0.0);
    let x1 = (dx + dw).min(rx + rw).min(cw as f32);
    let y1 = (dy + dh).min(ry + rh).min(ch as f32);
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    let px0 = (x0 - 0.5).ceil() as i32;
    let py0 = (y0 - 0.5).ceil() as i32;
    let px1 = (x1 - 0.5).ceil() as i32;
    let py1 = (y1 - 0.5).ceil() as i32;
    if px1 <= px0 || py1 <= py0 {
        return None;
    }
    Some(Placement {
        rx,
        ry,
        kx,
        ky,
        px0,
        px1,
        py0,
        py1,
    })
}

/// Resample `img` over a placement with nearest-neighbour sampling into a
/// premultiplied RGBA8 pixmap. Integer and IEEE-754 arithmetic only, so the
/// same display list gives the same pixels on every machine (SPEC-2.1.5).
/// Bilinear and mipmapped sampling arrive with the GPU path (M3).
fn resample(img: &RgbaImage, p: &Placement) -> Option<Pixmap> {
    let (w, h) = ((p.px1 - p.px0) as u32, (p.py1 - p.py0) as u32);
    let size = IntSize::from_wh(w, h)?;
    let (iw, ih) = (img.width as i64, img.height as i64);
    let mut out = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for py in p.py0..p.py1 {
        let sy = ((py as f32 + 0.5 - p.ry) / p.ky).floor() as i64;
        let sy = sy.clamp(0, ih - 1) as usize;
        let row = &img.data[sy * img.width as usize * 4..][..img.width as usize * 4];
        for px in p.px0..p.px1 {
            let sx = ((px as f32 + 0.5 - p.rx) / p.kx).floor() as i64;
            let sx = sx.clamp(0, iw - 1) as usize;
            let s = &row[sx * 4..sx * 4 + 4];
            let a = u32::from(s[3]);
            out.extend_from_slice(&[
                ((u32::from(s[0]) * a + 127) / 255) as u8,
                ((u32::from(s[1]) * a + 127) / 255) as u8,
                ((u32::from(s[2]) * a + 127) / 255) as u8,
                s[3],
            ]);
        }
    }
    Pixmap::from_vec(out, size)
}

/// Length of a path's control polygon in logical pixels: what a stroke
/// costs, roughly, times its width.
fn polyline_length(cmds: &[PathCmd]) -> f32 {
    let mut len = 0.0f32;
    let mut start = (0.0f32, 0.0f32);
    let mut at = start;
    let mut step = |from: (f32, f32), to: (f32, f32)| {
        len += ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2)).sqrt();
    };
    for c in cmds {
        match *c {
            PathCmd::MoveTo(x, y) => {
                start = (x, y);
                at = start;
            }
            PathCmd::LineTo(x, y) => {
                step(at, (x, y));
                at = (x, y);
            }
            PathCmd::QuadTo { ctrl, to } => {
                step(at, ctrl);
                step(ctrl, to);
                at = to;
            }
            PathCmd::Close => {
                step(at, start);
                at = start;
            }
        }
    }
    len
}

impl Canvas<'_> {
    fn transform(&self) -> Transform {
        Transform::from_scale(self.scale, self.scale)
    }

    /// The op's pixel bounds on the canvas under the clip, from a logical
    /// rectangle; `None` when nothing of it can be visible.
    fn visible(&self, x: f32, y: f32, w: f32, h: f32) -> Option<Pixels> {
        let s = self.scale;
        let r = Rect::from_xywh(x * s, y * s, (w * s).max(0.0), (h * s).max(0.0))?;
        let own = covering_pixels(&r, self.pixmap.width(), self.pixmap.height())?;
        own.intersect(&self.clip.bounds()?)
    }

    /// Fill a logical-coordinate path through the clip mask, charging
    /// `area` physical pixels at the pipeline weight (or at 1 when the fill
    /// is an opaque memset: no clip, opaque colour).
    fn fill(&mut self, path: &Path, rgba: u32, area: u64) -> Result<(), Exhausted> {
        let plain = self.clip.top().is_none() && rgba & 0xFF == 0xFF;
        let weight = if plain { 1 } else { PIPELINE_WEIGHT };
        self.budget.charge(area.saturating_mul(weight))?;
        let t = self.transform();
        let mask = self.clip.mask(&mut self.budget)?;
        self.pixmap
            .fill_path(path, &paint(rgba), FillRule::Winding, t, mask);
        Ok(())
    }

    fn stroke(&mut self, path: &Path, rgba: u32, width: f32, units: u64) -> Result<(), Exhausted> {
        self.budget.charge(units)?;
        let t = self.transform();
        let stroke = Stroke {
            width: width.max(0.0),
            ..Stroke::default()
        };
        let mask = self.clip.mask(&mut self.budget)?;
        self.pixmap
            .stroke_path(path, &paint(rgba), &stroke, t, mask);
        Ok(())
    }

    fn rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        rgba: u32,
    ) -> Result<(), Exhausted> {
        let Some(visible) = self.visible(x, y, w, h) else {
            return Ok(());
        };
        if radius <= 0.0 {
            return self.plain_rect(x, y, w, h, rgba, &visible);
        }
        if let Some(path) = rounded_rect(x, y, w, h, radius) {
            self.fill(&path, rgba, visible.area())?;
        }
        Ok(())
    }

    /// A plain rectangle. Under a clip it is first cut to the clip's pixel
    /// bounds, where the mask is zero anyway, so the masked fill touches
    /// only pixels that can change; it still goes through the mask, because
    /// tiny-skia blends a masked fill (source-over, coverage pre-scaled) and
    /// an unmasked one (source, a lerp) along different arithmetic paths
    /// that differ by a unit at anti-aliased edges, and the goldens are the
    /// masked bytes.
    fn plain_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        rgba: u32,
        visible: &Pixels,
    ) -> Result<(), Exhausted> {
        // The corners are scaled as a transformed logical path would have
        // them, so the edges land on the same sub-pixel values as before.
        let s = self.scale;
        let Some(r) = Rect::from_ltrb(x * s, y * s, (x + w) * s, (y + h) * s) else {
            return Ok(());
        };
        let opaque = rgba & 0xFF == 0xFF;
        if self.clip.top().is_none() {
            let weight = if opaque { 1 } else { PIPELINE_WEIGHT };
            self.budget.charge(visible.area().saturating_mul(weight))?;
            self.pixmap.fill_path(
                &PathBuilder::from_rect(r),
                &paint(rgba),
                FillRule::Winding,
                Transform::identity(),
                None,
            );
            return Ok(());
        }
        let Some(bounds) = self.clip.bounds() else {
            return Ok(());
        };
        let Some(c) = Rect::from_ltrb(
            r.left().max(bounds.x0 as f32),
            r.top().max(bounds.y0 as f32),
            r.right().min(bounds.x1 as f32),
            r.bottom().min(bounds.y1 as f32),
        )
        .filter(|c| c.width() > 0.0 && c.height() > 0.0) else {
            return Ok(());
        };
        self.budget
            .charge(visible.area().saturating_mul(PIPELINE_WEIGHT))?;
        let mask = self.clip.mask(&mut self.budget)?;
        self.pixmap.fill_path(
            &PathBuilder::from_rect(c),
            &paint(rgba),
            FillRule::Winding,
            Transform::identity(),
            mask,
        );
        Ok(())
    }

    fn path(&mut self, cmds: &[PathCmd], rgba: u32, stroke_w: f32) -> Result<(), Exhausted> {
        let mut pb = PathBuilder::new();
        for c in cmds {
            match *c {
                PathCmd::MoveTo(x, y) => pb.move_to(x, y),
                PathCmd::LineTo(x, y) => pb.line_to(x, y),
                PathCmd::QuadTo { ctrl, to } => pb.quad_to(ctrl.0, ctrl.1, to.0, to.1),
                PathCmd::Close => pb.close(),
            }
        }
        let Some(path) = pb.finish() else {
            return Ok(());
        };
        let setup = (cmds.len() as u64).saturating_mul(16);
        let b = path.bounds();
        let s = self.scale;
        if stroke_w > 0.0 {
            // A stroke touches about its length times its width; its
            // bounding box says little about a long thin line.
            let pad = stroke_w;
            let Some(visible) = self.visible(
                b.left() - pad,
                b.top() - pad,
                b.width() + 2.0 * pad,
                b.height() + 2.0 * pad,
            ) else {
                return Ok(());
            };
            let swept = (polyline_length(cmds) * s) as u64 * ((stroke_w * s).max(1.0) as u64);
            let units = swept
                .min(visible.area())
                .saturating_mul(PIPELINE_WEIGHT)
                .saturating_add(setup);
            self.stroke(&path, rgba, stroke_w, units)
        } else {
            let Some(visible) = self.visible(b.left(), b.top(), b.width(), b.height()) else {
                return Ok(());
            };
            self.budget.charge(setup)?;
            self.fill(&path, rgba, visible.area())
        }
    }

    fn image(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        resource_id: u16,
        fit_mode: u8,
    ) -> Result<(), Exhausted> {
        let Ok(img) = self.images.get(resource_id) else {
            return self.rect(x, y, w, h, 0.0, IMAGE_PLACEHOLDER_COLOR);
        };
        if self.visible(x, y, w, h).is_none() {
            return Ok(());
        }
        let s = self.scale;
        let Some(placement) = place(
            fit_mode,
            img.width,
            img.height,
            x * s,
            y * s,
            w * s,
            h * s,
            self.pixmap.width(),
            self.pixmap.height(),
        ) else {
            return Ok(());
        };
        self.budget
            .charge(placement.area().saturating_mul(PIPELINE_WEIGHT))?;
        let mask = self.clip.mask(&mut self.budget)?;
        let Some(sampled) = resample(img, &placement) else {
            return Ok(());
        };
        self.pixmap.draw_pixmap(
            placement.px0,
            placement.py0,
            sampled.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            mask,
        );
        Ok(())
    }

    fn clip_push(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32) {
        if !sane(&[x, y, w, h, radius]) {
            self.clip.push(ClipShape {
                rect: None,
                radius: 0.0,
            });
            return;
        }
        let s = self.scale;
        self.clip.push(ClipShape {
            rect: Rect::from_xywh(x * s, y * s, (w * s).max(0.0), (h * s).max(0.0)),
            radius: (radius * s).max(0.0),
        });
    }

    fn text(
        &mut self,
        x: f32,
        y: f32,
        max_w: f32,
        style_id: u16,
        text: &[u8],
    ) -> Result<(), Exhausted> {
        // Invalid UTF-8 is drawn lossily; the module is told nothing extra (SPEC-6.4.1).
        let s = String::from_utf8_lossy(text);
        let Some(style) = self.text.get_style(style_id) else {
            return Ok(());
        };
        if self.clip.bounds().is_none() {
            return Ok(());
        }
        // Shaping is paid up front per byte; the glyphs as they are blitted.
        self.budget
            .charge((text.len() as u64).saturating_mul(PIPELINE_WEIGHT))?;
        let rgba = style.color;
        let scale = self.scale;
        let mask = self.clip.mask(&mut self.budget)?;
        let pixmap = &mut self.pixmap;
        let mut glyph_px = 0u64;
        let _ = self
            .text
            .rasterize(style_id, (x, y), max_w, scale, &s, |g| {
                glyph_px +=
                    u64::from(g.image.placement.width) * u64::from(g.image.placement.height);
                blit_glyph(pixmap, mask, &g, rgba);
            });
        self.budget
            .charge_done(glyph_px.saturating_mul(PIPELINE_WEIGHT));
        Ok(())
    }
}

/// Composite one glyph bitmap in `rgba` onto the pixmap through the clip mask.
fn blit_glyph(pixmap: &mut Pixmap, mask: Option<&Mask>, g: &Glyph<'_>, rgba: u32) {
    let (w, h) = (g.image.placement.width, g.image.placement.height);
    let Some(size) = IntSize::from_wh(w, h) else {
        return;
    };
    let (r, gg, b, a) = (
        (rgba >> 24) & 0xFF,
        (rgba >> 16) & 0xFF,
        (rgba >> 8) & 0xFF,
        rgba & 0xFF,
    );
    let n = (w as usize) * (h as usize);
    let mut px = Vec::with_capacity(n * 4);
    match g.image.content {
        SwashContent::Mask => {
            for &cov in g.image.data.iter().take(n) {
                let alpha = (u32::from(cov) * a + 127) / 255;
                px.extend_from_slice(&[
                    ((r * alpha + 127) / 255) as u8,
                    ((gg * alpha + 127) / 255) as u8,
                    ((b * alpha + 127) / 255) as u8,
                    alpha as u8,
                ]);
            }
        }
        SwashContent::SubpixelMask => {
            for cov in g.image.data.chunks_exact(3).take(n) {
                let c = (u32::from(cov[0]) + u32::from(cov[1]) + u32::from(cov[2])) / 3;
                let alpha = (c * a + 127) / 255;
                px.extend_from_slice(&[
                    ((r * alpha + 127) / 255) as u8,
                    ((gg * alpha + 127) / 255) as u8,
                    ((b * alpha + 127) / 255) as u8,
                    alpha as u8,
                ]);
            }
        }
        SwashContent::Color => {
            // Colour bitmaps keep their own colour; only the style alpha applies.
            for p in g.image.data.chunks_exact(4).take(n) {
                let alpha = (u32::from(p[3]) * a + 127) / 255;
                px.extend_from_slice(&[
                    ((u32::from(p[0]) * alpha + 127) / 255) as u8,
                    ((u32::from(p[1]) * alpha + 127) / 255) as u8,
                    ((u32::from(p[2]) * alpha + 127) / 255) as u8,
                    alpha as u8,
                ]);
            }
        }
    }
    if px.len() != n * 4 {
        return;
    }
    let Some(glyph) = Pixmap::from_vec(px, size) else {
        return;
    };
    pixmap.draw_pixmap(
        g.x,
        g.y,
        glyph.as_ref(),
        &PixmapPaint::default(),
        Transform::identity(),
        mask,
    );
}

/// Rasterize a display list into a `width` × `height` physical-pixel image.
///
/// `width` and `height` are physical pixels; `scale` is the logical-to-physical
/// ratio (SPEC-6.3.1). The canvas starts opaque white. `images` holds the
/// package's resources, decoded by the runtime when `dl_image` ran
/// (SPEC-6.5.1); an `IMAGE` op naming a resource that is not decoded draws
/// [`IMAGE_PLACEHOLDER_COLOR`]. The work is bounded by
/// [`raster_budget_units`] (SPEC-5.3.3): when it runs out the remaining ops
/// are not drawn and the report says so.
pub fn render(
    display_list: &[u8],
    width: u32,
    height: u32,
    scale: f32,
    text: &mut TextEngine,
    images: &ImageCache,
) -> Result<Rendered, RenderError> {
    let ops = vy_dl::decode(display_list).map_err(RenderError::Decode)?;
    let scale_ok = scale.is_finite() && scale > 0.0;
    let side = 1..=MAX_CANVAS_SIDE_PX;
    if !side.contains(&width) || !side.contains(&height) || !scale_ok {
        return Err(RenderError::BadSize);
    }
    let mut pixmap = Pixmap::new(width, height).ok_or(RenderError::BadSize)?;
    pixmap.fill(Color::WHITE);
    let budget = raster_budget_units(width, height);
    let mut canvas = Canvas {
        pixmap,
        scale,
        clip: Clip::new(width, height),
        budget: Budget {
            limit: budget,
            used: 0,
        },
        text,
        images,
    };
    // Ops with non-finite or absurd values are skipped (`MAX_MAGNITUDE`);
    // `CLIP_PUSH` is the exception and always pushes, see `Canvas::clip_push`.
    // A rounded clip (`radius > 0`) clips to a rounded rectangle (SPEC-5.2.3).
    let mut ops_done = 0;
    let mut truncated = false;
    for op in &ops {
        if canvas.budget.exhausted() || canvas.budget.charge(OP_UNITS).is_err() {
            truncated = true;
            break;
        }
        let drawn = match op {
            Op::Rect {
                x,
                y,
                w,
                h,
                radius,
                color,
            } if sane(&[*x, *y, *w, *h, *radius]) => canvas.rect(*x, *y, *w, *h, *radius, *color),
            Op::Text {
                x,
                y,
                max_w,
                style_id,
                text,
            } if sane(&[*x, *y]) => canvas.text(*x, *y, *max_w, *style_id, text),
            Op::Image {
                x,
                y,
                w,
                h,
                resource_id,
                fit,
            } if sane(&[*x, *y, *w, *h]) => canvas.image(*x, *y, *w, *h, *resource_id, *fit),
            Op::Path {
                color,
                stroke_w,
                cmds,
            } if sane_path(cmds, *stroke_w) => canvas.path(cmds, *color, *stroke_w),
            Op::ClipPush { x, y, w, h, radius } => {
                canvas.clip_push(*x, *y, *w, *h, *radius);
                Ok(())
            }
            Op::ClipPop => {
                canvas.clip.pop();
                Ok(())
            }
            Op::Rect { .. } | Op::Text { .. } | Op::Image { .. } | Op::Path { .. } => Ok(()),
        };
        if drawn.is_err() {
            truncated = true;
            break;
        }
        ops_done += 1;
    }
    Ok(Rendered {
        image: RgbaImage {
            width,
            height,
            data: canvas.pixmap.take_demultiplied(),
        },
        report: RasterReport {
            truncated,
            ops_done,
            units: canvas.budget.used,
            budget,
        },
    })
}
