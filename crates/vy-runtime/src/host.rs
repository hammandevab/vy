//! Host state behind the `vy` imports (SPEC §5.2) and the functions themselves.
//!
//! Everything outside pure computation and drawing is a capability
//! (SPEC-2.1.4): an import whose capability is not granted returns `E_DENIED`,
//! logs, and leaves the module running (SPEC-5.3.1). Time and randomness go
//! through the host (SPEC-2.1.5): the headless clock advances only per frame
//! and the random source is a seeded generator.
//!
//! Two rules protect the host from the module (SPEC-9.3.1):
//!
//! - **Every copy out of module memory is capped.** [`slice`] and [`string`]
//!   are the only readers of `(ptr, len)` and both take the cap from
//!   `vy_abi::limits`; a `len` above it is `E_ARG`, logged with the cap, and
//!   nothing is copied. The semantic tree and the display list together hold
//!   at most `MAX_FRAME_TEXT_BYTES` of text per frame.
//! - **Host work costs fuel.** Every byte the host shapes, encodes, copies or
//!   generates on the module's behalf is charged at
//!   [`crate::FUEL_PER_HOST_BYTE`] against the same fuel that pays for wasm
//!   instructions, so expensive imports cannot bypass the 8 ms budget
//!   (SPEC-5.3.3). Decoding a package resource is the exception: a one-time
//!   cost bounded by the manifest, charged to nobody (SPEC-6.5.1).

use crate::rng::Rng;
use crate::{ClockSource, Transport, FUEL_PER_HOST_BYTE};
use std::collections::{BTreeMap, HashSet};
use vy_abi::{cap, err, fit, level, limits, method, ABI_VERSION};
use vy_dl::DisplayList;
use vy_render::{pack_x100, ImageCache, TextEngine};
use wasmtime::{Caller, Linker, Memory, StoreLimits, Trap};

/// Storage behind `state_*` (SPEC-5.2.9): key to value. The headless host
/// keeps it in memory for the instance's life; the conformance runner hands
/// it from one instance to the next to prove persistence across `vy_suspend`
/// and a fresh `vy_init` (SPEC-5.1.5).
pub type Storage = BTreeMap<Vec<u8>, Vec<u8>>;
/// Storage quota the headless host applies when the caller names none:
/// 1 MB, the policy default of SPEC-8.2.3.
pub const DEFAULT_STORAGE_QUOTA_BYTES: u64 = 1 << 20;

/// A response the headless host serves for `fetch_begin` to one URL
/// (SPEC-5.2.7): what a conformance case declares as
/// `fetch = [{url, status, body}]`. The headless host has no transport;
/// a URL without a canned response is denied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CannedResponse {
    /// The exact URL `fetch_begin` must name.
    pub url: String,
    /// HTTP status delivered in `FETCH_DONE`.
    pub status: i32,
    /// The whole body, buffered before `FETCH_DONE` (SPEC-5.2.7).
    pub body: Vec<u8>,
}

/// One request from `fetch_begin` to the `fetch_read` that returns 0 or
/// the `fetch_cancel` that drops it (SPEC-5.2.7).
pub(crate) struct Fetch {
    pub(crate) id: i32,
    pub(crate) status: i32,
    pub(crate) body: Vec<u8>,
    /// Bytes `fetch_read` has copied so far.
    pub(crate) pos: usize,
    /// True once the response is in `body` (a canned one from the start;
    /// an external one from [`crate::Runtime::fetch_complete`]).
    pub(crate) answered: bool,
    /// True once `FETCH_DONE` has been delivered and the body is readable.
    pub(crate) delivered: bool,
}

/// A request a module made with `fetch_begin` under [`Transport::External`]
/// (SPEC-5.2.7): the host process performs it and answers with
/// [`crate::Runtime::fetch_complete`]. The capability, method, byte caps
/// (SPEC-9.3.2), origin gate (SPEC-5.2.8) and rate limit were checked
/// before it was queued.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchRequest {
    /// The `req_id` the module holds.
    pub id: i32,
    /// `vy_abi::method`: 0 GET, 1 POST, 2 PUT, 3 DELETE.
    pub method: i32,
    /// An `https://` URL, or `http://` to loopback ([`origin_of`]), within
    /// the manifest's `net` origins.
    pub url: String,
    /// Request body, at most `MAX_FETCH_BODY_BYTES`.
    pub body: Vec<u8>,
}

/// A `navigate` call the host accepted (SPEC-5.2.10) and the shell carries
/// out; `vy://` in the tab, `https://` in the system browser (SPEC-2.1.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Navigation {
    /// The `vy://` or `https://` address.
    pub url: String,
    /// True to replace the current history entry instead of pushing one.
    pub replace: bool,
}

/// Origin of a URL a page may fetch: scheme, host and port, lower-cased
/// (SPEC-9.5.1). `https://` to any host, and plain `http://` only to a
/// loopback host (`localhost`, `127.0.0.1`, `[::1]`, with an optional
/// port), the one exception to TLS (SPEC-7.1.3); `None` for anything else.
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
        ("https", rest)
    } else {
        ("http", url.strip_prefix("http://")?)
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = rest[..end].to_ascii_lowercase();
    if authority.is_empty() || (scheme == "http" && !is_loopback_authority(&authority)) {
        return None;
    }
    Some(format!("{scheme}://{authority}"))
}

/// Whether a page whose own address is `vy://<page_authority>/…` may make a
/// plain-http request to `origin` (as [`origin_of`] gives it): only to its
/// own loopback origin, so `vy://localhost:8081/` reaches
/// `http://localhost:8081` and nothing else, and no page from another
/// origin reaches a loopback service over plain http (SPEC-7.1.3,
/// SPEC-9.5.1). True for every `https://` origin: this is the plain-http
/// rule only. For the host process, which knows the page's address; the
/// tab process does not.
pub fn plain_http_allowed(origin: &str, page_authority: &str) -> bool {
    match origin.strip_prefix("http://") {
        None => true,
        Some(authority) => {
            is_loopback_authority(authority) && authority.eq_ignore_ascii_case(page_authority)
        }
    }
}

/// `localhost`, `127.0.0.1` or `[::1]`, optionally with `:<digits>`, and
/// nothing else (no user info, no other name that merely starts so).
fn is_loopback_authority(authority: &str) -> bool {
    let (host, port) = match authority.strip_prefix("[::1]") {
        Some(rest) => ("[::1]", rest),
        None => match authority.find(':') {
            Some(i) => (&authority[..i], &authority[i..]),
            None => (authority, ""),
        },
    };
    let port_ok = port.is_empty()
        || (port.len() > 1
            && port.starts_with(':')
            && port[1..].bytes().all(|b| b.is_ascii_digit()));
    port_ok && matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// Milliseconds the headless clock advances per completed frame.
pub const HEADLESS_FRAME_MS: i64 = 16;
/// Seed of the headless random source; fixed so renders repeat byte for byte.
pub const HEADLESS_RANDOM_SEED: u64 = 0x5EED_5EED_0000_0001;
/// Upper bound on retained log entries per instance.
pub const MAX_LOG_ENTRIES: usize = 10_000;
/// How long after a user gesture `clipboard_read` is allowed (SPEC-5.2.12).
pub const GESTURE_WINDOW_MS: i64 = 1000;

/// Capabilities granted to the instance (SPEC-8.2.1). All false by default:
/// the headless host grants only what the caller names (`vy-cli render
/// --grant`, `capabilities` in a conformance case).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// `fetch_*` to listed origins.
    pub net: bool,
    /// `state_*`.
    pub storage: bool,
    /// `clipboard_*`.
    pub clipboard: bool,
    /// 10 µs clock resolution.
    pub hires_time: bool,
    /// `ime_begin`/`ime_end`.
    pub ime: bool,
    /// Frames while hidden.
    pub persist_frame: bool,
}

impl Capabilities {
    /// Grant the capability with this canonical name (`vy_abi::cap::NAMES`);
    /// `false` for a name 1.0 does not define, which grants nothing
    /// (SPEC-8.2.1, SPEC-8.2.2).
    pub fn grant(&mut self, name: &str) -> bool {
        match cap::id(name) {
            Some(cap::NET) => self.net = true,
            Some(cap::STORAGE) => self.storage = true,
            Some(cap::CLIPBOARD) => self.clipboard = true,
            Some(cap::HIRES_TIME) => self.hires_time = true,
            Some(cap::IME) => self.ime = true,
            Some(cap::PERSIST_FRAME) => self.persist_frame = true,
            _ => return false,
        }
        true
    }

    /// Capabilities from a list of canonical names; `Err` names the first
    /// unknown one.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<Self, String> {
        let mut caps = Self::default();
        for name in names {
            if !caps.grant(name) {
                return Err(name.to_string());
            }
        }
        Ok(caps)
    }

    /// True when the capability with this id (`vy_abi::cap`) is granted.
    pub fn has(&self, id: i32) -> bool {
        match id {
            cap::NET => self.net,
            cap::STORAGE => self.storage,
            cap::CLIPBOARD => self.clipboard,
            cap::HIRES_TIME => self.hires_time,
            cap::IME => self.ime,
            cap::PERSIST_FRAME => self.persist_frame,
            _ => false,
        }
    }

    /// Grant or revoke the capability with this id; `false` for an id 1.0
    /// does not define, which changes nothing.
    pub fn set(&mut self, id: i32, granted: bool) -> bool {
        let slot = match id {
            cap::NET => &mut self.net,
            cap::STORAGE => &mut self.storage,
            cap::CLIPBOARD => &mut self.clipboard,
            cap::HIRES_TIME => &mut self.hires_time,
            cap::IME => &mut self.ime,
            cap::PERSIST_FRAME => &mut self.persist_frame,
            _ => return false,
        };
        *slot = granted;
        true
    }

    /// The granted capabilities' canonical names, in id order.
    pub fn names(&self) -> Vec<&'static str> {
        cap::NAMES
            .into_iter()
            .filter(|n| self.has(cap::id(n).unwrap_or(0)))
            .collect()
    }
}

/// Who wrote a log entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogSource {
    /// The module via the `log` import.
    Module,
    /// The host, about the module (denied calls, bad arguments).
    Host,
}

/// One log line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    /// Origin.
    pub source: LogSource,
    /// Level as `vy_abi::level` numbers them: the module's argument to
    /// `log`, any value outside 0..=3 recorded as 3 (SPEC-5.2.1). Host
    /// entries are `WARN` for a refused call, and `DEBUG` for a call the
    /// headless host has no implementation for although its capability is
    /// granted, or for an outcome that is normal for the page (a key
    /// without a value).
    pub level: i32,
    /// Message text.
    pub message: String,
}

/// A node of the semantic tree (SPEC-10.1.1).
#[derive(Clone, Debug, PartialEq)]
pub struct SemNode {
    /// Module-chosen id, unique per frame.
    pub id: i32,
    /// Parent id, 0 for roots.
    pub parent_id: i32,
    /// Role 1..=24 (SPEC-A.3.1).
    pub role: i32,
    /// Left, logical pixels.
    pub x: f32,
    /// Top, logical pixels.
    pub y: f32,
    /// Width, logical pixels.
    pub w: f32,
    /// Height, logical pixels.
    pub h: f32,
    /// Text content or accessible name.
    pub text: String,
}

/// A link rectangle registered with `link_register` (SPEC-5.2.10).
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    /// Module-chosen id.
    pub id: i32,
    /// Left, logical pixels.
    pub x: f32,
    /// Top, logical pixels.
    pub y: f32,
    /// Width, logical pixels.
    pub w: f32,
    /// Height, logical pixels.
    pub h: f32,
    /// Target address.
    pub url: String,
}

/// Mutable host state owned by the wasmtime store.
pub struct Host {
    pub(crate) memory: Option<Memory>,
    pub(crate) limits: StoreLimits,
    pub(crate) caps: Capabilities,
    pub(crate) dl: DisplayList,
    pub(crate) text: TextEngine,
    pub(crate) images: ImageCache,
    pub(crate) sem: Vec<SemNode>,
    /// Ids registered in `sem` this frame: the duplicate check of
    /// SPEC-5.2.13 must not scan the tree, or a legal frame of 65 536 nodes
    /// costs the host seconds that no fuel pays for (SPEC-5.3.3).
    pub(crate) sem_ids: HashSet<i32>,
    pub(crate) sem_text_bytes: usize,
    pub(crate) links: Vec<Link>,
    pub(crate) log: Vec<LogEntry>,
    pub(crate) clock_ms: i64,
    /// Where the clock comes from: advanced per frame headless, set by
    /// the host process otherwise (SPEC-5.2.11).
    pub(crate) clock: ClockSource,
    pub(crate) rng: Rng,
    pub(crate) wants_frame: bool,
    pub(crate) exited: Option<i32>,
    pub(crate) keys_down: Vec<i32>,
    pub(crate) ime_rect: Option<(f32, f32, f32, f32)>,
    pub(crate) sem_full_logged: bool,
    /// Storage behind `state_*` when `storage` is granted (SPEC-5.2.9).
    pub(crate) state: Storage,
    /// Most bytes (keys and values together) `state` may hold (SPEC-9.3.1).
    pub(crate) state_quota: u64,
    /// Responses `fetch_begin` can be answered with (SPEC-5.2.7).
    pub(crate) canned: Vec<CannedResponse>,
    /// Origins the manifest lists under `net`, normalised with
    /// [`origin_of`]; empty when the caller has no manifest, and then the
    /// gate of SPEC-5.2.8 is not applied.
    pub(crate) net_origins: Vec<String>,
    /// Open requests in the order they were begun.
    pub(crate) fetches: Vec<Fetch>,
    pub(crate) next_fetch_id: i32,
    /// How `fetch_begin` is served (SPEC-5.2.7).
    pub(crate) transport: Transport,
    /// Requests queued for the host process under [`Transport::External`],
    /// taken with [`crate::Runtime::take_fetch_requests`].
    pub(crate) fetch_requests: Vec<FetchRequest>,
    /// True when `navigate` is carried out by the host (SPEC-5.2.10);
    /// false in the headless host, where it is `E_DENIED`.
    pub(crate) navigation: bool,
    /// Accepted `navigate` calls, taken with [`crate::Runtime::take_navigations`].
    pub(crate) navigations: Vec<Navigation>,
    /// The headless clipboard, when the caller gave the host one
    /// (SPEC-5.2.12).
    pub(crate) clipboard: Option<Vec<u8>>,
    /// Clock reading of the last user gesture (a pointer press or release,
    /// a key, committed text), for the 1 s window of `clipboard_read`
    /// (SPEC-5.2.12); `None` before the first.
    pub(crate) last_gesture_ms: Option<i64>,
    /// True while the event being delivered is a copy gesture the user
    /// made in the page (Ctrl+C or Ctrl+X): `clipboard_write` then
    /// succeeds without the `clipboard` capability (SPEC-8.2.4);
    /// `clipboard_read` never does.
    pub(crate) copy_gesture: bool,
    /// The bytes of the last `clipboard_write` not yet taken with
    /// [`crate::Runtime::take_clipboard_write`]: a host process forwards
    /// every write, even one that repeats the value (the system clipboard
    /// may have changed in between).
    pub(crate) clipboard_written: Option<Vec<u8>>,
    /// Fuel owed for host work in the current import call; settled by
    /// [`with_host`] / [`with_memory`] before the call returns to wasm.
    pending_fuel: u64,
    /// Fuel charged for host work since the last [`Host::take_host_fuel`].
    host_fuel: u64,
}

impl Host {
    /// Host state for one instance: the store limits and everything in
    /// [`crate::Options`] except the memory cap, which the limits carry.
    pub(crate) fn new(limits: StoreLimits, options: crate::Options) -> Self {
        let crate::Options {
            capabilities: caps,
            resources,
            storage: state,
            storage_quota_bytes: state_quota,
            fetch: canned,
            net_origins,
            clipboard,
            memory_max_pages: _,
            clock,
            random,
            transport,
            navigation,
        } = options;
        Self {
            memory: None,
            limits,
            caps,
            dl: DisplayList::new(),
            text: TextEngine::new(),
            images: ImageCache::new(resources),
            sem: Vec::new(),
            sem_ids: HashSet::new(),
            sem_text_bytes: 0,
            links: Vec::new(),
            log: Vec::new(),
            clock_ms: 0,
            clock,
            rng: Rng::new(random),
            wants_frame: false,
            exited: None,
            keys_down: Vec::new(),
            ime_rect: None,
            sem_full_logged: false,
            state,
            state_quota,
            canned,
            net_origins: net_origins.iter().filter_map(|o| origin_of(o)).collect(),
            fetches: Vec::new(),
            next_fetch_id: 1,
            transport,
            fetch_requests: Vec::new(),
            navigation,
            navigations: Vec::new(),
            clipboard,
            last_gesture_ms: None,
            copy_gesture: false,
            clipboard_written: None,
            pending_fuel: 0,
            host_fuel: 0,
        }
    }

    /// True when a user gesture happened within the last second of the
    /// host's clock (SPEC-5.2.12).
    fn recent_gesture(&self) -> bool {
        self.last_gesture_ms
            .is_some_and(|at| self.clock_ms - at <= GESTURE_WINDOW_MS)
    }

    /// The open request with this id, or `E_ARG`.
    fn fetch_mut(&mut self, name: &str, req_id: i32) -> Result<&mut Fetch, i32> {
        if self.fetches.iter().any(|f| f.id == req_id) {
            Ok(self
                .fetches
                .iter_mut()
                .find(|f| f.id == req_id)
                .expect("checked above"))
        } else {
            self.host_log(format!("{name}: E_ARG (no open request {req_id})"));
            Err(err::E_ARG)
        }
    }

    pub(crate) fn log(&mut self, source: LogSource, level: i32, message: String) {
        if self.log.len() < MAX_LOG_ENTRIES {
            self.log.push(LogEntry {
                source,
                level,
                message,
            });
        }
    }

    pub(crate) fn host_log(&mut self, message: String) {
        self.log(LogSource::Host, level::WARN, message);
    }

    /// Log a denied call and return `E_DENIED` (SPEC-5.3.1).
    fn denied(&mut self, name: &str, capability: &str) -> i32 {
        self.host_log(format!(
            "{name}: E_DENIED (capability `{capability}` not granted)"
        ));
        err::E_DENIED
    }

    /// `E_DENIED` for a call whose capability is granted but which the
    /// headless host cannot serve (no window to navigate, no clipboard, no
    /// transport without a canned response); logged at debug level, since
    /// it is the host's limitation and not the page's mistake.
    fn denied_headless(&mut self, name: &str, what: &str) -> i32 {
        self.log(
            LogSource::Host,
            level::DEBUG,
            format!("{name}: E_DENIED ({what}: not available in the headless host)"),
        );
        err::E_DENIED
    }

    /// Bytes `state` holds: keys and values together.
    fn state_bytes(&self) -> u64 {
        self.state
            .iter()
            .map(|(k, v)| (k.len() + v.len()) as u64)
            .sum()
    }

    /// Log a refusal and return its status code.
    fn refuse(&mut self, name: &str, fault: &Fault) -> i32 {
        self.report(name, fault);
        fault.code()
    }

    fn report(&mut self, name: &str, fault: &Fault) {
        match *fault {
            Fault::Code(err::OK) => {}
            Fault::Code(code) => self.host_log(format!("{name}: {}", err_name(code))),
            Fault::TooLong { what, len, cap } => self.host_log(format!(
                "{name}: E_ARG ({what} is {len} bytes, cap is {cap}; nothing copied)"
            )),
        }
    }

    /// Owe fuel for `bytes` of host work (see [`crate::FUEL_PER_HOST_BYTE`]).
    fn charge(&mut self, bytes: usize) {
        let fuel = (bytes as u64).saturating_mul(FUEL_PER_HOST_BYTE);
        self.pending_fuel = self.pending_fuel.saturating_add(fuel);
        self.host_fuel = self.host_fuel.saturating_add(fuel);
    }

    /// Fuel charged for host work since the previous call; resets the counter.
    pub(crate) fn take_host_fuel(&mut self) -> u64 {
        std::mem::take(&mut self.host_fuel)
    }

    /// Start a new semantic tree (every `vy_frame`, SPEC-10.1.1).
    pub(crate) fn clear_sem(&mut self) {
        self.sem.clear();
        self.sem_ids.clear();
        self.sem_text_bytes = 0;
        self.sem_full_logged = false;
    }

    /// Refuse `more` text bytes when the display list and the semantic tree
    /// together would hold more than `MAX_FRAME_TEXT_BYTES`.
    fn frame_text_room(&self, more: usize) -> Result<(), Fault> {
        let held = self.dl.text_bytes() + self.sem_text_bytes;
        if held + more > limits::MAX_FRAME_TEXT_BYTES as usize {
            return Err(Fault::Code(err::E_DL_FULL));
        }
        Ok(())
    }
}

/// Name of an ABI status code for log lines (`E_UNKNOWN` for one 1.0 does
/// not define).
pub fn err_name(code: i32) -> &'static str {
    err::name(code).unwrap_or("E_UNKNOWN")
}

/// Why an import refused a call: the ABI status code, plus the cap that was
/// hit when a `len` was too long, so the log line can say so.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fault {
    /// A plain status code from appendix A.
    Code(i32),
    /// `len` exceeded a cap in `vy_abi::limits`; the code is `E_ARG`.
    TooLong {
        what: &'static str,
        len: i32,
        cap: u32,
    },
}

impl From<i32> for Fault {
    fn from(code: i32) -> Self {
        Fault::Code(code)
    }
}

impl Fault {
    fn code(&self) -> i32 {
        match self {
            Fault::Code(c) => *c,
            Fault::TooLong { .. } => err::E_ARG,
        }
    }
}

/// Settle the fuel owed for host work against the store.
///
/// wasmtime checks fuel only at function entries and loop headers, so an
/// import that exhausts it traps `OutOfFuel` itself; the module is unwound
/// exactly as if a wasm instruction had run out, and [`crate::Runtime`]
/// reports `OverBudget`.
fn settle_fuel(caller: &mut Caller<'_, Host>) -> wasmtime::Result<()> {
    let owed = std::mem::take(&mut caller.data_mut().pending_fuel);
    if owed == 0 {
        return Ok(());
    }
    let left = caller.get_fuel()?;
    if owed >= left {
        caller.set_fuel(0)?;
        return Err(Trap::OutOfFuel.into());
    }
    caller.set_fuel(left - owed)
}

/// Run an import body against the host state, then settle its fuel.
fn with_host<R>(
    caller: &mut Caller<'_, Host>,
    f: impl FnOnce(&mut Host) -> R,
) -> wasmtime::Result<R> {
    let r = f(caller.data_mut());
    settle_fuel(caller)?;
    Ok(r)
}

/// Borrow the module memory and host state together, then settle the fuel.
///
/// The memory handle is stored right after instantiation; the ABI check
/// refuses start functions, so no import can run before that.
fn with_memory<R>(
    caller: &mut Caller<'_, Host>,
    f: impl FnOnce(&mut [u8], &mut Host) -> R,
) -> wasmtime::Result<R> {
    let r = match caller.data().memory {
        Some(memory) => {
            let (data, host) = memory.data_and_store_mut(&mut *caller);
            f(data, host)
        }
        None => f(&mut [], caller.data_mut()),
    };
    settle_fuel(caller)?;
    Ok(r)
}

/// Slice of module memory, or `E_ARG` when `len` is above `cap` (nothing is
/// read then) or the range is outside memory.
fn slice<'a>(
    data: &'a [u8],
    ptr: i32,
    len: i32,
    what: &'static str,
    cap: u32,
) -> Result<&'a [u8], Fault> {
    if ptr < 0 || len < 0 {
        return Err(Fault::Code(err::E_ARG));
    }
    if len as u32 > cap {
        return Err(Fault::TooLong { what, len, cap });
    }
    let start = ptr as usize;
    let end = start
        .checked_add(len as usize)
        .ok_or(Fault::Code(err::E_ARG))?;
    data.get(start..end).ok_or(Fault::Code(err::E_ARG))
}

/// UTF-8 string in module memory, capped like [`slice`]; invalid UTF-8 is `E_ARG`.
fn string(data: &[u8], ptr: i32, len: i32, what: &'static str, cap: u32) -> Result<String, Fault> {
    let bytes = slice(data, ptr, len, what, cap)?;
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| Fault::Code(err::E_ARG))
}

fn style_u16(style_id: i32) -> Result<u16, Fault> {
    u16::try_from(style_id).map_err(|_| Fault::Code(err::E_STYLE))
}

/// Define every import of namespace `vy` (SPEC-5.2.1..5.2.13).
///
/// The names and signatures are exactly `vy_abi::IMPORTS`; wasmtime checks the
/// signatures when a module is instantiated, and `tests::spec_5_2_all_imports_linked`
/// checks that nothing is missing.
pub(crate) fn link_imports(linker: &mut Linker<Host>) -> wasmtime::Result<()> {
    let m = vy_abi::IMPORT_MODULE;

    // lifecycle (SPEC-5.2.1)
    linker.func_wrap(m, "abi_version", |_: Caller<'_, Host>| -> i32 {
        ABI_VERSION
    })?;
    linker.func_wrap(
        m,
        "log",
        |mut c: Caller<'_, Host>, lvl: i32, ptr: i32, len: i32| -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                match slice(data, ptr, len, "message", limits::MAX_LOG_BYTES) {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(bytes).into_owned();
                        host.charge(text.len());
                        // A level outside 0..=3 is recorded as error; the
                        // call itself never fails on its level (SPEC-5.2.1).
                        host.log(LogSource::Module, level::clamp(lvl), text);
                        err::OK
                    }
                    Err(fault) => host.refuse("log", &fault),
                }
            })
        },
    )?;
    linker.func_wrap(m, "request_frame", |mut c: Caller<'_, Host>| {
        c.data_mut().wants_frame = true;
    })?;
    linker.func_wrap(m, "exit", |mut c: Caller<'_, Host>, code: i32| {
        c.data_mut().exited = Some(code);
    })?;

    // display list (SPEC-5.2.3): every call is charged the bytes it encoded.
    linker.func_wrap(
        m,
        "dl_begin",
        |mut c: Caller<'_, Host>| -> wasmtime::Result<i32> {
            with_host(&mut c, |host| {
                // A new list also starts a new link set (SPEC-5.2.10).
                match host.dl.begin() {
                    Ok(()) => {
                        host.links.clear();
                        err::OK
                    }
                    Err(code) => host.refuse("dl_begin", &code.into()),
                }
            })
        },
    )?;
    linker.func_wrap(
        m,
        "dl_end",
        |mut c: Caller<'_, Host>| -> wasmtime::Result<i32> {
            with_host(&mut c, |host| {
                let before = host.dl.bytes().len();
                let code = match host.dl.end() {
                    Ok(()) => err::OK,
                    Err(code) => host.refuse("dl_end", &code.into()),
                };
                host.charge(host.dl.bytes().len().saturating_sub(before));
                code
            })
        },
    )?;
    linker.func_wrap(
        m,
        "dl_rect",
        |mut c: Caller<'_, Host>,
         x: f32,
         y: f32,
         w: f32,
         h: f32,
         radius: f32,
         color: i32|
         -> wasmtime::Result<i32> {
            with_host(&mut c, |host| {
                let before = host.dl.bytes().len();
                let code = match host.dl.rect(x, y, w, h, radius, color as u32) {
                    Ok(()) => err::OK,
                    Err(code) => host.refuse("dl_rect", &code.into()),
                };
                host.charge(host.dl.bytes().len().saturating_sub(before));
                code
            })
        },
    )?;
    linker.func_wrap(
        m,
        "dl_text",
        |mut c: Caller<'_, Host>,
         x: f32,
         y: f32,
         w: f32,
         style_id: i32,
         ptr: i32,
         len: i32|
         -> wasmtime::Result<i64> {
            with_memory(&mut c, |data, host| {
                let before = host.dl.bytes().len();
                let result = (|| {
                    // Everything that can refuse the call is checked before
                    // the text is copied or shaped.
                    if !host.dl.is_open() {
                        return Err(Fault::Code(err::E_DL_STATE));
                    }
                    if host.dl.op_count() >= limits::MAX_OPS_PER_FRAME {
                        return Err(Fault::Code(err::E_DL_FULL));
                    }
                    let sid = style_u16(style_id)?;
                    if host.text.get_style(sid).is_none() {
                        return Err(Fault::Code(err::E_STYLE));
                    }
                    let text = string(data, ptr, len, "text", limits::MAX_TEXT_BYTES)?;
                    host.frame_text_room(text.len())?;
                    host.charge(text.len());
                    let size = host.text.measure(sid, w, &text)?;
                    host.dl.text(x, y, w, sid, text.as_bytes())?;
                    // Width and height packed like `text_measure` (SPEC-5.2.4, SPEC-5.2.5).
                    Ok(pack_x100(size.w, size.h))
                })();
                host.charge(host.dl.bytes().len().saturating_sub(before));
                match result {
                    Ok(packed) => packed,
                    Err(fault) => i64::from(host.refuse("dl_text", &fault)),
                }
            })
        },
    )?;
    linker.func_wrap(
        m,
        "dl_image",
        |mut c: Caller<'_, Host>,
         x: f32,
         y: f32,
         w: f32,
         h: f32,
         resource_id: i32,
         fit_mode: i32|
         -> wasmtime::Result<i32> {
            with_host(&mut c, |host| {
                let before = host.dl.bytes().len();
                let result = (|| {
                    if !host.dl.is_open() {
                        return Err(Fault::Code(err::E_DL_STATE));
                    }
                    if host.dl.op_count() >= limits::MAX_OPS_PER_FRAME {
                        return Err(Fault::Code(err::E_DL_FULL));
                    }
                    let fit_mode = match fit_mode {
                        fit::COVER => vy_dl::Fit::Cover,
                        fit::CONTAIN => vy_dl::Fit::Contain,
                        fit::STRETCH => vy_dl::Fit::Stretch,
                        _ => return Err(Fault::Code(err::E_ARG)),
                    };
                    // Index 0 is the module itself (SPEC-4.5.1); ids are u16 on the wire.
                    let id = match u16::try_from(resource_id) {
                        Ok(id) if id > 0 && usize::from(id) <= host.images.resource_count() => id,
                        _ => return Err(Fault::Code(err::E_RESOURCE)),
                    };
                    // Decoded at first use (SPEC-6.5.1). Decoding is a
                    // one-time cost bounded by the signed manifest, not by
                    // what the module does per frame, so it is not charged
                    // to the frame budget (SPEC-5.3.3); M3 decodes off the
                    // UI thread.
                    host.images.ensure(id);
                    if let Err(e) = host.images.get(id) {
                        host.host_log(format!("dl_image: resource {id}: {e}"));
                        return Err(Fault::Code(e.code()));
                    }
                    host.dl.image(x, y, w, h, id, fit_mode)?;
                    Ok(())
                })();
                let code = match result {
                    Ok(()) => err::OK,
                    Err(fault) => host.refuse("dl_image", &fault),
                };
                host.charge(host.dl.bytes().len().saturating_sub(before));
                code
            })
        },
    )?;
    linker.func_wrap(
        m,
        "dl_path",
        |mut c: Caller<'_, Host>,
         ptr: i32,
         len: i32,
         color: i32,
         stroke_w: f32|
         -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                let before = host.dl.bytes().len();
                let result = (|| {
                    if !host.dl.is_open() {
                        return Err(Fault::Code(err::E_DL_STATE));
                    }
                    let bytes = slice(data, ptr, len, "path", limits::MAX_PATH_BYTES)?;
                    host.charge(bytes.len());
                    let cmds =
                        vy_dl::decode_path_cmds(bytes).map_err(|_| Fault::Code(err::E_ARG))?;
                    host.dl.path(color as u32, stroke_w, &cmds)?;
                    Ok(())
                })();
                host.charge(host.dl.bytes().len().saturating_sub(before));
                match result {
                    Ok(()) => err::OK,
                    Err(fault) => host.refuse("dl_path", &fault),
                }
            })
        },
    )?;
    linker.func_wrap(
        m,
        "dl_clip_push",
        |mut c: Caller<'_, Host>,
         x: f32,
         y: f32,
         w: f32,
         h: f32,
         radius: f32|
         -> wasmtime::Result<i32> {
            with_host(&mut c, |host| {
                let before = host.dl.bytes().len();
                let code = match host.dl.clip_push(x, y, w, h, radius) {
                    Ok(()) => err::OK,
                    Err(code) => host.refuse("dl_clip_push", &code.into()),
                };
                host.charge(host.dl.bytes().len().saturating_sub(before));
                code
            })
        },
    )?;
    linker.func_wrap(
        m,
        "dl_clip_pop",
        |mut c: Caller<'_, Host>| -> wasmtime::Result<i32> {
            with_host(&mut c, |host| {
                let before = host.dl.bytes().len();
                let code = match host.dl.clip_pop() {
                    Ok(()) => err::OK,
                    Err(code) => host.refuse("dl_clip_pop", &code.into()),
                };
                host.charge(host.dl.bytes().len().saturating_sub(before));
                code
            })
        },
    )?;

    // text (SPEC-5.2.5): shaping is charged per byte of text.
    linker.func_wrap(
        m,
        "text_style",
        |mut c: Caller<'_, Host>,
         font_resource_id: i32,
         size_px: f32,
         weight: i32,
         color: i32|
         -> i32 {
            let host = c.data_mut();
            match host
                .text
                .style(font_resource_id, size_px, weight, color as u32)
            {
                Ok(id) => i32::from(id),
                Err(code) => {
                    host.report("text_style", &code.into());
                    code
                }
            }
        },
    )?;
    linker.func_wrap(
        m,
        "text_measure",
        |mut c: Caller<'_, Host>,
         style_id: i32,
         max_w: f32,
         ptr: i32,
         len: i32|
         -> wasmtime::Result<i64> {
            with_memory(&mut c, |data, host| {
                let result = (|| {
                    let sid = style_u16(style_id)?;
                    if host.text.get_style(sid).is_none() {
                        return Err(Fault::Code(err::E_STYLE));
                    }
                    let text = string(data, ptr, len, "text", limits::MAX_TEXT_BYTES)?;
                    host.charge(text.len());
                    let size = host.text.measure(sid, max_w, &text)?;
                    Ok(pack_x100(size.w, size.h))
                })();
                match result {
                    Ok(v) => v,
                    Err(fault) => {
                        host.report("text_measure", &fault);
                        i64::from(fault.code())
                    }
                }
            })
        },
    )?;
    linker.func_wrap(
        m,
        "text_caret",
        |mut c: Caller<'_, Host>,
         style_id: i32,
         ptr: i32,
         len: i32,
         byte_offset: i32|
         -> wasmtime::Result<i64> {
            with_memory(&mut c, |data, host| {
                let result = (|| {
                    let sid = style_u16(style_id)?;
                    if host.text.get_style(sid).is_none() {
                        return Err(Fault::Code(err::E_STYLE));
                    }
                    let text = string(data, ptr, len, "text", limits::MAX_TEXT_BYTES)?;
                    let offset =
                        usize::try_from(byte_offset).map_err(|_| Fault::Code(err::E_ARG))?;
                    host.charge(text.len());
                    let (x, y) = host.text.caret(sid, &text, offset)?;
                    Ok(pack_x100(x, y))
                })();
                match result {
                    Ok(v) => v,
                    Err(fault) => {
                        host.report("text_caret", &fault);
                        i64::from(fault.code())
                    }
                }
            })
        },
    )?;

    // input (SPEC-5.2.6)
    linker.func_wrap(m, "key_down", |c: Caller<'_, Host>, code: i32| -> i32 {
        i32::from(c.data().keys_down.contains(&code))
    })?;
    linker.func_wrap(
        m,
        "ime_begin",
        |mut c: Caller<'_, Host>, x: f32, y: f32, w: f32, h: f32| -> i32 {
            let host = c.data_mut();
            if host.caps.ime {
                host.ime_rect = Some((x, y, w, h));
                err::OK
            } else {
                host.denied("ime_begin", "ime")
            }
        },
    )?;
    linker.func_wrap(m, "ime_end", |mut c: Caller<'_, Host>| -> i32 {
        let host = c.data_mut();
        if host.caps.ime {
            host.ime_rect = None;
            err::OK
        } else {
            host.denied("ime_end", "ime")
        }
    })?;

    // network (SPEC-5.2.7): the headless host answers from canned responses
    // and otherwise denies; the capability gate comes first (SPEC-5.3.1),
    // then the method and the byte caps (SPEC-9.3.2), then the origin gate
    // (SPEC-5.2.8).
    linker.func_wrap(
        m,
        "fetch_begin",
        |mut c: Caller<'_, Host>,
         http_method: i32,
         url_ptr: i32,
         url_len: i32,
         body_ptr: i32,
         body_len: i32|
         -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                if !host.caps.net {
                    return host.denied("fetch_begin", "net");
                }
                if method::name(http_method).is_none() {
                    host.host_log(format!(
                        "fetch_begin: E_ARG (method {http_method} is not 0 GET, 1 POST, 2 PUT or 3 DELETE; no request made)"
                    ));
                    return err::E_ARG;
                }
                let checked = (|| {
                    let url = string(data, url_ptr, url_len, "url", limits::MAX_URL_BYTES)?;
                    let body = slice(
                        data,
                        body_ptr,
                        body_len,
                        "body",
                        limits::MAX_FETCH_BODY_BYTES,
                    )?;
                    Ok::<(String, usize), Fault>((url, body.len()))
                })();
                let (url, body_len) = match checked {
                    Err(fault) => return host.refuse("fetch_begin", &fault),
                    Ok(v) => v,
                };
                host.charge(url.len() + body_len);
                let Some(origin) = origin_of(&url) else {
                    host.host_log(format!(
                        "fetch_begin: E_ARG ({url:?} is neither https:// nor http:// to loopback)"
                    ));
                    return err::E_ARG;
                };
                // No manifest means no gate, but plain http is never let
                // through by default: only to a loopback origin the
                // manifest names (SPEC-7.1.3).
                let gated = !host.net_origins.is_empty() || origin.starts_with("http://");
                if gated && !host.net_origins.contains(&origin) {
                    host.host_log(format!(
                        "fetch_begin: E_ORIGIN ({origin} is not in the manifest's net list; no request made)"
                    ));
                    return err::E_ORIGIN;
                }
                if host.fetches.len() >= limits::MAX_OPEN_FETCHES as usize {
                    return host.refuse("fetch_begin", &err::E_RATE.into());
                }
                match host.transport {
                    Transport::Canned => {
                        let Some(canned) = host.canned.iter().find(|r| r.url == url) else {
                            return host.denied_headless("fetch_begin", "a transport");
                        };
                        let id = host.next_fetch_id;
                        host.next_fetch_id += 1;
                        host.fetches.push(Fetch {
                            id,
                            status: canned.status,
                            body: canned.body.clone(),
                            pos: 0,
                            answered: true,
                            delivered: false,
                        });
                        id
                    }
                    Transport::External => {
                        // The host process performs the request; the body
                        // arrives through `Runtime::fetch_complete`.
                        let body = data
                            .get(body_ptr as usize..body_ptr as usize + body_len)
                            .map(<[u8]>::to_vec)
                            .unwrap_or_default();
                        let id = host.next_fetch_id;
                        host.next_fetch_id += 1;
                        host.fetches.push(Fetch {
                            id,
                            status: 0,
                            body: Vec::new(),
                            pos: 0,
                            answered: false,
                            delivered: false,
                        });
                        host.fetch_requests.push(FetchRequest {
                            id,
                            method: http_method,
                            url,
                            body,
                        });
                        id
                    }
                }
            })
        },
    )?;
    linker.func_wrap(
        m,
        "fetch_poll",
        |mut c: Caller<'_, Host>, req_id: i32| -> i32 {
            let host = c.data_mut();
            if !host.caps.net {
                return host.denied("fetch_poll", "net");
            }
            // 0 while the response is on its way, 1 once `FETCH_DONE` has
            // been delivered and the body can be read.
            match host.fetch_mut("fetch_poll", req_id) {
                Ok(f) => i32::from(f.delivered),
                Err(code) => code,
            }
        },
    )?;
    linker.func_wrap(
        m,
        "fetch_read",
        |mut c: Caller<'_, Host>, req_id: i32, dst_ptr: i32, max: i32| -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                if !host.caps.net {
                    return host.denied("fetch_read", "net");
                }
                let (pos, remaining, delivered) = match host.fetch_mut("fetch_read", req_id) {
                    Ok(f) => (f.pos, f.body.len() - f.pos, f.delivered),
                    Err(code) => return code,
                };
                if !delivered {
                    host.host_log(format!(
                        "fetch_read: E_ARG (request {req_id} is not readable before FETCH_DONE)"
                    ));
                    return err::E_ARG;
                }
                if dst_ptr < 0 || max <= 0 {
                    return host.refuse("fetch_read", &err::E_ARG.into());
                }
                // The whole body is buffered (SPEC-5.2.7): 0 means the end,
                // and the end closes the request and frees the body.
                let n = remaining.min(max as usize);
                if n == 0 {
                    host.fetches.retain(|f| f.id != req_id);
                    return 0;
                }
                let start = dst_ptr as usize;
                let Some(dst) = start
                    .checked_add(n)
                    .and_then(|end| data.get_mut(start..end))
                else {
                    return host.refuse("fetch_read", &err::E_ARG.into());
                };
                let f = host
                    .fetches
                    .iter_mut()
                    .find(|f| f.id == req_id)
                    .expect("looked up above");
                dst.copy_from_slice(&f.body[pos..pos + n]);
                f.pos += n;
                host.charge(n);
                n as i32
            })
        },
    )?;
    linker.func_wrap(
        m,
        "fetch_cancel",
        |mut c: Caller<'_, Host>, req_id: i32| -> i32 {
            let host = c.data_mut();
            if !host.caps.net {
                return host.denied("fetch_cancel", "net");
            }
            match host.fetch_mut("fetch_cancel", req_id) {
                Ok(_) => {
                    host.fetches.retain(|f| f.id != req_id);
                    err::OK
                }
                Err(code) => code,
            }
        },
    )?;

    // storage (SPEC-5.2.9): a key space in host memory, capped by the quota
    // (SPEC-9.3.1); the capability gate comes first (SPEC-5.3.1), then the
    // byte caps (SPEC-9.3.2).
    linker.func_wrap(
        m,
        "state_put",
        |mut c: Caller<'_, Host>,
         key_ptr: i32,
         key_len: i32,
         val_ptr: i32,
         val_len: i32|
         -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                if !host.caps.storage {
                    return host.denied("state_put", "storage");
                }
                let copied = (|| {
                    let key = slice(data, key_ptr, key_len, "key", limits::MAX_STATE_KEY_BYTES)?;
                    let val = slice(
                        data,
                        val_ptr,
                        val_len,
                        "value",
                        limits::MAX_STATE_VALUE_BYTES,
                    )?;
                    Ok::<(Vec<u8>, Vec<u8>), Fault>((key.to_vec(), val.to_vec()))
                })();
                match copied {
                    Err(fault) => host.refuse("state_put", &fault),
                    Ok((key, val)) => {
                        let old = host
                            .state
                            .get(&key)
                            .map_or(0, |v| (key.len() + v.len()) as u64);
                        let after = host.state_bytes() - old + (key.len() + val.len()) as u64;
                        if after > host.state_quota {
                            return host.refuse("state_put", &err::E_QUOTA.into());
                        }
                        host.charge(key.len() + val.len());
                        host.state.insert(key, val);
                        err::OK
                    }
                }
            })
        },
    )?;
    linker.func_wrap(
        m,
        "state_get",
        |mut c: Caller<'_, Host>,
         key_ptr: i32,
         key_len: i32,
         dst_ptr: i32,
         max: i32|
         -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                if !host.caps.storage {
                    return host.denied("state_get", "storage");
                }
                let key = match slice(data, key_ptr, key_len, "key", limits::MAX_STATE_KEY_BYTES) {
                    Ok(k) => k.to_vec(),
                    Err(fault) => return host.refuse("state_get", &fault),
                };
                let Some(value) = host.state.get(&key).cloned() else {
                    // Nothing saved yet is a normal outcome for a page
                    // (SPEC-5.2.9); noted, not warned about.
                    host.log(
                        LogSource::Host,
                        0,
                        "state_get: E_RESOURCE (no value for the key)".into(),
                    );
                    return err::E_RESOURCE;
                };
                // The full length is reported even above `max`; at most
                // `max` bytes are copied (SPEC-5.2.9).
                let Ok(total) = i32::try_from(value.len()) else {
                    return host.refuse("state_get", &err::E_ARG.into());
                };
                let n = value.len().min(usize::try_from(max).unwrap_or(0));
                if dst_ptr < 0 || max < 0 {
                    return host.refuse("state_get", &err::E_ARG.into());
                }
                let start = dst_ptr as usize;
                let Some(dst) = start
                    .checked_add(n)
                    .and_then(|end| data.get_mut(start..end))
                else {
                    return host.refuse("state_get", &err::E_ARG.into());
                };
                dst.copy_from_slice(&value[..n]);
                host.charge(n);
                total
            })
        },
    )?;
    linker.func_wrap(
        m,
        "state_delete",
        |mut c: Caller<'_, Host>, key_ptr: i32, key_len: i32| -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                if !host.caps.storage {
                    return host.denied("state_delete", "storage");
                }
                match slice(data, key_ptr, key_len, "key", limits::MAX_STATE_KEY_BYTES) {
                    Err(fault) => host.refuse("state_delete", &fault),
                    Ok(key) => {
                        let key = key.to_vec();
                        host.state.remove(&key);
                        err::OK
                    }
                }
            })
        },
    )?;

    // navigation (SPEC-5.2.10): the headless host has nowhere to navigate;
    // the shell takes the address from `Runtime::take_navigations`.
    linker.func_wrap(
        m,
        "navigate",
        |mut c: Caller<'_, Host>,
         url_ptr: i32,
         url_len: i32,
         replace: i32|
         -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                match string(data, url_ptr, url_len, "url", limits::MAX_URL_BYTES) {
                    Ok(url) => {
                        host.charge(url.len());
                        if !(url.starts_with("vy://") || url.starts_with("https://")) {
                            host.refuse("navigate", &err::E_ARG.into())
                        } else if host.navigation {
                            host.navigations.push(Navigation {
                                url,
                                replace: replace != 0,
                            });
                            err::OK
                        } else {
                            host.denied_headless("navigate", "navigation")
                        }
                    }
                    Err(fault) => host.refuse("navigate", &fault),
                }
            })
        },
    )?;
    linker.func_wrap(
        m,
        "link_register",
        |mut c: Caller<'_, Host>,
         id: i32,
         x: f32,
         y: f32,
         w: f32,
         h: f32,
         url_ptr: i32,
         url_len: i32|
         -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                match string(data, url_ptr, url_len, "url", limits::MAX_URL_BYTES) {
                    Ok(url) => {
                        host.charge(url.len());
                        let link = Link {
                            id,
                            x,
                            y,
                            w,
                            h,
                            url,
                        };
                        if let Some(slot) = host.links.iter_mut().find(|l| l.id == id) {
                            *slot = link;
                            err::OK
                        } else if host.links.len() < limits::MAX_LINKS_PER_FRAME as usize {
                            host.links.push(link);
                            err::OK
                        } else {
                            host.refuse("link_register", &err::E_DL_FULL.into())
                        }
                    }
                    Err(fault) => host.refuse("link_register", &fault),
                }
            })
        },
    )?;

    // time and randomness (SPEC-5.2.11)
    linker.func_wrap(m, "now_ms", |c: Caller<'_, Host>| -> i64 {
        c.data().clock_ms
    })?;
    linker.func_wrap(
        m,
        "random",
        |mut c: Caller<'_, Host>, dst_ptr: i32, len: i32| -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                if dst_ptr < 0 || len < 0 {
                    return host.refuse("random", &err::E_ARG.into());
                }
                let (start, len) = (dst_ptr as usize, len as usize);
                let Some(dst) = start
                    .checked_add(len)
                    .and_then(|end| data.get_mut(start..end))
                else {
                    return host.refuse("random", &err::E_ARG.into());
                };
                host.charge(len);
                host.rng.fill(dst);
                err::OK
            })
        },
    )?;

    // clipboard (SPEC-5.2.12): the capability gate comes first
    // (SPEC-5.3.1), then the gesture window, then the arguments; the
    // headless host serves the clipboard the caller gave it, if any.
    linker.func_wrap(
        m,
        "clipboard_read",
        |mut c: Caller<'_, Host>, dst_ptr: i32, max: i32| -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                if !host.caps.clipboard {
                    return host.denied("clipboard_read", "clipboard");
                }
                if !host.recent_gesture() {
                    return host.refuse("clipboard_read", &err::E_GESTURE.into());
                }
                let Some(clip) = host.clipboard.clone() else {
                    return host.denied_headless("clipboard_read", "the clipboard");
                };
                if dst_ptr < 0 || max < 0 {
                    return host.refuse("clipboard_read", &err::E_ARG.into());
                }
                let Ok(len) = i32::try_from(clip.len()) else {
                    return host.refuse("clipboard_read", &err::E_ARG.into());
                };
                // `max` = 0 asks for the length; otherwise the whole
                // clipboard is copied or nothing is.
                if max == 0 {
                    return len;
                }
                if len > max {
                    host.host_log(format!(
                        "clipboard_read: E_ARG (clipboard is {len} bytes, buffer is {max}; nothing copied)"
                    ));
                    return err::E_ARG;
                }
                let start = dst_ptr as usize;
                let Some(dst) = start
                    .checked_add(clip.len())
                    .and_then(|end| data.get_mut(start..end))
                else {
                    return host.refuse("clipboard_read", &err::E_ARG.into());
                };
                dst.copy_from_slice(&clip);
                host.charge(clip.len());
                len
            })
        },
    )?;
    linker.func_wrap(
        m,
        "clipboard_write",
        |mut c: Caller<'_, Host>, ptr: i32, len: i32| -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                // A write during a copy gesture (Ctrl+C or Ctrl+X in the
                // page) needs no capability (SPEC-8.2.4): the user asked
                // for it, and a write leaks nothing.
                if !host.caps.clipboard && !host.copy_gesture {
                    return host.denied("clipboard_write", "clipboard");
                }
                match slice(data, ptr, len, "text", limits::MAX_CLIPBOARD_BYTES) {
                    Err(fault) => host.refuse("clipboard_write", &fault),
                    Ok(bytes) => {
                        if host.clipboard.is_none() {
                            return host.denied_headless("clipboard_write", "the clipboard");
                        }
                        let bytes = bytes.to_vec();
                        host.charge(bytes.len());
                        host.clipboard_written = Some(bytes.clone());
                        host.clipboard = Some(bytes);
                        err::OK
                    }
                }
            })
        },
    )?;

    // semantics (SPEC-5.2.13)
    linker.func_wrap(
        m,
        "sem_node",
        |mut c: Caller<'_, Host>,
         id: i32,
         parent_id: i32,
         role: i32,
         x: f32,
         y: f32,
         w: f32,
         h: f32,
         ptr: i32,
         len: i32|
         -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                let result = (|| {
                    if vy_abi::role::name(role).is_none() {
                        return Err(Fault::Code(err::E_ARG));
                    }
                    if host.sem.len() >= limits::MAX_SEM_NODES_PER_FRAME as usize {
                        return Err(Fault::Code(err::E_DL_FULL));
                    }
                    // One node per id and frame (SPEC-5.2.13); a set lookup,
                    // so the frame's cost stays linear in its node count.
                    if host.sem_ids.contains(&id) {
                        return Err(Fault::Code(err::E_ARG));
                    }
                    let text = if len == 0 {
                        String::new()
                    } else {
                        string(data, ptr, len, "text", limits::MAX_SEM_TEXT_BYTES)?
                    };
                    host.frame_text_room(text.len())?;
                    host.charge(text.len());
                    host.sem_text_bytes += text.len();
                    host.sem_ids.insert(id);
                    host.sem.push(SemNode {
                        id,
                        parent_id,
                        role,
                        x,
                        y,
                        w,
                        h,
                        text,
                    });
                    Ok(())
                })();
                match result {
                    Ok(()) => err::OK,
                    // A full tree is reported once per frame, like the first
                    // dropped opcode is what matters (SPEC-6.1.6).
                    Err(Fault::Code(err::E_DL_FULL)) => {
                        if !host.sem_full_logged {
                            host.sem_full_logged = true;
                            host.report("sem_node", &err::E_DL_FULL.into());
                        }
                        err::E_DL_FULL
                    }
                    Err(fault) => host.refuse("sem_node", &fault),
                }
            })
        },
    )?;

    Ok(())
}
