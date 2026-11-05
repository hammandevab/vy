//! A page project on disk: the crate name out of `Cargo.toml`, the manifest
//! draft, and a fingerprint of everything a build reads, for the watcher.

use crate::scaffold::{valid_name, MANIFEST_FILE};
use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::time::SystemTime;
use vy_manifest::Draft;

/// What [`Project::fingerprint`] returns: each watched path with its
/// modification time and length.
pub type Fingerprint = Vec<(PathBuf, Option<SystemTime>, u64)>;

/// A page project.
#[derive(Clone, Debug)]
pub struct Project {
    /// The project directory, absolute.
    pub dir: PathBuf,
    /// The crate name, which is also the last segment of the page's address.
    pub name: String,
    /// `CARGO_TARGET_DIR` for the build; `None` leaves it to cargo (the
    /// environment, `.cargo/config.toml`, else `<dir>/target`).
    pub target_dir: Option<PathBuf>,
}

impl Project {
    /// Open `dir`: it must hold a `Cargo.toml` whose `[package]` has a
    /// valid page name and a `manifest.json`.
    pub fn open(dir: &Path) -> Result<Self> {
        let dir =
            std::path::absolute(dir).with_context(|| format!("resolving {}", dir.display()))?;
        let cargo = dir.join("Cargo.toml");
        let text = std::fs::read_to_string(&cargo).with_context(|| {
            format!(
                "reading {} (is this a page project? `vy-dev new <dir>` makes one)",
                cargo.display()
            )
        })?;
        let name =
            package_name(&text).ok_or_else(|| anyhow!("{}: no [package] name", cargo.display()))?;
        if !valid_name(&name) {
            bail!(
                "{}: package name {name:?} is not a page name (lowercase letters, digits, - and _, starting with a letter)",
                cargo.display()
            );
        }
        if !dir.join(MANIFEST_FILE).is_file() {
            bail!("{} has no {MANIFEST_FILE}", dir.display());
        }
        Ok(Self {
            dir,
            name,
            target_dir: None,
        })
    }

    /// The manifest draft file.
    pub fn manifest_path(&self) -> PathBuf {
        self.dir.join(MANIFEST_FILE)
    }

    /// Read the draft now (it changes between builds).
    pub fn draft(&self) -> Result<Draft> {
        let path = self.manifest_path();
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        Draft::from_json(&text).map_err(|e| anyhow!("{}: {e}", path.display()))
    }

    /// `vy-dev`'s own scratch directory in the project, under cargo's
    /// ignored `target/`: the packages and the served sites live here.
    pub fn scratch_dir(&self) -> PathBuf {
        self.dir.join("target").join("vy-dev")
    }

    /// The files a build reads: everything under `src/` (and `res/` when
    /// there is one), `Cargo.toml`, the draft and the resources it names.
    /// `Cargo.lock` is left out: cargo writes it during the build, and a
    /// build must not schedule the next one.
    pub fn watched_files(&self) -> Vec<PathBuf> {
        let mut out = vec![self.dir.join("Cargo.toml"), self.manifest_path()];
        walk(&self.dir.join("src"), &mut out);
        walk(&self.dir.join("res"), &mut out);
        if let Ok(draft) = self.draft() {
            for r in &draft.resources {
                out.push(self.dir.join(&r.path));
            }
        }
        out
    }

    /// A value that changes when a watched file changes: each path with
    /// its modification time and length, in path order.
    pub fn fingerprint(&self) -> Fingerprint {
        let mut files = self.watched_files();
        files.sort();
        files.dedup();
        files
            .into_iter()
            .map(|p| {
                let meta = std::fs::metadata(&p).ok();
                let modified = meta.as_ref().and_then(|m| m.modified().ok());
                let len = meta.map(|m| m.len()).unwrap_or(0);
                (p, modified, len)
            })
            .collect()
    }
}

/// Every file under `dir`, recursively, skipping `target` and hidden
/// entries (editor swap files, `.git`).
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" {
            continue;
        }
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// The `name` of the `[package]` table of a `Cargo.toml`, read line by
/// line: the scaffold writes it in that form, and a hand-edited file keeps
/// it.
pub fn package_name(cargo_toml: &str) -> Option<String> {
    let mut in_package = false;
    for raw in cargo_toml.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            if key.trim() == "name" {
                let value = value.trim();
                let value = value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))?;
                return Some(value.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_package_name() {
        assert_eq!(
            package_name(
                "[package]\nname = \"min-sida\" # x\nversion = \"0.1.0\"\n[lib]\nname = \"other\"\n"
            ),
            Some("min-sida".into())
        );
        assert_eq!(
            package_name("[lib]\nname = \"other\"\n[package]\n name='a_b'\n"),
            Some("a_b".into())
        );
        assert_eq!(package_name("[dependencies]\nname = \"x\"\n"), None);
        assert_eq!(package_name(""), None);
    }

    #[test]
    fn opens_a_scaffold_and_fingerprints_it() {
        let dir = std::env::temp_dir().join(format!("vy-dev-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(Project::open(&dir).is_err());
        crate::scaffold::Scaffold {
            dir: dir.clone(),
            name: "min-sida".into(),
            sdk: None,
        }
        .create()
        .unwrap();
        let p = Project::open(&dir).unwrap();
        assert_eq!(p.name, "min-sida");
        assert!(p.dir.is_absolute());
        assert_eq!(p.draft().unwrap().name, "Min sida");
        let a = p.fingerprint();
        assert!(a.iter().any(|(path, _, _)| path.ends_with("src/lib.rs")));
        assert!(a.iter().any(|(path, _, _)| path.ends_with("manifest.json")));
        assert!(!a.iter().any(|(path, _, _)| path.ends_with("Cargo.lock")));
        std::fs::write(dir.join("src/.lib.rs.swp"), "x").unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("Cargo.lock"), "# cargo writes this\n").unwrap();
        assert_eq!(
            a,
            p.fingerprint(),
            "swap files and Cargo.lock are not watched"
        );
        std::fs::write(dir.join("src/extra.rs"), "// new\n").unwrap();
        assert_ne!(a, p.fingerprint());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
