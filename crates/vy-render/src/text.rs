//! Text engine: styles, shaping, line breaking and glyph rasterization (SPEC §6.4).
//!
//! Shaping and font rendering happen in the host; the module only ever sees
//! UTF-8 bytes and measurements (SPEC-6.4.1). Fonts are bundled into the
//! binary and no system font is ever loaded, so the same text lays out the
//! same way on every machine (SPEC-2.1.5).

use cosmic_text::{
    fontdb, Attrs, Buffer, Fallback, Family, FontSystem, Metrics, Shaping, SwashCache, SwashImage,
    Weight, Wrap,
};
use std::borrow::Cow;
use std::sync::Arc;
use vy_abi::{err, font, weight};

/// Line height as a multiple of the font size. Fixed for determinism; a
/// module that wants other spacing places runs itself.
pub const LINE_HEIGHT_FACTOR: f32 = 1.25;
/// Largest accepted font size in logical pixels.
pub const MAX_FONT_SIZE_PX: f32 = 512.0;
/// Largest number of styles per module instance (style ids are `u16` on the wire).
pub const MAX_STYLES: usize = u16::MAX as usize;
/// Largest magnitude of a text origin or wrap width, in pixels. The canvas is
/// at most 16384 px, so anything beyond this is invisible; cosmic-text bins
/// glyph positions into `i32` and overflows on larger values (a right-to-left
/// line starts at the wrap width).
pub const MAX_COORD_PX: f32 = 16_777_216.0;

/// Unicode bidi paragraph separators (class B) that cosmic-text does not
/// treat as line endings. In the same line as a right-to-left run they trip
/// an assertion inside the shaper, so the host maps them to `\n`, which both
/// cosmic-text and `caret` split lines on.
const PARAGRAPH_SEPARATORS: [char; 5] = ['\u{1C}', '\u{1D}', '\u{1E}', '\u{85}', '\u{2029}'];

/// Text as handed to the shaper: paragraph separators replaced by `\n`.
fn sanitize(text: &str) -> Cow<'_, str> {
    if text.contains(PARAGRAPH_SEPARATORS) {
        Cow::Owned(text.replace(PARAGRAPH_SEPARATORS, "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

/// The bundled faces per built-in role (SPEC-6.4.2, `vy_abi::font`):
///
/// | Roll | Resurs-id | Snitt |
/// | --- | --- | --- |
/// | `sans` | −1 | DejaVu Sans 400, DejaVu Sans Bold 700 |
/// | `serif` | −2 | DejaVu Serif 400 |
/// | `mono` | −3 | DejaVu Sans Mono 400 |
/// | `ui` | −4 | same faces as `sans` |
/// | `emoji` | −5 | same faces as `sans` (DejaVu Sans covers symbols and dingbats; a colour emoji face arrives in 1.x) |
///
/// `text_style.weight` is clamped to 1..=1000 and mapped to the nearest
/// of these weights, the heavier on a tie (SPEC-5.2.5): for `sans`, `ui`
/// and `emoji` 1–549 is DejaVu Sans and 550–1000 DejaVu Sans Bold; `serif`
/// and `mono` are always their one face. The mapping is done here, so the
/// shaper always gets an exact family and weight and never falls back to
/// another family for a weight it lacks.
const FAMILY_SANS: &str = "DejaVu Sans";
const FAMILY_SERIF: &str = "DejaVu Serif";
const FAMILY_MONO: &str = "DejaVu Sans Mono";

/// Weights bundled for the sans family (`sans`, `ui`, `emoji`).
const WEIGHTS_SANS: [i32; 2] = [weight::NORMAL, weight::BOLD];
/// Weights bundled for `serif` and `mono`.
const WEIGHTS_SINGLE: [i32; 1] = [weight::NORMAL];

static FONT_SANS: &[u8] = include_bytes!("../fonts/DejaVuSans.ttf");
static FONT_SANS_BOLD: &[u8] = include_bytes!("../fonts/DejaVuSans-Bold.ttf");
static FONT_SERIF: &[u8] = include_bytes!("../fonts/DejaVuSerif.ttf");
static FONT_MONO: &[u8] = include_bytes!("../fonts/DejaVuSansMono.ttf");

/// Family name and bundled weights for a built-in role, or `None` for a
/// resource id that is not one (package fonts arrive with WOFF2 support).
fn family_for(font_resource_id: i32) -> Option<(&'static str, &'static [i32])> {
    match font_resource_id {
        font::SANS | font::UI | font::EMOJI => Some((FAMILY_SANS, &WEIGHTS_SANS)),
        font::SERIF => Some((FAMILY_SERIF, &WEIGHTS_SINGLE)),
        font::MONO => Some((FAMILY_MONO, &WEIGHTS_SINGLE)),
        _ => None,
    }
}

/// The face weight the host draws `weight` with for a role: the requested
/// weight clamped to 1..=1000, then the nearest bundled weight, the
/// heavier on a tie (SPEC-5.2.5). `None` for an id that is not a role.
pub fn face_weight(font_resource_id: i32, requested: i32) -> Option<i32> {
    let (_, weights) = family_for(font_resource_id)?;
    weight::nearest(requested, weights)
}

/// Platform-independent fallback: only the bundled faces, in a fixed order.
struct BundledFallback;

impl Fallback for BundledFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &[FAMILY_SANS, FAMILY_SERIF, FAMILY_MONO]
    }
    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }
    fn script_fallback(&self, _script: unicode_script::Script, _locale: &str) -> &[&'static str] {
        &[]
    }
}

/// A text style created by `text_style` (SPEC-5.2.5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    family: &'static str,
    /// The font resource id the style was made with (`vy_abi::font` for
    /// the built-in roles, SPEC-6.4.2), so another text engine can be
    /// given the same style: the main process draws what the tab process
    /// measured (SPEC-9.1.2).
    pub font: i32,
    /// Font size in logical pixels.
    pub size_px: f32,
    /// The weight asked for, clamped to 1..=1000.
    pub weight: u16,
    /// The weight of the bundled face the text is drawn with: the nearest
    /// to `weight`, the heavier on a tie (SPEC-5.2.5; see [`face_weight`]).
    pub face_weight: u16,
    /// `0xRRGGBBAA`.
    pub color: u32,
}

impl Style {
    fn metrics(&self) -> Metrics {
        Metrics::new(self.size_px, self.size_px * LINE_HEIGHT_FACTOR)
    }
    fn attrs(&self) -> Attrs<'static> {
        Attrs::new()
            .family(Family::Name(self.family))
            .weight(Weight(self.face_weight))
    }
}

/// Measured size of a laid-out run in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    /// Widest line.
    pub w: f32,
    /// Number of lines times the line height.
    pub h: f32,
}

/// A rasterized glyph positioned in physical pixels.
pub struct Glyph<'a> {
    /// Left edge in physical pixels.
    pub x: i32,
    /// Top edge in physical pixels.
    pub y: i32,
    /// Coverage or colour bitmap.
    pub image: &'a SwashImage,
}

/// Shaping, layout and rasterization state for one module instance.
pub struct TextEngine {
    font_system: FontSystem,
    swash: SwashCache,
    buffer: Buffer,
    styles: Vec<Style>,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    /// Engine with the bundled fonts only (SPEC-6.4.2); never touches system fonts.
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        for bytes in [FONT_SANS, FONT_SANS_BOLD, FONT_SERIF, FONT_MONO] {
            db.load_font_source(fontdb::Source::Binary(Arc::new(bytes)));
        }
        db.set_sans_serif_family(FAMILY_SANS);
        db.set_serif_family(FAMILY_SERIF);
        db.set_monospace_family(FAMILY_MONO);
        // Fixed locale: the locale only selects script fallbacks, which are empty here.
        let mut font_system =
            FontSystem::new_with_locale_and_db_and_fallback("sv-SE".into(), db, BundledFallback);
        let buffer = Buffer::new(&mut font_system, Metrics::new(16.0, 20.0));
        Self {
            font_system,
            swash: SwashCache::new(),
            buffer,
            styles: Vec::new(),
        }
    }

    /// `text_style`: register a style and return its id (1-based) or an ABI error.
    ///
    /// `E_RESOURCE` for an unknown font resource, `E_ARG` for a size out of
    /// range, `E_STYLE` when the style table is full. The weight never
    /// fails: it is clamped to 1..=1000 and drawn with the nearest bundled
    /// face (SPEC-5.2.5, [`face_weight`]).
    pub fn style(
        &mut self,
        font_resource_id: i32,
        size_px: f32,
        requested_weight: i32,
        color: u32,
    ) -> Result<u16, i32> {
        let (family, weights) = family_for(font_resource_id).ok_or(err::E_RESOURCE)?;
        let size_ok = size_px.is_finite() && size_px > 0.0 && size_px <= MAX_FONT_SIZE_PX;
        if !size_ok {
            return Err(err::E_ARG);
        }
        if self.styles.len() >= MAX_STYLES {
            return Err(err::E_STYLE);
        }
        let clamped = weight::clamp(requested_weight);
        let face = weight::nearest(clamped, weights).expect("every role has a face");
        self.styles.push(Style {
            family,
            font: font_resource_id,
            size_px,
            weight: clamped as u16,
            face_weight: face as u16,
            color,
        });
        Ok(self.styles.len() as u16)
    }

    /// Look up a style by id.
    pub fn get_style(&self, style_id: u16) -> Option<Style> {
        if style_id == 0 {
            return None;
        }
        self.styles.get(usize::from(style_id) - 1).copied()
    }

    /// Number of registered styles.
    pub fn style_count(&self) -> usize {
        self.styles.len()
    }

    fn prepare(&mut self, style: Style, max_w: Option<f32>, text: &str) {
        let buffer = &mut self.buffer;
        buffer.set_metrics(style.metrics());
        buffer.set_wrap(Wrap::WordOrGlyph);
        // A non-positive, non-finite or absurd width means no wrapping: the
        // buffer is unbounded, so a right-to-left line starts at its own width,
        // not at the far right of some huge placeholder width.
        let width = max_w.filter(|w| *w > 0.0 && *w <= MAX_COORD_PX);
        buffer.set_size(width, None);
        buffer.set_text(&sanitize(text), &style.attrs(), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
    }

    fn measured(&self, style: Style) -> Size {
        let mut w: f32 = 0.0;
        let mut lines = 0u32;
        for run in self.buffer.layout_runs() {
            w = w.max(run.line_w);
            lines += 1;
        }
        Size {
            w,
            h: lines as f32 * style.size_px * LINE_HEIGHT_FACTOR,
        }
    }

    /// `text_measure`: width of the widest line and total height after wrapping
    /// at `max_w` logical pixels (SPEC-5.2.5, SPEC-6.4.3). `E_STYLE` for an unknown style.
    pub fn measure(&mut self, style_id: u16, max_w: f32, text: &str) -> Result<Size, i32> {
        let style = self.get_style(style_id).ok_or(err::E_STYLE)?;
        self.prepare(style, Some(max_w), text);
        Ok(self.measured(style))
    }

    /// `text_caret`: position of the caret before `byte_offset` in logical pixels,
    /// in an unwrapped layout. Offsets past the end clamp to the end.
    pub fn caret(
        &mut self,
        style_id: u16,
        text: &str,
        byte_offset: usize,
    ) -> Result<(f32, f32), i32> {
        let style = self.get_style(style_id).ok_or(err::E_STYLE)?;
        let byte_offset = byte_offset.min(text.len());
        // The shaper sees `sanitize(text)`, where each paragraph separator has
        // become one byte; move the offset the same way.
        let mut mapped = 0usize;
        for (i, ch) in text.char_indices() {
            if i >= byte_offset {
                break;
            }
            let taken = (byte_offset - i).min(ch.len_utf8());
            mapped += if PARAGRAPH_SEPARATORS.contains(&ch) {
                taken.min(1)
            } else {
                taken
            };
        }
        let text = sanitize(text);
        let byte_offset = mapped;
        self.prepare(style, None, &text);
        // Buffer lines split on line endings; find the line holding the offset.
        let mut line = 0usize;
        let mut line_start = 0usize;
        for (i, ch) in text.char_indices() {
            if i >= byte_offset {
                break;
            }
            if ch == '\n' {
                line += 1;
                line_start = i + 1;
            }
        }
        let cursor = cosmic_text::Cursor::new(line, byte_offset - line_start);
        Ok(self.buffer.cursor_position(&cursor).unwrap_or((0.0, 0.0)))
    }

    /// Lay out `text` at logical origin `(x, y)` wrapped at `max_w`, rasterize
    /// every glyph at `scale` and hand it to `sink` in paint order.
    ///
    /// Returns the measured size. `E_STYLE` for an unknown style.
    pub fn rasterize(
        &mut self,
        style_id: u16,
        origin: (f32, f32),
        max_w: f32,
        scale: f32,
        text: &str,
        mut sink: impl FnMut(Glyph<'_>),
    ) -> Result<Size, i32> {
        let style = self.get_style(style_id).ok_or(err::E_STYLE)?;
        self.prepare(style, Some(max_w), text);
        let size = self.measured(style);
        // An origin that is not finite, or so far away that nothing can be
        // visible, is measured but never positioned (see `MAX_COORD_PX`).
        let (px, py) = (origin.0 * scale, origin.1 * scale);
        if !(px.is_finite() && py.is_finite()) || px.abs() > MAX_COORD_PX || py.abs() > MAX_COORD_PX
        {
            return Ok(size);
        }
        let mut keys = Vec::new();
        for run in self.buffer.layout_runs() {
            let offset = (origin.0 * scale, (origin.1 + run.line_y) * scale);
            for glyph in run.glyphs {
                keys.push(glyph.physical(offset, scale));
            }
        }
        for pg in keys {
            if let Some(image) = self.swash.get_image(&mut self.font_system, pg.cache_key) {
                if image.placement.width == 0 || image.placement.height == 0 {
                    continue;
                }
                sink(Glyph {
                    x: pg.x + image.placement.left,
                    y: pg.y - image.placement.top,
                    image,
                });
            }
        }
        Ok(size)
    }
}

/// Pack a measurement as `dl_text`, `text_measure` and `text_caret` return
/// it: the first value × 100 in the high 32 bits, the second × 100 in the
/// low 32 bits, each half clamped to `0..=2³¹ − 1`, so the whole value is
/// never negative and a negative `i64` is always a status code
/// (SPEC-5.2.4, SPEC-5.2.5).
pub fn pack_x100(a: f32, b: f32) -> i64 {
    let half = |v: f32| (f64::from(v) * 100.0).round().clamp(0.0, 2_147_483_647.0) as i64;
    (half(a) << 32) | half(b)
}
