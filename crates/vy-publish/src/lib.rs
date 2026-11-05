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
    /// A frame of the publish-time run used more than the 8 ms budget
    /// (SPEC-13.4.1); `vy-publish build --allow-slow` publishes anyway.
    OverBudget {
        /// Which frame ([`FrameCost::frame`]).
        frame: String,
        /// Fuel it used.
        fuel: u64,
        /// The budget, `vy_runtime::FRAME_BUDGET_FUEL`.
        budget: u64,
    },
    /// The module could not be loaded or started for the publish-time run.
    Run(String),
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
            Self::OverBudget {
                frame,
                fuel,
                budget,
            } => write!(
                f,
                "{frame} used {fuel} fuel ({:.1} ms of logical time), over the budget of {budget} ({} ms, SPEC-13.4.1); --allow-slow publishes it anyway",
                *fuel as f64 / vy_runtime::FUEL_PER_MS as f64,
                budget / vy_runtime::FUEL_PER_MS
            ),
            Self::Run(e) => write!(f, "the module does not run: {e}"),
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
/// directory ([`Draft::load_resources`]), followed by `extra` paths taken
/// as they are.
pub fn load_resources(
    draft_path: &Path,
    draft: &Draft,
    extra: &[PathBuf],
) -> Result<Vec<ResourceInput>, BuildError> {
    let base = draft_path.parent().unwrap_or(Path::new("."));
    let mut out = draft.load_resources(base)?;
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
        Some(k) if package.manifest.verify(k).is_some() => SignatureStatus::Valid,
        Some(_) => SignatureStatus::Invalid,
    };
    Ok(Inspection {
        package,
        compressed_bytes: bytes.len(),
        signature,
    })
}

/// One frame of the publish-time run and what it cost (SPEC-13.4.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameCost {
    /// Which frame: its number and what came before it.
    pub frame: String,
    /// Fuel it consumed, host work included (`vy_runtime::FrameStats::fuel`).
    pub fuel: u64,
}

/// The window of the publish-time run, then the one it is resized to.
const CHECK_WINDOW: (u32, u32, f32) = (800, 600, 1.0);
const CHECK_RESIZE: (u32, u32, f32) = (1280, 800, 2.0);
/// Frames an animating page gets after the scripted steps.
const CHECK_EXTRA_FRAMES: usize = 5;

/// Run the package's module headless the way a client starts it, and
/// return what each frame cost (SPEC-13.4.1): `vy_init` at 800 × 600, the
/// first frame, a resize to 1280 × 800 at scale 2, a pointer move, press
/// and release at (400, 300), Enter pressed and released and a scroll of
/// 120 px, each followed by the
/// frame it asks for, and up to five more frames while the page animates.
/// Fuel is machine-independent (`vy_runtime::FUEL_PER_MS`), so the
/// verdict is the same on every machine. Returns
/// [`BuildError::OverBudget`] for the first frame over
/// `vy_runtime::FRAME_BUDGET_FUEL` (8 ms of logical time), and
/// [`BuildError::Run`] when the module does not load or start.
pub fn check_frames(package: &Package) -> Result<Vec<FrameCost>, BuildError> {
    use vy_abi::{event, key};
    let manifest = &package.manifest;
    let resources = package
        .resources
        .iter()
        .zip(&manifest.resources)
        .map(|(bytes, r)| vy_runtime::Resource {
            bytes: bytes.clone(),
            pixels: r.w.zip(r.h),
        })
        .collect();
    let options = vy_runtime::Options {
        memory_max_pages: manifest.memory_max,
        resources,
        ..vy_runtime::Options::default()
    };
    let run = |e: vy_runtime::Error| BuildError::Run(e.to_string());
    let mut rt = vy_runtime::Runtime::load(&package.module, &manifest.module.hash.0, options)
        .map_err(run)?;
    let (w, h, s) = CHECK_WINDOW;
    rt.init(w, h, s).map_err(run)?;
    let mut costs: Vec<FrameCost> = Vec::new();
    let mut frame = |rt: &mut vy_runtime::Runtime, after: &str| -> Result<bool, BuildError> {
        if !rt.wants_frame() {
            return Ok(false);
        }
        let n = costs.len() + 1;
        let fuel = match rt.frame() {
            Ok(_) => rt.last_frame().fuel,
            // Stopped at the hard limit: at least that much.
            Err(vy_runtime::Error::OverBudget { .. }) => vy_runtime::CALL_HARD_LIMIT_FUEL,
            Err(e) => return Err(run(e)),
        };
        let cost = FrameCost {
            frame: format!("frame {n} ({after})"),
            fuel,
        };
        if fuel > vy_runtime::FRAME_BUDGET_FUEL {
            return Err(BuildError::OverBudget {
                frame: cost.frame,
                fuel,
                budget: vy_runtime::FRAME_BUDGET_FUEL,
            });
        }
        costs.push(cost);
        Ok(true)
    };
    frame(&mut rt, &format!("first, {w} × {h} at scale {s}"))?;
    let (w, h, s) = CHECK_RESIZE;
    rt.resize(w, h, s).map_err(run)?;
    frame(
        &mut rt,
        &format!("after a resize to {w} × {h} at scale {s}"),
    )?;
    // SPEC-A.2.1: coordinates × 100, the primary button, no modifiers.
    let steps = [
        ("a pointer move", event::POINTER_MOVE, 40_000, 30_000, 0),
        ("a pointer press", event::POINTER_DOWN, 40_000, 30_000, 1),
        ("a pointer release", event::POINTER_UP, 40_000, 30_000, 1),
        ("a key press", event::KEY_DOWN, key::ENTER, 0, 0),
        ("a key release", event::KEY_UP, key::ENTER, 0, 0),
        ("a scroll", event::SCROLL, 0, 12_000, 0),
    ];
    for (what, kind, a, b, c) in steps {
        rt.event(kind, a, b, c, 0).map_err(run)?;
        frame(&mut rt, &format!("after {what}"))?;
    }
    for _ in 0..CHECK_EXTRA_FRAMES {
        if !frame(&mut rt, "animating")? {
            break;
        }
    }
    Ok(costs)
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
        Draft::from_json(include_str!("../../../conformance/manifests/hello.json")).unwrap()
    }

    /// SPEC-4.4.1, SPEC-7.4.1: build fills hashes, signs, and the package
    /// reads back with a valid signature.
    #[test]
    fn build_signs_and_roundtrips() {
        let key = SigningKey::from_seed(&SEED);
        let wasm = module();
        let p = build(&draft(), wasm.clone(), vec![], &key).unwrap();
        assert_eq!(p.manifest.module.hash, Hash::of(&wasm));
        assert!(p.manifest.verify(&key.public()).is_some());
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

    /// A module whose frames cost almost nothing until the window is wider
    /// than 1 000 px, when each frame loops `iterations` times.
    fn slow_after_resize(iterations: u32) -> Vec<u8> {
        wat::parse_str(format!(
            r#"(module (memory (export "memory") 2)
                (global $w (mut i32) (i32.const 0))
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32)
                    (global.set $w (local.get 1)) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32)
                    (local $i i32)
                    (if (i32.gt_s (global.get $w) (i32.const 1000)) (then
                        (loop $spin
                            (local.set $i (i32.add (local.get $i) (i32.const 1)))
                            (br_if $spin (i32.lt_u (local.get $i) (i32.const {iterations}))))))
                    i32.const 0)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
                (func (export "vy_resize") (param i32 i32 f32) (global.set $w (local.get 0)))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
        ))
        .unwrap()
    }

    /// SPEC-13.4.1: the publish-time run measures every frame in fuel; a
    /// module under the budget passes with the cost of each frame, one over
    /// it is refused with the frame named and its fuel.
    #[test]
    fn check_frames_names_the_slow_frame() {
        let key = SigningKey::from_seed(&SEED);
        let fast = build(&draft(), module(), vec![], &key).unwrap();
        let costs = check_frames(&fast).unwrap();
        assert_eq!(costs.len(), 2, "first frame and the resize: {costs:?}");
        assert!(costs.iter().all(|c| c.fuel < vy_runtime::FRAME_BUDGET_FUEL));

        let slow = build(&draft(), slow_after_resize(5_000_000), vec![], &key).unwrap();
        match check_frames(&slow) {
            Err(BuildError::OverBudget {
                frame,
                fuel,
                budget,
            }) => {
                assert_eq!(frame, "frame 2 (after a resize to 1280 × 800 at scale 2)");
                assert!(fuel > budget && budget == vy_runtime::FRAME_BUDGET_FUEL);
            }
            other => panic!("{other:?}"),
        }
        // Under the budget after the resize too: passes.
        let mild = build(&draft(), slow_after_resize(1_000), vec![], &key).unwrap();
        assert_eq!(check_frames(&mild).unwrap().len(), 2);
    }
}
