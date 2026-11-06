//! The compilation cache (SPEC-13.1.4): machine code wasmtime compiled for a
//! module, kept on disk by the module's SHA-256, so a module loaded again
//! is not compiled again.
//!
//! An entry is `MAGIC`, the SHA-256 of the payload, and the payload, which
//! is `Module::serialize`. Its name is the hex SHA-256 of the module's hash,
//! the cache format and the engine's compatibility hash (wasmtime's version
//! and every compilation setting of the engine in `lib.rs`), so another
//! wasmtime or another configuration never finds it.
//!
//! **Deserializing trusts the bytes:** `Module::deserialize` maps them as
//! executable code without validating them, so whoever can write an entry
//! can run code in every process that reads it. The mitigations, in
//! `docs/notes/m4-perf.md` under "Hotbild": only the client writes
//! ([`CompileCache::ReadWrite`]; a tab process gets [`CompileCache::ReadOnly`]
//! or nothing), the directory is the user's own (created 0700 on Unix and
//! refused when another user owns it or group or others may write it),
//! entries are written to a
//! temporary file and renamed, read whole into memory (never mapped from
//! the file, which could change under the module), and checked against
//! the payload hash, which catches a torn or truncated file but not an
//! attacker who can write the directory.

use crate::{sha256, Error};
use sha2::{Digest, Sha256};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use wasmtime::{Engine, Module};

/// First bytes of every entry; the version of this format.
const MAGIC: &[u8; 8] = b"VYCODE01";
/// Largest total size of a cache directory; past it the oldest entries
/// are removed when a new one is written.
pub const COMPILE_CACHE_CAP_BYTES: u64 = 256 << 20;

/// Where compiled machine code of modules is kept between loads
/// (SPEC-13.1.4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CompileCache {
    /// No cache: every load compiles and nothing is written. The default,
    /// so `vy-cli render`, the conformance suite and the tests keep no
    /// state on disk.
    #[default]
    Off,
    /// Use entries under this directory, never write them: a tab process.
    ReadOnly(PathBuf),
    /// Use and write entries under this directory: the client's main
    /// process, the only writer.
    ReadWrite(PathBuf),
}

impl CompileCache {
    fn dir(&self) -> Option<&Path> {
        match self {
            Self::Off => None,
            Self::ReadOnly(d) | Self::ReadWrite(d) => Some(d),
        }
    }
}

/// How [`crate::Runtime::load`] got the compiled module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileSource {
    /// Compiled by Cranelift now.
    Compiled,
    /// Read from the compilation cache.
    CacheHit,
}

/// What getting the compiled module cost (SPEC-13.1.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadStats {
    /// Compiled or read from the cache.
    pub source: CompileSource,
    /// Wall-clock time to get the compiled module: compiling, or reading,
    /// checking and deserializing the entry. Hashing, the ABI check and
    /// instantiation are not included.
    pub module_time: Duration,
    /// True when a new entry was written.
    pub written: bool,
}

/// The entry name for a module: hex SHA-256 over the format, the engine's
/// compatibility hash and the module's SHA-256.
fn entry_name(engine: &Engine, module_sha256: &[u8; 32]) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    engine.precompile_compatibility_hash().hash(&mut h);
    let mut key = Sha256::new();
    key.update(MAGIC);
    key.update(h.finish().to_le_bytes());
    key.update(module_sha256);
    format!("{}.code", crate::hex(&key.finalize()))
}

/// True when `dir` may hold entries: it exists and is a directory and, on
/// Unix, the user this process runs as (the effective uid) owns it and no
/// one else may write it. Mode alone is not enough: a 0700 directory that
/// another user made, and put where the cache is looked for, is theirs to
/// fill.
///
/// On Windows only existence and kind are checked; the ACL is not read.
/// The cache lives in the user's profile (`%LOCALAPPDATA%`), which only
/// the user, SYSTEM and administrators may write, and a tab process runs
/// at low integrity and cannot write there (SPEC-9.1.3).
fn dir_is_private(dir: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(dir) else {
        return false;
    };
    if !meta.is_dir() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if meta.permissions().mode() & 0o022 != 0 {
            return false;
        }
        // SAFETY: `geteuid` takes nothing, always succeeds and touches no
        // memory of ours.
        if meta.uid() != unsafe { libc::geteuid() } {
            return false;
        }
    }
    true
}

/// Read the entry for a module, check it and deserialize it; `None` for a
/// missing, short, foreign or damaged entry, or one wasmtime refuses.
fn read(engine: &Engine, path: &Path) -> Option<Module> {
    let bytes = std::fs::read(path).ok()?;
    let rest = bytes.strip_prefix(MAGIC.as_slice())?;
    let (digest, payload) = rest.split_at_checked(32)?;
    if digest != sha256(payload) {
        return None;
    }
    // SAFETY: the bytes are an entry this client wrote with
    // `Module::serialize` for this engine configuration (the name holds
    // the compatibility hash, wasmtime checks its version and settings
    // again), read whole into memory so they cannot change under the
    // module, and checked against the hash written with them. Who can
    // write the directory is the trust boundary: see the module docs.
    unsafe { Module::deserialize(engine, payload) }.ok()
}

/// Write the entry for `module` atomically: a temporary file in the
/// directory, then a rename. Errors are ignored by the caller: a cache that
/// cannot be written is a cache miss next time.
fn write(dir: &Path, name: &str, module: &Module) -> std::io::Result<()> {
    let payload = module
        .serialize()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    let mut entry = Vec::with_capacity(MAGIC.len() + 32 + payload.len());
    entry.extend_from_slice(MAGIC);
    entry.extend_from_slice(&sha256(&payload));
    entry.extend_from_slice(&payload);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = dir.join(format!(".{name}.{}.{nanos}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = options
        .open(&tmp)
        .and_then(|mut f| f.write_all(&entry))
        .and_then(|()| std::fs::rename(&tmp, dir.join(name)));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    evict(dir);
    Ok(())
}

/// Remove the oldest entries until the directory holds at most
/// [`COMPILE_CACHE_CAP_BYTES`].
fn evict(dir: &Path) {
    let Ok(read_dir) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<(std::time::SystemTime, u64, PathBuf)> = read_dir
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().ends_with(".code"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some((meta.modified().ok()?, meta.len(), e.path()))
        })
        .collect();
    let mut total: u64 = entries.iter().map(|(_, len, _)| len).sum();
    entries.sort();
    for (_, len, path) in entries {
        if total <= COMPILE_CACHE_CAP_BYTES {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

/// Create a cache directory the way the client wants it: 0700 on Unix.
pub fn create_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// The compiled module for verified `wasm` with SHA-256 `module_sha256`:
/// from the cache when it holds a good entry, otherwise compiled now (and
/// written when the cache is [`CompileCache::ReadWrite`]).
pub(crate) fn module(
    engine: &Engine,
    wasm: &[u8],
    module_sha256: &[u8; 32],
    cache: &CompileCache,
) -> Result<(Module, LoadStats), Error> {
    let started = Instant::now();
    let usable = cache.dir().filter(|d| {
        if let CompileCache::ReadWrite(dir) = cache {
            let _ = create_dir(dir);
        }
        dir_is_private(d)
    });
    let name = usable.map(|_| entry_name(engine, module_sha256));
    if let (Some(dir), Some(name)) = (usable, &name) {
        if let Some(module) = read(engine, &dir.join(name)) {
            return Ok((
                module,
                LoadStats {
                    source: CompileSource::CacheHit,
                    module_time: started.elapsed(),
                    written: false,
                },
            ));
        }
    }
    let module = Module::new(engine, wasm).map_err(|e| Error::Instantiate(e.to_string()))?;
    let module_time = started.elapsed();
    let mut written = false;
    if let (CompileCache::ReadWrite(dir), Some(name)) = (cache, &name) {
        if usable.is_some() {
            written = write(dir, name, &module).is_ok();
        }
    }
    Ok((
        module,
        LoadStats {
            source: CompileSource::Compiled,
            module_time,
            written,
        },
    ))
}

// The ownership check is Unix's; on Windows only existence is checked.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Review 0007, finding 6: a directory holds entries only when this
    /// process's user owns it and no one else may write it. As root the
    /// directory is given to uid 65534; as anyone else `/`, which root
    /// owns and others cannot write, stands for a directory of another
    /// user.
    #[test]
    fn private_dirs_are_owned_by_this_user() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let mode = |m: u32| std::fs::Permissions::from_mode(m);
        let dir = std::env::temp_dir().join(format!("vy-cc-owner-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, mode(0o700)).unwrap();
        assert!(dir_is_private(&dir));
        std::fs::set_permissions(&dir, mode(0o777)).unwrap();
        assert!(!dir_is_private(&dir), "writable by others");
        std::fs::set_permissions(&dir, mode(0o700)).unwrap();
        assert!(!dir_is_private(&dir.join("missing")));
        // SAFETY: as in `dir_is_private`.
        let me = unsafe { libc::geteuid() };
        if me == 0 {
            std::os::unix::fs::chown(&dir, Some(65534), Some(65534)).unwrap();
            assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);
            assert!(!dir_is_private(&dir), "0700, owned by uid 65534");
        } else {
            let root = std::fs::metadata("/").unwrap();
            assert_ne!(root.uid(), me);
            assert_eq!(root.mode() & 0o022, 0);
            assert!(!dir_is_private(Path::new("/")), "owned by root");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
