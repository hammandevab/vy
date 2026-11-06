//! The daemon's state and the dev API over it.
//!
//! One [`Dev`] per `vy-dev serve`: the build status and its log behind a
//! mutex, with a condvar that wakes the builder and anyone waiting. The API
//! is a pure function of a request ([`Dev::api`]), so it is tested without
//! sockets; `serve` wraps it in `tiny_http`.
//!
//! Who may call the API matters (see `docs/notes/m4-dev.md`, *Hotbild*):
//! `serve` binds loopback only, and [`Access::refuse_foreign`] refuses,
//! on every path of the Studio origin, a request that looks like a
//! browser's (an `Origin`, `Referer` or `Sec-Fetch-*` header, which a web
//! page's request carries and a Vy client's never does) and one whose
//! `Host` is not `localhost:<port>` or `127.0.0.1:<port>` of the API (DNS
//! rebinding); [`Access::guard`] adds this run's random token for the API,
//! which `serve` writes into the Studio module it serves and nowhere a page
//! can read it. The page under development is served on another origin,
//! and `net` is granted automatically to a page's own origin only
//! (SPEC-8.2.3, SPEC-9.5.1).

use crate::agent::{self, AgentCommand, Launch};
use crate::build::{build_project, BuildOutcome};
use crate::log::{Entry, Kind, Log};
use crate::project::{Fingerprint, Project};
use serde_json::{json, Value};
use std::ffi::OsString;
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use vy_manifest::SigningKey;

/// Path prefix of the dev API on the Studio origin.
pub const API_PREFIX: &str = "/_dev/";
/// Largest request body the API reads; a longer mission is refused.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
/// Longest a long-poll waits, in milliseconds (`wait` is capped here and
/// is the default).
pub const MAX_WAIT_MS: u64 = 20_000;
/// Most entries one `GET /_dev/agent/events` returns (`limit` is capped
/// here and is the default): the newest of them.
pub const MAX_EVENTS: usize = 1000;

/// Where the build is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildStatus {
    /// No build has run yet.
    Idle,
    /// A build is running.
    Building,
    /// The last build produced a package.
    Ok,
    /// The last build failed; the log says why.
    Error,
}

impl BuildStatus {
    /// The name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            BuildStatus::Idle => "idle",
            BuildStatus::Building => "building",
            BuildStatus::Ok => "ok",
            BuildStatus::Error => "error",
        }
    }
}

/// The mutable state.
#[derive(Debug)]
pub struct Inner {
    /// Build status.
    pub build: BuildStatus,
    /// Log of the last build, or of the one running so far.
    pub build_log: String,
    /// Builds finished, successful or not.
    pub builds: u32,
    /// A build is wanted as soon as the builder is free.
    pub build_requested: bool,
    /// The sources the last build started from, taken before cargo read
    /// them: the watcher asks for a build only when they differ.
    pub built: Option<Fingerprint>,
    /// The agent is running.
    pub agent_running: bool,
    /// Its process id while running.
    pub agent_pid: Option<u32>,
    /// Exit code of the last run; `None` before the first, and when it
    /// was stopped or ended by a signal.
    pub last_exit: Option<i32>,
    /// A stop was asked for the running agent.
    pub stop_requested: bool,
    /// What the agent said and did, and what the builds did.
    pub log: Log,
    /// The daemon is shutting down.
    pub stopping: bool,
}

/// The daemon's state for one project.
pub struct Dev {
    /// The project.
    pub project: Project,
    /// The address of the page under development.
    pub address: String,
    key: SigningKey,
    package_path: PathBuf,
    agent: AgentConfig,
    access: Access,
    inner: Mutex<Inner>,
    changed: Condvar,
}

/// Who may call the API: requests addressed to `localhost:<port>` or
/// `127.0.0.1:<port>` that carry `token=<token>` in their query.
#[derive(Clone, Debug)]
pub struct Access {
    /// The API's port, the only one a `Host` may name.
    pub port: u16,
    /// This run's token: 32 hex characters from OS entropy.
    pub token: String,
}

impl Access {
    /// A fresh token for the API on `port`.
    pub fn new(port: u16) -> anyhow::Result<Self> {
        let mut bytes = [0u8; 16];
        getrandom::getrandom(&mut bytes).map_err(|e| anyhow::anyhow!("no entropy: {e}"))?;
        Ok(Self {
            port,
            token: bytes.iter().map(|b| format!("{b:02x}")).collect(),
        })
    }

    /// `path` with this run's token added to its query.
    pub fn with_token(&self, path: &str) -> String {
        let sep = if path.contains('?') { '&' } else { '?' };
        format!("{path}{sep}token={}", self.token)
    }

    /// A request as the Vy client makes it: to `localhost:<port>`, with
    /// the token.
    pub fn request(&self, method: &str, path: &str, body: &[u8]) -> ApiRequest {
        ApiRequest {
            method: method.into(),
            path: self.with_token(path),
            body: body.to_vec(),
            host: Some(format!("localhost:{}", self.port)),
            browser_like: false,
        }
    }

    /// The refusal for a request that must not reach the Studio origin at
    /// all, or `None`: a browser's (`browser_like`, [`is_browser_header`])
    /// and one whose `Host` is not `localhost:<port>` or
    /// `127.0.0.1:<port>` ([`host_matches`], DNS rebinding). Every path on
    /// the Studio origin passes this first, not only the API's: the Studio
    /// package and its module carry this run's token.
    pub fn refuse_foreign(&self, host: Option<&str>, browser_like: bool) -> Option<ApiReply> {
        let refuse = |why: &str| Some(ApiReply::json(403, json!({ "error": why })));
        if browser_like {
            return refuse("the Studio origin answers the Vy client, not web pages");
        }
        if !host_matches(host, self.port) {
            return refuse(
                "the Studio origin answers to localhost:<port> and 127.0.0.1:<port> only",
            );
        }
        None
    }

    /// The refusal for a request that must not reach the API, or `None`:
    /// [`Access::refuse_foreign`], then this run's token.
    pub fn guard(&self, req: &ApiRequest) -> Option<ApiReply> {
        if let Some(refused) = self.refuse_foreign(req.host.as_deref(), req.browser_like) {
            return Some(refused);
        }
        let refuse = |why: &str| Some(ApiReply::json(403, json!({ "error": why })));
        let query = req.path.split_once('?').map_or("", |(_, q)| q);
        let token = query_value(query, "token").unwrap_or("");
        if !same_bytes(token.as_bytes(), self.token.as_bytes()) {
            return refuse("missing or wrong token: open Studio from vy-dev serve");
        }
        None
    }
}

/// Equal length and bytes, in time that does not depend on where they
/// differ.
fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// How the agent is started.
#[derive(Clone, Debug)]
pub struct AgentConfig {
    /// The command.
    pub cmd: AgentCommand,
    /// Put first on its `PATH` (the directory of this binary).
    pub prepend_path: Option<PathBuf>,
    /// More environment: `VY_DEV_KEY`, and `CARGO_TARGET_DIR` when the
    /// project has one.
    pub env: Vec<(OsString, OsString)>,
}

/// Why the agent did not start.
#[derive(Debug)]
pub enum AgentError {
    /// One at a time.
    Busy,
    /// Nothing to do.
    EmptyMission,
    /// The process did not start.
    Spawn(String),
}

impl Dev {
    /// The state for `project`: packages are signed with `key` and written
    /// to `package_path`; the page is announced at `address`; the agent is
    /// started as `agent` says; the API answers what `access` lets in.
    pub fn new(
        project: Project,
        key: SigningKey,
        package_path: PathBuf,
        address: String,
        agent: AgentConfig,
        access: Access,
    ) -> Self {
        Self {
            project,
            address,
            key,
            package_path,
            agent,
            access,
            inner: Mutex::new(Inner {
                build: BuildStatus::Idle,
                build_log: String::new(),
                builds: 0,
                build_requested: false,
                built: None,
                agent_running: false,
                agent_pid: None,
                last_exit: None,
                stop_requested: false,
                log: Log::default(),
                stopping: false,
            }),
            changed: Condvar::new(),
        }
    }

    /// The agent's command.
    pub fn agent_command(&self) -> &AgentCommand {
        &self.agent.cmd
    }

    /// Who may call the API.
    pub fn access(&self) -> &Access {
        &self.access
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock leaves the state readable.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` on the state and wake everyone waiting on it.
    pub fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let r = f(&mut self.lock());
        self.changed.notify_all();
        r
    }

    /// Wait on the state until `done` holds or `timeout` passes; returns
    /// whether it holds.
    pub fn wait_until(&self, timeout: Duration, mut done: impl FnMut(&Inner) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        let mut inner = self.lock();
        loop {
            if done(&inner) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            inner = self
                .changed
                .wait_timeout(inner, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// Where the package of the last successful build is.
    pub fn package_path(&self) -> &PathBuf {
        &self.package_path
    }

    /// Ask for a build; the builder picks it up when it is free.
    pub fn request_build(&self) {
        self.with(|s| s.build_requested = true);
    }

    /// Stop: the agent is stopped, the builder returns after its current
    /// build, waiters wake.
    pub fn shutdown(&self) {
        let pid = self.with(|s| {
            s.stopping = true;
            if s.agent_running {
                s.stop_requested = true;
            }
            s.agent_pid
        });
        if let Some(pid) = pid {
            agent::kill_tree(pid);
        }
    }

    /// True once [`Dev::shutdown`] was called.
    pub fn stopping(&self) -> bool {
        self.lock().stopping
    }

    /// Block until a build is requested; false when the daemon stops.
    pub fn next_build(&self) -> bool {
        let mut inner = self.lock();
        while !inner.build_requested && !inner.stopping {
            inner = self.changed.wait(inner).unwrap_or_else(|e| e.into_inner());
        }
        if inner.stopping {
            return false;
        }
        inner.build_requested = false;
        // Under the lock, so the watcher never sees a build taken without
        // its sources; before cargo runs, so an edit during the build is
        // a difference the watcher sees.
        inner.built = Some(self.project.fingerprint());
        inner.build = BuildStatus::Building;
        inner.build_log.clear();
        inner.log.push_build("building", "bygger");
        drop(inner);
        self.changed.notify_all();
        true
    }

    /// Run the pipeline once, streaming its log into the state, and record
    /// the outcome. A failure is a status, never a panic.
    pub fn run_build(&self) -> BuildOutcome {
        let mut progress = |line: &str| {
            self.with(|s| {
                s.build_log.push_str(line);
                s.build_log.push('\n');
                crate::build::trim_log(&mut s.build_log);
            })
        };
        let outcome = build_project(&self.project, &self.key, &self.package_path, &mut progress);
        self.with(|s| {
            s.builds += 1;
            s.build_log = outcome.log.clone();
            if outcome.ok() {
                s.build = BuildStatus::Ok;
                s.log.push_build("ok", format!("klar: {}", self.address));
            } else {
                s.build = BuildStatus::Error;
                let why = outcome.first_error().unwrap_or("se byggloggen");
                s.log.push_build("error", format!("fel: {why}"));
            }
        });
        outcome
    }

    /// Start the agent with `mission` in the project directory. Its output
    /// is read on threads of their own into the log; when it exits a build
    /// is requested.
    pub fn start_agent(self: &Arc<Self>, mission: &str) -> Result<(), AgentError> {
        if mission.trim().is_empty() {
            return Err(AgentError::EmptyMission);
        }
        let mut inner = self.lock();
        if inner.agent_running {
            return Err(AgentError::Busy);
        }
        let mut child = agent::spawn(&Launch {
            cmd: &self.agent.cmd,
            dir: &self.project.dir,
            mission,
            prepend_path: self.agent.prepend_path.as_deref(),
            env: &self.agent.env,
        })
        .map_err(|e| AgentError::Spawn(format!("{e:#}")))?;
        inner.agent_running = true;
        inner.agent_pid = Some(child.id());
        inner.last_exit = None;
        inner.stop_requested = false;
        inner
            .log
            .push(Kind::Agent, format!("uppdrag: {}", summarize(mission)));
        drop(inner);
        self.changed.notify_all();

        let stderr = child.stderr.take().map(|err| {
            let dev = Arc::clone(self);
            std::thread::spawn(move || {
                for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        dev.with(|s| s.log.push(Kind::Raw, format!("stderr: {line}")));
                    }
                }
            })
        });
        let stdout = child.stdout.take();
        let dev = Arc::clone(self);
        std::thread::spawn(move || {
            for line in stdout
                .into_iter()
                .flat_map(|out| std::io::BufReader::new(out).lines().map_while(Result::ok))
            {
                if line.trim().is_empty() {
                    continue;
                }
                let parsed = agent::parse_stream_line(&line);
                dev.with(|s| {
                    s.log.push(Kind::Raw, line);
                    for (kind, text) in parsed {
                        s.log.push(kind, text);
                    }
                });
            }
            let status = child.wait().ok();
            if let Some(h) = stderr {
                let _ = h.join();
            }
            let code = status.and_then(|s| s.code());
            dev.with(|s| {
                s.agent_running = false;
                s.agent_pid = None;
                let stopped = std::mem::take(&mut s.stop_requested);
                s.last_exit = if stopped { None } else { code };
                let text = match (stopped, code) {
                    (true, _) => "agenten stoppades".to_string(),
                    (false, Some(0)) => "agenten är klar".to_string(),
                    (false, Some(c)) => format!("agenten avslutades med kod {c}"),
                    (false, None) => "agenten avslutades av en signal".to_string(),
                };
                s.log.push(Kind::Agent, text);
                if !s.stopping {
                    s.build_requested = true;
                }
            });
        });
        Ok(())
    }

    /// Stop the running agent and what it started (on a thread of its
    /// own: the stop waits out a grace period). True when one was running.
    pub fn stop_agent(&self) -> bool {
        let pid = self.with(|s| {
            if s.agent_running && !s.stop_requested {
                s.stop_requested = true;
                s.log.push(Kind::Agent, "stoppar agenten");
            }
            s.agent_pid.filter(|_| s.agent_running)
        });
        match pid {
            Some(pid) => {
                std::thread::spawn(move || agent::kill_tree(pid));
                true
            }
            None => false,
        }
    }

    /// Entries after `after` ([`Log::select`]), waiting up to `wait` for at
    /// least one to appear.
    pub fn wait_events(&self, after: u64, wait: Duration, raw: bool, limit: usize) -> Vec<Entry> {
        let mut found = Vec::new();
        self.wait_until(wait, |s| {
            found = s.log.select(after, raw, limit);
            !found.is_empty() || s.stopping
        });
        found
    }

    /// The builder: waits for requests and runs the pipeline until
    /// [`Dev::shutdown`]; `after` gets each outcome. Run on a thread of
    /// its own.
    pub fn builder_loop(&self, mut after: impl FnMut(&BuildOutcome)) {
        while self.next_build() {
            let outcome = self.run_build();
            after(&outcome);
        }
    }

    /// The state as `GET /_dev/state` serves it: the page, the build with
    /// its log, the agent, and the last sequence number of the event log.
    pub fn state_json(&self) -> Value {
        let s = self.lock();
        json!({
            "name": self.project.name,
            "address": self.address,
            "build": {
                "status": s.build.name(),
                "log": s.build_log,
                "builds": s.builds,
            },
            "agent": {
                "running": s.agent_running,
                "last_exit": s.last_exit,
            },
            "seq": s.log.last_seq(),
        })
    }

    /// Route one dev API request. Every route needs `token=<token>` in the
    /// query besides its own parameters ([`Access::guard`]).
    ///
    /// - `GET /_dev/state`: [`Dev::state_json`].
    /// - `POST /_dev/build`: request a build; 202.
    /// - `POST /_dev/agent`: the body (UTF-8, at most [`MAX_BODY_BYTES`])
    ///   is the mission; 202 when the agent started, 409 while one runs.
    /// - `POST /_dev/agent/stop`: `{"stopped": bool}`.
    /// - `GET /_dev/agent/events?after=<seq>&wait=<ms>&raw=0&limit=<n>`:
    ///   `{"next": <seq>, "entries": [{seq, kind, text, status?}]}`, the
    ///   entries above `after`, waiting up to `wait` (default and cap
    ///   [`MAX_WAIT_MS`]) for one; `raw=0` leaves out the raw stream lines;
    ///   `next` is what to pass as `after` the next time.
    pub fn api(self: &Arc<Self>, req: &ApiRequest) -> ApiReply {
        if let Some(refused) = self.access.guard(req) {
            return refused;
        }
        let (path, query) = req.path.split_once('?').unwrap_or((&req.path, ""));
        match (req.method.as_str(), path) {
            ("GET", "/_dev/state") => ApiReply::json(200, self.state_json()),
            ("POST", "/_dev/build") => {
                self.request_build();
                ApiReply::json(202, json!({"ok": true}))
            }
            ("POST", "/_dev/agent") => {
                if req.body.len() > MAX_BODY_BYTES {
                    return ApiReply::json(413, json!({"error": "the mission is too long"}));
                }
                let Ok(mission) = std::str::from_utf8(&req.body) else {
                    return ApiReply::json(400, json!({"error": "the mission is not UTF-8"}));
                };
                match self.start_agent(mission) {
                    Ok(()) => ApiReply::json(202, json!({"ok": true})),
                    Err(AgentError::Busy) => {
                        ApiReply::json(409, json!({"error": "an agent is already running"}))
                    }
                    Err(AgentError::EmptyMission) => {
                        ApiReply::json(400, json!({"error": "the mission is empty"}))
                    }
                    Err(AgentError::Spawn(e)) => {
                        self.with(|s| s.log.push(Kind::Agent, format!("kunde inte starta: {e}")));
                        ApiReply::json(500, json!({"error": e}))
                    }
                }
            }
            ("POST", "/_dev/agent/stop") => {
                ApiReply::json(200, json!({"stopped": self.stop_agent()}))
            }
            ("GET", "/_dev/agent/events") => {
                let after = query_u64(query, "after").unwrap_or(0);
                let wait = query_u64(query, "wait")
                    .unwrap_or(MAX_WAIT_MS)
                    .min(MAX_WAIT_MS);
                let raw = query_u64(query, "raw") != Some(0);
                let limit = query_u64(query, "limit")
                    .map(|l| (l as usize).clamp(1, MAX_EVENTS))
                    .unwrap_or(MAX_EVENTS);
                let entries = self.wait_events(after, Duration::from_millis(wait), raw, limit);
                let next = entries.last().map_or(after, |e| e.seq);
                ApiReply::json(
                    200,
                    json!({
                        "next": next,
                        "entries": entries.iter().map(Entry::to_json).collect::<Vec<_>>(),
                    }),
                )
            }
            (
                _,
                "/_dev/state" | "/_dev/build" | "/_dev/agent" | "/_dev/agent/stop"
                | "/_dev/agent/events",
            ) => ApiReply::json(405, json!({"error": "method not allowed"})),
            _ => ApiReply::json(404, json!({"error": "no such route"})),
        }
    }
}

/// The first line of `text`, at most 80 characters.
fn summarize(text: &str) -> String {
    let line = text.trim().lines().next().unwrap_or("").trim();
    let mut s: String = line.chars().take(80).collect();
    if line.chars().count() > 80 || text.trim().lines().nth(1).is_some() {
        s.push('…');
    }
    s
}

/// The value of `key` in a query string.
fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

/// The value of `key` in a query string, as a number.
fn query_u64(query: &str, key: &str) -> Option<u64> {
    query_value(query, key).and_then(|v| v.parse().ok())
}

/// True for a `Host` of `localhost:<port>` or `127.0.0.1:<port>` (without
/// the port too when it is 80, as HTTP leaves it out then). A request that
/// reached the socket under any other name came through DNS rebinding, and
/// one naming another port was not meant for this API.
pub fn host_matches(host: Option<&str>, port: u16) -> bool {
    let Some(host) = host else {
        return false;
    };
    let host = host.trim().to_ascii_lowercase();
    let (name, given) = match host.split_once(':') {
        Some((n, p)) => (n, p.parse::<u16>().ok().filter(|_| !p.starts_with('+'))),
        None => (host.as_str(), (port == 80).then_some(80)),
    };
    given == Some(port) && matches!(name, "localhost" | "127.0.0.1")
}

/// Whether a header name marks a browser's request: `Origin`, `Referer`
/// and the `Sec-Fetch-*` metadata every current browser sends and a page
/// cannot remove. `vy-proto` sends none of them (SPEC-7.5.1: no
/// `Referer`).
pub fn is_browser_header(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "origin" || n == "referer" || n.starts_with("sec-fetch-")
}

/// True when the path is the dev API's.
pub fn is_api_path(path: &str) -> bool {
    path.starts_with(API_PREFIX)
}

/// A request to the dev API as the router sees it.
#[derive(Clone, Debug)]
pub struct ApiRequest {
    /// `GET` or `POST`.
    pub method: String,
    /// Path with query.
    pub path: String,
    /// The body (at most [`MAX_BODY_BYTES`] + 1 bytes are read).
    pub body: Vec<u8>,
    /// The `Host` header as sent, never `X-Forwarded-Host`.
    pub host: Option<String>,
    /// True when the request carried a header [`is_browser_header`] names.
    pub browser_like: bool,
}

impl ApiRequest {
    /// A request with no `Host` (refused by [`Access::guard`]);
    /// [`Access::request`] makes one that is let in.
    pub fn new(method: &str, path: &str, body: &[u8]) -> Self {
        Self {
            method: method.into(),
            path: path.into(),
            body: body.to_vec(),
            host: None,
            browser_like: false,
        }
    }
}

/// What the router answered: a status and a JSON body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiReply {
    /// HTTP status.
    pub status: u16,
    /// The JSON body.
    pub body: Vec<u8>,
}

impl ApiReply {
    /// `value` as the body.
    pub fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            body: value.to_string().into_bytes(),
        }
    }

    /// The body as JSON.
    pub fn value(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_must_name_the_api() {
        for ok in [
            "localhost:8081",
            "LOCALHOST:8081",
            "127.0.0.1:8081",
            " localhost:8081 ",
        ] {
            assert!(host_matches(Some(ok), 8081), "{ok}");
        }
        for bad in [
            "evil.example:8081",
            "localhost.evil.example:8081",
            "127.0.0.1.evil:8081",
            "localhost",
            "localhost:8080",
            "localhost:",
            "localhost:80x",
            "localhost:+8081",
            "localhost:8081@evil.example",
            "[::1]:8081",
            "",
        ] {
            assert!(!host_matches(Some(bad), 8081), "{bad}");
        }
        assert!(!host_matches(None, 8081));
        assert!(
            host_matches(Some("localhost"), 80),
            "port 80 may be left out"
        );
        assert!(is_browser_header("Origin"));
        assert!(is_browser_header("sec-fetch-site"));
        assert!(is_browser_header("Referer"));
        assert!(!is_browser_header("Accept"));
        assert!(is_api_path("/_dev/state"));
        assert!(!is_api_path("/index"));
        assert_eq!(query_u64("after=12&wait=5", "wait"), Some(5));
        assert_eq!(query_u64("after=12", "wait"), None);
        assert_eq!(query_u64("after=x", "after"), None);
        assert_eq!(query_value("a=1&token=t", "token"), Some("t"));
        assert_eq!(summarize("  gör en sida\nmed två rader"), "gör en sida…");
        assert_eq!(summarize(&"x".repeat(81)).chars().count(), 81);
    }

    /// The guard comes before the routes: a browser's request, one
    /// addressed to another name or port, and one without this run's
    /// token are refused whatever they ask for.
    #[test]
    fn guard_refuses_browsers_foreign_hosts_and_missing_tokens() {
        let access = Access::new(8081).unwrap();
        assert_eq!(access.token.len(), 32);
        assert!(access.token.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(access.token, Access::new(8081).unwrap().token);
        let ok = access.request("GET", "/_dev/state", b"");
        assert_eq!(ok.path, format!("/_dev/state?token={}", access.token));
        assert!(access.guard(&ok).is_none());
        let with_query = access.request("GET", "/_dev/agent/events?after=3", b"");
        assert!(with_query
            .path
            .ends_with(&format!("after=3&token={}", access.token)));
        assert!(access.guard(&with_query).is_none());
        let refused = |r: ApiRequest| access.guard(&r).map(|a| a.status);
        assert_eq!(
            refused(ApiRequest {
                browser_like: true,
                ..ok.clone()
            }),
            Some(403)
        );
        assert_eq!(
            refused(ApiRequest {
                host: Some("attacker.example:8081".into()),
                ..ok.clone()
            }),
            Some(403)
        );
        assert_eq!(
            refused(ApiRequest {
                host: Some("localhost:8080".into()),
                ..ok.clone()
            }),
            Some(403)
        );
        assert_eq!(
            refused(ApiRequest {
                host: None,
                ..ok.clone()
            }),
            Some(403)
        );
        assert_eq!(
            refused(ApiRequest {
                path: "/_dev/state".into(),
                ..ok.clone()
            }),
            Some(403),
            "no token"
        );
        let mut wrong = access.token.clone();
        wrong.replace_range(..1, if wrong.starts_with('0') { "1" } else { "0" });
        assert_eq!(
            refused(ApiRequest {
                path: format!("/_dev/state?token={wrong}"),
                ..ok.clone()
            }),
            Some(403),
            "wrong token"
        );
        assert_eq!(
            refused(ApiRequest {
                path: format!("/_dev/state?token={}x", access.token),
                ..ok
            }),
            Some(403),
            "longer token"
        );
        assert!(
            same_bytes(b"ab", b"ab") && !same_bytes(b"ab", b"abc") && !same_bytes(b"ab", b"ac")
        );
    }

    /// An agent command that does not start is a 500 with the reason, in
    /// the log too, and leaves no agent marked as running.
    #[test]
    fn an_agent_that_does_not_start_is_reported() {
        let dir = std::env::temp_dir().join(format!("vy-dev-nostart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::scaffold::Scaffold {
            dir: dir.clone(),
            name: "nostart".into(),
            sdk: None,
        }
        .create()
        .unwrap();
        let dev = Arc::new(Dev::new(
            Project::open(&dir).unwrap(),
            SigningKey::from_seed(&[7; 32]),
            dir.join("target/x.vyp"),
            "vy://localhost:1/nostart".into(),
            AgentConfig {
                cmd: AgentCommand::parse("/nonexistent/vy-dev-agent").unwrap(),
                prepend_path: None,
                env: Vec::new(),
            },
            Access::new(1).unwrap(),
        ));
        let r = dev.api(
            &dev.access()
                .request("POST", "/_dev/agent", "gör något".as_bytes()),
        );
        assert_eq!(r.status, 500);
        assert!(r.value()["error"]
            .as_str()
            .unwrap()
            .contains("is Claude Code installed"));
        assert_eq!(dev.state_json()["agent"]["running"], false);
        let entries = dev.wait_events(0, Duration::ZERO, false, 10);
        assert!(entries
            .iter()
            .any(|e| e.kind == Kind::Agent && e.text.starts_with("kunde inte starta")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
