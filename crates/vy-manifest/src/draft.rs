//! The publisher's input: a manifest before hashes, sizes and the
//! signature are known. `vy-publish build` turns a [`Draft`] into a
//! signed [`Manifest`]; `conformance/manifests/*.json` are drafts.

use crate::manifest::{Blob, CapabilityRequests, Manifest, MinSize, Resource};
use crate::validate::png_dimensions;
use crate::{AbiVersion, Error, Hash};
use serde::Deserialize;
use std::path::Path;

/// A resource as the publisher names it: a file path (relative to the
/// draft) and, when the extension does not say, its media type. An image
/// may declare `w`/`h`; the declaration is the publisher's claim and
/// [`Manifest::validate`] checks it against the PNG header (SPEC-8.1.2).
/// Images other than PNG must declare them, because the client does not
/// read their headers in M3.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DraftResource {
    /// File path relative to the draft file.
    pub path: String,
    /// Media type; inferred from the extension when absent.
    #[serde(default)]
    pub mime: Option<String>,
    /// Declared pixel width.
    #[serde(default)]
    pub w: Option<u32>,
    /// Declared pixel height.
    #[serde(default)]
    pub h: Option<u32>,
}

/// The fields a publisher writes by hand (SPEC-8.1.1, SPEC-8.1.2 minus
/// `module`, `sig` and `chunks`, which tooling fills).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    /// ABI version.
    pub abi: AbiVersion,
    /// Page name.
    pub name: String,
    /// Summary.
    pub summary: String,
    /// BCP 47 tag.
    pub lang: String,
    /// Memory cap in pages.
    pub memory_max: u32,
    /// Publication date.
    pub published: String,
    /// Resources by path.
    #[serde(default)]
    pub resources: Vec<DraftResource>,
    /// Requested capabilities.
    #[serde(default)]
    pub capabilities: CapabilityRequests,
    /// Outgoing links.
    #[serde(default)]
    pub links: Vec<String>,
    /// Fallback Markdown.
    #[serde(default)]
    pub fallback: Option<String>,
    /// Smallest window.
    #[serde(default)]
    pub min_size: Option<MinSize>,
    /// Icon resource index.
    #[serde(default)]
    pub icon: Option<u32>,
    /// Source location.
    #[serde(default)]
    pub source_url: Option<String>,
    /// Skeleton resource index.
    #[serde(default)]
    pub skeleton: Option<u32>,
    /// Embedded pages.
    #[serde(default)]
    pub embeds: Vec<String>,
    /// Announced successor key (SPEC-7.4.4).
    #[serde(default)]
    pub next_key_id: Option<String>,
}

/// Media type for a resource file name by extension (SPEC-4.1.5); raw
/// data otherwise.
pub fn mime_for_path(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Bytes of one resource with its media type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceInput {
    /// Media type.
    pub mime: String,
    /// The bytes.
    pub bytes: Vec<u8>,
    /// The size the draft declares for an image, `(w, h)`; `None` lets
    /// the PNG header fill it in.
    pub size_hint: Option<(u32, u32)>,
}

impl Draft {
    /// Parse the JSON form; unknown fields are an error.
    pub fn from_json(json: &str) -> Result<Draft, Error> {
        serde_json::from_str(json).map_err(|e| Error::Json(e.to_string()))
    }

    /// Read the bytes of every resource the draft names, relative to
    /// `base` (the draft file's directory), in the draft's order: media
    /// type from the entry or the extension, declared `w`/`h` as the size
    /// hint. `vy-publish build` and the conformance suite read the same
    /// files this way.
    pub fn load_resources(&self, base: &Path) -> Result<Vec<ResourceInput>, Error> {
        self.resources
            .iter()
            .map(|r| {
                let path = base.join(&r.path);
                let bytes = std::fs::read(&path)
                    .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
                Ok(ResourceInput {
                    mime: r
                        .mime
                        .clone()
                        .unwrap_or_else(|| mime_for_path(&r.path).to_string()),
                    bytes,
                    size_hint: r.w.zip(r.h),
                })
            })
            .collect()
    }

    /// Fill in hashes, sizes and image dimensions: a declared size is
    /// kept as declared (and checked by [`Manifest::validate`], SPEC-8.1.2);
    /// a PNG without one gets the size of its header. The result is
    /// unsigned; call [`Manifest::sign`] next and [`Manifest::validate`] to
    /// check.
    pub fn finish(&self, module: &[u8], resources: &[ResourceInput]) -> Manifest {
        let resources = resources
            .iter()
            .map(|r| {
                let (w, h) = if let Some((w, h)) = r.size_hint {
                    (Some(w), Some(h))
                } else if r.mime == "image/png" {
                    png_dimensions(&r.bytes)
                        .map(|(w, h)| (Some(w), Some(h)))
                        .unwrap_or((None, None))
                } else {
                    (None, None)
                };
                Resource {
                    hash: Hash::of(&r.bytes),
                    size: r.bytes.len() as u64,
                    mime: r.mime.clone(),
                    url: None,
                    w,
                    h,
                }
            })
            .collect();
        Manifest {
            abi: self.abi,
            name: self.name.clone(),
            summary: self.summary.clone(),
            lang: self.lang.clone(),
            module: Blob {
                hash: Hash::of(module),
                size: module.len() as u64,
                url: None,
            },
            memory_max: self.memory_max,
            published: self.published.clone(),
            resources,
            capabilities: self.capabilities.clone(),
            links: self.links.clone(),
            fallback: self.fallback.clone(),
            min_size: self.min_size,
            icon: self.icon,
            source_url: self.source_url.clone(),
            skeleton: self.skeleton,
            chunks: Vec::new(),
            embeds: self.embeds.clone(),
            next_key_id: self.next_key_id.clone(),
            sig: None,
        }
    }
}
