//! The bounded event log the dev API serves: what the agent said and did,
//! what the builds did, and the raw stream lines for the devtools.

use serde_json::{json, Value};
use std::collections::VecDeque;

/// Entries kept; the oldest go first.
pub const DEFAULT_CAP: usize = 4000;
/// Longest text of one entry; longer is cut (on a character boundary).
pub const MAX_ENTRY_BYTES: usize = 8 * 1024;

/// What an entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The agent started, exited or was stopped.
    Agent,
    /// Text the assistant wrote.
    Assistant,
    /// A tool the assistant used (name and what it touched).
    Tool,
    /// The run's final result.
    Result,
    /// A build started or finished; `status` says which.
    Build,
    /// A raw line of the agent's stream, as it came.
    Raw,
}

impl Kind {
    /// The name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Agent => "agent",
            Kind::Assistant => "assistant",
            Kind::Tool => "tool",
            Kind::Result => "result",
            Kind::Build => "build",
            Kind::Raw => "raw",
        }
    }
}

/// One entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Sequence number, from 1, never reused.
    pub seq: u64,
    /// What it is.
    pub kind: Kind,
    /// The text.
    pub text: String,
    /// For `Build`: `building`, `ok` or `error`.
    pub status: Option<&'static str>,
}

impl Entry {
    /// The JSON object on the wire.
    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "seq": self.seq,
            "kind": self.kind.name(),
            "text": self.text,
        });
        if let Some(s) = self.status {
            v["status"] = Value::from(s);
        }
        v
    }
}

/// The log: a ring of the last `cap` entries with a running sequence.
#[derive(Debug)]
pub struct Log {
    entries: VecDeque<Entry>,
    next_seq: u64,
    cap: usize,
}

impl Default for Log {
    fn default() -> Self {
        Self::new(DEFAULT_CAP)
    }
}

impl Log {
    /// An empty log keeping at most `cap` entries.
    pub fn new(cap: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            next_seq: 1,
            cap: cap.max(1),
        }
    }

    /// Append an entry; returns its sequence number.
    pub fn push(&mut self, kind: Kind, text: impl Into<String>) -> u64 {
        self.push_with(kind, text, None)
    }

    /// Append a build entry with its status.
    pub fn push_build(&mut self, status: &'static str, text: impl Into<String>) -> u64 {
        self.push_with(Kind::Build, text, Some(status))
    }

    fn push_with(
        &mut self,
        kind: Kind,
        text: impl Into<String>,
        status: Option<&'static str>,
    ) -> u64 {
        let mut text = text.into();
        if text.len() > MAX_ENTRY_BYTES {
            let mut cut = MAX_ENTRY_BYTES;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
            text.push_str(" [...]");
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        if self.entries.len() == self.cap {
            self.entries.pop_front();
        }
        self.entries.push_back(Entry {
            seq,
            kind,
            text,
            status,
        });
        seq
    }

    /// The last sequence number handed out, 0 when empty.
    pub fn last_seq(&self) -> u64 {
        self.next_seq - 1
    }

    /// Entries with a sequence number above `after`, oldest first.
    pub fn after(&self, after: u64) -> Vec<Entry> {
        self.select(after, true, usize::MAX)
    }

    /// Entries above `after`, without [`Kind::Raw`] unless `raw`, at most
    /// the `limit` newest of them, oldest first.
    pub fn select(&self, after: u64, raw: bool, limit: usize) -> Vec<Entry> {
        let start = self.entries.partition_point(|e| e.seq <= after);
        let mut out: Vec<Entry> = self
            .entries
            .range(start..)
            .rev()
            .filter(|e| raw || e.kind != Kind::Raw)
            .take(limit)
            .cloned()
            .collect();
        out.reverse();
        out
    }

    /// Entries kept.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing was ever pushed or everything was dropped.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_and_cap() {
        let mut log = Log::new(3);
        assert_eq!(log.last_seq(), 0);
        assert_eq!(log.push(Kind::Agent, "a"), 1);
        assert_eq!(log.push(Kind::Assistant, "b"), 2);
        assert_eq!(log.push_build("ok", "c"), 3);
        assert_eq!(log.push(Kind::Raw, "d"), 4);
        assert_eq!(log.len(), 3, "the oldest was dropped");
        assert_eq!(
            log.after(0).iter().map(|e| e.seq).collect::<Vec<_>>(),
            [2, 3, 4]
        );
        assert_eq!(log.after(3).len(), 1);
        assert!(log.after(4).is_empty());
        assert_eq!(log.after(2)[0].to_json()["status"], "ok");
        assert_eq!(log.after(3)[0].to_json()["kind"], "raw");
        assert!(log.after(3)[0].to_json().get("status").is_none());
        let long = "ä".repeat(MAX_ENTRY_BYTES);
        log.push(Kind::Raw, long);
        let last = log.after(4).pop().unwrap();
        assert!(last.text.ends_with(" [...]"));
        assert!(last.text.len() <= MAX_ENTRY_BYTES + 6);
    }

    #[test]
    fn select_filters_raw_and_keeps_the_newest() {
        let mut log = Log::new(10);
        log.push(Kind::Agent, "start");
        log.push(Kind::Raw, "{}");
        log.push(Kind::Assistant, "a");
        log.push(Kind::Raw, "{}");
        log.push(Kind::Tool, "Edit src/lib.rs");
        let seqs = |v: Vec<Entry>| v.iter().map(|e| e.seq).collect::<Vec<_>>();
        assert_eq!(seqs(log.select(0, false, 10)), [1, 3, 5]);
        assert_eq!(seqs(log.select(0, false, 2)), [3, 5], "the newest two");
        assert_eq!(seqs(log.select(3, true, 10)), [4, 5]);
        assert!(log.select(5, false, 10).is_empty());
    }
}
