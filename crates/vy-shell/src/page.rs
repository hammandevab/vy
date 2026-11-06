//! Getting a page: from a `vy://` address through `vy-proto` (SPEC §7),
//! or from a local `.vyp` package or bare `.wasm` for development; the
//! trust decision and the default capability verdicts (SPEC-7.4.3,
//! SPEC-8.2.3). Runs off the UI thread, since a fetch can take seconds:
//! pages on the loader thread, a page's own requests on its tab's worker.

use crate::ipc::ResourceBytes;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use vy_manifest::{Draft, Grant, Manifest, Package};
use vy_proto::{
    Address, Error as FetchError, Fetcher, Options as FetchOptions, Requester, Restriction,
};

/// What a tab is asked to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A `vy://` address, fetched (SPEC-7.1.1).
    Address(Address),
    /// A local file: `.vyp` (a package as `vy-publish build` writes it) or
    /// `.wasm` (a bare module, optionally with a manifest draft).
    File {
        /// The file.
        path: PathBuf,
        /// A manifest draft to finish around a bare module.
        draft: Option<PathBuf>,
    },
}

impl Source {
    /// What the user typed or clicked: a `vy://` address, else a path.
    /// `https://` is not a source: the shell opens it externally
    /// (SPEC-2.1.7) and never calls this for it.
    pub fn parse(text: &str) -> Result<Source, String> {
        let text = text.trim();
        if text.starts_with("vy://") {
            return Address::parse(text)
                .map(Source::Address)
                .map_err(|e| e.to_string());
        }
        if text.contains("://") {
            return Err(format!("{text}: not a vy:// address"));
        }
        Ok(Source::File {
            path: PathBuf::from(text),
            draft: None,
        })
    }

    /// What the address bar shows for it.
    pub fn display(&self) -> String {
        match self {
            Source::Address(a) => a.to_string(),
            Source::File { path, .. } => path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        }
    }

    /// The origin (SPEC-9.5.1), which a local file does not have.
    pub fn origin(&self) -> Option<String> {
        match self {
            Source::Address(a) => Some(a.origin()),
            Source::File { .. } => None,
        }
    }
}

/// A page ready to be handed to a tab process.
#[derive(Clone, Debug)]
pub struct LoadedPage {
    /// What the address bar shows.
    pub display: String,
    /// The origin, for grants and storage; `None` for a local file.
    pub origin: Option<String>,
    /// The manifest, when there is one (an address always has one).
    pub manifest: Option<Manifest>,
    /// Module bytes.
    pub wasm: Vec<u8>,
    /// SHA-256 the manifest states, or of the bytes for a bare module.
    pub sha256: [u8; 32],
    /// Resources with the manifest's image sizes.
    pub resources: Vec<ResourceBytes>,
    /// True when the signature verified under a trusted key (SPEC-7.4.2);
    /// false is restricted mode (SPEC-7.4.3).
    pub trusted: bool,
    /// Why a fetched page is restricted (SPEC-7.4.3, SPEC-7.4.5), shown
    /// beside the marker; `None` for a trusted page and for a local file,
    /// which has no origin to trust.
    pub restriction: Option<Restriction>,
    /// The default policy's verdict per requested capability; every one
    /// `Denied` for an untrusted page (SPEC-8.2.3, SPEC-7.4.3).
    pub verdicts: Vec<(&'static str, Grant)>,
    /// True when the page's own address is loopback (SPEC-7.1.3).
    pub is_localhost: bool,
}

/// Why a page could not be loaded; each is shown as an error view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadError {
    /// The response was not a Vy page (SPEC-7.2.2); the shell offers to
    /// open `url` in the system browser.
    NotVy {
        /// The HTTPS URL that answered.
        url: String,
        /// Its media type.
        media_type: String,
    },
    /// An HTTP status other than 200.
    Status {
        /// The URL.
        url: String,
        /// The status.
        status: u16,
    },
    /// The network, the manifest, the package or the hash.
    Fetch(String),
    /// A local file could not be read or is not a package or module.
    File(String),
}

impl LoadError {
    /// A short title for the error view.
    pub fn title(&self) -> &'static str {
        match self {
            Self::NotVy { .. } => "Inte en Vy-sida",
            Self::Status { status: 404, .. } => "Sidan finns inte",
            Self::Status { .. } => "Servern svarade med ett fel",
            Self::Fetch(_) => "Sidan kunde inte hämtas",
            Self::File(_) => "Filen kunde inte öppnas",
        }
    }

    /// The detail line.
    pub fn detail(&self) -> String {
        match self {
            Self::NotVy { url, media_type } => {
                format!("{url} svarade med {media_type}, som Vy aldrig renderar (SPEC-7.2.2).")
            }
            Self::Status { url, status } => format!("{url}: HTTP {status}."),
            Self::Fetch(s) | Self::File(s) => s.clone(),
        }
    }

    /// The address the error view offers to open externally (SPEC-7.2.2).
    pub fn external_url(&self) -> Option<&str> {
        match self {
            Self::NotVy { url, .. } => Some(url),
            _ => None,
        }
    }
}

/// The reason a page is restricted, short and in Swedish, as the chrome
/// shows it after "begränsat läge: " (SPEC-7.4.3, SPEC-7.4.5). Exhaustive,
/// so a new reason in `vy-proto` does not build without its text here.
pub fn restriction_reason(r: &Restriction) -> &'static str {
    match r {
        Restriction::Unsigned => "sidan är osignerad",
        Restriction::BadSignature => "signaturen stämmer inte",
        Restriction::UnknownKey { .. } => "okänd nyckel för sajten",
        Restriction::KeyFileUnavailable(_) => "nyckelfilen kunde inte hämtas",
        Restriction::KeyNotPublished { .. } => "nyckeln är inte publicerad",
        Restriction::DnsNoRecord { .. } => "nyckeln saknas i DNS",
        Restriction::DnsMismatch { .. } => "DNS anger en annan nyckel",
        Restriction::DnsFailed { .. } => "DNS-uppslaget misslyckades",
        Restriction::SuccessorNotPublished { .. } => "den nya nyckeln är inte publicerad",
        Restriction::LogInvalid(_) => "nyckelloggen är ogiltig",
        Restriction::NotInLog { .. } => "nyckeln saknas i nyckelloggen",
        Restriction::LogRewritten => "nyckelloggen har skrivits om",
        Restriction::LogWithdrawn => "nyckelloggen har tagits bort",
    }
}

/// Load `source`, fetching through `fetcher` for an address.
pub fn load(fetcher: &mut Fetcher, source: &Source) -> Result<LoadedPage, LoadError> {
    match source {
        Source::Address(address) => {
            let fetched = fetcher.fetch(address).map_err(|e| match e {
                FetchError::NotVy { url, media_type } => LoadError::NotVy { url, media_type },
                FetchError::Status { url, status } => LoadError::Status { url, status },
                other => LoadError::Fetch(other.to_string()),
            })?;
            let trusted = fetched.trusted();
            let restriction = fetched.restriction().cloned();
            let verdicts = fetched.grants();
            let Package {
                manifest,
                module,
                resources,
            } = fetched.package;
            let resources = resources
                .into_iter()
                .zip(&manifest.resources)
                .map(|(bytes, r)| (bytes, r.w.zip(r.h)))
                .collect();
            Ok(LoadedPage {
                display: address.to_string(),
                origin: Some(address.origin()),
                sha256: manifest.module.hash.0,
                manifest: Some(manifest),
                wasm: module,
                resources,
                trusted,
                restriction,
                verdicts,
                is_localhost: address.is_localhost(),
            })
        }
        Source::File { path, draft } => load_file(path, draft.as_deref()),
    }
}

/// A local package or module: restricted mode, since there is no origin
/// to trust a key for (SPEC-7.4.3).
fn load_file(path: &Path, draft: Option<&Path>) -> Result<LoadedPage, LoadError> {
    let bytes =
        std::fs::read(path).map_err(|e| LoadError::File(format!("{}: {e}", path.display())))?;
    let display = Source::File {
        path: path.to_path_buf(),
        draft: None,
    }
    .display();
    let is_wasm = bytes.starts_with(b"\0asm");
    let (manifest, wasm, resources) = if is_wasm {
        let manifest = match draft {
            None => None,
            Some(draft_path) => {
                let text = std::fs::read_to_string(draft_path)
                    .map_err(|e| LoadError::File(format!("{}: {e}", draft_path.display())))?;
                let draft = Draft::from_json(&text)
                    .map_err(|e| LoadError::File(format!("{}: {e}", draft_path.display())))?;
                let base = draft_path.parent().unwrap_or(Path::new("."));
                let inputs = draft
                    .load_resources(base)
                    .map_err(|e| LoadError::File(e.to_string()))?;
                let manifest = draft.finish(&bytes, &inputs);
                let resources: Vec<ResourceBytes> = inputs
                    .into_iter()
                    .zip(&manifest.resources)
                    .map(|(input, r)| (input.bytes, r.w.zip(r.h)))
                    .collect();
                // Unsigned: every verdict denied (SPEC-7.4.3).
                let verdicts = manifest.capabilities.default_grants(None, |_| false);
                return Ok(LoadedPage {
                    display,
                    origin: None,
                    sha256: manifest.module.hash.0,
                    manifest: Some(manifest),
                    wasm: bytes,
                    resources,
                    trusted: false,
                    restriction: None,
                    verdicts,
                    is_localhost: false,
                });
            }
        };
        (manifest, bytes, Vec::new())
    } else {
        let package = Package::unpack(&bytes).map_err(|e| {
            LoadError::File(format!(
                "{}: neither a module nor a package: {e}",
                path.display()
            ))
        })?;
        let problems = package.validate();
        if !problems.is_empty() {
            let lines: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
            return Err(LoadError::File(format!(
                "{}: the manifest does not describe the package: {}",
                path.display(),
                lines.join("; ")
            )));
        }
        let resources = package
            .resources
            .into_iter()
            .zip(&package.manifest.resources)
            .map(|(bytes, r)| (bytes, r.w.zip(r.h)))
            .collect();
        (Some(package.manifest), package.module, resources)
    };
    let sha256 = match &manifest {
        Some(m) => m.module.hash.0,
        None => vy_runtime::sha256(&wasm),
    };
    // A file has no origin and no key to trust: restricted mode, every
    // verdict denied (SPEC-7.4.3).
    let verdicts = manifest
        .as_ref()
        .map(|m| m.capabilities.default_grants(None, |_| false))
        .unwrap_or_default();
    Ok(LoadedPage {
        display,
        origin: None,
        manifest,
        wasm,
        sha256,
        resources,
        trusted: false,
        restriction: None,
        verdicts,
        is_localhost: false,
    })
}

/// A job for the loader ([`Loader::send`]): a page's request goes to its
/// tab's worker, everything else to the loader thread.
#[derive(Debug)]
pub enum Job {
    /// Load a page for a tab.
    Page {
        /// The tab.
        tab: u32,
        /// The tab's navigation count, to drop stale results.
        generation: u64,
        /// What to load.
        source: Source,
    },
    /// Live reload: has the page changed?
    Check(Check),
    /// Perform a page's own request (SPEC-5.2.7).
    Request {
        /// The tab.
        tab: u32,
        /// The tab's navigation count.
        generation: u64,
        /// `req_id`.
        id: i32,
        /// `GET`, `POST`, `PUT` or `DELETE`.
        method: &'static str,
        /// The URL.
        url: String,
        /// The body.
        body: Vec<u8>,
        /// Whether the page itself is on loopback (SPEC-7.1.3).
        page_is_localhost: bool,
    },
}

/// Live reload (devtools, SPEC-7.3.2): ask a page's server whether the
/// page changed, with a conditional GET whatever the HTTP cache's
/// freshness says.
#[derive(Debug)]
pub struct Check {
    /// The tab.
    pub tab: u32,
    /// The tab's navigation count.
    pub generation: u64,
    /// The page's address.
    pub address: Address,
}

/// What the loader reports back, from the thread that did the job.
#[derive(Debug)]
pub enum Done {
    /// A live-reload check finished: the SHA-256 of what the server answers
    /// for the page now, or why not.
    Check {
        /// The tab.
        tab: u32,
        /// The navigation count the job carried.
        generation: u64,
        /// The answer's hash.
        result: Result<[u8; 32], String>,
    },
    /// A page load finished.
    Page {
        /// The tab.
        tab: u32,
        /// The navigation count the job carried.
        generation: u64,
        /// The page, or why not.
        result: Box<Result<LoadedPage, LoadError>>,
    },
    /// A page's request finished (SPEC-5.2.7): the status, or a negative
    /// code from appendix A for a transport failure, and the body.
    Request {
        /// The tab.
        tab: u32,
        /// The navigation count the job carried.
        generation: u64,
        /// `req_id`.
        id: i32,
        /// Status or negative code.
        status: i32,
        /// The body.
        body: Vec<u8>,
    },
}

/// The loader (SPEC-5.2.7, SPEC-7.4.5): page loads and live-reload checks
/// run in order on one thread, which owns the [`Fetcher`] with its caches
/// and key store; a page's own requests run on a worker thread per tab,
/// in the order the tab made them, so a slow request (a long poll, a slow
/// server) holds up neither another tab's requests nor any page load, and a
/// page load (a first visit's DNS lookup can take seconds) holds up no
/// page's requests.
pub struct Loader {
    pages: Sender<Job>,
    requester: Requester,
    report: Report,
    workers: HashMap<u32, Sender<Job>>,
}

type Report = Arc<dyn Fn(Done) + Send + Sync>;

impl Loader {
    /// Hand `job` to the thread that does it: a request to its tab's
    /// worker (started at the tab's first request), anything else to the
    /// loader thread.
    pub fn send(&mut self, job: Job) {
        let Job::Request { tab, .. } = &job else {
            let _ = self.pages.send(job);
            return;
        };
        let tab = *tab;
        // A worker whose thread has ended hands the job back; start anew.
        let job = match self.workers.get(&tab) {
            None => job,
            Some(worker) => match worker.send(job) {
                Ok(()) => return,
                Err(std::sync::mpsc::SendError(job)) => job,
            },
        };
        match spawn_request_worker(tab, self.requester.clone(), self.report.clone()) {
            Ok(worker) => {
                let _ = worker.send(job);
                self.workers.insert(tab, worker);
            }
            // No thread: the request fails as a transport failure would.
            Err(_) => {
                if let Job::Request {
                    tab,
                    generation,
                    id,
                    ..
                } = job
                {
                    (self.report)(Done::Request {
                        tab,
                        generation,
                        id,
                        status: vy_abi::err::E_NET,
                        body: Vec::new(),
                    });
                }
            }
        }
    }

    /// The tab is gone: its worker ends once its queued requests are done.
    pub fn close_tab(&mut self, tab: u32) {
        self.workers.remove(&tab);
    }

    /// Tabs with a request worker, for tests.
    pub fn workers(&self) -> usize {
        self.workers.len()
    }
}

/// A worker that performs one tab's requests in order with `requester`.
fn spawn_request_worker(
    tab: u32,
    requester: Requester,
    report: Report,
) -> std::io::Result<Sender<Job>> {
    let (tx, rx) = std::sync::mpsc::channel::<Job>();
    thread::Builder::new()
        .name(format!("vy-requests-{tab}"))
        .spawn(move || {
            while let Ok(job) = rx.recv() {
                if let Job::Request {
                    tab,
                    generation,
                    id,
                    method,
                    url,
                    body,
                    page_is_localhost,
                } = job
                {
                    let outcome = requester.request(
                        method,
                        &url,
                        &body,
                        page_is_localhost,
                        u64::from(vy_abi::limits::MAX_FETCH_RESPONSE_BYTES) + 1,
                    );
                    let (status, body) = match outcome {
                        Ok((status, body)) => (i32::from(status), body),
                        Err(FetchError::TooLarge { .. }) => (
                            vy_abi::err::E_TOO_LARGE,
                            vec![0; vy_abi::limits::MAX_FETCH_RESPONSE_BYTES as usize + 1],
                        ),
                        Err(_) => (vy_abi::err::E_NET, Vec::new()),
                    };
                    report(Done::Request {
                        tab,
                        generation,
                        id,
                        status,
                        body,
                    });
                }
            }
        })?;
    Ok(tx)
}

/// Start the loader over the caches under `cache_dir`, reporting through
/// `report` (which also wakes the UI), from whichever thread did the job.
pub fn start_loader(
    cache_dir: PathBuf,
    report: impl Fn(Done) + Send + Sync + 'static,
) -> std::io::Result<Loader> {
    let report: Report = Arc::new(report);
    let requester = Requester::new(&FetchOptions::new(&cache_dir));
    let (tx, rx) = std::sync::mpsc::channel::<Job>();
    let pages_report = report.clone();
    thread::Builder::new()
        .name("vy-loader".into())
        .spawn(move || {
            let report = pages_report;
            let mut fetcher: Option<Fetcher> = None;
            let open = |fetcher: &mut Option<Fetcher>| -> Result<(), String> {
                if fetcher.is_none() {
                    *fetcher = Some(
                        Fetcher::new(FetchOptions::new(&cache_dir)).map_err(|e| e.to_string())?,
                    );
                }
                Ok(())
            };
            while let Ok(job) = rx.recv() {
                match job {
                    Job::Page {
                        tab,
                        generation,
                        source,
                    } => {
                        // A file needs no caches; it loads even when the
                        // cache directory is unusable.
                        let result = match &source {
                            Source::File { .. } => load_file_source(&source),
                            Source::Address(_) => match open(&mut fetcher) {
                                Ok(()) => load(fetcher.as_mut().expect("opened"), &source),
                                Err(e) => Err(LoadError::Fetch(format!("cache: {e}"))),
                            },
                        };
                        report(Done::Page {
                            tab,
                            generation,
                            result: Box::new(result),
                        });
                    }
                    Job::Check(Check {
                        tab,
                        generation,
                        address,
                    }) => {
                        let result = open(&mut fetcher).and_then(|()| {
                            fetcher
                                .as_mut()
                                .expect("opened")
                                .revalidate(&address)
                                .map(|h| h.0)
                                .map_err(|e| e.to_string())
                        });
                        report(Done::Check {
                            tab,
                            generation,
                            result,
                        });
                    }
                    // Requests go to their tab's worker (`Loader::send`).
                    Job::Request { .. } => {}
                }
            }
        })?;
    Ok(Loader {
        pages: tx,
        requester,
        report,
        workers: HashMap::new(),
    })
}

fn load_file_source(source: &Source) -> Result<LoadedPage, LoadError> {
    match source {
        Source::File { path, draft } => load_file(path, draft.as_deref()),
        Source::Address(_) => unreachable!("files only"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    /// A loopback server that answers every path with its own name, and
    /// holds a path starting with `/slow` until the test releases it.
    fn held_server() -> (u16, std::sync::mpsc::Sender<()>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let held = Arc::new(std::sync::Mutex::new(held));
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let held = held.clone();
                thread::spawn(move || {
                    let mut reader = BufReader::new(stream);
                    let mut first = String::new();
                    let _ = reader.read_line(&mut first);
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                    }
                    let path = first.split(' ').nth(1).unwrap_or("").to_string();
                    if path.starts_with("/slow") {
                        let _ = held
                            .lock()
                            .unwrap()
                            .recv_timeout(std::time::Duration::from_secs(30));
                    }
                    let _ = reader.into_inner().write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{path}",
                            path.len()
                        )
                        .as_bytes(),
                    );
                });
            }
        });
        (port, release)
    }

    /// SPEC-5.2.7, SPEC-7.4.5: a page's requests run on its tab's own
    /// worker, in the order it made them: a request the server holds (a
    /// long poll) delays neither another tab's request nor a page load, and
    /// a page load the server holds (as a first visit's DNS lookup can,
    /// for up to 6 s) delays no tab's request. Closing a tab ends its worker.
    #[test]
    fn spec_5_2_7_requests_do_not_wait_for_other_tabs() {
        let (port, release) = held_server();
        let (tx, rx) = std::sync::mpsc::channel::<Done>();
        let tx = std::sync::Mutex::new(tx);
        let dir = std::env::temp_dir().join(format!("vy-loader-{}", std::process::id()));
        let mut loader = start_loader(dir.clone(), move |d| {
            let _ = tx.lock().unwrap().send(d);
        })
        .unwrap();
        let request = |tab: u32, id: i32, path: &str| Job::Request {
            tab,
            generation: 1,
            id,
            method: "GET",
            url: format!("http://127.0.0.1:{port}{path}"),
            body: Vec::new(),
            page_is_localhost: true,
        };
        let page = |tab: u32, source: Source| Job::Page {
            tab,
            generation: 1,
            source,
        };
        // Tab 1: a held request, then one behind it; meanwhile tab 2's
        // request and tab 3's local page.
        loader.send(request(1, 1, "/slow-poll"));
        loader.send(request(1, 2, "/after"));
        loader.send(request(2, 1, "/other-tab"));
        let hello = root().join("target/wasm/hello.wasm");
        assert!(hello.is_file(), "run tools/build_modules.sh");
        loader.send(page(
            3,
            Source::File {
                path: hello,
                draft: None,
            },
        ));
        let wait = std::time::Duration::from_secs(20);
        let mut early = Vec::new();
        for _ in 0..2 {
            early.push(rx.recv_timeout(wait).expect("not held up by tab 1"));
        }
        // Tab 5's load is held; a tab whose first request comes meanwhile
        // gets its answer.
        let held_page = Source::parse(&format!("vy://127.0.0.1:{port}/slow-page")).unwrap();
        loader.send(page(5, held_page));
        loader.send(request(4, 1, "/during-load"));
        early.push(rx.recv_timeout(wait).expect("not held up by the load"));
        let tag = |d: &Done| match d {
            Done::Request { tab, id, body, .. } => {
                format!("request {tab}.{id} {}", String::from_utf8_lossy(body))
            }
            Done::Page { tab, result, .. } => format!("page {tab} {}", result.is_ok()),
            Done::Check { tab, .. } => format!("check {tab}"),
        };
        let mut early: Vec<String> = early.iter().map(tag).collect();
        early.sort();
        assert_eq!(
            early,
            [
                "page 3 true",
                "request 2.1 /other-tab",
                "request 4.1 /during-load"
            ]
        );
        assert!(rx.try_recv().is_err(), "tab 1 and tab 5 are still held");
        assert_eq!(loader.workers(), 3);

        // Released: tab 1's requests in the order it made them.
        release.send(()).unwrap();
        release.send(()).unwrap();
        let late: Vec<String> = (0..3)
            .map(|_| tag(&rx.recv_timeout(wait).unwrap()))
            .collect();
        let tab1: Vec<&String> = late
            .iter()
            .filter(|t| t.starts_with("request 1."))
            .collect();
        assert_eq!(
            tab1,
            ["request 1.1 /slow-poll", "request 1.2 /after"],
            "per-tab order"
        );
        assert!(
            late.contains(&"page 5 false".to_string()),
            "not a Vy page, but answered: {late:?}"
        );
        loader.close_tab(1);
        assert_eq!(loader.workers(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// SPEC-7.4.3, SPEC-7.4.5, SPEC-7.6.2: every reason `vy-proto` gives
    /// for restricted mode has its own short Swedish text for the chrome.
    #[test]
    fn spec_7_4_3_every_restriction_has_a_reason() {
        let id = || "0123456789abcdef".to_string();
        let name = || "_vy.exempel.se".to_string();
        let all = [
            Restriction::Unsigned,
            Restriction::BadSignature,
            Restriction::UnknownKey { key_id: id() },
            Restriction::KeyFileUnavailable("404".into()),
            Restriction::KeyNotPublished { key_id: id() },
            Restriction::DnsNoRecord { name: name() },
            Restriction::DnsMismatch { name: name() },
            Restriction::DnsFailed {
                name: name(),
                error: vy_proto::DnsError::Timeout,
            },
            Restriction::SuccessorNotPublished { key_id: id() },
            Restriction::LogInvalid("bruten kedja".into()),
            Restriction::NotInLog { key_id: id() },
            Restriction::LogRewritten,
            Restriction::LogWithdrawn,
        ];
        let reasons: std::collections::BTreeSet<&str> =
            all.iter().map(restriction_reason).collect();
        assert_eq!(reasons.len(), all.len(), "one text per reason");
        for r in &reasons {
            assert!(!r.is_empty() && r.chars().count() <= 36, "{r}");
        }
        assert_eq!(
            restriction_reason(&Restriction::DnsNoRecord { name: name() }),
            "nyckeln saknas i DNS"
        );
    }

    /// SPEC-7.4.3, SPEC-8.2.3: a local module or package runs in
    /// restricted mode with every verdict denied; a draft fills in the
    /// manifest (name, fallback, resources with sizes); text that is not
    /// an address is a path, and `https://` is never a source.
    #[test]
    fn spec_7_4_3_local_files_are_restricted() {
        let wasm = root().join("target/wasm/hello.wasm");
        let page = load_file(&wasm, None).unwrap();
        assert_eq!(page.display, "hello.wasm");
        assert!(page.manifest.is_none());
        assert!(!page.trusted);
        assert!(page.verdicts.is_empty());
        assert_eq!(page.sha256, vy_runtime::sha256(&page.wasm));
        assert_eq!(page.origin, None);

        let draft = root().join("conformance/manifests/shop.json");
        let shop = load_file(&root().join("target/wasm/shop.wasm"), Some(&draft)).unwrap();
        let m = shop.manifest.as_ref().unwrap();
        assert_eq!(m.name, "Vy-butiken");
        assert_eq!(shop.resources.len(), 8);
        assert!(shop.resources.iter().all(|(_, px)| px.is_some()));
        assert!(m.fallback.is_some());
        assert_eq!(shop.sha256, m.module.hash.0);
        let form = load_file(
            &root().join("target/wasm/form.wasm"),
            Some(&root().join("conformance/manifests/form.json")),
        )
        .unwrap();
        assert!(
            form.verdicts.iter().all(|(_, g)| *g == Grant::Denied),
            "{:?}",
            form.verdicts
        );
        assert!(form.verdicts.iter().any(|(n, _)| *n == "ime"));

        assert!(matches!(
            load_file(Path::new("/nonexistent/x.wasm"), None),
            Err(LoadError::File(_))
        ));
        let text = root().join("conformance/manifests/hello.json");
        let err = load_file(&text, None).unwrap_err();
        assert!(
            matches!(err, LoadError::File(ref s) if s.contains("neither a module nor a package")),
            "{err:?}"
        );

        assert_eq!(
            Source::parse(" vy://Docs.Example:443/a?b ").unwrap(),
            Source::Address(Address::parse("vy://docs.example/a?b").unwrap())
        );
        assert!(Source::parse("vy://x#frag").is_err());
        assert!(Source::parse("https://x.example").is_err());
        assert!(Source::parse("http://x.example").is_err());
        assert_eq!(
            Source::parse("target/wasm/hello.wasm").unwrap().display(),
            "hello.wasm"
        );
        assert_eq!(
            Source::parse("vy://docs.example/a").unwrap().origin(),
            Some("vy://docs.example".into())
        );
        let e = LoadError::NotVy {
            url: "https://x.example/".into(),
            media_type: "text/html".into(),
        };
        assert_eq!(e.external_url(), Some("https://x.example/"));
        assert!(e.detail().contains("SPEC-7.2.2"));
        assert_eq!(
            LoadError::Status {
                url: "u".into(),
                status: 404
            }
            .title(),
            "Sidan finns inte"
        );
    }
}
