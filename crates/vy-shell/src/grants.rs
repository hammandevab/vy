//! The capability store (SPEC-8.2.3, SPEC-9.2.1, SPEC-9.5.1): what the
//! user decided per origin and capability, kept across runs.
//!
//! A text file, one decision per line: `origin TAB capability TAB allow|deny`.
//! Only names 1.0 defines are kept (SPEC-8.2.1); a line that does not
//! parse is dropped on load, never guessed at.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use vy_manifest::Grant;

/// The user's standing decisions.
#[derive(Debug, Default)]
pub struct GrantStore {
    path: Option<PathBuf>,
    decisions: BTreeMap<(String, String), bool>,
}

impl GrantStore {
    /// A store that is not persisted.
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// Open (or start) the store at `path`.
    pub fn open(path: &Path) -> Self {
        let mut store = Self {
            path: Some(path.to_path_buf()),
            decisions: BTreeMap::new(),
        };
        if let Ok(text) = std::fs::read_to_string(path) {
            store.load(&text);
        }
        store
    }

    fn load(&mut self, text: &str) {
        for line in text.lines() {
            let mut parts = line.split('\t');
            let (Some(origin), Some(cap), Some(decision)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            if parts.next().is_some() || vy_abi::cap::id(cap).is_none() {
                continue;
            }
            let allow = match decision {
                "allow" => true,
                "deny" => false,
                _ => continue,
            };
            self.decisions
                .insert((origin.to_string(), cap.to_string()), allow);
        }
    }

    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        let mut text = String::new();
        for ((origin, cap), allow) in &self.decisions {
            text.push_str(origin);
            text.push('\t');
            text.push_str(cap);
            text.push('\t');
            text.push_str(if *allow { "allow" } else { "deny" });
            text.push('\n');
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, text);
    }

    /// The user's decision for `cap` at `origin`, if any.
    pub fn decision(&self, origin: &str, cap: &str) -> Option<bool> {
        self.decisions
            .get(&(origin.to_string(), cap.to_string()))
            .copied()
    }

    /// Record a decision.
    pub fn set(&mut self, origin: &str, cap: &str, allow: bool) {
        if vy_abi::cap::id(cap).is_none() {
            return;
        }
        self.decisions
            .insert((origin.to_string(), cap.to_string()), allow);
        self.save();
    }

    /// Forget a decision, so the default policy applies again.
    pub fn clear(&mut self, origin: &str, cap: &str) {
        self.decisions
            .remove(&(origin.to_string(), cap.to_string()));
        self.save();
    }

    /// What a page gets now, given the default policy's verdicts for its
    /// requests ([`vy_manifest::CapabilityRequests::default_grants`]) and
    /// the user's standing decisions: `(name, granted, ask)` per request.
    /// `Auto` is granted unless the user denied it; `Ask` is granted only
    /// if the user allowed it, and `ask` is true while they have not
    /// answered; `Denied` is granted only by an explicit allow (the
    /// policy's "nekas som standard" means the user turns it on).
    pub fn resolve(&self, origin: &str, verdicts: &[(&'static str, Grant)]) -> Vec<Resolved> {
        verdicts
            .iter()
            .map(|(name, grant)| {
                let user = self.decision(origin, name);
                let (granted, ask) = match (grant, user) {
                    (_, Some(allow)) => (allow, false),
                    (Grant::Auto, None) => (true, false),
                    (Grant::Ask, None) => (false, true),
                    (Grant::Denied, None) => (false, false),
                };
                Resolved { name, granted, ask }
            })
            .collect()
    }
}

/// One requested capability as the shell will treat it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// Canonical name.
    pub name: &'static str,
    /// Granted at load.
    pub granted: bool,
    /// The user must be asked (a dialog), after which `granted` may change.
    pub ask: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-8.2.3, SPEC-9.2.1, SPEC-9.5.1: the default policy applies
    /// until the user decides; decisions are per origin and capability,
    /// survive a reopen, and revoking one makes the policy apply again;
    /// names outside 1.0 are never stored.
    #[test]
    fn spec_9_2_1_grants_persist_per_origin() {
        let dir = std::env::temp_dir().join(format!("vy-grants-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("grants.txt");
        let verdicts = [
            ("net", Grant::Auto),
            ("clipboard", Grant::Ask),
            ("hires_time", Grant::Denied),
        ];
        let mut store = GrantStore::open(&path);
        let r = store.resolve("vy://a.example", &verdicts);
        assert_eq!(
            r,
            vec![
                Resolved {
                    name: "net",
                    granted: true,
                    ask: false
                },
                Resolved {
                    name: "clipboard",
                    granted: false,
                    ask: true
                },
                Resolved {
                    name: "hires_time",
                    granted: false,
                    ask: false
                },
            ]
        );
        store.set("vy://a.example", "clipboard", true);
        store.set("vy://a.example", "net", false);
        store.set("vy://b.example", "hires_time", true);
        store.set("vy://a.example", "teleport", true);
        let r = store.resolve("vy://a.example", &verdicts);
        assert_eq!(
            r.iter()
                .map(|x| (x.name, x.granted, x.ask))
                .collect::<Vec<_>>(),
            vec![
                ("net", false, false),
                ("clipboard", true, false),
                ("hires_time", false, false)
            ]
        );
        let r = store.resolve("vy://b.example", &verdicts);
        assert_eq!(
            r.iter()
                .map(|x| (x.name, x.granted, x.ask))
                .collect::<Vec<_>>(),
            vec![
                ("net", true, false),
                ("clipboard", false, true),
                ("hires_time", true, false)
            ],
            "another origin has its own decisions"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("teleport"));
        assert_eq!(text.lines().count(), 3);

        let mut again = GrantStore::open(&path);
        assert_eq!(again.decision("vy://a.example", "clipboard"), Some(true));
        assert_eq!(again.decision("vy://a.example", "net"), Some(false));
        again.clear("vy://a.example", "net");
        assert_eq!(again.decision("vy://a.example", "net"), None);
        assert!(again.resolve("vy://a.example", &verdicts)[0].granted);
        let third = GrantStore::open(&path);
        assert_eq!(
            third.decision("vy://a.example", "net"),
            None,
            "cleared on disk too"
        );
        let _ = std::fs::remove_dir_all(&dir);

        let mut mem = GrantStore::in_memory();
        mem.set("vy://x", "ime", false);
        assert_eq!(mem.decision("vy://x", "ime"), Some(false));
        // A corrupt line is skipped, the rest kept.
        let mut loaded = GrantStore::in_memory();
        loaded.load(
            "vy://x\tnet\tallow\ngarbage\nvy://x\tnet\tmaybe\nvy://y\tstorage\tdeny\textra\n",
        );
        assert_eq!(loaded.decisions.len(), 1);
    }
}
