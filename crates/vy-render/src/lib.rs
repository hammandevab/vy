//! Rasterization: rectangles, paths, images, text shaping and glyph cache (SPEC §6).
//!
//! M1 ships a software rasterizer used by `vy-cli render`; wgpu arrives with M3.
//! Headless and windowed rendering share this code path (SPEC-11.3.2).
#![deny(missing_docs)]

#[cfg(test)]
mod tests {
    /// SPEC-5.2.4, SPEC-6.4.3: dl_text wraps at w and returns height * 100.
    #[test]
    #[ignore = "M1: text layout"]
    fn spec_6_4_3_text_wrap_and_height() {
        todo!()
    }

    /// SPEC-6.3.1: logical pixels scaled by `scale` to physical pixels.
    #[test]
    #[ignore = "M1: raster"]
    fn spec_6_3_1_dpi_scaling() {
        todo!()
    }

    /// SPEC-6.4.1: shaping happens in the host; module never sees glyphs.
    #[test]
    #[ignore = "M1: text"]
    fn spec_6_4_1_host_shaping() {
        todo!()
    }

    /// SPEC-6.4.2: built-in font roles -1..-5.
    #[test]
    #[ignore = "M1: fonts"]
    fn spec_6_4_2_builtin_font_roles() {
        todo!()
    }

    /// SPEC-6.5.1: images decoded lazily, cached per package, max 8192x8192.
    #[test]
    #[ignore = "M1: images"]
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
