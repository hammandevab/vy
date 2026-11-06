//! The private-network rule (SPEC-9.5.3): which addresses a connection
//! made for a page may reach. It is decided after the host is resolved, in
//! the resolver of the agent that makes the connection, so every hop of a
//! redirect, a blob URL, the key file and the log, and the page's own
//! requests are covered, and so is a name that resolves to the user's
//! machine (DNS rebinding), not only a literal address.
//!
//! A page's class is that of its own address: loopback for the loopback
//! names of SPEC-7.1.3, else the most public class its host resolves to. A
//! page reaches its own class and every more public one. The page's own
//! first request is held the other way: it connects only to an address as
//! local as the class it was given or more, so a name that resolved to a
//! private address when it was classed cannot then be served from a public
//! one with private reach.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::Duration;
use ureq::config::Config;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{ConnectionDetails, Connector, DefaultConnector, NextTimeout};
use ureq::{http, Agent};

/// Longest wait for the lookup that classes a page's own address.
const CLASSIFY_TIMEOUT: Duration = Duration::from_secs(10);

/// Where an address leads (SPEC-9.5.3), from the most local to the most
/// public.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NetClass {
    /// The user's own machine: `127.0.0.0/8` and `::1`, and `0.0.0.0/8`
    /// and `::`, which reach it.
    Loopback,
    /// The local network: `10.0.0.0/8`, `100.64.0.0/10`, `172.16.0.0/12`,
    /// `192.168.0.0/16`, `fc00::/7` and the link-local `169.254.0.0/16`
    /// and `fe80::/10`.
    Private,
    /// Every other address.
    Public,
}

impl NetClass {
    /// The class of `ip`. An IPv4 address in IPv6 form (`::ffff:a.b.c.d`,
    /// and the deprecated `::a.b.c.d`) is classed as the IPv4 address.
    pub fn of(ip: IpAddr) -> NetClass {
        match ip {
            IpAddr::V4(v4) => Self::of_v4(v4),
            IpAddr::V6(v6) => Self::of_v6(v6),
        }
    }

    fn of_v4(ip: Ipv4Addr) -> NetClass {
        match ip.octets() {
            [127 | 0, ..] => NetClass::Loopback,
            [10, ..] | [192, 168, ..] | [169, 254, ..] => NetClass::Private,
            [172, b, ..] if b & 0xf0 == 16 => NetClass::Private,
            [100, b, ..] if b & 0xc0 == 64 => NetClass::Private,
            _ => NetClass::Public,
        }
    }

    fn of_v6(ip: Ipv6Addr) -> NetClass {
        if ip.is_loopback() || ip.is_unspecified() {
            return NetClass::Loopback;
        }
        let s = ip.segments();
        if s[..5] == [0; 5] && (s[5] == 0xffff || s[5] == 0) {
            let [a, b] = s[6].to_be_bytes();
            let [c, d] = s[7].to_be_bytes();
            return Self::of_v4(Ipv4Addr::new(a, b, c, d));
        }
        if s[0] & 0xfe00 == 0xfc00 || s[0] & 0xffc0 == 0xfe80 {
            return NetClass::Private;
        }
        NetClass::Public
    }

    /// Whether a page whose own address is of this class may connect to an
    /// address of class `target`: its own class and every more public one.
    pub fn may_reach(self, target: NetClass) -> bool {
        target >= self
    }
}

/// For a caller that knows only whether the page's own address is a
/// loopback name (SPEC-7.1.3): loopback, or else public, the class that
/// reaches the least.
impl From<bool> for NetClass {
    fn from(page_is_localhost: bool) -> Self {
        if page_is_localhost {
            NetClass::Loopback
        } else {
            NetClass::Public
        }
    }
}

impl fmt::Display for NetClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NetClass::Loopback => "loopback",
            NetClass::Private => "private",
            NetClass::Public => "public",
        })
    }
}

/// How one connection is checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rule {
    /// On behalf of a page of this class: only addresses it may reach.
    From(NetClass),
    /// The page's own first request: only addresses of this class or a
    /// more local one.
    Within(NetClass),
}

impl Rule {
    fn allows(self, class: NetClass) -> bool {
        match self {
            Rule::From(page) => page.may_reach(class),
            Rule::Within(page) => class <= page,
        }
    }

    /// The rule as agents are kept: the two that allow everything are one.
    fn key(self) -> Rule {
        match self {
            Rule::Within(NetClass::Public) => Rule::From(NetClass::Loopback),
            rule => rule,
        }
    }
}

/// One agent per distinct rule.
const RULES: [Rule; 5] = [
    Rule::From(NetClass::Loopback),
    Rule::From(NetClass::Private),
    Rule::From(NetClass::Public),
    Rule::Within(NetClass::Loopback),
    Rule::Within(NetClass::Private),
];

/// A connection the rule refused: carried through ureq as the source of
/// an I/O error and turned into `Error::PrivateNetwork` by the fetcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Refused {
    /// The address the host resolved to (the first refused, when all were).
    pub address: IpAddr,
    /// Its class.
    pub class: NetClass,
    /// The rule that refused it.
    pub rule: Rule,
}

impl Refused {
    /// The refusal inside a ureq error, if that is what the error is.
    pub(crate) fn find(e: &ureq::Error) -> Option<Refused> {
        match e {
            ureq::Error::Io(io) => io.get_ref()?.downcast_ref::<Refused>().copied(),
            _ => None,
        }
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.rule {
            Rule::From(page) => write!(
                f,
                "{} is a {} address, out of reach for a page on a {page} address (SPEC-9.5.3)",
                self.address, self.class
            ),
            Rule::Within(page) => write!(
                f,
                "{} is a {} address, but the page's own address was {page} when it was looked up (SPEC-9.5.3)",
                self.address, self.class
            ),
        }
    }
}

impl std::error::Error for Refused {}

/// The resolver of an agent: the addresses `inner` gives, less those
/// `rule` refuses; an error when none is left, so nothing is connected.
#[derive(Debug)]
struct Guard {
    inner: Arc<dyn Resolver>,
    rule: Rule,
}

impl Resolver for Guard {
    fn resolve(
        &self,
        uri: &http::Uri,
        config: &Config,
        timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let all = self.inner.resolve(uri, config, timeout)?;
        let mut kept = self.empty();
        let mut refused = None;
        for addr in all.iter() {
            let class = NetClass::of(addr.ip());
            if self.rule.allows(class) {
                kept.push(*addr);
            } else if refused.is_none() {
                refused = Some(Refused {
                    address: addr.ip(),
                    class,
                    rule: self.rule,
                });
            }
        }
        match refused {
            Some(r) if kept.is_empty() => Err(ureq::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                r,
            ))),
            _ => Ok(kept),
        }
    }
}

/// One connector shared by the agents of a [`Net`], so the TLS
/// configuration is built once.
#[derive(Debug)]
struct Shared<C>(Arc<C>);

impl<C: Connector> Connector for Shared<C> {
    type Out = C::Out;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        self.0.connect(details, chained)
    }
}

/// The agents a fetcher connects through: one per [`Rule`], over one
/// connector and one resolver.
#[derive(Clone, Debug)]
pub(crate) struct Net {
    config: Config,
    resolver: Arc<dyn Resolver>,
    agents: Arc<[(Rule, Agent)]>,
}

impl Net {
    /// ureq's connector (rustls) and the system's resolver, with `config`.
    pub(crate) fn new(config: Config) -> Net {
        Self::with_parts(
            config,
            DefaultConnector::default(),
            DefaultResolver::default(),
        )
    }

    /// Over `connector` and `resolver`; tests reach a loopback server
    /// under any name this way.
    pub(crate) fn with_parts<C: Connector>(
        config: Config,
        connector: C,
        resolver: impl Resolver,
    ) -> Net {
        let connector = Arc::new(connector);
        let resolver: Arc<dyn Resolver> = Arc::new(resolver);
        let agents = RULES
            .iter()
            .map(|&rule| {
                let guard = Guard {
                    inner: Arc::clone(&resolver),
                    rule,
                };
                let agent =
                    Agent::with_parts(config.clone(), Shared(Arc::clone(&connector)), guard);
                (rule, agent)
            })
            .collect();
        Net {
            config,
            resolver,
            agents,
        }
    }

    /// The agent whose every connection is checked with `rule`.
    pub(crate) fn agent(&self, rule: Rule) -> &Agent {
        let key = rule.key();
        self.agents
            .iter()
            .find(|(r, _)| *r == key)
            .map(|(_, agent)| agent)
            .expect("an agent per rule")
    }

    /// The class of a page whose own URL is `url` (SPEC-9.5.3): loopback
    /// when its address is a loopback name (`localhost`, SPEC-7.1.3), else
    /// the most public class its host resolves to, and public when it does
    /// not resolve.
    pub(crate) fn classify(&self, url: &str, localhost: bool) -> NetClass {
        if localhost {
            return NetClass::Loopback;
        }
        let Ok(uri) = url.parse::<http::Uri>() else {
            return NetClass::Public;
        };
        let timeout = NextTimeout {
            after: CLASSIFY_TIMEOUT.into(),
            reason: ureq::Timeout::Resolve,
            per_read: None,
            per_write: None,
        };
        match self.resolver.resolve(&uri, &self.config, timeout) {
            Ok(addrs) => addrs
                .iter()
                .map(|a| NetClass::of(a.ip()))
                .max()
                .unwrap_or(NetClass::Public),
            Err(_) => NetClass::Public,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-9.5.3: the classes of the address ranges, IPv4 in IPv6 form
    /// included, and who reaches whom.
    #[test]
    fn spec_9_5_3_address_classes() {
        use NetClass::*;
        for (ip, class) in [
            ("127.0.0.1", Loopback),
            ("127.255.255.254", Loopback),
            ("0.0.0.0", Loopback),
            ("0.1.2.3", Loopback),
            ("10.0.0.1", Private),
            ("10.255.255.255", Private),
            ("100.63.255.255", Public),
            ("100.64.0.1", Private),
            ("100.127.255.255", Private),
            ("100.128.0.0", Public),
            ("172.15.255.255", Public),
            ("172.16.0.1", Private),
            ("172.31.255.255", Private),
            ("172.32.0.0", Public),
            ("192.168.1.1", Private),
            ("192.169.0.1", Public),
            ("169.254.169.254", Private),
            ("8.8.8.8", Public),
            ("93.184.216.34", Public),
            ("::1", Loopback),
            ("::", Loopback),
            ("::ffff:127.0.0.1", Loopback),
            ("::ffff:0.0.0.0", Loopback),
            ("::127.0.0.1", Loopback),
            ("::ffff:10.0.0.1", Private),
            ("::ffff:169.254.169.254", Private),
            ("::ffff:8.8.8.8", Public),
            ("fc00::1", Private),
            ("fd12:3456::1", Private),
            ("fe80::1", Private),
            ("febf::1", Private),
            ("fec0::1", Public),
            ("2001:4860:4860::8888", Public),
            ("64:ff9b::808:808", Public),
        ] {
            assert_eq!(NetClass::of(ip.parse().unwrap()), class, "{ip}");
        }
        assert!(Loopback.may_reach(Loopback) && Loopback.may_reach(Private));
        assert!(Loopback.may_reach(Public));
        assert!(!Private.may_reach(Loopback));
        assert!(Private.may_reach(Private) && Private.may_reach(Public));
        assert!(!Public.may_reach(Loopback) && !Public.may_reach(Private));
        assert!(Public.may_reach(Public));
        assert_eq!(NetClass::from(true), Loopback);
        assert_eq!(NetClass::from(false), Public);
        assert!(Rule::Within(Private).allows(Loopback));
        assert!(Rule::Within(Private).allows(Private));
        assert!(!Rule::Within(Private).allows(Public));
        assert_eq!(Rule::Within(Public).key(), Rule::From(Loopback));
    }
}
