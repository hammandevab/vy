//! Fetching a page (SPEC §7): translate the address, GET it over HTTPS,
//! accept a whole package or a bare manifest, fill modules and resources
//! from the content-addressed cache, and decide trust from the signature.

use crate::address::Address;
use crate::cache::{
    now_secs, BlobCache, HttpCache, HttpRecord, DEFAULT_CACHE_CAP_BYTES,
    DEFAULT_HTTP_CACHE_CAP_BYTES,
};
use crate::dns::{DnsError, SystemResolver, TxtResolver};
use crate::keys::{KeyStore, LogHead};
use crate::net::{Net, NetClass, Refused, Rule};
use crate::url::{cache_control, media_type, resolve, CacheControl, HttpUrl};
use std::fmt;
use std::net::IpAddr;
use std::path::PathBuf;
use std::time::Duration;
use ureq::config::Config;
use vy_manifest::{
    dns_txt_name, parse_dns_txt, parse_key_file, Grant, Hash, KeyLog, Manifest, Package,
    PackageError, Problem, PublicKey, Trust, MAX_LOG_BYTES, MAX_MANIFEST_BYTES, MEDIA_MANIFEST,
    MEDIA_PACKAGE,
};

/// The `Accept` header of every page request (SPEC-7.1.1).
pub const ACCEPT: &str = "application/vy-package+zstd, application/vy-manifest+cbor";
/// Most redirects followed (SPEC-7.5.1).
pub const MAX_REDIRECTS: u32 = 3;
/// Largest compressed package downloaded: the decompressed cap of
/// [`vy_manifest::MAX_PACKAGE_BYTES`] is the real limit.
pub const MAX_PACKAGE_DOWNLOAD_BYTES: u64 = vy_manifest::MAX_PACKAGE_BYTES;
/// Largest `/.well-known/vy-key` body.
pub const MAX_KEY_FILE_BYTES: u64 = 16 * 1024;
/// Largest single blob fetched by hash, whatever a manifest declares: no
/// module or resource may be larger than the whole resource budget
/// (SPEC-4.1.4, SPEC-4.1.5).
pub const MAX_BLOB_BYTES: u64 = vy_manifest::MAX_RESOURCES_BYTES;
/// Where `/.well-known/vy-key` lives (SPEC-7.4.2).
pub const WELL_KNOWN_KEY_PATH: &str = "/.well-known/vy-key";
/// Where an origin serves its key transparency log (SPEC-7.6.1).
pub const WELL_KNOWN_LOG_PATH: &str = "/.well-known/vy-log";
/// Prefix of the default blob URL when a manifest entry has no `url`:
/// `/_vy/<hex sha256>` relative to the manifest.
pub const BLOB_PATH_PREFIX: &str = "/_vy/";

/// Why a fetch failed. None of these are rendered; the shell shows an
/// error view with the address and, for [`Error::NotVy`], an offer to open
/// it externally (SPEC-7.2.2).
#[derive(Debug)]
pub enum Error {
    /// A URL that needs TLS was not `https://` (SPEC-7.1.3).
    TlsRequired {
        /// The offending URL.
        url: String,
    },
    /// More than three redirects (SPEC-7.5.1).
    TooManyRedirects {
        /// The last URL.
        url: String,
    },
    /// A redirect without a usable `Location`.
    BadRedirect {
        /// The redirecting URL.
        url: String,
    },
    /// An HTTP status other than 200 (or 304 on revalidation).
    Status {
        /// The URL.
        url: String,
        /// The status.
        status: u16,
    },
    /// The response is neither a package nor a manifest (SPEC-7.2.2).
    NotVy {
        /// The URL.
        url: String,
        /// Its media type.
        media_type: String,
    },
    /// The body exceeded the limit for its kind.
    TooLarge {
        /// The URL.
        url: String,
        /// The limit in bytes.
        limit: u64,
    },
    /// Connection, DNS or protocol failure.
    Transport {
        /// The URL.
        url: String,
        /// What went wrong.
        message: String,
    },
    /// The private-network rule refused the connection (SPEC-9.5.3):
    /// nothing was connected.
    PrivateNetwork {
        /// The URL.
        url: String,
        /// The address its host resolved to.
        address: IpAddr,
        /// That address's class.
        class: NetClass,
        /// The class of the page's own address.
        page: NetClass,
    },
    /// The manifest does not decode (SPEC-4.1.3, SPEC-8.1.*).
    Manifest(vy_manifest::Error),
    /// The package does not unpack (SPEC-4.4.1).
    Package(PackageError),
    /// A blob's bytes do not hash to the manifest's value (SPEC-9.4.1).
    HashMismatch {
        /// The URL fetched.
        url: String,
        /// Expected hash.
        expected: Hash,
    },
    /// The manifest does not describe the module and resources fetched.
    Invalid(Vec<Problem>),
    /// The cache directory could not be used.
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TlsRequired { url } => write!(f, "{url}: TLS required (SPEC-7.1.3)"),
            Self::TooManyRedirects { url } => {
                write!(f, "{url}: more than {MAX_REDIRECTS} redirects (SPEC-7.5.1)")
            }
            Self::BadRedirect { url } => write!(f, "{url}: redirect without a usable Location"),
            Self::Status { url, status } => write!(f, "{url}: HTTP {status}"),
            Self::NotVy { url, media_type } => {
                write!(f, "{url}: {media_type} is not a Vy page (SPEC-7.2.2)")
            }
            Self::TooLarge { url, limit } => write!(f, "{url}: body over {limit} bytes"),
            Self::Transport { url, message } => write!(f, "{url}: {message}"),
            Self::PrivateNetwork {
                url,
                address,
                class,
                page,
            } => write!(
                f,
                "{url}: {address} is a {class} address, which a page on a {page} address may not reach (SPEC-9.5.3)"
            ),
            Self::Manifest(e) => write!(f, "manifest: {e}"),
            Self::Package(e) => write!(f, "package: {e}"),
            Self::HashMismatch { url, expected } => {
                write!(f, "{url}: bytes do not hash to {expected} (SPEC-9.4.1)")
            }
            Self::Invalid(problems) => {
                write!(f, "manifest does not describe the page:")?;
                for p in problems {
                    write!(f, "\n  {p}")?;
                }
                Ok(())
            }
            Self::Io(e) => write!(f, "cache: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<vy_manifest::Error> for Error {
    fn from(e: vy_manifest::Error) -> Self {
        Self::Manifest(e)
    }
}

impl From<PackageError> for Error {
    fn from(e: PackageError) -> Self {
        Self::Package(e)
    }
}

/// Settings of a [`Fetcher`].
#[derive(Clone, Debug)]
pub struct Options {
    /// Directory for the caches and the key store.
    pub cache_dir: PathBuf,
    /// Size cap of the content-addressed cache in bytes.
    pub cache_cap_bytes: u64,
    /// Size cap of the HTTP cache of page responses in bytes (SPEC-7.3.2).
    pub http_cache_cap_bytes: u64,
    /// Whole-request timeout.
    pub timeout: Duration,
}

impl Options {
    /// Defaults: 2 GB blob cap, 64 MB HTTP cap, 30 s timeout.
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        Options {
            cache_dir: cache_dir.into(),
            cache_cap_bytes: DEFAULT_CACHE_CAP_BYTES,
            http_cache_cap_bytes: DEFAULT_HTTP_CACHE_CAP_BYTES,
            timeout: Duration::from_secs(30),
        }
    }
}

/// Why a fetched page is not trusted (SPEC-7.4.3): the reason the shell
/// shows next to its "restricted mode" marker. None of these is an error
/// of the fetch; the page is rendered, without capabilities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Restriction {
    /// The manifest carries no signature.
    Unsigned,
    /// The signature does not verify under the key it names.
    BadSignature,
    /// A pinned origin: the key is neither pinned nor announced as a
    /// successor (SPEC-7.4.4).
    UnknownKey {
        /// The key id the manifest names.
        key_id: String,
    },
    /// `/.well-known/vy-key` could not be fetched.
    KeyFileUnavailable(String),
    /// A first visit: the key is not among those both the key file and
    /// DNS publish (SPEC-7.4.2).
    KeyNotPublished {
        /// The key id the manifest names.
        key_id: String,
    },
    /// A first visit: `_vy.<host>` has no `v=vy1` TXT record (SPEC-7.4.5).
    DnsNoRecord {
        /// The name looked up.
        name: String,
    },
    /// A first visit: the TXT records and the key file name no key in
    /// common (SPEC-7.4.5).
    DnsMismatch {
        /// The name looked up.
        name: String,
    },
    /// A first visit: the lookup failed (SPEC-7.4.5).
    DnsFailed {
        /// The name looked up.
        name: String,
        /// Why.
        error: DnsError,
    },
    /// An announced successor is not in `/.well-known/vy-key` (SPEC-7.4.4).
    SuccessorNotPublished {
        /// The key id announced.
        key_id: String,
    },
    /// The origin's key log could not be fetched or its chain does not
    /// hold (SPEC-7.6.2).
    LogInvalid(String),
    /// The origin serves a key log that does not list the key (SPEC-7.6.2).
    NotInLog {
        /// The key id the manifest names.
        key_id: String,
    },
    /// The key log no longer starts with the entries seen before: its
    /// history was rewritten (SPEC-7.6.2).
    LogRewritten,
    /// The origin served a key log once and now serves none (SPEC-7.6.3).
    LogWithdrawn,
}

impl fmt::Display for Restriction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsigned => f.write_str("the manifest is not signed"),
            Self::BadSignature => f.write_str("the signature does not verify"),
            Self::UnknownKey { key_id } => {
                write!(f, "key {key_id} is not pinned for the origin and was never announced (SPEC-7.4.4)")
            }
            Self::KeyFileUnavailable(e) => write!(f, "{WELL_KNOWN_KEY_PATH}: {e}"),
            Self::KeyNotPublished { key_id } => {
                write!(f, "key {key_id} is not among the keys published for a first visit ({WELL_KNOWN_KEY_PATH} and, off loopback, DNS; SPEC-7.4.5)")
            }
            Self::DnsNoRecord { name } => {
                write!(f, "no v=vy1 TXT record at {name} (SPEC-7.4.5)")
            }
            Self::DnsMismatch { name } => {
                write!(
                    f,
                    "the TXT records at {name} and {WELL_KNOWN_KEY_PATH} share no key (SPEC-7.4.5)"
                )
            }
            Self::DnsFailed { name, error } => write!(f, "TXT lookup of {name} failed: {error}"),
            Self::SuccessorNotPublished { key_id } => {
                write!(
                    f,
                    "announced key {key_id} is not in {WELL_KNOWN_KEY_PATH} (SPEC-7.4.4)"
                )
            }
            Self::LogInvalid(e) => write!(f, "{WELL_KNOWN_LOG_PATH}: {e} (SPEC-7.6.2)"),
            Self::NotInLog { key_id } => {
                write!(f, "key {key_id} is not in the origin's key log {WELL_KNOWN_LOG_PATH} (SPEC-7.6.2)")
            }
            Self::LogRewritten => write!(
                f,
                "{WELL_KNOWN_LOG_PATH} no longer starts with the entries seen before (SPEC-7.6.2)"
            ),
            Self::LogWithdrawn => write!(
                f,
                "the origin served a key log before and now serves none (SPEC-7.6.3)"
            ),
        }
    }
}

/// Which of the two response forms the server sent (SPEC-7.2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// `application/vy-package+zstd`.
    Package,
    /// `application/vy-manifest+cbor` with blobs fetched by hash.
    Manifest,
}

/// A fetched page.
#[derive(Debug)]
pub struct Fetched {
    /// The address asked for.
    pub address: Address,
    /// Manifest, module and resources, hashes verified (SPEC-9.4.1). The
    /// manifest's capability requests are readable here (the runtime needs
    /// the `net` origins and the storage quota) but yield no grant: that
    /// takes the trust token below, through [`Fetched::grants`].
    pub package: Package,
    /// Package identity (SPEC-4.1.2).
    pub identity: Hash,
    /// The signature's verification under a key trusted for the origin
    /// (SPEC-7.4.2); `None` means restricted mode: no capabilities
    /// (SPEC-7.4.3). Private, so the only way from a page to a grant is
    /// [`Fetched::grants`].
    trust: Option<Trust>,
    /// Why there is no trust, when there is none.
    restriction: Option<Restriction>,
    /// Id of the key the manifest names.
    pub key_id: String,
    /// Response form.
    pub form: Form,
    /// Network requests made, 0 when everything came from the caches.
    pub requests: u32,
    /// The class of the page's own address (SPEC-9.5.3).
    net_class: NetClass,
}

impl Fetched {
    /// The origin (SPEC-9.5.1).
    pub fn origin(&self) -> String {
        self.address.origin()
    }

    /// True when the signature verifies with a key trusted for the origin;
    /// false means restricted mode (SPEC-7.4.3).
    pub fn trusted(&self) -> bool {
        self.trust.is_some()
    }

    /// Why the page runs in restricted mode, for the shell to show
    /// (SPEC-7.4.5); `None` when it is trusted.
    pub fn restriction(&self) -> Option<&Restriction> {
        self.restriction.as_ref()
    }

    /// The class of the page's own address (SPEC-9.5.3), what its own
    /// requests are checked against: pass it to [`Fetcher::request`] or
    /// [`Requester::request`].
    pub fn net_class(&self) -> NetClass {
        self.net_class
    }

    /// Default grant per requested capability (SPEC-8.2.3); every one is
    /// [`Grant::Denied`] when the page is not trusted (SPEC-7.4.3). The
    /// only way to a grant for a fetched page: the trust token it needs is
    /// private to this struct and produced by signature verification alone.
    pub fn grants(&self) -> Vec<(&'static str, Grant)> {
        let authority = &self.origin()["vy://".len()..];
        let own_origins = [
            format!("vy://{authority}"),
            format!("https://{authority}"),
            format!("http://{authority}"),
        ];
        let own_origins = if self.address.is_localhost() {
            &own_origins[..]
        } else {
            &own_origins[..2]
        };
        self.package
            .manifest
            .capabilities
            .default_grants(self.trust.as_ref(), |n| {
                n.origins.iter().all(|o| own_origins.contains(o))
            })
    }
}

/// A plain HTTP response as the fetcher sees it.
struct Response {
    url: String,
    status: u16,
    media_type: String,
    etag: Option<String>,
    cache: CacheControl,
    body: Vec<u8>,
}

/// Whether `url` may be fetched for a page at an address that is or is not
/// localhost (SPEC-7.1.3): `https://` always; plain `http://` only to a
/// loopback host, and only for a page whose own address is loopback. The
/// exception is for developing against a local server, not a way for a
/// remote page to steer the client at services on the user's machine
/// through a redirect or a blob URL. Where an `https://` URL may lead is
/// the private-network rule's (SPEC-9.5.3), decided after the host is
/// resolved.
pub fn transport_allowed(page_is_localhost: bool, url: &HttpUrl) -> bool {
    url.scheme == "https" || (page_is_localhost && url.is_localhost())
}

/// A ureq error as the fetcher reports it: a refusal by the private-network
/// rule (SPEC-9.5.3) as [`Error::PrivateNetwork`], anything else as
/// [`Error::Transport`].
fn transport_error(url: &str, e: ureq::Error) -> Error {
    match Refused::find(&e) {
        Some(r) => Error::PrivateNetwork {
            url: url.to_string(),
            address: r.address,
            class: r.class,
            page: match r.rule {
                Rule::From(page) | Rule::Within(page) => page,
            },
        },
        None => Error::Transport {
            url: url.to_string(),
            message: e.to_string(),
        },
    }
}

/// Who one GET is made for (SPEC-7.1.3, SPEC-9.5.3).
#[derive(Clone, Copy, Debug)]
struct Reach {
    /// The page's own address is a loopback name: plain `http://` to
    /// loopback is allowed (SPEC-7.1.3).
    local: bool,
    /// The class of the page's own address.
    page: NetClass,
    /// The first hop is the page's own address: it connects only to an
    /// address of the page's class or a more local one.
    own: bool,
}

impl Reach {
    /// The rule for hop `hop` of the GET.
    fn rule(self, hop: u32) -> Rule {
        if self.own && hop == 0 {
            Rule::Within(self.page)
        } else {
            Rule::From(self.page)
        }
    }
}

/// The HTTP cache's key for `url` fetched for a page of class `page`: a
/// response is reused only under the class it was fetched with, so a page
/// whose name now resolves to a more local address is fetched again, held
/// to [`Rule::Within`], instead of running from the cache with more reach.
fn http_key(url: &str, page: NetClass) -> String {
    match page {
        NetClass::Public => url.to_string(),
        class => format!("{url} ({class})"),
    }
}

/// Body limit per media type of the page response: a manifest is at most
/// 64 KB (SPEC-4.1.3), a package at most the decompressed cap, and the
/// body of anything else is not read at all (SPEC-7.2.2).
fn page_limit(media_type: &str) -> u64 {
    match media_type {
        MEDIA_MANIFEST => MAX_MANIFEST_BYTES as u64,
        MEDIA_PACKAGE => MAX_PACKAGE_DOWNLOAD_BYTES,
        _ => 0,
    }
}

/// A page's own requests (`fetch_begin`, SPEC-5.2.7) without the rest of
/// a [`Fetcher`]: the same agent configuration, cheap to clone, usable
/// from any thread. Nothing is cached and no key is consulted; the same
/// rules apply as in [`Fetcher::request`].
#[derive(Clone, Debug)]
pub struct Requester {
    net: Net,
}

impl Requester {
    /// A requester with the agents [`Fetcher::new`] would build for
    /// `options`.
    pub fn new(options: &Options) -> Self {
        Self {
            net: Net::new(Fetcher::config(options)),
        }
    }

    /// As [`Fetcher::request`].
    pub fn request(
        &self,
        method: &str,
        url: &str,
        body: &[u8],
        page: NetClass,
        limit: u64,
    ) -> Result<(u16, Vec<u8>), Error> {
        request_with(&self.net, method, url, body, page, limit)
    }
}

/// [`Fetcher::request`] through `net`.
fn request_with(
    net: &Net,
    method: &str,
    url: &str,
    body: &[u8],
    page: NetClass,
    limit: u64,
) -> Result<(u16, Vec<u8>), Error> {
    let parsed = HttpUrl::parse(url).ok_or_else(|| Error::BadRedirect {
        url: url.to_string(),
    })?;
    if !transport_allowed(page == NetClass::Loopback, &parsed) {
        return Err(Error::TlsRequired {
            url: url.to_string(),
        });
    }
    let agent = net.agent(Rule::From(page));
    let transport = |e: ureq::Error| transport_error(url, e);
    let mut resp = match method {
        "GET" => agent.get(url).call(),
        "DELETE" => agent.delete(url).call(),
        "POST" => agent.post(url).send(body),
        "PUT" => agent.put(url).send(body),
        other => {
            return Err(Error::Transport {
                url: url.to_string(),
                message: format!("method {other} is not GET, POST, PUT or DELETE"),
            })
        }
    }
    .map_err(transport)?;
    let status = resp.status().as_u16();
    let body = resp
        .body_mut()
        .with_config()
        .limit(limit.saturating_add(1))
        .read_to_vec()
        .map_err(|e| match e {
            ureq::Error::BodyExceedsLimit(_) => Error::TooLarge {
                url: url.to_string(),
                limit,
            },
            e => transport(e),
        })?;
    if body.len() as u64 > limit {
        return Err(Error::TooLarge {
            url: url.to_string(),
            limit,
        });
    }
    Ok((status, body))
}

/// Fetches pages with a content-addressed cache, an HTTP cache for
/// manifests and a per-origin key store.
pub struct Fetcher {
    net: Net,
    blobs: BlobCache,
    http: HttpCache,
    keys: KeyStore,
    resolver: Box<dyn TxtResolver>,
}

impl Fetcher {
    /// Open the caches under `options.cache_dir`, with the system's DNS
    /// resolver for the first visit of an origin (SPEC-7.4.5).
    pub fn new(options: Options) -> Result<Self, Error> {
        Self::with_resolver(options, Box::new(SystemResolver::default()))
    }

    /// As [`Fetcher::new`], with `resolver` answering the TXT lookups.
    pub fn with_resolver(options: Options, resolver: Box<dyn TxtResolver>) -> Result<Self, Error> {
        let net = Net::new(Self::config(&options));
        Self::with_parts(options, resolver, net)
    }

    /// The HTTP configuration: no automatic redirects (they are followed
    /// by hand so every hop is checked for TLS, SPEC-7.1.3, and counted,
    /// SPEC-7.5.1), no ambient proxy (SPEC-2.1.4: an environment variable
    /// must not redirect a page fetch; explicit proxy settings are a later
    /// option), no content encoding, one whole-request timeout.
    fn config(options: &Options) -> Config {
        Config::builder()
            .max_redirects(0)
            .http_status_as_error(false)
            .proxy(None)
            .user_agent("vy-proto/0.1")
            .accept_encoding("")
            .timeout_global(Some(options.timeout))
            .build()
    }

    /// Open the caches under `options.cache_dir` around ready agents.
    /// Tests build them with a connector that reaches a loopback server
    /// under a remote name; the product uses [`Fetcher::new`].
    pub(crate) fn with_parts(
        options: Options,
        resolver: Box<dyn TxtResolver>,
        net: Net,
    ) -> Result<Self, Error> {
        let io = |e: std::io::Error| Error::Io(e.to_string());
        Ok(Fetcher {
            net,
            blobs: BlobCache::open(&options.cache_dir, options.cache_cap_bytes).map_err(io)?,
            http: HttpCache::open(&options.cache_dir, options.http_cache_cap_bytes).map_err(io)?,
            keys: KeyStore::open(&options.cache_dir).map_err(io)?,
            resolver,
        })
    }

    /// The agents' configuration for test agents built around another
    /// connector.
    #[cfg(any(test, feature = "test-net"))]
    pub(crate) fn test_config(options: &Options) -> Config {
        Self::config(options)
    }

    /// The content-addressed cache.
    pub fn blobs(&self) -> &BlobCache {
        &self.blobs
    }

    /// The key store.
    pub fn keys(&self) -> &KeyStore {
        &self.keys
    }

    /// One plain request on a page's behalf (`fetch_begin`, SPEC-5.2.7),
    /// made by the main process because the tab process has no network
    /// (SPEC-9.1.1). `page` is the class of the page's own address
    /// ([`Fetched::net_class`]).
    /// `method` is `GET`, `POST`, `PUT` or `DELETE`; the URL must be
    /// `https://`, or `http://` to loopback for a page on loopback
    /// (SPEC-7.1.3), and its host must resolve to an address the page may
    /// reach (SPEC-9.5.3), else [`Error::PrivateNetwork`] and nothing is
    /// connected; redirects are not followed, no `Referer` is sent
    /// (SPEC-7.5.1), and the body is read up to `limit` bytes, beyond
    /// which the result is [`Error::TooLarge`]. Returns the status and the
    /// body; a status other than 2xx is still a response, not an error.
    /// Nothing is cached: the page's own requests are its own business.
    pub fn request(
        &self,
        method: &str,
        url: &str,
        body: &[u8],
        page: NetClass,
        limit: u64,
    ) -> Result<(u16, Vec<u8>), Error> {
        request_with(&self.net, method, url, body, page, limit)
    }

    /// A handle that makes the same requests as [`Fetcher::request`] with
    /// this fetcher's agents, for another thread: the page's own requests
    /// need no cache or key store, so they need not wait for a page load.
    pub fn requester(&self) -> Requester {
        Requester {
            net: self.net.clone(),
        }
    }

    /// Ask the server whether a page changed, whatever the HTTP cache's
    /// freshness says (SPEC-7.3.2): a stored response is revalidated with
    /// `If-None-Match` (a 304 costs no body), anything else is fetched, and
    /// the cache keeps the answer, so a [`Fetcher::fetch`] right after loads
    /// what the server has now. Returns the SHA-256 of the manifest or
    /// package the server answers with; two different answers mean the page
    /// changed. For development: the client's live reload of a localhost
    /// page (vy-shell devtools).
    pub fn revalidate(&mut self, address: &Address) -> Result<Hash, Error> {
        let url = address.fetch_url();
        let reach = self.page_reach(address);
        let key = http_key(&url, reach.page);
        if let Some(mut rec) = self.http.get(&key) {
            rec.max_age = 0;
            let _ = self.http.put(&key, &rec);
        }
        let mut requests = 0;
        let resp = self.get_with_http_cache(&url, reach, &mut requests)?;
        Ok(Hash::of(&resp.body))
    }

    /// How the page at `address` is fetched: its address classed
    /// (SPEC-9.5.3), its own URL the first hop.
    fn page_reach(&self, address: &Address) -> Reach {
        let local = address.is_localhost();
        Reach {
            local,
            page: self.net.classify(&address.fetch_url(), local),
            own: true,
        }
    }

    /// Fetch a page.
    pub fn fetch(&mut self, address: &Address) -> Result<Fetched, Error> {
        let mut requests = 0;
        let reach = self.page_reach(address);
        // Everything after the page's own URL: blobs, key file, log.
        let fetched_for = Reach {
            own: false,
            ..reach
        };
        let primary = self.get_with_http_cache(&address.fetch_url(), reach, &mut requests)?;
        let (package, form) = match primary.media_type.as_str() {
            MEDIA_PACKAGE => {
                let package = Package::unpack(&primary.body)?;
                // Blobs from a package feed the content cache so a later
                // bare-manifest response for the same hashes needs no
                // network (SPEC-7.3.1).
                self.store_blob(&package.manifest.module.hash, &package.module)?;
                for (r, bytes) in package.manifest.resources.iter().zip(&package.resources) {
                    self.store_blob(&r.hash, bytes)?;
                }
                (package, Form::Package)
            }
            MEDIA_MANIFEST => {
                let manifest = Manifest::decode(&primary.body)?;
                // What the manifest declares bounds what is downloaded
                // (SPEC-4.1.4, SPEC-4.1.5): checked before the first blob,
                // or a hostile `size` would have the client buffer without end.
                let declared = manifest.declared_size_problems();
                if !declared.is_empty() {
                    return Err(Error::Invalid(declared));
                }
                let module = self.blob(
                    &manifest.module.hash,
                    manifest.module.size,
                    manifest.module.url.as_deref(),
                    &primary.url,
                    fetched_for,
                    &mut requests,
                )?;
                let mut resources = Vec::with_capacity(manifest.resources.len());
                for r in &manifest.resources {
                    resources.push(self.blob(
                        &r.hash,
                        r.size,
                        r.url.as_deref(),
                        &primary.url,
                        fetched_for,
                        &mut requests,
                    )?);
                }
                let package = Package {
                    manifest,
                    module,
                    resources,
                };
                let problems = package.validate();
                if !problems.is_empty() {
                    return Err(Error::Invalid(problems));
                }
                (package, Form::Manifest)
            }
            other => {
                return Err(Error::NotVy {
                    url: primary.url,
                    media_type: other.to_string(),
                })
            }
        };
        let key_id = package
            .manifest
            .sig
            .as_ref()
            .map(|s| s.key_id.clone())
            .unwrap_or_default();
        let (trust, restriction) =
            match self.trust(address, fetched_for, &package.manifest, &mut requests) {
                Ok(trust) => (Some(trust), None),
                Err(why) => (None, Some(why)),
            };
        Ok(Fetched {
            address: address.clone(),
            identity: package.identity(),
            package,
            trust,
            restriction,
            key_id,
            form,
            requests,
            net_class: reach.page,
        })
    }

    fn store_blob(&mut self, hash: &Hash, bytes: &[u8]) -> Result<(), Error> {
        if self.blobs.contains(hash) {
            return Ok(());
        }
        self.blobs
            .put(hash, bytes)
            .map_err(|e| Error::Io(e.to_string()))
    }

    /// A module or resource by hash: from the cache, else fetched from
    /// its URL and verified (SPEC-7.3.1, SPEC-9.4.1).
    fn blob(
        &mut self,
        hash: &Hash,
        size: u64,
        url: Option<&str>,
        base: &str,
        reach: Reach,
        requests: &mut u32,
    ) -> Result<Vec<u8>, Error> {
        if let Some(bytes) = self.blobs.get(hash) {
            return Ok(bytes);
        }
        let reference = match url {
            Some(u) => u.to_string(),
            None => format!("{BLOB_PATH_PREFIX}{}", hash.hex()),
        };
        let url = resolve(base, &reference).ok_or_else(|| Error::BadRedirect {
            url: reference.clone(),
        })?;
        // `size` is the manifest's claim, already bounded by
        // `declared_size_problems`; the cap here is the last line of defence.
        let limit = size.min(MAX_BLOB_BYTES);
        let resp = self.get(&url, "*/*", &|_| limit, None, reach, requests)?;
        if resp.status != 200 {
            return Err(Error::Status {
                url: resp.url,
                status: resp.status,
            });
        }
        if Hash::of(&resp.body) != *hash {
            return Err(Error::HashMismatch {
                url: resp.url,
                expected: *hash,
            });
        }
        self.blobs
            .put(hash, &resp.body)
            .map_err(|e| Error::Io(e.to_string()))?;
        Ok(resp.body)
    }

    /// GET through the HTTP cache (SPEC-7.3.2): a fresh record is used
    /// without a request; a stale one is revalidated with `If-None-Match`.
    /// Records are kept per class of the page ([`http_key`]).
    fn get_with_http_cache(
        &mut self,
        url: &str,
        reach: Reach,
        requests: &mut u32,
    ) -> Result<Response, Error> {
        let now = now_secs();
        let key = http_key(url, reach.page);
        let cached = self.http.get(&key);
        if let Some(rec) = &cached {
            if rec.is_fresh(now) {
                return Ok(Response {
                    url: rec.url.clone(),
                    status: 200,
                    media_type: rec.media_type.clone(),
                    etag: rec.etag.clone(),
                    cache: crate::url::CacheControl {
                        max_age: rec.max_age,
                        no_store: false,
                    },
                    body: rec.body.clone(),
                });
            }
        }
        let etag = cached.as_ref().and_then(|r| r.etag.clone());
        let resp = self.get(url, ACCEPT, &page_limit, etag.as_deref(), reach, requests)?;
        if resp.status == 304 {
            if let Some(mut rec) = cached {
                rec.fetched_at = now;
                if resp.cache.max_age > 0 {
                    rec.max_age = resp.cache.max_age;
                }
                if resp.etag.is_some() {
                    rec.etag = resp.etag.clone();
                }
                let _ = self.http.put(&key, &rec);
                return Ok(Response {
                    url: rec.url,
                    status: 200,
                    media_type: rec.media_type,
                    etag: rec.etag,
                    cache: resp.cache,
                    body: rec.body,
                });
            }
        }
        if resp.status != 200 {
            return Err(Error::Status {
                url: resp.url,
                status: resp.status,
            });
        }
        if resp.cache.no_store {
            self.http.remove(&key);
        } else {
            let rec = HttpRecord {
                url: resp.url.clone(),
                etag: resp.etag.clone(),
                max_age: resp.cache.max_age,
                fetched_at: now,
                media_type: resp.media_type.clone(),
                body: resp.body.clone(),
            };
            let _ = self.http.put(&key, &rec);
        }
        Ok(resp)
    }

    /// One GET with up to [`MAX_REDIRECTS`] hops, each checked with
    /// [`transport_allowed`] before it is connected (SPEC-7.1.3) and by
    /// the private-network rule once its host is resolved (SPEC-9.5.3), as
    /// `reach` says. Never sends `Referer` (SPEC-7.5.1). `limit` gives the
    /// body cap for the media type received; 0 means the body is not read.
    fn get(
        &self,
        url: &str,
        accept: &str,
        limit: &dyn Fn(&str) -> u64,
        if_none_match: Option<&str>,
        reach: Reach,
        requests: &mut u32,
    ) -> Result<Response, Error> {
        let mut url = url.to_string();
        for hop in 0..=MAX_REDIRECTS {
            let parsed =
                HttpUrl::parse(&url).ok_or_else(|| Error::BadRedirect { url: url.clone() })?;
            if !transport_allowed(reach.local, &parsed) {
                return Err(Error::TlsRequired { url });
            }
            let agent = self.net.agent(reach.rule(hop));
            let mut req = agent.get(&url).header("Accept", accept);
            if let Some(etag) = if_none_match {
                req = req.header("If-None-Match", etag);
            }
            *requests += 1;
            let mut resp = req.call().map_err(|e| transport_error(&url, e))?;
            let status = resp.status().as_u16();
            let header = |name: &str| -> Option<String> {
                resp.headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            };
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                if hop == MAX_REDIRECTS {
                    return Err(Error::TooManyRedirects { url });
                }
                let location =
                    header("location").ok_or_else(|| Error::BadRedirect { url: url.clone() })?;
                url = resolve(&url, &location).ok_or(Error::BadRedirect { url })?;
                continue;
            }
            let media_type = header("content-type")
                .map(|v| media_type(&v))
                .unwrap_or_default();
            let etag = header("etag");
            let cache = header("cache-control")
                .map(|v| cache_control(&v))
                .unwrap_or_default();
            let limit = limit(&media_type);
            let body = if limit == 0 {
                Vec::new()
            } else {
                // ureq's limit reader fails on the read after exactly
                // `limit` bytes, so allow one more and check the length.
                let body = resp
                    .body_mut()
                    .with_config()
                    .limit(limit.saturating_add(1))
                    .read_to_vec()
                    .map_err(|e| match e {
                        ureq::Error::BodyExceedsLimit(_) => Error::TooLarge {
                            url: url.clone(),
                            limit,
                        },
                        e => Error::Transport {
                            url: url.clone(),
                            message: e.to_string(),
                        },
                    })?;
                if body.len() as u64 > limit {
                    return Err(Error::TooLarge { url, limit });
                }
                body
            };
            return Ok(Response {
                url,
                status,
                media_type,
                etag,
                cache,
                body,
            });
        }
        Err(Error::TooManyRedirects { url })
    }

    /// Decide trust for the manifest of `address` (SPEC-7.4.2, SPEC-7.4.3,
    /// SPEC-7.4.4, SPEC-7.4.5): the key must be pinned for the origin, or
    /// be published by both the key file and DNS on a first visit, or have
    /// been announced as successor and be in the key file; the token is
    /// the signature's verification under that key. Anything else is
    /// restricted mode, with the reason.
    fn trust(
        &mut self,
        address: &Address,
        reach: Reach,
        manifest: &Manifest,
        requests: &mut u32,
    ) -> Result<Trust, Restriction> {
        let Some(sig) = &manifest.sig else {
            return Err(Restriction::Unsigned);
        };
        let origin = address.origin();
        let key = match self.keys.pinned(&origin, &sig.key_id) {
            Some(k) => k,
            None if !self.keys.has_pins(&origin) => {
                // First visit: pin what both sources publish (SPEC-7.4.5)
                // and, when the origin serves a log, the log lists
                // (SPEC-7.6.2).
                let agreed = self.first_visit_keys(address, reach, requests)?;
                let published = agreed.iter().any(|k| k.key_id() == sig.key_id);
                let (logged, head) = self.logged_keys(address, reach, agreed, requests)?;
                for k in &logged {
                    let _ = self.keys.pin(&origin, *k);
                }
                if let (Some((count, head)), false) = (head, logged.is_empty()) {
                    let _ = self.keys.set_log_head(&origin, count, head);
                }
                let key_id = sig.key_id.clone();
                match logged.into_iter().find(|k| k.key_id() == sig.key_id) {
                    Some(k) => k,
                    None if published => return Err(Restriction::NotInLog { key_id }),
                    None => return Err(Restriction::KeyNotPublished { key_id }),
                }
            }
            None if self.keys.is_successor(&origin, &sig.key_id) => {
                // Rotation (SPEC-7.4.4): the announced key comes from the
                // key file alone; DNS is not asked again.
                let published = self
                    .discover_keys(address, reach, requests)
                    .map_err(|e| Restriction::KeyFileUnavailable(e.to_string()))?;
                let key = published
                    .into_iter()
                    .find(|k| k.key_id() == sig.key_id)
                    .ok_or_else(|| Restriction::SuccessorNotPublished {
                        key_id: sig.key_id.clone(),
                    })?;
                // A served log must list the new key (SPEC-7.6.2).
                let (logged, head) = self.logged_keys(address, reach, vec![key], requests)?;
                if logged.is_empty() {
                    return Err(Restriction::NotInLog {
                        key_id: sig.key_id.clone(),
                    });
                }
                // The signature decides before the store changes: a
                // manifest that names the successor but is not signed by
                // it must not retire the old key.
                if manifest.verify(&key).is_none() {
                    return Err(Restriction::BadSignature);
                }
                let _ = self.keys.promote(&origin, key);
                if let Some((count, head)) = head {
                    let _ = self.keys.set_log_head(&origin, count, head);
                }
                key
            }
            None => {
                return Err(Restriction::UnknownKey {
                    key_id: sig.key_id.clone(),
                })
            }
        };
        let trust = manifest.verify(&key).ok_or(Restriction::BadSignature)?;
        if let Some(next) = &manifest.next_key_id {
            let _ = self.keys.announce_successor(&origin, next);
        }
        Ok(trust)
    }

    /// The keys a first visit may pin (SPEC-7.4.2, SPEC-7.4.5): those in
    /// `/.well-known/vy-key` that a `v=vy1` TXT record at `_vy.<host>`
    /// also names. Loopback has no DNS (SPEC-7.1.3) and takes the key file
    /// alone. A missing record, a disagreement or a failed lookup is a
    /// restriction: nothing is pinned, the next visit asks again.
    fn first_visit_keys(
        &mut self,
        address: &Address,
        reach: Reach,
        requests: &mut u32,
    ) -> Result<Vec<PublicKey>, Restriction> {
        let published = self
            .discover_keys(address, reach, requests)
            .map_err(|e| Restriction::KeyFileUnavailable(e.to_string()))?;
        if address.is_localhost() {
            return Ok(published);
        }
        let name = dns_txt_name(address.host());
        let records = self
            .resolver
            .txt(&name)
            .map_err(|error| Restriction::DnsFailed {
                name: name.clone(),
                error,
            })?;
        let in_dns: Vec<PublicKey> = records.iter().filter_map(|r| parse_dns_txt(r)).collect();
        if in_dns.is_empty() {
            return Err(Restriction::DnsNoRecord { name });
        }
        let agreed: Vec<PublicKey> = published
            .into_iter()
            .filter(|k| in_dns.contains(k))
            .collect();
        if agreed.is_empty() {
            return Err(Restriction::DnsMismatch { name });
        }
        Ok(agreed)
    }

    /// Of `candidates`, the keys the origin's key log lists for its host,
    /// with the log's length and head (SPEC-7.6.2); all of them and no
    /// head when the origin serves no log and never did (SPEC-7.6.3). A
    /// log that cannot be fetched, does not hold, or no longer starts with
    /// the entries seen before is a restriction.
    fn logged_keys(
        &mut self,
        address: &Address,
        reach: Reach,
        candidates: Vec<PublicKey>,
        requests: &mut u32,
    ) -> Result<(Vec<PublicKey>, Option<LogHead>), Restriction> {
        let origin = address.origin();
        let seen = self.keys.log_head(&origin);
        let log = match self.fetch_log(address, reach, requests) {
            Err(e) => return Err(Restriction::LogInvalid(e)),
            Ok(None) if seen.is_some() => return Err(Restriction::LogWithdrawn),
            Ok(None) => return Ok((candidates, None)),
            Ok(Some(log)) => log,
        };
        if let Some((count, head)) = seen {
            let fits = usize::try_from(count).is_ok_and(|n| log.extends(n, &head));
            if !fits {
                return Err(Restriction::LogRewritten);
            }
        }
        let host = origin.strip_prefix("vy://").unwrap_or(&origin);
        let listed = candidates
            .into_iter()
            .filter(|k| log.lists(host, k))
            .collect();
        Ok((listed, Some((log.entries.len() as u64, log.head()))))
    }

    /// `GET /.well-known/vy-log` (SPEC-7.6.1): `None` for a 404, the
    /// chain-checked log for a 200, the reason otherwise.
    fn fetch_log(
        &self,
        address: &Address,
        reach: Reach,
        requests: &mut u32,
    ) -> Result<Option<KeyLog>, String> {
        let base = address.fetch_url();
        let url = resolve(&base, WELL_KNOWN_LOG_PATH).ok_or_else(|| format!("bad base {base}"))?;
        let resp = self
            .get(
                &url,
                "text/plain",
                &|_| MAX_LOG_BYTES as u64,
                None,
                reach,
                requests,
            )
            .map_err(|e| e.to_string())?;
        match resp.status {
            404 => Ok(None),
            200 => KeyLog::parse(&String::from_utf8_lossy(&resp.body))
                .map(Some)
                .map_err(|e| e.to_string()),
            status => Err(format!("status {status}")),
        }
    }

    /// `GET /.well-known/vy-key` (SPEC-7.4.2).
    fn discover_keys(
        &self,
        address: &Address,
        reach: Reach,
        requests: &mut u32,
    ) -> Result<Vec<vy_manifest::PublicKey>, Error> {
        let base = address.fetch_url();
        let url = resolve(&base, WELL_KNOWN_KEY_PATH)
            .ok_or_else(|| Error::BadRedirect { url: base.clone() })?;
        let resp = self.get(
            &url,
            "text/plain",
            &|_| MAX_KEY_FILE_BYTES,
            None,
            reach,
            requests,
        )?;
        if resp.status != 200 {
            return Err(Error::Status {
                url: resp.url,
                status: resp.status,
            });
        }
        let text = String::from_utf8_lossy(&resp.body);
        Ok(parse_key_file(&text)?)
    }
}
