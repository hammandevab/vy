//! Text engine: styles, shaping, line breaking and glyph rasterization (SPEC §6.4).
//!
//! Shaping and font rendering happen in the host; the module only ever sees
//! UTF-8 bytes and measurements (SPEC-6.4.1). Fonts are bundled into the
//! binary and no system font is ever loaded, so the same text lays out the
//! same way on every machine (SPEC-2.1.5).

use cosmic_text::{
    fontdb, Attrs, Buffer, Fallback, Family, FontSystem, LayoutGlyph, Metrics, Shaping, SwashCache,
    SwashImage, Weight, Wrap,
};
use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
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

/// Hits and misses of the shaped-text cache (SPEC-13.2.3): a lookup is
/// one `dl_text`, `text_measure` or rasterized run; a hit reuses the
/// shaped and positioned glyphs, a miss shapes them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Lookups answered from the cache.
    pub hits: u64,
    /// Lookups that shaped the text.
    pub misses: u64,
}

impl CacheStats {
    /// Hits plus misses.
    pub fn lookups(&self) -> u64 {
        self.hits + self.misses
    }

    /// Hits over lookups, 0 when there were none.
    pub fn hit_rate(&self) -> f64 {
        if self.lookups() == 0 {
            0.0
        } else {
            self.hits as f64 / self.lookups() as f64
        }
    }
}

/// Units one generation of the shaped-text cache holds: a shaped text
/// costs one unit per glyph, per line and per byte of its text. The cache
/// keeps two generations, so an engine holds at most twice this many units,
/// a few megabytes; a text costing more than this is shaped every time.
pub const SHAPE_CACHE_UNITS: usize = 1 << 16;
/// Rasterized glyph images an engine keeps; past this the image cache
/// starts over, so a page cycling through sizes cannot grow it without
/// bound (cosmic-text's own cache has no limit).
pub const GLYPH_IMAGE_CAP: usize = 16_384;

/// A shaped and laid-out text: the glyphs of every line and the measured size.
struct Shaped {
    size: Size,
    lines: Vec<ShapedLine>,
    units: usize,
}

struct ShapedLine {
    line_y: f32,
    glyphs: Vec<LayoutGlyph>,
}

/// What shaping depends on: the face, the size, the wrap width and the
/// text. The colour does not, so styles that differ only in colour share
/// entries.
#[derive(Clone, Copy)]
struct Probe<'a> {
    family: &'static str,
    face_weight: u16,
    size_bits: u32,
    width_bits: Option<u32>,
    text: &'a str,
}

impl Probe<'_> {
    fn hash(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.family.hash(&mut h);
        self.face_weight.hash(&mut h);
        self.size_bits.hash(&mut h);
        self.width_bits.hash(&mut h);
        self.text.hash(&mut h);
        h.finish()
    }

    fn matches(&self, key: &ShapeKey) -> bool {
        self.family == key.family
            && self.face_weight == key.face_weight
            && self.size_bits == key.size_bits
            && self.width_bits == key.width_bits
            && self.text == &*key.text
    }

    fn key(&self) -> ShapeKey {
        ShapeKey {
            family: self.family,
            face_weight: self.face_weight,
            size_bits: self.size_bits,
            width_bits: self.width_bits,
            text: self.text.into(),
        }
    }
}

/// [`Probe`] with the text owned.
struct ShapeKey {
    family: &'static str,
    face_weight: u16,
    size_bits: u32,
    width_bits: Option<u32>,
    text: Box<str>,
}

type Bucket = Vec<(ShapeKey, Arc<Shaped>)>;

/// The shaped-text cache: two generations. A lookup searches the young
/// one, then the old one, and moves a hit found there to the young one;
/// when the young one is full it becomes the old one and the old one is
/// dropped. What a frame keeps using therefore stays, and the cache never
/// holds more than two generations of [`SHAPE_CACHE_UNITS`]. Hits only
/// save time: a cached text is what shaping it again would give.
#[derive(Default)]
struct ShapeCache {
    young: HashMap<u64, Bucket>,
    old: HashMap<u64, Bucket>,
    young_units: usize,
}

impl ShapeCache {
    fn get(&mut self, hash: u64, probe: &Probe<'_>) -> Option<Arc<Shaped>> {
        if let Some((_, shaped)) = self
            .young
            .get(&hash)
            .and_then(|b| b.iter().find(|(k, _)| probe.matches(k)))
        {
            return Some(Arc::clone(shaped));
        }
        let bucket = self.old.get_mut(&hash)?;
        let i = bucket.iter().position(|(k, _)| probe.matches(k))?;
        let (key, shaped) = bucket.swap_remove(i);
        if bucket.is_empty() {
            self.old.remove(&hash);
        }
        self.insert(hash, key, Arc::clone(&shaped));
        Some(shaped)
    }

    fn insert(&mut self, hash: u64, key: ShapeKey, shaped: Arc<Shaped>) {
        let units = shaped.units;
        if units > SHAPE_CACHE_UNITS {
            return;
        }
        if self.young_units + units > SHAPE_CACHE_UNITS {
            self.old = std::mem::take(&mut self.young);
            self.young_units = 0;
        }
        self.young_units += units;
        self.young.entry(hash).or_default().push((key, shaped));
    }
}

/// Shaping, layout and rasterization state for one module instance.
///
/// It persists across frames: the font system, the rasterized glyph
/// images (cosmic-text's `SwashCache`, capped at [`GLYPH_IMAGE_CAP`]) and
/// the shaped texts ([`SHAPE_CACHE_UNITS`]), so a text drawn again, by
/// `dl_text` and then by the rasterizer, or in the next frame, is shaped
/// once.
pub struct TextEngine {
    font_system: FontSystem,
    swash: SwashCache,
    buffer: Buffer,
    styles: Vec<Style>,
    cache: ShapeCache,
    stats: CacheStats,
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
            cache: ShapeCache::default(),
            stats: CacheStats::default(),
        }
    }

    /// Hits and misses of the shaped-text cache so far (SPEC-13.2.3).
    pub fn cache_stats(&self) -> CacheStats {
        self.stats
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

    /// Shape `text` into the buffer. `width` is the wrap width, already
    /// normalized by [`wrap_width`].
    fn shape_into_buffer(&mut self, style: Style, width: Option<f32>, text: &str) {
        let buffer = &mut self.buffer;
        buffer.set_metrics(style.metrics());
        buffer.set_wrap(Wrap::WordOrGlyph);
        buffer.set_size(width, None);
        buffer.set_text(&sanitize(text), &style.attrs(), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
    }

    /// The shaped text, from the cache or shaped now (SPEC-13.2.3).
    fn layout(&mut self, style: Style, max_w: Option<f32>, text: &str) -> Arc<Shaped> {
        let width = wrap_width(max_w);
        let probe = Probe {
            family: style.family,
            face_weight: style.face_weight,
            size_bits: style.size_px.to_bits(),
            width_bits: width.map(f32::to_bits),
            text,
        };
        let hash = probe.hash();
        if let Some(shaped) = self.cache.get(hash, &probe) {
            self.stats.hits += 1;
            return shaped;
        }
        self.stats.misses += 1;
        self.shape_into_buffer(style, width, text);
        let mut w: f32 = 0.0;
        let mut glyphs = 0;
        let mut lines = Vec::new();
        for run in self.buffer.layout_runs() {
            w = w.max(run.line_w);
            glyphs += run.glyphs.len();
            lines.push(ShapedLine {
                line_y: run.line_y,
                glyphs: run.glyphs.to_vec(),
            });
        }
        let shaped = Arc::new(Shaped {
            size: Size {
                w,
                h: lines.len() as f32 * style.size_px * LINE_HEIGHT_FACTOR,
            },
            units: glyphs + lines.len() + text.len(),
            lines,
        });
        self.cache.insert(hash, probe.key(), Arc::clone(&shaped));
        shaped
    }

    /// `text_measure`: width of the widest line and total height after wrapping
    /// at `max_w` logical pixels (SPEC-5.2.5, SPEC-6.4.3). `E_STYLE` for an unknown style.
    pub fn measure(&mut self, style_id: u16, max_w: f32, text: &str) -> Result<Size, i32> {
        let style = self.get_style(style_id).ok_or(err::E_STYLE)?;
        Ok(self.layout(style, Some(max_w), text).size)
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
        self.shape_into_buffer(style, None, &text);
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
        let shaped = self.layout(style, Some(max_w), text);
        let size = shaped.size;
        // An origin that is not finite, or so far away that nothing can be
        // visible, is measured but never positioned (see `MAX_COORD_PX`).
        let (px, py) = (origin.0 * scale, origin.1 * scale);
        if !(px.is_finite() && py.is_finite()) || px.abs() > MAX_COORD_PX || py.abs() > MAX_COORD_PX
        {
            return Ok(size);
        }
        if self.swash.image_cache.len() > GLYPH_IMAGE_CAP {
            self.swash.image_cache.clear();
        }
        for line in &shaped.lines {
            let offset = (origin.0 * scale, (origin.1 + line.line_y) * scale);
            for glyph in &line.glyphs {
                let pg = glyph.physical(offset, scale);
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
        }
        Ok(size)
    }
}

/// The wrap width the shaper gets for `max_w`: none for a non-positive,
/// non-finite or absurd width, so the buffer is unbounded and a
/// right-to-left line starts at its own width, not at the far right of
/// some huge placeholder width (SPEC-6.4.3).
fn wrap_width(max_w: Option<f32>) -> Option<f32> {
    max_w.filter(|w| *w > 0.0 && *w <= MAX_COORD_PX)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Glyphs of a rasterized run as plain data: position, placement and bitmap.
    fn glyphs(
        engine: &mut TextEngine,
        style: u16,
        max_w: f32,
        text: &str,
    ) -> Vec<(i32, i32, Vec<u8>)> {
        let mut out = Vec::new();
        engine
            .rasterize(style, (3.25, 7.5), max_w, 1.5, text, |g| {
                out.push((g.x, g.y, g.image.data.clone()));
            })
            .unwrap();
        out
    }

    impl ShapeCache {
        fn units(&self) -> usize {
            [&self.young, &self.old]
                .iter()
                .flat_map(|m| m.values())
                .flatten()
                .map(|(_, s)| s.units)
                .sum()
        }
    }

    /// SPEC-13.2.3: a text is shaped once per (face, size, wrap width,
    /// bytes): `dl_text` measuring it and the rasterizer drawing it, and
    /// styles that differ only in colour, share one shaping; another
    /// width, size or text is shaped anew; every width that means "no
    /// wrapping" is one key (SPEC-6.4.3).
    #[test]
    fn spec_13_2_3_shaped_text_cache_hits_and_misses() {
        let mut t = TextEngine::new();
        let ink = t.style(font::SANS, 14.0, 400, 0x1122_33FF).unwrap();
        let red = t.style(font::SANS, 14.0, 400, 0xFF00_00FF).unwrap();
        let big = t.style(font::SANS, 15.0, 400, 0x1122_33FF).unwrap();
        let bold = t.style(font::SANS, 14.0, 700, 0x1122_33FF).unwrap();
        let stats = |t: &TextEngine| (t.cache_stats().hits, t.cache_stats().misses);
        let a = t.measure(ink, 200.0, "Vy ritar text").unwrap();
        assert_eq!(stats(&t), (0, 1));
        assert_eq!(t.measure(ink, 200.0, "Vy ritar text").unwrap(), a);
        assert_eq!(t.measure(red, 200.0, "Vy ritar text").unwrap(), a);
        assert_eq!(stats(&t), (2, 1), "colour is not part of the key");
        glyphs(&mut t, ink, 200.0, "Vy ritar text");
        assert_eq!(stats(&t), (3, 1), "the rasterizer reuses the measurement");
        t.measure(ink, 40.0, "Vy ritar text").unwrap();
        t.measure(big, 200.0, "Vy ritar text").unwrap();
        t.measure(bold, 200.0, "Vy ritar text").unwrap();
        t.measure(ink, 200.0, "Vy ritar texten").unwrap();
        assert_eq!(stats(&t), (3, 5), "width, size, face and text are keys");
        t.measure(ink, 0.0, "Vy").unwrap();
        t.measure(ink, -5.0, "Vy").unwrap();
        t.measure(ink, f32::NAN, "Vy").unwrap();
        assert_eq!(stats(&t), (5, 6), "every non-wrapping width is one key");
        assert!((t.cache_stats().hit_rate() - 5.0 / 11.0).abs() < 1e-12);
    }

    /// SPEC-13.2.3, SPEC-2.1.5: a cached layout is what shaping gives: an
    /// engine that has shaped, evicted and reshaped many texts draws every
    /// glyph where a fresh engine draws it, with the same bitmap, and
    /// measures the same. Paragraph separators, right-to-left runs,
    /// wrapping and unwrapped runs included.
    #[test]
    fn cached_layout_is_what_shaping_gives() {
        let texts = [
            "Vy ritar text i värden, aldrig i modulen",
            "Lägesrapport för november: allt grönt och inget rött i sikte alls",
            "\u{5e9}\u{5dc}\u{5d5}\u{5dd} world \u{5e2}\u{5d5}\u{5dc}\u{5dd}",
            "one\u{2029}two\u{85}three\nfour",
            "",
            "    ",
            "0x2F5BD6FF  rect 12 x 7  clip 3  text 41",
        ];
        let mut warm = TextEngine::new();
        let mut cold = TextEngine::new();
        let style = |t: &mut TextEngine| {
            [
                t.style(font::SANS, 14.0, 400, 0x2221_1FFF).unwrap(),
                t.style(font::SERIF, 17.0, 400, 0x2221_1FFF).unwrap(),
                t.style(font::MONO, 12.0, 400, 0x2221_1FFF).unwrap(),
            ]
        };
        let ws = style(&mut warm);
        let cs = style(&mut cold);
        // Fill the cache past a generation so entries are evicted and
        // promoted between the draws below.
        let filler: String = "ord ".repeat(400);
        for round in 0..3 {
            for i in 0..80 {
                warm.measure(ws[i % 3], 300.0, &format!("{filler}{i}"))
                    .unwrap();
            }
            for (k, text) in texts.iter().enumerate() {
                for max_w in [0.0, 90.0, 300.0] {
                    let (w, c) = (ws[k % 3], cs[k % 3]);
                    assert_eq!(
                        warm.measure(w, max_w, text).unwrap(),
                        cold.measure(c, max_w, text).unwrap(),
                        "round {round}: {text:?} at {max_w}"
                    );
                    let mut fresh = TextEngine::new();
                    let fs = style(&mut fresh);
                    assert_eq!(
                        glyphs(&mut warm, w, max_w, text),
                        glyphs(&mut fresh, fs[k % 3], max_w, text),
                        "round {round}: {text:?} at {max_w}"
                    );
                }
            }
        }
        assert!(warm.cache_stats().hits > 0 && warm.cache_stats().misses > 0);
    }

    /// The cache holds at most two generations of [`SHAPE_CACHE_UNITS`]
    /// however many texts go through it, and a text too large for a
    /// generation is shaped every time rather than cached.
    #[test]
    fn shaped_text_cache_is_bounded() {
        let mut t = TextEngine::new();
        let s = t.style(font::SANS, 12.0, 400, 0x0000_00FF).unwrap();
        for i in 0..2_000 {
            t.measure(s, 500.0, &format!("{i} {}", "x".repeat(i % 300)))
                .unwrap();
            assert!(t.cache.units() <= 2 * SHAPE_CACHE_UNITS);
        }
        let huge = "y".repeat(SHAPE_CACHE_UNITS);
        let before = t.cache_stats().misses;
        t.measure(s, 0.0, &huge).unwrap();
        t.measure(s, 0.0, &huge).unwrap();
        assert_eq!(t.cache_stats().misses, before + 2);
        assert!(t.cache.units() <= 2 * SHAPE_CACHE_UNITS);
    }
}
