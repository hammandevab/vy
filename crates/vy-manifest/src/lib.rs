//! Content format (SPEC §4, §8): the manifest as canonical CBOR and as
//! JSON, validation against module and resources, the capability policy,
//! Ed25519 signatures (SPEC-7.4.1) and the package framing (SPEC-4.4.1).
//!
//! Nothing here touches the network; that is `vy-proto`.
#![deny(missing_docs)]

mod cbor;
mod draft;
mod manifest;
mod package;
mod policy;
mod sign;
mod validate;

pub use draft::{mime_for_path, Draft, DraftResource, ResourceInput};
pub use manifest::{
    from_hex, hex, AbiVersion, Blob, CapabilityRequests, Empty, Entry, Hash, Manifest, MinSize,
    NetRequest, Resource, SigBytes, Signature, StorageRequest, MAX_FALLBACK_BYTES,
    MAX_MANIFEST_BYTES, MAX_NAME_CHARS, MAX_NET_ORIGINS, MAX_STORAGE_QUOTA_KB, MAX_SUMMARY_BYTES,
    MIME_WASM,
};
pub use package::{
    tar_read, tar_write, Package, PackageError, ENTRY_MANIFEST, ENTRY_MODULE,
    ENTRY_RESOURCE_PREFIX, MAX_PACKAGE_BYTES, MEDIA_MANIFEST, MEDIA_PACKAGE, ZSTD_LEVEL,
};
pub use policy::{default_grant, Grant, Requested};
pub use sign::{key_file, key_id, parse_key_file, PublicKey, SigningKey, KEY_ID_HEX_LEN};
pub use validate::{
    is_bcp47, is_date, is_origin, module_initial_pages, png_dimensions, Problem,
    AUTO_STORAGE_QUOTA_KB, MAX_MODULE_BYTES, MAX_RESOURCES_BYTES, RESOURCE_MIMES,
};

use std::fmt;

/// Capabilities defined in 1.0 (SPEC-8.2.1), in id order (`vy_abi::cap`).
pub const CAPABILITIES: &[&str] = &vy_abi::cap::NAMES;

/// Why bytes or text are not a manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// More than 64 KB (SPEC-4.1.3).
    TooLarge {
        /// Size given.
        size: usize,
    },
    /// Valid CBOR that is not in canonical form (SPEC-4.1.3).
    NonCanonical {
        /// Byte offset of the offending item.
        at: usize,
        /// What is wrong.
        reason: &'static str,
    },
    /// Not CBOR, or a CBOR type the schema does not use.
    Malformed {
        /// Byte offset.
        at: usize,
        /// Decoder message.
        message: String,
    },
    /// Input ended inside an item.
    Truncated,
    /// Bytes after the manifest.
    Trailing {
        /// Offset of the first extra byte.
        at: usize,
    },
    /// A key the schema does not define (SPEC-8.1.2).
    UnknownField(String),
    /// The same key twice.
    DuplicateField(String),
    /// A required field is absent (SPEC-8.1.1).
    MissingField(&'static str),
    /// A field with the right type but an impossible value.
    Field {
        /// Field name.
        field: &'static str,
        /// What is wrong.
        reason: String,
    },
    /// `encode` on a manifest without `sig` (SPEC-8.1.1).
    Unsigned,
    /// The JSON form did not parse.
    Json(String),
    /// Compression or file error.
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { size } => {
                write!(
                    f,
                    "manifest is {size} bytes, at most {MAX_MANIFEST_BYTES} (SPEC-4.1.3)"
                )
            }
            Self::NonCanonical { at, reason } => {
                write!(f, "not canonical CBOR at byte {at}: {reason} (SPEC-4.1.3)")
            }
            Self::Malformed { at, message } => write!(f, "malformed CBOR at byte {at}: {message}"),
            Self::Truncated => f.write_str("truncated CBOR"),
            Self::Trailing { at } => write!(f, "trailing bytes after the manifest at byte {at}"),
            Self::UnknownField(k) => write!(f, "unknown field {k:?} (SPEC-8.1.2)"),
            Self::DuplicateField(k) => write!(f, "duplicate field {k:?}"),
            Self::MissingField(k) => write!(f, "missing required field {k:?} (SPEC-8.1.1)"),
            Self::Field { field, reason } => write!(f, "field {field:?}: {reason}"),
            Self::Unsigned => f.write_str("manifest is not signed; sig is required (SPEC-8.1.1)"),
            Self::Json(e) => write!(f, "manifest JSON: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbor::{encode_key, Enc};

    /// RFC 8032 §7.1 test vector 1: a publicly known key, never a secret.
    pub(crate) const TEST_SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    const TEST_PUBLIC_HEX: &str =
        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn test_key() -> SigningKey {
        SigningKey::from_seed(&TEST_SEED)
    }

    /// A module with `pages` initial pages and the six exports.
    fn module(pages: u32) -> Vec<u8> {
        wat::parse_str(format!(
            r#"(module (memory (export "memory") {pages})
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32) i32.const 0)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
        ))
        .unwrap()
    }

    /// A PNG header for a `w`×`h` image (signature + IHDR only).
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0]);
        b.extend_from_slice(&[0, 0, 0, 0]);
        b
    }

    fn draft() -> Draft {
        Draft::from_json(include_str!("../tests/fixtures/hello.json")).unwrap()
    }

    /// The fixture, finished with a 2-page module and no resources, signed.
    fn hello() -> (Manifest, Vec<u8>) {
        let wasm = module(2);
        let mut m = draft().finish(&wasm, &[]);
        m.sign(&test_key());
        (m, wasm)
    }

    /// A manifest with a PNG resource, signed.
    fn with_image() -> (Manifest, Vec<u8>, Vec<ResourceInput>) {
        let wasm = module(2);
        let res = vec![ResourceInput {
            mime: "image/png".into(),
            bytes: png(320, 200),
            size_hint: None,
        }];
        let mut m = draft().finish(&wasm, &res);
        m.sign(&test_key());
        (m, wasm, res)
    }

    /// Top-level entries of a CBOR map as (key, encoded value) pairs.
    fn entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let mut d = minicbor::Decoder::new(bytes);
        let n = d.map().unwrap().unwrap();
        let mut out = Vec::new();
        for _ in 0..n {
            let key = d.str().unwrap().to_string();
            let start = d.position();
            d.skip().unwrap();
            out.push((key, bytes[start..d.position()].to_vec()));
        }
        assert_eq!(d.position(), bytes.len());
        out
    }

    /// Rebuild a map from entries in the given order, without sorting.
    fn rebuild(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut e = Enc::new(Vec::new());
        e.map(entries.len() as u64).unwrap();
        let mut out = e.into_writer();
        for (k, v) in entries {
            out.extend(encode_key(k));
            out.extend_from_slice(v);
        }
        out
    }

    /// SPEC-8.2.1: the six capabilities of 1.0.
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
        // Net and storage carry parameters; the rest are empty objects
        // (decision 31), and the limits of the sentence are enforced.
        let (mut m, wasm) = hello();
        m.capabilities.net = Some(NetRequest {
            origins: (0..17).map(|i| format!("https://o{i}.example")).collect(),
        });
        m.capabilities.storage = Some(StorageRequest { quota_kb: 16_385 });
        let problems = m.validate(&wasm, &Vec::<Vec<u8>>::new());
        assert!(problems.contains(&Problem::NetOriginsTooMany { count: 17 }));
        assert!(problems.contains(&Problem::StorageQuotaTooLarge { quota_kb: 16_385 }));
        m.capabilities.net.as_mut().unwrap().origins.truncate(16);
        m.capabilities.storage.as_mut().unwrap().quota_kb = 16_384;
        assert!(m.validate(&wasm, &Vec::<Vec<u8>>::new()).is_empty());
    }

    /// SPEC-4.1.3, SPEC-4.1.4, SPEC-4.1.5: size limits.
    #[test]
    fn spec_4_1_3_size_limits() {
        assert_eq!(MAX_MANIFEST_BYTES, 65_536);
        assert_eq!(MAX_MODULE_BYTES, 8_388_608);
        assert_eq!(MAX_RESOURCES_BYTES, 33_554_432);
        let (mut m, wasm) = hello();
        m.fallback = Some("x".repeat(MAX_MANIFEST_BYTES));
        let problems = m.validate(&wasm, &Vec::<Vec<u8>>::new());
        assert!(problems
            .iter()
            .any(|p| matches!(p, Problem::ManifestTooLarge { .. })));
        assert!(problems
            .iter()
            .any(|p| matches!(p, Problem::FallbackTooLong { .. })));
        // Decoding refuses anything over the limit before parsing it.
        let big = vec![0u8; MAX_MANIFEST_BYTES + 1];
        assert_eq!(
            Manifest::decode(&big),
            Err(Error::TooLarge {
                size: MAX_MANIFEST_BYTES + 1
            })
        );
    }

    /// SPEC-4.1.1: a package is one manifest, exactly one module, zero or
    /// more resources; the manifest must account for every byte of them.
    #[test]
    fn spec_4_1_1_package_shape() {
        let (manifest, wasm) = hello();
        let p = Package {
            manifest,
            module: wasm,
            resources: vec![],
        };
        assert!(p.validate().is_empty());
        let unpacked = Package::unpack(&p.pack().unwrap()).unwrap();
        assert_eq!(unpacked, p);

        let (manifest, wasm, res) = with_image();
        let p = Package {
            manifest,
            module: wasm,
            resources: res.into_iter().map(|r| r.bytes).collect(),
        };
        assert!(p.validate().is_empty());
        assert_eq!(Package::unpack(&p.pack().unwrap()).unwrap(), p);

        // A resource the manifest does not declare, or one it declares but
        // the package lacks, is not a package.
        let mut extra = p.clone();
        extra.resources.push(vec![1, 2, 3]);
        assert!(extra.validate().contains(&Problem::ResourceCountMismatch {
            declared: 1,
            given: 2
        }));
        let mut missing = p.clone();
        missing.resources.clear();
        assert!(matches!(
            Package::unpack(&missing.pack().unwrap()),
            Err(PackageError::Invalid(problems))
                if problems.contains(&Problem::ResourceCountMismatch { declared: 1, given: 0 })
        ));
        // Exactly one module: other bytes under the manifest's hash are refused.
        let mut swapped = p.clone();
        swapped.module = module(3);
        assert!(matches!(
            Package::unpack(&swapped.pack().unwrap()),
            Err(PackageError::Invalid(problems))
                if problems.iter().any(|p| matches!(p, Problem::ModuleHashMismatch { .. }))
        ));
    }

    /// SPEC-4.1.2, SPEC-2.1.2: package identity is SHA-256 of the canonical
    /// CBOR without `sig`, so re-signing keeps the identity and any other
    /// change moves it. The canonical bytes of the fixture are pinned.
    #[test]
    fn spec_4_1_2_identity_is_hash_of_canonical_cbor() {
        let (signed, wasm) = hello();
        let unsigned = Manifest {
            sig: None,
            ..signed.clone()
        };
        assert_eq!(signed.identity(), unsigned.identity());
        assert_eq!(signed.identity(), Hash::of(&unsigned.signed_bytes()));
        assert_ne!(signed.signed_bytes(), signed.encode().unwrap());
        let mut other = signed.clone();
        other.name.push('!');
        assert_ne!(other.identity(), signed.identity());

        // Determinism: the fixture with this module encodes to these bytes,
        // keys sorted by their encoding (shorter first, then bytewise).
        let module_hash = Hash::of(&wasm).hex();
        let mut expected = String::new();
        expected.push_str("a9"); // map(9)
        expected.push_str("63616269"); // "abi"
        expected.push_str(&text_hex("1.0"));
        expected.push_str("646c616e67"); // "lang"
        expected.push_str(&text_hex("sv"));
        expected.push_str("646e616d65"); // "name"
        expected.push_str(&text_hex("Hej från Vy"));
        expected.push_str("666d6f64756c65"); // "module"
        expected.push_str("a2"); // {2}
        expected.push_str("6468617368"); // "hash"
        expected.push_str(&("5820".to_string() + &module_hash));
        expected.push_str("6473697a65"); // "size"
        expected.push_str(&uint_hex(wasm.len() as u64));
        expected.push_str("6773756d6d617279"); // "summary"
        expected.push_str(&text_hex(&signed.summary));
        expected.push_str("6866616c6c6261636b"); // "fallback"
        expected.push_str(&text_hex(signed.fallback.as_ref().unwrap()));
        expected.push_str("697075626c6973686564"); // "published"
        expected.push_str(&text_hex("2026-11-05"));
        expected.push_str("6a6d656d6f72795f6d6178"); // "memory_max"
        expected.push_str("1820"); // 32
        expected.push_str("6a736f757263655f75726c"); // "source_url"
        expected.push_str(&text_hex(signed.source_url.as_ref().unwrap()));
        assert_eq!(hex(&signed.signed_bytes()), expected);
        assert_eq!(
            signed.identity().hex(),
            Hash::of(&from_hex_vec(&expected)).hex()
        );
    }

    fn text_hex(s: &str) -> String {
        let mut e = Enc::new(Vec::new());
        e.str(s).unwrap();
        hex(&e.into_writer())
    }

    fn uint_hex(v: u64) -> String {
        let mut e = Enc::new(Vec::new());
        e.u64(v).unwrap();
        hex(&e.into_writer())
    }

    fn from_hex_vec(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    /// SPEC-4.1.3: non-canonical CBOR or > 64 KB is rejected: non-shortest
    /// integers, indefinite lengths, unsorted or duplicate keys, trailing
    /// bytes, empty optional containers.
    #[test]
    fn spec_4_1_3_rejects_non_canonical() {
        let (m, _) = hello();
        let canonical = m.encode().unwrap();
        assert_eq!(Manifest::decode(&canonical).unwrap(), m);

        // memory_max 32 as 0x19 0x00 0x20 instead of 0x18 0x20.
        let mut e = entries(&canonical);
        let i = e.iter().position(|(k, _)| k == "memory_max").unwrap();
        assert_eq!(e[i].1, vec![0x18, 0x20]);
        e[i].1 = vec![0x19, 0x00, 0x20];
        assert!(matches!(
            Manifest::decode(&rebuild(&e)),
            Err(Error::NonCanonical { reason, .. }) if reason.contains("shortest")
        ));

        // Indefinite-length top-level map.
        let mut indefinite = vec![0xbf];
        for (k, v) in entries(&canonical) {
            indefinite.extend(encode_key(&k));
            indefinite.extend(v);
        }
        indefinite.push(0xff);
        assert!(matches!(
            Manifest::decode(&indefinite),
            Err(Error::NonCanonical { reason, .. }) if reason.contains("indefinite")
        ));

        // Keys out of order.
        let mut e = entries(&canonical);
        e.swap(0, 1);
        assert!(matches!(
            Manifest::decode(&rebuild(&e)),
            Err(Error::NonCanonical { reason, .. }) if reason.contains("order")
        ));

        // Duplicate key.
        let mut e = entries(&canonical);
        let dup = e[0].clone();
        e.insert(1, dup);
        assert!(matches!(
            Manifest::decode(&rebuild(&e)),
            Err(Error::DuplicateField(k)) if k == "abi"
        ));

        // Trailing byte.
        let mut trailing = canonical.clone();
        trailing.push(0);
        assert!(matches!(
            Manifest::decode(&trailing),
            Err(Error::Trailing { .. })
        ));

        // Empty optional array must be omitted.
        let mut e = entries(&canonical);
        e.push(("resources".into(), vec![0x80]));
        e.sort_by_key(|a| encode_key(&a.0));
        assert!(matches!(
            Manifest::decode(&rebuild(&e)),
            Err(Error::NonCanonical { reason, .. }) if reason.contains("omitted")
        ));

        // A float where an integer belongs is malformed, not canonical.
        let mut e = entries(&canonical);
        let i = e.iter().position(|(k, _)| k == "memory_max").unwrap();
        e[i].1 = vec![0xf9, 0x50, 0x00];
        assert!(matches!(
            Manifest::decode(&rebuild(&e)),
            Err(Error::Malformed { .. })
        ));
    }

    /// SPEC-4.3.1: unknown ABI major version is rejected before loading.
    #[test]
    fn spec_4_3_1_unknown_major_rejected() {
        let (mut m, wasm) = hello();
        m.abi = "2.0".parse().unwrap();
        assert!(m
            .validate(&wasm, &Vec::<Vec<u8>>::new())
            .contains(&Problem::AbiMajor { found: 2 }));
        assert!(!m.abi.runs_on(AbiVersion::CURRENT));
        // The version text has one form.
        for bad in ["1", "1.0.0", "01.0", "1.00", "v1.0", "1. 0", ""] {
            assert!(bad.parse::<AbiVersion>().is_err(), "{bad:?}");
        }
        assert_eq!("1.0".parse::<AbiVersion>().unwrap(), AbiVersion::CURRENT);
    }

    /// SPEC-4.3.2: a 1.y module runs on a 1.x client when y <= x.
    #[test]
    fn spec_4_3_2_minor_compat() {
        let v = |major, minor| AbiVersion { major, minor };
        assert!(v(1, 0).runs_on(v(1, 0)));
        assert!(v(1, 0).runs_on(v(1, 3)));
        assert!(v(1, 2).runs_on(v(1, 3)));
        assert!(!v(1, 4).runs_on(v(1, 3)));
        assert!(!v(2, 0).runs_on(v(1, 9)));
        assert!(!v(0, 9).runs_on(v(1, 0)));
    }

    /// SPEC-4.4.1: package media type and zstd+tar framing with the entries
    /// `manifest.cbor`, `module.wasm`, `res/1`, … in that order.
    #[test]
    fn spec_4_4_1_package_framing() {
        assert_eq!(MEDIA_PACKAGE, "application/vy-package+zstd");
        let (manifest, module, res) = with_image();
        let p = Package {
            manifest,
            module,
            resources: res.into_iter().map(|r| r.bytes).collect(),
        };
        let bytes = p.pack().unwrap();
        assert_eq!(&bytes[..4], &[0x28, 0xb5, 0x2f, 0xfd], "zstd magic");
        let tar = zstd::stream::decode_all(&bytes[..]).unwrap();
        let entries = tar_read(&tar).unwrap();
        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["manifest.cbor", "module.wasm", "res/1"]);
        assert_eq!(entries[0].1, p.manifest.encode().unwrap());
        assert_eq!(entries[1].1, p.module);
        assert_eq!(entries[2].1, p.resources[0]);
        assert_eq!(Package::unpack(&bytes).unwrap(), p);
        // Packing is deterministic.
        assert_eq!(p.pack().unwrap(), bytes);

        // Entries out of order or extra are refused.
        let shuffled = tar_write(&[
            ("module.wasm".to_string(), &p.module[..]),
            ("manifest.cbor".to_string(), entries[0].1),
        ]);
        let z = zstd::stream::encode_all(&shuffled[..], 1).unwrap();
        assert!(matches!(
            Package::unpack(&z),
            Err(PackageError::MissingEntry("manifest.cbor"))
        ));
        let extra = tar_write(&[
            ("manifest.cbor".to_string(), entries[0].1),
            ("module.wasm".to_string(), &p.module[..]),
            ("res/1".to_string(), &p.resources[0][..]),
            ("README".to_string(), b"no"),
        ]);
        let z = zstd::stream::encode_all(&extra[..], 1).unwrap();
        assert!(matches!(
            Package::unpack(&z),
            Err(PackageError::UnexpectedEntry(n)) if n == "README"
        ));
        assert!(matches!(
            Package::unpack(b"not zstd at all"),
            Err(PackageError::Zstd(_))
        ));

        // The tar subset is readable by the system tar, when there is one.
        let dir = std::env::temp_dir().join(format!("vy-manifest-tar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.tar");
        std::fs::write(&path, &tar).unwrap();
        if let Ok(out) = std::process::Command::new("tar")
            .arg("-tf")
            .arg(&path)
            .output()
        {
            if out.status.success() {
                let listed = String::from_utf8_lossy(&out.stdout);
                let listed: Vec<&str> = listed.lines().collect();
                assert_eq!(listed, names);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-4.5.1: resource index 0 is the module.
    #[test]
    fn spec_4_5_1_resource_index_zero_is_module() {
        let (m, wasm, res) = with_image();
        let e0 = m.entry(0).unwrap();
        assert_eq!(*e0.hash, Hash::of(&wasm));
        assert_eq!(e0.mime, MIME_WASM);
        let e1 = m.entry(1).unwrap();
        assert_eq!(*e1.hash, Hash::of(&res[0].bytes));
        assert_eq!(e1.mime, "image/png");
        assert!(m.entry(2).is_none());
        // Packages name resources the same way: res/1 is index 1.
        assert_eq!(ENTRY_RESOURCE_PREFIX, "res/");
    }

    /// SPEC-8.1.1: required fields are enforced, each one.
    #[test]
    fn spec_8_1_1_required_fields() {
        let (m, wasm) = hello();
        let canonical = m.encode().unwrap();
        for required in [
            "abi",
            "name",
            "summary",
            "lang",
            "module",
            "memory_max",
            "published",
            "sig",
        ] {
            let e: Vec<_> = entries(&canonical)
                .into_iter()
                .filter(|(k, _)| k != required)
                .collect();
            assert_eq!(
                Manifest::decode(&rebuild(&e)),
                Err(Error::MissingField(required)),
                "{required}"
            );
        }
        // Sub-fields too.
        let mut e = entries(&canonical);
        let i = e.iter().position(|(k, _)| k == "module").unwrap();
        e[i].1 = {
            let mut enc = Enc::new(Vec::new());
            enc.map(1).unwrap().str("size").unwrap().u64(1).unwrap();
            enc.into_writer()
        };
        assert_eq!(
            Manifest::decode(&rebuild(&e)),
            Err(Error::MissingField("hash"))
        );
        // Unsigned manifests have no canonical encoding.
        let unsigned = Manifest {
            sig: None,
            ..m.clone()
        };
        assert_eq!(unsigned.encode(), Err(Error::Unsigned));
        // Limits of the sentence.
        let mut long = m.clone();
        long.name = "å".repeat(81);
        long.summary = "s".repeat(2049);
        long.lang = "not a tag!".into();
        let problems = long.validate(&wasm, &Vec::<Vec<u8>>::new());
        assert!(problems.contains(&Problem::NameTooLong { chars: 81 }));
        assert!(problems.contains(&Problem::SummaryTooLong { bytes: 2049 }));
        assert!(problems.contains(&Problem::LangInvalid("not a tag!".into())));
        long.name = "å".repeat(80);
        long.summary = "s".repeat(2048);
        long.lang = "sv-SE".into();
        assert!(long.validate(&wasm, &Vec::<Vec<u8>>::new()).is_empty());
    }

    /// SPEC-8.1.2: optional fields are accepted and unknown fields rejected,
    /// in both forms.
    #[test]
    fn spec_8_1_2_optional_fields() {
        let (mut m, wasm, res) = with_image();
        m.links = vec!["vy://a.example/b".into(), "https://c.example/".into()];
        m.fallback = Some("# Hej\n\nText.".into());
        m.min_size = Some(MinSize { w: 320, h: 200 });
        m.icon = Some(1);
        m.source_url = Some("https://example.com/src".into());
        m.skeleton = Some(1);
        m.chunks = vec![Blob {
            hash: Hash::of(b"chunk"),
            size: 5,
            url: Some("/_vy/chunk".into()),
        }];
        m.embeds = vec!["vy://e.example/".into()];
        m.next_key_id = Some("0123456789abcdef".into());
        m.capabilities = CapabilityRequests {
            net: Some(NetRequest {
                origins: vec!["https://api.example".into()],
            }),
            storage: Some(StorageRequest { quota_kb: 512 }),
            clipboard: Some(Empty {}),
            hires_time: Some(Empty {}),
            ime: Some(Empty {}),
            persist_frame: Some(Empty {}),
        };
        m.sign(&test_key());
        let bytes: Vec<Vec<u8>> = res.iter().map(|r| r.bytes.clone()).collect();
        assert!(m.validate(&wasm, &bytes).is_empty());
        let encoded = m.encode().unwrap();
        assert_eq!(Manifest::decode(&encoded).unwrap(), m);
        assert_eq!(Manifest::from_json(&m.to_json()).unwrap(), m);
        assert!(m.verify(&test_key().public()));

        // Unknown field in CBOR.
        let mut e = entries(&encoded);
        e.push(("zzz".into(), vec![0x01]));
        e.sort_by_key(|a| encode_key(&a.0));
        assert_eq!(
            Manifest::decode(&rebuild(&e)),
            Err(Error::UnknownField("zzz".into()))
        );
        // Unknown field in JSON.
        let json = m.to_json().replacen("\"abi\"", "\"zzz\": 1,\n  \"abi\"", 1);
        assert!(matches!(Manifest::from_json(&json), Err(Error::Json(_))));
        // Unknown capability.
        let mut e = entries(&encoded);
        let i = e.iter().position(|(k, _)| k == "capabilities").unwrap();
        e[i].1 = {
            let mut enc = Enc::new(Vec::new());
            enc.map(1).unwrap().str("file").unwrap().map(0).unwrap();
            enc.into_writer()
        };
        assert_eq!(
            Manifest::decode(&rebuild(&e)),
            Err(Error::UnknownField("capabilities.file".into()))
        );
        // Optional references are checked.
        let mut bad = m.clone();
        bad.icon = Some(2);
        bad.skeleton = Some(0);
        bad.links.push("ftp://x".into());
        bad.embeds.push("https://x".into());
        bad.source_url = Some("http://x".into());
        bad.min_size = Some(MinSize { w: 0, h: 1 });
        let problems = bad.validate(&wasm, &bytes);
        assert!(problems.contains(&Problem::ResourceIndexOutOfRange {
            field: "icon",
            index: 2
        }));
        assert!(problems.contains(&Problem::ResourceIndexOutOfRange {
            field: "skeleton",
            index: 0
        }));
        assert!(problems.contains(&Problem::LinkInvalid("ftp://x".into())));
        assert!(problems.contains(&Problem::EmbedInvalid("https://x".into())));
        assert!(problems.contains(&Problem::SourceUrlInvalid("http://x".into())));
        assert!(problems.contains(&Problem::MinSizeZero));
    }

    /// SPEC-8.2.2: a capability absent from the manifest can never be granted.
    #[test]
    fn spec_8_2_2_no_late_grants() {
        let (mut m, _) = hello();
        m.capabilities.ime = Some(Empty {});
        assert!(m.capabilities.requests("ime"));
        assert!(!m.capabilities.requests("clipboard"));
        // Saying yes to everything grants only what was requested.
        assert_eq!(m.capabilities.grants(|_| true), ["ime"]);
        let none = CapabilityRequests::default();
        assert!(none.grants(|_| true).is_empty());
        assert!(none.iter().next().is_none());
    }

    /// SPEC-8.2.3: default grant policy per capability.
    #[test]
    fn spec_8_2_3_default_policy() {
        let net = NetRequest {
            origins: vec!["https://self.example".into()],
        };
        assert_eq!(default_grant(&Requested::Net(&net), true), Grant::Auto);
        assert_eq!(default_grant(&Requested::Net(&net), false), Grant::Ask);
        let small = StorageRequest { quota_kb: 1024 };
        let big = StorageRequest { quota_kb: 1025 };
        assert_eq!(
            default_grant(&Requested::Storage(&small), false),
            Grant::Auto
        );
        assert_eq!(default_grant(&Requested::Storage(&big), false), Grant::Ask);
        assert_eq!(default_grant(&Requested::Ime, false), Grant::Auto);
        assert_eq!(default_grant(&Requested::Clipboard, true), Grant::Ask);
        assert_eq!(default_grant(&Requested::PersistFrame, true), Grant::Ask);
        assert_eq!(default_grant(&Requested::HiresTime, true), Grant::Denied);
        // Names and ids line up with vy-abi.
        let all = CapabilityRequests {
            net: Some(net.clone()),
            storage: Some(small.clone()),
            clipboard: Some(Empty {}),
            hires_time: Some(Empty {}),
            ime: Some(Empty {}),
            persist_frame: Some(Empty {}),
        };
        let ids: Vec<i32> = all.iter().map(|r| r.id()).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5, 6]);
        let names: Vec<&str> = all.iter().map(|r| r.name()).collect();
        assert_eq!(names, vy_abi::cap::NAMES);
    }

    /// SPEC-8.3.1: fallback is shown when the module is rejected or the window is under min_size.
    #[test]
    #[ignore = "M3: vy-shell decides when to show the fallback"]
    fn spec_8_3_1_fallback_conditions() {
        todo!()
    }

    /// SPEC-8.1.3, decision 29: `memory_max` is 1..=4096 pages and at
    /// least the module's initial memory plus one.
    #[test]
    fn spec_8_1_3_memory_max_bounds() {
        let (mut m, wasm) = hello();
        let no_res = Vec::<Vec<u8>>::new();
        assert_eq!(module_initial_pages(&wasm).unwrap(), 2);
        m.memory_max = 2;
        assert!(m
            .validate(&wasm, &no_res)
            .contains(&Problem::MemoryMaxBelowInitial {
                pages: 2,
                initial: 2
            }));
        m.memory_max = 3;
        assert!(m.validate(&wasm, &no_res).is_empty());
        m.memory_max = 4097;
        assert!(m
            .validate(&wasm, &no_res)
            .contains(&Problem::MemoryMaxOutOfRange { pages: 4097 }));
        m.memory_max = 0;
        assert!(m
            .validate(&wasm, &no_res)
            .contains(&Problem::MemoryMaxOutOfRange { pages: 0 }));
        assert!(module_initial_pages(b"\0asm").is_err());
    }

    /// SPEC-7.4.1: the signature covers the canonical encoding without
    /// `sig`; any change breaks it; another key does not verify.
    #[test]
    fn spec_7_4_1_sign_and_verify() {
        let key = test_key();
        assert_eq!(key.public().to_string(), TEST_PUBLIC_HEX);
        assert_eq!(key.public().key_id().len(), KEY_ID_HEX_LEN);
        assert_eq!(key.public().key_id(), key_id(&key.public().to_bytes()));
        assert_eq!(TEST_PUBLIC_HEX.parse::<PublicKey>().unwrap(), key.public());
        let (m, _) = hello();
        assert!(m.verify(&key.public()));
        assert_eq!(m.sig.as_ref().unwrap().key_id, key.public().key_id());
        let mut tampered = m.clone();
        tampered.memory_max += 1;
        assert!(!tampered.verify(&key.public()));
        let other = SigningKey::from_seed(&[7u8; 32]);
        assert!(!m.verify(&other.public()));
        let unsigned = Manifest { sig: None, ..m };
        assert!(!unsigned.verify(&key.public()));
        // The vy-key text form round-trips and rejects junk lines.
        let other_key = SigningKey::from_seed(&[7u8; 32]);
        let text = key_file(&[key.public(), other_key.public()]);
        assert_eq!(
            parse_key_file(&text).unwrap(),
            vec![key.public(), other_key.public()]
        );
        assert!(parse_key_file("# only a comment\n").unwrap().is_empty());
        assert!(parse_key_file("not hex\n").is_err());
    }

    /// Decision 7: image resources carry `w`/`h`, checked against the PNG
    /// header; SPEC-6.5.1 bounds them; SPEC-4.1.5 bounds the media types.
    #[test]
    fn image_resources_carry_dimensions() {
        let (mut m, wasm, res) = with_image();
        let bytes: Vec<Vec<u8>> = res.iter().map(|r| r.bytes.clone()).collect();
        assert_eq!((m.resources[0].w, m.resources[0].h), (Some(320), Some(200)));
        m.resources[0].w = None;
        assert!(m
            .validate(&wasm, &bytes)
            .contains(&Problem::ImageSizeMissing { index: 1 }));
        m.resources[0].w = Some(321);
        assert!(m
            .validate(&wasm, &bytes)
            .contains(&Problem::ImageSizeMismatch {
                index: 1,
                declared: (321, 200),
                actual: (320, 200)
            }));
        m.resources[0].w = Some(8193);
        assert!(m
            .validate(&wasm, &bytes)
            .iter()
            .any(|p| matches!(p, Problem::ImageTooLarge { index: 1, .. })));
        m.resources[0].w = Some(320);
        m.resources[0].mime = "image/gif".into();
        assert!(m
            .validate(&wasm, &bytes)
            .contains(&Problem::ResourceMimeUnknown {
                index: 1,
                mime: "image/gif".into()
            }));
        m.resources[0].mime = "image/png".into();
        let mut broken = bytes.clone();
        broken[0][12] = b'X';
        let problems = m.validate(&wasm, &broken);
        assert!(problems.contains(&Problem::ImageHeaderInvalid { index: 1 }));
        assert!(problems
            .iter()
            .any(|p| matches!(p, Problem::ResourceHashMismatch { index: 1, .. })));
        assert_eq!(png_dimensions(&png(1, 2)), Some((1, 2)));
        assert_eq!(png_dimensions(b"PNG"), None);
        assert_eq!(mime_for_path("a/b.PNG"), "image/png");
        assert_eq!(mime_for_path("font.woff2"), "font/woff2");
        assert_eq!(mime_for_path("data.bin"), "application/octet-stream");
    }

    /// The JSON form round-trips and reads the draft fixture.
    #[test]
    fn json_form_roundtrip() {
        let (mut m, _) = hello();
        m.capabilities.ime = Some(Empty {});
        m.sign(&test_key());
        let json = m.to_json();
        assert!(json.contains("\"sha256:"));
        assert!(json.contains("\"ed25519:"));
        assert!(json.contains("\"capabilities\": {\n    \"ime\": {}"));
        assert_eq!(Manifest::from_json(&json).unwrap(), m);
        assert!(Draft::from_json("{\"abi\": \"1.0\"}").is_err());
        let d = draft();
        assert_eq!(d.name, "Hej från Vy");
        assert!(d.capabilities.is_empty());
    }

    /// The real hello module, when built, satisfies the fixture.
    #[test]
    fn hello_wasm_matches_fixture() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/wasm/hello.wasm");
        let Ok(wasm) = std::fs::read(path) else {
            eprintln!("skipped: {path} missing (run tools/build_modules.sh)");
            return;
        };
        let mut m = draft().finish(&wasm, &[]);
        m.sign(&test_key());
        let problems = m.validate(&wasm, &Vec::<Vec<u8>>::new());
        assert!(problems.is_empty(), "{problems:?}");
    }
}
