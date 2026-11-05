//! Publisher keys per origin (SPEC-7.4.2, SPEC-7.4.4): trust on first use.
//!
//! On the first visit to an origin the keys published at
//! `/.well-known/vy-key` are pinned; later manifests must be signed by a
//! pinned key, or by a key the previous manifest announced as its
//! successor in `next_key_id`, which is then fetched and pinned in turn.
//! The DNS TXT half of SPEC-7.4.2 is not consulted yet (M4).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use vy_manifest::PublicKey;

/// Pins and announced successors of one origin.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OriginKeys {
    /// key id -> key.
    pub pinned: BTreeMap<String, PublicKey>,
    /// Key ids a trusted manifest named in `next_key_id` (SPEC-7.4.4).
    pub successors: BTreeSet<String>,
}

/// The key store: a text file `keys` with one line per pin or successor.
#[derive(Debug)]
pub struct KeyStore {
    path: PathBuf,
    origins: BTreeMap<String, OriginKeys>,
}

impl KeyStore {
    /// Open or create `dir/keys`.
    pub fn open(dir: &Path) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join("keys");
        let mut store = KeyStore {
            path,
            origins: BTreeMap::new(),
        };
        if let Ok(text) = fs::read_to_string(&store.path) {
            for line in text.lines() {
                let parts: Vec<&str> = line.split(' ').collect();
                match parts.as_slice() {
                    ["pin", origin, hex] => {
                        if let Ok(key) = hex.parse::<PublicKey>() {
                            store.entry(origin).pinned.insert(key.key_id(), key);
                        }
                    }
                    ["successor", origin, key_id] => {
                        store.entry(origin).successors.insert((*key_id).to_string());
                    }
                    _ => {}
                }
            }
        }
        Ok(store)
    }

    fn entry(&mut self, origin: &str) -> &mut OriginKeys {
        self.origins.entry(origin.to_string()).or_default()
    }

    fn save(&self) -> io::Result<()> {
        let mut text = String::new();
        for (origin, keys) in &self.origins {
            for key in keys.pinned.values() {
                text.push_str(&format!("pin {origin} {key}\n"));
            }
            for id in &keys.successors {
                text.push_str(&format!("successor {origin} {id}\n"));
            }
        }
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, &self.path)
    }

    /// Everything known about an origin.
    pub fn origin(&self, origin: &str) -> Option<&OriginKeys> {
        self.origins.get(origin)
    }

    /// True once any key is pinned for the origin (not a first visit).
    pub fn has_pins(&self, origin: &str) -> bool {
        self.origins
            .get(origin)
            .map(|o| !o.pinned.is_empty())
            .unwrap_or(false)
    }

    /// The pinned key with this id, if any.
    pub fn pinned(&self, origin: &str, key_id: &str) -> Option<PublicKey> {
        self.origins
            .get(origin)
            .and_then(|o| o.pinned.get(key_id))
            .copied()
    }

    /// Pin a key for the origin.
    pub fn pin(&mut self, origin: &str, key: PublicKey) -> io::Result<()> {
        self.entry(origin).pinned.insert(key.key_id(), key);
        self.save()
    }

    /// Record that a trusted manifest announced `key_id` as the next key.
    pub fn announce_successor(&mut self, origin: &str, key_id: &str) -> io::Result<()> {
        let o = self.entry(origin);
        if o.pinned.contains_key(key_id) || !o.successors.insert(key_id.to_string()) {
            return Ok(());
        }
        self.save()
    }

    /// True when `key_id` was announced as a successor and is not pinned yet.
    pub fn is_successor(&self, origin: &str, key_id: &str) -> bool {
        self.origins
            .get(origin)
            .map(|o| o.successors.contains(key_id))
            .unwrap_or(false)
    }

    /// Pin an announced successor once its key is known; the announcement
    /// is consumed.
    pub fn promote(&mut self, origin: &str, key: PublicKey) -> io::Result<()> {
        let o = self.entry(origin);
        o.successors.remove(&key.key_id());
        o.pinned.insert(key.key_id(), key);
        self.save()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vy_manifest::SigningKey;

    #[test]
    fn pins_and_successors_persist() {
        let dir = std::env::temp_dir().join(format!("vy-proto-keys-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let a = SigningKey::from_seed(&[1u8; 32]).public();
        let b = SigningKey::from_seed(&[2u8; 32]).public();
        {
            let mut s = KeyStore::open(&dir).unwrap();
            assert!(!s.has_pins("vy://x"));
            s.pin("vy://x", a).unwrap();
            s.announce_successor("vy://x", &b.key_id()).unwrap();
            assert!(s.is_successor("vy://x", &b.key_id()));
            assert_eq!(s.pinned("vy://x", &a.key_id()), Some(a));
            assert_eq!(s.pinned("vy://x", &b.key_id()), None);
        }
        let mut s = KeyStore::open(&dir).unwrap();
        assert!(s.has_pins("vy://x"));
        assert!(s.is_successor("vy://x", &b.key_id()));
        s.promote("vy://x", b).unwrap();
        assert!(!s.is_successor("vy://x", &b.key_id()));
        assert_eq!(s.pinned("vy://x", &b.key_id()), Some(b));
        assert!(!s.has_pins("vy://y"));
        let _ = fs::remove_dir_all(&dir);
    }
}
