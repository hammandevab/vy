//! DNS TXT lookup of `_vy.<host>` (SPEC-7.4.2, SPEC-7.4.5).
//!
//! A stub resolver of a few hundred lines instead of a resolver crate: the
//! client needs exactly one record type from the system's resolver. On
//! Unix the query is built here and sent over UDP to the nameservers in
//! `/etc/resolv.conf`, with a fallback to TCP when the answer is truncated;
//! on Windows the system's `DnsQuery_UTF8` does the same. The wire-format
//! functions ([`build_query`], [`parse_answer`]) are pure, so tests and the
//! fuzz target exercise them without a network, and the shell injects a
//! [`TxtResolver`] of its own in tests.
//!
//! Only the answer section of a response whose id and question match the
//! query is read; a response from another source, for another question or
//! with another id is ignored and the wait goes on until the timeout.

use std::fmt;

/// TXT record type (RFC 1035 §3.2.2).
pub const TYPE_TXT: u16 = 16;
/// CNAME record type: an alias the answer section may follow.
pub const TYPE_CNAME: u16 = 5;
/// Most aliases followed in one answer.
pub const MAX_CNAMES: usize = 8;
/// Class IN.
pub const CLASS_IN: u16 = 1;
/// Largest UDP answer read; without EDNS a resolver sends at most 512
/// bytes and sets TC beyond that, after which TCP is used.
pub const MAX_UDP_ANSWER: usize = 4096;
/// Largest TCP answer accepted (the two-byte length prefix's range).
pub const MAX_TCP_ANSWER: usize = 65535;
/// Longest name sent (RFC 1035 §2.3.4, presentation form).
pub const MAX_NAME_LEN: usize = 253;
/// Most TXT records taken from one answer; a key file is 16 KB
/// (`MAX_KEY_FILE_BYTES`) and holds far fewer keys.
pub const MAX_RECORDS: usize = 64;

/// Why a TXT lookup gave no usable answer. Every one of these means
/// restricted mode on a first visit (SPEC-7.4.5): the client fails closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DnsError {
    /// No nameserver to ask.
    NoResolver,
    /// No answer within the timeout, after the retries.
    Timeout,
    /// The resolver answered with an error code (2 SERVFAIL, 5 REFUSED, ...).
    ServerFailure(u8),
    /// The name cannot be put in a query (a label over 63 bytes, a
    /// character outside letters, digits, `-` and `_`).
    BadName,
    /// A response that is not an answer to the query, or does not parse.
    Malformed(&'static str),
    /// A socket error.
    Io(String),
}

impl fmt::Display for DnsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoResolver => f.write_str("no DNS resolver configured"),
            Self::Timeout => f.write_str("DNS lookup timed out"),
            Self::ServerFailure(code) => write!(f, "DNS resolver answered with rcode {code}"),
            Self::BadName => f.write_str("name cannot be looked up in DNS"),
            Self::Malformed(what) => write!(f, "malformed DNS answer: {what}"),
            Self::Io(e) => write!(f, "DNS socket error: {e}"),
        }
    }
}

impl std::error::Error for DnsError {}

/// Looks up the TXT records of a name. The records are returned as text,
/// each record's character-strings concatenated (RFC 7208 §3.3 does the
/// same); `Ok(empty)` means the name has no TXT records (NXDOMAIN or
/// NODATA), which is not an error of the lookup.
pub trait TxtResolver: Send + Sync {
    /// The TXT records of `name`.
    fn txt(&self, name: &str) -> Result<Vec<String>, DnsError>;
}

/// What a parsed answer says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The TXT records (empty for NXDOMAIN and NODATA).
    Records(Vec<String>),
    /// The answer did not fit in the UDP message (TC); ask again over TCP.
    Truncated,
}

/// Split a name into wire labels; `None` when it cannot be queried.
fn labels(name: &str) -> Option<Vec<&[u8]>> {
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return None;
    }
    let mut out = Vec::new();
    for label in name.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return None;
        }
        out.push(label.as_bytes());
    }
    Some(out)
}

/// A query for the TXT records of `name` with message id `id`, recursion
/// desired; `None` when the name cannot be queried.
pub fn build_query(id: u16, name: &str) -> Option<Vec<u8>> {
    let labels = labels(name)?;
    let mut q = Vec::with_capacity(12 + name.len() + 6);
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&0x0100u16.to_be_bytes()); // RD
    q.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    q.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // AN, NS, AR
    for l in labels {
        q.push(l.len() as u8);
        q.extend_from_slice(l);
    }
    q.push(0);
    q.extend_from_slice(&TYPE_TXT.to_be_bytes());
    q.extend_from_slice(&CLASS_IN.to_be_bytes());
    Some(q)
}

/// A cursor over a message.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8, DnsError> {
        let b = *self
            .bytes
            .get(self.pos)
            .ok_or(DnsError::Malformed("truncated"))?;
        self.pos += 1;
        Ok(b)
    }

    fn u16(&mut self) -> Result<u16, DnsError> {
        Ok(u16::from_be_bytes([self.u8()?, self.u8()?]))
    }

    fn skip(&mut self, n: usize) -> Result<(), DnsError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.bytes.len())
            .ok_or(DnsError::Malformed("truncated"))?;
        self.pos = end;
        Ok(())
    }

    /// Read a possibly compressed name as lower-case labels joined by
    /// `.`; a pointer may only point backwards, so no loop is possible.
    fn name(&mut self) -> Result<String, DnsError> {
        let mut out = String::new();
        let mut pos = self.pos;
        let mut jumped = false;
        let mut total = 0usize;
        loop {
            let len = *self
                .bytes
                .get(pos)
                .ok_or(DnsError::Malformed("truncated name"))?;
            if len & 0xC0 == 0xC0 {
                let low = *self
                    .bytes
                    .get(pos + 1)
                    .ok_or(DnsError::Malformed("truncated pointer"))?;
                let target = (usize::from(len & 0x3F) << 8) | usize::from(low);
                if target >= pos {
                    return Err(DnsError::Malformed("forward pointer"));
                }
                if !jumped {
                    self.pos = pos + 2;
                    jumped = true;
                }
                pos = target;
                continue;
            }
            if len & 0xC0 != 0 {
                return Err(DnsError::Malformed("label type"));
            }
            let len = usize::from(len);
            if len == 0 {
                if !jumped {
                    self.pos = pos + 1;
                }
                return Ok(out);
            }
            if len > 63 {
                return Err(DnsError::Malformed("label length"));
            }
            total += len + 1;
            if total > 255 {
                return Err(DnsError::Malformed("name length"));
            }
            let label = self
                .bytes
                .get(pos + 1..pos + 1 + len)
                .ok_or(DnsError::Malformed("truncated label"))?;
            if !out.is_empty() {
                out.push('.');
            }
            for &b in label {
                out.push(char::from(b.to_ascii_lowercase()));
            }
            pos += 1 + len;
        }
    }
}

/// Parse `packet` as the answer to [`build_query`]`(id, name)`. An answer
/// with another id or question is [`DnsError::Malformed`]`("not ours")`;
/// RCODE 3 (NXDOMAIN) is an empty record list; other error codes are
/// [`DnsError::ServerFailure`]; TC gives [`Answer::Truncated`].
pub fn parse_answer(id: u16, name: &str, packet: &[u8]) -> Result<Answer, DnsError> {
    let mut r = Reader {
        bytes: packet,
        pos: 0,
    };
    if r.u16()? != id {
        return Err(DnsError::Malformed("not ours"));
    }
    let flags = r.u16()?;
    if flags & 0x8000 == 0 {
        return Err(DnsError::Malformed("not a response"));
    }
    if (flags >> 11) & 0xF != 0 {
        return Err(DnsError::Malformed("opcode"));
    }
    let qdcount = r.u16()?;
    let ancount = r.u16()?;
    r.skip(4)?; // NSCOUNT, ARCOUNT: not read
    if qdcount != 1 {
        return Err(DnsError::Malformed("question count"));
    }
    let qname = r.name()?;
    let qtype = r.u16()?;
    let qclass = r.u16()?;
    let asked = name.strip_suffix('.').unwrap_or(name).to_ascii_lowercase();
    if qname != asked || qtype != TYPE_TXT || qclass != CLASS_IN {
        return Err(DnsError::Malformed("not ours"));
    }
    match (flags & 0xF) as u8 {
        0 => {}
        3 => return Ok(Answer::Records(Vec::new())),
        code => return Err(DnsError::ServerFailure(code)),
    }
    if flags & 0x0200 != 0 {
        return Ok(Answer::Truncated);
    }
    // Only records owned by the name asked, or by an alias the answer
    // itself names for it (a CNAME chain, in order), are read: a TXT record
    // for some other name in the same answer is not the origin's.
    let mut owners = vec![asked];
    let mut records = Vec::new();
    for _ in 0..ancount {
        let owner = r.name()?;
        let rtype = r.u16()?;
        let rclass = r.u16()?;
        r.skip(4)?; // TTL
        let rdlength = usize::from(r.u16()?);
        let start = r.pos;
        let rdata = packet
            .get(start..start + rdlength)
            .ok_or(DnsError::Malformed("truncated rdata"))?;
        r.skip(rdlength)?;
        if rclass != CLASS_IN || !owners.contains(&owner) {
            continue;
        }
        if rtype == TYPE_CNAME {
            let mut alias = Reader {
                bytes: &packet[..start + rdlength],
                pos: start,
            };
            let target = alias.name()?;
            if owners.len() <= MAX_CNAMES && !owners.contains(&target) {
                owners.push(target);
            }
            continue;
        }
        if rtype != TYPE_TXT {
            continue;
        }
        let mut text = Vec::with_capacity(rdlength);
        let mut p = 0;
        while p < rdata.len() {
            let len = usize::from(rdata[p]);
            let s = rdata
                .get(p + 1..p + 1 + len)
                .ok_or(DnsError::Malformed("character-string"))?;
            text.extend_from_slice(s);
            p += 1 + len;
        }
        if records.len() < MAX_RECORDS {
            records.push(String::from_utf8_lossy(&text).into_owned());
        }
    }
    Ok(Answer::Records(records))
}

/// The resolver of the machine the client runs on.
#[derive(Clone, Debug)]
pub struct SystemResolver {
    /// Wait per attempt.
    pub timeout: std::time::Duration,
    /// Attempts in all (the first try and the retries), spread over the
    /// configured nameservers in turn.
    pub attempts: u32,
}

impl Default for SystemResolver {
    /// Two seconds per attempt, three attempts: a lookup that fails takes
    /// at most six seconds, once per origin ever (SPEC-7.4.5).
    fn default() -> Self {
        Self {
            timeout: std::time::Duration::from_secs(2),
            attempts: 3,
        }
    }
}

impl TxtResolver for SystemResolver {
    fn txt(&self, name: &str) -> Result<Vec<String>, DnsError> {
        system::txt(self, name)
    }
}

#[cfg(unix)]
mod system {
    use super::*;
    use std::io::{Read, Write};
    use std::net::{IpAddr, SocketAddr, TcpStream, UdpSocket};
    use std::time::{Duration, Instant};

    /// Where resolv.conf lives.
    pub const RESOLV_CONF: &str = "/etc/resolv.conf";

    /// The `nameserver` lines of resolv.conf text as socket addresses;
    /// `127.0.0.1` when there are none, as the C library assumes.
    pub fn nameservers(text: &str) -> Vec<SocketAddr> {
        let mut out = Vec::new();
        for line in text.lines() {
            let line = line.split(['#', ';']).next().unwrap_or("");
            let mut words = line.split_whitespace();
            if words.next() != Some("nameserver") {
                continue;
            }
            let Some(addr) = words.next() else {
                continue;
            };
            // A scoped IPv6 address (`fe80::1%eth0`) is skipped: the
            // scope cannot be expressed here.
            if let Ok(ip) = addr.parse::<IpAddr>() {
                out.push(SocketAddr::new(ip, 53));
            }
            if out.len() == 3 {
                break;
            }
        }
        if out.is_empty() {
            out.push(SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 53));
        }
        out
    }

    fn random_id() -> u16 {
        let mut b = [0u8; 2];
        // Without entropy the id is still unpredictable enough for a
        // connected socket that only takes the server's datagrams.
        let _ = getrandom::getrandom(&mut b);
        u16::from_be_bytes(b)
    }

    fn io(e: std::io::Error) -> DnsError {
        DnsError::Io(e.to_string())
    }

    /// One UDP exchange: send the query, take the first matching answer
    /// before the deadline.
    pub(crate) fn udp(
        server: SocketAddr,
        query: &[u8],
        id: u16,
        name: &str,
        timeout: Duration,
    ) -> Result<Answer, DnsError> {
        let bind: SocketAddr = if server.is_ipv4() {
            "0.0.0.0:0".parse().expect("literal")
        } else {
            "[::]:0".parse().expect("literal")
        };
        let socket = UdpSocket::bind(bind).map_err(io)?;
        socket.connect(server).map_err(io)?;
        socket.send(query).map_err(io)?;
        let deadline = Instant::now() + timeout;
        let mut buf = vec![0u8; MAX_UDP_ANSWER];
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(DnsError::Timeout);
            }
            socket.set_read_timeout(Some(deadline - now)).map_err(io)?;
            let n = match socket.recv(&mut buf) {
                Ok(n) => n,
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return Err(DnsError::Timeout)
                }
                Err(e) => return Err(io(e)),
            };
            match parse_answer(id, name, &buf[..n]) {
                // Someone else's datagram, or a damaged one: keep waiting.
                Err(DnsError::Malformed(_)) => continue,
                other => return other,
            }
        }
    }

    /// The same query over TCP, for a truncated UDP answer.
    pub(crate) fn tcp(
        server: SocketAddr,
        query: &[u8],
        id: u16,
        name: &str,
        timeout: Duration,
    ) -> Result<Answer, DnsError> {
        let mut stream = TcpStream::connect_timeout(&server, timeout).map_err(io)?;
        stream.set_read_timeout(Some(timeout)).map_err(io)?;
        stream.set_write_timeout(Some(timeout)).map_err(io)?;
        let mut msg = Vec::with_capacity(query.len() + 2);
        msg.extend_from_slice(&(query.len() as u16).to_be_bytes());
        msg.extend_from_slice(query);
        stream.write_all(&msg).map_err(io)?;
        let mut len = [0u8; 2];
        stream.read_exact(&mut len).map_err(io)?;
        let len = usize::from(u16::from_be_bytes(len)).min(MAX_TCP_ANSWER);
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf).map_err(io)?;
        match parse_answer(id, name, &buf)? {
            Answer::Truncated => Err(DnsError::Malformed("truncated over TCP")),
            records => Ok(records),
        }
    }

    pub fn txt(resolver: &SystemResolver, name: &str) -> Result<Vec<String>, DnsError> {
        let servers = nameservers(&std::fs::read_to_string(RESOLV_CONF).unwrap_or_default());
        if servers.is_empty() {
            return Err(DnsError::NoResolver);
        }
        let mut last = DnsError::Timeout;
        for attempt in 0..resolver.attempts.max(1) {
            let server = servers[attempt as usize % servers.len()];
            let id = random_id();
            let query = build_query(id, name).ok_or(DnsError::BadName)?;
            let outcome = match udp(server, &query, id, name, resolver.timeout) {
                Ok(Answer::Truncated) => tcp(server, &query, id, name, resolver.timeout),
                other => other,
            };
            match outcome {
                Ok(Answer::Records(r)) => return Ok(r),
                Ok(Answer::Truncated) => last = DnsError::Malformed("truncated over TCP"),
                Err(DnsError::BadName) => return Err(DnsError::BadName),
                // Timeouts, socket errors and SERVFAIL are retried on the
                // next server; the last error is what the caller sees.
                Err(e) => last = e,
            }
        }
        Err(last)
    }
}

#[cfg(windows)]
mod system {
    use super::*;
    use windows_sys::Win32::Foundation::{
        DNS_ERROR_RCODE_FORMAT_ERROR, DNS_ERROR_RCODE_NAME_ERROR, DNS_ERROR_RCODE_NOT_IMPLEMENTED,
        DNS_ERROR_RCODE_REFUSED, DNS_ERROR_RCODE_SERVER_FAILURE, DNS_ERROR_RECORD_DOES_NOT_EXIST,
        DNS_INFO_NO_RECORDS, ERROR_SUCCESS, ERROR_TIMEOUT,
    };
    use windows_sys::Win32::NetworkManagement::Dns::{
        DnsFree, DnsFreeRecordList, DnsQuery_UTF8, DnsSectionAnswer, DNS_QUERY_STANDARD,
        DNS_RECORDA, DNS_TYPE_TEXT,
    };

    /// The system resolver through `DnsQuery_UTF8`, which honours the
    /// adapter's nameservers, timeouts and retries; the timeout and
    /// attempt fields of [`SystemResolver`] are the system's here.
    pub fn txt(_resolver: &SystemResolver, name: &str) -> Result<Vec<String>, DnsError> {
        // The name is validated the same way as on Unix, so the two
        // platforms refuse the same names.
        build_query(0, name).ok_or(DnsError::BadName)?;
        // A trailing dot makes the name fully qualified: no search suffix
        // of the adapter is appended to it.
        let fqdn = format!("{}.", name.strip_suffix('.').unwrap_or(name));
        let cname = std::ffi::CString::new(fqdn).map_err(|_| DnsError::BadName)?;
        let mut records: *mut DNS_RECORDA = std::ptr::null_mut();
        // SAFETY: the name is a valid NUL-terminated string, the out
        // pointer is a valid location, and the record list is freed with
        // the matching free below after it is read.
        let status = unsafe {
            DnsQuery_UTF8(
                cname.as_ptr().cast(),
                DNS_TYPE_TEXT,
                DNS_QUERY_STANDARD,
                std::ptr::null_mut(),
                &mut records,
                std::ptr::null_mut(),
            )
        };
        match status {
            ERROR_SUCCESS => {}
            DNS_ERROR_RCODE_NAME_ERROR | DNS_ERROR_RECORD_DOES_NOT_EXIST => return Ok(Vec::new()),
            s if s == DNS_INFO_NO_RECORDS as u32 => return Ok(Vec::new()),
            ERROR_TIMEOUT => return Err(DnsError::Timeout),
            DNS_ERROR_RCODE_FORMAT_ERROR => return Err(DnsError::ServerFailure(1)),
            DNS_ERROR_RCODE_SERVER_FAILURE => return Err(DnsError::ServerFailure(2)),
            DNS_ERROR_RCODE_NOT_IMPLEMENTED => return Err(DnsError::ServerFailure(4)),
            DNS_ERROR_RCODE_REFUSED => return Err(DnsError::ServerFailure(5)),
            other => return Err(DnsError::Io(format!("DnsQuery_UTF8 failed with {other}"))),
        }
        let mut out = Vec::new();
        let mut rec = records;
        while !rec.is_null() && out.len() < MAX_RECORDS {
            // SAFETY: `rec` is a node of the list the system just
            // returned and has not been freed yet.
            unsafe {
                let r = &*rec;
                // The section is the low two bits of the flags; only the
                // answer section is read, as on Unix. The system has
                // already followed any CNAME chain.
                let answer = (r.Flags.DW & 0x3) == DnsSectionAnswer as u32;
                if r.wType == DNS_TYPE_TEXT && answer {
                    let txt = &r.Data.TXT;
                    let strings = txt.pStringArray.as_ptr();
                    let mut text = Vec::new();
                    for i in 0..txt.dwStringCount as usize {
                        let s = *strings.add(i);
                        if !s.is_null() {
                            text.extend_from_slice(std::ffi::CStr::from_ptr(s.cast()).to_bytes());
                        }
                    }
                    out.push(String::from_utf8_lossy(&text).into_owned());
                }
                rec = r.pNext;
            }
        }
        if !records.is_null() {
            // SAFETY: `records` came from DnsQuery_UTF8 and is freed once.
            unsafe { DnsFree(records.cast(), DnsFreeRecordList) };
        }
        Ok(out)
    }
}

#[cfg(not(any(unix, windows)))]
mod system {
    use super::*;

    pub fn txt(_resolver: &SystemResolver, _name: &str) -> Result<Vec<String>, DnsError> {
        Err(DnsError::NoResolver)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A response to `query` with `rcode`, `tc` and the given TXT records
    /// (each a list of character-strings), the question echoed and the
    /// answer names compressed to point at it.
    pub(crate) fn answer(query: &[u8], rcode: u8, tc: bool, records: &[&[&[u8]]]) -> Vec<u8> {
        let mut p = query.to_vec();
        let flags = 0x8100u16 | (u16::from(tc) << 9) | u16::from(rcode);
        p[2..4].copy_from_slice(&flags.to_be_bytes());
        p[6..8].copy_from_slice(&(records.len() as u16).to_be_bytes());
        for strings in records {
            p.extend_from_slice(&[0xC0, 12]); // pointer to the question name
            p.extend_from_slice(&TYPE_TXT.to_be_bytes());
            p.extend_from_slice(&CLASS_IN.to_be_bytes());
            p.extend_from_slice(&300u32.to_be_bytes());
            let rdlength: usize = strings.iter().map(|s| s.len() + 1).sum();
            p.extend_from_slice(&(rdlength as u16).to_be_bytes());
            for s in *strings {
                p.push(s.len() as u8);
                p.extend_from_slice(s);
            }
        }
        p
    }

    /// SPEC-7.4.2, SPEC-7.4.5: the query is RFC 1035's, the answer parser
    /// takes only an answer to that query, joins the character-strings of
    /// each TXT record, reads NXDOMAIN as no records, reports TC and error
    /// codes, and refuses what does not parse.
    #[test]
    fn spec_7_4_2_txt_wire_format() {
        let q = build_query(0x1234, "_vy.exempel.se").unwrap();
        assert_eq!(
            q,
            [
                0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0, 3, b'_', b'v', b'y', 7, b'e', b'x',
                b'e', b'm', b'p', b'e', b'l', 2, b's', b'e', 0, 0, 16, 0, 1
            ]
        );
        assert_eq!(build_query(0x1234, "_vy.exempel.se."), Some(q.clone()));
        for bad in [
            "",
            ".",
            "a..b",
            "-",
            &"a".repeat(64),
            &format!("{}.se", "ab.".repeat(90)),
            "exempel.se:8443",
            "[::1]",
            "ex ample.se",
            "ex\u{e4}mpel.se",
        ] {
            assert_eq!(build_query(1, bad), None, "{bad:?}");
        }

        let key = "v=vy1; k=".to_string() + &"ab".repeat(32);
        let two = answer(&q, 0, false, &[&[key.as_bytes()], &[b"v=spf1 ", b"-all"]]);
        assert_eq!(
            parse_answer(0x1234, "_VY.Exempel.SE", &two),
            Ok(Answer::Records(vec![key.clone(), "v=spf1 -all".into()]))
        );
        assert_eq!(
            parse_answer(0x1234, "_vy.exempel.se", &answer(&q, 3, false, &[])),
            Ok(Answer::Records(vec![]))
        );
        assert_eq!(
            parse_answer(0x1234, "_vy.exempel.se", &answer(&q, 0, true, &[])),
            Ok(Answer::Truncated)
        );
        assert_eq!(
            parse_answer(0x1234, "_vy.exempel.se", &answer(&q, 2, false, &[])),
            Err(DnsError::ServerFailure(2))
        );
        assert_eq!(
            parse_answer(0x1235, "_vy.exempel.se", &two),
            Err(DnsError::Malformed("not ours"))
        );
        assert_eq!(
            parse_answer(0x1234, "_vy.annan.se", &two),
            Err(DnsError::Malformed("not ours"))
        );
        assert_eq!(
            parse_answer(0x1234, "_vy.exempel.se", &q),
            Err(DnsError::Malformed("not a response"))
        );
        // An A record among the answers is skipped; an empty TXT is a
        // record with no text.
        let mut mixed = answer(&q, 0, false, &[&[b"x"], &[]]);
        mixed[q.len() + 2] = 0;
        mixed[q.len() + 3] = 1; // type A for the first record
        assert_eq!(
            parse_answer(0x1234, "_vy.exempel.se", &mixed),
            Ok(Answer::Records(vec![String::new()]))
        );
        // Damage: truncated rdata, a forward pointer, a 64-byte label.
        let mut cut = two.clone();
        cut.truncate(two.len() - 3);
        assert!(matches!(
            parse_answer(0x1234, "_vy.exempel.se", &cut),
            Err(DnsError::Malformed(_))
        ));
        let mut forward = two.clone();
        forward[q.len() + 1] = 0xFF;
        assert_eq!(
            parse_answer(0x1234, "_vy.exempel.se", &forward),
            Err(DnsError::Malformed("forward pointer"))
        );
        let mut long = two.clone();
        long[12] = 64;
        assert!(parse_answer(0x1234, "_vy.exempel.se", &long).is_err());
        assert!(parse_answer(0x1234, "_vy.exempel.se", &[]).is_err());
        assert!(parse_answer(0x1234, "_vy.exempel.se", &two[..12]).is_err());
        // A CNAME chain is followed in order; a TXT record owned by a name
        // outside the chain is not read.
        let mut chain = q.clone();
        chain[2..4].copy_from_slice(&0x8180u16.to_be_bytes());
        chain[6..8].copy_from_slice(&3u16.to_be_bytes());
        let rr = |p: &mut Vec<u8>, owner: &[u8], rtype: u16, rdata: &[u8]| {
            p.extend_from_slice(owner);
            p.extend_from_slice(&rtype.to_be_bytes());
            p.extend_from_slice(&CLASS_IN.to_be_bytes());
            p.extend_from_slice(&60u32.to_be_bytes());
            p.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
            p.extend_from_slice(rdata);
        };
        let alias_at = chain.len() + 12; // where the CNAME's rdata starts
        rr(
            &mut chain,
            &[0xC0, 12],
            TYPE_CNAME,
            b"\x03_vy\x03cdn\x07example\x00",
        );
        let alias = [0xC0, alias_at as u8];
        let mut txt = vec![key.len() as u8];
        txt.extend_from_slice(key.as_bytes());
        rr(&mut chain, &alias, TYPE_TXT, &txt);
        rr(&mut chain, b"\x05other\x00", TYPE_TXT, b"\x05stray");
        assert_eq!(
            parse_answer(0x1234, "_vy.exempel.se", &chain),
            Ok(Answer::Records(vec![key.clone()]))
        );
        // A self-pointing name does not loop.
        let mut looped = q.clone();
        looped[2] = 0x81;
        looped.truncate(12);
        looped.extend_from_slice(&[0xC0, 12, 0, 16, 0, 1]);
        assert_eq!(
            parse_answer(0x1234, "_vy.exempel.se", &looped),
            Err(DnsError::Malformed("forward pointer"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolv_conf_nameservers() {
        let text = "# comment\nsearch example\nnameserver 10.0.0.1 ; x\nnameserver 2001:db8::1\n\
                    nameserver fe80::1%eth0\noptions timeout:1\nnameserver 10.0.0.2\nnameserver 10.0.0.3\n";
        let ns = system::nameservers(text);
        assert_eq!(
            ns,
            vec![
                "10.0.0.1:53".parse().unwrap(),
                "[2001:db8::1]:53".parse().unwrap(),
                "10.0.0.2:53".parse().unwrap(),
            ]
        );
        assert_eq!(
            system::nameservers(""),
            vec!["127.0.0.1:53".parse::<std::net::SocketAddr>().unwrap()]
        );
    }

    /// The system resolver against a fake nameserver on loopback: the
    /// first answer is truncated, so the same question goes over TCP; a
    /// datagram with another id is ignored; a silent server is a timeout
    /// after the attempts.
    #[cfg(unix)]
    #[test]
    fn system_resolver_udp_then_tcp() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, UdpSocket};
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let tcp = TcpListener::bind(("127.0.0.1", udp.local_addr().unwrap().port())).unwrap();
        let port = udp.local_addr().unwrap().port();
        let key = "v=vy1; k=".to_string() + &"cd".repeat(32);
        let served = key.clone();
        let server = std::thread::spawn(move || {
            let mut buf = [0u8; 512];
            let (n, from) = udp.recv_from(&mut buf).unwrap();
            let q = buf[..n].to_vec();
            // A wrong id first, then the truncated answer.
            let mut wrong = answer(&q, 0, false, &[&[b"nope"]]);
            wrong[0] ^= 0xFF;
            udp.send_to(&wrong, from).unwrap();
            udp.send_to(&answer(&q, 0, true, &[]), from).unwrap();
            let (mut s, _) = tcp.accept().unwrap();
            let mut len = [0u8; 2];
            s.read_exact(&mut len).unwrap();
            let mut tq = vec![0u8; usize::from(u16::from_be_bytes(len))];
            s.read_exact(&mut tq).unwrap();
            assert_eq!(tq[12..], q[12..], "same question over TCP");
            let a = answer(&tq, 0, false, &[&[served.as_bytes()]]);
            s.write_all(&(a.len() as u16).to_be_bytes()).unwrap();
            s.write_all(&a).unwrap();
        });
        let resolver = SystemResolver {
            timeout: std::time::Duration::from_secs(5),
            attempts: 1,
        };
        let id = 0x4242;
        let query = build_query(id, "_vy.test.example").unwrap();
        let server_addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        // Drive the two exchanges directly (the resolver would read
        // /etc/resolv.conf for the server).
        let first = system_test::udp(
            server_addr,
            &query,
            id,
            "_vy.test.example",
            resolver.timeout,
        );
        assert_eq!(first, Ok(Answer::Truncated));
        let second = system_test::tcp(
            server_addr,
            &query,
            id,
            "_vy.test.example",
            resolver.timeout,
        );
        assert_eq!(second, Ok(Answer::Records(vec![key])));
        server.join().unwrap();

        let silent = UdpSocket::bind("127.0.0.1:0").unwrap();
        let quick = SystemResolver {
            timeout: std::time::Duration::from_millis(100),
            attempts: 2,
        };
        let started = std::time::Instant::now();
        let r = system_test::udp(
            silent.local_addr().unwrap(),
            &query,
            id,
            "_vy.test.example",
            quick.timeout,
        );
        assert_eq!(r, Err(DnsError::Timeout));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[cfg(unix)]
    pub(crate) mod system_test {
        pub(crate) use super::super::system::{tcp, udp};
    }
}
