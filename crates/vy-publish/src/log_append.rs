//! `vy-publish log-append`: add a publisher's keys to the site's key
//! transparency log (SPEC-7.6.1), at keygen and at every rotation.
//!
//! The log is append-only: an existing file is read and its chain checked
//! first, and a log whose chain does not hold is never extended (a client
//! refuses it anyway). A key already listed for the host is not added
//! again. The file is replaced atomically.

use std::path::Path;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};
use vy_manifest::{hex, is_date, parse_key_file, KeyLog, LogEntry, PublicKey};

/// `log-append --log <file> --host <host[:port]> --public-key <dev.key.pub> [--published YYYY-MM-DD]`.
pub fn cmd(args: &[String]) -> Result<ExitCode, String> {
    let a = super::Args::parse(args, &["log", "host", "public-key", "published"])?;
    let (Some(log), Some(host), Some(public)) =
        (a.value("log"), a.value("host"), a.value("public-key"))
    else {
        return Ok(super::usage());
    };
    let text = std::fs::read_to_string(public).map_err(|e| format!("{public}: {e}"))?;
    let keys = parse_key_file(&text).map_err(|e| format!("{public}: {e}"))?;
    let published = match a.value("published") {
        Some(d) => d.to_string(),
        None => today(),
    };
    let (log_now, added) = append(Path::new(log), host, &keys, &published)?;
    println!(
        "{log}: {added} entr{} added, {} in all",
        if added == 1 { "y" } else { "ies" },
        log_now.entries.len()
    );
    println!("head:   {}", hex(&log_now.head()));
    println!("serve:  put {log} in the site directory as vy-log; vy-serve serves it at /.well-known/vy-log");
    Ok(ExitCode::SUCCESS)
}

/// Append every key of `keys` that the log at `path` does not yet list for
/// `host`; returns the log as written and how many entries were added.
pub fn append(
    path: &Path,
    host: &str,
    keys: &[PublicKey],
    published: &str,
) -> Result<(KeyLog, usize), String> {
    if !is_date(published) {
        return Err(format!("--published {published:?} is not YYYY-MM-DD"));
    }
    let mut log = match std::fs::read_to_string(path) {
        Ok(text) => KeyLog::parse(&text).map_err(|e| {
            format!(
                "{}: {e}; the log is append-only and this one does not hold, so it is not extended",
                path.display()
            )
        })?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => KeyLog::default(),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let mut added = 0;
    for key in keys {
        if log.lists(host, key) {
            continue;
        }
        let entry = log.append(host, *key, published).clone();
        // The decoder is the one definition of a valid entry (the host's
        // form among it): what it refuses is not written.
        LogEntry::decode(&entry.encode()).map_err(|e| format!("--host {host:?}: {e}"))?;
        added += 1;
    }
    if added > 0 {
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, log.text()).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok((log, added))
}

/// Today's date in UTC as `YYYY-MM-DD` (days to civil date, H. Hinnant).
fn today() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    civil(secs / 86_400)
}

fn civil(days: u64) -> String {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use vy_manifest::SigningKey;

    /// SPEC-7.6.1: `log-append` starts a log, chains each new key to the
    /// head, never lists a key twice for a host, refuses a host the entry
    /// form does not allow, and does not extend a log whose chain is broken.
    #[test]
    fn spec_7_6_1_log_append() {
        let dir = std::env::temp_dir().join(format!("vy-publish-log-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("vy-log");
        let a = SigningKey::from_seed(&[1; 32]).public();
        let b = SigningKey::from_seed(&[2; 32]).public();
        let (log, added) = append(&path, "exempel.se", &[a], "2026-11-05").unwrap();
        assert_eq!((log.entries.len(), added), (1, 1));
        let (log, added) = append(&path, "exempel.se", &[a, b], "2026-12-01").unwrap();
        assert_eq!((log.entries.len(), added), (2, 1), "a is listed already");
        let on_disk = KeyLog::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(on_disk, log);
        assert_eq!(on_disk.entries[1].prev_hash, on_disk.entries[0].hash());
        assert!(on_disk.lists("exempel.se", &b));
        assert_eq!(
            append(&path, "exempel.se", &[b], "2026-12-02").unwrap().1,
            0
        );
        assert!(append(&path, "Exempel.se", &[b], "2026-12-02").is_err());
        assert!(append(&path, "exempel.se", &[b], "2026-13-02").is_err());
        let text = std::fs::read_to_string(&path).unwrap();
        let broken: String = text
            .lines()
            .filter(|l| l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
            + text.lines().nth(2).unwrap();
        std::fs::write(&path, broken).unwrap();
        assert!(append(&path, "exempel.se", &[a], "2026-12-02")
            .unwrap_err()
            .contains("append-only"));
        assert_eq!(civil(0), "1970-01-01");
        assert_eq!(civil(20_397), "2025-11-05");
        assert_eq!(civil(11_016), "2000-02-29");
        assert!(is_date(&today()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
