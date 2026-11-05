//! Where `vy-dev` keeps its files: the developer key in the client's own
//! directory.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use vy_manifest::SigningKey;

/// File name of the developer key under [`data_dir`].
pub const KEY_FILE: &str = "dev.key";
/// Environment variable naming the key `vy-dev build` signs with; `serve`
/// sets it for the agent, so the agent's `vy-dev build` signs with the
/// daemon's key.
pub const ENV_KEY: &str = "VY_DEV_KEY";

/// The client's directory, the one `vy-shell` uses (`vy_shell::paths`):
/// `VY_DATA_DIR`, else `%APPDATA%\vy` on Windows,
/// `~/Library/Application Support/vy` on macOS, `$XDG_CONFIG_HOME/vy` or
/// `~/.config/vy` elsewhere. `None` when the environment names no home.
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

/// The key to sign with: `explicit` (`--key`), else `VY_DEV_KEY`, else
/// `<data_dir>/dev.key`.
pub fn key_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        return Ok(p.to_path_buf());
    }
    if let Some(p) = std::env::var_os(ENV_KEY).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    data_dir().map(|d| d.join(KEY_FILE)).ok_or_else(|| {
        anyhow!("no home directory in the environment: pass --key or set VY_DATA_DIR")
    })
}

/// Read the key at `path`, or make one there (with its `.pub` next to it,
/// `vy_publish::write_key`) when the file does not exist. Returns the key
/// and whether it was made. A key without its `.pub` gets one.
pub fn load_or_create_key(path: &Path) -> Result<(SigningKey, bool)> {
    let (key, created) = if path.is_file() {
        (
            vy_publish::read_key(path).map_err(|e| anyhow!("{e}"))?,
            false,
        )
    } else {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let key = vy_publish::keygen().map_err(|e| anyhow!("{e}"))?;
        vy_publish::write_key(path, &key).map_err(|e| anyhow!("{e}"))?;
        (key, true)
    };
    let pub_path = vy_publish::public_key_path(path);
    if !pub_path.is_file() {
        std::fs::write(&pub_path, vy_manifest::key_file(&[key.public()]))
            .with_context(|| format!("writing {}", pub_path.display()))?;
    }
    Ok((key, created))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_made_once() {
        let dir = std::env::temp_dir().join(format!("vy-dev-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sub").join(KEY_FILE);
        let (a, created) = load_or_create_key(&path).unwrap();
        assert!(created);
        let pub_path = vy_publish::public_key_path(&path);
        assert_eq!(
            vy_manifest::parse_key_file(&std::fs::read_to_string(&pub_path).unwrap()).unwrap(),
            vec![a.public()]
        );
        std::fs::remove_file(&pub_path).unwrap();
        let (b, created) = load_or_create_key(&path).unwrap();
        assert!(!created);
        assert_eq!(a.seed(), b.seed());
        assert!(pub_path.is_file(), "a missing .pub is written again");
        assert_eq!(key_path(Some(&path)).unwrap(), path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn data_dir_ends_in_vy() {
        if std::env::var_os("VY_DATA_DIR").is_none() {
            if let Some(d) = data_dir() {
                assert!(d.ends_with("vy"), "{}", d.display());
            }
        }
    }
}
