//! `vy-serve`: a static server for Vy packages (SPEC §17, M3).
//!
//! It serves a directory of `.vyp` packages. `GET /<path>` answers with
//! the whole package or the bare manifest depending on `Accept`
//! (SPEC-7.2.1), with the module and resources available by hash under
//! `/_vy/<sha256>` as immutable blobs (SPEC-7.3.1), `ETag` and `max-age`
//! on manifests (SPEC-7.3.2), the publisher key at `/.well-known/vy-key`
//! (SPEC-7.4.2), the DNS TXT records the operator must publish for it at
//! `/.well-known/vy-dns` (SPEC-7.4.5, plain text for people), an optional `index.json` under `/.well-known/vy-index/`,
//! and, for ordinary browsers, an HTML rendering of the manifest's
//! `fallback`, an index of every package at `/` and preview images from
//! `<dir>/previews/<name>.png` at `/_previews/<name>.png`. No trust lives
//! here: everything a client believes comes from the signature (SPEC-7.4.1).
//!
//! [`Site::reply`] is a pure function of the request, so the routing is
//! tested without sockets; [`start`] wraps it in `tiny_http`.
#![deny(missing_docs)]

mod html;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::SystemTime;
use vy_manifest::{
    dns_txt_zone_line, parse_key_file, Hash, Manifest, Package, PublicKey, MEDIA_MANIFEST,
    MEDIA_PACKAGE,
};

pub use html::{index as index_page, markdown, page as fallback_page, Entry};

/// Path of the publisher key file (SPEC-7.4.2).
pub const WELL_KNOWN_KEY_PATH: &str = "/.well-known/vy-key";
/// Path of the DNS TXT records the operator must publish for the keys
/// (SPEC-7.4.5), as plain text for the operator; no client reads it.
pub const WELL_KNOWN_DNS_PATH: &str = "/.well-known/vy-dns";
/// Prefix of the text index route.
pub const WELL_KNOWN_INDEX_PREFIX: &str = "/.well-known/vy-index/";
/// Prefix of content-addressed blobs.
pub const BLOB_PREFIX: &str = "/_vy/";
/// Prefix of preview images: `/_previews/<name>.png` is the file
/// `<dir>/previews/<name>.png`, rendered by `vy-cli` when the site was
/// published.
pub const PREVIEW_PREFIX: &str = "/_previews/";
/// Subdirectory of `dir` holding the preview images.
pub const PREVIEW_DIR: &str = "previews";
/// `Cache-Control` of manifests, packages and HTML (SPEC-7.3.2).
pub const CACHE_CONTROL_PAGE: &str = "max-age=60";
/// `Cache-Control` of blobs: the URL names the bytes (SPEC-7.3.1).
pub const CACHE_CONTROL_BLOB: &str = "public, max-age=31536000, immutable";
/// `Cache-Control` of preview images: one hour.
pub const CACHE_CONTROL_PREVIEW: &str = "public, max-age=3600";
/// File extension of packages.
pub const PACKAGE_EXTENSION: &str = "vyp";
/// Worker threads taking connections.
pub const WORKERS: usize = 4;

/// Server settings.
#[derive(Clone, Debug)]
pub struct Config {
    /// Directory of `.vyp` files.
    pub dir: PathBuf,
    /// Address to bind.
    pub bind: String,
    /// Port; 0 picks a free one.
    pub port: u16,
    /// Host name (with port when not 443) used in `vy://` links; defaults
    /// to `localhost:<port>`.
    pub host_name: Option<String>,
    /// A `vy-key` text file served at `/.well-known/vy-key`.
    pub public_key_file: Option<PathBuf>,
}

impl Config {
    /// Serve `dir` on an ephemeral loopback port.
    pub fn local(dir: impl Into<PathBuf>) -> Self {
        Config {
            dir: dir.into(),
            bind: "127.0.0.1".into(),
            port: 0,
            host_name: None,
            public_key_file: None,
        }
    }
}

/// Why the server could not start.
#[derive(Debug)]
pub enum Error {
    /// The directory or key file could not be read.
    Io(String),
    /// The key file is not `vy-key` text.
    KeyFile(String),
    /// The socket could not be bound.
    Bind(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::KeyFile(e) => write!(f, "public key file: {e}"),
            Self::Bind(e) => write!(f, "cannot bind: {e}"),
        }
    }
}

impl std::error::Error for Error {}

/// Request counts by outcome.
#[derive(Debug, Default)]
pub struct Stats {
    /// Whole packages served.
    pub packages: AtomicU64,
    /// Bare manifests served.
    pub manifests: AtomicU64,
    /// Blobs served.
    pub blobs: AtomicU64,
    /// HTML pages served.
    pub html: AtomicU64,
    /// Key files served (`vy-key`, and `vy-dns` for the operator).
    pub keys: AtomicU64,
    /// Index files served.
    pub index: AtomicU64,
    /// Preview images served.
    pub previews: AtomicU64,
    /// 304 answers.
    pub not_modified: AtomicU64,
    /// 404 answers.
    pub not_found: AtomicU64,
    /// Everything else (405, 400).
    pub rejected: AtomicU64,
}

/// A copy of [`Stats`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StatsSnapshot {
    /// Whole packages served.
    pub packages: u64,
    /// Bare manifests served.
    pub manifests: u64,
    /// Blobs served.
    pub blobs: u64,
    /// HTML pages served.
    pub html: u64,
    /// Key files served (`vy-key`, and `vy-dns` for the operator).
    pub keys: u64,
    /// Index files served.
    pub index: u64,
    /// Preview images served.
    pub previews: u64,
    /// 304 answers.
    pub not_modified: u64,
    /// 404 answers.
    pub not_found: u64,
    /// Everything else.
    pub rejected: u64,
}

impl Stats {
    /// Current counts.
    pub fn snapshot(&self) -> StatsSnapshot {
        let get = |a: &AtomicU64| a.load(Ordering::Relaxed);
        StatsSnapshot {
            packages: get(&self.packages),
            manifests: get(&self.manifests),
            blobs: get(&self.blobs),
            html: get(&self.html),
            keys: get(&self.keys),
            index: get(&self.index),
            previews: get(&self.previews),
            not_modified: get(&self.not_modified),
            not_found: get(&self.not_found),
            rejected: get(&self.rejected),
        }
    }
}

/// What a request looks like to the router.
#[derive(Clone, Debug)]
pub struct RequestInfo {
    /// `GET`, `HEAD`, …
    pub method: String,
    /// Path with query as sent.
    pub url: String,
    /// `Accept` header.
    pub accept: Option<String>,
    /// `If-None-Match` header.
    pub if_none_match: Option<String>,
    /// `X-Forwarded-Host` or `Host` header, used for the host in `vy://`
    /// links when no host name was configured (SPEC-7.1.1: the address a
    /// browser reached us on is the address a Vy client uses).
    pub host: Option<String>,
}

/// What the router decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// HTTP status.
    pub status: u16,
    /// Headers in order.
    pub headers: Vec<(String, String)>,
    /// Body (empty for 304 and HEAD).
    pub body: Vec<u8>,
}

impl Reply {
    fn new(status: u16, body: Vec<u8>, content_type: &str) -> Self {
        Reply {
            status,
            headers: vec![("Content-Type".into(), content_type.into())],
            body,
        }
    }

    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Value of a header, if present.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// The outcome counted in [`Stats`].
    fn count(&self, kind: Counted, stats: &Stats) {
        let counter = match (self.status, kind) {
            (304, _) => &stats.not_modified,
            (404, _) => &stats.not_found,
            (200, Counted::Package) => &stats.packages,
            (200, Counted::Manifest) => &stats.manifests,
            (200, Counted::Blob) => &stats.blobs,
            (200, Counted::Html) => &stats.html,
            (200, Counted::Keys) => &stats.keys,
            (200, Counted::Index) => &stats.index,
            (200, Counted::Preview) => &stats.previews,
            _ => &stats.rejected,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug)]
enum Counted {
    Package,
    Manifest,
    Blob,
    Html,
    Keys,
    Index,
    Preview,
    None,
}

/// A loaded package: both response forms with their ETags.
struct Loaded {
    modified: Option<SystemTime>,
    manifest: Manifest,
    identity: Hash,
    package_bytes: Arc<Vec<u8>>,
    package_etag: String,
    manifest_bytes: Arc<Vec<u8>>,
    manifest_etag: String,
}

/// A blob by hash.
#[derive(Clone)]
struct Blob {
    mime: String,
    bytes: Arc<Vec<u8>>,
}

/// The directory of packages and everything derived from it.
pub struct Site {
    dir: PathBuf,
    host_name: Mutex<String>,
    host_fixed: bool,
    key_text: Option<String>,
    keys: Vec<PublicKey>,
    loaded: Mutex<HashMap<String, Arc<Loaded>>>,
    blobs: Mutex<HashMap<Hash, Blob>>,
    stats: Stats,
}

fn etag_of(bytes: &[u8]) -> String {
    format!("\"{}\"", Hash::of(bytes).hex())
}

/// Decode `%XX` escapes; `None` for malformed escapes or NUL.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            let v = u8::from_str_radix(hex, 16).ok()?;
            if v == 0 {
                return None;
            }
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Turn a request path into a package name relative to the directory:
/// `/` and `/a/` mean `index`; `..`, empty segments and odd characters
/// are refused.
fn package_name(path: &str) -> Option<String> {
    let decoded = percent_decode(path)?;
    let rest = decoded.strip_prefix('/')?;
    let mut segments: Vec<&str> = rest.split('/').collect();
    if segments.last() == Some(&"") {
        segments.pop();
        segments.push("index");
    }
    for s in &segments {
        if s.is_empty()
            || *s == "."
            || *s == ".."
            || s.starts_with('.')
            || s.bytes()
                .any(|b| b.is_ascii_control() || b == b'\\' || b == b':')
        {
            return None;
        }
    }
    Some(segments.join("/"))
}

impl Site {
    /// Open `dir`, read the key file if any and load every package found.
    pub fn open(config: &Config) -> Result<Site, Error> {
        let (key_text, keys) = match &config.public_key_file {
            Some(p) => {
                let text = std::fs::read_to_string(p)
                    .map_err(|e| Error::Io(format!("{}: {e}", p.display())))?;
                let keys = parse_key_file(&text).map_err(|e| Error::KeyFile(e.to_string()))?;
                if keys.is_empty() {
                    return Err(Error::KeyFile("no keys in file".into()));
                }
                (Some(text), keys)
            }
            None => (None, Vec::new()),
        };
        if !config.dir.is_dir() {
            return Err(Error::Io(format!(
                "{} is not a directory",
                config.dir.display()
            )));
        }
        let site = Site {
            dir: config.dir.clone(),
            host_name: Mutex::new(config.host_name.clone().unwrap_or_default()),
            host_fixed: config.host_name.is_some(),
            key_text,
            keys,
            loaded: Mutex::new(HashMap::new()),
            blobs: Mutex::new(HashMap::new()),
            stats: Stats::default(),
        };
        site.scan(&config.dir.clone(), "")?;
        Ok(site)
    }

    fn scan(&self, dir: &Path, prefix: &str) -> Result<(), Error> {
        let entries =
            std::fs::read_dir(dir).map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if path.is_dir() {
                if !name.starts_with('.') {
                    self.scan(&path, &format!("{prefix}{name}/"))?;
                }
            } else if let Some(stem) = name.strip_suffix(&format!(".{PACKAGE_EXTENSION}")) {
                let _ = self.load(&format!("{prefix}{stem}"));
            }
        }
        Ok(())
    }

    /// Request counters.
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Names of the loaded packages.
    pub fn packages(&self) -> Vec<String> {
        let mut names: Vec<String> = self.loaded.lock().unwrap().keys().cloned().collect();
        names.sort();
        names
    }

    fn set_host_name(&self, name: String) {
        *self.host_name.lock().unwrap() = name;
    }

    /// The host written into `vy://` links for this request: the configured
    /// name when one was given, else the request's own `Host` when it is a
    /// plausible host name (letters, digits, `.`, `-`, `:`, brackets; no
    /// spaces, no empty label), else the fallback `localhost:<port>`.
    fn host_for(&self, req: &RequestInfo) -> String {
        if !self.host_fixed {
            if let Some(h) = req.host.as_deref() {
                let h = h.trim();
                let ok = !h.is_empty()
                    && h.len() <= 253
                    && h.bytes().all(|b| {
                        b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b':' | b'[' | b']')
                    })
                    && !h.starts_with('.')
                    && !h.starts_with('-');
                if ok {
                    return h.to_string();
                }
            }
        }
        self.host_name.lock().unwrap().clone()
    }

    /// The zone-file lines of the TXT records a client's first visit
    /// needs (SPEC-7.4.5), one per published key, for `host` (with or
    /// without a port; the DNS name has none).
    pub fn dns_zone_lines(&self, host: &str) -> Vec<String> {
        let hostname = hostname_of(host);
        self.keys
            .iter()
            .map(|k| dns_txt_zone_line(hostname, k))
            .collect()
    }

    /// The text served at [`WELL_KNOWN_DNS_PATH`] for `host`.
    fn dns_text(&self, host: &str) -> String {
        let mut out = format!(
            "; DNS TXT records for vy://{host} (SPEC-7.4.2, SPEC-7.4.5).\n\
             ; Publish every line at the DNS provider: a client's first visit trusts\n\
             ; only keys that stand both here and in {WELL_KNOWN_KEY_PATH}; without\n\
             ; the record every page of the site runs in restricted mode (SPEC-7.4.3).\n"
        );
        if is_loopback_host(host) {
            out.push_str("; Loopback has no DNS and needs no record (SPEC-7.4.5).\n");
        }
        for line in self.dns_zone_lines(host) {
            out.push_str(&line);
            out.push('\n');
        }
        out
    }

    fn package_path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.{PACKAGE_EXTENSION}"))
    }

    fn preview_path(&self, name: &str) -> PathBuf {
        self.dir.join(PREVIEW_DIR).join(format!("{name}.png"))
    }

    /// URL path of the preview of package `name` when the file exists.
    fn preview_url(&self, name: &str) -> Option<String> {
        self.preview_path(name)
            .is_file()
            .then(|| format!("{PREVIEW_PREFIX}{name}.png"))
    }

    /// Every package in name order, (re)loaded, for the index page: the
    /// directory is scanned again so a package published after start is
    /// listed, like it is served by name.
    fn entries(&self) -> Vec<(String, Arc<Loaded>)> {
        let _ = self.scan(&self.dir, "");
        self.packages()
            .into_iter()
            .filter_map(|name| self.load(&name).map(|l| (name, l)))
            .collect()
    }

    /// The package `name`, (re)loaded when its file changed.
    fn load(&self, name: &str) -> Option<Arc<Loaded>> {
        let path = self.package_path(name);
        let modified = std::fs::metadata(&path).ok()?.modified().ok();
        if let Some(l) = self.loaded.lock().unwrap().get(name) {
            if l.modified == modified {
                return Some(Arc::clone(l));
            }
        }
        let bytes = std::fs::read(&path).ok()?;
        let package = match Package::unpack(&bytes) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("vy-serve: {}: {e}", path.display());
                return None;
            }
        };
        let manifest_bytes = package.manifest.encode().ok()?;
        {
            let mut blobs = self.blobs.lock().unwrap();
            blobs.insert(
                package.manifest.module.hash,
                Blob {
                    mime: vy_manifest::MIME_WASM.into(),
                    bytes: Arc::new(package.module.clone()),
                },
            );
            for (r, b) in package.manifest.resources.iter().zip(&package.resources) {
                blobs.insert(
                    r.hash,
                    Blob {
                        mime: r.mime.clone(),
                        bytes: Arc::new(b.clone()),
                    },
                );
            }
        }
        let loaded = Arc::new(Loaded {
            modified,
            identity: package.identity(),
            manifest: package.manifest,
            package_etag: etag_of(&bytes),
            package_bytes: Arc::new(bytes),
            manifest_etag: etag_of(&manifest_bytes),
            manifest_bytes: Arc::new(manifest_bytes),
        });
        self.loaded
            .lock()
            .unwrap()
            .insert(name.to_string(), Arc::clone(&loaded));
        Some(loaded)
    }

    /// Route one request.
    pub fn reply(&self, req: &RequestInfo) -> Reply {
        let (reply, kind) = self.route(req);
        reply.count(kind, &self.stats);
        if req.method == "HEAD" {
            Reply {
                body: Vec::new(),
                ..reply
            }
        } else {
            reply
        }
    }

    fn route(&self, req: &RequestInfo) -> (Reply, Counted) {
        if req.method != "GET" && req.method != "HEAD" {
            return (
                Reply::new(
                    405,
                    b"method not allowed\n".to_vec(),
                    "text/plain; charset=utf-8",
                )
                .header("Allow", "GET, HEAD"),
                Counted::None,
            );
        }
        let path = req.url.split('?').next().unwrap_or("/");
        let accept = req.accept.as_deref().unwrap_or("");

        if path == WELL_KNOWN_KEY_PATH {
            return match &self.key_text {
                Some(text) => (
                    Reply::new(200, text.as_bytes().to_vec(), "text/plain; charset=utf-8")
                        .header("Cache-Control", CACHE_CONTROL_PAGE),
                    Counted::Keys,
                ),
                None => (self.not_found(path, accept), Counted::None),
            };
        }
        if path == WELL_KNOWN_DNS_PATH {
            if self.keys.is_empty() {
                return (self.not_found(path, accept), Counted::None);
            }
            let text = self.dns_text(&self.host_for(req));
            return (
                Reply::new(200, text.into_bytes(), "text/plain; charset=utf-8")
                    .header("Cache-Control", CACHE_CONTROL_PAGE),
                Counted::Keys,
            );
        }
        if let Some(rest) = path.strip_prefix(WELL_KNOWN_INDEX_PREFIX) {
            let Some(name) = package_name(&format!("/{rest}")) else {
                return (bad_request(), Counted::None);
            };
            let dir = match name.rfind('/') {
                Some(i) => self.dir.join(&name[..i]),
                None => self.dir.clone(),
            };
            return match std::fs::read(dir.join("index.json")) {
                Ok(body) => (
                    Reply::new(200, body, "application/json")
                        .header("Cache-Control", CACHE_CONTROL_PAGE),
                    Counted::Index,
                ),
                Err(_) => (self.not_found(path, accept), Counted::None),
            };
        }
        if let Some(hex) = path.strip_prefix(BLOB_PREFIX) {
            let Some(hash) = vy_manifest::from_hex::<32>(hex).map(Hash) else {
                return (bad_request(), Counted::None);
            };
            let blob = self.blobs.lock().unwrap().get(&hash).cloned();
            return match blob {
                Some(b) => {
                    let etag = format!("\"{}\"", hash.hex());
                    if req.if_none_match.as_deref() == Some(etag.as_str()) {
                        return (not_modified(&etag), Counted::Blob);
                    }
                    (
                        Reply::new(200, b.bytes.as_ref().clone(), &b.mime)
                            .header("ETag", &etag)
                            .header("Cache-Control", CACHE_CONTROL_BLOB),
                        Counted::Blob,
                    )
                }
                None => (self.not_found(path, accept), Counted::None),
            };
        }

        if let Some(rest) = path.strip_prefix(PREVIEW_PREFIX) {
            let Some(stem) = rest.strip_suffix(".png") else {
                return (self.not_found(path, accept), Counted::None);
            };
            let Some(name) = package_name(&format!("/{stem}")) else {
                return (bad_request(), Counted::None);
            };
            return match std::fs::read(self.preview_path(&name)) {
                Ok(body) => {
                    let etag = etag_of(&body);
                    if req.if_none_match.as_deref() == Some(etag.as_str()) {
                        return (not_modified(&etag), Counted::Preview);
                    }
                    (
                        Reply::new(200, body, "image/png")
                            .header("ETag", &etag)
                            .header("Cache-Control", CACHE_CONTROL_PREVIEW),
                        Counted::Preview,
                    )
                }
                Err(_) => (self.not_found(path, accept), Counted::None),
            };
        }

        let Some(name) = package_name(path) else {
            return (bad_request(), Counted::None);
        };
        let wants_vy = accepts(accept, MEDIA_MANIFEST) || accepts(accept, MEDIA_PACKAGE);
        if path == "/" && !wants_vy {
            let entries = self.entries();
            let entries: Vec<Entry<'_>> = entries
                .iter()
                .map(|(name, l)| Entry {
                    name,
                    manifest: &l.manifest,
                    preview: self.preview_url(name),
                })
                .collect();
            let html = html::index(&self.host_for(req), &entries);
            return (
                Reply::new(200, html.into_bytes(), "text/html; charset=utf-8")
                    .header("Cache-Control", CACHE_CONTROL_PAGE)
                    .header("Vary", "Accept"),
                Counted::Html,
            );
        }
        let Some(loaded) = self.load(&name) else {
            return (self.not_found(path, accept), Counted::None);
        };
        let (kind, etag, body, media) = if accepts(accept, MEDIA_MANIFEST) {
            (
                Counted::Manifest,
                &loaded.manifest_etag,
                &loaded.manifest_bytes,
                MEDIA_MANIFEST,
            )
        } else if accepts(accept, MEDIA_PACKAGE) {
            (
                Counted::Package,
                &loaded.package_etag,
                &loaded.package_bytes,
                MEDIA_PACKAGE,
            )
        } else {
            let html = html::page(
                &loaded.manifest,
                &self.vy_address(req, path),
                &loaded.identity,
                self.preview_url(&name).as_deref(),
            );
            return (
                Reply::new(200, html.into_bytes(), "text/html; charset=utf-8")
                    .header("Cache-Control", CACHE_CONTROL_PAGE)
                    .header("Vary", "Accept"),
                Counted::Html,
            );
        };
        if req.if_none_match.as_deref() == Some(etag.as_str()) {
            return (not_modified(etag).header("Vary", "Accept"), kind);
        }
        (
            Reply::new(200, body.as_ref().clone(), media)
                .header("ETag", etag)
                .header("Cache-Control", CACHE_CONTROL_PAGE)
                .header("Vary", "Accept"),
            kind,
        )
    }

    fn vy_address(&self, req: &RequestInfo, path: &str) -> String {
        format!("vy://{}{path}", self.host_for(req))
    }

    fn not_found(&self, path: &str, accept: &str) -> Reply {
        if accepts(accept, "text/html") {
            Reply::new(
                404,
                html::not_found(path).into_bytes(),
                "text/html; charset=utf-8",
            )
        } else {
            Reply::new(404, b"not found\n".to_vec(), "text/plain; charset=utf-8")
        }
    }
}

/// True for the loopback names a client exempts from TLS (SPEC-7.1.3)
/// and DNS (SPEC-7.4.5), with or without a port: `localhost`,
/// `127.0.0.1`, `[::1]`.
pub fn is_loopback_host(host: &str) -> bool {
    matches!(
        hostname_of(host).to_ascii_lowercase().as_str(),
        "localhost" | "127.0.0.1" | "[::1]"
    )
}

/// The host name without its port: `exempel.se:8443` is `exempel.se`,
/// `[::1]:8080` is `[::1]`.
fn hostname_of(host: &str) -> &str {
    if host.starts_with('[') {
        host.find(']').map_or(host, |i| &host[..=i])
    } else {
        host.split(':').next().unwrap_or(host)
    }
}

fn bad_request() -> Reply {
    Reply::new(400, b"bad request\n".to_vec(), "text/plain; charset=utf-8")
}

fn not_modified(etag: &str) -> Reply {
    Reply {
        status: 304,
        headers: vec![("ETag".into(), etag.into())],
        body: Vec::new(),
    }
}

/// True when the `Accept` media range list names `media` exactly, with a
/// quality above zero: `type;q=0` means "not acceptable" (RFC 9110 §12.4.2),
/// so a client that lists a type only to refuse it is not served it.
pub fn accepts(accept: &str, media: &str) -> bool {
    accept.split(',').any(|range| {
        let mut parts = range.split(';');
        let named = parts
            .next()
            .unwrap_or("")
            .trim()
            .eq_ignore_ascii_case(media);
        let refused = parts.any(|p| {
            let p = p.trim();
            p.len() > 2
                && p[..2].eq_ignore_ascii_case("q=")
                && p[2..]
                    .trim()
                    .parse::<f32>()
                    .map(|q| q <= 0.0)
                    .unwrap_or(false)
        });
        named && !refused
    })
}

/// A running server.
pub struct Handle {
    server: Arc<tiny_http::Server>,
    site: Arc<Site>,
    port: u16,
    workers: Vec<JoinHandle<()>>,
}

impl Handle {
    /// The bound port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The site, for its statistics.
    pub fn site(&self) -> &Site {
        &self.site
    }

    /// Request counts so far.
    pub fn stats(&self) -> StatsSnapshot {
        self.site.stats.snapshot()
    }

    /// Stop taking connections and join the workers.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        // `unblock` wakes one blocked `recv`; once per worker.
        for _ in 0..self.workers.len() {
            self.server.unblock();
        }
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Start serving `config` on [`WORKERS`] threads. Each request is logged
/// to stderr as one line when `log` is true.
pub fn start(config: Config, log: bool) -> Result<Handle, Error> {
    let site = Arc::new(Site::open(&config)?);
    let server = tiny_http::Server::http(format!("{}:{}", config.bind, config.port))
        .map_err(|e| Error::Bind(e.to_string()))?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .unwrap_or(config.port);
    site.set_host_name(
        config
            .host_name
            .clone()
            .unwrap_or_else(|| format!("localhost:{port}")),
    );
    let server = Arc::new(server);
    let workers = (0..WORKERS)
        .map(|_| {
            let server = Arc::clone(&server);
            let site = Arc::clone(&site);
            std::thread::spawn(move || {
                while let Ok(request) = server.recv() {
                    serve_one(&site, request, log);
                }
            })
        })
        .collect();
    Ok(Handle {
        server,
        site,
        port,
        workers,
    })
}

fn serve_one(site: &Site, request: tiny_http::Request, log: bool) {
    let header = |name: &'static str| -> Option<String> {
        request
            .headers()
            .iter()
            .find(|h| h.field.equiv(name))
            .map(|h| h.value.to_string())
    };
    let info = RequestInfo {
        method: request.method().as_str().to_string(),
        url: request.url().to_string(),
        accept: header("Accept"),
        if_none_match: header("If-None-Match"),
        host: header("X-Forwarded-Host").or_else(|| header("Host")),
    };
    let reply = site.reply(&info);
    if log {
        eprintln!(
            "{} {} {} {}",
            info.method,
            info.url,
            reply.status,
            reply.body.len()
        );
    }
    let mut response = tiny_http::Response::from_data(reply.body)
        .with_status_code(tiny_http::StatusCode(reply.status));
    for (name, value) in &reply.headers {
        if let Ok(h) = tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response = response.with_header(h);
        }
    }
    let _ = request.respond(response);
}

#[cfg(test)]
mod tests {
    use super::*;
    use vy_manifest::{Draft, ResourceInput, SigningKey};

    const SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];

    fn module() -> Vec<u8> {
        wat::parse_str(
            r#"(module (memory (export "memory") 2)
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32) i32.const 0)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
        )
        .unwrap()
    }

    fn png() -> Vec<u8> {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        b.extend_from_slice(&13u32.to_be_bytes());
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&8u32.to_be_bytes());
        b.extend_from_slice(&4u32.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        b
    }

    /// A directory with `hello.vyp`, `docs/intro.vyp`, `docs/index.json`,
    /// a preview for `hello` only, and a key file.
    fn site_dir() -> (PathBuf, Package, SigningKey) {
        let dir = std::env::temp_dir().join(format!(
            "vy-serve-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        let key = SigningKey::from_seed(&SEED);
        let draft =
            Draft::from_json(include_str!("../../../conformance/manifests/hello.json")).unwrap();
        let res = vec![ResourceInput {
            mime: "image/png".into(),
            bytes: png(),
            size_hint: None,
        }];
        let package = vy_publish::build(&draft, module(), res, &key).unwrap();
        std::fs::write(dir.join("hello.vyp"), package.pack().unwrap()).unwrap();
        std::fs::write(dir.join("docs/intro.vyp"), package.pack().unwrap()).unwrap();
        std::fs::write(dir.join("docs/index.json"), b"{\"pages\":[\"intro\"]}").unwrap();
        std::fs::write(dir.join("docs/notes.txt"), b"not a package").unwrap();
        std::fs::create_dir_all(dir.join(PREVIEW_DIR)).unwrap();
        std::fs::write(dir.join(PREVIEW_DIR).join("hello.png"), png()).unwrap();
        std::fs::write(
            dir.join("dev.key.pub"),
            vy_manifest::key_file(&[key.public()]),
        )
        .unwrap();
        (dir, package, key)
    }

    fn get(site: &Site, url: &str, accept: &str) -> Reply {
        site.reply(&RequestInfo {
            method: "GET".into(),
            url: url.into(),
            accept: Some(accept.into()),
            if_none_match: None,
            host: None,
        })
    }

    fn config(dir: &Path) -> Config {
        Config {
            public_key_file: Some(dir.join("dev.key.pub")),
            host_name: Some("vy.example".into()),
            ..Config::local(dir)
        }
    }

    /// SPEC-7.2.1: Accept chooses between the bare manifest and the whole
    /// package; both carry ETag and max-age (SPEC-7.3.2).
    #[test]
    fn negotiates_manifest_or_package() {
        let (dir, package, _) = site_dir();
        let site = Site::open(&config(&dir)).unwrap();
        assert_eq!(site.packages(), ["docs/intro", "hello"]);
        let both = "application/vy-package+zstd, application/vy-manifest+cbor";
        let r = get(&site, "/hello", both);
        assert_eq!(r.status, 200);
        assert_eq!(r.get("content-type"), Some(MEDIA_MANIFEST));
        assert_eq!(r.get("cache-control"), Some(CACHE_CONTROL_PAGE));
        assert_eq!(r.body, package.manifest.encode().unwrap());
        let etag = r.get("etag").unwrap().to_string();
        assert_eq!(etag, etag_of(&r.body));

        let p = get(&site, "/hello?x=1", MEDIA_PACKAGE);
        assert_eq!(p.get("content-type"), Some(MEDIA_PACKAGE));
        assert_eq!(Package::unpack(&p.body).unwrap(), package);
        assert_ne!(p.get("etag"), Some(etag.as_str()));
        // A quality of zero refuses the type (RFC 9110 §12.4.2): the
        // manifest is listed but not wanted, so the package is served.
        let q0 = format!("{MEDIA_MANIFEST};q=0, {MEDIA_PACKAGE};q=1");
        assert_eq!(
            get(&site, "/hello", &q0).get("content-type"),
            Some(MEDIA_PACKAGE)
        );
        assert!(accepts(" Text/HTML ; q=0.8 ,*/*", "text/html"));
        assert!(!accepts("text/html;q=0", "text/html"));
        assert!(!accepts("text/html; Q=0.0", "text/html"));
        assert!(
            accepts("text/html;q=x", "text/html"),
            "unparsable q is ignored"
        );
        assert!(
            !accepts("*/*", "text/html"),
            "no wildcards: the fallback is the default"
        );

        let same = site.reply(&RequestInfo {
            method: "GET".into(),
            url: "/hello".into(),
            accept: Some(both.into()),
            if_none_match: Some(etag),
            host: None,
        });
        assert_eq!(same.status, 304);
        assert!(same.body.is_empty());

        let head = site.reply(&RequestInfo {
            method: "HEAD".into(),
            url: "/hello".into(),
            accept: Some(both.into()),
            if_none_match: None,
            host: None,
        });
        assert_eq!(head.status, 200);
        assert!(head.body.is_empty());
        let s = site.stats().snapshot();
        assert_eq!((s.manifests, s.packages, s.not_modified), (2, 2, 1));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-7.3.1: blobs are served by hash as immutable.
    #[test]
    fn serves_blobs_by_hash() {
        let (dir, package, _) = site_dir();
        let site = Site::open(&config(&dir)).unwrap();
        let module_url = format!("{BLOB_PREFIX}{}", package.manifest.module.hash.hex());
        let r = get(&site, &module_url, "*/*");
        assert_eq!(r.status, 200);
        assert_eq!(r.body, package.module);
        assert_eq!(r.get("content-type"), Some("application/wasm"));
        assert_eq!(r.get("cache-control"), Some(CACHE_CONTROL_BLOB));
        let res_url = format!("{BLOB_PREFIX}{}", package.manifest.resources[0].hash.hex());
        let r = get(&site, &res_url, "*/*");
        assert_eq!(r.get("content-type"), Some("image/png"));
        assert_eq!(r.body, package.resources[0]);
        assert_eq!(get(&site, "/_vy/00", "*/*").status, 400);
        assert_eq!(
            get(&site, &format!("{BLOB_PREFIX}{}", "0".repeat(64)), "*/*").status,
            404
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Ordinary browsers get the fallback as HTML with the vy:// link, the
    /// package identity and the preview image when one exists; nothing
    /// else is ever HTML (SPEC-1.1.1 is the client's rule, the server only
    /// helps browsers find the client).
    #[test]
    fn html_fallback_for_browsers() {
        let (dir, package, _) = site_dir();
        let site = Site::open(&config(&dir)).unwrap();
        let r = get(&site, "/hello", "text/html,application/xhtml+xml,*/*;q=0.8");
        assert_eq!(r.status, 200);
        assert_eq!(r.get("content-type"), Some("text/html; charset=utf-8"));
        let html = String::from_utf8(r.body).unwrap();
        assert!(html.contains("href=\"vy://vy.example/hello\""));
        assert!(html.contains("<h1>Hej från Vy</h1>"));
        assert!(html.contains(&package.identity().to_string()));
        assert!(html.contains("<img src=\"/_previews/hello.png\""));
        assert!(!html.contains("<script"));
        let r = get(&site, "/docs/", "*/*");
        assert_eq!(r.status, 404, "docs/index.vyp does not exist");
        let r = get(&site, "/docs/intro", "*/*");
        assert_eq!(r.status, 200);
        assert!(r.get("content-type").unwrap().starts_with("text/html"));
        let html = String::from_utf8(r.body).unwrap();
        assert!(!html.contains("<img"), "docs/intro has no preview");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `/` lists every package for browsers, with manifest text escaped,
    /// and stays the `index` package for Vy clients.
    /// SPEC-7.1.1: without a configured host name the `vy://` links use the
    /// address the browser reached the server on (`X-Forwarded-Host`, then
    /// `Host`); a configured name wins; a hostile header is ignored.
    #[test]
    fn host_in_links_follows_the_request_unless_configured() {
        let (dir, _package, _key) = site_dir();
        let with = |site: &Site, host: Option<&str>| {
            let r = site.reply(&RequestInfo {
                method: "GET".into(),
                url: "/hello".into(),
                accept: Some("text/html".into()),
                if_none_match: None,
                host: host.map(str::to_string),
            });
            String::from_utf8(r.body).unwrap()
        };
        let open = Site::open(&Config {
            public_key_file: Some(dir.join("dev.key.pub")),
            host_name: None,
            ..Config::local(&dir)
        })
        .unwrap();
        open.set_host_name("localhost:8080".into());
        assert!(with(&open, None).contains("vy://localhost:8080/hello"));
        assert!(with(&open, Some("vy-production.up.railway.app"))
            .contains("vy://vy-production.up.railway.app/hello"));
        assert!(with(&open, Some("exempel.se:8443")).contains("vy://exempel.se:8443/hello"));
        for hostile in ["<script>", "a b", "", ".x", "-x", "x/y", "x\"y"] {
            let html = with(&open, Some(hostile));
            assert!(html.contains("vy://localhost:8080/hello"), "{hostile:?}");
        }
        let fixed = Site::open(&config(&dir)).unwrap();
        assert!(with(&fixed, Some("evil.example")).contains("vy://vy.example/hello"));
    }

    #[test]
    fn index_lists_packages() {
        let (dir, package, key) = site_dir();
        let site = Site::open(&config(&dir)).unwrap();
        let r = get(&site, "/", "text/html,*/*;q=0.8");
        assert_eq!(r.status, 200);
        assert_eq!(r.get("content-type"), Some("text/html; charset=utf-8"));
        assert_eq!(r.get("cache-control"), Some(CACHE_CONTROL_PAGE));
        let html = String::from_utf8(r.body).unwrap();
        assert!(html.contains("<title>Vy – sidor på vy.example</title>"));
        assert!(html.contains("2 sidor"));
        assert!(html.contains("<h2><a href=\"/hello\">Hej från Vy</a></h2>"));
        assert!(html.contains("<h2><a href=\"/docs/intro\">Hej från Vy</a></h2>"));
        assert!(html.contains(&format!("<p>{}</p>", package.manifest.summary)));
        assert!(html
            .contains("Öppna i Vy: <a href=\"vy://vy.example/hello\">vy://vy.example/hello</a>"));
        assert!(html.contains("href=\"vy://vy.example/docs/intro\""));
        assert!(html.contains("<a href=\"/hello\"><img src=\"/_previews/hello.png\""));
        assert_eq!(html.matches("<img").count(), 1, "docs/intro has no preview");
        assert!(html.contains("href=\"/.well-known/vy-key\""));
        assert!(!html.contains("<script"));
        // The query is ignored; HEAD has no body; `/` without an index.vyp
        // is still 404 for a Vy client (SPEC-7.2.1 names no index).
        assert_eq!(
            get(&site, "/?x=<b>", "text/html").body,
            get(&site, "/", "text/html").body
        );
        let head = site.reply(&RequestInfo {
            method: "HEAD".into(),
            url: "/".into(),
            accept: Some("text/html".into()),
            if_none_match: None,
            host: None,
        });
        assert_eq!((head.status, head.body.len()), (200, 0));
        let both = "application/vy-package+zstd, application/vy-manifest+cbor";
        assert_eq!(get(&site, "/", both).status, 404);
        assert_eq!(get(&site, "/", MEDIA_PACKAGE).status, 404);

        // Manifest text is escaped, and a package published after start is
        // listed (the directory is scanned again).
        let mut hostile = package.clone();
        hostile.manifest.name = "<b>Namn</b> & \"citat\"".into();
        hostile.manifest.summary = "<script>alert(1)</script>".into();
        hostile.manifest.sign(&key);
        std::fs::write(dir.join("sen.vyp"), hostile.pack().unwrap()).unwrap();
        let html = String::from_utf8(get(&site, "/", "text/html").body).unwrap();
        assert!(html.contains("3 sidor"));
        assert!(html.contains(
            "<h2><a href=\"/sen\">&lt;b&gt;Namn&lt;/b&gt; &amp; &quot;citat&quot;</a></h2>"
        ));
        assert!(html.contains("<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>"));
        assert!(!html.contains("<script"));
        assert!(!html.contains("<b>"));
        assert_eq!(site.stats().snapshot().html, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Previews are PNG files served under `/_previews/` with a one-hour
    /// cache and an ETag; a missing one is a clean 404 and the path is
    /// checked like a package name, so nothing outside `previews/` is
    /// reachable.
    #[test]
    fn preview_images() {
        let (dir, _, _) = site_dir();
        let site = Site::open(&config(&dir)).unwrap();
        let r = get(&site, "/_previews/hello.png", "image/png,*/*");
        assert_eq!(r.status, 200);
        assert_eq!(r.get("content-type"), Some("image/png"));
        assert_eq!(r.get("cache-control"), Some(CACHE_CONTROL_PREVIEW));
        assert_eq!(r.body, png());
        let etag = r.get("etag").unwrap().to_string();
        assert_eq!(etag, etag_of(&png()));
        let same = site.reply(&RequestInfo {
            method: "GET".into(),
            url: "/_previews/hello.png?v=2".into(),
            accept: None,
            if_none_match: Some(etag),
            host: None,
        });
        assert_eq!((same.status, same.body.len()), (304, 0));

        assert_eq!(get(&site, "/_previews/docs/intro.png", "*/*").status, 404);
        assert_eq!(get(&site, "/_previews/hello.txt", "*/*").status, 404);
        assert_eq!(get(&site, "/_previews/hello", "*/*").status, 404);
        assert_eq!(get(&site, "/_previews/", "*/*").status, 404);
        assert_eq!(
            get(&site, "/_previews/.png", "*/*").status,
            404,
            "`/` is the index package, so its preview is previews/index.png"
        );
        let r = get(&site, "/_previews/nope.png", "text/html");
        assert_eq!(r.status, 404);
        assert!(r.get("content-type").unwrap().starts_with("text/html"));
        assert!(String::from_utf8(r.body)
            .unwrap()
            .contains("/_previews/nope.png"));
        // Traversal and odd names are refused before any file is touched:
        // `dev.key.pub` sits next to `previews/`.
        for bad in [
            "/_previews/../dev.key.pub.png",
            "/_previews/..%2Fdev.key.pub.png",
            "/_previews/%2e%2e/hello.png",
            "/_previews/.hidden.png",
            "/_previews/a\\b.png",
            "/_previews/%00.png",
            "/_previews/docs//intro.png",
        ] {
            assert_eq!(get(&site, bad, "*/*").status, 400, "{bad}");
        }
        let s = site.stats().snapshot();
        assert_eq!(
            (s.previews, s.not_modified, s.not_found, s.rejected),
            (1, 1, 6, 7)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-7.4.2: the key file; the index; refused paths.
    #[test]
    fn well_known_and_paths() {
        let (dir, _, key) = site_dir();
        let site = Site::open(&config(&dir)).unwrap();
        let r = get(&site, WELL_KNOWN_KEY_PATH, "text/plain");
        assert_eq!(r.status, 200);
        assert_eq!(
            parse_key_file(std::str::from_utf8(&r.body).unwrap()).unwrap(),
            vec![key.public()]
        );
        let r = get(&site, "/.well-known/vy-index/docs/intro", "*/*");
        assert_eq!(r.status, 200);
        assert_eq!(r.get("content-type"), Some("application/json"));
        assert_eq!(r.body, b"{\"pages\":[\"intro\"]}");
        assert_eq!(get(&site, "/.well-known/vy-index/hello", "*/*").status, 404);
        for bad in [
            "/../etc/passwd",
            "/docs//intro",
            "/.hidden",
            "/a\\b",
            "/%00",
        ] {
            assert_eq!(get(&site, bad, "*/*").status, 400, "{bad}");
        }
        assert_eq!(get(&site, "/docs/notes", "*/*").status, 404);
        assert_eq!(get(&site, "/docs/intro.vyp", "*/*").status, 404);
        assert_eq!(get(&site, "/hello%2F..%2Fx", "*/*").status, 400);
        let post = site.reply(&RequestInfo {
            method: "POST".into(),
            url: "/hello".into(),
            accept: None,
            if_none_match: None,
            host: None,
        });
        assert_eq!(post.status, 405);
        // Without a key file the route is 404.
        let no_key = Site::open(&Config::local(&dir)).unwrap();
        assert_eq!(get(&no_key, WELL_KNOWN_KEY_PATH, "*/*").status, 404);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-7.4.5: the TXT records the operator must publish are served
    /// as plain text at `/.well-known/vy-dns`, one zone line per key at
    /// `_vy.<host>` without the port, for the configured host name or the
    /// request's; without a key file the route is 404.
    #[test]
    fn spec_7_4_5_dns_records_for_the_operator() {
        let (dir, _, key) = site_dir();
        let site = Site::open(&config(&dir)).unwrap();
        let r = get(&site, WELL_KNOWN_DNS_PATH, "text/plain");
        assert_eq!(r.status, 200);
        assert_eq!(r.get("content-type"), Some("text/plain; charset=utf-8"));
        let text = String::from_utf8(r.body).unwrap();
        let line = format!("_vy.vy.example. IN TXT \"v=vy1; k={}\"", key.public());
        assert!(text.contains(&line), "{text}");
        assert!(text.contains("SPEC-7.4.5"));
        assert_eq!(
            text.lines().filter(|l| !l.starts_with(';')).count(),
            1,
            "one record per key"
        );
        assert_eq!(
            site.dns_zone_lines("vy.example:8443"),
            std::slice::from_ref(&line)
        );
        assert_eq!(site.dns_zone_lines("vy.example"), [line]);
        assert_eq!(
            site.dns_zone_lines("[::1]:8080"),
            [format!("_vy.[::1]. IN TXT \"v=vy1; k={}\"", key.public())]
        );
        assert_eq!(
            vy_manifest::parse_dns_txt(&vy_manifest::dns_txt_record(&key.public())),
            Some(key.public())
        );
        // Without a configured name the request's host is used.
        let open = Site::open(&Config {
            public_key_file: Some(dir.join("dev.key.pub")),
            host_name: None,
            ..Config::local(&dir)
        })
        .unwrap();
        let r = open.reply(&RequestInfo {
            method: "GET".into(),
            url: WELL_KNOWN_DNS_PATH.into(),
            accept: None,
            if_none_match: None,
            host: Some("sajt.example:8443".into()),
        });
        assert!(String::from_utf8(r.body)
            .unwrap()
            .contains("_vy.sajt.example. IN TXT"));
        let no_key = Site::open(&Config::local(&dir)).unwrap();
        assert_eq!(get(&no_key, WELL_KNOWN_DNS_PATH, "*/*").status, 404);
        for (host, loopback) in [
            ("localhost:8080", true),
            ("LOCALHOST", true),
            ("127.0.0.1:1", true),
            ("[::1]:8080", true),
            ("vy.example", false),
            ("localhost.example", false),
        ] {
            assert_eq!(is_loopback_host(host), loopback, "{host}");
        }
        assert_eq!(site.stats().snapshot().keys, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A changed package file is picked up on the next request.
    #[test]
    fn reloads_changed_packages() {
        let (dir, package, key) = site_dir();
        let site = Site::open(&config(&dir)).unwrap();
        assert_eq!(
            get(&site, "/hello", MEDIA_PACKAGE).body,
            package.pack().unwrap()
        );
        let mut changed = package.clone();
        changed.manifest.name = "Ändrad".into();
        changed.manifest.sign(&key);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let path = dir.join("hello.vyp");
        std::fs::write(&path, changed.pack().unwrap()).unwrap();
        // Force a different mtime even on coarse file systems.
        let f = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.set_modified(SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        let r = get(&site, "/hello", MEDIA_MANIFEST);
        assert_eq!(Manifest::decode(&r.body).unwrap().name, "Ändrad");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The real server answers over TCP on an ephemeral port.
    #[test]
    fn serves_over_tcp() {
        use std::io::{Read, Write};
        let (dir, package, _) = site_dir();
        let handle = start(config(&dir), false).unwrap();
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", handle.port())).unwrap();
        write!(
            stream,
            "GET /hello HTTP/1.1\r\nHost: x\r\nAccept: {MEDIA_PACKAGE}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).unwrap();
        let text = String::from_utf8_lossy(&response);
        assert!(text.starts_with("HTTP/1.1 200"), "{text}");
        assert!(text
            .to_ascii_lowercase()
            .contains("content-type: application/vy-package+zstd"));
        let body_at = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert_eq!(Package::unpack(&response[body_at..]).unwrap(), package);
        assert_eq!(handle.stats().packages, 1);
        handle.stop();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
