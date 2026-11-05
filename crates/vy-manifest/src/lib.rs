//! Manifest: the typed model of SPEC §8, its readable JSON form and the
//! validation every package goes through (SPEC §4, §8).
//!
//! The wire form is canonical CBOR (SPEC-4.1.3); the repository keeps
//! manifests as JSON under `conformance/manifests/`, which is what the
//! publisher turns into CBOR (M3) and what `vy-cli conformance` reads today.
//! Signatures (SPEC-7.4.*) arrive in M4.
#![deny(missing_docs)]

pub mod json;

use json::Value;
use std::collections::BTreeMap;
use std::fmt;

/// Capabilities defined in 1.0 (SPEC-8.2.1), in id order (`vy_abi::cap`).
pub const CAPABILITIES: &[&str] = &vy_abi::cap::NAMES;

/// Maximum manifest size in bytes (SPEC-4.1.3).
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Maximum uncompressed module size in bytes (SPEC-4.1.4).
pub const MAX_MODULE_BYTES: usize = 8 * 1024 * 1024;
/// Maximum total resource size in bytes (SPEC-4.1.5).
pub const MAX_RESOURCES_BYTES: usize = 32 * 1024 * 1024;
/// Longest `name` in characters (SPEC-8.1.1).
pub const MAX_NAME_CHARS: usize = 80;
/// Longest `summary` in bytes (SPEC-8.1.1).
pub const MAX_SUMMARY_BYTES: usize = 2 * 1024;
/// Longest `fallback` in bytes (SPEC-8.1.2).
pub const MAX_FALLBACK_BYTES: usize = 16 * 1024;
/// Most `net` origins (SPEC-8.2.1).
pub const MAX_NET_ORIGINS: usize = 16;
/// Largest `storage` quota in KB (SPEC-8.2.1).
pub const MAX_STORAGE_QUOTA_KB: u32 = 16_384;
/// Resource media types of 1.0 (SPEC-4.1.5): PNG, WebP, AVIF, WOFF2 or raw.
pub const RESOURCE_TYPES: [&str; 5] = [
    "image/png",
    "image/webp",
    "image/avif",
    "font/woff2",
    "application/octet-stream",
];
/// The optional fields of SPEC-8.1.2 whose shape 1.0 leaves open; kept as
/// parsed and otherwise ignored.
const OPEN_FIELDS: [&str; 4] = ["icon", "skeleton", "chunks", "embeds"];

/// Why a manifest was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestError {
    /// The field at fault, `manifest` for the document as a whole.
    pub field: String,
    /// What is wrong, with the spec sentence that says so.
    pub message: String,
}

impl ManifestError {
    fn new(field: &str, message: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "manifest `{}`: {}", self.field, self.message)
    }
}

impl std::error::Error for ManifestError {}

/// `module` (SPEC-8.1.1): what the package's one module hashes to and weighs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleRef {
    /// `sha256:<hex>`, or a placeholder before `vy publish` fills it in.
    pub hash: String,
    /// Uncompressed size in bytes (SPEC-4.1.4).
    pub size: u64,
}

/// One entry of `resources` (SPEC-8.1.2); `resources[i]` is resource id
/// `i + 1`, id 0 is the module (SPEC-4.5.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    /// Path inside the package.
    pub path: String,
    /// Media type, one of [`RESOURCE_TYPES`].
    pub media_type: String,
    /// `sha256:<hex>`.
    pub hash: String,
    /// Size in bytes, when stated.
    pub size: Option<u64>,
    /// Width and height in pixels; required for images, absent otherwise
    /// (SPEC-8.1.2). The module learns its image sizes from here at build
    /// time; the host checks the decoded image against them (SPEC-6.5.1).
    pub pixels: Option<(u32, u32)>,
}

impl Resource {
    /// True for `image/*` resources.
    pub fn is_image(&self) -> bool {
        self.media_type.starts_with("image/")
    }
}

/// The parameters of one requested capability (SPEC-8.2.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Capability {
    /// `net`: the origins the page may fetch from (SPEC-5.2.8).
    Net {
        /// At most 16.
        origins: Vec<String>,
    },
    /// `storage`: the quota asked for in KB, at most 16 384.
    Storage {
        /// Quota in KB.
        quota_kb: u32,
    },
    /// `clipboard`.
    Clipboard,
    /// `hires_time`.
    HiresTime,
    /// `ime`.
    Ime,
    /// `persist_frame`.
    PersistFrame,
}

/// `sig` (SPEC-7.4.1): key id and signature bytes; verified in M4.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    /// Which of the publisher's keys signed.
    pub key_id: String,
    /// `ed25519:<base64>`, or a placeholder.
    pub bytes: String,
}

/// A parsed and validated manifest (SPEC §8).
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// `"MAJOR.MINOR"` (SPEC-4.3.1).
    pub abi: String,
    /// At most 80 characters.
    pub name: String,
    /// At most 2 KB.
    pub summary: String,
    /// BCP 47 tag.
    pub lang: String,
    /// The one module.
    pub module: ModuleRef,
    /// In wasm pages, at most 4096 (SPEC-8.1.3).
    pub memory_max: u32,
    /// Publication date.
    pub published: String,
    /// Signature.
    pub sig: Signature,
    /// Resources in id order (SPEC-4.5.1).
    pub resources: Vec<Resource>,
    /// Requested capabilities by name (SPEC-8.2.1).
    pub capabilities: BTreeMap<String, Capability>,
    /// Addresses the page links to.
    pub links: Vec<String>,
    /// Markdown shown instead of the module (SPEC-8.3.1), at most 16 KB.
    pub fallback: Option<String>,
    /// Smallest window the page supports, logical pixels (SPEC-8.3.1).
    pub min_size: Option<(u32, u32)>,
    /// Where the source lives.
    pub source_url: Option<String>,
    /// `icon`, `skeleton`, `chunks` and `embeds`, as parsed.
    pub open: Vec<(String, Value)>,
}

fn string(v: &Value, field: &str) -> Result<String, ManifestError> {
    v.as_str()
        .map(str::to_owned)
        .ok_or_else(|| ManifestError::new(field, "must be a string"))
}

fn uint(v: &Value, field: &str) -> Result<u64, ManifestError> {
    v.as_u64()
        .ok_or_else(|| ManifestError::new(field, "must be a non-negative integer"))
}

fn u32_field(v: &Value, field: &str) -> Result<u32, ManifestError> {
    u32::try_from(uint(v, field)?).map_err(|_| ManifestError::new(field, "too large"))
}

fn object<'a>(v: &'a Value, field: &str) -> Result<&'a [(String, Value)], ManifestError> {
    v.as_object()
        .ok_or_else(|| ManifestError::new(field, "must be an object"))
}

fn array<'a>(v: &'a Value, field: &str) -> Result<&'a [Value], ManifestError> {
    v.as_array()
        .ok_or_else(|| ManifestError::new(field, "must be an array"))
}

fn only_keys(
    members: &[(String, Value)],
    allowed: &[&str],
    field: &str,
) -> Result<(), ManifestError> {
    for (k, _) in members {
        if !allowed.contains(&k.as_str()) {
            return Err(ManifestError::new(
                field,
                format!("unknown field `{k}` (SPEC-8.1.2 lists the optional fields)"),
            ));
        }
    }
    Ok(())
}

impl Manifest {
    /// Parse the JSON form and validate it against SPEC §4 and §8.
    pub fn from_json(text: &str) -> Result<Self, ManifestError> {
        let v = Value::parse(text).map_err(|e| ManifestError::new("manifest", e.to_string()))?;
        Self::from_value(&v)
    }

    /// Build from a parsed value and validate (SPEC-8.1.1, SPEC-8.1.2,
    /// SPEC-8.1.3, SPEC-8.2.1, SPEC-4.3.1, SPEC-4.1.5).
    pub fn from_value(v: &Value) -> Result<Self, ManifestError> {
        let members = object(v, "manifest")?;
        const REQUIRED: [&str; 8] = [
            "abi",
            "name",
            "summary",
            "lang",
            "module",
            "memory_max",
            "published",
            "sig",
        ];
        const OPTIONAL: [&str; 11] = [
            "resources",
            "capabilities",
            "links",
            "fallback",
            "min_size",
            "icon",
            "source_url",
            "skeleton",
            "chunks",
            "embeds",
            "net",
        ];
        let allowed: Vec<&str> = REQUIRED.iter().chain(OPTIONAL.iter()).copied().collect();
        only_keys(members, &allowed, "manifest")?;
        if v.get("net").is_some() {
            return Err(ManifestError::new(
                "net",
                "capability parameters live under `capabilities` (SPEC-8.2.1)",
            ));
        }
        let get = |key: &str| -> Result<&Value, ManifestError> {
            v.get(key)
                .ok_or_else(|| ManifestError::new(key, "required field is missing (SPEC-8.1.1)"))
        };

        let abi = string(get("abi")?, "abi")?;
        let (major, _) = parse_abi(&abi)?;
        if major != vy_abi::ABI_MAJOR {
            return Err(ManifestError::new(
                "abi",
                format!(
                    "major version {major} is not {} (SPEC-4.3.1)",
                    vy_abi::ABI_MAJOR
                ),
            ));
        }
        let name = string(get("name")?, "name")?;
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
            return Err(ManifestError::new(
                "name",
                "1 to 80 characters (SPEC-8.1.1)",
            ));
        }
        let summary = string(get("summary")?, "summary")?;
        if summary.len() > MAX_SUMMARY_BYTES {
            return Err(ManifestError::new("summary", "at most 2 KB (SPEC-8.1.1)"));
        }
        let lang = string(get("lang")?, "lang")?;
        if !is_bcp47(&lang) {
            return Err(ManifestError::new(
                "lang",
                "must be a BCP 47 tag (SPEC-8.1.1)",
            ));
        }
        let module = {
            let m = get("module")?;
            let mm = object(m, "module")?;
            only_keys(mm, &["hash", "size"], "module")?;
            ModuleRef {
                hash: string(
                    m.get("hash")
                        .ok_or_else(|| ManifestError::new("module.hash", "missing"))?,
                    "module.hash",
                )?,
                size: uint(
                    m.get("size")
                        .ok_or_else(|| ManifestError::new("module.size", "missing"))?,
                    "module.size",
                )?,
            }
        };
        if module.size as usize > MAX_MODULE_BYTES {
            return Err(ManifestError::new("module.size", "over 8 MB (SPEC-4.1.4)"));
        }
        let memory_max = u32_field(get("memory_max")?, "memory_max")?;
        if memory_max == 0 || memory_max > vy_abi::limits::MAX_MEMORY_PAGES {
            return Err(ManifestError::new(
                "memory_max",
                format!(
                    "must be 1 to {} pages (SPEC-8.1.3)",
                    vy_abi::limits::MAX_MEMORY_PAGES
                ),
            ));
        }
        let published = string(get("published")?, "published")?;
        let sig = {
            let s = get("sig")?;
            let sm = object(s, "sig")?;
            only_keys(sm, &["key_id", "bytes"], "sig")?;
            Signature {
                key_id: string(
                    s.get("key_id")
                        .ok_or_else(|| ManifestError::new("sig.key_id", "missing"))?,
                    "sig.key_id",
                )?,
                bytes: string(
                    s.get("bytes")
                        .ok_or_else(|| ManifestError::new("sig.bytes", "missing"))?,
                    "sig.bytes",
                )?,
            }
        };

        let mut resources = Vec::new();
        let mut total = 0u64;
        if let Some(list) = v.get("resources") {
            for (i, r) in array(list, "resources")?.iter().enumerate() {
                let field = format!("resources[{i}]");
                let rm = object(r, &field)?;
                only_keys(rm, &["path", "type", "hash", "size", "w", "h"], &field)?;
                let need = |key: &str| -> Result<&Value, ManifestError> {
                    r.get(key)
                        .ok_or_else(|| ManifestError::new(&format!("{field}.{key}"), "missing"))
                };
                let media_type = string(need("type")?, &format!("{field}.type"))?;
                if !RESOURCE_TYPES.contains(&media_type.as_str()) {
                    return Err(ManifestError::new(
                        &format!("{field}.type"),
                        "must be image/png, image/webp, image/avif, font/woff2 or application/octet-stream (SPEC-4.1.5)",
                    ));
                }
                let size = r
                    .get("size")
                    .map(|s| uint(s, &format!("{field}.size")))
                    .transpose()?;
                total += size.unwrap_or(0);
                let w = r
                    .get("w")
                    .map(|w| u32_field(w, &format!("{field}.w")))
                    .transpose()?;
                let h = r
                    .get("h")
                    .map(|h| u32_field(h, &format!("{field}.h")))
                    .transpose()?;
                let pixels = match (media_type.starts_with("image/"), w, h) {
                    (true, Some(w), Some(h)) if w > 0 && h > 0 => Some((w, h)),
                    (true, _, _) => {
                        return Err(ManifestError::new(
                            &field,
                            "an image needs `w` and `h` in pixels, both above 0 (SPEC-8.1.2)",
                        ))
                    }
                    (false, None, None) => None,
                    (false, _, _) => {
                        return Err(ManifestError::new(
                            &field,
                            "`w` and `h` belong to images only (SPEC-8.1.2)",
                        ))
                    }
                };
                if let Some((w, h)) = pixels {
                    let max = vy_abi::limits::MAX_IMAGE_SIDE_PX;
                    if w > max || h > max {
                        return Err(ManifestError::new(
                            &field,
                            format!("{w}x{h} px exceeds {max} px per side (SPEC-6.5.1)"),
                        ));
                    }
                }
                resources.push(Resource {
                    path: string(need("path")?, &format!("{field}.path"))?,
                    media_type,
                    hash: string(need("hash")?, &format!("{field}.hash"))?,
                    size,
                    pixels,
                });
            }
        }
        if total as usize > MAX_RESOURCES_BYTES {
            return Err(ManifestError::new(
                "resources",
                "over 32 MB in all (SPEC-4.1.5)",
            ));
        }

        let mut capabilities = BTreeMap::new();
        if let Some(caps) = v.get("capabilities") {
            for (name, params) in object(caps, "capabilities")? {
                let field = format!("capabilities.{name}");
                let pm = object(params, &field)?;
                let cap = match name.as_str() {
                    "net" => {
                        only_keys(pm, &["origins"], &field)?;
                        let origins = params
                            .get("origins")
                            .map(|o| array(o, &format!("{field}.origins")))
                            .transpose()?
                            .unwrap_or(&[])
                            .iter()
                            .map(|o| string(o, &format!("{field}.origins")))
                            .collect::<Result<Vec<_>, _>>()?;
                        if origins.len() > MAX_NET_ORIGINS {
                            return Err(ManifestError::new(
                                &format!("{field}.origins"),
                                "at most 16 origins (SPEC-8.2.1)",
                            ));
                        }
                        Capability::Net { origins }
                    }
                    "storage" => {
                        only_keys(pm, &["quota_kb"], &field)?;
                        let quota_kb = params
                            .get("quota_kb")
                            .map(|q| u32_field(q, &format!("{field}.quota_kb")))
                            .transpose()?
                            .unwrap_or(0);
                        if quota_kb > MAX_STORAGE_QUOTA_KB {
                            return Err(ManifestError::new(
                                &format!("{field}.quota_kb"),
                                "at most 16384 KB (SPEC-8.2.1)",
                            ));
                        }
                        Capability::Storage { quota_kb }
                    }
                    "clipboard" | "hires_time" | "ime" | "persist_frame" => {
                        only_keys(pm, &[], &field)?;
                        match name.as_str() {
                            "clipboard" => Capability::Clipboard,
                            "hires_time" => Capability::HiresTime,
                            "ime" => Capability::Ime,
                            _ => Capability::PersistFrame,
                        }
                    }
                    other => {
                        return Err(ManifestError::new(
                            "capabilities",
                            format!("`{other}` is not a capability of 1.0 (SPEC-8.2.1)"),
                        ))
                    }
                };
                capabilities.insert(name.clone(), cap);
            }
        }

        let links = v
            .get("links")
            .map(|l| array(l, "links"))
            .transpose()?
            .unwrap_or(&[])
            .iter()
            .map(|l| string(l, "links"))
            .collect::<Result<Vec<_>, _>>()?;
        let fallback = v
            .get("fallback")
            .map(|f| string(f, "fallback"))
            .transpose()?;
        if fallback
            .as_ref()
            .is_some_and(|f| f.len() > MAX_FALLBACK_BYTES)
        {
            return Err(ManifestError::new("fallback", "at most 16 KB (SPEC-8.1.2)"));
        }
        let min_size = match v.get("min_size") {
            None => None,
            Some(m) => {
                let a = array(m, "min_size")?;
                if a.len() != 2 {
                    return Err(ManifestError::new("min_size", "must be [width, height]"));
                }
                Some((u32_field(&a[0], "min_size")?, u32_field(&a[1], "min_size")?))
            }
        };
        let source_url = v
            .get("source_url")
            .map(|s| string(s, "source_url"))
            .transpose()?;
        let open = members
            .iter()
            .filter(|(k, _)| OPEN_FIELDS.contains(&k.as_str()))
            .cloned()
            .collect();

        Ok(Self {
            abi,
            name,
            summary,
            lang,
            module,
            memory_max,
            published,
            sig,
            resources,
            capabilities,
            links,
            fallback,
            min_size,
            source_url,
            open,
        })
    }

    /// `(MAJOR, MINOR)` of the `abi` field (SPEC-4.3.1).
    pub fn abi_version(&self) -> (u32, u32) {
        parse_abi(&self.abi).expect("validated at construction")
    }

    /// True when a client of ABI 1.`client_minor` runs this module
    /// (SPEC-4.3.2): the manifest's minor is at most the client's.
    pub fn runs_on(&self, client_minor: u32) -> bool {
        let (major, minor) = self.abi_version();
        major == vy_abi::ABI_MAJOR && minor <= client_minor
    }

    /// True when the manifest requests capability `name` (SPEC-8.2.2: what
    /// is not requested can never be granted).
    pub fn requests(&self, name: &str) -> bool {
        self.capabilities.contains_key(name)
    }

    /// The resource with id `id`; id 0 is the module and never a resource
    /// (SPEC-4.5.1).
    pub fn resource(&self, id: u16) -> Option<&Resource> {
        usize::from(id)
            .checked_sub(1)
            .and_then(|i| self.resources.get(i))
    }

    /// SPEC-8.1.3: `memory_max` must leave the module room to grow at all,
    /// at least one page above its initial memory, or the first allocation
    /// traps the page in its first frame.
    pub fn check_memory(&self, initial_pages: u32) -> Result<(), ManifestError> {
        if self.memory_max < initial_pages.saturating_add(1) {
            return Err(ManifestError::new(
                "memory_max",
                format!(
                    "{} pages, but the module starts with {initial_pages}; memory_max must be at least initial + 1 (SPEC-8.1.3)",
                    self.memory_max
                ),
            ));
        }
        Ok(())
    }

    /// SPEC-8.1.2, SPEC-6.5.1: every image resource's `w` and `h` match the
    /// header of the file `read` returns for its path (`None` when the file
    /// cannot be read). Only PNG headers are read before M3.
    pub fn check_resource_sizes(
        &self,
        mut read: impl FnMut(&str) -> Option<Vec<u8>>,
    ) -> Result<(), ManifestError> {
        for (i, r) in self.resources.iter().enumerate() {
            let Some(declared) = r.pixels else {
                continue;
            };
            let field = format!("resources[{i}]");
            let bytes = read(&r.path)
                .ok_or_else(|| ManifestError::new(&field, format!("cannot read `{}`", r.path)))?;
            if r.media_type != "image/png" {
                continue;
            }
            let actual = png_size(&bytes)
                .ok_or_else(|| ManifestError::new(&field, format!("`{}` is not a PNG", r.path)))?;
            if actual != declared {
                return Err(ManifestError::new(
                    &field,
                    format!(
                        "`{}` is {}x{} px but the manifest says {}x{} (SPEC-8.1.2)",
                        r.path, actual.0, actual.1, declared.0, declared.1
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// `"MAJOR.MINOR"` as two numbers (SPEC-4.3.1).
fn parse_abi(abi: &str) -> Result<(u32, u32), ManifestError> {
    let bad = || ManifestError::new("abi", "must be \"MAJOR.MINOR\" (SPEC-4.3.1)");
    let (major, minor) = abi.split_once('.').ok_or_else(bad)?;
    Ok((
        major.parse().map_err(|_| bad())?,
        minor.parse().map_err(|_| bad())?,
    ))
}

/// A conservative BCP 47 check: 2–8 letter language subtag, then optional
/// `-` separated alphanumeric subtags of 1–8 characters.
fn is_bcp47(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let Some(lang) = parts.next() else {
        return false;
    };
    let ok_lang = (2..=8).contains(&lang.len()) && lang.chars().all(|c| c.is_ascii_alphabetic());
    ok_lang
        && parts.all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// Width and height from a PNG's IHDR chunk, without decoding anything.
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    fn manifest(name: &str) -> Manifest {
        let path = root().join(format!("conformance/manifests/{name}.json"));
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        Manifest::from_json(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    /// A minimal valid manifest as JSON text, with `extra` members spliced in.
    fn minimal(extra: &str) -> String {
        format!(
            r#"{{"abi": "1.0", "name": "x", "summary": "s", "lang": "sv", "module": {{"hash": "sha256:0", "size": 1}}, "memory_max": 32, "published": "2026-11-05", "sig": {{"key_id": "dev", "bytes": "ed25519:0"}}{extra}}}"#
        )
    }

    /// SPEC-8.2.1: the six capabilities of 1.0, and the manifest form is a
    /// map from name to parameter object: `net` carries its origins,
    /// `storage` its quota, the rest an empty object; limits and unknown
    /// names are refused; a top-level `net` is the old form and refused.
    #[test]
    fn spec_8_2_1_capability_list() {
        assert_eq!(CAPABILITIES.len(), 6);
        for c in [
            "net",
            "storage",
            "clipboard",
            "hires_time",
            "ime",
            "persist_frame",
        ] {
            assert!(CAPABILITIES.contains(&c));
        }
        let m = Manifest::from_json(&minimal(
            r#", "capabilities": {"net": {"origins": ["https://a.example"]}, "storage": {"quota_kb": 16}, "ime": {}, "clipboard": {}, "hires_time": {}, "persist_frame": {}}"#,
        ))
        .unwrap();
        assert_eq!(
            m.capabilities["net"],
            Capability::Net {
                origins: vec!["https://a.example".into()]
            }
        );
        assert_eq!(
            m.capabilities["storage"],
            Capability::Storage { quota_kb: 16 }
        );
        assert_eq!(m.capabilities["ime"], Capability::Ime);
        assert!(m.requests("persist_frame") && !m.requests("file"));
        let chat = manifest("chat");
        assert!(
            matches!(&chat.capabilities["net"], Capability::Net { origins } if origins == &["https://chat.example"])
        );
        assert_eq!(
            manifest("settings").capabilities["storage"],
            Capability::Storage { quota_kb: 16 }
        );
        for bad in [
            r#", "capabilities": ["ime"]"#,
            r#", "capabilities": {"file": {}}"#,
            r#", "capabilities": {"ime": {"x": 1}}"#,
            r#", "capabilities": {"net": {"origins": "https://a.example"}}"#,
            r#", "capabilities": {"storage": {"quota_kb": 16385}}"#,
            r#", "net": {"origins": []}"#,
        ] {
            assert!(Manifest::from_json(&minimal(bad)).is_err(), "{bad}");
        }
        let many: Vec<String> = (0..17)
            .map(|i| format!("\"https://{i}.example\""))
            .collect();
        let bad = format!(
            r#", "capabilities": {{"net": {{"origins": [{}]}}}}"#,
            many.join(",")
        );
        assert!(Manifest::from_json(&minimal(&bad)).is_err(), "17 origins");
    }

    /// SPEC-4.1.3, SPEC-4.1.4, SPEC-4.1.5: size limits.
    #[test]
    fn spec_4_1_3_size_limits() {
        assert_eq!(MAX_MANIFEST_BYTES, 65_536);
        assert_eq!(MAX_MODULE_BYTES, 8_388_608);
        assert_eq!(MAX_RESOURCES_BYTES, 33_554_432);
        assert!(Manifest::from_json(&minimal(
            r#", "resources": [{"path": "a", "type": "application/octet-stream", "hash": "sha256:0", "size": 33554433}]"#
        ))
        .is_err());
        assert!(Manifest::from_json(&minimal(
            r#", "resources": [{"path": "a", "type": "text/html", "hash": "sha256:0"}]"#
        ))
        .is_err());
    }

    /// SPEC-4.1.1: a package is one manifest, exactly one module, zero or more resources.
    #[test]
    fn spec_4_1_1_package_shape() {
        let hello = manifest("hello");
        assert_eq!(hello.module.hash, "sha256:FYLLS-I-AV-vy-publish");
        assert!(hello.resources.is_empty());
        assert_eq!(manifest("gallery").resources.len(), 8);
        assert!(
            Manifest::from_json(r#"{"abi": "1.0"}"#).is_err(),
            "no module"
        );
    }

    /// SPEC-4.1.2, SPEC-2.1.2: package identity is SHA-256 of canonical CBOR.
    #[test]
    #[ignore = "M3: canonical CBOR + hash"]
    fn spec_4_1_2_identity_is_hash_of_canonical_cbor() {
        todo!()
    }

    /// SPEC-4.1.3: non-canonical CBOR or > 64 KB is rejected.
    #[test]
    #[ignore = "M3: canonical CBOR check"]
    fn spec_4_1_3_rejects_non_canonical() {
        todo!()
    }

    /// SPEC-4.3.1: unknown ABI major version is rejected before loading.
    #[test]
    fn spec_4_3_1_unknown_major_rejected() {
        let m = Manifest::from_json(&minimal("")).unwrap();
        assert_eq!(m.abi_version(), (1, 0));
        for bad in ["2.0", "0.9", "1", "x.y", "1.0.0"] {
            let text = minimal("").replace("\"1.0\"", &format!("\"{bad}\""));
            assert!(Manifest::from_json(&text).is_err(), "{bad}");
        }
    }

    /// SPEC-4.3.2: a 1.y module runs on a 1.x client when y <= x.
    #[test]
    fn spec_4_3_2_minor_compat() {
        let one_one = Manifest::from_json(&minimal("").replace("\"1.0\"", "\"1.1\"")).unwrap();
        assert_eq!(one_one.abi_version(), (1, 1));
        assert!(!one_one.runs_on(0), "a 1.0 client cannot run a 1.1 module");
        assert!(one_one.runs_on(1) && one_one.runs_on(2));
        assert!(manifest("hello").runs_on(vy_abi::ABI_MINOR));
    }

    /// SPEC-4.4.1: package media type and zstd+tar framing.
    #[test]
    #[ignore = "M3: package unpack"]
    fn spec_4_4_1_package_framing() {
        todo!()
    }

    /// SPEC-4.5.1: resource index 0 is the module; `resources[i]` is id i + 1.
    #[test]
    fn spec_4_5_1_resource_index_zero_is_module() {
        let shop = manifest("shop");
        assert!(shop.resource(0).is_none(), "id 0 is the module");
        assert_eq!(shop.resource(1).unwrap().path, "assets/01-mugg.png");
        assert_eq!(shop.resource(8).unwrap().path, "assets/08-plad.png");
        assert!(shop.resource(9).is_none());
    }

    /// SPEC-8.1.1: required fields are enforced, with their limits.
    #[test]
    fn spec_8_1_1_required_fields() {
        let ok = Manifest::from_json(&minimal("")).unwrap();
        assert_eq!(ok.name, "x");
        let full = minimal("");
        for field in [
            "\"abi\": \"1.0\", ",
            "\"name\": \"x\", ",
            "\"summary\": \"s\", ",
            "\"lang\": \"sv\", ",
            "\"module\": {\"hash\": \"sha256:0\", \"size\": 1}, ",
            "\"memory_max\": 32, ",
            "\"published\": \"2026-11-05\", ",
            ", \"sig\": {\"key_id\": \"dev\", \"bytes\": \"ed25519:0\"}",
        ] {
            assert!(full.contains(field), "{field}");
            let e = Manifest::from_json(&full.replace(field, "")).unwrap_err();
            assert!(e.message.contains("SPEC-8.1.1"), "{field}: {e}");
        }
        let long_name = "a".repeat(81);
        assert!(
            Manifest::from_json(&minimal("").replace("\"x\"", &format!("\"{long_name}\"")))
                .is_err()
        );
        assert!(Manifest::from_json(
            &minimal("").replace("\"s\"", &format!("\"{}\"", "s".repeat(2049)))
        )
        .is_err());
        assert!(Manifest::from_json(&minimal("").replace("\"sv\"", "\"not a tag!\"")).is_err());
        assert!(Manifest::from_json(&minimal("").replace("\"sv\"", "\"sv-SE\"")).is_ok());
    }

    /// SPEC-8.1.2: optional fields are accepted, unknown fields rejected,
    /// and an image resource must carry `w` and `h` while a raw one must not.
    #[test]
    fn spec_8_1_2_optional_fields() {
        let m = Manifest::from_json(&minimal(
            r##", "resources": [{"path": "a.png", "type": "image/png", "hash": "sha256:0", "w": 320, "h": 200}, {"path": "b.bin", "type": "application/octet-stream", "hash": "sha256:1", "size": 10}], "links": ["vy://x/"], "fallback": "# x", "min_size": [320, 240], "icon": "icon.png", "source_url": "https://x/", "skeleton": {"a": 1}, "chunks": [], "embeds": null"##,
        ))
        .unwrap();
        assert_eq!(m.resources[0].pixels, Some((320, 200)));
        assert!(m.resources[0].is_image());
        assert_eq!(m.resources[1].pixels, None);
        assert_eq!(m.links, vec!["vy://x/".to_string()]);
        assert_eq!(m.min_size, Some((320, 240)));
        assert_eq!(
            m.open.len(),
            4,
            "icon, skeleton, chunks, embeds kept as parsed"
        );
        for bad in [
            r#", "colour": "blue""#,
            r#", "resources": [{"path": "a.png", "type": "image/png", "hash": "sha256:0"}]"#,
            r#", "resources": [{"path": "a.png", "type": "image/png", "hash": "sha256:0", "w": 320}]"#,
            r#", "resources": [{"path": "a.png", "type": "image/png", "hash": "sha256:0", "w": 0, "h": 1}]"#,
            r#", "resources": [{"path": "a.png", "type": "image/png", "hash": "sha256:0", "w": 8193, "h": 1}]"#,
            r#", "resources": [{"path": "b.bin", "type": "application/octet-stream", "hash": "sha256:1", "w": 1, "h": 1}]"#,
            r#", "resources": [{"path": "a.png", "type": "image/png", "hash": "sha256:0", "w": 1, "h": 1, "alt": "x"}]"#,
            r#", "min_size": [320]"#,
        ] {
            assert!(Manifest::from_json(&minimal(bad)).is_err(), "{bad}");
        }
        let big = "f".repeat(16 * 1024 + 1);
        assert!(Manifest::from_json(&minimal(&format!(r#", "fallback": "{big}""#))).is_err());
    }

    /// SPEC-8.1.2, SPEC-6.5.1: the sizes the manifest declares for the
    /// gallery's and the shop's images are the sizes in the PNG headers; a
    /// wrong declaration, a missing file or a non-PNG is refused.
    #[test]
    fn spec_8_1_2_image_sizes_match_the_files() {
        for (name, dir) in [("gallery", "modules/gallery"), ("shop", "modules/shop")] {
            let m = manifest(name);
            assert_eq!(m.resources.len(), 8);
            assert!(m
                .resources
                .iter()
                .all(|r| r.is_image() && r.pixels.is_some()));
            m.check_resource_sizes(|path| fs::read(root().join(dir).join(path)).ok())
                .unwrap();
        }
        let gallery = manifest("gallery");
        assert_eq!(gallery.resources[0].pixels, Some((320, 200)));
        assert_eq!(gallery.resources[2].pixels, Some((200, 300)));
        let mut wrong = gallery.clone();
        wrong.resources[2].pixels = Some((300, 200));
        let e = wrong
            .check_resource_sizes(|path| fs::read(root().join("modules/gallery").join(path)).ok())
            .unwrap_err();
        assert!(
            e.message
                .contains("200x300 px but the manifest says 300x200"),
            "{e}"
        );
        assert!(
            gallery.check_resource_sizes(|_| None).is_err(),
            "unreadable file"
        );
        assert!(gallery
            .check_resource_sizes(|_| Some(b"not a png".to_vec()))
            .is_err());
        assert_eq!(
            png_size(&fs::read(root().join("modules/gallery/res/02-rutor.png")).unwrap()),
            Some((240, 240))
        );
        assert_eq!(png_size(b"short"), None);
    }

    /// SPEC-8.1.3: `memory_max` is in pages, at most 4096, and at least the
    /// module's initial memory plus one page.
    #[test]
    fn spec_8_1_3_memory_max_bounds() {
        let m = Manifest::from_json(&minimal("")).unwrap();
        assert_eq!(m.memory_max, 32);
        m.check_memory(31).unwrap();
        assert!(m.check_memory(32).is_err(), "no room to grow");
        assert!(m.check_memory(4096).is_err());
        assert!(Manifest::from_json(&minimal("").replace("32", "4097")).is_err());
        assert!(Manifest::from_json(&minimal("").replace("32", "0")).is_err());
        Manifest::from_json(&minimal("").replace("32", "4096"))
            .unwrap()
            .check_memory(4095)
            .unwrap();
    }

    /// SPEC-8.2.2: a capability absent from the manifest can never be granted.
    #[test]
    #[ignore = "M3: policy"]
    fn spec_8_2_2_no_late_grants() {
        todo!()
    }

    /// SPEC-8.2.3: default grant policy per capability.
    #[test]
    #[ignore = "M3: policy"]
    fn spec_8_2_3_default_policy() {
        todo!()
    }

    /// SPEC-8.3.1: fallback is shown when the module is rejected or the window is under min_size.
    #[test]
    #[ignore = "M3"]
    fn spec_8_3_1_fallback_conditions() {
        todo!()
    }

    /// Every manifest under `conformance/manifests/` parses and validates,
    /// one per reference module (appendix B).
    #[test]
    fn conformance_manifests_validate() {
        let dir = root().join("conformance/manifests");
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| {
                e.file_name()
                    .to_string_lossy()
                    .trim_end_matches(".json")
                    .to_string()
            })
            .collect();
        names.sort();
        assert_eq!(names.len(), 11, "{names:?}");
        for n in &names {
            let m = manifest(n);
            assert!(
                root().join("modules").join(n).join("Cargo.toml").is_file(),
                "{n}"
            );
            assert!(m.runs_on(vy_abi::ABI_MINOR));
        }
    }

    /// The JSON reader: values, escapes, numbers, and the refusals a
    /// hand-edited manifest is likely to need.
    #[test]
    fn json_reader() {
        let v =
            Value::parse(r#"{"a": [1, 2.5, -3e2, true, null], "s": "h\u00e9j\n\"\ud83d\ude00"}"#)
                .unwrap();
        assert_eq!(v.get("a").unwrap().as_array().unwrap().len(), 5);
        assert_eq!(v.get("a").unwrap().as_array().unwrap()[0].as_u64(), Some(1));
        assert_eq!(v.get("a").unwrap().as_array().unwrap()[1].as_u64(), None);
        assert_eq!(v.get("s").unwrap().as_str(), Some("héj\n\"\u{1F600}"));
        for bad in [
            "",
            "{",
            r#"{"a": 1,}"#,
            r#"[1 2]"#,
            r#"{"a": 1, "a": 2}"#,
            r#"{"a": 1} x"#,
            "\"\\x\"",
            "01",
            "\"\n\"",
        ] {
            assert!(Value::parse(bad).is_err(), "{bad:?}");
        }
        let deep = "[".repeat(100) + &"]".repeat(100);
        assert!(Value::parse(&deep).is_err(), "nesting cap");
    }
}
