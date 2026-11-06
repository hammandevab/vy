//! Test support, behind the `test-net` feature (and in this crate's own
//! tests): a [`Fetcher`] whose host names resolve to addresses a test
//! chooses while every connection goes to one server on loopback, in plain
//! HTTP that reports itself as TLS. The private-network rule (SPEC-9.5.3)
//! judges the chosen addresses before anything is connected, so a test in
//! another crate can load a page "from" a private or public address and
//! see what its requests reach. No product build enables the feature; the
//! product's agents use ureq's resolver and rustls ([`Fetcher::new`]).

use crate::dns::TxtResolver;
use crate::fetch::{Error, Fetcher, Options};
use crate::net::Net;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use ureq::config::Config;
use ureq::http;
use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, NextTimeout, TcpConnector, Transport,
};

/// A fetcher over the caches under `options.cache_dir`, with `resolver`
/// answering its TXT lookups, whose names resolve as `names` says (each
/// lookup of a name takes the next address of its list, the last one
/// again when the list runs out; an IP literal is itself) while every
/// connection goes to `port` on 127.0.0.1. Its [`Fetcher::requester`]
/// connects the same way. A name not in `names` is not found.
pub fn fetcher(
    options: Options,
    resolver: Box<dyn TxtResolver>,
    port: u16,
    names: &[(&str, &[&str])],
) -> Result<Fetcher, Error> {
    let net = net_with_hosts(Fetcher::test_config(&options), port, names);
    Fetcher::with_parts(options, resolver, net)
}

/// The plain transport, reporting itself as TLS.
#[derive(Debug)]
pub(crate) struct PlainAsTls<T: Transport>(T);

impl<T: Transport> Transport for PlainAsTls<T> {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.0.buffers()
    }
    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), ureq::Error> {
        self.0.transmit_output(amount, timeout)
    }
    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, ureq::Error> {
        self.0.await_input(timeout)
    }
    fn is_open(&mut self) -> bool {
        self.0.is_open()
    }
    fn is_tls(&self) -> bool {
        true
    }
}

/// The TLS step of a test agent: none.
#[derive(Debug)]
pub(crate) struct PretendTls;

impl<In: Transport> Connector<In> for PretendTls {
    type Out = PlainAsTls<In>;
    fn connect(
        &self,
        _details: &ConnectionDetails,
        chained: Option<In>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        Ok(chained.map(PlainAsTls))
    }
}

/// Names resolve as a test says; every address on `port`.
#[derive(Debug)]
struct Hosts {
    names: HashMap<String, Vec<IpAddr>>,
    lookups: Mutex<HashMap<String, usize>>,
    port: u16,
}

impl Resolver for Hosts {
    fn resolve(
        &self,
        uri: &http::Uri,
        _config: &Config,
        _timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let host = uri.host().unwrap_or("");
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let ip = match host.parse::<IpAddr>() {
            Ok(ip) => ip,
            Err(_) => {
                let list = self.names.get(host).ok_or(ureq::Error::HostNotFound)?;
                let mut lookups = self.lookups.lock().unwrap();
                let n = lookups.entry(host.to_string()).or_insert(0);
                let ip = list[(*n).min(list.len() - 1)];
                *n += 1;
                ip
            }
        };
        let mut addrs = self.empty();
        addrs.push(SocketAddr::new(ip, self.port));
        Ok(addrs)
    }
}

/// Connects to `port` on loopback whatever address the resolver gave:
/// the private-network rule has judged that address before this runs.
#[derive(Debug)]
struct ToServer(u16);

impl Connector for ToServer {
    type Out = <TcpConnector as Connector>::Out;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let mut addrs = details.resolver.empty();
        addrs.push(SocketAddr::from(([127, 0, 0, 1], self.0)));
        let to_server = ConnectionDetails {
            uri: details.uri,
            addrs,
            config: details.config,
            request_level: details.request_level,
            resolver: details.resolver,
            now: details.now,
            timeout: details.timeout,
            current_time: details.current_time.clone(),
            run_connector: details.run_connector.clone(),
            register_shutdown: details.register_shutdown.clone(),
        };
        TcpConnector::default().connect(&to_server, chained)
    }
}

/// Agents with `config` whose names resolve as `names` says, while every
/// connection goes to `port` on loopback in plain HTTP: the
/// private-network rule sees the addresses a test chose.
pub(crate) fn net_with_hosts(config: Config, port: u16, names: &[(&str, &[&str])]) -> Net {
    let names = names
        .iter()
        .map(|(n, ips)| {
            let ips = ips.iter().map(|ip| ip.parse().unwrap()).collect();
            (n.to_string(), ips)
        })
        .collect();
    Net::with_parts(
        config,
        ToServer(port).chain(PretendTls),
        Hosts {
            names,
            lookups: Mutex::new(HashMap::new()),
            port,
        },
    )
}
