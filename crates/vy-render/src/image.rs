//! Straight-alpha RGBA8 image and PNG encoding/decoding for the headless path.

use std::fmt;

/// An 8-bit RGBA image with straight (non-premultiplied) alpha, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
    /// `width * height * 4` bytes, `RGBA` per pixel.
    pub data: Vec<u8>,
}

/// PNG encode or decode failure.
#[derive(Debug)]
pub struct PngError(String);

impl fmt::Display for PngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PngError {}

impl RgbaImage {
    /// Transparent black image.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    /// Pixel at `(x, y)` as `[r, g, b, a]`, or `None` outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        Some([
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ])
    }

    /// Encode as 8-bit RGBA PNG. Same image, same bytes (SPEC-11.3.1).
    pub fn encode_png(&self) -> Result<Vec<u8>, PngError> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.width, self.height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Balanced);
            let mut w = enc.write_header().map_err(|e| PngError(e.to_string()))?;
            w.write_image_data(&self.data)
                .map_err(|e| PngError(e.to_string()))?;
            w.finish().map_err(|e| PngError(e.to_string()))?;
        }
        Ok(out)
    }

    /// Decode a PNG of any colour type into straight-alpha RGBA8.
    pub fn decode_png(bytes: &[u8]) -> Result<Self, PngError> {
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = dec.read_info().map_err(|e| PngError(e.to_string()))?;
        let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| PngError(e.to_string()))?;
        buf.truncate(info.buffer_size());
        let data = match info.color_type {
            png::ColorType::Rgba => buf,
            png::ColorType::Rgb => buf
                .chunks_exact(3)
                .flat_map(|p| [p[0], p[1], p[2], 255])
                .collect(),
            png::ColorType::GrayscaleAlpha => buf
                .chunks_exact(2)
                .flat_map(|p| [p[0], p[0], p[0], p[1]])
                .collect(),
            png::ColorType::Grayscale => buf.iter().flat_map(|&v| [v, v, v, 255]).collect(),
            other => return Err(PngError(format!("unsupported PNG colour type {other:?}"))),
        };
        Ok(Self {
            width: info.width,
            height: info.height,
            data,
        })
    }
}

/// Fraction of pixels (0.0..=1.0) that differ in any channel between two images.
///
/// Images of different sizes differ completely (1.0).
pub fn diff_ratio(a: &RgbaImage, b: &RgbaImage) -> f64 {
    if a.width != b.width || a.height != b.height {
        return 1.0;
    }
    let total = (a.width as usize) * (a.height as usize);
    if total == 0 {
        return 0.0;
    }
    let differing = a
        .data
        .chunks_exact(4)
        .zip(b.data.chunks_exact(4))
        .filter(|(p, q)| p != q)
        .count();
    differing as f64 / total as f64
}
