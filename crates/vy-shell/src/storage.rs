//! Page storage on disk, per origin (SPEC-5.2.9, SPEC-9.5.1, SPEC-9.5.2):
//! what a tab's `state_put` saved, handed to the next instance of a page
//! at that origin and to no one else.
//!
//! One file per origin under the data directory, named by the SHA-256 of
//! the origin so no origin can name another's file. The file is the
//! entries in order: `u32 key length, key, u32 value length, value`.
//! A file that does not parse is an empty storage; nothing is guessed.

use crate::ipc::Entry;
use std::path::{Path, PathBuf};

/// Where an origin's storage lives under `dir`.
pub fn storage_path(dir: &Path, origin: &str) -> PathBuf {
    let hash = vy_runtime::sha256(origin.as_bytes());
    dir.join(format!("{}.bin", vy_runtime::hex(&hash)))
}

/// Encode entries to the file form.
pub fn encode(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, v) in entries {
        out.extend_from_slice(&(k.len() as u32).to_le_bytes());
        out.extend_from_slice(k);
        out.extend_from_slice(&(v.len() as u32).to_le_bytes());
        out.extend_from_slice(v);
    }
    out
}

/// Decode the file form; `None` when it is not well formed.
pub fn decode(bytes: &[u8]) -> Option<Vec<Entry>> {
    let mut out = Vec::new();
    let mut pos = 0;
    let take = |pos: &mut usize, n: usize| -> Option<&[u8]> {
        let s = bytes.get(*pos..pos.checked_add(n)?)?;
        *pos += n;
        Some(s)
    };
    let len = |pos: &mut usize| -> Option<usize> {
        let b = take(pos, 4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
    };
    while pos < bytes.len() {
        let kl = len(&mut pos)?;
        let k = take(&mut pos, kl)?.to_vec();
        let vl = len(&mut pos)?;
        let v = take(&mut pos, vl)?.to_vec();
        out.push((k, v));
    }
    Some(out)
}

/// The storage saved for `origin`, or nothing.
pub fn load(dir: &Path, origin: &str) -> Vec<Entry> {
    std::fs::read(storage_path(dir, origin))
        .ok()
        .and_then(|b| decode(&b))
        .unwrap_or_default()
}

/// Save `entries` for `origin`; an empty storage removes the file.
pub fn save(dir: &Path, origin: &str, entries: &[Entry]) -> std::io::Result<()> {
    let path = storage_path(dir, origin);
    if entries.is_empty() {
        match std::fs::remove_file(&path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        }
    }
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, encode(entries))?;
    std::fs::rename(&tmp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-9.5.1, SPEC-9.5.2, SPEC-5.2.9: storage round-trips through
    /// the file, each origin has its own file and reads nothing of
    /// another's, and a damaged file is empty storage.
    #[test]
    fn spec_9_5_2_storage_is_per_origin_on_disk() {
        let dir = std::env::temp_dir().join(format!("vy-storage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = vec![
            (b"theme".to_vec(), b"dark".to_vec()),
            (vec![], vec![0, 1, 2]),
        ];
        save(&dir, "vy://a.example", &a).unwrap();
        save(&dir, "vy://b.example", &[(b"x".to_vec(), b"y".to_vec())]).unwrap();
        assert_eq!(load(&dir, "vy://a.example"), a);
        assert_eq!(
            load(&dir, "vy://b.example"),
            vec![(b"x".to_vec(), b"y".to_vec())]
        );
        assert_eq!(load(&dir, "vy://c.example"), vec![]);
        assert_ne!(
            storage_path(&dir, "vy://a.example"),
            storage_path(&dir, "vy://a.example:8443"),
            "the port is part of the origin"
        );
        assert!(storage_path(&dir, "vy://../x")
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .ends_with(".bin"));
        std::fs::write(storage_path(&dir, "vy://a.example"), [9, 9, 9]).unwrap();
        assert_eq!(load(&dir, "vy://a.example"), vec![], "damaged is empty");
        save(&dir, "vy://b.example", &[]).unwrap();
        assert!(!storage_path(&dir, "vy://b.example").exists());
        assert_eq!(decode(&encode(&a)), Some(a));
        assert_eq!(decode(&[1, 0, 0, 0]), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
