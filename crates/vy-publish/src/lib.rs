//! `vy-publish`: turn a draft manifest, a module and resources into a signed
//! package (SPEC-4.4.1), make keys, and inspect packages.
//!
//! The library part is what `vy-serve`'s end-to-end test and future tooling
//! call; `main.rs` is the command line over it.
#![deny(missing_docs)]

use std::fmt;
use std::path::{Path, PathBuf};
use vy_manifest::{
    key_file, mime_for_path, Draft, Error as ManifestError, Package, PackageError, Problem,
    PublicKey, ResourceInput, SigningKey,
};

/// Why a package could not be built.
#[derive(Debug)]
pub enum BuildError {
    /// The manifest does not describe the inputs; nothing was written.
    Invalid(Vec<Problem>),
    /// Encoding failed.
    Manifest(ManifestError),
    /// A file could not be read or written.
    Io {
        /// The path.
        path: PathBuf,
        /// The error.
        error: std::io::Error,
    },
    /// The package does not read back.
    Package(PackageError),
    /// Entropy was not available.
    Entropy(String),
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(problems) => {
                write!(f, "manifest does not describe the package:")?;
                for p in problems {
                    write!(f, "\n  {p}")?;
                }
                Ok(())
            }
            Self::Manifest(e) => write!(f, "{e}"),
            Self::Io { path, error } => write!(f, "{}: {error}", path.display()),
            Self::Package(e) => write!(f, "{e}"),
            Self::Entropy(e) => write!(f, "no entropy for key generation: {e}"),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<ManifestError> for BuildError {
    fn from(e: ManifestError) -> Self {
        Self::Manifest(e)
    }
}

fn io_err(path: &Path, error: std::io::Error) -> BuildError {
    BuildError::Io {
        path: path.to_path_buf(),
        error,
    }
}

/// Fill the draft, sign it with `key` and check the result against the
/// bytes. The package is returned only when [`Package::validate`] has
/// nothing to say.
pub fn build(
    draft: &Draft,
    module: Vec<u8>,
    resources: Vec<ResourceInput>,
    key: &SigningKey,
) -> Result<Package, BuildError> {
    let mut manifest = draft.finish(&module, &resources);
    manifest.sign(key);
    let package = Package {
        manifest,
        module,
        resources: resources.into_iter().map(|r| r.bytes).collect(),
    };
    let problems = package.validate();
    if !problems.is_empty() {
        return Err(BuildError::Invalid(problems));
    }
    Ok(package)
}

/// Read the resources a draft names, relative to the draft file's
/// directory, followed by `extra` paths taken as they are.
pub fn load_resources(
    draft_path: &Path,
    draft: &Draft,
    extra: &[PathBuf],
) -> Result<Vec<ResourceInput>, BuildError> {
    let base = draft_path.parent().unwrap_or(Path::new("."));
    let mut out = Vec::new();
    for r in &draft.resources {
        let path = base.join(&r.path);
        let bytes = std::fs::read(&path).map_err(|e| io_err(&path, e))?;
        out.push(ResourceInput {
            mime: r
                .mime
                .clone()
                .unwrap_or_else(|| mime_for_path(&r.path).to_string()),
            bytes,
            size_hint: r.w.zip(r.h),
        });
    }
    for path in extra {
        let bytes = std::fs::read(path).map_err(|e| io_err(path, e))?;
        out.push(ResourceInput {
            mime: mime_for_path(&path.to_string_lossy()).to_string(),
            bytes,
            size_hint: None,
        });
    }
    Ok(out)
}

/// A fresh Ed25519 key from OS entropy.
pub fn keygen() -> Result<SigningKey, BuildError> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| BuildError::Entropy(e.to_string()))?;
    Ok(SigningKey::from_seed(&seed))
}

/// Write the 32-byte seed to `path` (owner-only on Unix) and the public key
/// in `vy-key` text form to `<path>.pub`. Refuses to overwrite.
pub fn write_key(path: &Path, key: &SigningKey) -> Result<PathBuf, BuildError> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(path).map_err(|e| io_err(path, e))?;
    f.write_all(&key.seed()).map_err(|e| io_err(path, e))?;
    let pub_path = public_key_path(path);
    std::fs::write(&pub_path, key_file(&[key.public()])).map_err(|e| io_err(&pub_path, e))?;
    Ok(pub_path)
}

/// `<path>.pub`.
pub fn public_key_path(key_path: &Path) -> PathBuf {
    let mut s = key_path.as_os_str().to_os_string();
    s.push(".pub");
    PathBuf::from(s)
}

/// Read a seed file written by [`write_key`].
pub fn read_key(path: &Path) -> Result<SigningKey, BuildError> {
    let bytes = std::fs::read(path).map_err(|e| io_err(path, e))?;
    let seed: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
        io_err(
            path,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("expected a 32-byte seed, found {} bytes", bytes.len()),
            ),
        )
    })?;
    Ok(SigningKey::from_seed(&seed))
}

/// What `inspect` found.
#[derive(Debug)]
pub struct Inspection {
    /// The package, hashes verified.
    pub package: Package,
    /// Compressed size.
    pub compressed_bytes: usize,
    /// Signature status against the key given, if any.
    pub signature: SignatureStatus,
}

/// Signature status of an inspected package.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureStatus {
    /// No public key was given; the signature could not be checked.
    NotChecked,
    /// Verifies with the key given.
    Valid,
    /// Does not verify with the key given.
    Invalid,
}

/// Unpack and validate; verify the signature when a key is given.
pub fn inspect(bytes: &[u8], public_key: Option<&PublicKey>) -> Result<Inspection, BuildError> {
    let package = Package::unpack(bytes).map_err(BuildError::Package)?;
    let signature = match public_key {
        None => SignatureStatus::NotChecked,
        Some(k) if package.manifest.verify(k) => SignatureStatus::Valid,
        Some(_) => SignatureStatus::Invalid,
    };
    Ok(Inspection {
        package,
        compressed_bytes: bytes.len(),
        signature,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use vy_manifest::Hash;

    const SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];

    fn module() -> Vec<u8> {
        wat::parse_str(
            r#"(module (memory (export "memory") 2)
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32) i32.const 0)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
        )
        .unwrap()
    }

    fn draft() -> Draft {
        Draft::from_json(include_str!("../../vy-manifest/tests/fixtures/hello.json")).unwrap()
    }

    /// SPEC-4.4.1, SPEC-7.4.1: build fills hashes, signs, and the package
    /// reads back with a valid signature.
    #[test]
    fn build_signs_and_roundtrips() {
        let key = SigningKey::from_seed(&SEED);
        let wasm = module();
        let p = build(&draft(), wasm.clone(), vec![], &key).unwrap();
        assert_eq!(p.manifest.module.hash, Hash::of(&wasm));
        assert!(p.manifest.verify(&key.public()));
        let bytes = p.pack().unwrap();
        let i = inspect(&bytes, Some(&key.public())).unwrap();
        assert_eq!(i.signature, SignatureStatus::Valid);
        assert_eq!(i.package, p);
        let other = SigningKey::from_seed(&[1u8; 32]);
        assert_eq!(
            inspect(&bytes, Some(&other.public())).unwrap().signature,
            SignatureStatus::Invalid
        );
        assert_eq!(
            inspect(&bytes, None).unwrap().signature,
            SignatureStatus::NotChecked
        );
    }

    /// A draft whose memory_max cannot hold the module is refused before
    /// anything is written (SPEC-8.1.3).
    #[test]
    fn build_refuses_inconsistent_input() {
        let key = SigningKey::from_seed(&SEED);
        let mut d = draft();
        d.memory_max = 1;
        assert!(matches!(
            build(&d, module(), vec![], &key),
            Err(BuildError::Invalid(p)) if p.iter().any(|p| matches!(p, Problem::MemoryMaxBelowInitial { .. }))
        ));
    }

    /// Keys: seed file round-trips, the .pub file is the vy-key text form.
    #[test]
    fn key_files_roundtrip() {
        let dir = std::env::temp_dir().join(format!("vy-publish-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dev.key");
        let key = keygen().unwrap();
        let pub_path = write_key(&path, &key).unwrap();
        assert_eq!(pub_path, dir.join("dev.key.pub"));
        assert_eq!(read_key(&path).unwrap().seed(), key.seed());
        let text = std::fs::read_to_string(&pub_path).unwrap();
        assert_eq!(
            vy_manifest::parse_key_file(&text).unwrap(),
            vec![key.public()]
        );
        assert!(write_key(&path, &key).is_err(), "never overwrites a key");
        assert_ne!(keygen().unwrap().seed(), key.seed());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
