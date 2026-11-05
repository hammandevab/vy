//! Straight-alpha RGBA8 image, PNG encoding/decoding, and the per-package
//! image cache behind `IMAGE` (SPEC-6.5.1).

use crate::raster::MAX_CANVAS_SIDE_PX;
use std::fmt;
use vy_abi::limits;

/// Largest decoded image memory one package may hold in the host: one
/// maximal image (8192 × 8192 × 4 bytes, SPEC-6.5.1). A decode that would
/// push the cache past it fails with `E_RESOURCE`, so a package cannot make
/// the host hold more than this however many resources it ships
/// (SPEC-9.3.1). Host policy, not ABI.
pub const MAX_IMAGE_CACHE_BYTES: usize =
    limits::MAX_IMAGE_SIDE_PX as usize * limits::MAX_IMAGE_SIDE_PX as usize * 4;

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
    ///
    /// A header declaring a side above [`MAX_CANVAS_SIDE_PX`] (what `render`
    /// can have written) is refused before any pixel buffer is allocated, so
    /// a crafted 16-byte header cannot make the host allocate gigabytes.
    /// Package resources go through [`ImageCache`] and the stricter limit of
    /// SPEC-6.5.1.
    pub fn decode_png(bytes: &[u8]) -> Result<Self, PngError> {
        Self::decode_png_within(bytes, Some(MAX_CANVAS_SIDE_PX)).map_err(|e| match e {
            ImageError::Decode(s) => PngError(s),
            other => PngError(format!("{other}")),
        })
    }

    /// Decode a PNG, refusing one whose header declares a side above
    /// `max_side` before any pixel buffer is allocated (SPEC-6.5.1).
    fn decode_png_within(bytes: &[u8], max_side: Option<u32>) -> Result<Self, ImageError> {
        let decode_err = |e: png::DecodingError| ImageError::Decode(e.to_string());
        let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
        dec.set_transformations(png::Transformations::normalize_to_color8());
        let header = dec.read_header_info().map_err(decode_err)?;
        let (width, height) = (header.width, header.height);
        if let Some(max) = max_side {
            if width > max || height > max {
                return Err(ImageError::TooLarge {
                    width,
                    height,
                    max_side: max,
                });
            }
        }
        let mut reader = dec.read_info().map_err(decode_err)?;
        let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
        let info = reader.next_frame(&mut buf).map_err(decode_err)?;
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
            other => {
                return Err(ImageError::Decode(format!(
                    "unsupported PNG colour type {other:?}"
                )))
            }
        };
        Ok(Self {
            width: info.width,
            height: info.height,
            data,
        })
    }
}

/// One package resource as the host holds it before decoding: the bytes and,
/// for an image, the size the manifest declares for it (SPEC-8.1.2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resource {
    /// The file's bytes.
    pub bytes: Vec<u8>,
    /// `(w, h)` from the manifest, checked against the decoded image
    /// (SPEC-6.5.1); `None` when the caller did not state one.
    pub pixels: Option<(u32, u32)>,
}

impl Resource {
    /// A resource with no declared size.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            pixels: None,
        }
    }

    /// A resource the manifest says is `w` × `h` pixels.
    pub fn with_size(bytes: Vec<u8>, w: u32, h: u32) -> Self {
        Self {
            bytes,
            pixels: Some((w, h)),
        }
    }
}

/// Why an image resource cannot be drawn. Every variant is `E_RESOURCE` on
/// the ABI (SPEC-A.1.1: "unknown resource id or wrong type").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Resource id 0 (the module itself, SPEC-4.5.1) or past the last resource.
    Unknown,
    /// Width or height above the side limit (`MAX_IMAGE_SIDE_PX` for a
    /// resource, SPEC-6.5.1; `MAX_CANVAS_SIDE_PX` for `decode_png`);
    /// nothing was decoded.
    TooLarge {
        /// Declared width.
        width: u32,
        /// Declared height.
        height: u32,
        /// The limit that was exceeded.
        max_side: u32,
    },
    /// Not a PNG, or a PNG that does not decode. WebP and AVIF (SPEC-4.1.5)
    /// are not decoded before M3 and land here.
    Decode(String),
    /// Decoding would push the package past [`MAX_IMAGE_CACHE_BYTES`].
    CacheFull,
    /// The decoded size is not what the manifest declared (SPEC-8.1.2,
    /// SPEC-6.5.1); nothing is kept.
    SizeMismatch {
        /// Decoded width and height.
        actual: (u32, u32),
        /// Declared width and height.
        declared: (u32, u32),
    },
}

impl fmt::Display for ImageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("unknown resource id"),
            Self::TooLarge {
                width,
                height,
                max_side,
            } => {
                write!(f, "{width}x{height} px exceeds {max_side} px per side")?;
                if *max_side == limits::MAX_IMAGE_SIDE_PX {
                    f.write_str(" (SPEC-6.5.1)")?;
                }
                Ok(())
            }
            Self::Decode(s) => write!(f, "cannot decode image: {s}"),
            Self::CacheFull => write!(
                f,
                "decoded images would exceed {} bytes",
                MAX_IMAGE_CACHE_BYTES
            ),
            Self::SizeMismatch { actual, declared } => write!(
                f,
                "{}x{} px but the manifest says {}x{} (SPEC-8.1.2)",
                actual.0, actual.1, declared.0, declared.1
            ),
        }
    }
}

impl ImageError {
    /// The ABI status code every image refusal maps to.
    pub fn code(&self) -> i32 {
        vy_abi::err::E_RESOURCE
    }
}

impl std::error::Error for ImageError {}

enum Slot {
    /// Raw resource bytes, not yet touched by `dl_image`.
    Raw(Resource),
    /// Decoded on first use.
    Ready(RgbaImage),
    /// Refused on first use; the raw length is kept for accounting.
    Failed(ImageError),
}

/// Image resources of one package, decoded lazily at the first `dl_image`
/// that names them and cached for the life of the instance (SPEC-6.5.1).
///
/// `resources[i]` is resource id `i + 1`; id 0 is the module itself
/// (SPEC-4.5.1) and never an image. A failed decode is cached too, so a bad
/// resource costs the host one attempt, not one per frame.
pub struct ImageCache {
    slots: Vec<Slot>,
    pub(crate) decoded_bytes: usize,
    decodes: u32,
}

impl ImageCache {
    /// Cache over the package's resources, nothing decoded yet.
    pub fn new(resources: Vec<Resource>) -> Self {
        Self {
            slots: resources.into_iter().map(Slot::Raw).collect(),
            decoded_bytes: 0,
            decodes: 0,
        }
    }

    /// Cache over resources with no declared sizes.
    pub fn from_bytes(resources: Vec<Vec<u8>>) -> Self {
        Self::new(resources.into_iter().map(Resource::new).collect())
    }

    /// A cache of `count` resources with no bytes at all: every id draws
    /// the placeholder until [`ImageCache::set_decoded`] fills it. The
    /// main process holds one per tab, filled with the pixels the tab
    /// process decoded (SPEC-9.1.2: it rasterizes, the tab decodes).
    pub fn empty(count: usize) -> Self {
        Self {
            slots: (0..count)
                .map(|_| Slot::Failed(ImageError::Unknown))
                .collect(),
            decoded_bytes: 0,
            decodes: 0,
        }
    }

    /// Put an already decoded image behind resource `id`. False, and
    /// nothing kept, for an id outside the package, for an image already
    /// in place, or when the pixels would push the cache past
    /// [`MAX_IMAGE_CACHE_BYTES`] (SPEC-9.3.1: the limit holds whoever
    /// decoded).
    pub fn set_decoded(&mut self, id: u16, image: RgbaImage) -> bool {
        let Some(slot) = Self::index(id).and_then(|i| self.slots.get_mut(i)) else {
            return false;
        };
        if matches!(slot, Slot::Ready(_)) {
            return false;
        }
        let expected = (image.width as usize) * (image.height as usize) * 4;
        if image.data.len() != expected
            || image.width > limits::MAX_IMAGE_SIDE_PX
            || image.height > limits::MAX_IMAGE_SIDE_PX
            || image.data.len() > MAX_IMAGE_CACHE_BYTES.saturating_sub(self.decoded_bytes)
        {
            return false;
        }
        self.decoded_bytes += image.data.len();
        *slot = Slot::Ready(image);
        true
    }

    /// Number of resources in the package (ids `1..=count`).
    pub fn resource_count(&self) -> usize {
        self.slots.len()
    }

    /// Decode attempts made so far; a cache hit makes none.
    pub fn decodes(&self) -> u32 {
        self.decodes
    }

    /// Bytes of decoded pixels held.
    pub fn decoded_bytes(&self) -> usize {
        self.decoded_bytes
    }

    fn index(id: u16) -> Option<usize> {
        usize::from(id).checked_sub(1)
    }

    /// Make sure resource `id` is decoded or marked failed. Returns the number
    /// of resource bytes the host read to do so: the whole resource on a
    /// decode attempt, 0 on a cache hit or when only the header was read.
    pub fn ensure(&mut self, id: u16) -> usize {
        let Some(slot) = Self::index(id).and_then(|i| self.slots.get_mut(i)) else {
            return 0;
        };
        let Slot::Raw(raw) = slot else {
            return 0;
        };
        self.decodes += 1;
        let Resource { bytes: raw, pixels } = std::mem::take(raw);
        let room = MAX_IMAGE_CACHE_BYTES.saturating_sub(self.decoded_bytes);
        let (outcome, cost) =
            match RgbaImage::decode_png_within(&raw, Some(limits::MAX_IMAGE_SIDE_PX)) {
                Err(e @ ImageError::TooLarge { .. }) => (Slot::Failed(e), 0),
                Err(e) => (Slot::Failed(e), raw.len()),
                Ok(img) if pixels.is_some_and(|p| p != (img.width, img.height)) => (
                    Slot::Failed(ImageError::SizeMismatch {
                        actual: (img.width, img.height),
                        declared: pixels.unwrap_or_default(),
                    }),
                    raw.len(),
                ),
                Ok(img) if img.data.len() > room => {
                    (Slot::Failed(ImageError::CacheFull), raw.len())
                }
                Ok(img) => {
                    self.decoded_bytes += img.data.len();
                    (Slot::Ready(img), raw.len())
                }
            };
        *slot = outcome;
        cost
    }

    /// The decoded image, or why it is not available. A resource that has not
    /// been through [`ImageCache::ensure`] is reported as `Unknown` to the
    /// rasterizer, which then draws nothing for it.
    pub fn get(&self, id: u16) -> Result<&RgbaImage, ImageError> {
        match Self::index(id).and_then(|i| self.slots.get(i)) {
            Some(Slot::Ready(img)) => Ok(img),
            Some(Slot::Failed(e)) => Err(e.clone()),
            Some(Slot::Raw(_)) | None => Err(ImageError::Unknown),
        }
    }

    /// Decode on first use and return the image (SPEC-6.5.1): `ensure` then `get`.
    pub fn decode(&mut self, id: u16) -> Result<&RgbaImage, ImageError> {
        self.ensure(id);
        self.get(id)
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
