//! The window's rasterizer: the reference's bytes in less time
//! (SPEC-13.2.1, SPEC-11.3.2, SPEC-2.1.5).
//!
//! A [`Renderer`] lives as long as the page it draws and keeps what it
//! drew. On the fast path ([`RasterPath::Fast`]) a frame is four steps:
//!
//! 1. **Text runs resolved once.** Every `TEXT` op is turned into
//!    positioned glyphs and an index into a table of glyph bitmaps, kept
//!    across frames under everything the result depends on (face, size,
//!    wrap width, origin, scale and bytes, all compared exactly), so a run
//!    drawn again costs a lookup instead of a layout and a glyph lookup per
//!    glyph (2 ms of the 2 000-op frame).
//! 2. **The dry pass.** `raster::dry_pass` runs the reference's own code
//!    with drawing switched off: the same budget charges in the same order
//!    (SPEC-5.3.3), so the same report and truncation point, and every op's
//!    footprint – the pixels it may change.
//! 3. **Damage.** The list is compared with the last one, op by op and
//!    exactly (floats by their bits, each op's enclosing clips included):
//!    the ops before the first and after the last difference are the same
//!    ops in the same order, so outside the footprints of the ops between
//!    them every pixel is what it was, and only that region is drawn.
//! 4. **Bands.** What is drawn is split into horizontal bands, drawn on
//!    threads (`std::thread::scope`).
//!
//! Why this gives the reference's bytes: tiny-skia's anti-aliased fill is
//! not translation invariant across a clip edge (an edge that starts
//! outside is cut in floating point and stepped in fixed point after), so
//! a band or a damage region is never drawn as a smaller pixmap. Each is
//! drawn on a pixmap of the whole canvas's size: every tiny-skia call is
//! the reference's call, with the same pixmap size, path, paint and mask,
//! and what vy-render fills itself (whole pixels, glyphs, images) is cut
//! to the region. Ops whose footprint misses the region are passed over;
//! every pixel's value depends only on the ops that reach it, in order.
//! The tests compare the two byte for byte and report for report.

use crate::image::{ImageCache, RgbaImage};
use crate::raster::{self, Pixels, PlacedGlyph, RasterReport, RenderError, ResolvedRun, TextOp};
use crate::text::{Style, TextEngine};
use cosmic_text::{CacheKey, SwashImage};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use tiny_skia::{IntSize, Pixmap};
use vy_dl::{Op, PathCmd};

/// Which rasterizer a [`Renderer`] runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RasterPath {
    /// [`crate::render`] for every frame: headless rendering, the
    /// conformance suite and the shell's screenshots.
    Reference,
    /// The same bytes with text runs kept across frames, damage tracking
    /// and bands on threads: the window.
    Fast,
}

/// Most bands a frame is split into.
pub const MAX_BANDS: usize = 4;
/// Fewest rows a band gets: below this a thread costs more than it saves.
const MIN_BAND_ROWS: i32 = 48;
/// A damage region over this share of the canvas is drawn as a whole
/// frame, in bands.
const DAMAGE_MAX_SHARE: f64 = 0.5;
/// Bytes of whole-canvas scratch pixmaps the bands of one frame may use.
const SCRATCH_BYTES: usize = 64 << 20;
/// Glyph bitmaps the run cache keeps; past this it starts over.
pub const RUN_GLYPH_IMAGES: usize = 16_384;
/// Placed glyphs the run cache keeps; past this it starts over.
pub const RUN_GLYPHS: usize = 1 << 18;

/// What the last [`Renderer::render`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameInfo {
    /// The pixels drawn, `[x, y, width, height]`: the whole canvas, the
    /// damage since the last frame, or `None` when the list draws what
    /// the last one drew.
    pub redrawn: Option<[u32; 4]>,
    /// True when the whole canvas was drawn.
    pub full: bool,
    /// Bands the drawing was split into (1: no threads; 0 on the GPU).
    pub bands: usize,
    /// True when the frame was drawn by the GL path (`gl` feature).
    pub gpu: bool,
}

/// A rasterizer that keeps its last frame (see the module docs).
pub struct Renderer {
    path: RasterPath,
    bands: usize,
    /// Tests: split into exactly this many bands, whatever the size.
    forced_bands: Option<usize>,
    image: RgbaImage,
    report: RasterReport,
    info: FrameInfo,
    last: Option<Record>,
    pub(crate) runs: RunCache,
    /// Why the fast path was given up for the reference, if it was.
    fell_back: Option<String>,
}

/// A frame resolved for drawing (steps 1 and 2 of the module docs): what
/// the bands and the GL path draw from.
pub(crate) struct Resolved {
    pub(crate) ops: Vec<Op>,
    pub(crate) styles: Vec<Option<StyleKey>>,
    pub(crate) text_ops: Vec<Option<TextOp>>,
    pub(crate) report: RasterReport,
    pub(crate) footprints: Vec<Option<Pixels>>,
    pub(crate) size: IntSize,
    pub(crate) scale: f32,
}

/// What the damage tracking compares the next list with.
struct Record {
    width: u32,
    height: u32,
    scale: u32,
    images: u64,
    ops: Vec<Op>,
    styles: Vec<Option<StyleKey>>,
    parents: Vec<u32>,
    footprints: Vec<Option<Pixels>>,
}

/// Everything about a text style that its pixels depend on.
pub(crate) type StyleKey = (i32, u32, u16, u16, u32);

fn style_key(s: &Style) -> StyleKey {
    (
        s.font,
        s.size_px.to_bits(),
        s.weight,
        s.face_weight,
        s.color,
    )
}

/// No enclosing clip.
const NO_PARENT: u32 = u32::MAX;

impl Renderer {
    /// A renderer on `path`; the fast path uses up to [`MAX_BANDS`]
    /// threads, as many as the machine has.
    pub fn new(path: RasterPath) -> Self {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        Self {
            path,
            bands: threads.clamp(1, MAX_BANDS),
            forced_bands: None,
            image: RgbaImage {
                width: 0,
                height: 0,
                data: Vec::new(),
            },
            report: RasterReport::default(),
            info: FrameInfo::default(),
            last: None,
            runs: RunCache::default(),
            fell_back: None,
        }
    }

    /// The fast path with at most `bands` bands (1: no threads).
    pub fn with_bands(bands: usize) -> Self {
        let mut r = Self::new(RasterPath::Fast);
        r.bands = bands.clamp(1, MAX_BANDS);
        r
    }

    /// The path this renderer runs.
    pub fn path(&self) -> RasterPath {
        self.path
    }

    /// Why the fast path was given up for the reference, when it was:
    /// a panic while drawing a frame. The renderer then stays on the
    /// reference (the last link of the window's fallback chain).
    pub fn fell_back(&self) -> Option<&str> {
        self.fell_back.as_deref()
    }

    /// One line for the log: which path, how many bands.
    pub fn describe(&self) -> String {
        match self.path {
            RasterPath::Reference => match &self.fell_back {
                Some(why) => format!(
                    "raster: software reference (vy_render::render), the fast path having failed: {why}"
                ),
                None => "raster: software reference (vy_render::render)".into(),
            },
            RasterPath::Fast => format!(
                "raster: fast software path, byte for byte the reference: damage tracking, up to {} band{} on threads",
                self.bands,
                if self.bands == 1 { "" } else { "s" }
            ),
        }
    }

    /// The last frame's pixels (empty before the first).
    pub fn image(&self) -> &RgbaImage {
        &self.image
    }

    /// The last frame's report (SPEC-5.3.3).
    pub fn report(&self) -> RasterReport {
        self.report
    }

    /// What the last frame drew.
    pub fn info(&self) -> FrameInfo {
        self.info
    }

    /// Forget the last frame: the next one is drawn whole.
    pub fn invalidate(&mut self) {
        self.last = None;
    }

    /// Rasterize `display_list` as [`crate::render`] does, with the same
    /// arguments, pixels and report; the pixels are [`Renderer::image`].
    pub fn render(
        &mut self,
        display_list: &[u8],
        width: u32,
        height: u32,
        scale: f32,
        text: &mut TextEngine,
        images: &ImageCache,
    ) -> Result<RasterReport, RenderError> {
        match self.path {
            RasterPath::Reference => {
                let r = raster::render(display_list, width, height, scale, text, images)?;
                self.image = r.image;
                self.report = r.report;
                self.info = FrameInfo {
                    redrawn: Some([0, 0, width, height]),
                    full: true,
                    bands: 1,
                    gpu: false,
                };
                Ok(r.report)
            }
            RasterPath::Fast => {
                let fast = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.render_fast(display_list, width, height, scale, text, images, true)
                }));
                match fast {
                    Ok(r) => r,
                    Err(e) => {
                        // The fallback chain's last link: a bug in the fast
                        // path costs speed, never the page.
                        let why = e
                            .downcast_ref::<&str>()
                            .map(|s| (*s).to_owned())
                            .or_else(|| e.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "panic".into());
                        self.give_up(why);
                        self.render(display_list, width, height, scale, text, images)
                    }
                }
            }
        }
    }

    /// Leave the fast path for the reference, for good.
    pub(crate) fn give_up(&mut self, why: String) {
        self.path = RasterPath::Reference;
        self.fell_back = Some(why);
        self.last = None;
        self.runs = RunCache::default();
    }

    /// [`Renderer::render`] on the fast path without damage tracking: the
    /// whole canvas is drawn, whatever the last frame was. What
    /// SPEC-13.2.1 measures.
    pub fn render_full(
        &mut self,
        display_list: &[u8],
        width: u32,
        height: u32,
        scale: f32,
        text: &mut TextEngine,
        images: &ImageCache,
    ) -> Result<RasterReport, RenderError> {
        self.render_fast(display_list, width, height, scale, text, images, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_fast(
        &mut self,
        display_list: &[u8],
        width: u32,
        height: u32,
        scale: f32,
        text: &mut TextEngine,
        images: &ImageCache,
        damage: bool,
    ) -> Result<RasterReport, RenderError> {
        let Resolved {
            ops,
            styles,
            text_ops,
            report,
            footprints,
            size,
            scale,
        } = self.resolve(display_list, width, height, scale, text, images)?;
        let parents = parents(&ops);
        let canvas = raster::whole(width, height);
        let same_canvas = self.last.as_ref().is_some_and(|l| {
            (l.width, l.height, l.scale, l.images)
                == (width, height, scale.to_bits(), images.generation())
        }) && (self.image.width, self.image.height) == (width, height);
        let region = match (&self.last, damage && same_canvas && !report.truncated) {
            (Some(last), true) => damage_region(last, &ops, &styles, &parents, &footprints),
            _ => Some(canvas),
        };
        let full = region.is_some_and(|r| {
            r == canvas || r.area() as f64 > DAMAGE_MAX_SHARE * canvas.area() as f64
        });
        let region = if full { Some(canvas) } else { region };
        let bands = match region {
            Some(r) => {
                let job = Job {
                    ops: &ops,
                    stop: report.ops_done,
                    footprints: &footprints,
                    scale,
                    text_ops: &text_ops,
                    glyph_images: &self.runs.images,
                    images,
                    size,
                };
                let bands = self.band_count(&r, size, full);
                if (self.image.width, self.image.height) != (width, height) {
                    self.image = RgbaImage {
                        width,
                        height,
                        data: vec![255; width as usize * height as usize * 4],
                    };
                }
                draw(&job, &mut self.image, r, full, bands)
            }
            None => 0,
        };
        self.report = report;
        self.info = FrameInfo {
            redrawn: region.map(|r| {
                [
                    r.x0 as u32,
                    r.y0 as u32,
                    (r.x1 - r.x0) as u32,
                    (r.y1 - r.y0) as u32,
                ]
            }),
            full,
            bands,
            gpu: false,
        };
        self.last = (!report.truncated).then(|| Record {
            width,
            height,
            scale: scale.to_bits(),
            images: images.generation(),
            ops,
            styles,
            parents,
            footprints,
        });
        Ok(report)
    }

    /// Rasterize `display_list` on the GPU into `target`'s page
    /// framebuffer: the same arguments and report as [`Renderer::render`],
    /// the picture within SPEC-11.3.2's tolerance. A frame the pipeline
    /// does not draw is drawn by this renderer's CPU path and uploaded
    /// ([`FrameInfo::gpu`] says which); a GL failure is returned, and the
    /// caller leaves the GL path.
    #[cfg(feature = "gl")]
    #[allow(clippy::too_many_arguments)]
    pub fn render_gl(
        &mut self,
        gl: &glow::Context,
        target: &mut crate::gl::GlRaster,
        display_list: &[u8],
        width: u32,
        height: u32,
        scale: f32,
        text: &mut TextEngine,
        images: &ImageCache,
    ) -> Result<RasterReport, crate::gl::GlError> {
        use crate::gl::GlError;
        let f = self
            .resolve(display_list, width, height, scale, text, images)
            .map_err(GlError::Render)?;
        match target.draw_page(gl, &f, &self.runs.images, &self.runs.keys, images) {
            Ok(()) => {
                self.report = f.report;
                self.info = FrameInfo {
                    redrawn: Some([0, 0, width, height]),
                    full: true,
                    bands: 0,
                    gpu: true,
                };
                Ok(f.report)
            }
            Err(GlError::Unsupported(_)) => {
                let report = self
                    .render(display_list, width, height, scale, text, images)
                    .map_err(GlError::Render)?;
                target.upload_page(gl, &self.image)?;
                Ok(report)
            }
            Err(e) => Err(e),
        }
    }

    /// Steps 1 and 2 of the module docs: the list decoded, its text runs
    /// resolved (from the cache or laid out now) and the dry pass run.
    pub(crate) fn resolve(
        &mut self,
        display_list: &[u8],
        width: u32,
        height: u32,
        scale: f32,
        text: &mut TextEngine,
        images: &ImageCache,
    ) -> Result<Resolved, RenderError> {
        let ops = vy_dl::decode(display_list).map_err(RenderError::Decode)?;
        let size = raster::check_size(width, height, scale)?;
        self.runs.trim();
        let mut text_ops = Vec::with_capacity(ops.len());
        let mut styles = Vec::with_capacity(ops.len());
        for op in &ops {
            let Op::Text {
                x,
                y,
                max_w,
                style_id,
                text: bytes,
            } = op
            else {
                styles.push(None);
                text_ops.push(None);
                continue;
            };
            let style = text.get_style(*style_id);
            styles.push(style.as_ref().map(style_key));
            text_ops.push(match style {
                Some(st) if raster::sane(&[*x, *y]) => Some(
                    self.runs
                        .resolve(text, *style_id, &st, *x, *y, *max_w, scale, bytes),
                ),
                _ => None,
            });
        }
        let (report, footprints) = raster::dry_pass(
            &ops,
            width,
            height,
            scale,
            &text_ops,
            &self.runs.images,
            images,
        )?;
        Ok(Resolved {
            ops,
            styles,
            text_ops,
            report,
            footprints,
            size,
            scale,
        })
    }

    /// Bands for drawing `region`: as many as the machine has (at most
    /// [`MAX_BANDS`]), no thinner than [`MIN_BAND_ROWS`], and no more
    /// whole-canvas scratch pixmaps than [`SCRATCH_BYTES`] allows.
    fn band_count(&self, region: &Pixels, size: IntSize, full: bool) -> usize {
        let rows = (region.y1 - region.y0).max(1) as usize;
        if let Some(k) = self.forced_bands {
            return k.clamp(1, rows);
        }
        let canvas_bytes = size.width() as usize * size.height() as usize * 4;
        // A whole frame draws its first band on the output itself.
        let scratch = (SCRATCH_BYTES / canvas_bytes.max(1)).max(1) + usize::from(full);
        let by_rows = (rows / MIN_BAND_ROWS as usize).max(1);
        self.bands.min(by_rows).min(scratch).max(1)
    }
}

/// What every band of a frame shares.
struct Job<'a> {
    ops: &'a [Op],
    stop: usize,
    footprints: &'a [Option<Pixels>],
    scale: f32,
    text_ops: &'a [Option<TextOp>],
    glyph_images: &'a [SwashImage],
    images: &'a ImageCache,
    size: IntSize,
}

impl Job<'_> {
    fn draw(&self, region: Pixels, data: Vec<u8>) -> Vec<u8> {
        let pixmap = Pixmap::from_vec(data, self.size).expect("a buffer of the canvas's size");
        raster::draw_region(
            self.ops,
            self.stop,
            self.footprints,
            region,
            pixmap,
            self.scale,
            self.text_ops,
            self.glyph_images,
            self.images,
        )
        .take()
    }
}

/// Draw `region` of `out` in `bands` bands; returns the bands used. A
/// whole frame draws its first band on `out` itself and the others on
/// scratch canvases; a damage region draws every band on a scratch
/// canvas, since tiny-skia also writes outside the region, where `out`
/// holds the last frame.
fn draw(job: &Job<'_>, out: &mut RgbaImage, region: Pixels, full: bool, bands: usize) -> usize {
    let rows = region.y1 - region.y0;
    let k = bands.clamp(1, rows.max(1) as usize);
    let split: Vec<Pixels> = (0..k)
        .map(|i| Pixels {
            y0: region.y0 + (rows as i64 * i as i64 / k as i64) as i32,
            y1: region.y0 + (rows as i64 * (i as i64 + 1) / k as i64) as i32,
            ..region
        })
        .filter(|b| b.y1 > b.y0)
        .collect();
    let len = out.data.len();
    let (first, rest) = if full {
        (Some(split[0]), &split[1..])
    } else {
        (None, &split[..])
    };
    let mut own = first.map(|_| std::mem::take(&mut out.data));
    let drawn: Vec<(Pixels, Vec<u8>)> = std::thread::scope(|s| {
        let mut handles = Vec::new();
        let mut inline = Vec::new();
        for band in rest.iter().copied() {
            let buf = pool_take(len);
            match std::thread::Builder::new()
                .name("vy-raster".into())
                .spawn_scoped(s, move || (band, job.draw(band, buf)))
            {
                Ok(h) => handles.push(h),
                // No thread: the band is drawn here, after the others.
                Err(_) => inline.push(band),
            }
        }
        if let (Some(band), Some(data)) = (first, own.take()) {
            own = Some(job.draw(band, data));
        }
        let mut done: Vec<(Pixels, Vec<u8>)> = handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect();
        for band in inline {
            done.push((band, job.draw(band, pool_take(len))));
        }
        done
    });
    if let Some(data) = own {
        out.data = data;
    }
    let w = out.width as usize * 4;
    for (band, buf) in drawn {
        let (x0, x1) = (band.x0 as usize * 4, band.x1 as usize * 4);
        for y in band.y0..band.y1 {
            let at = y as usize * w;
            out.data[at + x0..at + x1].copy_from_slice(&buf[at + x0..at + x1]);
        }
        pool_give(buf);
    }
    split.len()
}

/// Whole-canvas scratch buffers shared by every renderer of the process
/// (one page is drawn at a time; a pool per tab would cost a canvas per
/// tab).
static POOL: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

fn pool_take(len: usize) -> Vec<u8> {
    let mut v = POOL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop()
        .unwrap_or_default();
    v.resize(len, 0);
    v
}

fn pool_give(v: Vec<u8>) {
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    if pool.len() < MAX_BANDS && v.len() <= SCRATCH_BYTES {
        pool.push(v);
    }
}

/// For each op, the index of the innermost `CLIP_PUSH` around it (a pop:
/// the push it closes), or [`NO_PARENT`].
fn parents(ops: &[Op]) -> Vec<u32> {
    let mut stack: Vec<u32> = Vec::new();
    ops.iter()
        .enumerate()
        .map(|(i, op)| {
            let parent = stack.last().copied().unwrap_or(NO_PARENT);
            match op {
                Op::ClipPush { .. } => stack.push(i as u32),
                Op::ClipPop => {
                    stack.pop();
                }
                _ => {}
            }
            parent
        })
        .collect()
}

/// The pixels that may differ between the last frame and this one, or
/// `None` when the lists are the same ops in the same clips.
fn damage_region(
    last: &Record,
    ops: &[Op],
    styles: &[Option<StyleKey>],
    parents: &[u32],
    footprints: &[Option<Pixels>],
) -> Option<Pixels> {
    let same = |mut i: usize, mut j: usize| loop {
        if !op_eq(&last.ops[i], &ops[j]) || last.styles[i] != styles[j] {
            return false;
        }
        match (last.parents[i], parents[j]) {
            (NO_PARENT, NO_PARENT) => return true,
            (NO_PARENT, _) | (_, NO_PARENT) => return false,
            (a, b) => (i, j) = (a as usize, b as usize),
        }
    };
    let (n, m) = (last.ops.len(), ops.len());
    let mut p = 0;
    while p < n && p < m && same(p, p) {
        p += 1;
    }
    let mut q = 0;
    while q < n - p && q < m - p && same(n - 1 - q, m - 1 - q) {
        q += 1;
    }
    last.footprints[p..n - q]
        .iter()
        .chain(&footprints[p..m - q])
        .flatten()
        .copied()
        .reduce(|a, b| union(&a, &b))
}

fn union(a: &Pixels, b: &Pixels) -> Pixels {
    Pixels {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

/// Two ops are the same op: every field equal, floats by their bits.
fn op_eq(a: &Op, b: &Op) -> bool {
    let f = |a: &f32, b: &f32| a.to_bits() == b.to_bits();
    match (a, b) {
        (
            Op::Rect {
                x,
                y,
                w,
                h,
                radius,
                color,
            },
            Op::Rect {
                x: x2,
                y: y2,
                w: w2,
                h: h2,
                radius: r2,
                color: c2,
            },
        ) => f(x, x2) && f(y, y2) && f(w, w2) && f(h, h2) && f(radius, r2) && color == c2,
        (
            Op::Text {
                x,
                y,
                max_w,
                style_id,
                text,
            },
            Op::Text {
                x: x2,
                y: y2,
                max_w: m2,
                style_id: s2,
                text: t2,
            },
        ) => f(x, x2) && f(y, y2) && f(max_w, m2) && style_id == s2 && text == t2,
        (
            Op::Image {
                x,
                y,
                w,
                h,
                resource_id,
                fit,
            },
            Op::Image {
                x: x2,
                y: y2,
                w: w2,
                h: h2,
                resource_id: r2,
                fit: f2,
            },
        ) => f(x, x2) && f(y, y2) && f(w, w2) && f(h, h2) && resource_id == r2 && fit == f2,
        (
            Op::Path {
                color,
                stroke_w,
                cmds,
            },
            Op::Path {
                color: c2,
                stroke_w: s2,
                cmds: k2,
            },
        ) => {
            color == c2
                && f(stroke_w, s2)
                && cmds.len() == k2.len()
                && cmds.iter().zip(k2).all(|(a, b)| match (a, b) {
                    (PathCmd::MoveTo(x, y), PathCmd::MoveTo(x2, y2))
                    | (PathCmd::LineTo(x, y), PathCmd::LineTo(x2, y2)) => f(x, x2) && f(y, y2),
                    (PathCmd::QuadTo { ctrl, to }, PathCmd::QuadTo { ctrl: c2, to: t2 }) => {
                        f(&ctrl.0, &c2.0) && f(&ctrl.1, &c2.1) && f(&to.0, &t2.0) && f(&to.1, &t2.1)
                    }
                    (PathCmd::Close, PathCmd::Close) => true,
                    _ => false,
                })
        }
        (
            Op::ClipPush { x, y, w, h, radius },
            Op::ClipPush {
                x: x2,
                y: y2,
                w: w2,
                h: h2,
                radius: r2,
            },
        ) => f(x, x2) && f(y, y2) && f(w, w2) && f(h, h2) && f(radius, r2),
        (Op::ClipPop, Op::ClipPop) => true,
        _ => false,
    }
}

/// Text runs resolved to positioned glyphs, kept across frames, and the
/// glyph bitmaps they index (step 1 of the module docs).
#[derive(Default)]
pub(crate) struct RunCache {
    runs: HashMap<u64, Vec<(RunKey, Arc<ResolvedRun>)>>,
    index: HashMap<CacheKey, u32>,
    pub(crate) images: Vec<SwashImage>,
    /// The cache key of each bitmap in `images`: what the GL path's
    /// glyph atlas is keyed on.
    pub(crate) keys: Vec<CacheKey>,
    glyphs: usize,
}

/// Everything a resolved run depends on, compared exactly.
struct RunKey {
    font: i32,
    face_weight: u16,
    size: u32,
    max_w: u32,
    x: u32,
    y: u32,
    scale: u32,
    text: Box<[u8]>,
}

impl RunCache {
    /// Start over when the cache holds more than its caps; only ever
    /// between frames, so a frame's runs and bitmaps stay put while its
    /// bands draw them.
    fn trim(&mut self) {
        if self.images.len() > RUN_GLYPH_IMAGES || self.glyphs > RUN_GLYPHS {
            *self = Self::default();
        }
    }

    /// The run of a `TEXT` op whose style is `style`, from the cache or
    /// laid out and rasterized now, exactly as the reference's
    /// [`TextEngine::rasterize`] hands it out.
    #[allow(clippy::too_many_arguments)]
    fn resolve(
        &mut self,
        engine: &mut TextEngine,
        style_id: u16,
        style: &Style,
        x: f32,
        y: f32,
        max_w: f32,
        scale: f32,
        bytes: &[u8],
    ) -> TextOp {
        let key = |text: Box<[u8]>| RunKey {
            font: style.font,
            face_weight: style.face_weight,
            size: style.size_px.to_bits(),
            max_w: max_w.to_bits(),
            x: x.to_bits(),
            y: y.to_bits(),
            scale: scale.to_bits(),
            text,
        };
        let hash = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (
                style.font,
                style.face_weight,
                style.size_px.to_bits(),
                max_w.to_bits(),
                x.to_bits(),
                y.to_bits(),
                scale.to_bits(),
            )
                .hash(&mut h);
            bytes.hash(&mut h);
            h.finish()
        };
        let probe = |k: &RunKey| {
            (
                k.font,
                k.face_weight,
                k.size,
                k.max_w,
                k.x,
                k.y,
                k.scale,
                &*k.text,
            ) == (
                style.font,
                style.face_weight,
                style.size_px.to_bits(),
                max_w.to_bits(),
                x.to_bits(),
                y.to_bits(),
                scale.to_bits(),
                bytes,
            )
        };
        if let Some((_, run)) = self
            .runs
            .get(&hash)
            .and_then(|b| b.iter().find(|(k, _)| probe(k)))
        {
            return TextOp {
                run: Arc::clone(run),
                color: style.color,
            };
        }
        let s = String::from_utf8_lossy(bytes);
        let (index, images, keys) = (&mut self.index, &mut self.images, &mut self.keys);
        let mut glyphs = Vec::new();
        let mut bounds: Option<Pixels> = None;
        let mut px = 0u64;
        let _ = engine.rasterize(style_id, (x, y), max_w, scale, &s, |g| {
            let id = *index.entry(g.key).or_insert_with(|| {
                images.push(g.image.clone());
                keys.push(g.key);
                (images.len() - 1) as u32
            });
            let (w, h) = (g.image.placement.width, g.image.placement.height);
            px += u64::from(w) * u64::from(h);
            let b = Pixels {
                x0: g.x,
                y0: g.y,
                x1: g.x.saturating_add(w as i32),
                y1: g.y.saturating_add(h as i32),
            };
            bounds = Some(bounds.map_or(b, |u| union(&u, &b)));
            glyphs.push(PlacedGlyph {
                x: g.x,
                y: g.y,
                image: id,
            });
        });
        let run = Arc::new(ResolvedRun {
            glyphs,
            bounds,
            px,
            #[cfg(feature = "gl")]
            atlas: std::sync::Mutex::new((0, Arc::from(Vec::new()))),
        });
        self.glyphs += run.glyphs.len();
        self.runs
            .entry(hash)
            .or_default()
            .push((key(bytes.into()), Arc::clone(&run)));
        TextOp {
            run,
            color: style.color,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raster::tests::{random_list, test_picture, Gen};
    use crate::Rendered;
    use vy_dl::{DisplayList, Fit};

    /// The three styles of the differential tests, registered in `t`.
    fn styles(t: &mut TextEngine) -> [u16; 3] {
        [
            t.style(vy_abi::font::SANS, 11.0, 400, 0x2221_1FFF).unwrap(),
            t.style(vy_abi::font::SERIF, 14.0, 700, 0x3366_9980)
                .unwrap(),
            t.style(vy_abi::font::MONO, 9.0, 400, 0xD64B_2FFF).unwrap(),
        ]
    }

    /// The same three style ids as [`styles`] with every property another:
    /// what a page that registers its styles anew may make of them.
    fn other_styles(t: &mut TextEngine) -> [u16; 3] {
        [
            t.style(vy_abi::font::SERIF, 13.5, 700, 0x1040_90FF)
                .unwrap(),
            t.style(vy_abi::font::MONO, 10.0, 400, 0x9933_66C0).unwrap(),
            t.style(vy_abi::font::SANS, 12.0, 700, 0x2221_1FFF).unwrap(),
        ]
    }

    /// The reference's frame, drawn with a fresh engine.
    fn reference(bytes: &[u8], w: u32, h: u32, scale: f32, images: &ImageCache) -> Rendered {
        let mut t = TextEngine::new();
        styles(&mut t);
        raster::render(bytes, w, h, scale, &mut t, images).unwrap()
    }

    /// [`reference`] with the styles of [`other_styles`].
    fn reference_restyled(
        bytes: &[u8],
        w: u32,
        h: u32,
        scale: f32,
        images: &ImageCache,
    ) -> Rendered {
        let mut t = TextEngine::new();
        other_styles(&mut t);
        raster::render(bytes, w, h, scale, &mut t, images).unwrap()
    }

    /// `got` is `want` byte for byte, report for report.
    fn same(got: (&RgbaImage, RasterReport), want: &Rendered, what: &str) {
        assert_eq!(got.1, want.report, "{what}: report");
        let (a, b) = (&got.0.data, &want.image.data);
        if a != b {
            let w = want.image.width as usize;
            let i = a
                .chunks_exact(4)
                .zip(b.chunks_exact(4))
                .position(|(p, q)| p != q)
                .unwrap_or(0);
            panic!(
                "{what}: first differing pixel ({}, {}): {:?}, reference {:?}",
                i % w,
                i / w,
                &a[i * 4..i * 4 + 4],
                &b[i * 4..i * 4 + 4]
            );
        }
    }

    /// A list made of `ops`, in order.
    fn encode(ops: &[Op]) -> Vec<u8> {
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        for op in ops {
            match op {
                Op::Rect {
                    x,
                    y,
                    w,
                    h,
                    radius,
                    color,
                } => dl.rect(*x, *y, *w, *h, *radius, *color).unwrap(),
                Op::Text {
                    x,
                    y,
                    max_w,
                    style_id,
                    text,
                } => {
                    dl.text(*x, *y, *max_w, *style_id, text).unwrap();
                }
                Op::Image {
                    x,
                    y,
                    w,
                    h,
                    resource_id,
                    fit,
                } => {
                    let fit = [Fit::Cover, Fit::Contain, Fit::Stretch][usize::from(*fit) % 3];
                    dl.image(*x, *y, *w, *h, *resource_id, fit).unwrap()
                }
                Op::Path {
                    color,
                    stroke_w,
                    cmds,
                } => dl.path(*color, *stroke_w, cmds).unwrap(),
                Op::ClipPush { x, y, w, h, radius } => {
                    dl.clip_push(*x, *y, *w, *h, *radius).unwrap()
                }
                Op::ClipPop => dl.clip_pop().unwrap(),
            }
        }
        dl.end().unwrap();
        dl.bytes().to_vec()
    }

    /// One edit of the kind a page makes between frames: a drawing op
    /// recoloured, moved, resized, removed or added, a clip moved or its
    /// radius changed, a text changed. Pushes and pops stay paired.
    fn edit(g: &mut Gen, ops: &mut Vec<Op>, styles: &[u16; 3], w: f32, h: f32) {
        let n = ops.len();
        let i = g.below(n as u64 + 1) as usize;
        match g.below(8) {
            0 | 1 if i < n => match &mut ops[i] {
                Op::Rect { color, .. } | Op::Path { color, .. } => *color = g.color(),
                Op::Text { text, .. } => text.extend_from_slice(b" 42"),
                Op::Image { x, .. } | Op::ClipPush { x, .. } => *x += 1.5,
                Op::ClipPop => {}
            },
            2 if i < n => match &mut ops[i] {
                Op::Rect { x, y, .. } | Op::Text { x, y, .. } | Op::Image { x, y, .. } => {
                    *x += g.coord(20.0) - 10.0;
                    *y += 0.5;
                }
                Op::ClipPush { radius, w, .. } => {
                    *radius = if *radius > 0.0 { 0.0 } else { 6.0 };
                    *w *= 0.75;
                }
                Op::Path { stroke_w, .. } => *stroke_w = if *stroke_w > 0.0 { 0.0 } else { 1.5 },
                Op::ClipPop => {}
            },
            3 if i < n && !matches!(ops[i], Op::ClipPush { .. } | Op::ClipPop) => {
                ops.remove(i);
            }
            4 => ops.insert(
                i,
                Op::Rect {
                    x: g.coord(w),
                    y: g.coord(h),
                    w: g.length(w / 2.0),
                    h: g.length(h / 2.0),
                    radius: if g.below(2) == 0 { 0.0 } else { 4.0 },
                    color: g.color(),
                },
            ),
            5 => ops.insert(
                i,
                Op::Text {
                    x: g.coord(w),
                    y: g.coord(h),
                    max_w: 40.0,
                    style_id: styles[g.below(3) as usize],
                    text: b"Vy 13".to_vec(),
                },
            ),
            // A new clip around one existing drawing op.
            6 if i < n && !matches!(ops[i], Op::ClipPush { .. } | Op::ClipPop) => {
                ops.insert(i + 1, Op::ClipPop);
                ops.insert(
                    i,
                    Op::ClipPush {
                        x: g.coord(w),
                        y: g.coord(h),
                        w: g.length(w),
                        h: g.length(h),
                        radius: if g.below(2) == 0 { 0.0 } else { 5.0 },
                    },
                );
            }
            _ => {}
        }
    }

    /// SPEC-11.3.2, SPEC-2.1.5, SPEC-13.2.1: the window's path draws what
    /// the reference draws. Hundreds of random lists – every op kind
    /// under nested plain and rounded clips, at four scales – drawn whole
    /// in one to five bands give the reference's bytes and budget report.
    #[test]
    fn spec_11_3_2_bands_draw_what_the_reference_draws() {
        let _serial = crate::tests::serial();
        let mut images = ImageCache::from_bytes(vec![test_picture()]);
        images.decode(1).unwrap();
        let mut g = Gen(0x5DEE_CE66_D1CE_4E5B);
        let mut t = TextEngine::new();
        let st = styles(&mut t);
        let mut renderers: Vec<Renderer> = (1..=5)
            .map(|k| {
                let mut r = Renderer::new(RasterPath::Fast);
                r.forced_bands = Some(k);
                r
            })
            .collect();
        for case in 0..160 {
            let (w, h) = [(97u32, 61u32), (64, 64), (128, 40), (33, 90)][case % 4];
            let scale = [1.0f32, 2.0, 1.5, 0.75][(case / 4) % 4];
            let bytes = random_list(&mut g, &st, w as f32 / scale, h as f32 / scale);
            let want = reference(&bytes, w, h, scale, &images);
            for (k, r) in renderers.iter_mut().enumerate() {
                let report = r.render_full(&bytes, w, h, scale, &mut t, &images).unwrap();
                same(
                    (r.image(), report),
                    &want,
                    &format!("case {case}, {w}x{h}@{scale}, {} bands", k + 1),
                );
                assert!(r.info().full);
            }
        }
    }

    /// SPEC-11.3.2, SPEC-2.1.5: damage tracking draws what the reference
    /// draws. Random lists edited the way pages change between frames –
    /// ops recoloured, moved, added and removed, clips moved, added and
    /// rounded – are drawn in sequence by one renderer, which redraws only
    /// what changed, in one or three bands; every frame is the reference's
    /// frame for that list, and a list drawn twice redraws nothing.
    ///
    /// Then what a window does to a page without changing its list
    /// (granskning 0011:6), on that same renderer: another size, another
    /// scale, the same style ids meaning other styles, each alone, back
    /// again, and all three at once; and an edit after that. Every frame
    /// is still the reference's, a new size or scale draws everything,
    /// and a change of style alone is seen though no op changed.
    #[test]
    fn spec_11_3_2_damage_draws_what_the_reference_draws() {
        let _serial = crate::tests::serial();
        let mut images = ImageCache::from_bytes(vec![test_picture()]);
        images.decode(1).unwrap();
        let mut g = Gen(0x2F5B_D6FF_0BAD_CAFE);
        let mut t = TextEngine::new();
        let st = styles(&mut t);
        let mut restyled_engine = TextEngine::new();
        assert_eq!(
            other_styles(&mut restyled_engine),
            st,
            "the same ids, other styles"
        );
        let mut partial = 0;
        let mut restyles_seen = 0;
        for case in 0..90 {
            let (w, h) = [(160u32, 120u32), (97, 61), (128, 128)][case % 3];
            let scale = [1.0f32, 2.0, 1.5][(case / 3) % 3];
            let (lw, lh) = (w as f32 / scale, h as f32 / scale);
            let mut ops = vy_dl::decode(&random_list(&mut g, &st, lw, lh)).unwrap();
            let mut r = Renderer::new(RasterPath::Fast);
            r.forced_bands = Some(if case % 2 == 0 { 1 } else { 3 });
            for step in 0..7 {
                if step > 0 {
                    for _ in 0..1 + g.below(3) {
                        edit(&mut g, &mut ops, &st, lw, lh);
                    }
                }
                let bytes = encode(&ops);
                let want = reference(&bytes, w, h, scale, &images);
                let report = r.render(&bytes, w, h, scale, &mut t, &images).unwrap();
                let what = format!("case {case} step {step}, {w}x{h}@{scale}");
                same((r.image(), report), &want, &what);
                if step > 0 && !r.info().full {
                    partial += 1;
                }
            }
            // The same list again: nothing to draw, the same pixels.
            let bytes = encode(&ops);
            let before = r.image().data.clone();
            r.render(&bytes, w, h, scale, &mut t, &images).unwrap();
            assert_eq!(r.info().redrawn, None, "case {case}: nothing changed");
            assert_eq!(r.image().data, before);

            // The same list on the same renderer: the window resized, the
            // scale changed, the styles registered anew; then all back,
            // and all at once.
            let has_text = ops.iter().any(|o| matches!(o, Op::Text { .. }));
            let (w2, h2) = (w + 31, h - 13);
            let scale2 = scale * 1.25;
            for (step, (cw, ch, cs, restyled)) in [
                (w2, h2, scale, false),
                (w, h, scale, false),
                (w, h, scale2, false),
                (w, h, scale, false),
                (w, h, scale, true),
                (w, h, scale, false),
                (w2, h2, scale2, true),
            ]
            .into_iter()
            .enumerate()
            {
                let what =
                    format!("case {case}, window step {step}: {cw}x{ch}@{cs}, restyled {restyled}");
                let (want, engine) = if restyled {
                    (
                        reference_restyled(&bytes, cw, ch, cs, &images),
                        &mut restyled_engine,
                    )
                } else {
                    (reference(&bytes, cw, ch, cs, &images), &mut t)
                };
                let report = r.render(&bytes, cw, ch, cs, engine, &images).unwrap();
                same((r.image(), report), &want, &what);
                // Steps 4 and 5 change the styles and nothing else: what
                // is redrawn is the text that shows, if any does.
                if step == 4 || step == 5 {
                    let redrew = r.info().redrawn.is_some();
                    assert!(
                        has_text || !redrew,
                        "{what}: a change of style redraws nothing without text"
                    );
                    restyles_seen += usize::from(redrew);
                } else {
                    assert!(
                        r.info().full,
                        "{what}: a new size or scale draws everything"
                    );
                }
            }
            // And the renderer goes on from there: an edit, at the new
            // size and scale with the other styles.
            let (lw2, lh2) = (w2 as f32 / scale2, h2 as f32 / scale2);
            for _ in 0..2 {
                edit(&mut g, &mut ops, &st, lw2, lh2);
            }
            let bytes = encode(&ops);
            let want = reference_restyled(&bytes, w2, h2, scale2, &images);
            let report = r
                .render(&bytes, w2, h2, scale2, &mut restyled_engine, &images)
                .unwrap();
            same(
                (r.image(), report),
                &want,
                &format!("case {case}, an edit after the window changed"),
            );
        }
        assert!(partial > 200, "only {partial} frames were partial redraws");
        assert!(
            restyles_seen > 100,
            "only {restyles_seen} changes of style alone had text to redraw"
        );
    }

    /// SPEC-13.2.1, SPEC-11.3.2: the frame the budget is measured on, whole
    /// in bands and then with one card's button recoloured, which redraws
    /// only that button; a list that runs out of budget gives the
    /// reference's truncation and is drawn whole the next time; a decoded
    /// image (a new `ImageCache::generation`) redraws everything.
    #[test]
    fn spec_13_2_1_fast_path_frame_and_budget() {
        let _serial = crate::tests::serial();
        let (w, h) = (640u32, 400u32);
        let mut t = TextEngine::new();
        let frame = crate::bench::frame_2000(&mut t, w as f32, h as f32);
        let mut fresh = TextEngine::new();
        crate::bench::frame_2000(&mut fresh, w as f32, h as f32);
        let want = raster::render(&frame.list, w, h, 1.0, &mut fresh, &frame.images).unwrap();
        let mut r = Renderer::with_bands(3);
        r.forced_bands = Some(3);
        let report = r
            .render(&frame.list, w, h, 1.0, &mut t, &frame.images)
            .unwrap();
        same((r.image(), report), &want, "frame_2000 in bands");
        assert!(r.info().full);
        assert_eq!(r.info().bands, 3);
        // Recolour the first opaque rounded rectangle (a button).
        let mut ops = vy_dl::decode(&frame.list).unwrap();
        let k = ops
            .iter()
            .position(|o| matches!(o, Op::Rect { radius, w, .. } if *radius == 4.0 && *w == 80.0))
            .unwrap();
        if let Op::Rect { color, .. } = &mut ops[k] {
            *color = 0xFF00_00FF;
        }
        let edited = encode(&ops);
        let want = raster::render(&edited, w, h, 1.0, &mut fresh, &frame.images).unwrap();
        let report = r.render(&edited, w, h, 1.0, &mut t, &frame.images).unwrap();
        same(
            (r.image(), report),
            &want,
            "frame_2000, one button recoloured",
        );
        let [_, _, rw, rh] = r.info().redrawn.unwrap();
        assert!(!r.info().full);
        assert!(rw <= 82 && rh <= 26, "redrew {rw} x {rh}");

        // Out of budget: 70 translucent full-canvas fills at 256 x 256.
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        for _ in 0..70 {
            dl.rect(0.0, 0.0, 256.0, 256.0, 0.0, 0x0000_FF80).unwrap();
        }
        dl.end().unwrap();
        let none = ImageCache::empty(0);
        let want = raster::render(dl.bytes(), 256, 256, 1.0, &mut fresh, &none).unwrap();
        assert!(want.report.truncated);
        for _ in 0..2 {
            let report = r.render(dl.bytes(), 256, 256, 1.0, &mut t, &none).unwrap();
            same((r.image(), report), &want, "truncated");
            assert!(r.info().full);
        }

        // An image decoded after the first frame.
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.image(4.0, 4.0, 40.0, 30.0, 1, Fit::Cover).unwrap();
        dl.end().unwrap();
        let mut images = ImageCache::from_bytes(vec![test_picture()]);
        let want = raster::render(dl.bytes(), 64, 48, 1.0, &mut fresh, &images).unwrap();
        r.render(dl.bytes(), 64, 48, 1.0, &mut t, &images).unwrap();
        same((r.image(), r.report()), &want, "placeholder");
        images.decode(1).unwrap();
        let want = raster::render(dl.bytes(), 64, 48, 1.0, &mut fresh, &images).unwrap();
        r.render(dl.bytes(), 64, 48, 1.0, &mut t, &images).unwrap();
        same((r.image(), r.report()), &want, "decoded");
        assert!(r.info().full);
    }

    /// SPEC-11.3.2: the reference path is `render` itself; the fast one
    /// says what it does in the log line.
    #[test]
    fn spec_11_3_2_reference_path_is_render() {
        let mut t = TextEngine::new();
        let st = styles(&mut t);
        let images = ImageCache::empty(1);
        let mut g = Gen(77);
        let bytes = random_list(&mut g, &st, 80.0, 60.0);
        let want = reference(&bytes, 80, 60, 1.0, &images);
        let mut r = Renderer::new(RasterPath::Reference);
        let report = r.render(&bytes, 80, 60, 1.0, &mut t, &images).unwrap();
        same((r.image(), report), &want, "reference path");
        assert!(r.describe().contains("reference"));
        assert!(Renderer::with_bands(2).describe().contains("2 bands"));
        assert!(Renderer::with_bands(1).describe().contains("1 band on"));
        assert_eq!(
            r.render(&[0x01], 10, 10, 1.0, &mut t, &images).unwrap_err(),
            raster::render(&[0x01], 10, 10, 1.0, &mut t, &images).unwrap_err()
        );
        let mut f = Renderer::new(RasterPath::Fast);
        assert_eq!(
            f.render(&bytes, 0, 10, 1.0, &mut t, &images).unwrap_err(),
            RenderError::BadSize
        );
    }
}
