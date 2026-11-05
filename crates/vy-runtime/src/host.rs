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
//!   (SPEC-5.3.3).

use crate::FUEL_PER_HOST_BYTE;
use vy_abi::{cap, err, fit, limits, ABI_VERSION};
use vy_dl::DisplayList;
use vy_render::{pack_x100, ImageCache, TextEngine};
use wasmtime::{Caller, Linker, Memory, StoreLimits, Trap};

/// Milliseconds the headless clock advances per completed frame.
pub const HEADLESS_FRAME_MS: i64 = 16;
/// Seed of the headless random source; fixed so renders repeat byte for byte.
pub const HEADLESS_RANDOM_SEED: u64 = 0x5EED_5EED_0000_0001;
/// Upper bound on retained log entries per instance.
pub const MAX_LOG_ENTRIES: usize = 10_000;
/// Upper bound on links registered in one frame (SPEC-9.3.2); the list is
/// emptied by every `dl_begin` (SPEC-5.2.10).
pub const MAX_LINKS_PER_FRAME: usize = 4_096;

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
    /// Module-chosen level (0 debug .. 3 error); host entries use 2 (warn).
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
    pub(crate) sem_text_bytes: usize,
    pub(crate) links: Vec<Link>,
    pub(crate) log: Vec<LogEntry>,
    pub(crate) clock_ms: i64,
    pub(crate) rng: u64,
    pub(crate) wants_frame: bool,
    pub(crate) exited: Option<i32>,
    pub(crate) keys_down: Vec<i32>,
    pub(crate) ime_rect: Option<(f32, f32, f32, f32)>,
    pub(crate) sem_full_logged: bool,
    /// Fuel owed for host work in the current import call; settled by
    /// [`with_host`] / [`with_memory`] before the call returns to wasm.
    pending_fuel: u64,
    /// Fuel charged for host work since the last [`Host::take_host_fuel`].
    host_fuel: u64,
}

impl Host {
    pub(crate) fn new(limits: StoreLimits, caps: Capabilities, resources: Vec<Vec<u8>>) -> Self {
        Self {
            memory: None,
            limits,
            caps,
            dl: DisplayList::new(),
            text: TextEngine::new(),
            images: ImageCache::new(resources),
            sem: Vec::new(),
            sem_text_bytes: 0,
            links: Vec::new(),
            log: Vec::new(),
            clock_ms: 0,
            rng: HEADLESS_RANDOM_SEED,
            wants_frame: false,
            exited: None,
            keys_down: Vec::new(),
            ime_rect: None,
            sem_full_logged: false,
            pending_fuel: 0,
            host_fuel: 0,
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

    fn host_log(&mut self, message: String) {
        self.log(LogSource::Host, 2, message);
    }

    /// Log a denied call and return `E_DENIED` (SPEC-5.3.1).
    fn denied(&mut self, name: &str, capability: &str) -> i32 {
        self.host_log(format!(
            "{name}: E_DENIED (capability `{capability}` not granted)"
        ));
        err::E_DENIED
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

    /// Next 64 random bits (xorshift64*; headless only, see `HEADLESS_RANDOM_SEED`).
    fn next_random(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Name of an ABI status code for log lines.
pub fn err_name(code: i32) -> &'static str {
    match code {
        err::OK => "OK",
        err::E_DENIED => "E_DENIED",
        err::E_ARG => "E_ARG",
        err::E_OOM => "E_OOM",
        err::E_QUOTA => "E_QUOTA",
        err::E_RATE => "E_RATE",
        err::E_DL_STATE => "E_DL_STATE",
        err::E_DL_FULL => "E_DL_FULL",
        err::E_DL_CLIP => "E_DL_CLIP",
        err::E_RESOURCE => "E_RESOURCE",
        err::E_STYLE => "E_STYLE",
        err::E_ORIGIN => "E_ORIGIN",
        err::E_NET => "E_NET",
        err::E_TOO_LARGE => "E_TOO_LARGE",
        err::E_GESTURE => "E_GESTURE",
        _ => "E_UNKNOWN",
    }
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
        |mut c: Caller<'_, Host>, level: i32, ptr: i32, len: i32| -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                match slice(data, ptr, len, "message", limits::MAX_LOG_BYTES) {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(bytes).into_owned();
                        host.charge(text.len());
                        host.log(LogSource::Module, level, text);
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
                    // Decoded at first use (SPEC-6.5.1); the resource bytes
                    // the host reads for it are charged like any host work
                    // (SPEC-5.3.3). A cache hit reads nothing.
                    let read = host.images.ensure(id);
                    host.charge(read);
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

    // network (SPEC-5.2.7): M3 implements fetching. The capability gate comes
    // first (SPEC-5.3.1), then the byte caps, so M3 inherits both; a headless
    // host with `net` granted still has no transport and denies.
    linker.func_wrap(
        m,
        "fetch_begin",
        |mut c: Caller<'_, Host>,
         _method: i32,
         url_ptr: i32,
         url_len: i32,
         body_ptr: i32,
         body_len: i32|
         -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                if !host.caps.net {
                    return host.denied("fetch_begin", "net");
                }
                let checked = (|| {
                    slice(data, url_ptr, url_len, "url", limits::MAX_URL_BYTES)?;
                    slice(
                        data,
                        body_ptr,
                        body_len,
                        "body",
                        limits::MAX_FETCH_BODY_BYTES,
                    )?;
                    Ok::<(), Fault>(())
                })();
                match checked {
                    Err(fault) => {
                        host.report("fetch_begin", &fault);
                        fault.code()
                    }
                    Ok(()) => host.denied("fetch_begin", "net (headless host)"),
                }
            })
        },
    )?;
    linker.func_wrap(
        m,
        "fetch_poll",
        |mut c: Caller<'_, Host>, _req_id: i32| -> i32 { c.data_mut().denied("fetch_poll", "net") },
    )?;
    linker.func_wrap(
        m,
        "fetch_read",
        |mut c: Caller<'_, Host>, _req_id: i32, _dst_ptr: i32, _max: i32| -> i32 {
            c.data_mut().denied("fetch_read", "net")
        },
    )?;
    linker.func_wrap(
        m,
        "fetch_cancel",
        |mut c: Caller<'_, Host>, _req_id: i32| -> i32 {
            c.data_mut().denied("fetch_cancel", "net")
        },
    )?;

    // storage (SPEC-5.2.9): M3 implements storage; same gate order as above.
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
                let checked = (|| {
                    slice(data, key_ptr, key_len, "key", limits::MAX_STATE_KEY_BYTES)?;
                    slice(
                        data,
                        val_ptr,
                        val_len,
                        "value",
                        limits::MAX_STATE_VALUE_BYTES,
                    )?;
                    Ok::<(), Fault>(())
                })();
                match checked {
                    Err(fault) => {
                        host.report("state_put", &fault);
                        fault.code()
                    }
                    Ok(()) => host.denied("state_put", "storage (headless host)"),
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
         _dst_ptr: i32,
         _max: i32|
         -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                if !host.caps.storage {
                    return host.denied("state_get", "storage");
                }
                match slice(data, key_ptr, key_len, "key", limits::MAX_STATE_KEY_BYTES) {
                    Err(fault) => {
                        host.report("state_get", &fault);
                        fault.code()
                    }
                    Ok(_) => host.denied("state_get", "storage (headless host)"),
                }
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
                    Ok(_) => host.denied("state_delete", "storage (headless host)"),
                }
            })
        },
    )?;

    // navigation (SPEC-5.2.10): the headless host has nowhere to navigate.
    linker.func_wrap(
        m,
        "navigate",
        |mut c: Caller<'_, Host>,
         url_ptr: i32,
         url_len: i32,
         _replace: i32|
         -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                match string(data, url_ptr, url_len, "url", limits::MAX_URL_BYTES) {
                    Ok(url) => {
                        host.charge(url.len());
                        if url.starts_with("vy://") || url.starts_with("https://") {
                            host.denied("navigate", "navigation (headless host)")
                        } else {
                            host.refuse("navigate", &err::E_ARG.into())
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
                        } else if host.links.len() < MAX_LINKS_PER_FRAME {
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
                for chunk in dst.chunks_mut(8) {
                    let bytes = host.next_random().to_le_bytes();
                    chunk.copy_from_slice(&bytes[..chunk.len()]);
                }
                err::OK
            })
        },
    )?;

    // clipboard (SPEC-5.2.12): never granted headless; same gate order as above.
    linker.func_wrap(
        m,
        "clipboard_read",
        |mut c: Caller<'_, Host>, _dst_ptr: i32, _max: i32| -> i32 {
            c.data_mut().denied("clipboard_read", "clipboard")
        },
    )?;
    linker.func_wrap(
        m,
        "clipboard_write",
        |mut c: Caller<'_, Host>, ptr: i32, len: i32| -> wasmtime::Result<i32> {
            with_memory(&mut c, |data, host| {
                if !host.caps.clipboard {
                    return host.denied("clipboard_write", "clipboard");
                }
                match slice(data, ptr, len, "text", limits::MAX_CLIPBOARD_BYTES) {
                    Err(fault) => host.refuse("clipboard_write", &fault),
                    Ok(_) => host.denied("clipboard_write", "clipboard (headless host)"),
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
                    // One node per id and frame (SPEC-5.2.13).
                    if host.sem.iter().any(|n| n.id == id) {
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
