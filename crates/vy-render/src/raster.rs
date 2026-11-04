//! Software rasterizer for display lists (SPEC §6).
//!
//! Every op is drawn in order, later over earlier (SPEC-6.1.3). Coordinates
//! are logical pixels scaled by `scale` to physical pixels (SPEC-6.3.1).
//! Headless and windowed rendering share this function (SPEC-11.3.2).

use crate::image::RgbaImage;
use crate::text::{Glyph, TextEngine};
use cosmic_text::SwashContent;
use tiny_skia::{
    Color, FillRule, IntSize, Mask, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke,
    Transform,
};
use vy_dl::{DecodeError, Op, PathCmd};

/// Number of nested rectangles used to approximate a blurred shadow (M1).
///
/// A real Gaussian blur arrives with the GPU path in M3. Until then a shadow
/// is `SHADOW_STEPS` concentric rounded rectangles, each with `alpha /
/// SHADOW_STEPS`, spread linearly from `-blur` to `+blur` around the edge.
pub const SHADOW_STEPS: u32 = 8;
/// Colour of the placeholder drawn for `IMAGE` until image decoding lands (M1.5).
pub const IMAGE_PLACEHOLDER_COLOR: u32 = 0x8080_80FF;

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
                f.write_str("output size must be at least 1x1 and at most 16384x16384")
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

/// Clip stack in physical pixels. The mask is rebuilt only when the stack changes.
struct Clip {
    width: u32,
    height: u32,
    stack: Vec<Option<Rect>>,
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

    fn current(&self) -> Option<Option<Rect>> {
        self.stack.last().copied()
    }

    fn push(&mut self, rect: Option<Rect>) {
        let next = match (self.current(), rect) {
            (None, r) => r,
            (Some(None), _) | (Some(_), None) => None,
            (Some(Some(a)), Some(b)) => a.intersect(&b),
        };
        self.stack.push(next);
        self.rebuild();
    }

    fn pop(&mut self) {
        self.stack.pop();
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.mask = match self.current() {
            None => None,
            Some(rect) => {
                let mut mask = Mask::new(self.width, self.height);
                if let Some(m) = mask.as_mut() {
                    if let Some(r) = rect {
                        m.fill_path(
                            &PathBuilder::from_rect(r),
                            FillRule::Winding,
                            false,
                            Transform::identity(),
                        );
                    }
                }
                mask
            }
        };
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

    #[allow(clippy::too_many_arguments)]
    fn shadow(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32, blur: f32, rgba: u32) {
        let blur = blur.max(0.0);
        if blur == 0.0 {
            self.rect(x, y, w, h, radius, rgba);
            return;
        }
        // Source-over accumulation: after step k the coverage is alpha * k / N,
        // a linear ramp from the outer edge (+blur) to the inner edge (-blur).
        let alpha = (rgba & 0xFF) as f32 / 255.0;
        let n = SHADOW_STEPS as f32;
        for k in 1..=SHADOW_STEPS {
            let step_alpha = alpha / (n - alpha * (k as f32 - 1.0));
            let step_color =
                (rgba & !0xFF) | ((step_alpha * 255.0).round().clamp(0.0, 255.0) as u32);
            let d = blur * (1.0 - (2.0 * k as f32 - 1.0) / n);
            self.rect(
                x - d,
                y - d,
                w + 2.0 * d,
                h + 2.0 * d,
                radius + d,
                step_color,
            );
        }
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, w: f32, rgba: u32) {
        let mut pb = PathBuilder::new();
        pb.move_to(x1, y1);
        pb.line_to(x2, y2);
        if let Some(path) = pb.finish() {
            self.stroke(&path, rgba, w);
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

    fn clip_push(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let s = self.scale;
        self.clip.push(Rect::from_xywh(
            x * s,
            y * s,
            (w * s).max(0.0),
            (h * s).max(0.0),
        ));
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
/// ratio (SPEC-6.3.1). The canvas starts opaque white.
pub fn render(
    display_list: &[u8],
    width: u32,
    height: u32,
    scale: f32,
    text: &mut TextEngine,
) -> Result<RgbaImage, RenderError> {
    let ops = vy_dl::decode(display_list).map_err(RenderError::Decode)?;
    let scale_ok = scale.is_finite() && scale > 0.0;
    if !(1..=16384).contains(&width) || !(1..=16384).contains(&height) || !scale_ok {
        return Err(RenderError::BadSize);
    }
    let mut pixmap = Pixmap::new(width, height).ok_or(RenderError::BadSize)?;
    pixmap.fill(Color::WHITE);
    let mut canvas = Canvas {
        pixmap,
        scale,
        clip: Clip::new(width, height),
        text,
    };
    for op in &ops {
        match op {
            Op::Rect {
                x,
                y,
                w,
                h,
                radius,
                color,
            } => canvas.rect(*x, *y, *w, *h, *radius, *color),
            Op::Text {
                x,
                y,
                max_w,
                style_id,
                text,
            } => canvas.text(*x, *y, *max_w, *style_id, text),
            Op::Image { x, y, w, h, .. } => {
                canvas.rect(*x, *y, *w, *h, 0.0, IMAGE_PLACEHOLDER_COLOR);
            }
            Op::Path {
                color,
                stroke_w,
                cmds,
            } => canvas.path(cmds, *color, *stroke_w),
            Op::ClipPush { x, y, w, h } => canvas.clip_push(*x, *y, *w, *h),
            Op::ClipPop => canvas.clip.pop(),
            Op::Line {
                x1,
                y1,
                x2,
                y2,
                w,
                color,
            } => canvas.line(*x1, *y1, *x2, *y2, *w, *color),
            Op::Shadow {
                x,
                y,
                w,
                h,
                radius,
                blur,
                color,
            } => canvas.shadow(*x, *y, *w, *h, *radius, *blur, *color),
        }
    }
    Ok(RgbaImage {
        width,
        height,
        data: canvas.pixmap.take_demultiplied(),
    })
}
