//! Software rasterizer for display lists (SPEC §6).
//!
//! Every op is drawn in order, later over earlier (SPEC-6.1.3). Coordinates
//! are logical pixels scaled by `scale` to physical pixels (SPEC-6.3.1).
//! Headless and windowed rendering share this function (SPEC-11.3.2).
//!
//! Two things bound the host's own work here (SPEC-5.3.3, SPEC-9.3.1):
//!
//! - **The clip stack is incremental.** Each `CLIP_PUSH` adds a level that
//!   knows the analytic intersection of the rectangles so far; a mask is
//!   built only when an op needs one, from the parent's level, over the
//!   shape's own bounds, and kept for the level's life, so a push or pop
//!   never rebuilds the whole stack, a plain-rectangle clip's mask is a
//!   memset of pixels computed with the scan converter's rounding, and ops
//!   are cut to the clip's bounds before they are filled (see [`Clip`]).
//! - **The frame has a work budget.** Every op is charged units proportional
//!   to the pixels it makes the rasterizer touch ([`Budget`]); when the
//!   budget runs out the rest of the list is not drawn and [`RasterReport`]
//!   says so, so a legal list of 65 536 full-screen fills costs the main
//!   process a bounded amount of time instead of minutes.
//! - **Speed never changes a byte (SPEC-13.2.1, SPEC-2.1.5).** The goldens
//!   are what tiny-skia draws through a clip mask, and the budget is what
//!   M3 charged; the fast paths reproduce both exactly. Under plain
//!   rectangular clips no mask is built: ops are cut to the clip's pixels
//!   (where a plain clip's mask is 255, the identity in tiny-skia's
//!   arithmetic) and opaque fills are drawn with an alpha a hair below one
//!   ([`ALPHA_BELOW_ONE`]), so tiny-skia keeps the source-over blend a mask
//!   forces instead of switching to its source blend; the mask's cost is
//!   still charged, once per level. Rectangles on whole pixels are filled
//!   here with tiny-skia's low-precision integer arithmetic, and glyphs and
//!   images are composited here with its high-precision float arithmetic
//!   ([`over_highp`]), without a pixmap and three compiled pipelines per
//!   glyph. Masks of rounded clips come from a pool and are cleared over
//!   what they covered. `raster_reference.rs` keeps the M3 rasterizer for
//!   the tests that compare the two byte for byte.

use crate::image::{ImageCache, RgbaImage};
use crate::text::{Glyph, TextEngine};
use cosmic_text::SwashContent;
use tiny_skia::{
    Color, FillRule, IntSize, Mask, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform,
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

/// The largest `f32` below one. tiny-skia blends an opaque colour without
/// a mask as "source" (a lerp by coverage) and with a mask as "source
/// over" (coverage pre-scaled), which differ by one unit at anti-aliased
/// edges; a colour with this alpha is not opaque to tiny-skia, so it keeps
/// source over, while the colour it stores is the same `u8` as alpha one
/// gives (`(v × a × 255 + 0.5)` truncates the same, see the tests).
pub const ALPHA_BELOW_ONE: f32 = 1.0 - f32::EPSILON / 2.0;

/// [`paint`] blended as under a clip mask: source over even when opaque.
fn paint_over(rgba: u32) -> Paint<'static> {
    let mut c = color(rgba);
    if c.is_opaque() {
        c.set_alpha(ALPHA_BELOW_ONE);
    }
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

/// Rounded rectangle path in logical coordinates, built in `pb` (whose
/// buffers are reused); `None` if degenerate.
fn rounded_rect(mut pb: PathBuilder, x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<Path> {
    if !(w > 0.0 && h > 0.0) || !x.is_finite() || !y.is_finite() {
        return None;
    }
    let r = radius.max(0.0).min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        // The same verbs, points and bounds as `PathBuilder::from_rect`.
        pb.push_rect(Rect::from_xywh(x, y, w, h)?);
        return pb.finish();
    }
    // Cubic approximation of a quarter circle.
    const K: f32 = 0.552_284_8;
    let k = r * K;
    let (l, t, rt, b) = (x, y, x + w, y + h);
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

    fn contains(&self, o: &Pixels) -> bool {
        o.x0 >= self.x0 && o.y0 >= self.y0 && o.x1 <= self.x1 && o.y1 <= self.y1
    }

    /// Grown by `n` pixels on every side, within a `w` × `h` canvas.
    fn grown(&self, n: i32, w: u32, h: u32) -> Pixels {
        Pixels {
            x0: (self.x0 - n).max(0),
            y0: (self.y0 - n).max(0),
            x1: (self.x1 + n).min(w as i32),
            y1: (self.y1 + n).min(h as i32),
        }
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
    /// True once the mask of a plain level has been paid for, built or
    /// not: ops under a plain clip are drawn without it but charged as if
    /// it had been built at their first use.
    charged: bool,
    /// Where the mask may be non-zero: cleared when the level is popped
    /// and the mask goes back to the pool.
    dirty: Option<Pixels>,
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
    /// All-zero masks of popped levels, for the next level that needs one.
    pool: Vec<Mask>,
}

impl Clip {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            levels: Vec::new(),
            pool: Vec::new(),
        }
    }

    /// An all-zero mask the size of the canvas.
    fn zeroed_mask(&mut self) -> Option<Mask> {
        self.pool
            .pop()
            .or_else(|| Mask::new(self.width, self.height))
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
            charged: false,
            dirty: None,
        });
    }

    fn pop(&mut self) {
        let Some(level) = self.levels.pop() else {
            return;
        };
        if let Some(mut mask) = level.mask {
            if let Some(d) = level.dirty {
                let w = self.width as usize;
                let data = mask.data_mut();
                for y in d.y0..d.y1 {
                    let row = y as usize * w;
                    data[row + d.x0 as usize..row + d.x1 as usize].fill(0);
                }
            }
            self.pool.push(mask);
        }
    }

    /// The current level's mask, when it has been built.
    fn top_mask(&self) -> Option<&Mask> {
        self.top().and_then(|l| l.mask.as_ref())
    }

    /// The current level when it is plain: ops under it are cut to its
    /// pixels and drawn without a mask.
    fn plain_top(&self) -> Option<&Level> {
        self.top().filter(|l| !l.rounded)
    }

    /// Charge the current plain level's mask without building it, once per
    /// level, exactly when [`Clip::mask`] would have built and charged it.
    fn charge_plain(&mut self, budget: &mut Budget) -> Result<(), Exhausted> {
        let Some(level) = self.levels.last_mut() else {
            return Ok(());
        };
        debug_assert!(!level.rounded);
        if level.charged || level.mask.is_some() {
            return Ok(());
        }
        if let Some(bounds) = level.bounds {
            budget.charge(bounds.area())?;
        }
        level.charged = true;
        Ok(())
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
            self.levels[k].mask = self.zeroed_mask();
            return Ok(());
        };
        if !self.levels[k].rounded {
            if !self.levels[k].charged {
                budget.charge(bounds.area())?;
                self.levels[k].charged = true;
            }
            let mut mask = self.zeroed_mask().ok_or(Exhausted)?;
            fill_pixels(&mut mask, &bounds, w);
            self.levels[k].mask = Some(mask);
            self.levels[k].dirty = Some(bounds);
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
        let mut mask = self.zeroed_mask().ok_or(Exhausted)?;
        match shape.rect {
            Some(r) if shape.radius > 0.0 => {
                if let Some(path) = rounded_rect(
                    PathBuilder::new(),
                    r.x(),
                    r.y(),
                    r.width(),
                    r.height(),
                    shape.radius,
                ) {
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
        // The fill touches the shape's covering pixels; a pixel of margin
        // costs nothing and keeps the pool's masks zero if it ever did not.
        self.levels[k].dirty = Some(region.grown(1, w, h));
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
    /// Buffers of the last path, reused by the next.
    builder: PathBuilder,
    /// One row of a whole-pixel fill, reused.
    row: Vec<u8>,
    /// Source columns of an image's pixels, reused.
    columns: Vec<usize>,
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

/// `1 / 255` as tiny-skia's high-precision pipeline loads a channel.
const INV_255: f32 = 1.0 / 255.0;

/// `v.round_ties_even() as u8` for `0 ≤ v ≤ 255`, as tiny-skia stores a
/// channel (`cvtps2dq` on x86_64, `vcvtnq` on AArch64: nearest, half to
/// even). Adding 2²³ leaves no fraction bits, so the add itself rounds
/// that way (IEEE 754 default rounding) and the subtraction is exact; this
/// is two instructions where `round_ties_even` is a libm call on x86_64
/// without SSE4.1.
#[inline]
fn round_even_u8(v: f32) -> u8 {
    const TWO_23: f32 = 8_388_608.0;
    ((v + TWO_23) - TWO_23) as u8
}

/// Source over of one premultiplied pixel `s` onto `d` through clip-mask
/// coverage `m` (255 without a mask), with exactly the arithmetic of
/// tiny-skia's high-precision pipeline, which `draw_pixmap` always runs
/// (a pattern shader gathers): channels loaded as `v × (1/255)`, the mask
/// as `m / 255`, `d × (1 − sa) + s`, clamped to 0..1, × 255 and rounded
/// half to even. `m = 0` leaves the pixel as it is, as the pipeline skips
/// it. A transparent source leaves it too, and an opaque one under full
/// coverage replaces it; the exhaustive test against `draw_pixmap` covers
/// both shortcuts.
#[inline]
fn over_highp(d: &mut [u8], s: [u8; 4], m: u8) {
    if m == 0 || s == [0; 4] {
        return;
    }
    if m == 255 && s[3] == 255 {
        d[..4].copy_from_slice(&s);
        return;
    }
    let mut sf = [
        f32::from(s[0]) * INV_255,
        f32::from(s[1]) * INV_255,
        f32::from(s[2]) * INV_255,
        f32::from(s[3]) * INV_255,
    ];
    if m != 255 {
        let c = f32::from(m) / 255.0;
        for v in &mut sf {
            *v *= c;
        }
    }
    let inv = 1.0 - sf[3];
    for i in 0..4 {
        let out = f32::from(d[i]) * INV_255 * inv + sf[i];
        d[i] = round_even_u8(out.clamp(0.0, 1.0) * 255.0);
    }
}

/// The premultiplied colour tiny-skia's low-precision pipeline fills with
/// (`push_uniform_color`): the colour as floats, premultiplied, × 255 + 0.5
/// truncated.
fn uniform_u16(rgba: u32) -> [u16; 4] {
    let c = color(rgba).premultiply();
    let q = |v: f32| (v * 255.0 + 0.5) as u16;
    [q(c.red()), q(c.green()), q(c.blue()), q(c.alpha())]
}

/// Fill whole pixels `p` with `rgba` as tiny-skia does a rectangle that
/// covers them fully: the colour itself when opaque (one row built in
/// `row`, then copied, so it is a memcpy in any build), otherwise its
/// low-precision source over, `s + (d × (255 − sa) + 255) >> 8`.
fn fill_whole_pixels(pixmap: &mut Pixmap, row: &mut Vec<u8>, p: &Pixels, rgba: u32) {
    let w = pixmap.width() as usize;
    let data = pixmap.data_mut();
    let [r, g, b, a] = uniform_u16(rgba);
    let (x0, x1) = (p.x0 as usize * 4, p.x1 as usize * 4);
    if a == 255 {
        let px = [r as u8, g as u8, b as u8, 255];
        row.clear();
        row.extend(px.iter().copied().cycle().take(x1 - x0));
        for y in p.y0..p.y1 {
            let at = y as usize * w * 4;
            data[at + x0..at + x1].copy_from_slice(row);
        }
        return;
    }
    let inv = 255 - a;
    let over = |s: u16, d: u8| (s + ((u16::from(d) * inv + 255) >> 8)) as u8;
    for y in p.y0..p.y1 {
        let at = y as usize * w * 4;
        for d in data[at + x0..at + x1].chunks_exact_mut(4) {
            d[0] = over(r, d[0]);
            d[1] = over(g, d[1]);
            d[2] = over(b, d[2]);
            d[3] = over(a, d[3]);
        }
    }
}

/// The whole pixels `r` covers when every edge lies on a pixel boundary,
/// limited to a `w` × `h` canvas: what an anti-aliased fill of it covers
/// fully, and nothing else. `None` for a rectangle with an edge between
/// pixels, or nothing on the canvas.
fn whole_pixels(r: &Rect, w: u32, h: u32) -> Option<Pixels> {
    let edges = [r.left(), r.top(), r.right(), r.bottom()];
    if edges.iter().any(|e| e.fract() != 0.0) {
        return None;
    }
    let p = Pixels {
        x0: (r.left().max(0.0)) as i32,
        y0: (r.top().max(0.0)) as i32,
        x1: (r.right().min(w as f32)) as i32,
        y1: (r.bottom().min(h as f32)) as i32,
    };
    (p.x1 > p.x0 && p.y1 > p.y0).then_some(p)
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

/// How an op under the current clip reaches the canvas.
enum Through {
    /// No clip: anywhere on the canvas.
    Canvas,
    /// A plain clip, already charged: within these pixels, no mask.
    Pixels(Pixels),
    /// Through the current level's mask, built and charged
    /// ([`Clip::top_mask`]).
    Mask,
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

    /// How an op that touches at most `own` reaches the canvas: directly
    /// without a clip; cut to a plain clip's pixels (charged as its mask)
    /// when `cut` says the op can be cut there, or when it lies within
    /// them; otherwise through the mask, built and charged on first use.
    fn through(&mut self, own: Option<Pixels>, cut: bool) -> Result<Through, Exhausted> {
        let Some(top) = self.clip.top() else {
            return Ok(Through::Canvas);
        };
        if let (Some(level), Some(bounds)) = (self.clip.plain_top(), top.bounds) {
            debug_assert!(level.exact == level.bounds);
            if cut || own.is_some_and(|o| bounds.contains(&o)) {
                self.clip.charge_plain(&mut self.budget)?;
                return Ok(Through::Pixels(bounds));
            }
        }
        match self.clip.mask(&mut self.budget)? {
            Some(_) => Ok(Through::Mask),
            None => Ok(Through::Canvas),
        }
    }

    /// Fill a path already in physical pixels with `paint` through `mask`,
    /// then keep its buffers for the next path.
    fn fill_physical(&mut self, path: Path, paint: &Paint<'_>, mask: Option<&Mask>) {
        self.pixmap
            .fill_path(&path, paint, FillRule::Winding, Transform::identity(), mask);
        self.builder = path.clear();
    }

    /// Fill a logical-coordinate path through the clip, charging `area`
    /// physical pixels at the pipeline weight (or at 1 when the fill is an
    /// opaque memset: no clip, opaque colour). The path is scaled here the
    /// way `fill_path` scales it (`Path::transform`), so the edges land on
    /// the same values.
    fn fill(&mut self, path: Path, rgba: u32, area: u64) -> Result<(), Exhausted> {
        let plain = self.clip.top().is_none() && rgba & 0xFF == 0xFF;
        let weight = if plain { 1 } else { PIPELINE_WEIGHT };
        self.budget.charge(area.saturating_mul(weight))?;
        let t = self.transform();
        let Some(path) = path.transform(t) else {
            return Ok(());
        };
        let own = covering_pixels(&path.bounds(), self.pixmap.width(), self.pixmap.height());
        match self.through(own, false)? {
            Through::Canvas => {
                let p = paint(rgba);
                self.fill_physical(path, &p, None);
            }
            Through::Pixels(_) => {
                let p = paint_over(rgba);
                self.fill_physical(path, &p, None);
            }
            Through::Mask => {
                let p = paint(rgba);
                self.pixmap.fill_path(
                    &path,
                    &p,
                    FillRule::Winding,
                    Transform::identity(),
                    self.clip.top_mask(),
                );
                self.builder = path.clear();
            }
        }
        Ok(())
    }

    fn stroke(&mut self, path: &Path, rgba: u32, width: f32, units: u64) -> Result<(), Exhausted> {
        self.budget.charge(units)?;
        let t = self.transform();
        let stroke = Stroke {
            width: width.max(0.0),
            ..Stroke::default()
        };
        // A stroke reaches past its bounds by its joins, so it is never
        // cut: under a clip it goes through the mask.
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
        let builder = std::mem::take(&mut self.builder);
        if let Some(path) = rounded_rect(builder, x, y, w, h, radius) {
            self.fill(path, rgba, visible.area())?;
        }
        Ok(())
    }

    /// A plain rectangle. Under a clip it is first cut to the clip's pixel
    /// bounds, where the mask is zero anyway, so the fill touches only
    /// pixels that can change. A rectangle on whole pixels is filled here
    /// ([`fill_whole_pixels`]); otherwise tiny-skia fills it, through the
    /// mask of a rounded clip, or blended as through one under a plain clip
    /// ([`paint_over`]), because a masked and an unmasked opaque fill
    /// differ by a unit at anti-aliased edges and the goldens are the
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
        let (cw, ch) = (self.pixmap.width(), self.pixmap.height());
        let opaque = rgba & 0xFF == 0xFF;
        if self.clip.top().is_none() {
            let weight = if opaque { 1 } else { PIPELINE_WEIGHT };
            self.budget.charge(visible.area().saturating_mul(weight))?;
            if let Some(p) = whole_pixels(&r, cw, ch) {
                fill_whole_pixels(&mut self.pixmap, &mut self.row, &p, rgba);
                return Ok(());
            }
            let mut pb = std::mem::take(&mut self.builder);
            pb.push_rect(r);
            if let Some(path) = pb.finish() {
                self.fill_physical(path, &paint(rgba), None);
            }
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
        let mut pb = std::mem::take(&mut self.builder);
        pb.push_rect(c);
        let Some(path) = pb.finish() else {
            return Ok(());
        };
        if let Through::Mask = self.through(None, true)? {
            self.pixmap.fill_path(
                &path,
                &paint(rgba),
                FillRule::Winding,
                Transform::identity(),
                self.clip.top_mask(),
            );
            self.builder = path.clear();
            return Ok(());
        }
        // Cut to a plain clip's pixels (there is a clip, so never `Canvas`).
        match whole_pixels(&c, cw, ch) {
            Some(p) => {
                fill_whole_pixels(&mut self.pixmap, &mut self.row, &p, rgba);
                self.builder = path.clear();
            }
            None => self.fill_physical(path, &paint_over(rgba), None),
        }
        Ok(())
    }

    fn path(&mut self, cmds: &[PathCmd], rgba: u32, stroke_w: f32) -> Result<(), Exhausted> {
        let mut pb = std::mem::take(&mut self.builder);
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
            let stroked = self.stroke(&path, rgba, stroke_w, units);
            self.builder = path.clear();
            stroked
        } else {
            let Some(visible) = self.visible(b.left(), b.top(), b.width(), b.height()) else {
                return Ok(());
            };
            self.budget.charge(setup)?;
            self.fill(path, rgba, visible.area())
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
        let images = self.images;
        let Ok(img) = images.get(resource_id) else {
            return self.rect(x, y, w, h, 0.0, IMAGE_PLACEHOLDER_COLOR);
        };
        if self.visible(x, y, w, h).is_none() {
            return Ok(());
        }
        let s = self.scale;
        let (cw, ch) = (self.pixmap.width(), self.pixmap.height());
        let Some(p) = place(
            fit_mode,
            img.width,
            img.height,
            x * s,
            y * s,
            w * s,
            h * s,
            cw,
            ch,
        ) else {
            return Ok(());
        };
        self.budget
            .charge(p.area().saturating_mul(PIPELINE_WEIGHT))?;
        let all = Pixels {
            x0: p.px0,
            y0: p.py0,
            x1: p.px1,
            y1: p.py1,
        };
        // Nearest-neighbour sampling in integer and IEEE-754 arithmetic, so
        // the same list gives the same pixels on every machine
        // (SPEC-2.1.5), composited as `draw_pixmap` composites the sampled
        // pixmap. Each pixel is sampled on its own, so cutting the region to
        // a plain clip changes nothing in it.
        let (region, masked) = match self.through(None, true)? {
            Through::Canvas => (Some(all), false),
            Through::Pixels(b) => (all.intersect(&b), false),
            Through::Mask => (Some(all), true),
        };
        let Some(region) = region else {
            return Ok(());
        };
        let mask = if masked {
            self.clip.top_mask().map(Mask::data)
        } else {
            None
        };
        let data = self.pixmap.data_mut();
        let (iw, ih) = (i64::from(img.width), i64::from(img.height));
        // The source column of a pixel depends on its x alone: computed
        // once per column, with the same expression as per pixel.
        let columns = &mut self.columns;
        columns.clear();
        columns.extend((region.x0..region.x1).map(|px| {
            let sx = ((px as f32 + 0.5 - p.rx) / p.kx).floor() as i64;
            sx.clamp(0, iw - 1) as usize
        }));
        for py in region.y0..region.y1 {
            let sy = ((py as f32 + 0.5 - p.ry) / p.ky).floor() as i64;
            let sy = sy.clamp(0, ih - 1) as usize;
            let src_row = &img.data[sy * img.width as usize * 4..][..img.width as usize * 4];
            let row = py as usize * cw as usize;
            for (px, &sx) in (region.x0..region.x1).zip(columns.iter()) {
                let q = &src_row[sx * 4..sx * 4 + 4];
                let a = u32::from(q[3]);
                let src = [
                    ((u32::from(q[0]) * a + 127) / 255) as u8,
                    ((u32::from(q[1]) * a + 127) / 255) as u8,
                    ((u32::from(q[2]) * a + 127) / 255) as u8,
                    q[3],
                ];
                let i = row + px as usize;
                let m = mask.map_or(255, |m| m[i]);
                over_highp(&mut data[i * 4..i * 4 + 4], src, m);
            }
        }
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
        let (cw, ch) = (self.pixmap.width(), self.pixmap.height());
        let canvas = Pixels {
            x0: 0,
            y0: 0,
            x1: cw as i32,
            y1: ch as i32,
        };
        let (limit, masked) = match self.through(None, true)? {
            Through::Canvas => (canvas, false),
            Through::Pixels(b) => (b, false),
            Through::Mask => (canvas, true),
        };
        let mask = if masked { self.clip.top_mask() } else { None };
        let pixmap = &mut self.pixmap;
        let tint = tint_table(rgba);
        let mut glyph_px = 0u64;
        let _ = self
            .text
            .rasterize(style_id, (x, y), max_w, scale, &s, |g| {
                glyph_px +=
                    u64::from(g.image.placement.width) * u64::from(g.image.placement.height);
                blit_glyph(pixmap, &limit, mask, &g, &tint, rgba & 0xFF);
            });
        self.budget
            .charge_done(glyph_px.saturating_mul(PIPELINE_WEIGHT));
        Ok(())
    }
}

/// The premultiplied pixel M3 built for each glyph coverage `0..=255` in a
/// style colour `rgba`: `alpha = (cov × a + 127) / 255`, each channel
/// `(c × alpha + 127) / 255`. One table per text run, so the pixels of a
/// glyph are table lookups.
fn tint_table(rgba: u32) -> [[u8; 4]; 256] {
    let (r, g, b, a) = (
        (rgba >> 24) & 0xFF,
        (rgba >> 16) & 0xFF,
        (rgba >> 8) & 0xFF,
        rgba & 0xFF,
    );
    std::array::from_fn(|cov| {
        let alpha = (cov as u32 * a + 127) / 255;
        [
            ((r * alpha + 127) / 255) as u8,
            ((g * alpha + 127) / 255) as u8,
            ((b * alpha + 127) / 255) as u8,
            alpha as u8,
        ]
    })
}

/// Composite one glyph bitmap onto the pixmap within `limit`, through the
/// clip mask when there is one, as `draw_pixmap` composites the
/// premultiplied glyph pixmap M3 built for it ([`over_highp`]): coverage
/// tinted with `tint` ([`tint_table`]), subpixel coverage averaged first,
/// colour bitmaps keeping their colour with the style alpha `alpha`
/// applied. A bitmap shorter than its placement says draws nothing.
fn blit_glyph(
    pixmap: &mut Pixmap,
    limit: &Pixels,
    mask: Option<&Mask>,
    g: &Glyph<'_>,
    tint: &[[u8; 4]; 256],
    alpha: u32,
) {
    let (w, h) = (g.image.placement.width, g.image.placement.height);
    let n = (w as usize) * (h as usize);
    let bytes_per_px = match g.image.content {
        SwashContent::Mask => 1,
        SwashContent::SubpixelMask => 3,
        SwashContent::Color => 4,
    };
    if n == 0 || g.image.data.len() < n * bytes_per_px {
        return;
    }
    let glyph = Pixels {
        x0: g.x,
        y0: g.y,
        x1: g.x.saturating_add(w as i32),
        y1: g.y.saturating_add(h as i32),
    };
    let Some(region) = glyph.intersect(limit) else {
        return;
    };
    let cw = pixmap.width() as usize;
    let mask = mask.map(Mask::data);
    let data = pixmap.data_mut();
    let src = &g.image.data;
    let (x0, x1) = (region.x0 as usize, region.x1 as usize);
    let gx0 = (region.x0 - g.x) as usize;
    for y in region.y0..region.y1 {
        let at = y as usize * cw;
        let dst = &mut data[(at + x0) * 4..(at + x1) * 4];
        let masks = mask.map(|m| &m[at + x0..at + x1]);
        let k0 = (y - g.y) as usize * w as usize + gx0;
        let len = x1 - x0;
        let m_at = |j: usize| masks.map_or(255, |m| m[j]);
        match g.image.content {
            SwashContent::Mask => {
                for (j, (&cov, d)) in src[k0..k0 + len]
                    .iter()
                    .zip(dst.chunks_exact_mut(4))
                    .enumerate()
                {
                    over_highp(d, tint[usize::from(cov)], m_at(j));
                }
            }
            SwashContent::SubpixelMask => {
                for (j, (c, d)) in src[k0 * 3..(k0 + len) * 3]
                    .chunks_exact(3)
                    .zip(dst.chunks_exact_mut(4))
                    .enumerate()
                {
                    let cov = (u32::from(c[0]) + u32::from(c[1]) + u32::from(c[2])) / 3;
                    over_highp(d, tint[cov as usize], m_at(j));
                }
            }
            SwashContent::Color => {
                for (j, (p, d)) in src[k0 * 4..(k0 + len) * 4]
                    .chunks_exact(4)
                    .zip(dst.chunks_exact_mut(4))
                    .enumerate()
                {
                    let a = (u32::from(p[3]) * alpha + 127) / 255;
                    let px = [
                        ((u32::from(p[0]) * a + 127) / 255) as u8,
                        ((u32::from(p[1]) * a + 127) / 255) as u8,
                        ((u32::from(p[2]) * a + 127) / 255) as u8,
                        a as u8,
                    ];
                    over_highp(d, px, m_at(j));
                }
            }
        }
    }
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
    // Opaque white, premultiplied: every byte 255.
    let size = IntSize::from_wh(width, height).ok_or(RenderError::BadSize)?;
    let pixmap = Pixmap::from_vec(vec![255; width as usize * height as usize * 4], size)
        .ok_or(RenderError::BadSize)?;
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
        builder: PathBuilder::new(),
        row: Vec::new(),
        columns: Vec::new(),
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
    // The canvas starts opaque and every op blends source over it, so every
    // pixel stays opaque and demultiplying would change nothing.
    let data = canvas.pixmap.take();
    debug_assert!(data.chunks_exact(4).all(|p| p[3] == 255));
    Ok(Rendered {
        image: RgbaImage {
            width,
            height,
            data,
        },
        report: RasterReport {
            truncated,
            ops_done,
            units: canvas.budget.used,
            budget,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vy_dl::{DisplayList, Fit};

    /// `exact_pixels` must agree with tiny-skia's own non-anti-aliased fill
    /// of the same rectangle, which is what a clip mask used to be built
    /// from: the fast path for rectangles under rectangular clips is only
    /// correct if the two never differ, so a few hundred thousand random
    /// rectangles, with edges on and around pixel centres and outside the
    /// canvas, are compared mask against mask.
    #[test]
    fn exact_pixels_match_tiny_skia() {
        let _serial = crate::tests::serial();
        let (w, h) = (24u32, 17u32);
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let coord = |v: u64, lo: f32, hi: f32| lo + (v % 100_000) as f32 / 100_000.0 * (hi - lo);
        let mut checked = 0;
        for i in 0..200_000u32 {
            let (a, b, c, d) = (next(), next(), next(), next());
            // Mostly inside, sometimes off the canvas; sizes from slivers up.
            let x = coord(a, -3.0, w as f32 + 3.0);
            let y = coord(b, -3.0, h as f32 + 3.0);
            let (rw, rh) = match i % 7 {
                0 => (coord(c, 0.0, 0.001), coord(d, 0.0, 5.0)),
                1 => (coord(c, 0.0, 5.0), coord(d, 0.0, 0.001)),
                _ => (coord(c, 0.0, 12.0), coord(d, 0.0, 12.0)),
            };
            // Edges exactly on pixel centres and boundaries too.
            let snap = |v: f32| match i % 5 {
                0 => v.floor() + 0.5,
                1 => v.floor(),
                _ => v,
            };
            let (x, y) = (snap(x), snap(y));
            let Some(r) = Rect::from_xywh(x, y, rw, rh) else {
                continue;
            };
            let mut mask = Mask::new(w, h).unwrap();
            mask.fill_path(
                &PathBuilder::from_rect(r),
                FillRule::Winding,
                false,
                Transform::identity(),
            );
            let mut mine = Mask::new(w, h).unwrap();
            if let Some(p) = exact_pixels(&r, w, h) {
                fill_pixels(&mut mine, &p, w);
            }
            assert_eq!(
                mine.data(),
                mask.data(),
                "rect {x} {y} {rw} {rh}: exact_pixels {:?}",
                exact_pixels(&r, w, h)
            );
            checked += 1;
        }
        assert!(checked > 150_000);
    }

    /// The budget constants hold the frames the spec sentence is about: a
    /// legal 2 000-op frame at 1080p fits with margin, 65 000 full-screen
    /// rectangles at 1280 × 800 do not.
    #[test]
    fn budget_sizing() {
        let legal = 2_000 * (OP_UNITS + 100 * 50) + 2_000 * 200 * PIPELINE_WEIGHT;
        assert!(legal * 4 < raster_budget_units(1920, 1080), "{legal}");
        let hostile = 65_000 * (OP_UNITS + 1280 * 800);
        assert!(hostile > raster_budget_units(1280, 800) * 100);
        assert_eq!(raster_budget_units(1280, 800), RASTER_BUDGET_UNITS);
        assert_eq!(raster_budget_units(1, 1), RASTER_BUDGET_UNITS);
        assert_eq!(
            raster_budget_units(3840, 2160),
            3840 * 2160 * RASTER_BUDGET_OVERDRAWS
        );
        assert_eq!(multiply_coverage(255, 255), 255);
        assert_eq!(multiply_coverage(100, 255), 100);
        assert_eq!(multiply_coverage(0, 77), 0);
        assert_eq!(multiply_coverage(128, 128), 64);
    }

    /// The largest `f32` below one is not opaque to tiny-skia but stores
    /// the same colour bytes as alpha one, for every channel value: the
    /// arithmetic of `Color::premultiply` and `push_uniform_color`.
    #[test]
    fn alpha_below_one_stores_the_same_colour() {
        assert_eq!(ALPHA_BELOW_ONE.next_up(), 1.0);
        let q = |v: f32| (v * 255.0 + 0.5) as u16;
        for v in 0..=255u8 {
            let opaque = Color::from_rgba8(v, v, v, 255).premultiply();
            let mut c = Color::from_rgba8(v, v, v, 255);
            c.set_alpha(ALPHA_BELOW_ONE);
            assert!(!c.is_opaque());
            let below = c.premultiply();
            assert_eq!(q(below.red()), q(opaque.red()), "{v}");
            assert_eq!(q(below.alpha()), 255);
        }
        for rgba in [0x2F5B_D6FFu32, 0xFFFF_FFFF, 0x0000_00FF, 0x6688_AAFF] {
            assert_eq!(uniform_u16(rgba), {
                let c = paint_over(rgba);
                let tiny_skia::Shader::SolidColor(c) = c.shader else {
                    unreachable!()
                };
                let p = c.premultiply();
                [q(p.red()), q(p.green()), q(p.blue()), q(p.alpha())]
            });
        }
    }

    /// `round_even_u8` is `round_ties_even` on 0..=255: every value with a
    /// fraction of a half, near a half and arbitrary.
    #[test]
    fn round_even_u8_is_round_ties_even() {
        let mut v = 0.0f32;
        while v <= 255.0 {
            for x in [
                v,
                v.next_up(),
                v.next_down().max(0.0),
                v + 0.5,
                (v + 0.5).next_up(),
            ] {
                if x <= 255.0 {
                    assert_eq!(round_even_u8(x), x.round_ties_even() as u8, "{x}");
                }
            }
            v += 0.125;
        }
        let mut g = Gen(7);
        for _ in 0..1_000_000 {
            let x = (g.next() >> 40) as f32 / (1u64 << 24) as f32 * 255.0;
            assert_eq!(round_even_u8(x), x.round_ties_even() as u8, "{x}");
        }
    }

    /// `over_highp` is the arithmetic of `draw_pixmap`, checked over every
    /// source alpha, every premultiplied source channel at or under it and
    /// every destination channel, on an opaque destination and on one whose
    /// alpha varies; and through a clip mask for a spread of coverages. A
    /// debug build checks every seventh destination value (and 254, 255),
    /// so the suite stays quick; a release build (the CI bench job) checks
    /// them all.
    #[test]
    fn over_highp_is_what_draw_pixmap_does() {
        let _serial = crate::tests::serial();
        use tiny_skia::PixmapPaint;
        let dsts: Vec<u32> = if cfg!(debug_assertions) {
            (0..256).step_by(7).chain([254, 255]).collect()
        } else {
            (0..256).collect()
        };
        let check = |sa: u32, dst_px: &dyn Fn(u32) -> [u8; 4], m: Option<u8>| {
            let (w, h) = (dsts.len() as u32, sa + 1);
            let mut src = Pixmap::new(w, h).unwrap();
            let mut dst = Pixmap::new(w, h).unwrap();
            for sc in 0..=sa {
                for (x, &d) in dsts.iter().enumerate() {
                    let i = ((sc * w + x as u32) * 4) as usize;
                    src.data_mut()[i..i + 4].copy_from_slice(&[
                        sc as u8,
                        (sa - sc) as u8,
                        sc as u8 / 2,
                        sa as u8,
                    ]);
                    dst.data_mut()[i..i + 4].copy_from_slice(&dst_px(d));
                }
            }
            let mut mine = dst.clone();
            let mask = m.map(|m| {
                let mut mask = Mask::new(w, h).unwrap();
                mask.data_mut().fill(m);
                mask
            });
            dst.draw_pixmap(
                0,
                0,
                src.as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                mask.as_ref(),
            );
            let s = src.data();
            for (i, d) in mine.data_mut().chunks_exact_mut(4).enumerate() {
                let p = [s[i * 4], s[i * 4 + 1], s[i * 4 + 2], s[i * 4 + 3]];
                over_highp(d, p, m.unwrap_or(255));
            }
            assert_eq!(mine.data(), dst.data(), "sa {sa}, mask {m:?}");
        };
        let opaque = |d: u32| [d as u8, (255 - d) as u8, (d / 3) as u8, 255];
        let varying = |d: u32| [d as u8, (d / 2) as u8, 0, d as u8];
        for sa in 0..=255 {
            check(sa, &opaque, None);
            check(sa, &varying, None);
        }
        for m in [0u8, 1, 2, 17, 64, 127, 128, 200, 254, 255] {
            for sa in (0..=255).step_by(15).chain([254, 255]) {
                check(sa, &opaque, Some(m));
            }
        }
    }

    /// A deterministic generator for the differential tests.
    struct Gen(u64);

    impl Gen {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        /// A coordinate: whole, half, quarter or any value, sometimes off the canvas.
        fn coord(&mut self, max: f32) -> f32 {
            let whole = self.below((max as u64) + 20) as f32 - 10.0;
            match self.below(5) {
                0 | 1 => whole,
                2 => whole + 0.5,
                3 => whole + 0.25,
                _ => whole + (self.below(1000) as f32) / 1000.0,
            }
        }
        fn length(&mut self, max: f32) -> f32 {
            match self.below(4) {
                0 => (1 + self.below(max as u64)) as f32,
                1 => (1 + self.below(12)) as f32 + 0.5,
                _ => (self.below((max * 1000.0) as u64) as f32) / 1000.0 + 0.01,
            }
        }
        fn color(&mut self) -> u32 {
            let rgb = (self.next() as u32) & 0xFFFF_FF00;
            match self.below(10) {
                0..=4 => rgb | 0xFF,
                5 => rgb,
                _ => rgb | (1 + self.below(254)) as u32,
            }
        }
    }

    /// A small PNG with every kind of alpha.
    fn test_picture() -> Vec<u8> {
        let mut img = RgbaImage::new(9, 7);
        for (i, p) in img.data.chunks_exact_mut(4).enumerate() {
            let a = [255u8, 0, 128, 1, 254, 77][i % 6];
            p.copy_from_slice(&[(i * 29) as u8, (i * 53) as u8, (i * 91) as u8, a]);
        }
        img.encode_png().unwrap()
    }

    /// A random list over a `w` × `h` logical window: rectangles (whole
    /// pixels, halves and anything, plain and rounded, opaque, translucent
    /// and invisible), nested plain and rounded clips with their pops,
    /// text in three styles, images in three fits, filled and stroked paths.
    fn random_list(g: &mut Gen, styles: &[u16], w: f32, h: f32) -> Vec<u8> {
        const TEXTS: [&str; 4] = [
            "Vy ritar text",
            "Lägesrapport: allt grönt",
            "0x2F5B rect 12 x 7",
            "ä",
        ];
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        let mut depth = 0;
        for _ in 0..(5 + g.below(50)) {
            match g.below(20) {
                0..=6 => {
                    let radius = if g.below(5) < 3 { 0.0 } else { g.length(14.0) };
                    let (x, y) = (g.coord(w), g.coord(h));
                    let (rw, rh) = (g.length(w), g.length(h));
                    dl.rect(x, y, rw, rh, radius, g.color()).unwrap();
                }
                7..=9 if depth < 8 => {
                    let radius = if g.below(5) < 3 { 0.0 } else { g.length(16.0) };
                    let (x, y) = (g.coord(w), g.coord(h));
                    let (cw, ch) = (g.length(w), g.length(h));
                    dl.clip_push(x, y, cw, ch, radius).unwrap();
                    depth += 1;
                }
                10..=11 if depth > 0 => {
                    dl.clip_pop().unwrap();
                    depth -= 1;
                }
                12..=14 => {
                    let style = styles[g.below(styles.len() as u64) as usize];
                    let max_w = [0.0, 30.0, 80.0][g.below(3) as usize];
                    let text = TEXTS[g.below(4) as usize];
                    dl.text(g.coord(w), g.coord(h), max_w, style, text.as_bytes())
                        .unwrap();
                }
                15..=16 => {
                    let fit = [Fit::Cover, Fit::Contain, Fit::Stretch][g.below(3) as usize];
                    let (x, y) = (g.coord(w), g.coord(h));
                    dl.image(x, y, g.length(w), g.length(h), 1, fit).unwrap();
                }
                17 => {
                    let stroke = if g.below(2) == 0 { 0.0 } else { 1.5 };
                    let (x, y) = (g.coord(w), g.coord(h));
                    let cmds = [
                        PathCmd::MoveTo(x, y),
                        PathCmd::LineTo(x + g.length(30.0), y + g.length(5.0)),
                        PathCmd::QuadTo {
                            ctrl: (x + 20.0, y + 30.0),
                            to: (x + g.length(10.0), y + g.length(25.0)),
                        },
                        PathCmd::Close,
                    ];
                    dl.path(g.color(), stroke, &cmds).unwrap();
                }
                _ => {
                    // Whole-pixel rectangles at scale 1 and 2 hit the direct fill.
                    let (x, y) = (g.below(w as u64 + 4) as f32 - 2.0, g.below(h as u64) as f32);
                    let (rw, rh) = ((1 + g.below(40)) as f32, (1 + g.below(20)) as f32);
                    dl.rect(x, y, rw, rh, 0.0, g.color()).unwrap();
                }
            }
        }
        for _ in 0..depth {
            dl.clip_pop().unwrap();
        }
        dl.end().unwrap();
        dl.bytes().to_vec()
    }

    /// Render with the new rasterizer and the M3 one; same pixels, same report.
    fn same_as_m3(bytes: &[u8], w: u32, h: u32, scale: f32, images: &ImageCache) {
        let style = |t: &mut TextEngine| {
            [
                t.style(vy_abi::font::SANS, 11.0, 400, 0x2221_1FFF).unwrap(),
                t.style(vy_abi::font::SERIF, 14.0, 700, 0x3366_9980)
                    .unwrap(),
                t.style(vy_abi::font::MONO, 9.0, 400, 0xD64B_2FFF).unwrap(),
            ]
        };
        let (mut a, mut b) = (TextEngine::new(), TextEngine::new());
        style(&mut a);
        style(&mut b);
        let new = render(bytes, w, h, scale, &mut a, images).unwrap();
        let old = crate::raster_reference::render(bytes, w, h, scale, &mut b, images).unwrap();
        let (n, o) = (new.report, old.report);
        assert_eq!(
            (n.truncated, n.ops_done, n.units, n.budget),
            (o.truncated, o.ops_done, o.units, o.budget),
            "report at {w}x{h}@{scale}"
        );
        if new.image.data != old.image.data {
            let differ = new
                .image
                .data
                .chunks_exact(4)
                .zip(old.image.data.chunks_exact(4))
                .position(|(p, q)| p != q)
                .unwrap();
            panic!(
                "{w}x{h}@{scale}: first differing pixel ({}, {}): {:?} vs M3 {:?}",
                differ % w as usize,
                differ / w as usize,
                &new.image.data[differ * 4..differ * 4 + 4],
                &old.image.data[differ * 4..differ * 4 + 4],
            );
        }
    }

    /// SPEC-2.1.5, SPEC-13.2.1, SPEC-5.3.3: the fast paths change speed and
    /// nothing else. Hundreds of random lists, every op kind under nested
    /// plain and rounded clips at four scales, give the same bytes and the
    /// same budget report as the M3 rasterizer, which drew the goldens.
    #[test]
    fn fast_paths_draw_what_m3_drew() {
        let _serial = crate::tests::serial();
        let mut images = ImageCache::from_bytes(vec![test_picture()]);
        images.decode(1).unwrap();
        let mut g = Gen(0x2545_F491_4F6C_DD1D);
        let mut styles_engine = TextEngine::new();
        let styles = [
            styles_engine
                .style(vy_abi::font::SANS, 11.0, 400, 0)
                .unwrap(),
            styles_engine
                .style(vy_abi::font::SERIF, 14.0, 700, 0)
                .unwrap(),
            styles_engine
                .style(vy_abi::font::MONO, 9.0, 400, 0)
                .unwrap(),
        ];
        for case in 0..400 {
            let (w, h) = [(97u32, 61u32), (64, 64), (128, 40)][case % 3];
            let scale = [1.0f32, 2.0, 1.5, 0.75][(case / 3) % 4];
            let bytes = random_list(&mut g, &styles, w as f32 / scale, h as f32 / scale);
            same_as_m3(&bytes, w, h, scale, &images);
        }
    }

    /// The same for frames that run out of budget, and for the frame of
    /// SPEC-13.2.1 itself.
    #[test]
    fn fast_paths_spend_the_budget_m3_spent() {
        let _serial = crate::tests::serial();
        let images = ImageCache::empty(0);
        // Full-canvas translucent fills under a plain clip, and under a
        // rounded one, until the budget stops the frame.
        for radius in [0.0, 6.0] {
            let mut dl = DisplayList::new();
            dl.begin().unwrap();
            dl.clip_push(3.5, 2.0, 1200.0, 790.0, radius).unwrap();
            for i in 0..80u32 {
                dl.rect(0.0, 0.0, 1280.0, 800.0, 0.0, 0x1020_3080 + i)
                    .unwrap();
            }
            dl.clip_pop().unwrap();
            dl.end().unwrap();
            same_as_m3(dl.bytes(), 1280, 800, 1.0, &images);
        }
        let mut t = TextEngine::new();
        let frame = crate::bench::frame_2000(&mut t, 640.0, 400.0);
        let style = |t: &mut TextEngine| crate::bench::frame_2000(t, 640.0, 400.0);
        let (mut a, mut b) = (TextEngine::new(), TextEngine::new());
        let fa = style(&mut a);
        style(&mut b);
        assert_eq!(fa.list, frame.list);
        let new = render(&frame.list, 640, 400, 1.0, &mut a, &fa.images).unwrap();
        let old = crate::raster_reference::render(&frame.list, 640, 400, 1.0, &mut b, &fa.images)
            .unwrap();
        assert_eq!(new.report.units, old.report.units);
        assert!(new.image.data == old.image.data, "frame_2000 differs");
    }
}
