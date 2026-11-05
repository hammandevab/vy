//! The manifest (SPEC §8): fields, canonical CBOR codec and JSON form.

use crate::cbor::{self, Enc, MapWriter, Reader};
use crate::Error;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;

/// Maximum manifest size in bytes (SPEC-4.1.3).
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Maximum `name` length in characters (SPEC-8.1.1).
pub const MAX_NAME_CHARS: usize = 80;
/// Maximum `summary` length in bytes (SPEC-8.1.1).
pub const MAX_SUMMARY_BYTES: usize = 2 * 1024;
/// Maximum `fallback` length in bytes (SPEC-8.1.2).
pub const MAX_FALLBACK_BYTES: usize = 16 * 1024;
/// Maximum `net.origins` entries (SPEC-8.2.1).
pub const MAX_NET_ORIGINS: usize = 16;
/// Maximum `storage.quota_kb` (SPEC-8.2.1).
pub const MAX_STORAGE_QUOTA_KB: u32 = 16_384;
/// Media type of the module inside a package.
pub const MIME_WASM: &str = "application/wasm";

/// ABI version `"MAJOR.MINOR"` (SPEC-4.3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AbiVersion {
    /// Incompatible generation; 1 in 1.x.
    pub major: u32,
    /// Backwards-compatible revision (SPEC-4.3.2).
    pub minor: u32,
}

impl AbiVersion {
    /// The version this crate implements.
    pub const CURRENT: AbiVersion = AbiVersion {
        major: vy_abi::ABI_MAJOR,
        minor: vy_abi::ABI_MINOR,
    };

    /// True when a client of `client` runs a module declaring `self`:
    /// same major and a minor no newer than the client's (SPEC-4.3.1,
    /// SPEC-4.3.2).
    pub fn runs_on(&self, client: AbiVersion) -> bool {
        self.major == client.major && self.minor <= client.minor
    }
}

impl fmt::Display for AbiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

fn parse_canonical_u32(s: &str) -> Option<u32> {
    if s.is_empty() || s.len() > 9 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if s.len() > 1 && s.starts_with('0') {
        return None;
    }
    s.parse().ok()
}

impl FromStr for AbiVersion {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let bad = || Error::Field {
            field: "abi",
            reason: format!("{s:?} is not MAJOR.MINOR"),
        };
        let (major, minor) = s.split_once('.').ok_or_else(bad)?;
        Ok(AbiVersion {
            major: parse_canonical_u32(major).ok_or_else(bad)?,
            minor: parse_canonical_u32(minor).ok_or_else(bad)?,
        })
    }
}

impl Serialize for AbiVersion {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for AbiVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse()
            .map_err(|e: Error| D::Error::custom(e.to_string()))
    }
}

/// Lower-case hex of bytes.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bytes from lower-case hex of the expected length, or `None`.
pub fn from_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != N * 2
        || !s
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    let mut out = [0u8; N];
    for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
        let hi = (chunk[0] as char).to_digit(16)?;
        let lo = (chunk[1] as char).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}

/// A SHA-256 digest. CBOR: 32-byte string; JSON: `"sha256:<hex>"`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    /// Digest of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Hash(Sha256::digest(bytes).into())
    }

    /// Lower-case hex without prefix.
    pub fn hex(&self) -> String {
        hex(&self.0)
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", self.hex())
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", self.hex())
    }
}

impl FromStr for Hash {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        s.strip_prefix("sha256:")
            .and_then(from_hex::<32>)
            .map(Hash)
            .ok_or_else(|| Error::Field {
                field: "hash",
                reason: format!("{s:?} is not sha256:<64 hex digits>"),
            })
    }
}

impl Serialize for Hash {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse()
            .map_err(|e: Error| D::Error::custom(e.to_string()))
    }
}

/// Module or chunk reference: `{hash, size, url?}` (SPEC-8.1.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blob {
    /// SHA-256 of the bytes (SPEC-9.4.1).
    pub hash: Hash,
    /// Byte length.
    pub size: u64,
    /// Where to fetch the bytes in the bare-manifest form (SPEC-7.2.1);
    /// absent means `/_vy/<hex hash>` relative to the manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Resource entry: `{hash, size, mime, url?, w?, h?}`; `w`/`h` are
/// mandatory for images (review 0002, decision 7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    /// SHA-256 of the bytes.
    pub hash: Hash,
    /// Byte length.
    pub size: u64,
    /// One of the media types of SPEC-4.1.5.
    pub mime: String,
    /// As for [`Blob::url`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Pixel width (images).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<u32>,
    /// Pixel height (images).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
}

impl Resource {
    /// True for `image/*` media types.
    pub fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }
}

/// Empty object: the value of a capability without parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Empty {}

/// `net` request (SPEC-8.2.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetRequest {
    /// Origins `fetch_begin` may reach (SPEC-5.2.8); at most 16.
    pub origins: Vec<String>,
}

/// `storage` request (SPEC-8.2.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageRequest {
    /// Quota in KB; at most 16 384.
    pub quota_kb: u32,
}

/// `capabilities`: a map from capability name to its parameters (review
/// 0002, decision 31). Absent capabilities cannot be granted (SPEC-8.2.2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityRequests {
    /// `net` {origins}.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<NetRequest>,
    /// `storage` {quota_kb}.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<StorageRequest>,
    /// `clipboard`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<Empty>,
    /// `hires_time`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hires_time: Option<Empty>,
    /// `ime`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ime: Option<Empty>,
    /// `persist_frame`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persist_frame: Option<Empty>,
}

impl CapabilityRequests {
    /// True when nothing is requested.
    pub fn is_empty(&self) -> bool {
        self.net.is_none()
            && self.storage.is_none()
            && self.clipboard.is_none()
            && self.hires_time.is_none()
            && self.ime.is_none()
            && self.persist_frame.is_none()
    }
}

/// Smallest window the module lays out for (SPEC-8.3.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinSize {
    /// Logical pixels.
    pub w: u32,
    /// Logical pixels.
    pub h: u32,
}

/// Ed25519 signature bytes. CBOR: 64-byte string; JSON: `"ed25519:<hex>"`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SigBytes(pub [u8; 64]);

impl fmt::Debug for SigBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ed25519:{}", hex(&self.0))
    }
}

impl fmt::Display for SigBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ed25519:{}", hex(&self.0))
    }
}

impl FromStr for SigBytes {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        s.strip_prefix("ed25519:")
            .and_then(from_hex::<64>)
            .map(SigBytes)
            .ok_or_else(|| Error::Field {
                field: "sig.bytes",
                reason: format!("{s:?} is not ed25519:<128 hex digits>"),
            })
    }
}

impl Serialize for SigBytes {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SigBytes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse()
            .map_err(|e: Error| D::Error::custom(e.to_string()))
    }
}

/// `sig`: Ed25519 over the canonical encoding without this field
/// (SPEC-7.4.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    /// Id of the publisher key: [`crate::key_id`] of its public bytes.
    pub key_id: String,
    /// The signature.
    pub bytes: SigBytes,
}

/// The manifest (SPEC-8.1.1, SPEC-8.1.2).
///
/// Field order here is the order of the JSON form; the CBOR form sorts keys
/// canonically regardless (SPEC-4.1.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// ABI version (SPEC-4.3.1).
    pub abi: AbiVersion,
    /// At most 80 characters.
    pub name: String,
    /// At most 2 KB.
    pub summary: String,
    /// BCP 47 language tag.
    pub lang: String,
    /// The module (SPEC-4.1.1); resource index 0 (SPEC-4.5.1).
    pub module: Blob,
    /// Memory cap in wasm pages (SPEC-8.1.3).
    pub memory_max: u32,
    /// Publication date `YYYY-MM-DD`.
    pub published: String,
    /// Resources, index 1 and up (SPEC-4.5.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<Resource>,
    /// Requested capabilities (SPEC-8.2.*).
    #[serde(default, skip_serializing_if = "CapabilityRequests::is_empty")]
    pub capabilities: CapabilityRequests,
    /// Outgoing `vy://` or `https://` links the page may navigate to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<String>,
    /// Markdown shown when the module is rejected or the window is too
    /// small (SPEC-8.3.1); at most 16 KB.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
    /// Smallest window (SPEC-8.3.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_size: Option<MinSize>,
    /// Resource index of an icon image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<u32>,
    /// Where the source lives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// Resource index of a skeleton display list (1.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton: Option<u32>,
    /// Streamed module chunks (1.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chunks: Vec<Blob>,
    /// Embedded pages (1.1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embeds: Vec<String>,
    /// Key rotation: id of the key that signs the next manifest (SPEC-7.4.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_key_id: Option<String>,
    /// The signature; `None` only before [`Manifest::sign`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sig: Option<Signature>,
}

/// What a resource index refers to (SPEC-4.5.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    /// Content hash.
    pub hash: &'a Hash,
    /// Byte length.
    pub size: u64,
    /// Media type; [`MIME_WASM`] for the module.
    pub mime: &'a str,
    /// Explicit URL, if any.
    pub url: Option<&'a str>,
}

impl Manifest {
    /// The entry behind a resource index: 0 is the module, `i` is
    /// `resources[i - 1]` (SPEC-4.5.1).
    pub fn entry(&self, index: u32) -> Option<Entry<'_>> {
        if index == 0 {
            return Some(Entry {
                hash: &self.module.hash,
                size: self.module.size,
                mime: MIME_WASM,
                url: self.module.url.as_deref(),
            });
        }
        let r = self.resources.get(index as usize - 1)?;
        Some(Entry {
            hash: &r.hash,
            size: r.size,
            mime: &r.mime,
            url: r.url.as_deref(),
        })
    }

    /// Whether the client shows `fallback` instead of the page
    /// (SPEC-8.3.1): when the module was rejected, or when the page's area
    /// is `width` × `height` logical pixels and that is smaller than
    /// `min_size` in either direction. Never for a page that has no
    /// `fallback`, since there is nothing to show; the client reports the
    /// rejection its own way then.
    pub fn needs_fallback(&self, module_rejected: bool, width: u32, height: u32) -> bool {
        if self.fallback.is_none() {
            return false;
        }
        if module_rejected {
            return true;
        }
        self.min_size
            .is_some_and(|min| width < min.w || height < min.h)
    }

    /// Canonical CBOR of everything but `sig`: the bytes that are signed
    /// (SPEC-7.4.1) and hashed for the package identity (SPEC-4.1.2).
    pub fn signed_bytes(&self) -> Vec<u8> {
        let mut enc = Enc::new(Vec::new());
        self.encode_into(&mut enc, false);
        enc.into_writer()
    }

    /// Package identity: SHA-256 of the canonical encoding without `sig`
    /// (SPEC-4.1.2).
    pub fn identity(&self) -> Hash {
        Hash::of(&self.signed_bytes())
    }

    /// Canonical CBOR including `sig` (SPEC-4.1.3); [`Error::Unsigned`]
    /// before signing, because `sig` is required (SPEC-8.1.1).
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        if self.sig.is_none() {
            return Err(Error::Unsigned);
        }
        let mut enc = Enc::new(Vec::new());
        self.encode_into(&mut enc, true);
        Ok(enc.into_writer())
    }

    fn encode_into(&self, enc: &mut Enc, with_sig: bool) {
        let mut m = MapWriter::new();
        m.field("abi", |e| cbor::text(e, &self.abi.to_string()));
        m.field("name", |e| cbor::text(e, &self.name));
        m.field("summary", |e| cbor::text(e, &self.summary));
        m.field("lang", |e| cbor::text(e, &self.lang));
        m.field("module", |e| encode_blob(e, &self.module));
        m.field("memory_max", |e| cbor::uint(e, u64::from(self.memory_max)));
        m.field("published", |e| cbor::text(e, &self.published));
        if !self.resources.is_empty() {
            m.field("resources", |e| {
                e.array(self.resources.len() as u64)
                    .expect("vec write cannot fail");
                for r in &self.resources {
                    encode_resource(e, r);
                }
            });
        }
        if !self.capabilities.is_empty() {
            m.field("capabilities", |e| {
                encode_capabilities(e, &self.capabilities)
            });
        }
        if !self.links.is_empty() {
            m.field("links", |e| cbor::text_array(e, &self.links));
        }
        if let Some(f) = &self.fallback {
            m.field("fallback", |e| cbor::text(e, f));
        }
        if let Some(s) = &self.min_size {
            m.field("min_size", |e| {
                let mut w = MapWriter::new();
                w.field("w", |e| cbor::uint(e, u64::from(s.w)));
                w.field("h", |e| cbor::uint(e, u64::from(s.h)));
                w.finish(e);
            });
        }
        if let Some(i) = self.icon {
            m.field("icon", |e| cbor::uint(e, u64::from(i)));
        }
        if let Some(u) = &self.source_url {
            m.field("source_url", |e| cbor::text(e, u));
        }
        if let Some(i) = self.skeleton {
            m.field("skeleton", |e| cbor::uint(e, u64::from(i)));
        }
        if !self.chunks.is_empty() {
            m.field("chunks", |e| {
                e.array(self.chunks.len() as u64)
                    .expect("vec write cannot fail");
                for c in &self.chunks {
                    encode_blob(e, c);
                }
            });
        }
        if !self.embeds.is_empty() {
            m.field("embeds", |e| cbor::text_array(e, &self.embeds));
        }
        if let Some(k) = &self.next_key_id {
            m.field("next_key_id", |e| cbor::text(e, k));
        }
        if with_sig {
            if let Some(sig) = &self.sig {
                m.field("sig", |e| {
                    let mut w = MapWriter::new();
                    w.field("key_id", |e| cbor::text(e, &sig.key_id));
                    w.field("bytes", |e| cbor::bytes(e, &sig.bytes.0));
                    w.finish(e);
                });
            }
        }
        m.finish(enc);
    }

    /// Decode canonical CBOR (SPEC-4.1.3): at most 64 KB, shortest-form
    /// integers, definite lengths, sorted keys, known fields only
    /// (SPEC-8.1.2), every required field present (SPEC-8.1.1).
    pub fn decode(bytes: &[u8]) -> Result<Manifest, Error> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            return Err(Error::TooLarge { size: bytes.len() });
        }
        let mut r = Reader::new(bytes);
        let m = decode_manifest(&mut r)?;
        r.finish()?;
        Ok(m)
    }

    /// Pretty JSON of the human-readable form (Appendix B).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("manifest serializes")
    }

    /// Parse the JSON form; unknown fields are an error (SPEC-8.1.2).
    pub fn from_json(json: &str) -> Result<Manifest, Error> {
        serde_json::from_str(json).map_err(|e| Error::Json(e.to_string()))
    }
}

fn encode_blob(e: &mut Enc, b: &Blob) {
    let mut w = MapWriter::new();
    w.field("hash", |e| cbor::bytes(e, &b.hash.0));
    w.field("size", |e| cbor::uint(e, b.size));
    if let Some(u) = &b.url {
        w.field("url", |e| cbor::text(e, u));
    }
    w.finish(e);
}

fn encode_resource(e: &mut Enc, r: &Resource) {
    let mut w = MapWriter::new();
    w.field("hash", |e| cbor::bytes(e, &r.hash.0));
    w.field("size", |e| cbor::uint(e, r.size));
    w.field("mime", |e| cbor::text(e, &r.mime));
    if let Some(u) = &r.url {
        w.field("url", |e| cbor::text(e, u));
    }
    if let Some(v) = r.w {
        w.field("w", |e| cbor::uint(e, u64::from(v)));
    }
    if let Some(v) = r.h {
        w.field("h", |e| cbor::uint(e, u64::from(v)));
    }
    w.finish(e);
}

fn encode_capabilities(e: &mut Enc, c: &CapabilityRequests) {
    let mut w = MapWriter::new();
    if let Some(n) = &c.net {
        w.field("net", |e| {
            let mut w = MapWriter::new();
            w.field("origins", |e| cbor::text_array(e, &n.origins));
            w.finish(e);
        });
    }
    if let Some(s) = &c.storage {
        w.field("storage", |e| {
            let mut w = MapWriter::new();
            w.field("quota_kb", |e| cbor::uint(e, u64::from(s.quota_kb)));
            w.finish(e);
        });
    }
    for (name, present) in [
        ("clipboard", c.clipboard.is_some()),
        ("hires_time", c.hires_time.is_some()),
        ("ime", c.ime.is_some()),
        ("persist_frame", c.persist_frame.is_some()),
    ] {
        if present {
            w.field(name, |e| MapWriter::new().finish(e));
        }
    }
    w.finish(e);
}

fn decode_blob<'b>(r: &mut Reader<'b>, field: &'static str) -> Result<Blob, Error> {
    let mut hash = None;
    let mut size = None;
    let mut url = None;
    r.map_fields(|r, key| {
        match key {
            "hash" => hash = Some(Hash(r.bytes_exact::<32>("hash")?)),
            "size" => size = Some(r.uint()?),
            "url" => url = Some(r.text()?.to_string()),
            other => return Err(Error::UnknownField(format!("{field}.{other}"))),
        }
        Ok(())
    })?;
    Ok(Blob {
        hash: hash.ok_or(Error::MissingField("hash"))?,
        size: size.ok_or(Error::MissingField("size"))?,
        url,
    })
}

fn decode_resource<'b>(r: &mut Reader<'b>) -> Result<Resource, Error> {
    let mut hash = None;
    let mut size = None;
    let mut mime = None;
    let mut url = None;
    let mut w = None;
    let mut h = None;
    r.map_fields(|r, key| {
        match key {
            "hash" => hash = Some(Hash(r.bytes_exact::<32>("hash")?)),
            "size" => size = Some(r.uint()?),
            "mime" => mime = Some(r.text()?.to_string()),
            "url" => url = Some(r.text()?.to_string()),
            "w" => w = Some(r.u32("w")?),
            "h" => h = Some(r.u32("h")?),
            other => return Err(Error::UnknownField(format!("resources[].{other}"))),
        }
        Ok(())
    })?;
    Ok(Resource {
        hash: hash.ok_or(Error::MissingField("resources[].hash"))?,
        size: size.ok_or(Error::MissingField("resources[].size"))?,
        mime: mime.ok_or(Error::MissingField("resources[].mime"))?,
        url,
        w,
        h,
    })
}

fn decode_empty<'b>(r: &mut Reader<'b>, name: &str) -> Result<Empty, Error> {
    r.map_fields(|_, key| Err(Error::UnknownField(format!("capabilities.{name}.{key}"))))?;
    Ok(Empty {})
}

fn decode_capabilities<'b>(r: &mut Reader<'b>) -> Result<CapabilityRequests, Error> {
    let mut c = CapabilityRequests::default();
    r.map_fields(|r, key| {
        match key {
            "net" => {
                let mut origins = None;
                r.map_fields(|r, k| match k {
                    "origins" => {
                        origins = Some(r.text_array()?);
                        Ok(())
                    }
                    other => Err(Error::UnknownField(format!("capabilities.net.{other}"))),
                })?;
                c.net = Some(NetRequest {
                    origins: origins.ok_or(Error::MissingField("capabilities.net.origins"))?,
                });
            }
            "storage" => {
                let mut quota = None;
                r.map_fields(|r, k| match k {
                    "quota_kb" => {
                        quota = Some(r.u32("quota_kb")?);
                        Ok(())
                    }
                    other => Err(Error::UnknownField(format!("capabilities.storage.{other}"))),
                })?;
                c.storage = Some(StorageRequest {
                    quota_kb: quota.ok_or(Error::MissingField("capabilities.storage.quota_kb"))?,
                });
            }
            "clipboard" => c.clipboard = Some(decode_empty(r, key)?),
            "hires_time" => c.hires_time = Some(decode_empty(r, key)?),
            "ime" => c.ime = Some(decode_empty(r, key)?),
            "persist_frame" => c.persist_frame = Some(decode_empty(r, key)?),
            other => return Err(Error::UnknownField(format!("capabilities.{other}"))),
        }
        Ok(())
    })?;
    if c.is_empty() {
        return Err(Error::NonCanonical {
            at: r.position(),
            reason: "empty capabilities map must be omitted",
        });
    }
    Ok(c)
}

fn non_empty_array(r: &Reader<'_>, n: u64, field: &'static str) -> Result<(), Error> {
    if n == 0 {
        Err(Error::NonCanonical {
            at: r.position(),
            reason: match field {
                "resources" => "empty resources must be omitted",
                "links" => "empty links must be omitted",
                "chunks" => "empty chunks must be omitted",
                _ => "empty embeds must be omitted",
            },
        })
    } else {
        Ok(())
    }
}

fn decode_manifest<'b>(r: &mut Reader<'b>) -> Result<Manifest, Error> {
    let mut abi = None;
    let mut name = None;
    let mut summary = None;
    let mut lang = None;
    let mut module = None;
    let mut memory_max = None;
    let mut published = None;
    let mut resources = Vec::new();
    let mut capabilities = CapabilityRequests::default();
    let mut links = Vec::new();
    let mut fallback = None;
    let mut min_size = None;
    let mut icon = None;
    let mut source_url = None;
    let mut skeleton = None;
    let mut chunks = Vec::new();
    let mut embeds = Vec::new();
    let mut next_key_id = None;
    let mut sig = None;

    r.map_fields(|r, key| {
        match key {
            "abi" => abi = Some(r.text()?.parse::<AbiVersion>()?),
            "name" => name = Some(r.text()?.to_string()),
            "summary" => summary = Some(r.text()?.to_string()),
            "lang" => lang = Some(r.text()?.to_string()),
            "module" => module = Some(decode_blob(r, "module")?),
            "memory_max" => memory_max = Some(r.u32("memory_max")?),
            "published" => published = Some(r.text()?.to_string()),
            "resources" => {
                let n = r.array()?;
                non_empty_array(r, n, "resources")?;
                for _ in 0..n {
                    resources.push(decode_resource(r)?);
                }
            }
            "capabilities" => capabilities = decode_capabilities(r)?,
            "links" => {
                links = r.text_array()?;
                non_empty_array(r, links.len() as u64, "links")?;
            }
            "fallback" => fallback = Some(r.text()?.to_string()),
            "min_size" => {
                let mut w = None;
                let mut h = None;
                r.map_fields(|r, k| match k {
                    "w" => {
                        w = Some(r.u32("min_size.w")?);
                        Ok(())
                    }
                    "h" => {
                        h = Some(r.u32("min_size.h")?);
                        Ok(())
                    }
                    other => Err(Error::UnknownField(format!("min_size.{other}"))),
                })?;
                min_size = Some(MinSize {
                    w: w.ok_or(Error::MissingField("min_size.w"))?,
                    h: h.ok_or(Error::MissingField("min_size.h"))?,
                });
            }
            "icon" => icon = Some(r.u32("icon")?),
            "source_url" => source_url = Some(r.text()?.to_string()),
            "skeleton" => skeleton = Some(r.u32("skeleton")?),
            "chunks" => {
                let n = r.array()?;
                non_empty_array(r, n, "chunks")?;
                for _ in 0..n {
                    chunks.push(decode_blob(r, "chunks[]")?);
                }
            }
            "embeds" => {
                embeds = r.text_array()?;
                non_empty_array(r, embeds.len() as u64, "embeds")?;
            }
            "next_key_id" => next_key_id = Some(r.text()?.to_string()),
            "sig" => {
                let mut key_id = None;
                let mut bytes = None;
                r.map_fields(|r, k| match k {
                    "key_id" => {
                        key_id = Some(r.text()?.to_string());
                        Ok(())
                    }
                    "bytes" => {
                        bytes = Some(SigBytes(r.bytes_exact::<64>("sig.bytes")?));
                        Ok(())
                    }
                    other => Err(Error::UnknownField(format!("sig.{other}"))),
                })?;
                sig = Some(Signature {
                    key_id: key_id.ok_or(Error::MissingField("sig.key_id"))?,
                    bytes: bytes.ok_or(Error::MissingField("sig.bytes"))?,
                });
            }
            other => return Err(Error::UnknownField(other.to_string())),
        }
        Ok(())
    })?;

    Ok(Manifest {
        abi: abi.ok_or(Error::MissingField("abi"))?,
        name: name.ok_or(Error::MissingField("name"))?,
        summary: summary.ok_or(Error::MissingField("summary"))?,
        lang: lang.ok_or(Error::MissingField("lang"))?,
        module: module.ok_or(Error::MissingField("module"))?,
        memory_max: memory_max.ok_or(Error::MissingField("memory_max"))?,
        published: published.ok_or(Error::MissingField("published"))?,
        resources,
        capabilities,
        links,
        fallback,
        min_size,
        icon,
        source_url,
        skeleton,
        chunks,
        embeds,
        next_key_id,
        sig: Some(sig.ok_or(Error::MissingField("sig"))?),
    })
}
