//! Compositing: the page's rasterized frame under the chrome's, in the
//! page area and nowhere else (SPEC-9.6.1), and the conversion to the
//! window's pixel format.

use vy_render::RgbaImage;

/// Physical pixel size of a logical size at `scale`, at least 1 × 1.
pub fn physical(width: f32, height: f32, scale: f32) -> (u32, u32) {
    (
        (width * scale).round().max(1.0) as u32,
        (height * scale).round().max(1.0) as u32,
    )
}

/// Copy `src` into `dst` with its top-left corner at `(x, y)` physical
/// pixels, cut to `dst`'s bounds and to `clip` (left, top, width,
/// height) when given. Straight copy: the page's frame is opaque white
/// where it drew nothing, like the headless render.
pub fn blit(
    dst: &mut RgbaImage,
    src: &RgbaImage,
    x: i64,
    y: i64,
    clip: Option<(i64, i64, i64, i64)>,
) {
    let (mut x0, mut y0) = (x.max(0), y.max(0));
    let (mut x1, mut y1) = (
        (x + i64::from(src.width)).min(i64::from(dst.width)),
        (y + i64::from(src.height)).min(i64::from(dst.height)),
    );
    if let Some((cx, cy, cw, ch)) = clip {
        x0 = x0.max(cx);
        y0 = y0.max(cy);
        x1 = x1.min(cx + cw);
        y1 = y1.min(cy + ch);
    }
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let row_len = ((x1 - x0) * 4) as usize;
    for row in y0..y1 {
        let sy = (row - y) as usize;
        let sx = (x0 - x) as usize;
        let s = (sy * src.width as usize + sx) * 4;
        let d = (row as usize * dst.width as usize + x0 as usize) * 4;
        dst.data[d..d + row_len].copy_from_slice(&src.data[s..s + row_len]);
    }
}

/// Blend `src` over `dst` at `(x, y)` with `src`'s straight alpha, for
/// the status overlay.
pub fn blend(dst: &mut RgbaImage, src: &RgbaImage, x: i64, y: i64) {
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = (
        (x + i64::from(src.width)).min(i64::from(dst.width)),
        (y + i64::from(src.height)).min(i64::from(dst.height)),
    );
    for row in y0..y1 {
        for col in x0..x1 {
            let s = (((row - y) as usize) * src.width as usize + (col - x) as usize) * 4;
            let d = ((row as usize) * dst.width as usize + col as usize) * 4;
            let a = u32::from(src.data[s + 3]);
            if a == 0 {
                continue;
            }
            for c in 0..3 {
                let sv = u32::from(src.data[s + c]);
                let dv = u32::from(dst.data[d + c]);
                dst.data[d + c] = ((sv * a + dv * (255 - a) + 127) / 255) as u8;
            }
            dst.data[d + 3] = 255;
        }
    }
}

/// Blend a rectangle of colour `rgba` (`0xRRGGBBAA`, straight alpha) over
/// `dst` at `(x, y, w, h)` physical pixels, cut to `dst` and to `clip`
/// (left, top, width, height): the shell's overlays on the page area
/// (the selection, find matches, the devtools outline).
pub fn fill_blend(
    dst: &mut RgbaImage,
    rect: (i64, i64, i64, i64),
    rgba: u32,
    clip: (i64, i64, i64, i64),
) {
    let (x, y, w, h) = rect;
    let (cx, cy, cw, ch) = clip;
    let x0 = x.max(cx).max(0);
    let y0 = y.max(cy).max(0);
    let x1 = (x + w).min(cx + cw).min(i64::from(dst.width));
    let y1 = (y + h).min(cy + ch).min(i64::from(dst.height));
    let a = rgba & 0xFF;
    if x1 <= x0 || y1 <= y0 || a == 0 {
        return;
    }
    let src = [(rgba >> 24) & 0xFF, (rgba >> 16) & 0xFF, (rgba >> 8) & 0xFF];
    for row in y0..y1 {
        for col in x0..x1 {
            let d = ((row as usize) * dst.width as usize + col as usize) * 4;
            for (c, sv) in src.iter().enumerate() {
                let dv = u32::from(dst.data[d + c]);
                dst.data[d + c] = ((sv * a + dv * (255 - a) + 127) / 255) as u8;
            }
            dst.data[d + 3] = 255;
        }
    }
}

/// The frame as `0x00RRGGBB` words, which is what `softbuffer` presents.
pub fn to_xrgb(img: &RgbaImage, out: &mut [u32]) {
    for (px, dst) in img.data.chunks_exact(4).zip(out.iter_mut()) {
        *dst = (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, rgba: [u8; 4]) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for px in img.data.chunks_exact_mut(4) {
            px.copy_from_slice(&rgba);
        }
        img
    }

    /// SPEC-9.6.1: a blit never writes outside the destination or the
    /// clip, whatever the offset; the overlay blends by alpha; the window
    /// format drops alpha.
    #[test]
    fn spec_9_6_1_blit_stays_in_bounds() {
        let mut dst = solid(10, 10, [0, 0, 0, 255]);
        let src = solid(6, 6, [255, 0, 0, 255]);
        blit(&mut dst, &src, -3, 7, None);
        assert_eq!(dst.pixel(0, 9), Some([255, 0, 0, 255]));
        assert_eq!(dst.pixel(2, 7), Some([255, 0, 0, 255]));
        assert_eq!(dst.pixel(3, 7), Some([0, 0, 0, 255]));
        assert_eq!(dst.pixel(0, 6), Some([0, 0, 0, 255]));
        let mut dst = solid(10, 10, [0, 0, 0, 255]);
        blit(&mut dst, &src, 2, 2, Some((0, 4, 10, 10)));
        assert_eq!(dst.pixel(3, 3), Some([0, 0, 0, 255]), "above the clip");
        assert_eq!(dst.pixel(3, 4), Some([255, 0, 0, 255]));
        assert_eq!(dst.pixel(7, 7), Some([255, 0, 0, 255]));
        assert_eq!(dst.pixel(8, 7), Some([0, 0, 0, 255]));
        blit(&mut dst, &src, 100, 100, None);
        blit(&mut dst, &src, -100, -100, None);
        let mut dst = solid(2, 1, [0, 0, 0, 255]);
        blend(&mut dst, &solid(1, 1, [255, 255, 255, 128]), 0, 0);
        assert_eq!(dst.pixel(0, 0), Some([128, 128, 128, 255]));
        assert_eq!(dst.pixel(1, 0), Some([0, 0, 0, 255]));
        let mut words = [0u32; 2];
        to_xrgb(&solid(2, 1, [1, 2, 3, 255]), &mut words);
        assert_eq!(words, [0x0001_0203, 0x0001_0203]);
        assert_eq!(physical(800.0, 600.0, 1.5), (1200, 900));
        assert_eq!(physical(0.0, 0.0, 1.0), (1, 1));
    }
}
