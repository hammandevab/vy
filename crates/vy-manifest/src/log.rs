//! The key transparency log (SPEC §7.6): an append-only, mirrorable list
//! of the keys a publisher has used, one entry per line.
//!
//! Each line is the canonical CBOR encoding (SPEC-4.1.3) of one entry, in
//! lower-case hex: `{host, key_id, key, published, prev_hash}`, where
//! `prev_hash` is SHA-256 of the previous entry's encoding (32 zero bytes
//! for the first). The log's head is SHA-256 of the last entry's encoding;
//! a client that remembers a head can tell an extended log (the head is
//! still an entry's hash) from a rewritten one (it is not). Anyone can copy
//! the file and check the chain without trusting the server.

use crate::cbor::{self, Enc, MapWriter, Reader};
use crate::manifest::hex;
use crate::sign::PublicKey;
use crate::validate::{is_date, is_key_id};
use crate::Error;
use sha2::{Digest, Sha256};

/// Largest log a client reads (`/.well-known/vy-log`): about 3 000 entries.
pub const MAX_LOG_BYTES: usize = 1 << 20;
/// Longest `host` in an entry (a DNS name and a port).
pub const MAX_LOG_HOST_BYTES: usize = 260;
/// `prev_hash` of the first entry.
pub const LOG_GENESIS: [u8; 32] = [0; 32];

/// One log entry (SPEC-7.6.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    /// The origin's authority as in its `vy://` address: lower-case host,
    /// `:port` when it is not 443.
    pub host: String,
    /// The key's id, which must be the key's derived id (SPEC-8.1.1).
    pub key_id: String,
    /// The publisher key.
    pub key: PublicKey,
    /// `YYYY-MM-DD`, the day the key was put in the log.
    pub published: String,
    /// SHA-256 of the previous entry's encoding.
    pub prev_hash: [u8; 32],
}

impl LogEntry {
    /// The entry for `key` at `host`, chained after `prev_hash`.
    pub fn new(host: &str, key: PublicKey, published: &str, prev_hash: [u8; 32]) -> Self {
        Self {
            host: host.to_string(),
            key_id: key.key_id(),
            key,
            published: published.to_string(),
            prev_hash,
        }
    }

    /// The canonical CBOR encoding.
    pub fn encode(&self) -> Vec<u8> {
        let mut m = MapWriter::new();
        m.field("host", |e| cbor::text(e, &self.host));
        m.field("key_id", |e| cbor::text(e, &self.key_id));
        m.field("key", |e| cbor::bytes(e, &self.key.to_bytes()));
        m.field("published", |e| cbor::text(e, &self.published));
        m.field("prev_hash", |e| cbor::bytes(e, &self.prev_hash));
        let mut enc = Enc::new(Vec::new());
        m.finish(&mut enc);
        enc.into_writer()
    }

    /// SHA-256 of [`LogEntry::encode`]: what the next entry chains to.
    pub fn hash(&self) -> [u8; 32] {
        Sha256::digest(self.encode()).into()
    }

    /// Decode one entry; only the canonical encoding of a valid entry is
    /// accepted (every field present, no other, the id derived from the key).
    pub fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(bytes);
        let (mut host, mut key_id, mut key, mut published, mut prev) =
            (None, None, None, None, None);
        r.map_fields(|r, k| {
            match k {
                "host" => host = Some(r.text()?.to_string()),
                "key_id" => key_id = Some(r.text()?.to_string()),
                "key" => key = Some(PublicKey::from_bytes(&r.bytes_exact::<32>("key")?)?),
                "published" => published = Some(r.text()?.to_string()),
                "prev_hash" => prev = Some(r.bytes_exact::<32>("prev_hash")?),
                other => return Err(Error::UnknownField(other.to_string())),
            }
            Ok(())
        })?;
        r.finish()?;
        let entry = LogEntry {
            host: host.ok_or(Error::MissingField("host"))?,
            key_id: key_id.ok_or(Error::MissingField("key_id"))?,
            key: key.ok_or(Error::MissingField("key"))?,
            published: published.ok_or(Error::MissingField("published"))?,
            prev_hash: prev.ok_or(Error::MissingField("prev_hash"))?,
        };
        let field = |field, reason: &str| Error::Field {
            field,
            reason: reason.to_string(),
        };
        if !is_log_host(&entry.host) {
            return Err(field("host", "not a lower-case host[:port]"));
        }
        if !is_key_id(&entry.key_id) || entry.key_id != entry.key.key_id() {
            return Err(field("key_id", "not the key's id"));
        }
        if !is_date(&entry.published) {
            return Err(field("published", "not YYYY-MM-DD"));
        }
        Ok(entry)
    }

    /// The entry as one line of the log file (hex, no newline).
    pub fn line(&self) -> String {
        hex(&self.encode())
    }
}

/// A host as entries carry it: lower-case letters, digits, `.`, `-`, an
/// optional `:port`, or a bracketed IPv6 literal; nothing that could break
/// a line-oriented store.
fn is_log_host(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= MAX_LOG_HOST_BYTES
        && h.bytes().all(|b| {
            b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b'.' | b'-' | b':' | b'[' | b']')
        })
}

/// A parsed, chain-checked log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyLog {
    /// The entries in order.
    pub entries: Vec<LogEntry>,
}

impl KeyLog {
    /// Parse the text form and check the chain (SPEC-7.6.1): every line an
    /// entry, the first chained to [`LOG_GENESIS`], each next one to the
    /// hash of the one before. Blank lines and `#` comments are skipped.
    pub fn parse(text: &str) -> Result<Self, Error> {
        if text.len() > MAX_LOG_BYTES {
            return Err(Error::TooLarge { size: text.len() });
        }
        let mut entries: Vec<LogEntry> = Vec::new();
        let mut prev = LOG_GENESIS;
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bytes = hex_bytes(line).ok_or_else(|| Error::Field {
                field: "log",
                reason: format!("line {} is not lower-case hex", n + 1),
            })?;
            let entry = LogEntry::decode(&bytes)?;
            if entry.prev_hash != prev {
                return Err(Error::Field {
                    field: "prev_hash",
                    reason: format!("line {} does not chain to the entry before", n + 1),
                });
            }
            prev = Sha256::digest(&bytes).into();
            entries.push(entry);
        }
        Ok(Self { entries })
    }

    /// The text form: a comment line and one line per entry.
    pub fn text(&self) -> String {
        let mut out = String::from(
            "# vy-log: key transparency log, one canonical CBOR entry per line (SPEC-7.6.1)\n",
        );
        for e in &self.entries {
            out.push_str(&e.line());
            out.push('\n');
        }
        out
    }

    /// The head: hash of the last entry, [`LOG_GENESIS`] when empty.
    pub fn head(&self) -> [u8; 32] {
        self.entries.last().map_or(LOG_GENESIS, LogEntry::hash)
    }

    /// Append `key` for `host`; the new entry chains to the current head.
    pub fn append(&mut self, host: &str, key: PublicKey, published: &str) -> &LogEntry {
        let entry = LogEntry::new(host, key, published, self.head());
        self.entries.push(entry);
        self.entries.last().expect("just pushed")
    }

    /// True when the log lists `key` for `host`.
    pub fn lists(&self, host: &str, key: &PublicKey) -> bool {
        self.entries.iter().any(|e| e.host == host && e.key == *key)
    }

    /// True when `head` (after `count` entries) is a prefix of this log:
    /// the log has only grown since that head was seen.
    pub fn extends(&self, count: usize, head: &[u8; 32]) -> bool {
        match count {
            0 => *head == LOG_GENESIS,
            n => self.entries.get(n - 1).is_some_and(|e| e.hash() == *head),
        }
    }
}

/// Lower-case hex to bytes, on bytes: a line of any text, multi-byte
/// characters included, is refused rather than sliced.
fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if !b.len().is_multiple_of(2) {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    b.chunks(2)
        .map(|p| Some(digit(p[0])? << 4 | digit(p[1])?))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SigningKey;

    fn key(seed: u8) -> PublicKey {
        SigningKey::from_seed(&[seed; 32]).public()
    }

    /// SPEC-7.6.1: one canonical CBOR entry per line in lower-case hex,
    /// `{host, key_id, key, published, prev_hash}` in canonical key order,
    /// each chained to the hash of the one before (zeros for the first),
    /// the head the hash of the last; the text form round-trips and every
    /// tampering (a changed byte, a dropped, swapped or reordered entry,
    /// another encoding of the same entry, a forged id) is refused.
    #[test]
    fn spec_7_6_1_log_format_and_chain() {
        let mut log = KeyLog::default();
        assert_eq!(log.head(), LOG_GENESIS);
        log.append("exempel.se", key(1), "2026-11-05");
        log.append("localhost:8080", key(2), "2026-11-05");
        log.append("exempel.se", key(3), "2026-12-01");
        assert_eq!(log.entries[0].prev_hash, LOG_GENESIS);
        assert_eq!(log.entries[1].prev_hash, log.entries[0].hash());
        assert_eq!(log.entries[2].prev_hash, log.entries[1].hash());
        assert_eq!(log.head(), log.entries[2].hash());
        assert_eq!(log.entries[2].key_id, key(3).key_id());

        // The encoding: a map of five, keys shortest first, then bytewise.
        let bytes = log.entries[0].encode();
        assert_eq!(bytes[0], 0xA5);
        let mut r = Reader::new(&bytes);
        let mut order = Vec::new();
        r.map_fields(|r, k| {
            order.push(k.to_string());
            match k {
                "key" | "prev_hash" => r.bytes().map(|_| ()),
                _ => r.text().map(|_| ()),
            }
        })
        .unwrap();
        assert_eq!(order, ["key", "host", "key_id", "prev_hash", "published"]);
        assert_eq!(LogEntry::decode(&bytes).unwrap(), log.entries[0]);

        let text = log.text();
        assert!(text.starts_with('#'));
        assert_eq!(KeyLog::parse(&text).unwrap(), log);
        let with_blanks = text.replace('\n', "\n\n# a comment\n");
        assert_eq!(KeyLog::parse(&with_blanks).unwrap(), log);
        assert!(log.lists("exempel.se", &key(3)));
        assert!(!log.lists("exempel.se", &key(2)), "listed for another host");

        let lines: Vec<&str> = text.lines().skip(1).collect();
        let join = |l: &[&str]| l.join("\n");
        // A changed byte, a dropped first entry, two swapped entries.
        let mut flipped = lines[1].to_string().into_bytes();
        let i = flipped.len() / 2;
        flipped[i] = if flipped[i] == b'0' { b'1' } else { b'0' };
        let flipped = String::from_utf8(flipped).unwrap();
        for bad in [
            join(&[lines[0], &flipped, lines[2]]),
            join(&lines[1..]),
            join(&[lines[1], lines[0], lines[2]]),
            join(&[lines[0], lines[2]]),
            lines[0].to_ascii_uppercase(),
            format!("{}0", lines[0]),
            "zz".into(),
        ] {
            assert!(KeyLog::parse(&bad).is_err(), "{bad}");
        }
        assert!(matches!(
            KeyLog::parse(&"#".repeat(MAX_LOG_BYTES + 1)),
            Err(Error::TooLarge { .. })
        ));
        // Found by the fuzz replay: a multi-byte character inside a line
        // was sliced through and panicked; it is refused.
        assert!(KeyLog::parse("a\u{e9}b").is_err());
        assert!(KeyLog::parse(&format!("{}\u{e9}", &lines[0][1..])).is_err());

        // Another encoding of the same entry, an unknown field, a forged
        // id, a host that could break a line-oriented store.
        let e = &log.entries[0];
        let mut unordered = Enc::new(Vec::new());
        unordered.map(5).unwrap();
        for (k, v) in [("host", &e.host), ("key_id", &e.key_id)] {
            cbor::text(&mut unordered, k);
            cbor::text(&mut unordered, v);
        }
        cbor::text(&mut unordered, "key");
        cbor::bytes(&mut unordered, &e.key.to_bytes());
        cbor::text(&mut unordered, "prev_hash");
        cbor::bytes(&mut unordered, &e.prev_hash);
        cbor::text(&mut unordered, "published");
        cbor::text(&mut unordered, &e.published);
        assert!(matches!(
            LogEntry::decode(&unordered.into_writer()),
            Err(Error::NonCanonical { .. })
        ));
        let encode = |f: &dyn Fn(&mut MapWriter)| {
            let mut m = MapWriter::new();
            f(&mut m);
            let mut enc = Enc::new(Vec::new());
            m.finish(&mut enc);
            enc.into_writer()
        };
        let base = |m: &mut MapWriter, host: &str, id: &str| {
            m.field("host", |x| cbor::text(x, host));
            m.field("key_id", |x| cbor::text(x, id));
            m.field("key", |x| cbor::bytes(x, &e.key.to_bytes()));
            m.field("published", |x| cbor::text(x, &e.published));
            m.field("prev_hash", |x| cbor::bytes(x, &e.prev_hash));
        };
        let extra = encode(&|m| {
            base(m, &e.host, &e.key_id);
            m.field("note", |x| cbor::text(x, "x"));
        });
        assert!(matches!(
            LogEntry::decode(&extra),
            Err(Error::UnknownField(_))
        ));
        let forged = encode(&|m| base(m, &e.host, &key(9).key_id()));
        assert!(LogEntry::decode(&forged).is_err());
        for host in ["Exempel.se", "a b", "x\npin vy://bank.example", ""] {
            assert!(LogEntry::decode(&encode(&|m| base(m, host, &e.key_id))).is_err());
        }
        assert!(LogEntry::decode(&encode(&|m| base(m, "exempel.se", &e.key_id))).is_ok());

        // Append-only: the grown log still holds the old head at its old
        // length; a rewritten one does not.
        let (count, head) = (log.entries.len(), log.head());
        let mut grown = log.clone();
        grown.append("exempel.se", key(4), "2027-01-01");
        assert!(grown.extends(count, &head));
        assert!(grown.extends(0, &LOG_GENESIS));
        assert!(!grown.extends(count + 2, &head));
        let mut rewritten = KeyLog::default();
        rewritten.append("exempel.se", key(5), "2026-11-05");
        for e in &log.entries[1..] {
            rewritten.append(&e.host, e.key, &e.published);
        }
        assert!(!rewritten.extends(count, &head));
    }
}
