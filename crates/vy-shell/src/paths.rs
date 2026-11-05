//! Where the client keeps its files: the grant store, page storage, the
//! caches and the key store of `vy-proto`.
//!
//! One directory, `vy`, under the platform's configuration directory:
//! `%APPDATA%\vy` on Windows, `~/Library/Application Support/vy` on
//! macOS, `$XDG_CONFIG_HOME/vy` or `~/.config/vy` elsewhere. Read from
//! the environment, not a crate: it is three variables.

use std::path::PathBuf;

/// The client's directory, or `None` when the environment names no home.
pub fn data_dir() -> Option<PathBuf> {
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    if let Some(dir) = env("VY_DATA_DIR") {
        return Some(PathBuf::from(dir));
    }
    let base = if cfg!(target_os = "windows") {
        env("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        env("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("vy"))
}

/// A directory of its own under the temporary directory, for a run that
/// must not touch the user's files (screenshots, tests).
pub fn scratch_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("vy-{tag}-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `VY_DATA_DIR` wins; the platform default ends in `vy`.
    #[test]
    fn data_dir_follows_the_environment() {
        let dir = data_dir();
        if std::env::var_os("VY_DATA_DIR").is_none() {
            assert!(dir.is_none_or(|d| d.ends_with("vy")));
        }
        assert!(scratch_dir("x").to_string_lossy().contains("vy-x-"));
    }
}
