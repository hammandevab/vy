//! Package framing (SPEC-4.1.1, SPEC-4.4.1): a zstd-compressed tar stream
//! with the entries `manifest.cbor`, `module.wasm`, `res/1`, `res/2`, …
//! in that order and nothing else.
//!
//! The tar subset is POSIX ustar with regular files only, fixed metadata
//! (mode 0644, uid/gid 0, mtime 0) so the same inputs give the same bytes
//! (SPEC-9.7.1); `tar --zstd -xf` reads it.

use crate::manifest::{Manifest, MAX_MANIFEST_BYTES};
use crate::validate::{Problem, MAX_MODULE_BYTES, MAX_RESOURCES_BYTES};
use crate::{Error, Hash};
use std::fmt;
use std::io::Read;

/// Media type of a whole package (SPEC-4.4.1, SPEC-7.2.1).
pub const MEDIA_PACKAGE: &str = "application/vy-package+zstd";
/// Media type of a bare manifest (SPEC-7.2.1).
pub const MEDIA_MANIFEST: &str = "application/vy-manifest+cbor";
/// Tar entry holding the manifest.
pub const ENTRY_MANIFEST: &str = "manifest.cbor";
/// Tar entry holding the module.
pub const ENTRY_MODULE: &str = "module.wasm";
/// Prefix of resource entries: `res/<index>` with index from 1 (SPEC-4.5.1).
pub const ENTRY_RESOURCE_PREFIX: &str = "res/";
/// zstd level used when packing: slow, small, deterministic for a given
/// library version.
pub const ZSTD_LEVEL: i32 = 19;
/// Largest decompressed tar stream accepted: manifest + module + resources
/// (SPEC-4.1.3, SPEC-4.1.4, SPEC-4.1.5) plus headers and padding.
pub const MAX_PACKAGE_BYTES: u64 =
    MAX_MANIFEST_BYTES as u64 + MAX_MODULE_BYTES as u64 + MAX_RESOURCES_BYTES + 1024 * 1024;

const BLOCK: usize = 512;

/// Why bytes are not a package.
#[derive(Debug)]
pub enum PackageError {
    /// Not a zstd stream.
    Zstd(String),
    /// Decompresses to more than [`MAX_PACKAGE_BYTES`].
    TooLarge,
    /// Not the tar subset of SPEC-4.4.1.
    Tar(String),
    /// A required entry is missing or out of order.
    MissingEntry(&'static str),
    /// An entry the format does not have.
    UnexpectedEntry(String),
    /// The manifest does not decode.
    Manifest(Error),
    /// The manifest does not describe the module and resources.
    Invalid(Vec<Problem>),
}

impl fmt::Display for PackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zstd(e) => write!(f, "not a zstd stream: {e} (SPEC-4.4.1)"),
            Self::TooLarge => write!(
                f,
                "package decompresses to more than {MAX_PACKAGE_BYTES} bytes"
            ),
            Self::Tar(e) => write!(f, "not a tar stream: {e} (SPEC-4.4.1)"),
            Self::MissingEntry(e) => write!(f, "package lacks entry {e} (SPEC-4.4.1)"),
            Self::UnexpectedEntry(e) => {
                write!(f, "package has unexpected entry {e:?} (SPEC-4.4.1)")
            }
            Self::Manifest(e) => write!(f, "manifest.cbor: {e}"),
            Self::Invalid(problems) => {
                write!(f, "manifest does not describe the package:")?;
                for p in problems {
                    write!(f, "\n  {p}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for PackageError {}

/// A manifest, its module and its resources (SPEC-4.1.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// The manifest, signed.
    pub manifest: Manifest,
    /// Resource index 0 (SPEC-4.5.1).
    pub module: Vec<u8>,
    /// Resource indices 1 and up.
    pub resources: Vec<Vec<u8>>,
}

impl Package {
    /// Package identity (SPEC-4.1.2).
    pub fn identity(&self) -> Hash {
        self.manifest.identity()
    }

    /// Every problem of the manifest against the bytes ([`Manifest::validate`]).
    pub fn validate(&self) -> Vec<Problem> {
        self.manifest.validate(&self.module, &self.resources)
    }

    /// Frame as zstd-compressed tar (SPEC-4.4.1). Fails only for an
    /// unsigned manifest (SPEC-8.1.1); consistency is the caller's job
    /// ([`Package::validate`]).
    pub fn pack(&self) -> Result<Vec<u8>, Error> {
        let manifest = self.manifest.encode()?;
        let mut entries: Vec<(String, &[u8])> = vec![
            (ENTRY_MANIFEST.to_string(), &manifest),
            (ENTRY_MODULE.to_string(), &self.module),
        ];
        for (i, r) in self.resources.iter().enumerate() {
            entries.push((format!("{ENTRY_RESOURCE_PREFIX}{}", i + 1), r));
        }
        let tar = tar_write(&entries);
        zstd::stream::encode_all(&tar[..], ZSTD_LEVEL).map_err(|e| Error::Io(e.to_string()))
    }

    /// Unframe, decode the manifest and validate it against the bytes
    /// (SPEC-9.4.1). The signature is not checked here.
    pub fn unpack(bytes: &[u8]) -> Result<Package, PackageError> {
        let decoder = zstd::stream::read::Decoder::with_buffer(bytes)
            .map_err(|e| PackageError::Zstd(e.to_string()))?;
        let mut tar = Vec::new();
        decoder
            .take(MAX_PACKAGE_BYTES + 1)
            .read_to_end(&mut tar)
            .map_err(|e| PackageError::Zstd(e.to_string()))?;
        if tar.len() as u64 > MAX_PACKAGE_BYTES {
            return Err(PackageError::TooLarge);
        }
        let entries = tar_read(&tar).map_err(PackageError::Tar)?;
        let mut it = entries.into_iter();
        let manifest_bytes = match it.next() {
            Some((name, data)) if name == ENTRY_MANIFEST => data,
            _ => return Err(PackageError::MissingEntry(ENTRY_MANIFEST)),
        };
        let module = match it.next() {
            Some((name, data)) if name == ENTRY_MODULE => data.to_vec(),
            _ => return Err(PackageError::MissingEntry(ENTRY_MODULE)),
        };
        let mut resources = Vec::new();
        for (name, data) in it {
            let expected = format!("{ENTRY_RESOURCE_PREFIX}{}", resources.len() + 1);
            if name != expected {
                return Err(PackageError::UnexpectedEntry(name));
            }
            resources.push(data.to_vec());
        }
        let manifest = Manifest::decode(manifest_bytes).map_err(PackageError::Manifest)?;
        let package = Package {
            manifest,
            module,
            resources,
        };
        let problems = package.validate();
        if !problems.is_empty() {
            return Err(PackageError::Invalid(problems));
        }
        Ok(package)
    }
}

fn octal(field: &mut [u8], value: u64) {
    // Width minus one digits, then NUL.
    let digits = field.len() - 1;
    let s = format!("{value:0width$o}", width = digits);
    field[..digits].copy_from_slice(&s.as_bytes()[s.len() - digits..]);
    field[digits] = 0;
}

/// Write a ustar stream with regular files and fixed metadata.
pub fn tar_write(entries: &[(String, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    for (name, data) in entries {
        let mut h = [0u8; BLOCK];
        let name = name.as_bytes();
        assert!(name.len() <= 100, "tar entry name too long");
        h[..name.len()].copy_from_slice(name);
        octal(&mut h[100..108], 0o644);
        octal(&mut h[108..116], 0);
        octal(&mut h[116..124], 0);
        octal(&mut h[124..136], data.len() as u64);
        octal(&mut h[136..148], 0);
        h[148..156].copy_from_slice(b"        ");
        h[156] = b'0';
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        octal(&mut h[329..337], 0);
        octal(&mut h[337..345], 0);
        let sum: u64 = h.iter().map(|&b| u64::from(b)).sum();
        let chk = format!("{sum:06o}\0 ");
        h[148..156].copy_from_slice(chk.as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(data);
        let pad = (BLOCK - data.len() % BLOCK) % BLOCK;
        out.extend(std::iter::repeat_n(0u8, pad));
    }
    out.extend(std::iter::repeat_n(0u8, 2 * BLOCK));
    out
}

fn parse_octal(field: &[u8]) -> Result<u64, String> {
    let end = field
        .iter()
        .position(|&b| b == 0 || b == b' ')
        .unwrap_or(field.len());
    let s = std::str::from_utf8(&field[..end]).map_err(|_| "non-ASCII numeric field")?;
    if s.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(s, 8).map_err(|_| format!("bad octal field {s:?}"))
}

/// Read a ustar stream of regular files; returns (name, data) in order.
pub fn tar_read(bytes: &[u8]) -> Result<Vec<(String, &[u8])>, String> {
    let mut entries = Vec::new();
    let mut pos = 0;
    loop {
        let Some(h) = bytes.get(pos..pos + BLOCK) else {
            return Err("truncated header".into());
        };
        if h.iter().all(|&b| b == 0) {
            // End of archive: a zero block (the second one is optional).
            return Ok(entries);
        }
        if &h[257..262] != b"ustar" {
            return Err("not a ustar header".into());
        }
        let declared = parse_octal(&h[148..156])?;
        let actual: u64 = h
            .iter()
            .enumerate()
            .map(|(i, &b)| {
                if (148..156).contains(&i) {
                    32
                } else {
                    u64::from(b)
                }
            })
            .sum();
        if declared != actual {
            return Err("header checksum mismatch".into());
        }
        let typeflag = h[156];
        if typeflag != b'0' && typeflag != 0 {
            return Err(format!(
                "entry type {:?} is not a regular file",
                typeflag as char
            ));
        }
        if h[345] != 0 {
            return Err("prefix field is not used by this format".into());
        }
        let name_len = h[..100].iter().position(|&b| b == 0).unwrap_or(100);
        let name = std::str::from_utf8(&h[..name_len])
            .map_err(|_| "entry name is not UTF-8")?
            .to_string();
        let size = parse_octal(&h[124..136])?;
        let size = usize::try_from(size).map_err(|_| "entry too large")?;
        let start = pos + BLOCK;
        let data = bytes
            .get(start..start + size)
            .ok_or_else(|| "truncated entry".to_string())?;
        entries.push((name, data));
        pos = start + size.div_ceil(BLOCK) * BLOCK;
    }
}
