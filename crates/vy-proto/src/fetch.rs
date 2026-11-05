//! Fetching a page (SPEC §7): translate the address, GET it over HTTPS,
//! accept a whole package or a bare manifest, fill modules and resources
//! from the content-addressed cache, and decide trust from the signature.

use crate::address::Address;
use crate::cache::{now_secs, BlobCache, HttpCache, HttpRecord, DEFAULT_CACHE_CAP_BYTES};
use crate::keys::KeyStore;
use crate::url::{cache_control, media_type, resolve, CacheControl, HttpUrl};
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;
use ureq::config::Config;
use ureq::Agent;
use vy_manifest::{
    default_grant, parse_key_file, Grant, Hash, Manifest, Package, PackageError, Problem,
    MAX_MANIFEST_BYTES, MEDIA_MANIFEST, MEDIA_PACKAGE,
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
    /// Whole-request timeout.
    pub timeout: Duration,
}

impl Options {
    /// Defaults: 2 GB cap, 30 s timeout.
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        Options {
            cache_dir: cache_dir.into(),
            cache_cap_bytes: DEFAULT_CACHE_CAP_BYTES,
            timeout: Duration::from_secs(30),
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
    /// Manifest, module and resources, hashes verified (SPEC-9.4.1).
    pub package: Package,
    /// Package identity (SPEC-4.1.2).
    pub identity: Hash,
    /// True when the signature verifies with a key trusted for the origin;
    /// false means restricted mode: no capabilities (SPEC-7.4.3).
    pub trusted: bool,
    /// Id of the key the manifest names.
    pub key_id: String,
    /// Response form.
    pub form: Form,
    /// Network requests made, 0 when everything came from the caches.
    pub requests: u32,
}

impl Fetched {
    /// The origin (SPEC-9.5.1).
    pub fn origin(&self) -> String {
        self.address.origin()
    }

    /// Default grant per requested capability (SPEC-8.2.3); every one is
    /// [`Grant::Denied`] when the page is not trusted (SPEC-7.4.3).
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
            .iter()
            .map(|r| {
                let grant = if !self.trusted {
                    Grant::Denied
                } else {
                    let own = match r {
                        vy_manifest::Requested::Net(n) => {
                            n.origins.iter().all(|o| own_origins.contains(o))
                        }
                        _ => true,
                    };
                    default_grant(&r, own)
                };
                (r.name(), grant)
            })
            .collect()
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
/// through a redirect or a blob URL.
pub fn transport_allowed(page_is_localhost: bool, url: &HttpUrl) -> bool {
    url.scheme == "https" || (page_is_localhost && url.is_localhost())
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

/// Fetches pages with a content-addressed cache, an HTTP cache for
/// manifests and a per-origin key store.
pub struct Fetcher {
    agent: Agent,
    blobs: BlobCache,
    http: HttpCache,
    keys: KeyStore,
}

impl Fetcher {
    /// Open the caches under `options.cache_dir`.
    pub fn new(options: Options) -> Result<Self, Error> {
        let io = |e: std::io::Error| Error::Io(e.to_string());
        let config = Config::builder()
            // Redirects are followed by hand so every hop is checked for
            // TLS (SPEC-7.1.3) and counted (SPEC-7.5.1).
            .max_redirects(0)
            .http_status_as_error(false)
            // No ambient proxy (SPEC-2.1.4): an environment variable must
            // not redirect a page fetch. Explicit proxy settings are a
            // later option.
            .proxy(None)
            .user_agent("vy-proto/0.1")
            .accept_encoding("")
            .timeout_global(Some(options.timeout))
            .build();
        Ok(Fetcher {
            agent: Agent::new_with_config(config),
            blobs: BlobCache::open(&options.cache_dir, options.cache_cap_bytes).map_err(io)?,
            http: HttpCache::open(&options.cache_dir).map_err(io)?,
            keys: KeyStore::open(&options.cache_dir).map_err(io)?,
        })
    }

    /// The content-addressed cache.
    pub fn blobs(&self) -> &BlobCache {
        &self.blobs
    }

    /// The key store.
    pub fn keys(&self) -> &KeyStore {
        &self.keys
    }

    /// Fetch a page.
    pub fn fetch(&mut self, address: &Address) -> Result<Fetched, Error> {
        let mut requests = 0;
        let local = address.is_localhost();
        let primary = self.get_with_http_cache(&address.fetch_url(), local, &mut requests)?;
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
                    local,
                    &mut requests,
                )?;
                let mut resources = Vec::with_capacity(manifest.resources.len());
                for r in &manifest.resources {
                    resources.push(self.blob(
                        &r.hash,
                        r.size,
                        r.url.as_deref(),
                        &primary.url,
                        local,
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
        let trusted = self.trust(address, &package.manifest, &mut requests);
        Ok(Fetched {
            address: address.clone(),
            identity: package.identity(),
            package,
            trusted,
            key_id,
            form,
            requests,
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
        local: bool,
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
        let resp = self.get(&url, "*/*", &|_| limit, None, local, requests)?;
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
    fn get_with_http_cache(
        &mut self,
        url: &str,
        local: bool,
        requests: &mut u32,
    ) -> Result<Response, Error> {
        let now = now_secs();
        let cached = self.http.get(url);
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
        let resp = self.get(url, ACCEPT, &page_limit, etag.as_deref(), local, requests)?;
        if resp.status == 304 {
            if let Some(mut rec) = cached {
                rec.fetched_at = now;
                if resp.cache.max_age > 0 {
                    rec.max_age = resp.cache.max_age;
                }
                if resp.etag.is_some() {
                    rec.etag = resp.etag.clone();
                }
                let _ = self.http.put(url, &rec);
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
            self.http.remove(url);
        } else {
            let rec = HttpRecord {
                url: resp.url.clone(),
                etag: resp.etag.clone(),
                max_age: resp.cache.max_age,
                fetched_at: now,
                media_type: resp.media_type.clone(),
                body: resp.body.clone(),
            };
            let _ = self.http.put(url, &rec);
        }
        Ok(resp)
    }

    /// One GET with up to [`MAX_REDIRECTS`] hops, each checked with
    /// [`transport_allowed`] before it is connected (SPEC-7.1.3); `local`
    /// says whether the page's own address is loopback. Never sends
    /// `Referer` (SPEC-7.5.1). `limit` gives the body cap for the media
    /// type received; 0 means the body is not read.
    fn get(
        &self,
        url: &str,
        accept: &str,
        limit: &dyn Fn(&str) -> u64,
        if_none_match: Option<&str>,
        local: bool,
        requests: &mut u32,
    ) -> Result<Response, Error> {
        let mut url = url.to_string();
        for hop in 0..=MAX_REDIRECTS {
            let parsed =
                HttpUrl::parse(&url).ok_or_else(|| Error::BadRedirect { url: url.clone() })?;
            if !transport_allowed(local, &parsed) {
                return Err(Error::TlsRequired { url });
            }
            let mut req = self.agent.get(&url).header("Accept", accept);
            if let Some(etag) = if_none_match {
                req = req.header("If-None-Match", etag);
            }
            *requests += 1;
            let mut resp = req.call().map_err(|e| Error::Transport {
                url: url.clone(),
                message: e.to_string(),
            })?;
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
    /// SPEC-7.4.4): the key must be pinned for the origin, or be
    /// discoverable on a first visit, or have been announced as successor.
    fn trust(&mut self, address: &Address, manifest: &Manifest, requests: &mut u32) -> bool {
        let Some(sig) = &manifest.sig else {
            return false;
        };
        let origin = address.origin();
        let key = match self.keys.pinned(&origin, &sig.key_id) {
            Some(k) => Some(k),
            None => {
                let first_visit = !self.keys.has_pins(&origin);
                let successor = self.keys.is_successor(&origin, &sig.key_id);
                if first_visit || successor {
                    match self.discover_keys(address, requests) {
                        Ok(published) => {
                            if first_visit {
                                for k in &published {
                                    let _ = self.keys.pin(&origin, *k);
                                }
                            } else if let Some(k) =
                                published.iter().find(|k| k.key_id() == sig.key_id)
                            {
                                let _ = self.keys.promote(&origin, *k);
                            }
                            self.keys.pinned(&origin, &sig.key_id)
                        }
                        Err(_) => None,
                    }
                } else {
                    None
                }
            }
        };
        let trusted = key.map(|k| manifest.verify(&k)).unwrap_or(false);
        if trusted {
            if let Some(next) = &manifest.next_key_id {
                let _ = self.keys.announce_successor(&origin, next);
            }
        }
        trusted
    }

    /// `GET /.well-known/vy-key` (SPEC-7.4.2). DNS TXT `_vy.<host>` is not
    /// consulted yet.
    fn discover_keys(
        &self,
        address: &Address,
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
            address.is_localhost(),
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
