//! The compilation cache (SPEC-13.1.4, SPEC-9.4.2): machine code wasmtime
//! compiled for a module, kept on disk by the client's main process, so a
//! module loaded again is not compiled again.
//!
//! Two halves, in two processes:
//!
//! - A **tab process** has no file system (SPEC-9.1.3). Its runtime is given
//!   [`CompileCache::Host`]: the code the main process held for the module,
//!   if any, which it deserializes instead of compiling; when it compiles,
//!   it keeps the serialized module for the main process to store
//!   ([`crate::Runtime::take_code`]).
//! - The **main process** owns the [`CodeStore`]: files of bytes it never
//!   deserializes. An entry's name is the [`CodeKey`], which the main
//!   process computes from the module bytes it holds, this build's
//!   engine (wasmtime's version and every compilation setting of the
//!   engine in `lib.rs`) and the client's version; nothing a tab process
//!   says is part of it. An entry is therefore never offered by another
//!   client version than the one that kept it (granskning 0014:1): code
//!   that got into the store through a fault of the client's own host
//!   code stops being used when the client is updated, and the directory
//!   is emptied the first time another version opens it
//!   ([`CodeStore::open`]).
//!
//! **Deserializing trusts the bytes:** `Module::deserialize` maps them as
//! executable code without validating them, so an entry is native code for
//! whoever deserializes it. What bounds that is in
//! `docs/notes/m5-polish.md` under "Hotbild": an entry for a module is
//! only ever what a tab process loaded with that same module handed over,
//! it is stored only under that module's key and handed only to a tab
//! process loaded with that module, where it runs inside the OS sandbox;
//! the directory is the user's own (created 0700 on Unix and refused when
//! another user owns it or group or others may write it); entries are
//! written to a temporary file and renamed, read whole into memory, and
//! checked against the hash written with them, which catches a torn or
//! damaged file but not someone who can write the directory.

use crate::{sha256, Error};
use sha2::{Digest, Sha256};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime};
use wasmtime::{Engine, Module};

/// First bytes of every entry; the version of this format.
const MAGIC: &[u8; 8] = b"VYCODE01";
/// Bytes of an entry before the code: the magic and the code's SHA-256.
const HEADER_BYTES: usize = MAGIC.len() + 32;
/// Largest total size of a cache directory; past it the entries used
/// longest ago are removed when a new one is written.
pub const COMPILE_CACHE_CAP_BYTES: u64 = 256 << 20;
/// Largest code kept for one module, whatever the module's size.
pub const MAX_CODE_BYTES: usize = 64 << 20;
/// Largest code kept for one module, in bytes per byte of the module:
/// Cranelift's output for a real module is a few times its size, and a
/// module of a few hundred bytes still gets a few kilobytes of headers.
pub const MAX_CODE_PER_MODULE_BYTE: usize = 256;
/// How old a temporary file of an interrupted write has to be before it
/// is cleared away.
const STALE_TMP: Duration = Duration::from_secs(600);
/// The version of the client the entries of this build belong to. Every
/// crate of the workspace carries the one version
/// (`workspace.package.version`), so the runtime's is the client's;
/// `vy-shell` has a test that holds the two equal.
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// The file in a cache directory that says which client version its
/// entries were kept by ([`CodeStore::open`]). Not an entry: it has no
/// `.code` in its name.
pub const STAMP_FILE: &str = "VERSION";
/// Longest stamp read: a stamp is the format and a version, a few bytes.
const MAX_STAMP_BYTES: u64 = 256;

/// How a runtime gets the compiled code of its module (SPEC-13.1.4).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CompileCache {
    /// No cache: every load compiles and nothing is kept. The default, so
    /// `vy-cli render`, the conformance suite and the tests keep no state.
    #[default]
    Off,
    /// The host process holds the cache, since a tab process has no file
    /// system: `code` is what its [`CodeStore`] held for this module, if
    /// anything. Code that deserializes is used instead of compiling; when
    /// the load compiles (nothing offered, or the offer refused), the
    /// serialized module is kept for the host ([`crate::Runtime::take_code`]).
    Host {
        /// The entry the host process read for this module.
        code: Option<Vec<u8>>,
    },
}

/// How [`crate::Runtime::load`] got the compiled module.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileSource {
    /// Compiled by Cranelift now.
    Compiled,
    /// Deserialized from the code the host process offered.
    CacheHit,
}

/// What getting the compiled module cost (SPEC-13.1.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadStats {
    /// Compiled or taken from the cache.
    pub source: CompileSource,
    /// Wall-clock time to get the compiled module: compiling, or
    /// deserializing the offered code. Hashing, the ABI check and
    /// instantiation are not included.
    pub module_time: Duration,
    /// True when code was offered and did not deserialize, so the module
    /// was compiled after all: the host drops that entry.
    pub refused: bool,
}

/// This build's engine as eight bytes: wasmtime's version and every
/// compilation setting, which wasmtime hashes for exactly this purpose.
fn engine_tag(engine: &Engine) -> [u8; 8] {
    static TAG: OnceLock<[u8; 8]> = OnceLock::new();
    *TAG.get_or_init(|| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        engine.precompile_compatibility_hash().hash(&mut h);
        h.finish().to_le_bytes()
    })
}

/// The name of a module's entry in a [`CodeStore`] (SPEC-9.4.2): the hex
/// SHA-256 over the cache format, this build's engine, the client's
/// version and the SHA-256 of the module's bytes. Made only from the
/// module bytes the caller holds; there is no way to make one from a name
/// or a path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CodeKey(String);

impl CodeKey {
    /// The key of the module with exactly these bytes, in this client
    /// ([`CLIENT_VERSION`]).
    pub fn of(wasm: &[u8]) -> CodeKey {
        Self::of_client(wasm, CLIENT_VERSION)
    }

    /// The key the client of version `client` makes for the module with
    /// exactly these bytes: what [`CodeKey::of`] is for this build, and
    /// how the tests name an entry another version kept. Two versions
    /// never make the same key for a module, so neither finds the other's
    /// code.
    pub fn of_client(wasm: &[u8], client: &str) -> CodeKey {
        let mut key = Sha256::new();
        key.update(MAGIC);
        key.update(engine_tag(crate::engine()));
        // The one field of no fixed length, so with its length.
        key.update((client.len() as u64).to_le_bytes());
        key.update(client.as_bytes());
        key.update(sha256(wasm));
        CodeKey(format!("{}.code", crate::hex(&key.finalize())))
    }

    /// The entry's file name.
    pub fn file_name(&self) -> &str {
        &self.0
    }
}

/// True when `dir` may hold entries: it exists and is a directory and, on
/// Unix, the user this process runs as (the effective uid) owns it and no
/// one else may write it. Mode alone is not enough: a 0700 directory that
/// another user made, and put where the cache is looked for, is theirs to
/// fill.
///
/// On Windows only existence and kind are checked; the ACL is not read.
/// The cache lives in the user's profile (`%APPDATA%`), which only the
/// user, SYSTEM and administrators may write, and a tab process runs at
/// low integrity and cannot write there (SPEC-9.1.3).
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

/// How the store makes a file: new, and on Unix the user's alone (0600).
fn private_file() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

/// Why [`CodeStore::put`] kept nothing.
#[derive(Debug, PartialEq, Eq)]
pub enum PutError {
    /// No code at all.
    Empty,
    /// More than [`MAX_CODE_BYTES`], or more than
    /// [`MAX_CODE_PER_MODULE_BYTE`] times the module's size.
    TooLarge,
    /// The directory is no longer the user's own.
    NotPrivate,
    /// The file could not be written.
    Io(String),
}

impl std::fmt::Display for PutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("no code"),
            Self::TooLarge => f.write_str("code over the size cap for its module"),
            Self::NotPrivate => f.write_str("the cache directory is not the user's own"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

/// The main process's store of compiled code (SPEC-9.4.2): one file per
/// [`CodeKey`] in a directory of the user's own. It holds bytes and never
/// deserializes them; that is a tab process's, inside its sandbox.
#[derive(Clone, Debug)]
pub struct CodeStore {
    dir: PathBuf,
    cap_bytes: u64,
}

impl CodeStore {
    /// The store at `dir`, created 0700 when it is missing. `None` when
    /// the directory cannot be made, is not the user's own (another
    /// owner, or writable by group or others) or cannot be stamped: then
    /// there is no cache.
    ///
    /// The directory carries the version of the client that keeps its
    /// entries ([`STAMP_FILE`]). When that is not this client's, or there
    /// is none, the directory is emptied and stamped anew: no client
    /// version could find another's entries anyway (the version is in the
    /// [`CodeKey`]), and what a change of client or of wasmtime left
    /// unreadable does not lie there until the size cap pushes it out.
    pub fn open(dir: &Path) -> Option<CodeStore> {
        Self::open_for(dir, CLIENT_VERSION)
    }

    /// [`CodeStore::open`] as the client of version `client` does it:
    /// what `open` is for this build, and how the tests leave a directory
    /// another version kept.
    pub fn open_for(dir: &Path, client: &str) -> Option<CodeStore> {
        let _ = create_dir(dir);
        if !dir_is_private(dir) {
            return None;
        }
        let store = CodeStore {
            dir: dir.to_path_buf(),
            cap_bytes: COMPILE_CACHE_CAP_BYTES,
        };
        store.stamp(client).then_some(store)
    }

    /// Make the directory this client's: true when its stamp names
    /// `client` already, or once everything in it is removed and the
    /// stamp written. The stamp is the cache format and the version.
    fn stamp(&self, client: &str) -> bool {
        let want = format!("{} {client}\n", String::from_utf8_lossy(MAGIC));
        let path = self.dir.join(STAMP_FILE);
        let found = std::fs::symlink_metadata(&path)
            .ok()
            .filter(|m| m.is_file() && m.len() <= MAX_STAMP_BYTES)
            .and_then(|_| std::fs::read(&path).ok());
        if found.as_deref() == Some(want.as_bytes()) {
            return true;
        }
        self.clear();
        // A temporary file and a rename, as for an entry: the stamp is
        // whole or absent, and a link left under its name is replaced,
        // never written through.
        let tmp = self
            .dir
            .join(format!(".{STAMP_FILE}.{}.tmp", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        let written = private_file()
            .open(&tmp)
            .and_then(|mut f| f.write_all(want.as_bytes()))
            .and_then(|()| std::fs::rename(&tmp, &path));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written.is_ok()
    }

    /// Remove every entry, and the temporary files of writes under way or
    /// interrupted: the cache is empty, as when the directory was made.
    /// Returns how many entries went. Nothing is done once the directory
    /// is not private.
    pub fn clear(&self) -> usize {
        if !dir_is_private(&self.dir) {
            return 0;
        }
        let Ok(read_dir) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let mut removed = 0;
        for e in read_dir.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let entry = name.ends_with(".code");
            if (entry || name.ends_with(".tmp")) && std::fs::remove_file(e.path()).is_ok() {
                removed += usize::from(entry);
            }
        }
        removed
    }

    /// The same store with another total size cap (tests).
    pub fn with_cap(mut self, cap_bytes: u64) -> CodeStore {
        self.cap_bytes = cap_bytes;
        self
    }

    /// The directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The code kept under `key`, read whole and checked against the hash
    /// written with it; reading it marks the entry as just used. A file
    /// that is short, damaged or over the size cap is removed and is a
    /// miss, and so is everything once the directory is not private.
    pub fn get(&self, key: &CodeKey) -> Option<Vec<u8>> {
        if !dir_is_private(&self.dir) {
            return None;
        }
        let path = self.dir.join(&key.0);
        let len = std::fs::metadata(&path).ok()?.len();
        let good = (len <= (HEADER_BYTES + MAX_CODE_BYTES) as u64)
            .then(|| std::fs::read(&path).ok())
            .flatten()
            .and_then(|mut bytes| {
                let rest = bytes.strip_prefix(MAGIC.as_slice())?;
                let (digest, code) = rest.split_at_checked(32)?;
                if code.is_empty() || digest != sha256(code) {
                    return None;
                }
                bytes.drain(..HEADER_BYTES);
                Some(bytes)
            });
        match good {
            Some(code) => {
                // Used now: the last to be evicted.
                if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&path) {
                    let _ = f.set_modified(SystemTime::now());
                }
                Some(code)
            }
            None => {
                let _ = std::fs::remove_file(&path);
                None
            }
        }
    }

    /// Keep `code` under `key` for a module of `module_len` bytes, in
    /// place of whatever was there: a temporary file in the directory,
    /// then a rename, then the entries used longest ago are removed until
    /// the directory is within its cap. Code over the size caps is refused.
    pub fn put(&self, key: &CodeKey, module_len: usize, code: &[u8]) -> Result<(), PutError> {
        if code.is_empty() {
            return Err(PutError::Empty);
        }
        if code.len() > MAX_CODE_BYTES
            || code.len() > module_len.saturating_mul(MAX_CODE_PER_MODULE_BYTE)
        {
            return Err(PutError::TooLarge);
        }
        if !dir_is_private(&self.dir) {
            return Err(PutError::NotPrivate);
        }
        let nanos = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let tmp = self
            .dir
            .join(format!(".{}.{}.{nanos}.tmp", key.0, std::process::id()));
        let result = private_file()
            .open(&tmp)
            .and_then(|mut f| {
                f.write_all(MAGIC)?;
                f.write_all(&sha256(code))?;
                f.write_all(code)
            })
            .and_then(|()| std::fs::rename(&tmp, self.dir.join(&key.0)));
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            return Err(PutError::Io(e.to_string()));
        }
        self.evict();
        Ok(())
    }

    /// Forget the entry under `key`.
    pub fn remove(&self, key: &CodeKey) {
        if dir_is_private(&self.dir) {
            let _ = std::fs::remove_file(self.dir.join(&key.0));
        }
    }

    /// Bytes the entries take in all.
    pub fn size(&self) -> u64 {
        self.entries().iter().map(|(_, len, _)| len).sum()
    }

    /// Every entry as (last used, size, path), and the stale temporary
    /// files of interrupted writes removed on the way.
    fn entries(&self) -> Vec<(SystemTime, u64, PathBuf)> {
        let Ok(read_dir) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        for e in read_dir.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let Ok(meta) = e.metadata() else {
                continue;
            };
            let Ok(modified) = meta.modified() else {
                continue;
            };
            if name.ends_with(".code") {
                entries.push((modified, meta.len(), e.path()));
            } else if name.ends_with(".tmp") && modified.elapsed().is_ok_and(|age| age > STALE_TMP)
            {
                let _ = std::fs::remove_file(e.path());
            }
        }
        entries
    }

    /// Remove the entries used longest ago until the directory holds at
    /// most its cap.
    fn evict(&self) {
        let mut entries = self.entries();
        let mut total: u64 = entries.iter().map(|(_, len, _)| len).sum();
        entries.sort();
        for (_, len, path) in entries {
            if total <= self.cap_bytes {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total -= len;
            }
        }
    }
}

/// The compiled module for verified `wasm`: deserialized from the code
/// the host offered, otherwise compiled now. With [`CompileCache::Host`] a
/// compiled module comes with its serialized form, for the host to store.
pub(crate) fn module(
    engine: &Engine,
    wasm: &[u8],
    cache: CompileCache,
) -> Result<(Module, LoadStats, Option<Vec<u8>>), Error> {
    let started = Instant::now();
    let (hosted, offered) = match cache {
        CompileCache::Off => (false, None),
        CompileCache::Host { code } => (true, code),
    };
    let refused = match offered {
        None => false,
        Some(code) => {
            // SAFETY: the bytes are what the client's main process kept
            // for this very module (its `CodeKey` is made from the module
            // bytes the main process holds, SPEC-9.4.2), and an entry is
            // only ever what a tab process loaded with this module handed
            // over from `Module::serialize`. wasmtime checks its version
            // and settings again. The bytes are still only as good as that
            // earlier tab process: this is the trust boundary the module
            // docs describe, and the reason the caller is a sandboxed tab
            // process (SPEC-9.1.3) and never the main process.
            match unsafe { Module::deserialize(engine, &code) } {
                Ok(module) => {
                    let stats = LoadStats {
                        source: CompileSource::CacheHit,
                        module_time: started.elapsed(),
                        refused: false,
                    };
                    return Ok((module, stats, None));
                }
                Err(_) => true,
            }
        }
    };
    let module = Module::new(engine, wasm).map_err(|e| Error::Instantiate(e.to_string()))?;
    let module_time = started.elapsed();
    let code = hosted.then(|| module.serialize().ok()).flatten();
    let stats = LoadStats {
        source: CompileSource::Compiled,
        module_time,
        refused,
    };
    Ok((module, stats, code))
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
            assert!(CodeStore::open(&dir).is_none(), "no store there");
        } else {
            let root = std::fs::metadata("/").unwrap();
            assert_ne!(root.uid(), me);
            assert_eq!(root.mode() & 0o022, 0);
            assert!(!dir_is_private(Path::new("/")), "owned by root");
            assert!(CodeStore::open(Path::new("/")).is_none(), "no store there");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
