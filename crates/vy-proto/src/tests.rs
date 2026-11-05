//! Spec tests for §7, against an in-process HTTP server on loopback.

use crate::test_server::{Request, Response, TestServer};
use crate::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use vy_manifest::{
    key_file, Draft, Grant, Hash, Package, PackageError, Problem, ResourceInput, SigningKey,
};

/// RFC 8032 §7.1 test vector 1: a publicly known key, never a secret.
const SEED_A: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];

fn key(seed: u8) -> SigningKey {
    if seed == 0 {
        SigningKey::from_seed(&SEED_A)
    } else {
        SigningKey::from_seed(&[seed; 32])
    }
}

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

fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    b.extend_from_slice(&13u32.to_be_bytes());
    b.extend_from_slice(b"IHDR");
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    b
}

fn draft() -> Draft {
    Draft::from_json(include_str!("../../../conformance/manifests/hello.json")).unwrap()
}

/// A signed package with one PNG resource.
fn package(key: &SigningKey, next_key_id: Option<&str>) -> Package {
    let mut d = draft();
    d.next_key_id = next_key_id.map(str::to_string);
    let res = vec![ResourceInput {
        mime: "image/png".into(),
        bytes: png(32, 16),
        size_hint: None,
    }];
    let wasm = module();
    let mut manifest = d.finish(&wasm, &res);
    manifest.sign(key);
    let p = Package {
        manifest,
        module: wasm,
        resources: res.into_iter().map(|r| r.bytes).collect(),
    };
    assert!(p.validate().is_empty());
    p
}

/// What the test site serves; tests mutate it between fetches.
struct Site {
    packages: HashMap<String, Package>,
    form: Form,
    cache_control: Option<String>,
    etag: bool,
    keys: String,
    /// Bytes to serve for a hash instead of the real ones.
    blob_override: HashMap<Hash, Vec<u8>>,
    /// Paths that redirect: path -> Location.
    redirects: HashMap<String, String>,
    /// Paths served as something else: path -> (content type, body).
    other: HashMap<String, (String, Vec<u8>)>,
}

impl Site {
    fn new(p: Package, form: Form, keys: &[SigningKey]) -> Arc<Mutex<Site>> {
        let mut packages = HashMap::new();
        packages.insert("/hello".to_string(), p);
        Arc::new(Mutex::new(Site {
            packages,
            form,
            cache_control: Some("max-age=60".into()),
            etag: true,
            keys: key_file(&keys.iter().map(|k| k.public()).collect::<Vec<_>>()),
            blob_override: HashMap::new(),
            redirects: HashMap::new(),
            other: HashMap::new(),
        }))
    }
}

fn handle(site: &Arc<Mutex<Site>>, req: &Request) -> Response {
    let site = site.lock().unwrap();
    if let Some(location) = site.redirects.get(&req.path) {
        return Response::new(302).header("Location", location);
    }
    if let Some((ct, body)) = site.other.get(&req.path) {
        return Response::ok(ct, body.clone());
    }
    if req.path == WELL_KNOWN_KEY_PATH {
        return Response::ok("text/plain", site.keys.clone());
    }
    if let Some(hex) = req.path.strip_prefix(BLOB_PATH_PREFIX) {
        let Some(hash) = vy_manifest::from_hex::<32>(hex).map(Hash) else {
            return Response::new(404);
        };
        if let Some(b) = site.blob_override.get(&hash) {
            return Response::ok("application/octet-stream", b.clone());
        }
        for p in site.packages.values() {
            if p.manifest.module.hash == hash {
                return Response::ok("application/wasm", p.module.clone());
            }
            for (r, bytes) in p.manifest.resources.iter().zip(&p.resources) {
                if r.hash == hash {
                    return Response::ok(&r.mime, bytes.clone());
                }
            }
        }
        return Response::new(404);
    }
    let Some(p) = site.packages.get(&req.path) else {
        return Response::new(404);
    };
    let (media, body) = match site.form {
        Form::Manifest => (MEDIA_MANIFEST, p.manifest.encode().unwrap()),
        Form::Package => (MEDIA_PACKAGE, p.pack().unwrap()),
    };
    // The ETag names the body, signature included, so a re-signed page
    // revalidates to new bytes.
    let etag = format!("\"{}\"", Hash::of(&body).hex());
    if site.etag && req.header("if-none-match") == Some(etag.as_str()) {
        return Response::new(304).header("ETag", &etag);
    }
    let mut resp = Response::ok(media, body);
    if site.etag {
        resp = resp.header("ETag", &etag);
    }
    if let Some(cc) = &site.cache_control {
        resp = resp.header("Cache-Control", cc);
    }
    resp
}

fn serve(site: &Arc<Mutex<Site>>) -> TestServer {
    let site = Arc::clone(site);
    TestServer::start(move |req| handle(&site, req))
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vy-proto-test-{name}-{}-{}",
        std::process::id(),
        cache::now_secs()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn fetcher(name: &str) -> (Fetcher, PathBuf) {
    let dir = temp_dir(name);
    (Fetcher::new(Options::new(&dir)).unwrap(), dir)
}

fn addr(server: &TestServer, path: &str) -> Address {
    Address::parse(&format!("vy://localhost:{}{path}", server.port())).unwrap()
}

/// A TXT resolver the tests script: an answer per name, and a count of
/// lookups so a test can assert that none was made.
#[derive(Clone, Debug, Default)]
struct FakeResolver(Arc<Mutex<FakeDns>>);

#[derive(Debug, Default)]
struct FakeDns {
    answers: HashMap<String, Result<Vec<String>, DnsError>>,
    lookups: Vec<String>,
}

impl FakeResolver {
    fn set(&self, name: &str, answer: Result<Vec<String>, DnsError>) {
        self.0
            .lock()
            .unwrap()
            .answers
            .insert(name.to_string(), answer);
    }

    fn lookups(&self) -> Vec<String> {
        self.0.lock().unwrap().lookups.clone()
    }
}

impl TxtResolver for FakeResolver {
    fn txt(&self, name: &str) -> Result<Vec<String>, DnsError> {
        let mut dns = self.0.lock().unwrap();
        dns.lookups.push(name.to_string());
        dns.answers
            .get(name)
            .cloned()
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}

/// Plumbing that lets a test address a loopback server by a remote name:
/// every host resolves to the server's port on 127.0.0.1, and the plain
/// TCP connection reports itself as TLS so an `https://` request goes
/// through. Only tests build agents this way; the product's agent uses
/// ureq's resolver and rustls.
mod remote {
    use std::net::SocketAddr;
    use ureq::config::Config;
    use ureq::unversioned::resolver::{ResolvedSocketAddrs, Resolver};
    use ureq::unversioned::transport::{
        Buffers, ConnectionDetails, Connector, NextTimeout, TcpConnector, Transport,
    };
    use ureq::{http, Agent, Error};

    #[derive(Debug)]
    struct Loopback(u16);

    impl Resolver for Loopback {
        fn resolve(
            &self,
            _uri: &http::Uri,
            _config: &Config,
            _timeout: NextTimeout,
        ) -> Result<ResolvedSocketAddrs, Error> {
            let mut addrs = self.empty();
            addrs.push(SocketAddr::from(([127, 0, 0, 1], self.0)));
            Ok(addrs)
        }
    }

    #[derive(Debug)]
    struct PlainAsTls<T: Transport>(T);

    impl<T: Transport> Transport for PlainAsTls<T> {
        fn buffers(&mut self) -> &mut dyn Buffers {
            self.0.buffers()
        }
        fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
            self.0.transmit_output(amount, timeout)
        }
        fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
            self.0.await_input(timeout)
        }
        fn is_open(&mut self) -> bool {
            self.0.is_open()
        }
        fn is_tls(&self) -> bool {
            true
        }
    }

    #[derive(Debug)]
    struct PretendTls;

    impl<In: Transport> Connector<In> for PretendTls {
        type Out = PlainAsTls<In>;
        fn connect(
            &self,
            _details: &ConnectionDetails,
            chained: Option<In>,
        ) -> Result<Option<Self::Out>, Error> {
            Ok(chained.map(PlainAsTls))
        }
    }

    /// An agent with `config` whose every connection goes to `port` on
    /// loopback, in plain HTTP, whatever the URL says.
    pub fn agent(config: Config, port: u16) -> Agent {
        Agent::with_parts(
            config,
            TcpConnector::default().chain(PretendTls),
            Loopback(port),
        )
    }
}

/// A fetcher that reaches `server` under any host name, with `resolver`
/// answering its TXT lookups.
fn remote_fetcher(name: &str, server: &TestServer, resolver: &FakeResolver) -> (Fetcher, PathBuf) {
    let dir = temp_dir(name);
    let options = Options::new(&dir);
    let agent = remote::agent(Fetcher::test_config(&options), server.port());
    (
        Fetcher::with_parts(options, Box::new(resolver.clone()), agent).unwrap(),
        dir,
    )
}

/// SPEC-7.1.1: vy://host/path?q becomes https://host/path?q with the Accept header.
#[test]
fn spec_7_1_1_translation() {
    assert_eq!(
        to_https("vy://exempel.se/butik?x=1").as_deref(),
        Some("https://exempel.se/butik?x=1")
    );
    assert_eq!(
        to_https("vy://exempel.se:8443/").as_deref(),
        Some("https://exempel.se:8443/")
    );
    assert_eq!(to_https("https://exempel.se/"), None);
    let a = Address::parse("vy://Exempel.SE:443").unwrap();
    assert_eq!(a.to_string(), "vy://exempel.se/");
    assert_eq!(a.origin(), "vy://exempel.se");
    assert_eq!(a.port(), 443);
    assert_eq!(a.path(), "/");
    let b = Address::parse("vy://exempel.se:8443/a/b?c=d").unwrap();
    assert_eq!(b.origin(), "vy://exempel.se:8443");
    assert_eq!(b.query(), Some("c=d"));
    assert_eq!(b.fetch_url(), "https://exempel.se:8443/a/b?c=d");
    assert_eq!(
        Address::parse("vy://[::1]:8080/x").unwrap().fetch_url(),
        "http://[::1]:8080/x"
    );
    for bad in [
        "https://exempel.se/",
        "vy://",
        "vy:///path",
        "vy://user@host/",
        "vy://host:0/",
        "vy://host:70000/",
        "vy://host:08/",
        "vy://hos t/",
        "vy://exempel.se/sida#avsnitt",
        "vy://exämpel.se/",
    ] {
        assert!(Address::parse(bad).is_err(), "{bad}");
    }
    assert_eq!(
        ACCEPT,
        "application/vy-package+zstd, application/vy-manifest+cbor"
    );
    // The header actually sent is exactly that.
    let site = Site::new(package(&key(0), None), Form::Package, &[key(0)]);
    let server = serve(&site);
    let (mut f, dir) = fetcher("accept");
    f.fetch(&addr(&server, "/hello")).unwrap();
    let first = &server.requests()[0];
    assert_eq!(first.path, "/hello");
    assert_eq!(first.header("accept"), Some(ACCEPT));
    let _ = std::fs::remove_dir_all(dir);
}

/// SPEC-7.1.2: fragments are not allowed.
#[test]
fn spec_7_1_2_no_fragments() {
    assert_eq!(to_https("vy://exempel.se/sida#avsnitt"), None);
    assert_eq!(
        Address::parse("vy://exempel.se/sida#"),
        Err(AddressError::Fragment)
    );
}

/// SPEC-7.2.1, SPEC-8.1.2: a response is the whole package or the bare
/// manifest with addresses to module and resources; both give the same
/// page, and an entry without `url` is fetched from `/_vy/<64 hex>`
/// relative to the manifest.
#[test]
fn spec_7_2_1_media_types() {
    assert_eq!(MEDIA_PACKAGE, "application/vy-package+zstd");
    assert_eq!(MEDIA_MANIFEST, "application/vy-manifest+cbor");
    let p = package(&key(0), None);
    let site = Site::new(p.clone(), Form::Package, &[key(0)]);
    let server = serve(&site);
    let (mut f, dir) = fetcher("forms");
    let whole = f.fetch(&addr(&server, "/hello")).unwrap();
    assert_eq!(whole.form, Form::Package);
    assert_eq!(whole.package, p);
    assert_eq!(whole.identity, p.identity());
    assert!(whole.trusted());
    // Page plus key discovery.
    assert_eq!(whole.requests, 2);

    site.lock().unwrap().form = Form::Manifest;
    let (mut g, dir2) = fetcher("forms2");
    let bare = g.fetch(&addr(&server, "/hello")).unwrap();
    assert_eq!(bare.form, Form::Manifest);
    assert_eq!(bare.package, p);
    // Manifest, module, resource, keys.
    assert_eq!(bare.requests, 4);
    let paths: Vec<String> = server.requests().iter().map(|r| r.path.clone()).collect();
    assert!(p.manifest.module.url.is_none() && p.manifest.resources[0].url.is_none());
    assert!(paths.contains(&format!("/_vy/{}", p.manifest.module.hash.hex())));
    assert!(paths.contains(&format!("/_vy/{}", p.manifest.resources[0].hash.hex())));
    assert_eq!(BLOB_PATH_PREFIX, "/_vy/");
    assert_eq!(p.manifest.module.hash.hex().len(), 64);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

/// SPEC-7.1.3: TLS is required; localhost is the only exception. A hop to
/// a plain `http://` host is refused before anything is connected.
#[test]
fn spec_7_1_3_tls_required() {
    assert_eq!(
        Address::parse("vy://exempel.se/").unwrap().fetch_url(),
        "https://exempel.se/"
    );
    assert!(Address::parse("vy://localhost/").unwrap().is_localhost());
    assert!(Address::parse("vy://127.0.0.1/").unwrap().is_localhost());
    assert!(!Address::parse("vy://localhost.example/")
        .unwrap()
        .is_localhost());

    let site = Site::new(package(&key(0), None), Form::Package, &[key(0)]);
    site.lock()
        .unwrap()
        .redirects
        .insert("/plain".into(), "http://example.invalid/hello".into());
    let server = serve(&site);
    let (mut f, dir) = fetcher("tls");
    match f.fetch(&addr(&server, "/plain")) {
        Err(Error::TlsRequired { url }) => assert_eq!(url, "http://example.invalid/hello"),
        other => panic!("expected TlsRequired, got {other:?}"),
    }
    // Only the redirecting request reached a server; `.invalid` was never resolved.
    assert_eq!(server.requests().len(), 1);
    // 127.0.0.1 is loopback too.
    let a = Address::parse(&format!("vy://127.0.0.1:{}/hello", server.port())).unwrap();
    assert!(f.fetch(&a).unwrap().trusted());

    // The exception belongs to the page's own address, not to every hop:
    // a remote page must not be able to point a redirect or a blob URL at
    // `http://localhost:<port>/...` on the user's machine (review 0003).
    let url = |s: &str| HttpUrl::parse(s).unwrap();
    for local in [
        "http://localhost:8080/x",
        "http://127.0.0.1/x",
        "http://[::1]:1/",
    ] {
        assert!(transport_allowed(true, &url(local)), "{local}");
        assert!(!transport_allowed(false, &url(local)), "{local}");
    }
    for remote in ["http://example.com/", "http://localhost.example/"] {
        assert!(!transport_allowed(true, &url(remote)), "{remote}");
        assert!(!transport_allowed(false, &url(remote)), "{remote}");
    }
    for secure in ["https://example.com/", "https://localhost:1/"] {
        assert!(transport_allowed(true, &url(secure)), "{secure}");
        assert!(transport_allowed(false, &url(secure)), "{secure}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// SPEC-7.2.2, SPEC-1.1.1: a response with another media type (e.g. HTML)
/// is never rendered: the fetch fails naming the type.
#[test]
fn spec_7_2_2_other_media_type_not_rendered() {
    let site = Site::new(package(&key(0), None), Form::Package, &[key(0)]);
    site.lock().unwrap().other.insert(
        "/page.html".into(),
        (
            "text/html; charset=utf-8".into(),
            b"<html>no</html>".to_vec(),
        ),
    );
    site.lock().unwrap().other.insert(
        "/data".into(),
        ("application/octet-stream".into(), vec![0; 10]),
    );
    let server = serve(&site);
    let (mut f, dir) = fetcher("notvy");
    match f.fetch(&addr(&server, "/page.html")) {
        Err(Error::NotVy { media_type, url }) => {
            assert_eq!(media_type, "text/html");
            assert!(url.ends_with("/page.html"));
        }
        other => panic!("expected NotVy, got {other:?}"),
    }
    assert!(matches!(
        f.fetch(&addr(&server, "/data")),
        Err(Error::NotVy { media_type, .. }) if media_type == "application/octet-stream"
    ));
    assert!(matches!(
        f.fetch(&addr(&server, "/missing")),
        Err(Error::Status { status: 404, .. })
    ));
    let _ = std::fs::remove_dir_all(dir);
}

/// SPEC-7.3.1: modules and resources are cached by SHA-256, not URL; a
/// matching hash is never refetched; bytes are verified on read.
#[test]
fn spec_7_3_1_content_addressed_cache() {
    let p = package(&key(0), None);
    let site = Site::new(p.clone(), Form::Manifest, &[key(0)]);
    // No HTTP caching of the manifest: every fetch asks the server again.
    site.lock().unwrap().cache_control = None;
    site.lock().unwrap().etag = false;
    let server = serve(&site);
    let module_path = format!("/_vy/{}", p.manifest.module.hash.hex());
    let res_path = format!("/_vy/{}", p.manifest.resources[0].hash.hex());
    let (mut f, dir) = fetcher("blobs");
    f.fetch(&addr(&server, "/hello")).unwrap();
    assert_eq!(server.count(&module_path), 1);
    assert_eq!(server.count(&res_path), 1);
    let second = f.fetch(&addr(&server, "/hello")).unwrap();
    assert_eq!(server.count("/hello"), 2);
    assert_eq!(server.count(&module_path), 1, "module refetched");
    assert_eq!(server.count(&res_path), 1, "resource refetched");
    assert_eq!(second.package, p);
    assert!(f.blobs().contains(&p.manifest.module.hash));

    // Another page using the same module under another URL: not fetched.
    let mut other = p.clone();
    other.manifest.name = "Annan sida".into();
    other.manifest.module.url = Some("/elsewhere/module.wasm".into());
    other.manifest.sign(&key(0));
    site.lock()
        .unwrap()
        .packages
        .insert("/other".into(), other.clone());
    let third = f.fetch(&addr(&server, "/other")).unwrap();
    assert_eq!(third.package.module, p.module);
    assert_eq!(server.count("/elsewhere/module.wasm"), 0);
    assert_eq!(server.count(&module_path), 1);

    // A tampered cache file is a miss: verified on read, then refetched.
    std::fs::write(
        dir.join("blobs").join(p.manifest.module.hash.hex()),
        b"tampered",
    )
    .unwrap();
    f.fetch(&addr(&server, "/hello")).unwrap();
    assert_eq!(server.count(&module_path), 2);

    // A server answering a hash URL with other bytes is refused (SPEC-9.4.1).
    let (mut g, dir2) = fetcher("blobs2");
    site.lock()
        .unwrap()
        .blob_override
        .insert(p.manifest.module.hash, vec![0; p.module.len()]);
    match g.fetch(&addr(&server, "/hello")) {
        Err(Error::HashMismatch { expected, .. }) => {
            assert_eq!(expected, p.manifest.module.hash)
        }
        other => panic!("expected HashMismatch, got {other:?}"),
    }
    assert!(!g.blobs().contains(&p.manifest.module.hash));
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

/// SPEC-4.1.4, SPEC-4.1.5, SPEC-7.2.1: in the bare-manifest form the sizes
/// the manifest declares bound what the client downloads, and they are
/// checked before the first blob request. A manifest claiming a 1 TB
/// module used to make the client read whatever the blob URL streamed
/// until the hash check at the end (review 0003: 300 MB buffered from a
/// loopback server before it gave up).
#[test]
fn spec_4_1_4_declared_sizes_bound_blob_downloads() {
    let p = package(&key(0), None);
    let site = Site::new(p.clone(), Form::Manifest, &[key(0)]);
    site.lock().unwrap().cache_control = None;
    let server = serve(&site);
    let module_path = format!("/_vy/{}", p.manifest.module.hash.hex());
    let (mut f, dir) = fetcher("declared");

    let mut huge = p.clone();
    huge.manifest.module.size = 1 << 40;
    huge.manifest.sign(&key(0));
    site.lock().unwrap().packages.insert("/huge".into(), huge);
    match f.fetch(&addr(&server, "/huge")) {
        Err(Error::Invalid(problems)) => assert!(
            matches!(problems.as_slice(), [Problem::ModuleTooLarge { .. }]),
            "{problems:?}"
        ),
        other => panic!("expected Invalid, got {other:?}"),
    }
    assert_eq!(server.count(&module_path), 0, "no blob was requested");

    let mut wide = p.clone();
    wide.manifest.resources[0].size = u64::MAX;
    wide.manifest.sign(&key(0));
    site.lock().unwrap().packages.insert("/wide".into(), wide);
    match f.fetch(&addr(&server, "/wide")) {
        Err(Error::Invalid(problems)) => assert!(
            matches!(problems.as_slice(), [Problem::ResourcesTooLarge { .. }]),
            "{problems:?}"
        ),
        other => panic!("expected Invalid, got {other:?}"),
    }
    assert_eq!(server.count(&module_path), 0);
    assert_eq!(server.requests().len(), 2, "only the two manifests");
    const _: () = assert!(MAX_BLOB_BYTES <= vy_manifest::MAX_PACKAGE_BYTES);

    // The honest manifest still works, and its blob is fetched once.
    assert!(f.fetch(&addr(&server, "/hello")).unwrap().trusted());
    assert_eq!(server.count(&module_path), 1);
    let _ = std::fs::remove_dir_all(dir);
}

/// SPEC-7.3.2: manifests use ordinary HTTP caching: `max-age` serves from
/// disk without a request, `ETag` revalidates with 304, `no-store` never.
#[test]
fn spec_7_3_2_manifest_http_cache() {
    let p = package(&key(0), None);
    let site = Site::new(p.clone(), Form::Manifest, &[key(0)]);
    let server = serve(&site);
    let (mut f, dir) = fetcher("http");
    let first = f.fetch(&addr(&server, "/hello")).unwrap();
    assert_eq!(first.requests, 4);
    // Fresh for 60 s: no request at all.
    let second = f.fetch(&addr(&server, "/hello")).unwrap();
    assert_eq!(second.requests, 0);
    assert_eq!(second.package, p);
    assert_eq!(server.count("/hello"), 1);

    // Stale at once, with ETag: one conditional request, answered 304.
    site.lock().unwrap().cache_control = Some("max-age=0".into());
    let (mut g, dir2) = fetcher("http2");
    g.fetch(&addr(&server, "/hello")).unwrap();
    let third = g.fetch(&addr(&server, "/hello")).unwrap();
    assert_eq!(third.requests, 1);
    assert_eq!(third.package, p);
    let conditional = server
        .requests()
        .into_iter()
        .filter(|r| r.path == "/hello" && r.header("if-none-match").is_some())
        .count();
    assert_eq!(conditional, 1);

    // The page changes: the ETag no longer matches, the new manifest is used.
    let mut changed = p.clone();
    changed.manifest.name = "Ny version".into();
    changed.manifest.sign(&key(0));
    site.lock()
        .unwrap()
        .packages
        .insert("/hello".into(), changed.clone());
    let fourth = g.fetch(&addr(&server, "/hello")).unwrap();
    assert_eq!(fourth.package.manifest.name, "Ny version");
    assert_eq!(fourth.requests, 1, "module and resource are unchanged");

    // no-store: refetched every time, with the body.
    site.lock().unwrap().cache_control = Some("no-store".into());
    site.lock().unwrap().etag = false;
    let (mut h, dir3) = fetcher("http3");
    h.fetch(&addr(&server, "/hello")).unwrap();
    let before = server.count("/hello");
    let again = h.fetch(&addr(&server, "/hello")).unwrap();
    assert_eq!(again.requests, 1);
    assert_eq!(server.count("/hello"), before + 1);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
    let _ = std::fs::remove_dir_all(dir3);
}

/// SPEC-7.4.1: the Ed25519 signature covers the canonical CBOR without
/// `sig`: a manifest altered after signing is not trusted, the identity
/// does not depend on the signature.
#[test]
fn spec_7_4_1_signature_scope() {
    let p = package(&key(0), None);
    let mut altered = p.clone();
    altered.manifest.name = "Ändrad efter signering".into();
    assert_eq!(altered.manifest.sig, p.manifest.sig);
    let site = Site::new(altered.clone(), Form::Package, &[key(0)]);
    let server = serve(&site);
    let (mut f, dir) = fetcher("scope");
    let fetched = f.fetch(&addr(&server, "/hello")).unwrap();
    assert!(!fetched.trusted());
    assert_eq!(fetched.key_id, key(0).public().key_id());
    // Signing again with any key leaves the identity alone.
    let mut resigned = altered.clone();
    resigned.manifest.sign(&key(5));
    assert_eq!(resigned.identity(), altered.identity());
    assert_ne!(resigned.manifest.sig, altered.manifest.sig);
    let _ = std::fs::remove_dir_all(dir);
}

/// SPEC-7.4.2: the publisher key is read from `/.well-known/vy-key` on the
/// first visit and pinned; later visits use the pin and never ask again.
/// (The DNS TXT half is `spec_7_4_2_dns_txt_must_agree` below.)
#[test]
fn spec_7_4_2_key_discovery() {
    let p = package(&key(0), None);
    let site = Site::new(p.clone(), Form::Package, &[key(0), key(9)]);
    site.lock().unwrap().cache_control = None;
    let server = serve(&site);
    let (mut f, dir) = fetcher("tofu");
    let a = addr(&server, "/hello");
    let first = f.fetch(&a).unwrap();
    assert!(first.trusted());
    assert_eq!(server.count(WELL_KNOWN_KEY_PATH), 1);
    let pinned = f.keys().origin(&a.origin()).unwrap();
    assert_eq!(pinned.pinned.len(), 2, "every published key is pinned");

    // Same origin again: the pin answers, no discovery.
    f.fetch(&a).unwrap();
    assert_eq!(server.count(WELL_KNOWN_KEY_PATH), 1);

    // A key that appears on the site later, unannounced, is not believed.
    let mut other = p.clone();
    other.manifest.sign(&key(7));
    site.lock().unwrap().keys = key_file(&[key(0).public(), key(7).public()]);
    site.lock().unwrap().packages.insert("/hello".into(), other);
    let later = f.fetch(&a).unwrap();
    assert!(!later.trusted());
    assert_eq!(server.count(WELL_KNOWN_KEY_PATH), 1);

    // A first visit to an origin whose key file lacks the signing key.
    site.lock().unwrap().keys = key_file(&[key(0).public()]);
    let (mut g, dir2) = fetcher("tofu2");
    assert!(!g.fetch(&a).unwrap().trusted());
    assert!(
        g.keys().has_pins(&a.origin()),
        "published keys are pinned anyway"
    );
    // A first visit without any key file: untrusted, nothing pinned.
    site.lock().unwrap().keys.clear();
    let (mut h, dir3) = fetcher("tofu3");
    let none = h.fetch(&a).unwrap();
    assert!(!none.trusted());
    assert!(!h.keys().has_pins(&a.origin()));
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
    let _ = std::fs::remove_dir_all(dir3);
}

/// SPEC-7.4.2, SPEC-7.4.5: on the first visit of a remote origin the key
/// file and the TXT records at `_vy.<host>` must agree: only keys in both
/// are pinned and only one of them makes the page trusted. A record that
/// names another key, no `v=vy1` record at all, or a lookup that fails is
/// restricted mode with the reason, nothing pinned, and the next visit asks
/// again. A pinned origin never asks DNS again, and loopback never asks at
/// all (SPEC-7.1.3).
#[test]
fn spec_7_4_2_dns_txt_must_agree() {
    let publisher = key(0);
    let backup = key(9);
    let stranger = key(4);
    let site = Site::new(
        package(&publisher, None),
        Form::Package,
        &[publisher.clone(), backup.clone()],
    );
    site.lock().unwrap().cache_control = None;
    let server = serve(&site);
    let a = Address::parse("vy://exempel.test/hello").unwrap();
    assert!(!a.is_localhost());
    let name = "_vy.exempel.test";
    assert_eq!(vy_manifest::dns_txt_name(a.host()), name);

    // Agree: the record names the publisher; the backup key is only in the
    // key file and is not pinned.
    let dns = FakeResolver::default();
    dns.set(
        name,
        Ok(vec![
            "v=spf1 -all".into(),
            vy_manifest::dns_txt_record(&publisher.public()),
        ]),
    );
    let (mut f, dir) = remote_fetcher("dns-agree", &server, &dns);
    let first = f.fetch(&a).unwrap();
    assert!(first.trusted(), "{:?}", first.restriction());
    assert_eq!(first.restriction(), None);
    assert_eq!(dns.lookups(), [name]);
    let pins = f.keys().origin(&a.origin()).unwrap();
    assert_eq!(
        pins.pinned.keys().collect::<Vec<_>>(),
        [&publisher.public().key_id()],
        "only the key both sources publish is pinned"
    );
    // Pinned: no lookup on the next visit, and a page signed by the
    // backup key is not trusted even though the key file lists it.
    assert!(f.fetch(&a).unwrap().trusted());
    site.lock()
        .unwrap()
        .packages
        .insert("/hello".into(), package(&backup, None));
    let by_backup = f.fetch(&a).unwrap();
    assert!(!by_backup.trusted());
    assert!(matches!(
        by_backup.restriction(),
        Some(Restriction::UnknownKey { key_id }) if *key_id == backup.public().key_id()
    ));
    assert_eq!(dns.lookups().len(), 1, "a pinned origin never asks DNS");
    site.lock()
        .unwrap()
        .packages
        .insert("/hello".into(), package(&publisher, None));

    // Mismatch: the record names a key the key file does not have.
    let dns = FakeResolver::default();
    dns.set(
        name,
        Ok(vec![vy_manifest::dns_txt_record(&stranger.public())]),
    );
    let (mut g, dir2) = remote_fetcher("dns-mismatch", &server, &dns);
    let mismatch = g.fetch(&a).unwrap();
    assert!(!mismatch.trusted());
    assert_eq!(
        mismatch.restriction(),
        Some(&Restriction::DnsMismatch { name: name.into() })
    );
    assert!(!g.keys().has_pins(&a.origin()), "nothing is pinned");
    assert!(mismatch
        .restriction()
        .unwrap()
        .to_string()
        .contains("SPEC-7.4.5"));
    assert!(mismatch.grants().iter().all(|(_, g)| *g == Grant::Denied));

    // Missing: no record, or only records of other forms.
    for records in [vec![], vec!["v=spf1 -all".to_string()]] {
        let dns = FakeResolver::default();
        dns.set(name, Ok(records));
        let (mut h, dir3) = remote_fetcher("dns-missing", &server, &dns);
        let missing = h.fetch(&a).unwrap();
        assert!(!missing.trusted());
        assert_eq!(
            missing.restriction(),
            Some(&Restriction::DnsNoRecord { name: name.into() })
        );
        assert!(!h.keys().has_pins(&a.origin()));
        let _ = std::fs::remove_dir_all(dir3);
    }

    // The lookup fails: restricted, fail closed, and asked again next time.
    let dns = FakeResolver::default();
    dns.set(name, Err(DnsError::Timeout));
    let (mut t, dir4) = remote_fetcher("dns-timeout", &server, &dns);
    let timed_out = t.fetch(&a).unwrap();
    assert!(!timed_out.trusted());
    assert_eq!(
        timed_out.restriction(),
        Some(&Restriction::DnsFailed {
            name: name.into(),
            error: DnsError::Timeout
        })
    );
    assert!(!t.keys().has_pins(&a.origin()));
    dns.set(
        name,
        Ok(vec![vy_manifest::dns_txt_record(&publisher.public())]),
    );
    assert!(t.fetch(&a).unwrap().trusted(), "the next visit asks again");
    assert_eq!(dns.lookups().len(), 2);

    // The signing key is in DNS but not in the key file: not pinned.
    let dns = FakeResolver::default();
    dns.set(
        name,
        Ok(vec![
            vy_manifest::dns_txt_record(&publisher.public()),
            vy_manifest::dns_txt_record(&stranger.public()),
        ]),
    );
    site.lock()
        .unwrap()
        .packages
        .insert("/hello".into(), package(&stranger, None));
    let (mut s, dir5) = remote_fetcher("dns-half", &server, &dns);
    let half = s.fetch(&a).unwrap();
    assert!(!half.trusted());
    assert!(matches!(
        half.restriction(),
        Some(Restriction::KeyNotPublished { key_id }) if *key_id == stranger.public().key_id()
    ));
    assert_eq!(
        s.keys()
            .origin(&a.origin())
            .unwrap()
            .pinned
            .keys()
            .collect::<Vec<_>>(),
        [&publisher.public().key_id()]
    );
    site.lock()
        .unwrap()
        .packages
        .insert("/hello".into(), package(&publisher, None));

    // Loopback has no DNS: the key file alone, no lookup (SPEC-7.1.3).
    let dns = FakeResolver::default();
    let dir6 = temp_dir("dns-local");
    let mut l = Fetcher::with_resolver(Options::new(&dir6), Box::new(dns.clone())).unwrap();
    let local = l.fetch(&addr(&server, "/hello")).unwrap();
    assert!(local.trusted());
    assert!(dns.lookups().is_empty());
    for d in [dir, dir2, dir4, dir5, dir6] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// SPEC-7.4.3: a page without a valid signature runs in restricted mode:
/// no capability is granted, not even those granted automatically. The
/// grants come from `Fetched::grants` alone: the trust token behind them is
/// private to `Fetched` and made by signature verification, so a shell
/// holding the manifest's requests cannot grant past the trust decision.
#[test]
fn spec_7_4_3_restricted_mode() {
    let mut d = draft();
    d.capabilities.ime = Some(vy_manifest::Empty {});
    d.capabilities.storage = Some(vy_manifest::StorageRequest { quota_kb: 64 });
    let wasm = module();
    let mut manifest = d.finish(&wasm, &[]);
    manifest.sign(&key(0));
    let good = Package {
        manifest,
        module: wasm,
        resources: vec![],
    };
    let mut bad = good.clone();
    bad.manifest.sig.as_mut().unwrap().bytes.0[0] ^= 1;

    let site = Site::new(bad, Form::Package, &[key(0)]);
    site.lock().unwrap().cache_control = None;
    let server = serve(&site);
    let (mut f, dir) = fetcher("restricted");
    let a = addr(&server, "/hello");
    let restricted = f.fetch(&a).unwrap();
    assert!(!restricted.trusted());
    assert_eq!(
        restricted.grants(),
        [("storage", Grant::Denied), ("ime", Grant::Denied)]
    );
    // The requests are readable, but without the token every request is
    // denied whatever the caller claims about origins.
    assert!(restricted.package.manifest.capabilities.requests("storage"));
    assert!(restricted
        .package
        .manifest
        .capabilities
        .default_grants(None, |_| true)
        .iter()
        .all(|(_, g)| *g == Grant::Denied));
    site.lock().unwrap().packages.insert("/hello".into(), good);
    let trusted = f.fetch(&a).unwrap();
    assert!(trusted.trusted());
    assert_eq!(
        trusted.grants(),
        [("storage", Grant::Auto), ("ime", Grant::Auto)]
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// SPEC-7.4.4, SPEC-7.4.6: key rotation end to end: a manifest signed with the old
/// key names the new key id; the new key is then fetched from
/// `.well-known/vy-key` and accepted for the origin, and the old key is
/// retired at that moment (the decision: rotation takes a key out of
/// service, so a client never honours a rotated-away key again). A key the
/// site publishes without a chain from a trusted manifest is refused
/// ("ingen annan väg ger en okänd nyckel förtroende"), an announced key
/// that the key file lacks waits, and a manifest that merely names the
/// successor without being signed by it neither promotes nor retires.
#[test]
fn spec_7_4_4_key_rotation() {
    let old = key(0);
    let new = key(3);
    let stranger = key(4);
    let announcing = package(&old, Some(&new.public().key_id()));
    let site = Site::new(
        announcing.clone(),
        Form::Package,
        std::slice::from_ref(&old),
    );
    site.lock().unwrap().cache_control = None;
    let server = serve(&site);
    let (mut f, dir) = fetcher("rotation");
    let a = addr(&server, "/hello");
    assert!(f.fetch(&a).unwrap().trusted());
    assert!(f.keys().is_successor(&a.origin(), &new.public().key_id()));

    // Announced but not yet in the key file: restricted, the announcement
    // kept, and a page still signed by the old key is still trusted.
    site.lock()
        .unwrap()
        .packages
        .insert("/hello".into(), package(&new, None));
    let early = f.fetch(&a).unwrap();
    assert!(!early.trusted());
    assert!(matches!(
        early.restriction(),
        Some(Restriction::SuccessorNotPublished { key_id }) if *key_id == new.public().key_id()
    ));
    assert!(f.keys().is_successor(&a.origin(), &new.public().key_id()));
    assert!(f
        .keys()
        .pinned(&a.origin(), &old.public().key_id())
        .is_some());
    site.lock()
        .unwrap()
        .packages
        .insert("/hello".into(), announcing.clone());
    assert!(f.fetch(&a).unwrap().trusted());

    // A manifest that names the successor's id but is signed by someone
    // else: a bad signature, and nothing changes in the store.
    let mut forged = package(&stranger, None);
    forged.manifest.sig.as_mut().unwrap().key_id = new.public().key_id();
    site.lock().unwrap().keys = key_file(&[old.public(), new.public(), stranger.public()]);
    site.lock()
        .unwrap()
        .packages
        .insert("/hello".into(), forged);
    let bad = f.fetch(&a).unwrap();
    assert_eq!(bad.restriction(), Some(&Restriction::BadSignature));
    assert!(f.keys().is_successor(&a.origin(), &new.public().key_id()));
    assert!(f
        .keys()
        .pinned(&a.origin(), &old.public().key_id())
        .is_some());

    // The publisher rotates: the new key signs, both keys published.
    site.lock()
        .unwrap()
        .packages
        .insert("/hello".into(), package(&new, None));
    let rotated = f.fetch(&a).unwrap();
    assert!(rotated.trusted());
    assert_eq!(rotated.key_id, new.public().key_id());
    assert!(f
        .keys()
        .pinned(&a.origin(), &new.public().key_id())
        .is_some());
    assert!(!f.keys().is_successor(&a.origin(), &new.public().key_id()));
    assert_eq!(
        f.keys().pinned(&a.origin(), &old.public().key_id()),
        None,
        "the old key is retired by the rotation"
    );
    let text = std::fs::read_to_string(dir.join("keys")).unwrap();
    assert_eq!(text, format!("pin {} {}\n", a.origin(), new.public()));

    // A page still signed by the old key is restricted from now on, even
    // though the key file still lists the key.
    site.lock()
        .unwrap()
        .packages
        .insert("/hello".into(), package(&old, None));
    let stale = f.fetch(&a).unwrap();
    assert!(!stale.trusted());
    assert!(matches!(
        stale.restriction(),
        Some(Restriction::UnknownKey { key_id }) if *key_id == old.public().key_id()
    ));

    // A key that was published but never announced is refused: no chain
    // from a trusted manifest, no trust.
    site.lock()
        .unwrap()
        .packages
        .insert("/hello".into(), package(&stranger, None));
    let unknown = f.fetch(&a).unwrap();
    assert!(!unknown.trusted());
    assert!(matches!(
        unknown.restriction(),
        Some(Restriction::UnknownKey { key_id }) if *key_id == stranger.public().key_id()
    ));
    assert_eq!(
        server.count(WELL_KNOWN_KEY_PATH),
        4,
        "the key file is fetched only for a first visit or an announced successor"
    );
    assert!(unknown
        .restriction()
        .unwrap()
        .to_string()
        .contains("SPEC-7.4.4"));
    let _ = std::fs::remove_dir_all(dir);
}

/// SPEC-7.4.4, SPEC-9.5.2: a trusted publisher cannot reach another
/// origin's trust through `next_key_id`. The key store is a text file with
/// one line per pin, so a value with a newline in it used to be written
/// as a `pin` line for whatever origin it named (review 0003). Such a
/// manifest is now refused at validation, before trust is even decided,
/// and the store never sees it.
#[test]
fn spec_7_4_4_next_key_id_cannot_forge_pins() {
    let publisher = key(0);
    let stranger = key(4);
    let forged = format!("x\npin vy://bank.example {}", stranger.public());
    let mut d = draft();
    d.next_key_id = Some(forged);
    let wasm = module();
    let mut manifest = d.finish(&wasm, &[]);
    manifest.sign(&publisher);
    let hostile = Package {
        manifest,
        module: wasm,
        resources: vec![],
    };
    assert!(matches!(
        hostile.validate().as_slice(),
        [Problem::NextKeyIdInvalid(_)]
    ));
    for form in [Form::Package, Form::Manifest] {
        let site = Site::new(hostile.clone(), form, std::slice::from_ref(&publisher));
        site.lock().unwrap().cache_control = None;
        let server = serve(&site);
        let (mut f, dir) = fetcher("forge");
        let a = addr(&server, "/hello");
        let err = f.fetch(&a).unwrap_err();
        assert!(
            matches!(
                &err,
                Error::Invalid(p) | Error::Package(PackageError::Invalid(p))
                    if p.iter().any(|p| matches!(p, Problem::NextKeyIdInvalid(_)))
            ),
            "{form:?}: {err}"
        );
        assert!(!f.keys().has_pins("vy://bank.example"));
        assert!(
            !f.keys().has_pins(&a.origin()),
            "nothing is pinned for a page that was never trusted"
        );
        let text = std::fs::read_to_string(dir.join("keys")).unwrap_or_default();
        assert!(!text.contains("bank.example"), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// SPEC-7.5.1: no Referer, at most 3 redirects.
#[test]
fn spec_7_5_1_no_referer_max_redirects() {
    let site = Site::new(package(&key(0), None), Form::Package, &[key(0)]);
    {
        let mut s = site.lock().unwrap();
        s.redirects.insert("/r1".into(), "/r2".into());
        s.redirects.insert("/r2".into(), "r3".into());
        s.redirects.insert("/r3".into(), "/hello".into());
        s.redirects.insert("/r0".into(), "/r1".into());
    }
    let server = serve(&site);
    let (mut f, dir) = fetcher("redirects");
    // Three hops: allowed.
    let fetched = f.fetch(&addr(&server, "/r1")).unwrap();
    assert!(fetched.trusted());
    // Four hops: refused at the fourth redirect, before following it.
    match f.fetch(&addr(&server, "/r0")) {
        Err(Error::TooManyRedirects { url }) => assert!(url.ends_with("/r3")),
        other => panic!("expected TooManyRedirects, got {other:?}"),
    }
    assert_eq!(MAX_REDIRECTS, 3);
    for r in server.requests() {
        assert!(r.header("referer").is_none(), "{r:?}");
    }
    assert!(server.requests().len() >= 8);
    let _ = std::fs::remove_dir_all(dir);
}

/// SPEC-5.2.7, SPEC-9.1.1, SPEC-7.1.3, SPEC-7.5.1: a page's own
/// `fetch_begin` is performed by the main process through
/// `Fetcher::request`: GET and POST with the body as given, the status
/// and body returned whatever the status, no `Referer`, no redirect
/// followed, plain `http://` only to loopback for a loopback page, and a
/// body over the limit refused.
#[test]
fn spec_5_2_7_page_requests_through_the_main_process() {
    let server = TestServer::start(|req| match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/api/items") => Response::ok("application/json", "[1,2,3]"),
        ("POST", "/api/messages") => {
            Response::new(201).body(format!("got {} bytes", req.body.len()))
        }
        ("GET", "/api/big") => Response::ok("text/plain", vec![b'x'; 10_000]),
        ("GET", "/api/moved") => Response::new(302).header("Location", "/api/items"),
        _ => Response::new(404).body("nope"),
    });
    let (f, dir) = fetcher("request");
    let base = format!("http://localhost:{}", server.port());
    let (status, body) = f
        .request("GET", &format!("{base}/api/items"), b"", true, 1 << 20)
        .unwrap();
    assert_eq!((status, body.as_slice()), (200, &b"[1,2,3]"[..]));
    let (status, body) = f
        .request(
            "POST",
            &format!("{base}/api/messages"),
            br#"{"text":"hej"}"#,
            true,
            1 << 20,
        )
        .unwrap();
    assert_eq!((status, body.as_slice()), (201, &b"got 14 bytes"[..]));
    let (status, body) = f
        .request("GET", &format!("{base}/missing"), b"", true, 1 << 20)
        .unwrap();
    assert_eq!(
        (status, body.as_slice()),
        (404, &b"nope"[..]),
        "a status, not an error"
    );
    let (status, _) = f
        .request("GET", &format!("{base}/api/moved"), b"", true, 1 << 20)
        .unwrap();
    assert_eq!(status, 302, "redirects are the page's to handle");
    match f.request("GET", &format!("{base}/api/big"), b"", true, 9_999) {
        Err(Error::TooLarge { limit, .. }) => assert_eq!(limit, 9_999),
        other => panic!("expected TooLarge, got {other:?}"),
    }
    match f.request("GET", &format!("{base}/api/items"), b"", false, 1 << 20) {
        Err(Error::TlsRequired { .. }) => {}
        other => panic!("http:// for a remote page: expected TlsRequired, got {other:?}"),
    }
    assert!(f
        .request("PATCH", &format!("{base}/api/items"), b"", true, 1 << 20)
        .is_err());
    for r in server.requests() {
        assert!(r.header("referer").is_none(), "{r:?}");
    }
    assert_eq!(server.requests().len(), 5);
    let _ = std::fs::remove_dir_all(dir);
}
