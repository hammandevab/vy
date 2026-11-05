//! `vy://` addresses (SPEC-7.1.1, SPEC-7.1.2, SPEC-9.5.1).

use std::fmt;
use std::str::FromStr;

/// Why text is not a `vy://` address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddressError {
    /// Not `vy://`.
    Scheme,
    /// Contains `#` (SPEC-7.1.2).
    Fragment,
    /// Empty or malformed host.
    Host,
    /// Port is not 1–65535 in canonical form.
    Port,
    /// `user@` is not part of the form.
    Userinfo,
    /// Whitespace, control or non-ASCII characters; hosts must be
    /// punycode and paths percent-encoded.
    Characters,
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Scheme => "address must start with vy:// (SPEC-7.1.1)",
            Self::Fragment => "fragments are not allowed in vy:// addresses (SPEC-7.1.2)",
            Self::Host => "address has no valid host",
            Self::Port => "address port is not 1-65535",
            Self::Userinfo => "address must not carry user information",
            Self::Characters => "address has characters outside printable ASCII",
        })
    }
}

impl std::error::Error for AddressError {}

/// A parsed `vy://host[:port]/path[?query]` address in canonical form:
/// lower-case host, port omitted when it is 443, path at least `/`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Address {
    host: String,
    port: Option<u16>,
    path: String,
    query: Option<String>,
}

/// Port implied when none is given: the HTTPS port (SPEC-7.1.1).
pub const DEFAULT_PORT: u16 = 443;

impl Address {
    /// Parse an address.
    pub fn parse(s: &str) -> Result<Address, AddressError> {
        let rest = s.strip_prefix("vy://").ok_or(AddressError::Scheme)?;
        if s.contains('#') {
            return Err(AddressError::Fragment);
        }
        if rest.bytes().any(|b| b <= b' ' || b >= 0x7f) {
            return Err(AddressError::Characters);
        }
        let (authority, path_query) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        if authority.contains('@') {
            return Err(AddressError::Userinfo);
        }
        let (host, port) = split_host_port(authority)?;
        let (path, query) = match path_query.split_once('?') {
            Some((p, q)) => (p.to_string(), Some(q.to_string())),
            None => (path_query.to_string(), None),
        };
        Ok(Address {
            host,
            port: port.filter(|&p| p != DEFAULT_PORT),
            path,
            query,
        })
    }

    /// Lower-case host (or bracketed IPv6 literal).
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Port, 443 when none was given.
    pub fn port(&self) -> u16 {
        self.port.unwrap_or(DEFAULT_PORT)
    }

    /// Path, starting with `/`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Query without `?`, if any.
    pub fn query(&self) -> Option<&str> {
        self.query.as_deref()
    }

    /// `host` or `host:port`.
    fn authority(&self) -> String {
        match self.port {
            Some(p) => format!("{}:{p}", self.host),
            None => self.host.clone(),
        }
    }

    /// The origin: scheme + host + port (SPEC-9.5.1), as `vy://host[:port]`.
    pub fn origin(&self) -> String {
        format!("vy://{}", self.authority())
    }

    /// True for the loopback names exempt from TLS (SPEC-7.1.3).
    pub fn is_localhost(&self) -> bool {
        matches!(self.host.as_str(), "localhost" | "127.0.0.1" | "[::1]")
    }

    /// The HTTPS URL the address maps to (SPEC-7.1.1).
    pub fn to_https(&self) -> String {
        format!("https://{}{}", self.authority(), self.path_and_query())
    }

    /// The URL actually fetched: `http://` for localhost, the only host
    /// allowed without TLS (SPEC-7.1.3), `https://` otherwise.
    pub fn fetch_url(&self) -> String {
        if self.is_localhost() {
            format!("http://{}{}", self.authority(), self.path_and_query())
        } else {
            self.to_https()
        }
    }

    fn path_and_query(&self) -> String {
        match &self.query {
            Some(q) => format!("{}?{q}", self.path),
            None => self.path.clone(),
        }
    }
}

fn split_host_port(authority: &str) -> Result<(String, Option<u16>), AddressError> {
    let (host, port) = if let Some(after) = authority.strip_prefix('[') {
        let (inner, tail) = after.split_once(']').ok_or(AddressError::Host)?;
        if inner.is_empty() || !inner.bytes().all(|b| b.is_ascii_hexdigit() || b == b':') {
            return Err(AddressError::Host);
        }
        let port = match tail {
            "" => None,
            t => Some(t.strip_prefix(':').ok_or(AddressError::Host)?),
        };
        (format!("[{}]", inner.to_ascii_lowercase()), port)
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        };
        if host.is_empty()
            || host.starts_with('.')
            || host.ends_with('.')
            || host.contains("..")
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        {
            return Err(AddressError::Host);
        }
        (host.to_ascii_lowercase(), port)
    };
    let port = match port {
        None => None,
        Some(p) => {
            if p.is_empty()
                || p.len() > 5
                || p.starts_with('0')
                || !p.bytes().all(|b| b.is_ascii_digit())
            {
                return Err(AddressError::Port);
            }
            Some(p.parse::<u16>().map_err(|_| AddressError::Port)?)
        }
    };
    Ok((host, port))
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "vy://{}{}", self.authority(), self.path_and_query())
    }
}

impl FromStr for Address {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, AddressError> {
        Address::parse(s)
    }
}

/// Translate a `vy://` address to the HTTPS URL it is fetched from
/// (SPEC-7.1.1); `None` for text that is not an address, including any
/// with a fragment (SPEC-7.1.2).
pub fn to_https(addr: &str) -> Option<String> {
    Address::parse(addr).ok().map(|a| a.to_https())
}
