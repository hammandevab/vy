//! Software rasterizer for display lists (SPEC §6).
//!
//! Every op is drawn in order, later over earlier (SPEC-6.1.3). Coordinates
//! are logical pixels scaled by `scale` to physical pixels (SPEC-6.3.1).
//! Headless and windowed rendering share this function (SPEC-11.3.2).

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

/// One `CLIP_PUSH` in physical pixels: a rectangle with a corner radius, or
/// nothing at all when the rectangle was degenerate or absurd.
#[derive(Clone, Copy)]
struct ClipShape {
    rect: Option<Rect>,
    radius: f32,
}

/// Clip stack in physical pixels. The mask is rebuilt only when the stack
/// changes: plain rectangles are intersected analytically and filled without
/// anti-aliasing, so a clip never softens a crisp edge; as soon as a rounded
/// shape is on the stack every shape is rasterized and the masks multiplied
/// (SPEC-5.2.3, SPEC-6.1.4).
struct Clip {
    width: u32,
    height: u32,
    stack: Vec<ClipShape>,
    mask: Option<Mask>,
}

impl Clip {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            stack: Vec::new(),
            mask: None,
        }
    }

    fn push(&mut self, shape: ClipShape) {
        self.stack.push(shape);
        self.rebuild();
    }

    fn pop(&mut self) {
        self.stack.pop();
        self.rebuild();
    }

    fn rebuild(&mut self) {
        if self.stack.is_empty() {
            self.mask = None;
            return;
        }
        let Some(mut mask) = Mask::new(self.width, self.height) else {
            self.mask = None;
            return;
        };
        let rounded = self.stack.iter().any(|s| s.radius > 0.0);
        if !rounded {
            // Rectangles only: the intersection is a rectangle.
            let mut current: Option<Rect> = self.stack[0].rect;
            for s in &self.stack[1..] {
                current = match (current, s.rect) {
                    (Some(a), Some(b)) => a.intersect(&b),
                    _ => None,
                };
            }
            if let Some(r) = current {
                mask.fill_path(
                    &PathBuilder::from_rect(r),
                    FillRule::Winding,
                    false,
                    Transform::identity(),
                );
            }
        } else {
            for (i, s) in self.stack.iter().enumerate() {
                let path = s
                    .rect
                    .and_then(|r| rounded_rect(r.x(), r.y(), r.width(), r.height(), s.radius));
                match path {
                    Some(p) if i == 0 => {
                        mask.fill_path(&p, FillRule::Winding, s.radius > 0.0, Transform::identity())
                    }
                    Some(p) => mask.intersect_path(
                        &p,
                        FillRule::Winding,
                        s.radius > 0.0,
                        Transform::identity(),
                    ),
                    // An empty shape clips everything.
                    None => {
                        mask.clear();
                        break;
                    }
                }
            }
        }
        self.mask = Some(mask);
    }

    fn mask(&self) -> Option<&Mask> {
        self.mask.as_ref()
    }
}

struct Canvas<'a> {
    pixmap: Pixmap,
    scale: f32,
    clip: Clip,
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

impl Canvas<'_> {
    fn transform(&self) -> Transform {
        Transform::from_scale(self.scale, self.scale)
    }

    fn fill(&mut self, path: &Path, rgba: u32) {
        let t = self.transform();
        self.pixmap
            .fill_path(path, &paint(rgba), FillRule::Winding, t, self.clip.mask());
    }

    fn stroke(&mut self, path: &Path, rgba: u32, width: f32) {
        let t = self.transform();
        let stroke = Stroke {
            width: width.max(0.0),
            ..Stroke::default()
        };
        self.pixmap
            .stroke_path(path, &paint(rgba), &stroke, t, self.clip.mask());
    }

    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, rgba: u32) {
        if let Some(path) = rounded_rect(x, y, w, h, radius) {
            self.fill(&path, rgba);
        }
    }

    fn path(&mut self, cmds: &[PathCmd], rgba: u32, stroke_w: f32) {
        let mut pb = PathBuilder::new();
        for c in cmds {
            match *c {
                PathCmd::MoveTo(x, y) => pb.move_to(x, y),
                PathCmd::LineTo(x, y) => pb.line_to(x, y),
                PathCmd::QuadTo { ctrl, to } => pb.quad_to(ctrl.0, ctrl.1, to.0, to.1),
                PathCmd::Close => pb.close(),
            }
        }
        let Some(path) = pb.finish() else { return };
        if stroke_w > 0.0 {
            self.stroke(&path, rgba, stroke_w);
        } else {
            self.fill(&path, rgba);
        }
    }

    fn image(&mut self, x: f32, y: f32, w: f32, h: f32, resource_id: u16, fit_mode: u8) {
        let Ok(img) = self.images.get(resource_id) else {
            self.rect(x, y, w, h, 0.0, IMAGE_PLACEHOLDER_COLOR);
            return;
        };
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
            return;
        };
        let Some(sampled) = resample(img, &placement) else {
            return;
        };
        self.pixmap.draw_pixmap(
            placement.px0,
            placement.py0,
            sampled.as_ref(),
            &PixmapPaint::default(),
            Transform::identity(),
            self.clip.mask(),
        );
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

    fn text(&mut self, x: f32, y: f32, max_w: f32, style_id: u16, text: &[u8]) {
        // Invalid UTF-8 is drawn lossily; the module is told nothing extra (SPEC-6.4.1).
        let s = String::from_utf8_lossy(text);
        let Some(style) = self.text.get_style(style_id) else {
            return;
        };
        let rgba = style.color;
        let scale = self.scale;
        let pixmap = &mut self.pixmap;
        let mask = self.clip.mask();
        let _ = self
            .text
            .rasterize(style_id, (x, y), max_w, scale, &s, |g| {
                blit_glyph(pixmap, mask, &g, rgba);
            });
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
/// [`IMAGE_PLACEHOLDER_COLOR`].
pub fn render(
    display_list: &[u8],
    width: u32,
    height: u32,
    scale: f32,
    text: &mut TextEngine,
    images: &ImageCache,
) -> Result<RgbaImage, RenderError> {
    let ops = vy_dl::decode(display_list).map_err(RenderError::Decode)?;
    let scale_ok = scale.is_finite() && scale > 0.0;
    let side = 1..=MAX_CANVAS_SIDE_PX;
    if !side.contains(&width) || !side.contains(&height) || !scale_ok {
        return Err(RenderError::BadSize);
    }
    let mut pixmap = Pixmap::new(width, height).ok_or(RenderError::BadSize)?;
    pixmap.fill(Color::WHITE);
    let mut canvas = Canvas {
        pixmap,
        scale,
        clip: Clip::new(width, height),
        text,
        images,
    };
    // Ops with non-finite or absurd values are skipped (`MAX_MAGNITUDE`);
    // `CLIP_PUSH` is the exception and always pushes, see `Canvas::clip_push`.
    // A rounded clip (`radius > 0`) clips to a rounded rectangle (SPEC-5.2.3).
    for op in &ops {
        match op {
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
            Op::ClipPush { x, y, w, h, radius } => canvas.clip_push(*x, *y, *w, *h, *radius),
            Op::ClipPop => canvas.clip.pop(),
            Op::Rect { .. } | Op::Text { .. } | Op::Image { .. } | Op::Path { .. } => {}
        }
    }
    Ok(RgbaImage {
        width,
        height,
        data: canvas.pixmap.take_demultiplied(),
    })
}
