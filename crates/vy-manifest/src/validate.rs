//! Validation of a manifest against the module and resources it describes
//! (SPEC §4, §8, SPEC-9.4.1).

use crate::manifest::{
    Manifest, MAX_FALLBACK_BYTES, MAX_MANIFEST_BYTES, MAX_NAME_CHARS, MAX_NET_ORIGINS,
    MAX_STORAGE_QUOTA_KB, MAX_SUMMARY_BYTES,
};
use crate::Hash;
use std::fmt;
use vy_abi::limits;

/// Maximum uncompressed module size in bytes (SPEC-4.1.4).
pub const MAX_MODULE_BYTES: usize = 8 * 1024 * 1024;
/// Maximum total resource size in bytes (SPEC-4.1.5).
pub const MAX_RESOURCES_BYTES: u64 = 32 * 1024 * 1024;
/// Largest `storage` quota granted without asking (SPEC-8.2.3): 1 MB.
pub const AUTO_STORAGE_QUOTA_KB: u32 = 1024;
/// Resource media types of 1.0 (SPEC-4.1.5); "raw data" is
/// `application/octet-stream`.
pub const RESOURCE_MIMES: [&str; 5] = [
    "image/png",
    "image/webp",
    "image/avif",
    "font/woff2",
    "application/octet-stream",
];

/// One reason a manifest does not describe its package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Problem {
    /// ABI major is not 1 (SPEC-4.3.1).
    AbiMajor {
        /// Declared major.
        found: u32,
    },
    /// `name` is empty.
    NameEmpty,
    /// `name` is over 80 characters (SPEC-8.1.1).
    NameTooLong {
        /// Length in characters.
        chars: usize,
    },
    /// `summary` is over 2 KB (SPEC-8.1.1).
    SummaryTooLong {
        /// Length in bytes.
        bytes: usize,
    },
    /// `lang` is not a BCP 47 tag (SPEC-8.1.1).
    LangInvalid(String),
    /// `published` is not `YYYY-MM-DD`.
    PublishedInvalid(String),
    /// `fallback` is over 16 KB (SPEC-8.1.2).
    FallbackTooLong {
        /// Length in bytes.
        bytes: usize,
    },
    /// `memory_max` is 0 or over 4096 pages (SPEC-8.1.3).
    MemoryMaxOutOfRange {
        /// Declared pages.
        pages: u32,
    },
    /// `memory_max` leaves the module no room to grow (review 0002,
    /// decision 29): it must be at least the initial pages plus one.
    MemoryMaxBelowInitial {
        /// Declared pages.
        pages: u32,
        /// Initial pages of the module's memory.
        initial: u32,
    },
    /// The module bytes are not a WebAssembly module with a memory.
    ModuleNotWasm(String),
    /// The module is over 8 MB (SPEC-4.1.4).
    ModuleTooLarge {
        /// Actual size.
        bytes: usize,
    },
    /// `module.hash` differs from the bytes (SPEC-9.4.1).
    ModuleHashMismatch {
        /// Hash of the bytes given.
        actual: Hash,
    },
    /// `module.size` differs from the bytes.
    ModuleSizeMismatch {
        /// Declared size.
        declared: u64,
        /// Actual size.
        actual: u64,
    },
    /// The number of resources given differs from the manifest (SPEC-4.1.1).
    ResourceCountMismatch {
        /// Entries in the manifest.
        declared: usize,
        /// Resources given.
        given: usize,
    },
    /// A resource's hash differs from its bytes (resource index, 1-based).
    ResourceHashMismatch {
        /// Resource index (SPEC-4.5.1).
        index: u32,
        /// Hash of the bytes given.
        actual: Hash,
    },
    /// A resource's size differs from its bytes.
    ResourceSizeMismatch {
        /// Resource index.
        index: u32,
        /// Declared size.
        declared: u64,
        /// Actual size.
        actual: u64,
    },
    /// Resources total over 32 MB (SPEC-4.1.5).
    ResourcesTooLarge {
        /// Total bytes.
        bytes: u64,
    },
    /// A resource media type outside SPEC-4.1.5.
    ResourceMimeUnknown {
        /// Resource index.
        index: u32,
        /// The media type.
        mime: String,
    },
    /// An image resource without `w`/`h` (review 0002, decision 7).
    ImageSizeMissing {
        /// Resource index.
        index: u32,
    },
    /// Declared `w`/`h` differ from the image header.
    ImageSizeMismatch {
        /// Resource index.
        index: u32,
        /// Declared (w, h).
        declared: (u32, u32),
        /// From the header.
        actual: (u32, u32),
    },
    /// An image side over 8192 px (SPEC-6.5.1).
    ImageTooLarge {
        /// Resource index.
        index: u32,
        /// Declared (w, h).
        size: (u32, u32),
    },
    /// Bytes declared as an image whose header cannot be read.
    ImageHeaderInvalid {
        /// Resource index.
        index: u32,
    },
    /// More than 16 `net` origins (SPEC-8.2.1).
    NetOriginsTooMany {
        /// Origins listed.
        count: usize,
    },
    /// A `net` origin that is not `scheme://host[:port]`.
    NetOriginInvalid(String),
    /// `storage.quota_kb` over 16 384 (SPEC-8.2.1).
    StorageQuotaTooLarge {
        /// Requested quota.
        quota_kb: u32,
    },
    /// A `links` entry that is not `vy://` or `https://`.
    LinkInvalid(String),
    /// An `embeds` entry that is not `vy://`.
    EmbedInvalid(String),
    /// `source_url` is not `https://`.
    SourceUrlInvalid(String),
    /// `icon` or `skeleton` names a resource that does not exist.
    ResourceIndexOutOfRange {
        /// Field name.
        field: &'static str,
        /// The index.
        index: u32,
    },
    /// `min_size` has a zero side.
    MinSizeZero,
    /// The encoded manifest is over 64 KB (SPEC-4.1.3).
    ManifestTooLarge {
        /// Encoded size.
        bytes: usize,
    },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AbiMajor { found } => write!(f, "abi major {found} is not 1 (SPEC-4.3.1)"),
            Self::NameEmpty => f.write_str("name is empty (SPEC-8.1.1)"),
            Self::NameTooLong { chars } => {
                write!(
                    f,
                    "name is {chars} characters, at most {MAX_NAME_CHARS} (SPEC-8.1.1)"
                )
            }
            Self::SummaryTooLong { bytes } => {
                write!(
                    f,
                    "summary is {bytes} bytes, at most {MAX_SUMMARY_BYTES} (SPEC-8.1.1)"
                )
            }
            Self::LangInvalid(l) => write!(f, "lang {l:?} is not a BCP 47 tag (SPEC-8.1.1)"),
            Self::PublishedInvalid(p) => write!(f, "published {p:?} is not YYYY-MM-DD"),
            Self::FallbackTooLong { bytes } => write!(
                f,
                "fallback is {bytes} bytes, at most {MAX_FALLBACK_BYTES} (SPEC-8.1.2)"
            ),
            Self::MemoryMaxOutOfRange { pages } => {
                write!(
                    f,
                    "memory_max {pages} is not in 1..=4096 pages (SPEC-8.1.3)"
                )
            }
            Self::MemoryMaxBelowInitial { pages, initial } => write!(
                f,
                "memory_max {pages} is below the module's initial {initial} pages + 1 (SPEC-8.1.3)"
            ),
            Self::ModuleNotWasm(m) => write!(f, "module is not a wasm module with a memory: {m}"),
            Self::ModuleTooLarge { bytes } => {
                write!(
                    f,
                    "module is {bytes} bytes, at most {MAX_MODULE_BYTES} (SPEC-4.1.4)"
                )
            }
            Self::ModuleHashMismatch { actual } => {
                write!(
                    f,
                    "module hash mismatch, bytes hash to {actual} (SPEC-9.4.1)"
                )
            }
            Self::ModuleSizeMismatch { declared, actual } => {
                write!(f, "module size {declared} declared, {actual} actual")
            }
            Self::ResourceCountMismatch { declared, given } => {
                write!(
                    f,
                    "{declared} resources declared, {given} given (SPEC-4.1.1)"
                )
            }
            Self::ResourceHashMismatch { index, actual } => {
                write!(f, "resource {index} hash mismatch, bytes hash to {actual}")
            }
            Self::ResourceSizeMismatch {
                index,
                declared,
                actual,
            } => write!(
                f,
                "resource {index} size {declared} declared, {actual} actual"
            ),
            Self::ResourcesTooLarge { bytes } => write!(
                f,
                "resources total {bytes} bytes, at most {MAX_RESOURCES_BYTES} (SPEC-4.1.5)"
            ),
            Self::ResourceMimeUnknown { index, mime } => {
                write!(
                    f,
                    "resource {index} media type {mime:?} is not allowed (SPEC-4.1.5)"
                )
            }
            Self::ImageSizeMissing { index } => {
                write!(f, "image resource {index} lacks w/h")
            }
            Self::ImageSizeMismatch {
                index,
                declared,
                actual,
            } => write!(
                f,
                "image resource {index} declared {}x{}, header says {}x{}",
                declared.0, declared.1, actual.0, actual.1
            ),
            Self::ImageTooLarge { index, size } => write!(
                f,
                "image resource {index} is {}x{}, at most {} per side (SPEC-6.5.1)",
                size.0,
                size.1,
                limits::MAX_IMAGE_SIDE_PX
            ),
            Self::ImageHeaderInvalid { index } => {
                write!(f, "image resource {index} has no readable header")
            }
            Self::NetOriginsTooMany { count } => {
                write!(
                    f,
                    "{count} net origins, at most {MAX_NET_ORIGINS} (SPEC-8.2.1)"
                )
            }
            Self::NetOriginInvalid(o) => write!(f, "net origin {o:?} is not scheme://host[:port]"),
            Self::StorageQuotaTooLarge { quota_kb } => write!(
                f,
                "storage quota {quota_kb} KB, at most {MAX_STORAGE_QUOTA_KB} (SPEC-8.2.1)"
            ),
            Self::LinkInvalid(l) => write!(f, "link {l:?} is not vy:// or https://"),
            Self::EmbedInvalid(e) => write!(f, "embed {e:?} is not vy://"),
            Self::SourceUrlInvalid(u) => write!(f, "source_url {u:?} is not https://"),
            Self::ResourceIndexOutOfRange { field, index } => {
                write!(f, "{field} names resource {index}, which does not exist")
            }
            Self::MinSizeZero => f.write_str("min_size has a zero side"),
            Self::ManifestTooLarge { bytes } => {
                write!(
                    f,
                    "manifest encodes to {bytes} bytes, at most {MAX_MANIFEST_BYTES} (SPEC-4.1.3)"
                )
            }
        }
    }
}

/// Width and height from a PNG header (signature + IHDR), or `None`.
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 24 || bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let be = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    if be(8) != 13 {
        return None;
    }
    let (w, h) = (be(16), be(20));
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

/// Initial size of the module's linear memory in pages, from its memory
/// section (SPEC-4.2.4).
pub fn module_initial_pages(wasm: &[u8]) -> Result<u32, String> {
    use wasmparser::{Parser, Payload, TypeRef};
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.map_err(|e| e.to_string())? {
            Payload::MemorySection(reader) => {
                let first = reader
                    .into_iter()
                    .next()
                    .ok_or_else(|| "empty memory section".to_string())?
                    .map_err(|e| e.to_string())?;
                return u32::try_from(first.initial)
                    .map_err(|_| "initial memory does not fit 32 bits".to_string());
            }
            Payload::ImportSection(reader) => {
                for import in reader {
                    let import = import.map_err(|e| e.to_string())?;
                    if let TypeRef::Memory(m) = import.ty {
                        return u32::try_from(m.initial)
                            .map_err(|_| "initial memory does not fit 32 bits".to_string());
                    }
                }
            }
            _ => {}
        }
    }
    Err("no memory".to_string())
}

/// Loose BCP 47 syntax: a 2–8 letter primary subtag and optional 1–8
/// character alphanumeric subtags.
pub fn is_bcp47(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let Some(primary) = parts.next() else {
        return false;
    };
    if !(2..=8).contains(&primary.len()) || !primary.bytes().all(|b| b.is_ascii_alphabetic()) {
        return false;
    }
    parts.all(|p| (1..=8).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// `YYYY-MM-DD` with plausible month and day.
pub fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<u32> {
        if b[r.clone()].iter().all(u8::is_ascii_digit) {
            s[r].parse().ok()
        } else {
            None
        }
    };
    match (digits(0..4), digits(5..7), digits(8..10)) {
        (Some(_), Some(m), Some(d)) => (1..=12).contains(&m) && (1..=31).contains(&d),
        _ => false,
    }
}

/// `scheme://host[:port]` without path, query or fragment (SPEC-9.5.1).
pub fn is_origin(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "https" | "http" | "vy") || rest.is_empty() {
        return false;
    }
    let (host_ok, port) = if let Some(after) = rest.strip_prefix('[') {
        let Some((inner, tail)) = after.split_once(']') else {
            return false;
        };
        let port = match tail {
            "" => None,
            t => match t.strip_prefix(':') {
                Some(p) => Some(p),
                None => return false,
            },
        };
        (
            !inner.is_empty() && inner.bytes().all(|b| b.is_ascii_hexdigit() || b == b':'),
            port,
        )
    } else {
        let (host, port) = match rest.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (rest, None),
        };
        (
            !host.is_empty()
                && host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.'),
            port,
        )
    };
    let port_ok = match port {
        None => true,
        Some(p) => p.parse::<u16>().map(|n| n > 0).unwrap_or(false) && !p.starts_with('0'),
    };
    host_ok && port_ok
}

fn is_url_with(s: &str, schemes: &[&str]) -> bool {
    schemes.iter().any(|sch| {
        s.strip_prefix(sch)
            .map(|rest| {
                !rest.is_empty()
                    && !rest
                        .bytes()
                        .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
            })
            .unwrap_or(false)
    })
}

impl Manifest {
    /// Every reason this manifest does not describe `module` and
    /// `resources`; empty means the package is consistent. Checks hashes
    /// and sizes (SPEC-9.4.1, SPEC-4.1.1), image headers against `w`/`h`,
    /// the memory cap against the module's initial memory (SPEC-8.1.3), the
    /// ABI major (SPEC-4.3.1) and every size limit of §4 and §8.
    ///
    /// Signature validity is a separate question ([`Manifest::verify`]):
    /// an unsigned or badly signed package is still a package, run in
    /// restricted mode (SPEC-7.4.3).
    pub fn validate(&self, module: &[u8], resources: &[impl AsRef<[u8]>]) -> Vec<Problem> {
        let mut p = Vec::new();

        if self.abi.major != vy_abi::ABI_MAJOR {
            p.push(Problem::AbiMajor {
                found: self.abi.major,
            });
        }
        let chars = self.name.chars().count();
        if chars == 0 {
            p.push(Problem::NameEmpty);
        } else if chars > MAX_NAME_CHARS {
            p.push(Problem::NameTooLong { chars });
        }
        if self.summary.len() > MAX_SUMMARY_BYTES {
            p.push(Problem::SummaryTooLong {
                bytes: self.summary.len(),
            });
        }
        if !is_bcp47(&self.lang) {
            p.push(Problem::LangInvalid(self.lang.clone()));
        }
        if !is_date(&self.published) {
            p.push(Problem::PublishedInvalid(self.published.clone()));
        }
        if let Some(f) = &self.fallback {
            if f.len() > MAX_FALLBACK_BYTES {
                p.push(Problem::FallbackTooLong { bytes: f.len() });
            }
        }

        // Memory (SPEC-8.1.3, SPEC-4.2.4).
        if self.memory_max == 0 || self.memory_max > limits::MAX_MEMORY_PAGES {
            p.push(Problem::MemoryMaxOutOfRange {
                pages: self.memory_max,
            });
        }
        match module_initial_pages(module) {
            Ok(initial) => {
                if self.memory_max < initial.saturating_add(1) {
                    p.push(Problem::MemoryMaxBelowInitial {
                        pages: self.memory_max,
                        initial,
                    });
                }
            }
            Err(e) => p.push(Problem::ModuleNotWasm(e)),
        }

        // Module bytes (SPEC-4.1.4, SPEC-9.4.1).
        if module.len() > MAX_MODULE_BYTES {
            p.push(Problem::ModuleTooLarge {
                bytes: module.len(),
            });
        }
        if self.module.size != module.len() as u64 {
            p.push(Problem::ModuleSizeMismatch {
                declared: self.module.size,
                actual: module.len() as u64,
            });
        }
        let actual = Hash::of(module);
        if self.module.hash != actual {
            p.push(Problem::ModuleHashMismatch { actual });
        }

        // Resources (SPEC-4.1.1, SPEC-4.1.5, SPEC-6.5.1, decision 7).
        if self.resources.len() != resources.len() {
            p.push(Problem::ResourceCountMismatch {
                declared: self.resources.len(),
                given: resources.len(),
            });
        }
        let mut total: u64 = 0;
        for (i, r) in self.resources.iter().enumerate() {
            let index = i as u32 + 1;
            total = total.saturating_add(r.size);
            if !RESOURCE_MIMES.contains(&r.mime.as_str()) {
                p.push(Problem::ResourceMimeUnknown {
                    index,
                    mime: r.mime.clone(),
                });
            }
            let bytes = resources.get(i).map(AsRef::as_ref);
            if let Some(b) = bytes {
                if r.size != b.len() as u64 {
                    p.push(Problem::ResourceSizeMismatch {
                        index,
                        declared: r.size,
                        actual: b.len() as u64,
                    });
                }
                let actual = Hash::of(b);
                if r.hash != actual {
                    p.push(Problem::ResourceHashMismatch { index, actual });
                }
            }
            if r.is_image() {
                match (r.w, r.h) {
                    (Some(w), Some(h)) => {
                        if w == 0
                            || h == 0
                            || w > limits::MAX_IMAGE_SIDE_PX
                            || h > limits::MAX_IMAGE_SIDE_PX
                        {
                            p.push(Problem::ImageTooLarge {
                                index,
                                size: (w, h),
                            });
                        }
                        if r.mime == "image/png" {
                            if let Some(b) = bytes {
                                match png_dimensions(b) {
                                    Some(actual) if actual != (w, h) => {
                                        p.push(Problem::ImageSizeMismatch {
                                            index,
                                            declared: (w, h),
                                            actual,
                                        })
                                    }
                                    Some(_) => {}
                                    None => p.push(Problem::ImageHeaderInvalid { index }),
                                }
                            }
                        }
                    }
                    _ => p.push(Problem::ImageSizeMissing { index }),
                }
            }
        }
        if total > MAX_RESOURCES_BYTES {
            p.push(Problem::ResourcesTooLarge { bytes: total });
        }

        // Capabilities (SPEC-8.2.1).
        if let Some(net) = &self.capabilities.net {
            if net.origins.len() > MAX_NET_ORIGINS {
                p.push(Problem::NetOriginsTooMany {
                    count: net.origins.len(),
                });
            }
            for o in &net.origins {
                if !is_origin(o) {
                    p.push(Problem::NetOriginInvalid(o.clone()));
                }
            }
        }
        if let Some(s) = &self.capabilities.storage {
            if s.quota_kb > MAX_STORAGE_QUOTA_KB {
                p.push(Problem::StorageQuotaTooLarge {
                    quota_kb: s.quota_kb,
                });
            }
        }

        // Links and references.
        for l in &self.links {
            if !is_url_with(l, &["vy://", "https://"]) {
                p.push(Problem::LinkInvalid(l.clone()));
            }
        }
        for e in &self.embeds {
            if !is_url_with(e, &["vy://"]) {
                p.push(Problem::EmbedInvalid(e.clone()));
            }
        }
        if let Some(u) = &self.source_url {
            if !is_url_with(u, &["https://"]) {
                p.push(Problem::SourceUrlInvalid(u.clone()));
            }
        }
        for (field, index) in [("icon", self.icon), ("skeleton", self.skeleton)] {
            if let Some(index) = index {
                if index == 0 || index as usize > self.resources.len() {
                    p.push(Problem::ResourceIndexOutOfRange { field, index });
                }
            }
        }
        if let Some(s) = &self.min_size {
            if s.w == 0 || s.h == 0 {
                p.push(Problem::MinSizeZero);
            }
        }

        // Encoded size (SPEC-4.1.3): the signature adds a fixed 85 bytes.
        let encoded = match self.encode() {
            Ok(b) => b.len(),
            Err(_) => self.signed_bytes().len() + SIG_FIELD_BYTES,
        };
        if encoded > MAX_MANIFEST_BYTES {
            p.push(Problem::ManifestTooLarge { bytes: encoded });
        }

        p
    }
}

/// Encoded size of the `sig` field: key (4) + map head (1) + `bytes` key
/// (6) + 64-byte string (66) + `key_id` key (7) + 16-character id (17).
const SIG_FIELD_BYTES: usize = 4 + 1 + 6 + 66 + 7 + 17;
