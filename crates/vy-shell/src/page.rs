//! Getting a page: from a `vy://` address through `vy-proto` (SPEC §7),
//! or from a local `.vyp` package or bare `.wasm` for development; the
//! trust decision and the default capability verdicts (SPEC-7.4.3,
//! SPEC-8.2.3). Runs off the UI thread, since a fetch can take seconds:
//! pages on the loader's workers, one origin at a time on each, and a
//! page's own requests on its tab's worker.

use crate::ipc::ResourceBytes;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use vy_manifest::{Draft, Grant, Manifest, Package};
use vy_proto::{
    Address, Error as FetchError, Fetcher, NetClass, Options as FetchOptions, Requester,
    Restriction,
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
    /// The class of the page's own address as its fetch found it
    /// (`Fetched::net_class`, SPEC-9.5.3): what the page's own requests
    /// may reach. A local file, which makes no requests, is public, the
    /// class that reaches the least.
    pub net_class: NetClass,
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
pub fn load(fetcher: &Fetcher, source: &Source) -> Result<LoadedPage, LoadError> {
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
            let net_class = fetched.net_class();
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
                net_class,
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
                    net_class: NetClass::Public,
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
        net_class: NetClass::Public,
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
        /// The class of the page's own address (SPEC-9.5.3,
        /// [`LoadedPage::net_class`]).
        page: NetClass,
    },
}

impl Job {
    /// True for a page load or a live-reload check of `tab` made for a
    /// navigation before `generation`: one whose result the shell would
    /// drop.
    fn is_older(&self, tab: u32, generation: u64) -> bool {
        match self {
            Job::Page {
                tab: t,
                generation: g,
                ..
            }
            | Job::Check(Check {
                tab: t,
                generation: g,
                ..
            }) => *t == tab && *g < generation,
            Job::Request { .. } => false,
        }
    }
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

/// How many page loads and live-reload checks run at once, each for a
/// different origin.
pub const LOADER_WORKERS: usize = 4;

/// The loader (SPEC-5.2.7, SPEC-7.4.5): page loads and live-reload checks
/// run on [`LOADER_WORKERS`] threads that share one [`Fetcher`] with its
/// caches and key store. Jobs for different origins run side by side, so
/// an origin whose server is slow or whose first visit's DNS lookup hangs
/// (up to 6 s) holds up no other origin's page and no live-reload check;
/// jobs for one origin run one at a time in the order they were asked for,
/// and never take a second worker, since the trust decision for an origin
/// is made by one fetch at a time (the fetcher holds to that by itself).
/// A page's own requests run on a worker thread per tab, in the order the
/// tab made them, so a slow request (a long poll, a slow server) holds up
/// neither another tab's requests nor any page load, and a page load holds
/// up no page's requests.
pub struct Loader {
    queue: Arc<Queue>,
    requester: Requester,
    report: Report,
    workers: HashMap<u32, Sender<Job>>,
}

type Report = Arc<dyn Fn(Done) + Send + Sync>;

/// The page loads and checks waiting for a worker, with the origin each
/// is for (a local file has none), and the origins a worker is on.
struct Queue {
    state: Mutex<QueueState>,
    changed: Condvar,
}

#[derive(Default)]
struct QueueState {
    jobs: VecDeque<(Option<String>, Job)>,
    busy: HashSet<String>,
    closed: bool,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Queue {
    /// The next job a worker may take: the oldest whose origin no worker
    /// is on, which that origin then is until [`Queue::done`]. Waits for
    /// one; `None` once the loader is gone.
    fn take(&self) -> Option<(Option<String>, Job)> {
        let mut state = lock(&self.state);
        loop {
            if state.closed {
                return None;
            }
            let free = state
                .jobs
                .iter()
                .position(|(origin, _)| origin.as_ref().is_none_or(|o| !state.busy.contains(o)));
            if let Some(job) = free.and_then(|i| state.jobs.remove(i)) {
                if let Some(origin) = &job.0 {
                    state.busy.insert(origin.clone());
                }
                return Some(job);
            }
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// The job for `origin` is finished: its next one may start.
    fn done(&self, origin: Option<&String>) {
        if let Some(origin) = origin {
            lock(&self.state).busy.remove(origin);
            self.changed.notify_all();
        }
    }
}

impl Drop for Loader {
    /// The workers end once the job each is on is done; what waits is
    /// dropped with the shell that asked for it.
    fn drop(&mut self) {
        lock(&self.queue.state).closed = true;
        self.queue.changed.notify_all();
    }
}

impl Loader {
    /// Hand `job` to a thread that does it: a request to its tab's worker
    /// (started at the tab's first request), anything else to the loader's
    /// workers, queued behind what is already waiting for its origin.
    pub fn send(&mut self, job: Job) {
        let tab = match &job {
            Job::Request { tab, .. } => *tab,
            Job::Page {
                tab,
                generation,
                source,
            } => {
                let origin = source.origin();
                let (tab, generation) = (*tab, *generation);
                let mut state = lock(&self.queue.state);
                // What still waits for an earlier navigation of this tab
                // is for nothing: the shell drops its result (granskning
                // 0014:6). Five reloads of a page whose server hangs are
                // then the fetch under way and the last one, not five in
                // a row. The job a worker is on runs to its end.
                state
                    .jobs
                    .retain(|(_, queued)| !queued.is_older(tab, generation));
                state.jobs.push_back((origin, job));
                drop(state);
                self.queue.changed.notify_all();
                return;
            }
            Job::Check(check) => {
                let origin = Some(check.address.origin());
                lock(&self.queue.state).jobs.push_back((origin, job));
                self.queue.changed.notify_all();
                return;
            }
        };
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

    /// The tab is gone: its worker ends once its queued requests are
    /// done, and its page loads and checks that still wait are dropped.
    pub fn close_tab(&mut self, tab: u32) {
        self.workers.remove(&tab);
        lock(&self.queue.state)
            .jobs
            .retain(|(_, queued)| !queued.is_older(tab, u64::MAX));
    }

    /// Tabs with a request worker, for tests.
    pub fn workers(&self) -> usize {
        self.workers.len()
    }

    /// Make the page's requests with `requester` from now on (tests: one
    /// from `vy_proto::test_net`); the tabs' workers start anew.
    #[cfg(test)]
    pub(crate) fn set_requester(&mut self, requester: Requester) {
        self.requester = requester;
        self.workers.clear();
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
                    page,
                } = job
                {
                    let outcome = requester.request(
                        method,
                        &url,
                        &body,
                        page,
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

/// What a loader worker does with a job: the work, and what it found.
type Work = Arc<dyn Fn(Job) -> Option<Done> + Send + Sync>;

/// A job a worker is on (granskning 0014:5). Dropped, it reports the job
/// as failed unless the job was finished, and then frees the job's origin
/// for its next job: a job whose code panics still answers its tab, with
/// an error instead of a page that loads for ever, and holds up no later
/// page of its origin.
struct Running<'a> {
    queue: &'a Queue,
    origin: Option<String>,
    /// What the tab is told when the job does not finish; `None` once it
    /// has.
    failed: Option<Done>,
    report: &'a Report,
}

impl Running<'_> {
    /// The job is done: report what it found. Its origin is freed after
    /// the report, so one origin's answers come in the order asked for.
    fn finish(mut self, done: Option<Done>) {
        self.failed = None;
        if let Some(done) = done {
            (self.report)(done);
        }
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        if let Some(failed) = self.failed.take() {
            // Mid-unwind already: a report that panics too must not take
            // the process down, nor keep the origin.
            let report = self.report;
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| report(failed)));
        }
        self.queue.done(self.origin.as_ref());
    }
}

/// What a tab is told about a job that did not finish.
fn failure(job: &Job) -> Option<Done> {
    const WHY: &str = "internal error: the loader failed on this page; loading it again may work";
    match job {
        Job::Page {
            tab,
            generation,
            source,
        } => Some(Done::Page {
            tab: *tab,
            generation: *generation,
            result: Box::new(Err(match source {
                Source::Address(_) => LoadError::Fetch(WHY.into()),
                Source::File { .. } => LoadError::File(WHY.into()),
            })),
        }),
        Job::Check(check) => Some(Done::Check {
            tab: check.tab,
            generation: check.generation,
            result: Err(WHY.into()),
        }),
        Job::Request { .. } => None,
    }
}

/// Start the loader over the caches under `cache_dir`, reporting through
/// `report` (which also wakes the UI), from whichever thread did the job.
pub fn start_loader(
    cache_dir: PathBuf,
    report: impl Fn(Done) + Send + Sync + 'static,
) -> std::io::Result<Loader> {
    let work = loader_work(&cache_dir);
    start_loader_with(cache_dir, Arc::new(report), work)
}

/// The loader's work on a page load or a live-reload check, over the
/// caches under `cache_dir`.
fn loader_work(cache_dir: &Path) -> Work {
    // Opened by the first job that fetches, and again by the next one if
    // the cache directory could not be used: a file needs no caches and
    // loads all the same.
    let fetcher: Mutex<Option<Arc<Fetcher>>> = Mutex::new(None);
    let cache_dir = cache_dir.to_path_buf();
    Arc::new(move |job| run_job(job, &fetcher, &cache_dir))
}

/// [`start_loader`] with the workers doing `work` on each job (tests: a
/// job whose work panics).
fn start_loader_with(cache_dir: PathBuf, report: Report, work: Work) -> std::io::Result<Loader> {
    let requester = Requester::new(&FetchOptions::new(&cache_dir));
    let queue = Arc::new(Queue {
        state: Mutex::new(QueueState::default()),
        changed: Condvar::new(),
    });
    for n in 0..LOADER_WORKERS {
        let (queue, report, work) = (queue.clone(), report.clone(), work.clone());
        let started = thread::Builder::new()
            .name(format!("vy-loader-{n}"))
            .spawn(move || {
                while let Some((origin, job)) = queue.take() {
                    // A job that panics ends neither the worker nor its
                    // origin's turn: the guard answers for it, and the
                    // worker takes the next job.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let running = Running {
                            queue: &queue,
                            origin,
                            failed: failure(&job),
                            report: &report,
                        };
                        let done = work(job);
                        running.finish(done);
                    }));
                }
            });
        // One worker is a loader; fewer than asked for is a slower one.
        if let (Err(e), 0) = (started, n) {
            return Err(e);
        }
    }
    Ok(Loader {
        queue,
        requester,
        report,
        workers: HashMap::new(),
    })
}

/// The loader's fetcher, opened at first use.
fn open_fetcher(
    shared: &Mutex<Option<Arc<Fetcher>>>,
    cache_dir: &Path,
) -> Result<Arc<Fetcher>, String> {
    let mut slot = lock(shared);
    if let Some(fetcher) = slot.as_ref() {
        return Ok(fetcher.clone());
    }
    let fetcher = Arc::new(Fetcher::new(FetchOptions::new(cache_dir)).map_err(|e| e.to_string())?);
    *slot = Some(fetcher.clone());
    Ok(fetcher)
}

/// Do one page load or live-reload check; what it found is for the tab.
fn run_job(job: Job, fetcher: &Mutex<Option<Arc<Fetcher>>>, cache_dir: &Path) -> Option<Done> {
    match job {
        Job::Page {
            tab,
            generation,
            source,
        } => {
            // A file needs no caches; it loads even when the cache
            // directory is unusable.
            let result = match &source {
                Source::File { .. } => load_file_source(&source),
                Source::Address(_) => match open_fetcher(fetcher, cache_dir) {
                    Ok(fetcher) => load(&fetcher, &source),
                    Err(e) => Err(LoadError::Fetch(format!("cache: {e}"))),
                },
            };
            Some(Done::Page {
                tab,
                generation,
                result: Box::new(result),
            })
        }
        Job::Check(Check {
            tab,
            generation,
            address,
        }) => {
            let result = open_fetcher(fetcher, cache_dir).and_then(|fetcher| {
                fetcher
                    .revalidate(&address)
                    .map(|h| h.0)
                    .map_err(|e| e.to_string())
            });
            Some(Done::Check {
                tab,
                generation,
                result,
            })
        }
        // Requests go to their tab's worker (`Loader::send`).
        Job::Request { .. } => None,
    }
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
        let (port, release, _) = held_server_logged();
        (port, release)
    }

    /// [`held_server`], and the paths it was asked for, in the order the
    /// requests arrived (a held one is listed when it arrives).
    fn held_server_logged() -> (u16, std::sync::mpsc::Sender<()>, Arc<Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release, held) = std::sync::mpsc::channel::<()>();
        let held = Arc::new(std::sync::Mutex::new(held));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let held = held.clone();
                let log = log.clone();
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
                    lock(&log).push(path.clone());
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
        (port, release, seen)
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
            page: NetClass::Loopback,
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

    /// SPEC-7.4.5, SPEC-9.5.1 (granskning 0009:2): page loads of different
    /// origins do not wait for each other. While origin A's server holds
    /// its page open (as a first visit's DNS lookup can, for up to 6 s),
    /// five more pages of origin A queue behind it, more than there are
    /// workers, and still a page of origin B and a live-reload check of it
    /// are answered: jobs of one origin take one worker between them. Once
    /// A's server answers, A's pages come in the order they were asked for.
    #[test]
    fn spec_7_4_5_page_loads_do_not_wait_for_other_origins() {
        let (held_port, release) = held_server();
        let (other_port, _other) = held_server();
        let (tx, rx) = std::sync::mpsc::channel::<Done>();
        let tx = std::sync::Mutex::new(tx);
        let dir = std::env::temp_dir().join(format!("vy-loader-origins-{}", std::process::id()));
        let mut loader = start_loader(dir.clone(), move |d| {
            let _ = tx.lock().unwrap().send(d);
        })
        .unwrap();
        let address = |port: u16, path: &str| {
            Address::parse(&format!("vy://127.0.0.1:{port}{path}")).unwrap()
        };
        let page = |tab: u32, port: u16, path: &str| Job::Page {
            tab,
            generation: 1,
            source: Source::Address(address(port, path)),
        };
        let tag = |d: &Done| match d {
            Done::Page { tab, .. } => format!("page {tab}"),
            Done::Check { tab, .. } => format!("check {tab}"),
            Done::Request { tab, id, .. } => format!("request {tab}.{id}"),
        };
        // The queue below is longer than the pool.
        const { assert!(LOADER_WORKERS < 6) };
        loader.send(page(1, held_port, "/slow-page"));
        for tab in 2..=6 {
            loader.send(page(tab, held_port, "/after"));
        }
        loader.send(page(7, other_port, "/page"));
        let wait = std::time::Duration::from_secs(20);
        let first = rx
            .recv_timeout(wait)
            .expect("origin B's page is not held up");
        assert_eq!(tag(&first), "page 7");
        loader.send(Job::Check(Check {
            tab: 7,
            generation: 1,
            address: address(other_port, "/page"),
        }));
        let second = rx.recv_timeout(wait).expect("nor is its live-reload check");
        assert_eq!(tag(&second), "check 7");
        assert!(rx.try_recv().is_err(), "origin A's pages are still waiting");

        release.send(()).unwrap();
        let late: Vec<String> = (0..6)
            .map(|_| tag(&rx.recv_timeout(wait).expect("origin A's pages")))
            .collect();
        assert_eq!(
            late,
            ["page 1", "page 2", "page 3", "page 4", "page 5", "page 6"],
            "one origin's pages in the order they were asked for"
        );
        drop(loader);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// SPEC-7.4.5 (granskning 0014:6): a page load that still waits when
    /// its tab navigates again is dropped, not fetched. While origin A's
    /// server holds a tab's first load, the tab is reloaded four times,
    /// another tab asks for a page of A, and a third asks and is closed;
    /// what waited for the tab's earlier navigations and for the closed
    /// tab is never asked of the server. Once the server answers: the
    /// load that was under way, the other tab's page and the tab's last
    /// navigation, in that order, and nothing else.
    #[test]
    fn spec_7_4_5_stale_page_loads_are_dropped_from_the_queue() {
        let (port, release, seen) = held_server_logged();
        let (tx, rx) = std::sync::mpsc::channel::<Done>();
        let tx = std::sync::Mutex::new(tx);
        let dir = std::env::temp_dir().join(format!("vy-loader-stale-{}", std::process::id()));
        let mut loader = start_loader(dir.clone(), move |d| {
            let _ = tx.lock().unwrap().send(d);
        })
        .unwrap();
        let address = |path: &str| Address::parse(&format!("vy://127.0.0.1:{port}{path}")).unwrap();
        let page = |tab: u32, generation: u64, path: &str| Job::Page {
            tab,
            generation,
            source: Source::Address(address(path)),
        };
        let wait = std::time::Duration::from_secs(20);
        let asked = || -> Vec<String> { lock(&seen).clone() };

        loader.send(page(1, 1, "/slow-first"));
        let deadline = std::time::Instant::now() + wait;
        while asked().is_empty() {
            assert!(
                std::time::Instant::now() < deadline,
                "the first load never started"
            );
            thread::sleep(std::time::Duration::from_millis(5));
        }
        // Four reloads, a check for the first navigation, another tab's
        // page in between, and a tab that is closed while it waits.
        loader.send(page(1, 2, "/reload-2"));
        loader.send(page(1, 3, "/reload-3"));
        loader.send(Job::Check(Check {
            tab: 1,
            generation: 3,
            address: address("/check-3"),
        }));
        loader.send(page(2, 1, "/other-tab"));
        loader.send(page(3, 1, "/closed-tab"));
        loader.send(page(1, 4, "/reload-4"));
        loader.close_tab(3);
        loader.send(page(1, 5, "/reload-5"));
        assert!(rx.try_recv().is_err(), "all of it waits for origin A");
        assert_eq!(asked(), ["/slow-first"]);

        release.send(()).unwrap();
        let done: Vec<(u32, u64)> = (0..3)
            .map(|_| match rx.recv_timeout(wait).expect("a page") {
                Done::Page {
                    tab, generation, ..
                } => (tab, generation),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(done, [(1, 1), (2, 1), (1, 5)]);
        // A page asked for now comes next: nothing else was waiting.
        loader.send(page(4, 1, "/last"));
        match rx.recv_timeout(wait).expect("the last page") {
            Done::Page { tab: 4, .. } => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(
            asked(),
            ["/slow-first", "/other-tab", "/reload-5", "/last"],
            "what waited for an earlier navigation was never fetched"
        );
        drop(loader);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// SPEC-7.4.5, SPEC-9.5.1 (granskning 0014:5): a loader worker whose
    /// job panics answers for it and goes on. The tab of the job is told
    /// its page failed, a later page of the same origin is loaded (the
    /// origin is not left marked as being worked on), a live-reload check
    /// that panics is answered too, and after more panics than there are
    /// workers the loader still loads.
    #[test]
    fn spec_7_4_5_a_loader_panic_fails_the_page_and_frees_its_origin() {
        const BAD: u32 = 13;
        let (port, _held) = held_server();
        let (tx, rx) = std::sync::mpsc::channel::<Done>();
        let tx = std::sync::Mutex::new(tx);
        let dir = std::env::temp_dir().join(format!("vy-loader-panic-{}", std::process::id()));
        let real = loader_work(&dir);
        // The jobs of tab 13 and up panic.
        let work: Work = Arc::new(move |job| match &job {
            Job::Page { tab, .. } | Job::Check(Check { tab, .. }) if *tab >= BAD => {
                panic!("a fault in the loader's code (this test's)")
            }
            _ => real(job),
        });
        let report: Report = Arc::new(move |d| {
            let _ = tx.lock().unwrap().send(d);
        });
        let mut loader = start_loader_with(dir.clone(), report, work).unwrap();
        let address = |path: &str| Address::parse(&format!("vy://127.0.0.1:{port}{path}")).unwrap();
        let page = |tab: u32, generation: u64, path: &str| Job::Page {
            tab,
            generation,
            source: Source::Address(address(path)),
        };
        let wait = std::time::Duration::from_secs(20);

        // The job panics: its tab is told, with the navigation it was for.
        loader.send(page(BAD, 7, "/page"));
        match rx.recv_timeout(wait).expect("the panicked job is answered") {
            Done::Page {
                tab: BAD,
                generation: 7,
                result,
            } => match *result {
                Err(LoadError::Fetch(why)) => assert!(why.contains("internal error"), "{why}"),
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
        // The origin is free: its next page is loaded (not a Vy page, but
        // answered by the server).
        loader.send(page(1, 1, "/after"));
        match rx.recv_timeout(wait).expect("the origin is not held") {
            Done::Page { tab: 1, result, .. } => {
                assert!(
                    matches!(*result, Err(LoadError::NotVy { .. })),
                    "{result:?}"
                )
            }
            other => panic!("{other:?}"),
        }
        // A check that panics is answered, so live reload asks again.
        loader.send(Job::Check(Check {
            tab: BAD,
            generation: 8,
            address: address("/page"),
        }));
        match rx
            .recv_timeout(wait)
            .expect("the panicked check is answered")
        {
            Done::Check {
                tab: BAD,
                generation: 8,
                result,
            } => assert!(result.is_err()),
            other => panic!("{other:?}"),
        }
        // More panics than workers (a tab each, a local file among
        // them), then a page.
        let hello = root().join("target/wasm/hello.wasm");
        for n in 0..2 * LOADER_WORKERS as u32 {
            loader.send(page(100 + n, 1, "/again"));
        }
        loader.send(Job::Page {
            tab: 200,
            generation: 1,
            source: Source::File {
                path: hello.clone(),
                draft: None,
            },
        });
        let mut failed = Vec::new();
        for _ in 0..2 * LOADER_WORKERS + 1 {
            match rx
                .recv_timeout(wait)
                .expect("every panicked job is answered")
            {
                Done::Page { tab, result, .. } => {
                    assert!(result.is_err());
                    failed.push(tab);
                }
                other => panic!("{other:?}"),
            }
        }
        failed.sort();
        let mut want: Vec<u32> = (100..100 + 2 * LOADER_WORKERS as u32).collect();
        want.push(200);
        assert_eq!(failed, want);
        loader.send(Job::Page {
            tab: 2,
            generation: 1,
            source: Source::File {
                path: hello,
                draft: None,
            },
        });
        match rx.recv_timeout(wait).expect("the workers are still there") {
            Done::Page { tab: 2, result, .. } => assert!(result.is_ok()),
            other => panic!("{other:?}"),
        }
        drop(loader);
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
