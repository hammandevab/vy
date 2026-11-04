//! Status codes of SPEC appendix A as a Rust error (SPEC-A.1.1).

use core::fmt;
use vy_abi::err;

/// Why the host, or the SDK's local check, refused a call.
///
/// Each variant is one status code; [`VyError::code`] gives it back. The SDK
/// raises the same variant the host would log, so a page can treat both the
/// same way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VyError {
    /// Capability missing or revoked (`E_DENIED`).
    Denied,
    /// Invalid argument: pointer, length, fit, role (`E_ARG`).
    Arg,
    /// Module memory cap reached (`E_OOM`).
    Oom,
    /// Storage quota exhausted (`E_QUOTA`).
    Quota,
    /// Rate limit exceeded (`E_RATE`).
    Rate,
    /// Display-list call outside `dl_begin`/`dl_end` (`E_DL_STATE`).
    DlState,
    /// 65 536 opcodes or nodes reached (`E_DL_FULL`).
    DlFull,
    /// Clip stack deeper than 32, or pop without push (`E_DL_CLIP`).
    DlClip,
    /// Unknown resource id or wrong type (`E_RESOURCE`).
    Resource,
    /// Unknown style id (`E_STYLE`).
    Style,
    /// Origin not in the manifest's `net` list (`E_ORIGIN`).
    Origin,
    /// Transport error (`E_NET`).
    Net,
    /// Response larger than 64 MB (`E_TOO_LARGE`).
    TooLarge,
    /// Call requires a user gesture (`E_GESTURE`).
    Gesture,
    /// A negative code this SDK does not know; carried as is.
    Unknown(i32),
}

impl VyError {
    /// The error for a negative status code, or `None` for `OK` and other
    /// non-negative results.
    pub fn from_code(code: i32) -> Option<Self> {
        Some(match code {
            c if c >= err::OK => return None,
            err::E_DENIED => Self::Denied,
            err::E_ARG => Self::Arg,
            err::E_OOM => Self::Oom,
            err::E_QUOTA => Self::Quota,
            err::E_RATE => Self::Rate,
            err::E_DL_STATE => Self::DlState,
            err::E_DL_FULL => Self::DlFull,
            err::E_DL_CLIP => Self::DlClip,
            err::E_RESOURCE => Self::Resource,
            err::E_STYLE => Self::Style,
            err::E_ORIGIN => Self::Origin,
            err::E_NET => Self::Net,
            err::E_TOO_LARGE => Self::TooLarge,
            err::E_GESTURE => Self::Gesture,
            other => Self::Unknown(other),
        })
    }

    /// The ABI status code (SPEC-A.1.1).
    pub fn code(self) -> i32 {
        match self {
            Self::Denied => err::E_DENIED,
            Self::Arg => err::E_ARG,
            Self::Oom => err::E_OOM,
            Self::Quota => err::E_QUOTA,
            Self::Rate => err::E_RATE,
            Self::DlState => err::E_DL_STATE,
            Self::DlFull => err::E_DL_FULL,
            Self::DlClip => err::E_DL_CLIP,
            Self::Resource => err::E_RESOURCE,
            Self::Style => err::E_STYLE,
            Self::Origin => err::E_ORIGIN,
            Self::Net => err::E_NET,
            Self::TooLarge => err::E_TOO_LARGE,
            Self::Gesture => err::E_GESTURE,
            Self::Unknown(c) => c,
        }
    }

    /// Canonical name as the host logs it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Denied => "E_DENIED",
            Self::Arg => "E_ARG",
            Self::Oom => "E_OOM",
            Self::Quota => "E_QUOTA",
            Self::Rate => "E_RATE",
            Self::DlState => "E_DL_STATE",
            Self::DlFull => "E_DL_FULL",
            Self::DlClip => "E_DL_CLIP",
            Self::Resource => "E_RESOURCE",
            Self::Style => "E_STYLE",
            Self::Origin => "E_ORIGIN",
            Self::Net => "E_NET",
            Self::TooLarge => "E_TOO_LARGE",
            Self::Gesture => "E_GESTURE",
            Self::Unknown(_) => "E_UNKNOWN",
        }
    }
}

impl fmt::Display for VyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(c) => write!(f, "E_UNKNOWN({c})"),
            other => f.write_str(other.name()),
        }
    }
}

/// `Ok(code)` for a non-negative status, the error otherwise.
pub(crate) fn check(code: i32) -> Result<i32, VyError> {
    match VyError::from_code(code) {
        None => Ok(code),
        Some(e) => Err(e),
    }
}

/// `check` for the `i64` results of the text imports, whose negative codes
/// are sign-extended.
pub(crate) fn check64(v: i64) -> Result<i64, VyError> {
    if v < 0 {
        Err(VyError::from_code(v as i32).unwrap_or(VyError::Unknown(v as i32)))
    } else {
        Ok(v)
    }
}
