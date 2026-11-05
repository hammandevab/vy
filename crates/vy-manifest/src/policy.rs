//! Capability policy (SPEC-8.2.*).

use crate::manifest::{CapabilityRequests, NetRequest, StorageRequest};
use crate::validate::AUTO_STORAGE_QUOTA_KB;

/// What the client does with a requested capability before the user says
/// anything (SPEC-8.2.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grant {
    /// Granted without asking.
    Auto,
    /// Granted only after the user agrees.
    Ask,
    /// Not granted unless the user turns it on explicitly.
    Denied,
}

/// One capability request as it stands in the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requested<'a> {
    /// `net` with its origins.
    Net(&'a NetRequest),
    /// `storage` with its quota.
    Storage(&'a StorageRequest),
    /// `clipboard`.
    Clipboard,
    /// `hires_time`.
    HiresTime,
    /// `ime`.
    Ime,
    /// `persist_frame`.
    PersistFrame,
}

impl Requested<'_> {
    /// Canonical name (`vy_abi::cap::NAMES`).
    pub fn name(&self) -> &'static str {
        match self {
            Self::Net(_) => "net",
            Self::Storage(_) => "storage",
            Self::Clipboard => "clipboard",
            Self::HiresTime => "hires_time",
            Self::Ime => "ime",
            Self::PersistFrame => "persist_frame",
        }
    }

    /// Capability id (`vy_abi::cap`).
    pub fn id(&self) -> i32 {
        vy_abi::cap::id(self.name()).expect("known capability")
    }
}

impl CapabilityRequests {
    /// The requests in id order.
    pub fn iter(&self) -> impl Iterator<Item = Requested<'_>> {
        let mut out = Vec::with_capacity(6);
        if let Some(n) = &self.net {
            out.push(Requested::Net(n));
        }
        if let Some(s) = &self.storage {
            out.push(Requested::Storage(s));
        }
        if self.clipboard.is_some() {
            out.push(Requested::Clipboard);
        }
        if self.hires_time.is_some() {
            out.push(Requested::HiresTime);
        }
        if self.ime.is_some() {
            out.push(Requested::Ime);
        }
        if self.persist_frame.is_some() {
            out.push(Requested::PersistFrame);
        }
        out.into_iter()
    }

    /// True when the manifest requests the capability with this name.
    pub fn requests(&self, name: &str) -> bool {
        self.iter().any(|r| r.name() == name)
    }

    /// Names of the capabilities to grant given a decision per request. A
    /// name the manifest does not request is never in the result, whatever
    /// the decision (SPEC-8.2.2).
    pub fn grants(&self, mut decide: impl FnMut(&Requested<'_>) -> bool) -> Vec<&'static str> {
        self.iter()
            .filter(|r| decide(r))
            .map(|r| r.name())
            .collect()
    }
}

/// The default policy (SPEC-8.2.3): `net` for the page's own origin,
/// `storage` up to 1 MB and `ime` are granted automatically; `clipboard`
/// and `persist_frame` after asking; `hires_time` is denied by default.
/// `net` with foreign origins and `storage` over 1 MB are not covered by
/// the sentence and are treated as "ask".
///
/// `origin_is_own` says whether every origin in a `net` request is the
/// page's own (SPEC-9.5.1); it is ignored for other capabilities.
pub fn default_grant(request: &Requested<'_>, origin_is_own: bool) -> Grant {
    match request {
        Requested::Net(_) => {
            if origin_is_own {
                Grant::Auto
            } else {
                Grant::Ask
            }
        }
        Requested::Storage(s) => {
            if s.quota_kb <= AUTO_STORAGE_QUOTA_KB {
                Grant::Auto
            } else {
                Grant::Ask
            }
        }
        Requested::Ime => Grant::Auto,
        Requested::Clipboard | Requested::PersistFrame => Grant::Ask,
        Requested::HiresTime => Grant::Denied,
    }
}
