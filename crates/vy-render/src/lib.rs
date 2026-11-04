//! Rasterization: rectangles, paths, images, text shaping and glyph cache (SPEC §6).
//!
//! M1 ships a software rasterizer used by `vy-cli render`; wgpu arrives with M3.
//! Headless and windowed rendering share this code path (SPEC-11.3.2).
#![deny(missing_docs)]

pub mod image;
pub mod raster;
pub mod text;

pub use image::{diff_ratio, RgbaImage};
pub use raster::{render, RenderError};
pub use text::{height_x100, pack_x100, Size, TextEngine};

#[cfg(test)]
mod tests {
    use super::*;
    use vy_abi::err;
    use vy_dl::DisplayList;

    fn sans(engine: &mut TextEngine, size: f32) -> u16 {
        engine
            .style(text::role::SANS, size, 400, 0x0000_00FF)
            .unwrap()
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
        assert_eq!(height_x100(narrow.h), (narrow.h * 100.0).round() as i32);
        // Packed measurement: width in the high 32 bits, height in the low (SPEC-5.2.5).
        let packed = pack_x100(narrow.w, narrow.h);
        assert_eq!((packed & 0xFFFF_FFFF) as i32, height_x100(narrow.h));
        assert_eq!((packed >> 32) as i32, (narrow.w * 100.0).round() as i32);
        assert_eq!(t.measure(99, 100.0, txt), Err(err::E_STYLE));
    }

    /// SPEC-6.3.1: scale 2 gives twice the physical pixels and the same logical layout.
    #[test]
    fn spec_6_3_1_dpi_scaling() {
        let mut t = TextEngine::new();
        let s = sans(&mut t, 20.0);
        let mut dl = DisplayList::new();
        dl.begin();
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
        dl.begin();
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

    /// SPEC-6.4.2: resource ids -1..-5 are the built-in roles; anything else is E_RESOURCE.
    #[test]
    fn spec_6_4_2_builtin_font_roles() {
        let mut t = TextEngine::new();
        for id in -5..=-1 {
            let sid = t.style(id, 16.0, 400, 0).unwrap();
            assert!(sid > 0, "role {id}");
            assert!(t.measure(sid, 100.0, "abc").unwrap().w > 0.0, "role {id}");
        }
        assert_eq!(t.style(-6, 16.0, 400, 0), Err(err::E_RESOURCE));
        assert_eq!(t.style(0, 16.0, 400, 0), Err(err::E_RESOURCE));
        assert_eq!(t.style(-1, 0.0, 400, 0), Err(err::E_ARG));
        assert_eq!(t.style(-1, 16.0, 0, 0), Err(err::E_ARG));
        // Serif and mono are distinct faces: the same string measures differently.
        let sans = t.style(text::role::SANS, 16.0, 400, 0).unwrap();
        let mono = t.style(text::role::MONO, 16.0, 400, 0).unwrap();
        let a = t.measure(sans, 1000.0, "iiii mmmm").unwrap().w;
        let b = t.measure(mono, 1000.0, "iiii mmmm").unwrap().w;
        assert_ne!(a, b);
    }

    /// SPEC-6.1.3, SPEC-6.1.4: later ops paint over earlier ones and clipping limits drawing.
    #[test]
    fn spec_6_1_3_paint_order_and_clip() {
        let mut t = TextEngine::new();
        let mut dl = DisplayList::new();
        dl.begin();
        dl.rect(0.0, 0.0, 40.0, 40.0, 0.0, 0xFF00_00FF).unwrap();
        dl.clip_push(0.0, 0.0, 20.0, 40.0).unwrap();
        dl.rect(0.0, 0.0, 40.0, 40.0, 0.0, 0x00FF_00FF).unwrap();
        dl.clip_pop().unwrap();
        dl.line(0.0, 35.0, 40.0, 35.0, 2.0, 0x0000_FFFF).unwrap();
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

    /// SPEC-C.1.2: paths fill (stroke_w 0) and stroke; shadows and images draw something.
    #[test]
    fn spec_c_1_2_path_fill_and_stroke() {
        use vy_dl::PathCmd;
        let mut t = TextEngine::new();
        let mut dl = DisplayList::new();
        dl.begin();
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
        dl.shadow(50.0, 50.0, 20.0, 20.0, 0.0, 4.0, 0x0000_00FF)
            .unwrap();
        dl.image(80.0, 80.0, 10.0, 10.0, 1, vy_dl::Fit::Contain)
            .unwrap();
        dl.end().unwrap();
        let img = render(dl.bytes(), 100, 100, 1.0, &mut t).unwrap();
        assert_eq!(img.pixel(5, 5).unwrap(), [0, 0, 0, 255], "filled triangle");
        assert_eq!(
            img.pixel(35, 35).unwrap(),
            [255, 255, 255, 255],
            "outside triangle"
        );
        assert_eq!(img.pixel(30, 39).unwrap(), [255, 0, 0, 255], "stroked line");
        let shadow = img.pixel(60, 60).unwrap();
        assert!(shadow[0] <= 2, "shadow centre is opaque: {shadow:?}");
        let edge = img.pixel(48, 60).unwrap();
        assert!(
            edge[0] > shadow[0] && edge[0] < 255,
            "shadow fades: {edge:?}"
        );
        assert_eq!(
            img.pixel(85, 85).unwrap(),
            [128, 128, 128, 255],
            "image placeholder"
        );
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

    /// SPEC-6.5.1: images decoded lazily, cached per package, max 8192x8192.
    #[test]
    #[ignore = "M1.5: images"]
    fn spec_6_5_1_image_decode_limits() {
        todo!()
    }

    /// SPEC-13.2.1: 2 000 ops rasterize in <= 2 ms on the reference machine.
    #[test]
    #[ignore = "M3: bench"]
    fn spec_13_2_1_raster_budget() {
        todo!()
    }
}
