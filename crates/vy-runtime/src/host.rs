//! Host state behind the `vy` imports (SPEC §5.2) and the functions themselves.
//!
//! Everything outside pure computation and drawing is a capability
//! (SPEC-2.1.4): an import whose capability is not granted returns `E_DENIED`,
//! logs, and leaves the module running (SPEC-5.3.1). Time and randomness go
//! through the host (SPEC-2.1.5): the headless clock advances only per frame
//! and the random source is a seeded generator.

use vy_abi::{err, ABI_VERSION};
use vy_dl::DisplayList;
use vy_render::{height_x100, pack_x100, TextEngine};
use wasmtime::{Caller, Linker, Memory, StoreLimits};

/// Milliseconds the headless clock advances per completed frame.
pub const HEADLESS_FRAME_MS: i64 = 16;
/// Seed of the headless random source; fixed so renders repeat byte for byte.
pub const HEADLESS_RANDOM_SEED: u64 = 0x5EED_5EED_0000_0001;
/// Upper bound on retained log entries per instance.
pub const MAX_LOG_ENTRIES: usize = 10_000;
/// Upper bound on semantic nodes per frame.
pub const MAX_SEM_NODES: usize = 65_536;
/// Upper bound on registered links.
pub const MAX_LINKS: usize = 4_096;

/// Capabilities granted to the instance (SPEC-8.2.1). All false in headless M1.
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
    pub(crate) sem: Vec<SemNode>,
    pub(crate) links: Vec<Link>,
    pub(crate) log: Vec<LogEntry>,
    pub(crate) clock_ms: i64,
    pub(crate) rng: u64,
    pub(crate) wants_frame: bool,
    pub(crate) exited: Option<i32>,
    pub(crate) pointer: (i32, i32),
    pub(crate) keys_down: Vec<i32>,
    pub(crate) ime_rect: Option<(f32, f32, f32, f32)>,
    pub(crate) sem_overflow_logged: bool,
}

impl Host {
    pub(crate) fn new(limits: StoreLimits, caps: Capabilities) -> Self {
        Self {
            memory: None,
            limits,
            caps,
            dl: DisplayList::new(),
            text: TextEngine::new(),
            sem: Vec::new(),
            links: Vec::new(),
            log: Vec::new(),
            clock_ms: 0,
            rng: HEADLESS_RANDOM_SEED,
            wants_frame: false,
            exited: None,
            pointer: (0, 0),
            keys_down: Vec::new(),
            ime_rect: None,
            sem_overflow_logged: false,
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

    fn report(&mut self, name: &str, code: i32) {
        if code != err::OK {
            self.host_log(format!("{name}: {}", err_name(code)));
        }
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

/// Borrow the module memory and host state together.
///
/// The memory handle is stored right after instantiation; the ABI check
/// refuses start functions, so no import can run before that.
fn with_memory<R>(caller: &mut Caller<'_, Host>, f: impl FnOnce(&mut [u8], &mut Host) -> R) -> R {
    match caller.data().memory {
        Some(memory) => {
            let (data, host) = memory.data_and_store_mut(caller);
            f(data, host)
        }
        None => f(&mut [], caller.data_mut()),
    }
}

/// Slice of module memory, or `E_ARG` when outside it.
fn slice(data: &[u8], ptr: i32, len: i32) -> Result<&[u8], i32> {
    if ptr < 0 || len < 0 {
        return Err(err::E_ARG);
    }
    let start = ptr as usize;
    let end = start.checked_add(len as usize).ok_or(err::E_ARG)?;
    data.get(start..end).ok_or(err::E_ARG)
}

/// UTF-8 string in module memory, or `E_ARG`.
fn string(data: &[u8], ptr: i32, len: i32) -> Result<String, i32> {
    let bytes = slice(data, ptr, len)?;
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| err::E_ARG)
}

fn style_u16(style_id: i32) -> Result<u16, i32> {
    u16::try_from(style_id).map_err(|_| err::E_STYLE)
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
        |mut c: Caller<'_, Host>, level: i32, ptr: i32, len: i32| {
            with_memory(&mut c, |data, host| match slice(data, ptr, len) {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(bytes).into_owned();
                    host.log(LogSource::Module, level, text);
                }
                Err(code) => host.report("log", code),
            });
        },
    )?;
    linker.func_wrap(m, "request_frame", |mut c: Caller<'_, Host>| {
        c.data_mut().wants_frame = true;
    })?;
    linker.func_wrap(m, "exit", |mut c: Caller<'_, Host>, code: i32| {
        c.data_mut().exited = Some(code);
    })?;

    // display list (SPEC-5.2.3)
    linker.func_wrap(m, "dl_begin", |mut c: Caller<'_, Host>| {
        c.data_mut().dl.begin();
    })?;
    linker.func_wrap(m, "dl_end", |mut c: Caller<'_, Host>| {
        let host = c.data_mut();
        if let Err(code) = host.dl.end() {
            host.report("dl_end", code);
        }
    })?;
    linker.func_wrap(
        m,
        "dl_rect",
        |mut c: Caller<'_, Host>, x: f32, y: f32, w: f32, h: f32, radius: f32, color: i32| {
            let host = c.data_mut();
            if let Err(code) = host.dl.rect(x, y, w, h, radius, color as u32) {
                host.report("dl_rect", code);
            }
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
         -> i32 {
            with_memory(&mut c, |data, host| {
                let result = (|| {
                    let text = string(data, ptr, len)?;
                    let sid = style_u16(style_id)?;
                    let size = host.text.measure(sid, w, &text)?;
                    host.dl.text(x, y, w, sid, text.as_bytes())?;
                    Ok(height_x100(size.h))
                })();
                match result {
                    Ok(h) => h,
                    Err(code) => {
                        host.report("dl_text", code);
                        code
                    }
                }
            })
        },
    )?;
    linker.func_wrap(
        m,
        "dl_image",
        |mut c: Caller<'_, Host>, x: f32, y: f32, w: f32, h: f32, resource_id: i32| {
            let host = c.data_mut();
            // Index 0 is the module itself (SPEC-4.5.1); ids are u16 on the wire.
            let result = match u16::try_from(resource_id) {
                Ok(id) if id > 0 => host.dl.image(x, y, w, h, id, vy_dl::Fit::Cover),
                _ => Err(err::E_RESOURCE),
            };
            if let Err(code) = result {
                host.report("dl_image", code);
            }
        },
    )?;
    linker.func_wrap(
        m,
        "dl_path",
        |mut c: Caller<'_, Host>, ptr: i32, len: i32, color: i32, stroke_w: f32| {
            with_memory(&mut c, |data, host| {
                let result = (|| {
                    let bytes = slice(data, ptr, len)?;
                    let cmds = vy_dl::decode_path_cmds(bytes).map_err(|_| err::E_ARG)?;
                    host.dl.path(color as u32, stroke_w, &cmds)
                })();
                if let Err(code) = result {
                    host.report("dl_path", code);
                }
            });
        },
    )?;
    linker.func_wrap(
        m,
        "dl_clip_push",
        |mut c: Caller<'_, Host>, x: f32, y: f32, w: f32, h: f32| {
            let host = c.data_mut();
            if let Err(code) = host.dl.clip_push(x, y, w, h) {
                host.report("dl_clip_push", code);
            }
        },
    )?;
    linker.func_wrap(m, "dl_clip_pop", |mut c: Caller<'_, Host>| {
        let host = c.data_mut();
        if let Err(code) = host.dl.clip_pop() {
            host.report("dl_clip_pop", code);
        }
    })?;

    // text (SPEC-5.2.5)
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
                    host.report("text_style", code);
                    code
                }
            }
        },
    )?;
    linker.func_wrap(
        m,
        "text_measure",
        |mut c: Caller<'_, Host>, style_id: i32, max_w: f32, ptr: i32, len: i32| -> i64 {
            with_memory(&mut c, |data, host| {
                let result = (|| {
                    let text = string(data, ptr, len)?;
                    let size = host.text.measure(style_u16(style_id)?, max_w, &text)?;
                    Ok(pack_x100(size.w, size.h))
                })();
                match result {
                    Ok(v) => v,
                    Err(code) => {
                        host.report("text_measure", code);
                        i64::from(code)
                    }
                }
            })
        },
    )?;
    linker.func_wrap(
        m,
        "text_caret",
        |mut c: Caller<'_, Host>, style_id: i32, ptr: i32, len: i32, byte_offset: i32| -> i64 {
            with_memory(&mut c, |data, host| {
                let result = (|| {
                    let text = string(data, ptr, len)?;
                    let offset = usize::try_from(byte_offset).map_err(|_| err::E_ARG)?;
                    let (x, y) = host.text.caret(style_u16(style_id)?, &text, offset)?;
                    Ok(pack_x100(x, y))
                })();
                match result {
                    Ok(v) => v,
                    Err(code) => {
                        host.report("text_caret", code);
                        i64::from(code)
                    }
                }
            })
        },
    )?;

    // input (SPEC-5.2.6)
    linker.func_wrap(m, "pointer_get", |c: Caller<'_, Host>| -> i64 {
        let (x, y) = c.data().pointer;
        (i64::from(x) << 32) | i64::from(y as u32)
    })?;
    linker.func_wrap(m, "key_down", |c: Caller<'_, Host>, code: i32| -> i32 {
        i32::from(c.data().keys_down.contains(&code))
    })?;
    linker.func_wrap(
        m,
        "ime_begin",
        |mut c: Caller<'_, Host>, x: f32, y: f32, w: f32, h: f32| {
            let host = c.data_mut();
            if host.caps.ime {
                host.ime_rect = Some((x, y, w, h));
            } else {
                host.denied("ime_begin", "ime");
            }
        },
    )?;
    linker.func_wrap(m, "ime_end", |mut c: Caller<'_, Host>| {
        let host = c.data_mut();
        if host.caps.ime {
            host.ime_rect = None;
        } else {
            host.denied("ime_end", "ime");
        }
    })?;

    // network (SPEC-5.2.7): M3 implements fetching; until then `net` is never granted.
    linker.func_wrap(
        m,
        "fetch_begin",
        |mut c: Caller<'_, Host>,
         _method: i32,
         _url_ptr: i32,
         _url_len: i32,
         _body_ptr: i32,
         _body_len: i32|
         -> i32 { c.data_mut().denied("fetch_begin", "net") },
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
        |mut c: Caller<'_, Host>, _req_id: i32| {
            c.data_mut().denied("fetch_cancel", "net");
        },
    )?;

    // storage (SPEC-5.2.9): M3 implements storage; until then `storage` is never granted.
    linker.func_wrap(
        m,
        "state_put",
        |mut c: Caller<'_, Host>,
         _key_ptr: i32,
         _key_len: i32,
         _val_ptr: i32,
         _val_len: i32|
         -> i32 { c.data_mut().denied("state_put", "storage") },
    )?;
    linker.func_wrap(
        m,
        "state_get",
        |mut c: Caller<'_, Host>, _key_ptr: i32, _key_len: i32, _dst_ptr: i32, _max: i32| -> i32 {
            c.data_mut().denied("state_get", "storage")
        },
    )?;
    linker.func_wrap(
        m,
        "state_delete",
        |mut c: Caller<'_, Host>, _key_ptr: i32, _key_len: i32| {
            c.data_mut().denied("state_delete", "storage");
        },
    )?;

    // navigation (SPEC-5.2.10): the headless host has nowhere to navigate.
    linker.func_wrap(
        m,
        "navigate",
        |mut c: Caller<'_, Host>, url_ptr: i32, url_len: i32, _replace: i32| {
            with_memory(&mut c, |data, host| match string(data, url_ptr, url_len) {
                Ok(url) if url.starts_with("vy://") || url.starts_with("https://") => {
                    host.denied("navigate", "navigation (headless host)");
                }
                _ => host.report("navigate", err::E_ARG),
            });
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
         url_len: i32| {
            with_memory(&mut c, |data, host| match string(data, url_ptr, url_len) {
                Ok(url) => {
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
                    } else if host.links.len() < MAX_LINKS {
                        host.links.push(link);
                    } else {
                        host.report("link_register", err::E_QUOTA);
                    }
                }
                Err(code) => host.report("link_register", code),
            });
        },
    )?;

    // time and randomness (SPEC-5.2.11)
    linker.func_wrap(m, "now_ms", |c: Caller<'_, Host>| -> i64 {
        c.data().clock_ms
    })?;
    linker.func_wrap(
        m,
        "random",
        |mut c: Caller<'_, Host>, dst_ptr: i32, len: i32| {
            with_memory(&mut c, |data, host| {
                if dst_ptr < 0 || len < 0 {
                    host.report("random", err::E_ARG);
                    return;
                }
                let (start, len) = (dst_ptr as usize, len as usize);
                let Some(dst) = start
                    .checked_add(len)
                    .and_then(|end| data.get_mut(start..end))
                else {
                    host.report("random", err::E_ARG);
                    return;
                };
                for chunk in dst.chunks_mut(8) {
                    let bytes = host.next_random().to_le_bytes();
                    chunk.copy_from_slice(&bytes[..chunk.len()]);
                }
            });
        },
    )?;

    // clipboard (SPEC-5.2.12): never granted headless.
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
        |mut c: Caller<'_, Host>, _ptr: i32, _len: i32| {
            c.data_mut().denied("clipboard_write", "clipboard");
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
         len: i32| {
            with_memory(&mut c, |data, host| {
                if vy_abi::role::name(role).is_none() {
                    host.report("sem_node", err::E_ARG);
                    return;
                }
                let text = if len == 0 {
                    Ok(String::new())
                } else {
                    string(data, ptr, len)
                };
                match text {
                    Ok(text) => {
                        if host.sem.len() < MAX_SEM_NODES {
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
                        } else if !host.sem_overflow_logged {
                            host.sem_overflow_logged = true;
                            host.report("sem_node", err::E_QUOTA);
                        }
                    }
                    Err(code) => host.report("sem_node", code),
                }
            });
        },
    )?;

    Ok(())
}
