//! Getting a page: from a `vy://` address through `vy-proto` (SPEC §7),
//! or from a local `.vyp` package or bare `.wasm` for development; the
//! trust decision and the default capability verdicts (SPEC-7.4.3,
//! SPEC-8.2.3). Runs on the loader thread, never on the UI thread, since
//! a fetch can take seconds.

use crate::ipc::ResourceBytes;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;
use vy_manifest::{Draft, Grant, Manifest, Package};
use vy_proto::{Address, Error as FetchError, Fetcher, Options as FetchOptions};

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
        verdicts,
        is_localhost: false,
    })
}

/// A job for the loader thread.
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

/// What the loader thread reports back.
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

/// Start the loader thread over the caches under `cache_dir`, reporting
/// through `report` (which also wakes the UI). Returns the job sender.
pub fn start_loader(
    cache_dir: PathBuf,
    report: impl Fn(Done) + Send + 'static,
) -> std::io::Result<Sender<Job>> {
    let (tx, rx) = std::sync::mpsc::channel::<Job>();
    thread::Builder::new()
        .name("vy-loader".into())
        .spawn(move || {
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
                    Job::Request {
                        tab,
                        generation,
                        id,
                        method,
                        url,
                        body,
                        page_is_localhost,
                    } => {
                        let outcome = match open(&mut fetcher) {
                            Ok(()) => fetcher.as_ref().expect("opened").request(
                                method,
                                &url,
                                &body,
                                page_is_localhost,
                                u64::from(vy_abi::limits::MAX_FETCH_RESPONSE_BYTES) + 1,
                            ),
                            Err(e) => Err(FetchError::Io(e)),
                        };
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
            }
        })?;
    Ok(tx)
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
