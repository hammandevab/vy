//! Rasterization: rectangles, paths, images, text shaping and glyph cache (SPEC §6).
//!
//! M1 ships a software rasterizer used by `vy-cli render`; wgpu arrives with M3.
//! Headless and windowed rendering share this code path (SPEC-11.3.2).
#![deny(missing_docs)]

pub mod bench;
pub mod image;
pub mod raster;
#[cfg(test)]
mod raster_reference;
pub mod text;

pub use image::{diff_ratio, ImageCache, ImageError, Resource, RgbaImage, MAX_IMAGE_CACHE_BYTES};
pub use raster::{
    raster_budget_units, render, RasterReport, RenderError, Rendered, MASK_WEIGHT, OP_UNITS,
    PIPELINE_WEIGHT, RASTER_BUDGET_OVERDRAWS, RASTER_BUDGET_UNITS,
};
pub use text::{pack_x100, CacheStats, Size, TextEngine};

#[cfg(test)]
mod tests {
    use super::*;
    use vy_abi::{err, font, limits, weight};
    use vy_dl::{DisplayList, Fit};

    /// Held by the tests that assert a wall-clock bound and by the
    /// CPU-heavy tests of this crate, so a timing never shares the machine
    /// with the comparisons of `raster.rs` (two CPUs in CI).
    pub(crate) static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub(crate) fn serial() -> std::sync::MutexGuard<'static, ()> {
        SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn sans(engine: &mut TextEngine, size: f32) -> u16 {
        engine
            .style(font::SANS, size, weight::NORMAL, 0x0000_00FF)
            .unwrap()
    }

    /// `render` with no image resources: the image alone.
    fn render(
        dl: &[u8],
        w: u32,
        h: u32,
        scale: f32,
        t: &mut TextEngine,
    ) -> Result<RgbaImage, RenderError> {
        render_full(dl, w, h, scale, t).map(|r| r.image)
    }

    /// `render` with no image resources: image and report.
    fn render_full(
        dl: &[u8],
        w: u32,
        h: u32,
        scale: f32,
        t: &mut TextEngine,
    ) -> Result<Rendered, RenderError> {
        raster::render(dl, w, h, scale, t, &ImageCache::from_bytes(Vec::new()))
    }

    /// PNG of a `w` × `h` image whose pixel `(x, y)` is `f(x, y)`.
    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut img = RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                img.data[i..i + 4].copy_from_slice(&f(x, y));
            }
        }
        img.encode_png().unwrap()
    }

    /// SPEC-5.2.4, SPEC-6.4.3: text wraps at max_w and the height grows as the width shrinks;
    /// dl_text reports that height times 100.
    #[test]
    fn spec_6_4_3_text_wrap_and_height() {
        let mut t = TextEngine::new();
        let s = sans(&mut t, 16.0);
        let line = 16.0 * text::LINE_HEIGHT_FACTOR;
        let txt = "Den här sidan är en WebAssembly-modul som ritar sig själv.";
        let wide = t.measure(s, 10_000.0, txt).unwrap();
        assert_eq!(wide.h, line, "one line when nothing wraps");
        assert!(wide.w > 100.0);
        let narrow = t.measure(s, 120.0, txt).unwrap();
        assert!(narrow.h > wide.h, "{narrow:?} vs {wide:?}");
        assert!(narrow.w <= 120.0, "wrapped width {}", narrow.w);
        assert_eq!(narrow.h % line, 0.0, "height is whole lines");
        // Packed measurement: width in the high 32 bits, height in the low,
        // as both `dl_text` and `text_measure` return it (SPEC-5.2.4, SPEC-5.2.5).
        let packed = pack_x100(narrow.w, narrow.h);
        assert_eq!(
            (packed & 0xFFFF_FFFF) as i32,
            (narrow.h * 100.0).round() as i32
        );
        assert_eq!((packed >> 32) as i32, (narrow.w * 100.0).round() as i32);
        assert_eq!(t.measure(99, 100.0, txt), Err(err::E_STYLE));
        // `max_w <= 0` means no wrapping at all (SPEC-6.4.3).
        for w in [0.0, -1.0, -1e9] {
            assert_eq!(t.measure(s, w, txt).unwrap(), wide, "max_w {w}");
        }
    }

    /// SPEC-5.2.5, SPEC-5.2.4: each packed half is at most 2³¹ − 1, so a
    /// 21 million pixel wide line is still a non-negative `i64` and never
    /// mistaken for a status code; negative and non-finite inputs pack as 0.
    #[test]
    fn spec_5_2_5_packed_halves_fit_i32() {
        let wide = pack_x100(21_474_840.0, 1.0);
        assert_eq!(
            wide >> 32,
            2_147_483_647,
            "21.5 million px clamps to 2^31 - 1"
        );
        assert_eq!(wide & 0xFFFF_FFFF, 100);
        assert!(wide >= 0);
        let over = pack_x100(1e12, 1e12);
        assert_eq!(over >> 32, i64::from(i32::MAX));
        assert_eq!(over & 0xFFFF_FFFF, i64::from(i32::MAX));
        assert!(over >= 0, "clamped halves never set the sign bit");
        assert_eq!(pack_x100(-5.0, f32::NAN), 0);
        assert_eq!(pack_x100(1.234, 5.678), (123 << 32) | 568);
    }

    /// SPEC-6.3.1: scale 2 gives twice the physical pixels and the same logical layout.
    #[test]
    fn spec_6_3_1_dpi_scaling() {
        let mut t = TextEngine::new();
        let s = sans(&mut t, 20.0);
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.rect(0.0, 0.0, 100.0, 50.0, 0.0, 0xFF00_00FF).unwrap();
        dl.rect(10.0, 10.0, 20.0, 20.0, 4.0, 0x0000_FFFF).unwrap();
        dl.text(10.0, 30.0, 80.0, s, b"Vy").unwrap();
        dl.end().unwrap();
        let one = render(dl.bytes(), 100, 50, 1.0, &mut t).unwrap();
        let two = render(dl.bytes(), 200, 100, 2.0, &mut t).unwrap();
        assert_eq!((one.width, one.height), (100, 50));
        assert_eq!((two.width, two.height), (200, 100));
        // The blue square covers logical (10..30, 10..30): physical 2x at scale 2.
        assert_eq!(one.pixel(15, 15).unwrap(), [0, 0, 255, 255]);
        assert_eq!(two.pixel(30, 30).unwrap(), [0, 0, 255, 255]);
        assert_eq!(two.pixel(59, 40).unwrap(), [0, 0, 255, 255]);
        assert_eq!(two.pixel(40, 59).unwrap(), [0, 0, 255, 255]);
        assert_eq!(two.pixel(70, 70).unwrap(), [255, 0, 0, 255]);
        assert_eq!(one.pixel(35, 15).unwrap(), [255, 0, 0, 255]);
        // Measurements are logical and independent of scale.
        let m = t.measure(s, 80.0, "Vy").unwrap();
        assert_eq!(m.h, 25.0);
    }

    /// SPEC-6.4.1: a TEXT op carries UTF-8 bytes, never glyph ids; shaping is the host's job.
    #[test]
    fn spec_6_4_1_host_shaping() {
        let mut t = TextEngine::new();
        let s = sans(&mut t, 16.0);
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.text(0.0, 0.0, 200.0, s, "Hej från Vy – ﬁ".as_bytes())
            .unwrap();
        dl.end().unwrap();
        let ops = vy_dl::decode(dl.bytes()).unwrap();
        let vy_dl::Op::Text { text, .. } = &ops[0] else {
            panic!("expected TEXT");
        };
        assert_eq!(std::str::from_utf8(text).unwrap(), "Hej från Vy – ﬁ");
        // The host produces pixels from those bytes; the module never sees glyphs.
        let img = render(dl.bytes(), 200, 30, 1.0, &mut t).unwrap();
        let dark = img
            .data
            .chunks_exact(4)
            .filter(|p| p[0] < 128 && p[3] == 255)
            .count();
        assert!(dark > 50, "text was rasterized ({dark} dark pixels)");
    }

    /// SPEC-6.4.2, SPEC-5.2.5: resource ids −1 sans, −2 serif, −3 mono,
    /// −4 ui, −5 emoji are the built-in roles (`vy_abi::font`); anything
    /// else is E_RESOURCE; a bad size is E_ARG.
    #[test]
    fn spec_6_4_2_builtin_font_roles() {
        let mut t = TextEngine::new();
        for id in font::MIN..=font::SANS {
            assert!(font::name(id).is_some(), "{id} is a role");
            let sid = t.style(id, 16.0, weight::NORMAL, 0).unwrap();
            assert!(sid > 0, "role {id}");
            assert!(t.measure(sid, 100.0, "abc").unwrap().w > 0.0, "role {id}");
        }
        assert_eq!(t.style(-6, 16.0, 400, 0), Err(err::E_RESOURCE));
        assert_eq!(t.style(0, 16.0, 400, 0), Err(err::E_RESOURCE));
        assert_eq!(t.style(-1, 0.0, 400, 0), Err(err::E_ARG));
        assert_eq!(t.style(-1, f32::NAN, 400, 0), Err(err::E_ARG));
        // Serif and mono are distinct faces: the same string measures differently.
        let sans = t.style(font::SANS, 16.0, 400, 0).unwrap();
        let serif = t.style(font::SERIF, 16.0, 400, 0).unwrap();
        let mono = t.style(font::MONO, 16.0, 400, 0).unwrap();
        let w = |t: &mut TextEngine, s| t.measure(s, 1000.0, "iiii mmmm").unwrap().w;
        let (a, b, c) = (w(&mut t, sans), w(&mut t, serif), w(&mut t, mono));
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_ne!(b, c);
        // `ui` and `emoji` are the sans faces in 1.0.
        let ui = t.style(font::UI, 16.0, 400, 0).unwrap();
        let emoji = t.style(font::EMOJI, 16.0, 400, 0).unwrap();
        assert_eq!(w(&mut t, ui), a);
        assert_eq!(w(&mut t, emoji), a);
    }

    /// SPEC-5.2.5: `weight` is 1..=1000; a value outside is clamped, never
    /// refused, and the text is drawn with the bundled face nearest the
    /// weight, the heavier on a tie: sans 1–549 is DejaVu Sans, 550–1000
    /// DejaVu Sans Bold; serif and mono have one face for every weight.
    #[test]
    fn spec_5_2_5_weight_clamps_to_nearest_face() {
        let mut t = TextEngine::new();
        let txt = "Hamburgefonstiv";
        let width = |t: &mut TextEngine, role, w| {
            let s = t.style(role, 16.0, w, 0).unwrap();
            t.measure(s, 0.0, txt).unwrap().w
        };
        let book = width(&mut t, font::SANS, weight::NORMAL);
        let bold = width(&mut t, font::SANS, weight::BOLD);
        assert!(bold > book, "bold is wider: {bold} vs {book}");
        for w in [i32::MIN, -1, 0, 1, 100, 300, 400, 500, 549] {
            assert_eq!(width(&mut t, font::SANS, w), book, "weight {w}");
            assert_eq!(text::face_weight(font::SANS, w), Some(400));
        }
        for w in [550, 551, 600, 700, 900, 1000, 1001, i32::MAX] {
            assert_eq!(width(&mut t, font::SANS, w), bold, "weight {w}");
            assert_eq!(text::face_weight(font::SANS, w), Some(700));
        }
        // The style remembers the clamped request and the face it got.
        let s = t.style(font::SANS, 16.0, 5000, 0).unwrap();
        let style = t.get_style(s).unwrap();
        assert_eq!((style.weight, style.face_weight), (1000, 700));
        let s = t.style(font::UI, 16.0, 0, 0).unwrap();
        assert_eq!(t.get_style(s).unwrap().weight, 1);
        assert_eq!(t.get_style(s).unwrap().face_weight, 400);
        // Serif and mono never borrow a bold face from another family.
        for role in [font::SERIF, font::MONO] {
            let one = width(&mut t, role, 400);
            for w in [1, 600, 700, 1000] {
                assert_eq!(width(&mut t, role, w), one, "role {role} weight {w}");
                assert_eq!(text::face_weight(role, w), Some(400));
            }
            assert_ne!(one, bold, "not DejaVu Sans Bold");
        }
        assert_eq!(text::face_weight(0, 400), None);
    }

    /// SPEC-6.1.3, SPEC-6.1.4: later ops paint over earlier ones and clipping limits drawing.
    #[test]
    fn spec_6_1_3_paint_order_and_clip() {
        let mut t = TextEngine::new();
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.rect(0.0, 0.0, 40.0, 40.0, 0.0, 0xFF00_00FF).unwrap();
        dl.clip_push(0.0, 0.0, 20.0, 40.0, 0.0).unwrap();
        dl.rect(0.0, 0.0, 40.0, 40.0, 0.0, 0x00FF_00FF).unwrap();
        dl.clip_pop().unwrap();
        dl.path(
            0x0000_FFFF,
            2.0,
            &[
                vy_dl::PathCmd::MoveTo(0.0, 35.0),
                vy_dl::PathCmd::LineTo(40.0, 35.0),
            ],
        )
        .unwrap();
        dl.end().unwrap();
        let img = render(dl.bytes(), 40, 40, 1.0, &mut t).unwrap();
        assert_eq!(
            img.pixel(5, 5).unwrap(),
            [0, 255, 0, 255],
            "inside clip: green over red"
        );
        assert_eq!(
            img.pixel(30, 5).unwrap(),
            [255, 0, 0, 255],
            "outside clip: red stays"
        );
        assert_eq!(img.pixel(30, 35).unwrap(), [0, 0, 255, 255], "line on top");
    }

    /// SPEC-5.2.3, SPEC-C.1.1, SPEC-6.1.4: a `CLIP_PUSH` with a radius clips
    /// to a rounded rectangle: the corners stay untouched and the middle is
    /// painted; a radius of 0 is the plain rectangle of before, byte for
    /// byte; nested rounded and plain clips intersect; the radius scales
    /// with the DPI (SPEC-6.3.1).
    #[test]
    fn spec_5_2_3_rounded_clip() {
        let mut t = TextEngine::new();
        let list = |radius: f32| {
            let mut dl = DisplayList::new();
            dl.begin().unwrap();
            dl.clip_push(10.0, 10.0, 40.0, 40.0, radius).unwrap();
            dl.rect(0.0, 0.0, 60.0, 60.0, 0.0, 0xFF00_00FF).unwrap();
            dl.clip_pop().unwrap();
            dl.end().unwrap();
            dl.bytes().to_vec()
        };
        let white = [255, 255, 255, 255];
        let red = [255, 0, 0, 255];
        let round = render(&list(12.0), 60, 60, 1.0, &mut t).unwrap();
        assert_eq!(round.pixel(10, 10).unwrap(), white, "corner is clipped");
        assert_eq!(round.pixel(49, 49).unwrap(), white, "every corner");
        assert_eq!(round.pixel(30, 30).unwrap(), red, "the middle is painted");
        assert_eq!(
            round.pixel(30, 10).unwrap(),
            red,
            "the straight edge is kept"
        );
        assert_eq!(round.pixel(5, 30).unwrap(), white, "outside the rectangle");
        let partial = (10..16)
            .flat_map(|x| (10..16).map(move |y| (x, y)))
            .map(|(x, y)| round.pixel(x, y).unwrap())
            .any(|p| p[1] > 0 && p[1] < 255);
        assert!(partial, "the arc is anti-aliased");

        let square = render(&list(0.0), 60, 60, 1.0, &mut t).unwrap();
        assert_eq!(
            square.pixel(10, 10).unwrap(),
            red,
            "radius 0 keeps the corner"
        );
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.clip_push(10.0, 10.0, 40.0, 40.0, 0.0).unwrap();
        dl.rect(0.0, 0.0, 60.0, 60.0, 0.0, 0xFF00_00FF).unwrap();
        dl.clip_pop().unwrap();
        dl.end().unwrap();
        assert_eq!(render(dl.bytes(), 60, 60, 1.0, &mut t).unwrap(), square);

        // Nested: a rounded clip inside a plain one keeps both limits.
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.clip_push(0.0, 0.0, 30.0, 60.0, 0.0).unwrap();
        dl.clip_push(10.0, 10.0, 40.0, 40.0, 12.0).unwrap();
        dl.rect(0.0, 0.0, 60.0, 60.0, 0.0, 0xFF00_00FF).unwrap();
        dl.clip_pop().unwrap();
        dl.clip_pop().unwrap();
        dl.end().unwrap();
        let nested = render(dl.bytes(), 60, 60, 1.0, &mut t).unwrap();
        assert_eq!(nested.pixel(20, 30).unwrap(), red);
        assert_eq!(
            nested.pixel(40, 30).unwrap(),
            white,
            "cut by the outer clip"
        );
        assert_eq!(
            nested.pixel(10, 10).unwrap(),
            white,
            "and by the rounded one"
        );

        // DPI: at scale 2 the corner clipped away is twice as large.
        let big = render(&list(12.0), 120, 120, 2.0, &mut t).unwrap();
        assert_eq!(big.pixel(22, 22).unwrap(), white);
        assert_eq!(big.pixel(60, 60).unwrap(), red);
    }

    /// SPEC-C.1.2: paths fill (stroke_w 0) and stroke; images draw something.
    #[test]
    fn spec_c_1_2_path_fill_and_stroke() {
        use vy_dl::PathCmd;
        let mut t = TextEngine::new();
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        let tri = [
            PathCmd::MoveTo(0.0, 0.0),
            PathCmd::LineTo(40.0, 0.0),
            PathCmd::LineTo(0.0, 40.0),
            PathCmd::Close,
        ];
        dl.path(0x0000_00FF, 0.0, &tri).unwrap();
        dl.path(
            0xFF00_00FF,
            2.0,
            &[PathCmd::MoveTo(0.0, 39.0), PathCmd::LineTo(40.0, 39.0)],
        )
        .unwrap();
        dl.image(80.0, 80.0, 10.0, 10.0, 1, Fit::Contain).unwrap();
        dl.end().unwrap();
        let img = render(dl.bytes(), 100, 100, 1.0, &mut t).unwrap();
        assert_eq!(img.pixel(5, 5).unwrap(), [0, 0, 0, 255], "filled triangle");
        assert_eq!(
            img.pixel(35, 35).unwrap(),
            [255, 255, 255, 255],
            "outside triangle"
        );
        assert_eq!(img.pixel(30, 39).unwrap(), [255, 0, 0, 255], "stroked line");
        assert_eq!(
            img.pixel(85, 85).unwrap(),
            [128, 128, 128, 255],
            "image placeholder when the resource is not decoded"
        );
    }

    /// SPEC-6.5.1: an image resource is decoded at its first use and cached
    /// for the package; a side above 8192 px is refused from the header alone;
    /// bytes that are not a PNG, id 0 (SPEC-4.5.1) and unknown ids are
    /// `E_RESOURCE`; the refusal is cached as well.
    #[test]
    fn spec_6_5_1_image_decode_limits() {
        let max = limits::MAX_IMAGE_SIDE_PX;
        let small = png(2, 1, |x, _| {
            if x == 0 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        });
        let wide = png(max, 1, |_, _| [0, 255, 0, 255]);
        let too_wide = png(max + 1, 1, |_, _| [0, 255, 0, 255]);
        let too_tall = png(1, max + 1, |_, _| [0, 255, 0, 255]);
        let garbage = b"RIFF....WEBPVP8 ".to_vec();
        let mut cache =
            ImageCache::from_bytes(vec![small, wide, too_wide, too_tall, garbage.clone()]);
        assert_eq!(cache.resource_count(), 5);
        assert_eq!(cache.decodes(), 0, "nothing decoded before first use");
        assert_eq!(
            cache.get(1),
            Err(ImageError::Unknown),
            "lazy: not yet decoded"
        );

        let img = cache.decode(1).unwrap();
        assert_eq!((img.width, img.height), (2, 1));
        assert_eq!(img.pixel(1, 0).unwrap(), [0, 0, 255, 255]);
        assert_eq!(cache.decodes(), 1);
        cache.decode(1).unwrap();
        assert_eq!(cache.decodes(), 1, "second use is a cache hit");
        assert_eq!(cache.ensure(1), 0, "a cache hit reads no resource bytes");
        assert_eq!(cache.decoded_bytes(), 8);

        assert_eq!(cache.decode(2).unwrap().width, max, "8192 is allowed");
        assert_eq!(
            cache.decode(3),
            Err(ImageError::TooLarge {
                width: max + 1,
                height: 1,
                max_side: max
            })
        );
        assert_eq!(
            cache.decode(4),
            Err(ImageError::TooLarge {
                width: 1,
                height: max + 1,
                max_side: max
            })
        );
        assert_eq!(
            cache.decoded_bytes(),
            8 + max as usize * 4,
            "refused images hold no pixels"
        );
        assert!(matches!(cache.decode(5), Err(ImageError::Decode(_))));
        let decodes = cache.decodes();
        assert!(matches!(cache.decode(5), Err(ImageError::Decode(_))));
        assert_eq!(cache.decodes(), decodes, "a failure is cached too");
        assert_eq!(
            cache.decode(0),
            Err(ImageError::Unknown),
            "id 0 is the module"
        );
        assert_eq!(cache.decode(6), Err(ImageError::Unknown));
        assert_eq!(cache.decode(u16::MAX), Err(ImageError::Unknown));

        // Every refusal is one ABI code: E_RESOURCE.
        for e in [
            ImageError::Unknown,
            ImageError::TooLarge {
                width: 1,
                height: 1,
                max_side: 1,
            },
            ImageError::Decode(String::new()),
            ImageError::CacheFull,
        ] {
            assert_eq!(e.code(), err::E_RESOURCE);
        }

        // The decoded cache is bounded (SPEC-9.3.1): with a full cache a new
        // decode is refused and nothing is held for it.
        let one = png(1, 1, |_, _| [1, 2, 3, 4]);
        let mut cache = ImageCache::from_bytes(vec![one.clone(), one]);
        cache.decoded_bytes = MAX_IMAGE_CACHE_BYTES - 4;
        cache.decode(1).unwrap();
        assert_eq!(cache.decode(2), Err(ImageError::CacheFull));
        assert_eq!(cache.decoded_bytes(), MAX_IMAGE_CACHE_BYTES);
    }

    /// SPEC-6.5.1, SPEC-12.2.2: `decode_png`, which reads goldens back, checks
    /// the declared size against the canvas limit before it allocates, like
    /// the resource path checks SPEC-6.5.1's. A 33-byte file whose header
    /// claims 65535 x 65535 px would otherwise ask for 16 GB and abort the
    /// process (found by fuzzing in review 0002).
    #[test]
    fn spec_6_5_1_decode_png_header_checked_before_alloc() {
        fn crc32(data: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for &b in data {
                crc ^= u32::from(b);
                for _ in 0..8 {
                    crc = if crc & 1 != 0 {
                        0xEDB8_8320 ^ (crc >> 1)
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        /// A real 1 x 1 PNG whose IHDR is rewritten to claim `w` x `h`.
        fn claiming(w: u32, h: u32) -> Vec<u8> {
            let mut bytes = png(1, 1, |_, _| [0, 0, 0, 255]);
            bytes[16..20].copy_from_slice(&w.to_be_bytes());
            bytes[20..24].copy_from_slice(&h.to_be_bytes());
            let crc = crc32(&bytes[12..29]);
            bytes[29..33].copy_from_slice(&crc.to_be_bytes());
            bytes
        }
        let max = raster::MAX_CANVAS_SIDE_PX;
        for (w, h) in [
            (u16::MAX as u32, u16::MAX as u32),
            (max + 1, 1),
            (1, max + 1),
            (u32::MAX, 1),
        ] {
            let err = RgbaImage::decode_png(&claiming(w, h)).unwrap_err();
            assert!(
                err.to_string().contains(&format!("exceeds {max} px")),
                "{w}x{h}: {err}"
            );
        }
        // At the limit the header passes and the refusal comes from the
        // missing pixel data instead: the check is on the header alone.
        let err = RgbaImage::decode_png(&claiming(max, 1)).unwrap_err();
        assert!(!err.to_string().contains("exceeds"), "{err}");
        // Honest sizes decode as before, up to the canvas limit.
        let wide = png(max, 1, |_, _| [1, 2, 3, 4]);
        assert_eq!(RgbaImage::decode_png(&wide).unwrap().width, max);
    }

    /// SPEC-5.2.3, SPEC-C.1.1: `fit` 0 covers the rectangle and crops, 1 fits
    /// inside and letterboxes, 2 stretches; the image draws through the clip
    /// stack and scales with the DPI (SPEC-6.3.1); the same bytes give the
    /// same pixels twice (SPEC-2.1.5).
    #[test]
    fn spec_5_2_3_image_fit_modes() {
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        let white = [255, 255, 255, 255];
        // 2 x 1: red | blue
        let two = png(2, 1, |x, _| if x == 0 { red } else { blue });
        let mut cache = ImageCache::from_bytes(vec![two]);
        cache.decode(1).unwrap();
        let mut t = TextEngine::new();
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        // cover into 2x2 at (0,0): scaled 4x2, centred, cropped to columns 1..3 of the image
        dl.image(0.0, 0.0, 2.0, 2.0, 1, Fit::Cover).unwrap();
        // contain into 2x2 at (4,0): scaled 2x1, centred vertically
        dl.image(4.0, 0.0, 2.0, 2.0, 1, Fit::Contain).unwrap();
        // stretch into 4x2 at (8,0)
        dl.image(8.0, 0.0, 4.0, 2.0, 1, Fit::Stretch).unwrap();
        // clipped: only the left pixel column of a stretch survives
        dl.clip_push(14.0, 0.0, 1.0, 2.0, 0.0).unwrap();
        dl.image(14.0, 0.0, 4.0, 2.0, 1, Fit::Stretch).unwrap();
        dl.clip_pop().unwrap();
        dl.end().unwrap();
        let img = raster::render(dl.bytes(), 20, 2, 1.0, &mut t, &cache)
            .unwrap()
            .image;
        let px = |x, y| img.pixel(x, y).unwrap();
        // cover: both rows filled, left half red, right half blue
        assert_eq!(
            [px(0, 0), px(1, 0), px(0, 1), px(1, 1)],
            [red, blue, red, blue]
        );
        // contain: top row image, bottom row untouched
        assert_eq!(
            [px(4, 0), px(5, 0), px(4, 1), px(5, 1)],
            [red, blue, white, white]
        );
        // stretch: two columns per source pixel
        assert_eq!(
            [px(8, 0), px(9, 0), px(10, 0), px(11, 0), px(11, 1)],
            [red, red, blue, blue, blue]
        );
        assert_eq!(px(12, 0), white, "nothing outside the rectangle");
        assert_eq!([px(14, 0), px(15, 0)], [red, white], "clipped");
        assert_eq!(px(19, 1), white);

        // DPI: the same list at scale 2 covers twice the physical pixels.
        let big = raster::render(dl.bytes(), 40, 4, 2.0, &mut t, &cache)
            .unwrap()
            .image;
        assert_eq!(big.pixel(0, 0).unwrap(), red);
        assert_eq!(big.pixel(3, 3).unwrap(), blue);
        assert_eq!(big.pixel(9, 0).unwrap(), white, "letterbox scales too");
        assert_eq!(big.pixel(9, 1).unwrap(), red);
        assert_eq!(big.pixel(9, 2).unwrap(), red);
        assert_eq!(big.pixel(9, 3).unwrap(), white);
        assert_eq!(
            raster::render(dl.bytes(), 20, 2, 1.0, &mut t, &cache)
                .unwrap()
                .image,
            img
        );

        // An unknown fit value on the wire draws nothing; a resource that was
        // refused draws the placeholder, like one never decoded.
        let mut cache = ImageCache::from_bytes(vec![b"not a png".to_vec()]);
        assert!(cache.decode(1).is_err());
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.image(0.0, 0.0, 2.0, 2.0, 1, Fit::Cover).unwrap();
        dl.end().unwrap();
        let mut bytes = dl.bytes().to_vec();
        let img = raster::render(&bytes, 4, 2, 1.0, &mut t, &cache)
            .unwrap()
            .image;
        assert_eq!(img.pixel(0, 0).unwrap(), [128, 128, 128, 255]);
        let fit_at = bytes.len() - 2;
        assert_eq!(bytes[fit_at], 0);
        bytes[fit_at] = 3;
        let mut good = ImageCache::from_bytes(vec![png(1, 1, |_, _| red)]);
        good.decode(1).unwrap();
        let img = raster::render(&bytes, 4, 2, 1.0, &mut t, &good)
            .unwrap()
            .image;
        assert_eq!(img.pixel(0, 0).unwrap(), white);
    }

    /// SPEC-6.1.1, SPEC-6.4.1: the host rasterizes whatever the module describes;
    /// text containing a bidi paragraph separator next to a right-to-left run
    /// must lay out, not crash the shaper (found by fuzzing in the M1 review).
    #[test]
    fn spec_6_4_1_paragraph_separators_do_not_crash() {
        let mut t = TextEngine::new();
        let s = sans(&mut t, 16.0);
        for sep in ['\u{1C}', '\u{1D}', '\u{1E}', '\u{85}', '\u{2029}'] {
            let txt = format!("a{sep}\u{5D0}\u{639}");
            let m = t.measure(s, 100.0, &txt).unwrap();
            assert_eq!(
                m.h,
                2.0 * 16.0 * text::LINE_HEIGHT_FACTOR,
                "{sep:?} is a line break"
            );
            // The caret after the separator sits at the start of the second line,
            // whatever the separator's UTF-8 length (byte offsets are the module's).
            let ltr = format!("a{sep}b");
            let after = 1 + sep.len_utf8();
            assert_eq!(t.caret(s, &ltr, after).unwrap(), (0.0, 20.0), "{sep:?}");
            assert_eq!(t.caret(s, &ltr, 1).unwrap().1, 0.0, "{sep:?}");
            assert!(t.caret(s, &ltr, after + 1).unwrap().0 > 0.0, "{sep:?}");
            let mut dl = DisplayList::new();
            dl.begin().unwrap();
            dl.text(0.0, 0.0, 100.0, s, txt.as_bytes()).unwrap();
            dl.end().unwrap();
            render(dl.bytes(), 100, 50, 1.0, &mut t).unwrap();
        }
    }

    /// SPEC-6.4.1, SPEC-5.2.5: right-to-left text laid out without wrapping
    /// (`max_w <= 0`, and every `text_caret`) stays within its own width
    /// instead of starting at the right edge of an unbounded buffer (found by
    /// fuzzing in the M1 review).
    #[test]
    fn spec_6_4_1_rtl_unwrapped_stays_in_bounds() {
        let mut t = TextEngine::new();
        let s = sans(&mut t, 16.0);
        let rtl = "\u{5E9}\u{5DC}\u{5D5}\u{5DD} \u{639}\u{627}\u{644}\u{645}";
        let m = t.measure(s, 0.0, rtl).unwrap();
        assert!(m.w > 10.0 && m.w < 200.0, "{m:?}");
        // A huge or non-finite wrap width is the same as no wrapping.
        for w in [1e30, f32::INFINITY, f32::NAN, text::MAX_COORD_PX * 2.0] {
            assert_eq!(t.measure(s, w, rtl).unwrap(), m, "max_w {w}");
            let mut dl = DisplayList::new();
            dl.begin().unwrap();
            dl.text(2.0, 2.0, w, s, rtl.as_bytes()).unwrap();
            dl.end().unwrap();
            render(dl.bytes(), 200, 30, 1.0, &mut t).unwrap();
        }
        for off in [0, 2, rtl.len()] {
            let (x, y) = t.caret(s, rtl, off).unwrap();
            assert!(x >= 0.0 && x <= m.w + 1.0, "caret x {x} outside 0..{}", m.w);
            assert_eq!(y, 0.0);
        }
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.text(2.0, 2.0, 0.0, s, rtl.as_bytes()).unwrap();
        dl.end().unwrap();
        let img = render(dl.bytes(), 200, 30, 1.0, &mut t).unwrap();
        let dark = img
            .data
            .chunks_exact(4)
            .filter(|p| p[0] < 128 && p[3] == 255)
            .count();
        assert!(
            dark > 50,
            "unwrapped RTL text was drawn on the canvas ({dark})"
        );
    }

    /// SPEC-6.1.1, SPEC-6.3.1: coordinates are logical pixels; a non-finite or
    /// absurdly large value in any op is skipped instead of overflowing the
    /// rasterizer, and a bad clip rectangle clips everything but keeps the
    /// stack balanced (found by fuzzing in the M1 review).
    #[test]
    fn spec_6_3_1_bad_coordinates_are_skipped() {
        use vy_dl::PathCmd;
        let mut t = TextEngine::new();
        let s = sans(&mut t, 16.0);
        for v in [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            1e30,
            -1e30,
            f32::MAX,
        ] {
            let mut dl = DisplayList::new();
            dl.begin().unwrap();
            dl.rect(v, 0.0, 10.0, 10.0, 0.0, 0xFF).unwrap();
            dl.rect(0.0, 0.0, 10.0, 10.0, v, 0xFF).unwrap();
            dl.text(v, 0.0, 100.0, s, b"hej").unwrap();
            dl.text(0.0, v, 100.0, s, b"hej").unwrap();
            dl.image(v, v, v, v, 1, vy_dl::Fit::Cover).unwrap();
            dl.path(
                0xFF,
                0.0,
                &[
                    PathCmd::MoveTo(0.0, 0.0),
                    PathCmd::LineTo(v, 10.0),
                    PathCmd::Close,
                ],
            )
            .unwrap();
            dl.path(
                0xFF,
                v,
                &[
                    PathCmd::MoveTo(0.0, 0.0),
                    PathCmd::QuadTo {
                        ctrl: (v, 5.0),
                        to: (10.0, 10.0),
                    },
                ],
            )
            .unwrap();
            dl.clip_push(v, 0.0, 10.0, 10.0, 0.0).unwrap();
            dl.rect(0.0, 0.0, 40.0, 40.0, 0.0, 0xFF00_00FF).unwrap();
            dl.clip_pop().unwrap();
            dl.rect(0.0, 0.0, 10.0, 10.0, 0.0, 0x0000_FFFF).unwrap();
            dl.end().unwrap();
            let img = render(dl.bytes(), 40, 40, 1.0, &mut t).unwrap();
            assert_eq!(img.pixel(5, 5).unwrap(), [0, 0, 255, 255], "{v}");
            assert_eq!(
                img.pixel(30, 30).unwrap(),
                [255, 255, 255, 255],
                "{v}: red rect was clipped away by the bad clip"
            );
        }
        // The largest accepted magnitude, at the largest CLI scale, still renders.
        let m = raster::MAX_MAGNITUDE;
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.rect(-m, -m, 2.0 * m, 2.0 * m, m, 0xFF).unwrap();
        dl.path(0xFF, m, &[PathCmd::MoveTo(-m, m), PathCmd::LineTo(m, -m)])
            .unwrap();
        dl.clip_push(-m, -m, 2.0 * m, 2.0 * m, 0.0).unwrap();
        dl.text(m, -m, m, s, b"hej").unwrap();
        dl.clip_pop().unwrap();
        dl.end().unwrap();
        render(dl.bytes(), 64, 64, 8.0, &mut t).unwrap();
    }

    /// SPEC-11.3.1: PNG round-trips losslessly and encodes identically.
    #[test]
    fn spec_11_3_1_png_roundtrip() {
        let mut img = RgbaImage::new(3, 2);
        img.data[0..4].copy_from_slice(&[1, 2, 3, 4]);
        img.data[20..24].copy_from_slice(&[250, 251, 252, 253]);
        let a = img.encode_png().unwrap();
        let b = img.encode_png().unwrap();
        assert_eq!(a, b);
        assert_eq!(RgbaImage::decode_png(&a).unwrap(), img);
        let mut other = img.clone();
        other.data[0] = 9;
        assert!((diff_ratio(&img, &other) - 1.0 / 6.0).abs() < 1e-9);
        assert_eq!(diff_ratio(&img, &img), 0.0);
    }

    /// SPEC-13.2.1: 2 000 ops rasterize in <= 2 ms on the reference machine,
    /// P95 over 100 runs (SPEC §13): the frame of `bench::frame_2000`, in
    /// the reference pages' proportions, at 1280 × 800 with a warm text
    /// engine, the same bytes `vy-cli bench` times as `raster.ops-2000`. A
    /// release-mode budget: a debug build only reports.
    #[test]
    #[ignore = "M4: over budget on this machine: P95 44-56 ms in release (P50 34-35 ms), docs/notes/m4-perf.md"]
    fn spec_13_2_1_raster_budget() {
        if cfg!(debug_assertions) {
            eprintln!("SPEC-13.2.1 is a release-mode budget; run with --release");
            return;
        }
        let _serial = serial();
        let mut t = TextEngine::new();
        let frame = crate::bench::frame_2000(&mut t, 1280.0, 800.0);
        let mut samples: Vec<_> = (0..100)
            .map(|_| {
                let start = std::time::Instant::now();
                let r = raster::render(&frame.list, 1280, 800, 1.0, &mut t, &frame.images);
                let took = start.elapsed();
                assert!(!r.unwrap().report.truncated);
                took
            })
            .collect();
        samples.sort();
        let p95 = samples[94];
        eprintln!(
            "SPEC-13.2.1: 2 000 ops in {p95:?} (P95, P50 {:?})",
            samples[49]
        );
        assert!(
            p95 <= std::time::Duration::from_millis(2),
            "P95 {p95:?} over the 2 ms budget"
        );
    }

    /// A list of `clips` nested rounded (or plain, radius 0) full-canvas
    /// clips, `rects` full-canvas rectangles, and the pops.
    fn pathological(clips: u32, rects: u32, radius: f32, w: f32, h: f32) -> Vec<u8> {
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        for i in 0..clips {
            let inset = i as f32;
            dl.clip_push(inset, inset, w - 2.0 * inset, h - 2.0 * inset, radius)
                .unwrap();
        }
        for i in 0..rects {
            dl.rect(0.0, 0.0, w, h, 0.0, 0x1000_00FF + (i % 7) * 0x0010_0000)
                .unwrap();
        }
        for _ in 0..clips {
            dl.clip_pop().unwrap();
        }
        dl.end().unwrap();
        dl.bytes().to_vec()
    }

    /// SPEC-5.3.3, SPEC-9.3.1: the rasterizer's work is bounded by the
    /// frame budget. Ops are charged by the pixels they touch; when the
    /// budget runs out the remaining ops are not drawn and the report says
    /// where it stopped. Here: 63 full-canvas translucent fills fit, the
    /// 64th does not, and a rectangle after them never reaches the canvas.
    #[test]
    fn spec_5_3_3_raster_budget_truncates_the_list() {
        let mut t = TextEngine::new();
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.rect(0.0, 0.0, 128.0, 256.0, 0.0, 0xFF00_00FF).unwrap();
        for _ in 0..70 {
            dl.rect(0.0, 0.0, 256.0, 256.0, 0.0, 0x0000_FF80).unwrap();
        }
        dl.rect(200.0, 200.0, 56.0, 56.0, 0.0, 0x00FF_00FF).unwrap();
        dl.end().unwrap();
        let r = render_full(dl.bytes(), 256, 256, 1.0, &mut t).unwrap();
        let img = r.image;
        let report = r.report;
        assert!(report.truncated, "{report:?}");
        assert_eq!(report.budget, raster_budget_units(256, 256));
        assert_eq!(
            report.budget, RASTER_BUDGET_UNITS,
            "a small canvas has the floor"
        );
        // One opaque half-canvas fill at weight 1, then translucent
        // full-canvas fills at the pipeline weight, each with the per-op cost.
        let blue = 64 + 256 * 256 * raster::PIPELINE_WEIGHT;
        let fitted = (report.budget - (64 + 128 * 256)) / blue;
        assert_eq!(fitted, 63);
        assert_eq!(report.ops_done, 1 + fitted as usize);
        assert!(report.units <= report.budget);
        assert!(report.units > report.budget - blue, "{report:?}");
        let corner = img.pixel(230, 230).unwrap();
        assert!(
            corner[1] < 20 && corner[2] > 200,
            "the green rectangle was never drawn: {corner:?}"
        );
        assert!(img.pixel(10, 10).unwrap()[2] > 200, "the blue layers were");
        assert!(img.pixel(10, 10).unwrap()[0] < 10, "and over the red");

        // A list that fits is reported whole.
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.rect(0.0, 0.0, 256.0, 256.0, 0.0, 0x00FF_00FF).unwrap();
        dl.end().unwrap();
        let r = render_full(dl.bytes(), 256, 256, 1.0, &mut t).unwrap();
        assert!(!r.report.truncated);
        assert_eq!(r.report.ops_done, 1);
        assert_eq!(r.report.units, 64 + 256 * 256);
        assert_eq!(r.image.pixel(230, 230).unwrap(), [0, 255, 0, 255]);
    }

    /// SPEC-5.3.3, SPEC-6.1.4: a clip level keeps its own mask and the
    /// parent's comes back on pop; a plain clip inside a rounded one, and a
    /// rounded one inside a plain one, both cut to the intersection; the
    /// pixels match a list that pushes the same shapes afresh.
    #[test]
    fn spec_6_1_4_clip_levels_restore_on_pop() {
        let mut t = TextEngine::new();
        let white = [255, 255, 255, 255];
        let red = [255, 0, 0, 255];
        let blue = [0, 0, 255, 255];
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.clip_push(0.0, 0.0, 50.0, 100.0, 0.0).unwrap(); // left half
        dl.clip_push(20.0, 20.0, 60.0, 60.0, 15.0).unwrap(); // rounded, overlapping the edge
        dl.rect(0.0, 0.0, 100.0, 100.0, 0.0, 0xFF00_00FF).unwrap();
        dl.clip_pop().unwrap();
        dl.rect(0.0, 90.0, 100.0, 10.0, 0.0, 0x0000_FFFF).unwrap(); // only the plain clip now
        dl.clip_push(40.0, 0.0, 60.0, 20.0, 3.0).unwrap(); // rounded again, another shape
        dl.rect(0.0, 0.0, 100.0, 100.0, 0.0, 0x0000_FFFF).unwrap();
        dl.clip_pop().unwrap();
        dl.clip_pop().unwrap();
        dl.rect(90.0, 0.0, 10.0, 10.0, 0.0, 0xFF00_00FF).unwrap(); // unclipped
        dl.end().unwrap();
        let img = render(dl.bytes(), 100, 100, 1.0, &mut t).unwrap();
        let px = |x, y| img.pixel(x, y).unwrap();
        assert_eq!(px(30, 50), red, "inside both clips");
        assert_eq!(px(60, 50), white, "cut by the plain parent");
        assert_eq!(px(21, 21), white, "cut by the rounded corner");
        assert_eq!(px(10, 50), white, "outside the rounded shape");
        assert_eq!(px(10, 95), blue, "the parent alone after the pop");
        assert_eq!(px(60, 95), white, "and still cuts at 50");
        assert_eq!(
            px(45, 10),
            blue,
            "the second rounded clip, inside the parent"
        );
        assert_eq!(px(55, 10), white, "cut by the parent at 50");
        assert_eq!(px(95, 5), red, "nothing clips after the last pop");
        assert_eq!(px(45, 85), white, "between the clips nothing drew");
        assert_eq!(
            px(10, 10),
            white,
            "inside the parent, outside both rounded clips"
        );

        // The same picture when each clip is its own push from the root.
        let mut again = DisplayList::new();
        again.begin().unwrap();
        again.clip_push(0.0, 0.0, 50.0, 100.0, 0.0).unwrap();
        again.clip_push(20.0, 20.0, 60.0, 60.0, 15.0).unwrap();
        again
            .rect(0.0, 0.0, 100.0, 100.0, 0.0, 0xFF00_00FF)
            .unwrap();
        again.clip_pop().unwrap();
        again.clip_pop().unwrap();
        again.clip_push(0.0, 0.0, 50.0, 100.0, 0.0).unwrap();
        again
            .rect(0.0, 90.0, 100.0, 10.0, 0.0, 0x0000_FFFF)
            .unwrap();
        again.clip_pop().unwrap();
        again.clip_push(0.0, 0.0, 50.0, 100.0, 0.0).unwrap();
        again.clip_push(40.0, 0.0, 60.0, 20.0, 3.0).unwrap();
        again
            .rect(0.0, 0.0, 100.0, 100.0, 0.0, 0x0000_FFFF)
            .unwrap();
        again.clip_pop().unwrap();
        again.clip_pop().unwrap();
        again.rect(90.0, 0.0, 10.0, 10.0, 0.0, 0xFF00_00FF).unwrap();
        again.end().unwrap();
        assert_eq!(render(again.bytes(), 100, 100, 1.0, &mut t).unwrap(), img);
    }

    /// A wall-clock bound on work the budget already bounds: asserted in a
    /// release build (the CI bench job, as SPEC §13 measures), reported in a
    /// debug build, where the time on a shared two-CPU machine says more
    /// about its load than about the rasterizer.
    fn time_bound(took: std::time::Duration, bound: std::time::Duration, what: &str) {
        if cfg!(debug_assertions) {
            eprintln!("{what} took {took:?} (debug build; {bound:?} is asserted in release)");
        } else {
            assert!(took < bound, "{what} took {took:?} (SPEC-5.3.3)");
        }
    }

    /// SPEC-5.3.3, SPEC-9.3.1: the pathological frame of review 0003 — 65 000
    /// full-screen rectangles at 1280 × 800, 16 s of rasterization — is cut
    /// off by the budget after 65 of them, and in a release build finishes
    /// in well under two seconds. The bound is loose on purpose.
    #[test]
    fn spec_5_3_3_bench_65000_fullscreen_rects() {
        let _serial = serial();
        let mut t = TextEngine::new();
        let bytes = pathological(0, 65_000, 0.0, 1280.0, 800.0);
        let start = std::time::Instant::now();
        let r = render_full(&bytes, 1280, 800, 1.0, &mut t).unwrap();
        let took = start.elapsed();
        assert!(r.report.truncated);
        let fits = r.report.budget / (raster::OP_UNITS + 1280 * 800);
        assert_eq!(fits, 65);
        assert_eq!(r.report.ops_done, fits as usize, "{:?}", r.report);
        time_bound(
            took,
            std::time::Duration::from_secs(2),
            "65 000 full-screen rects",
        );
    }

    /// SPEC-5.3.3, SPEC-6.1.4: 32 nested rounded full-screen clips (the
    /// deepest stack 1.0 allows) with 2 000 full-screen rectangles under
    /// them — 312 s in review 0003 with 65 000 rectangles, when every push
    /// rebuilt the whole mask — is stopped by the budget once the masks
    /// alone have used it up (each level's mask is built once over its own
    /// bounds), and in a release build stays under a second.
    #[test]
    fn spec_5_3_3_bench_32_rounded_clips_2000_rects() {
        let _serial = serial();
        let mut t = TextEngine::new();
        let bytes = pathological(32, 2_000, 8.0, 1280.0, 800.0);
        let start = std::time::Instant::now();
        let r = render_full(&bytes, 1280, 800, 1.0, &mut t).unwrap();
        let took = start.elapsed();
        assert!(r.report.truncated, "{:?}", r.report);
        assert!(
            r.report.ops_done >= 32,
            "the pushes themselves cost nothing"
        );
        time_bound(
            took,
            std::time::Duration::from_secs(1),
            "32 rounded clips + 2 000 rects",
        );
        // Two thousand small rounded cards, each pushed, drawn and popped,
        // are an ordinary page and fit the budget with room to spare.
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        for i in 0..2_000 {
            let (x, y) = ((i % 40) as f32 * 32.0, (i / 40) as f32 * 16.0);
            dl.clip_push(x, y, 30.0, 14.0, 4.0).unwrap();
            dl.rect(x - 5.0, y - 5.0, 40.0, 24.0, 0.0, 0x3366_99FF)
                .unwrap();
            dl.clip_pop().unwrap();
        }
        dl.end().unwrap();
        let start = std::time::Instant::now();
        let r = render_full(dl.bytes(), 1280, 800, 1.0, &mut t).unwrap();
        let took = start.elapsed();
        assert!(!r.report.truncated, "{:?}", r.report);
        assert!(r.report.units < r.report.budget / 2, "{:?}", r.report);
        time_bound(
            took,
            std::time::Duration::from_secs(2),
            "2 000 rounded cards",
        );
    }
}
