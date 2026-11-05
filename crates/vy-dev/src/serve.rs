//! `vy-dev serve`: two origins on loopback and the threads behind them.
//!
//! Origin A, `vy://localhost:<port>/<name>`, is the page under development:
//! a `vy-serve` [`Site`] over `target/vy-dev/site/`, opened afresh after
//! every successful build (so the blobs of earlier builds are let go), its
//! manifests and packages served with `Cache-Control: no-cache` so a reload
//! always revalidates.
//!
//! Origin B, `vy://localhost:<studio-port>/`, is Studio and the dev API
//! under `/_dev/`. The two are different origins (SPEC-9.5.1: scheme, host
//! and port), so the page under development, whose `net` the default policy
//! grants for its own origin only (SPEC-8.2.3), cannot call the API that
//! builds and runs an agent in its project.
//!
//! Both bind `127.0.0.1` only. The developer key is served at
//! `/.well-known/vy-key` on both, which is what the client pins on first
//! visit (SPEC-7.4.2).

use crate::agent::AgentCommand;
use crate::dev::{self, Access, AgentConfig, ApiRequest, Dev};
use crate::paths;
use crate::project::Project;
use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use vy_manifest::{MEDIA_MANIFEST, MEDIA_PACKAGE};
use vy_serve::{Config, Reply, RequestInfo, Site};

/// How often the sources are polled for changes.
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Worker threads on the page origin.
pub const PAGE_WORKERS: usize = 4;
/// Worker threads on the Studio origin; a long-poll holds one.
pub const STUDIO_WORKERS: usize = 8;
/// Default port of the page under development.
pub const DEFAULT_PORT: u16 = 8080;
/// Default port of Studio and the dev API.
pub const DEFAULT_STUDIO_PORT: u16 = 8081;

/// What `vy-dev serve` takes.
#[derive(Clone, Debug)]
pub struct ServeConfig {
    /// The project directory.
    pub project_dir: PathBuf,
    /// Port of the page under development; 0 picks a free one.
    pub port: u16,
    /// Port of Studio and the dev API; 0 picks a free one.
    pub studio_port: u16,
    /// The developer key; `None` is `VY_DEV_KEY` or `<data_dir>/dev.key`,
    /// made on first run.
    pub key_path: Option<PathBuf>,
    /// `CARGO_TARGET_DIR` for the page's builds (and the agent's); `None`
    /// leaves it to cargo.
    pub target_dir: Option<PathBuf>,
    /// The agent command line; `None` is `VY_AGENT_CMD`, else `claude`.
    pub agent_cmd: Option<String>,
    /// Log every request to stderr.
    pub log: bool,
}

impl ServeConfig {
    /// Defaults for `project_dir`.
    pub fn new(project_dir: impl Into<PathBuf>) -> Self {
        Self {
            project_dir: project_dir.into(),
            port: DEFAULT_PORT,
            studio_port: DEFAULT_STUDIO_PORT,
            key_path: None,
            target_dir: None,
            agent_cmd: None,
            log: false,
        }
    }
}

/// A running daemon.
pub struct Handle {
    /// The state.
    pub dev: Arc<Dev>,
    /// Port of the page under development.
    pub port: u16,
    /// Port of Studio and the dev API.
    pub studio_port: u16,
    /// The key file used.
    pub key_path: PathBuf,
    /// True when this run made the key.
    pub key_created: bool,
    /// The key's id (SPEC-8.1.1).
    pub key_id: String,
    servers: Vec<Arc<tiny_http::Server>>,
    threads: Vec<JoinHandle<()>>,
}

impl Handle {
    /// The address of the page under development.
    pub fn address(&self) -> &str {
        &self.dev.address
    }

    /// The address of Studio.
    pub fn studio_address(&self) -> String {
        format!("vy://localhost:{}/", self.studio_port)
    }

    /// The dev API's URL for `path` (`/_dev/state`), with this run's token:
    /// what Studio fetches, for `curl` and the devtools.
    pub fn api_url(&self, path: &str) -> String {
        format!(
            "http://localhost:{}{}",
            self.studio_port,
            self.dev.access().with_token(path)
        )
    }

    /// Stop everything: the builder after its current build, the watcher,
    /// the servers.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.dev.shutdown();
        for s in &self.servers {
            // One `unblock` wakes one blocked `recv`.
            for _ in 0..PAGE_WORKERS.max(STUDIO_WORKERS) {
                s.unblock();
            }
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The site of origin A, replaced after every successful build.
struct PageSite {
    config: Config,
    site: Mutex<Arc<Site>>,
}

impl PageSite {
    fn open(config: Config) -> Result<Self> {
        let site = Site::open(&config).map_err(|e| anyhow!("{e}"))?;
        Ok(Self {
            config,
            site: Mutex::new(Arc::new(site)),
        })
    }

    fn reopen(&self) {
        match Site::open(&self.config) {
            Ok(site) => *self.site.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(site),
            Err(e) => eprintln!("vy-dev: reopening the page site: {e}"),
        }
    }

    fn current(&self) -> Arc<Site> {
        Arc::clone(&self.site.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Bind `127.0.0.1:<port>`; 0 picks a free port. Returns the server and
/// the port it got.
fn bind(port: u16) -> Result<(Arc<tiny_http::Server>, u16)> {
    let server = tiny_http::Server::http(("127.0.0.1", port)).map_err(|e| {
        anyhow!("cannot listen on 127.0.0.1:{port}: {e} (is another vy-dev running? pick another port with --port / --studio-port)")
    })?;
    let port = server
        .server_addr()
        .to_ip()
        .map(|a| a.port())
        .ok_or_else(|| anyhow!("the server has no IP address"))?;
    Ok((Arc::new(server), port))
}

/// A `vy-serve` site configuration for `dir` on `port` with the key file.
fn site_config(dir: &Path, port: u16, pub_path: &Path) -> Config {
    Config {
        dir: dir.to_path_buf(),
        bind: "127.0.0.1".into(),
        port,
        host_name: Some(format!("localhost:{port}")),
        public_key_file: Some(pub_path.to_path_buf()),
    }
}

/// Start the daemon: open the project and the key, bind both origins, and
/// start the builder (with a first build), the watcher and the workers.
pub fn start(config: ServeConfig) -> Result<Handle> {
    let mut project = Project::open(&config.project_dir)?;
    project.target_dir.clone_from(&config.target_dir);
    let key_path = paths::key_path(config.key_path.as_deref())?;
    let (key, key_created) = paths::load_or_create_key(&key_path)?;
    let pub_path = vy_publish::public_key_path(&key_path);

    let scratch = project.scratch_dir();
    let site_dir = scratch.join("site");
    let studio_dir = scratch.join("studio");
    for d in [&site_dir, &studio_dir] {
        std::fs::create_dir_all(d).with_context(|| format!("creating {}", d.display()))?;
    }
    // A package from an earlier run is not served: what this run builds is.
    let package_path = site_dir.join(format!("{}.vyp", project.name));
    let _ = std::fs::remove_file(&package_path);

    let (page_server, port) = bind(config.port)?;
    let (studio_server, studio_port) = bind(config.studio_port)?;
    let page_site = Arc::new(PageSite::open(site_config(&site_dir, port, &pub_path))?);
    let studio_site = Arc::new(
        Site::open(&site_config(&studio_dir, studio_port, &pub_path))
            .map_err(|e| anyhow!("{e}"))?,
    );

    let address = format!("vy://localhost:{port}/{}", project.name);
    let key_id = key.public().key_id();
    let mut env = vec![(
        paths::ENV_KEY.into(),
        std::path::absolute(&key_path)?.into_os_string(),
    )];
    if let Some(dir) = &project.target_dir {
        env.push(("CARGO_TARGET_DIR".into(), dir.clone().into_os_string()));
    }
    let agent = AgentConfig {
        cmd: AgentCommand::from_env_or(config.agent_cmd.as_deref())?,
        prepend_path: std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf)),
        env,
    };
    let access = Access::new(studio_port)?;
    let dev = Arc::new(Dev::new(project, key, package_path, address, agent, access));
    dev.request_build();

    let mut threads = Vec::new();
    {
        let dev = Arc::clone(&dev);
        let page_site = Arc::clone(&page_site);
        threads.push(std::thread::spawn(move || {
            dev.builder_loop(|outcome| {
                if outcome.ok() {
                    page_site.reopen();
                }
            })
        }));
    }
    {
        let dev = Arc::clone(&dev);
        threads.push(std::thread::spawn(move || watcher_loop(&dev)));
    }
    for _ in 0..PAGE_WORKERS {
        let server = Arc::clone(&page_server);
        let site = Arc::clone(&page_site);
        let log = config.log;
        threads.push(std::thread::spawn(move || {
            while let Ok(request) = server.recv() {
                serve_page(&site.current(), request, log);
            }
        }));
    }
    for _ in 0..STUDIO_WORKERS {
        let server = Arc::clone(&studio_server);
        let site = Arc::clone(&studio_site);
        let dev = Arc::clone(&dev);
        let log = config.log;
        threads.push(std::thread::spawn(move || {
            while let Ok(request) = server.recv() {
                serve_studio(&site, &dev, request, log);
            }
        }));
    }
    Ok(Handle {
        dev,
        port,
        studio_port,
        key_path,
        key_created,
        key_id,
        servers: vec![page_server, studio_server],
        threads,
    })
}

/// Poll the sources every [`POLL_INTERVAL`]. Sources that stayed put for
/// one interval (an editor's save of several files is one build) and
/// differ from what the last build started from request a build. While
/// the agent runs nothing is requested: it builds for itself, and its exit
/// requests the build that counts, which then covers its edits.
fn watcher_loop(dev: &Dev) {
    let mut last = dev.project.fingerprint();
    loop {
        std::thread::sleep(POLL_INTERVAL);
        if dev.stopping() {
            return;
        }
        let now = dev.project.fingerprint();
        let stable = now == last;
        if stable {
            dev.with(|s| {
                if !s.agent_running && !s.build_requested && s.built.as_ref() != Some(&now) {
                    s.build_requested = true;
                }
            });
        }
        last = now;
    }
}

/// What the router needs from a `tiny_http` request. `Host` is taken as
/// sent: `X-Forwarded-Host` is a header any client can add, and the dev
/// API decides on the name the request was addressed to.
fn request_info(request: &tiny_http::Request) -> RequestInfo {
    let header = |name: &'static str| -> Option<String> {
        request
            .headers()
            .iter()
            .find(|h| h.field.equiv(name))
            .map(|h| h.value.to_string())
    };
    RequestInfo {
        method: request.method().as_str().to_string(),
        url: request.url().to_string(),
        accept: header("Accept"),
        if_none_match: header("If-None-Match"),
        host: header("Host"),
    }
}

/// Answer a `tiny_http` request with a [`Reply`].
fn send(request: tiny_http::Request, reply: Reply) {
    let mut response = tiny_http::Response::from_data(reply.body)
        .with_status_code(tiny_http::StatusCode(reply.status));
    for (name, value) in &reply.headers {
        if let Ok(h) = tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()) {
            response = response.with_header(h);
        }
    }
    let _ = request.respond(response);
}

/// Manifests and packages change under development, so a client's HTTP
/// cache must not keep one past a rebuild (SPEC-7.3.2): `no-cache` keeps
/// the `ETag` revalidation and drops the freshness window.
pub fn fresh(mut reply: Reply) -> Reply {
    let content_type = reply.get("content-type").unwrap_or("").to_string();
    if content_type == MEDIA_MANIFEST || content_type == MEDIA_PACKAGE || reply.status == 304 {
        let mut found = false;
        for (name, value) in &mut reply.headers {
            if name.eq_ignore_ascii_case("cache-control") {
                *value = "no-cache".into();
                found = true;
            }
        }
        if !found {
            reply
                .headers
                .push(("Cache-Control".into(), "no-cache".into()));
        }
    }
    reply
}

fn log_line(log: bool, method: &str, url: &str, status: u16, len: usize) {
    if log {
        eprintln!("{method} {url} {status} {len}");
    }
}

fn serve_page(site: &Site, request: tiny_http::Request, log: bool) {
    let info = request_info(&request);
    let reply = fresh(site.reply(&info));
    log_line(log, &info.method, &info.url, reply.status, reply.body.len());
    send(request, reply);
}

fn serve_studio(site: &Site, dev: &Arc<Dev>, mut request: tiny_http::Request, log: bool) {
    let info = request_info(&request);
    let path = info.url.split('?').next().unwrap_or("/");
    if !dev::is_api_path(path) {
        let reply = fresh(site.reply(&info));
        log_line(log, &info.method, &info.url, reply.status, reply.body.len());
        send(request, reply);
        return;
    }
    let browser_like = request
        .headers()
        .iter()
        .any(|h| dev::is_browser_header(h.field.as_str().as_str()));
    let mut body = Vec::new();
    {
        use std::io::Read;
        // One byte past the cap is enough to tell "too long".
        let _ = request
            .as_reader()
            .take(dev::MAX_BODY_BYTES as u64 + 1)
            .read_to_end(&mut body);
    }
    let api = ApiRequest {
        method: info.method.clone(),
        path: info.url.clone(),
        body,
        host: info.host.clone(),
        browser_like,
    };
    let reply = dev.api(&api);
    log_line(log, &info.method, &info.url, reply.status, reply.body.len());
    send(
        request,
        Reply {
            status: reply.status,
            headers: vec![
                (
                    "Content-Type".into(),
                    "application/json; charset=utf-8".into(),
                ),
                ("Cache-Control".into(), "no-store".into()),
            ],
            body: reply.body,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_rewrites_cache_control_for_pages_only() {
        let page = Reply {
            status: 200,
            headers: vec![
                ("Content-Type".into(), MEDIA_PACKAGE.into()),
                ("Cache-Control".into(), "max-age=60".into()),
            ],
            body: vec![],
        };
        assert_eq!(fresh(page).get("cache-control"), Some("no-cache"));
        let not_modified = Reply {
            status: 304,
            headers: vec![("ETag".into(), "\"x\"".into())],
            body: vec![],
        };
        assert_eq!(fresh(not_modified).get("cache-control"), Some("no-cache"));
        let html = Reply {
            status: 200,
            headers: vec![
                ("Content-Type".into(), "text/html; charset=utf-8".into()),
                ("Cache-Control".into(), "max-age=60".into()),
            ],
            body: vec![],
        };
        assert_eq!(fresh(html).get("cache-control"), Some("max-age=60"));
    }
}
