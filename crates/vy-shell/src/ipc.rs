//! Framing and messages between the main process and a tab process
//! (SPEC-11.2.1, SPEC-11.2.2).
//!
//! A frame on the wire is `u32 len` (little-endian, the number of bytes
//! that follow), then `u8 tag`, then `len - 1` bytes of payload. Every
//! message is one frame; the payload is fixed-width integers and
//! length-prefixed byte strings, little-endian, no alignment, no
//! self-description: the tag says which message, the message says what
//! fields follow (one canonical encoding, SPEC-2.1.1). A frame the other
//! side cannot decode is an error that ends the connection, never a
//! guess.
//!
//! The transport underneath is a pair of pipes in M3 (the child's stdin
//! and stdout); the [`IpcTransport`] trait is what the tab loop talks to,
//! so a shared-memory ring buffer per tab (the target of SPEC-11.2.1) can
//! replace the pipes without touching the messages: it carries the same
//! frames, only without the copy.

use std::io::{self, BufRead, Read, Write};
use std::sync::mpsc::{Receiver, Sender};
use vy_abi::limits;
use vy_render::RgbaImage;
use vy_runtime::{Link, RandomSource, SemNode};

/// Largest frame either side accepts: one maximal decoded image
/// (`vy_render::MAX_IMAGE_CACHE_BYTES`, 256 MB) plus room for everything
/// else. Bounded so a misbehaving tab process cannot make the main
/// process allocate without end (SPEC-9.3.1).
pub const MAX_FRAME_BYTES: u32 = vy_render::MAX_IMAGE_CACHE_BYTES as u32 + (1 << 20);

/// Largest display list in one [`ToShell::Frame`]. A runtime that keeps
/// its budget copies at most a couple of megabytes out of the module per
/// call (`vy_runtime::CALL_HARD_LIMIT_FUEL` / `FUEL_PER_HOST_BYTE`), so a
/// list beyond this did not come from one (SPEC-9.3.2).
pub const MAX_DISPLAY_LIST_BYTES: usize = 64 << 20;
/// Largest text in a log line, an error or a stop reason from the tab:
/// a module's `log` is 4 KB (SPEC-9.3.2) and the host's own lines carry
/// at most a URL and a message.
pub const MAX_MESSAGE_TEXT_BYTES: usize = 64 << 10;
/// Largest storage a tab may hand back: the quota's ceiling (SPEC-5.2.9,
/// SPEC-8.2.1: `quota_kb` ≤ 16 384).
pub const MAX_STORAGE_BYTES: usize = 16 << 20;
/// Most lines in each list of a [`ToShell::Sandbox`] report: a lockdown
/// has a handful of layers.
pub const MAX_SANDBOX_LINES: usize = 32;
/// Largest line in a sandbox report: a layer's name or why it failed.
pub const MAX_SANDBOX_LINE_BYTES: usize = 1024;

/// Why a frame or a message could not be read.
#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The payload ended before the field.
    Short,
    /// Bytes that should be UTF-8 were not.
    Utf8,
    /// A tag no message has.
    Tag(u8),
    /// A value outside its domain (an enum byte, a length over the cap).
    Value(&'static str),
    /// Bytes left over after the message.
    Trailing(usize),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Short => f.write_str("message ends early"),
            Self::Utf8 => f.write_str("text is not UTF-8"),
            Self::Tag(t) => write!(f, "unknown message tag {t}"),
            Self::Value(what) => write!(f, "bad value for {what}"),
            Self::Trailing(n) => write!(f, "{n} bytes after the message"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Byte writer for message payloads.
#[derive(Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    /// A writer that starts with the tag.
    pub fn new(tag: u8) -> Self {
        Self(vec![tag])
    }
    /// The frame body: tag and payload.
    pub fn finish(self) -> Vec<u8> {
        self.0
    }
    /// One byte.
    pub fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    /// One boolean as a byte.
    pub fn bool(&mut self, v: bool) {
        self.0.push(u8::from(v));
    }
    /// A `u16`.
    pub fn u16(&mut self, v: u16) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// A `u32`.
    pub fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// An `i32`.
    pub fn i32(&mut self, v: i32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// A `u64`.
    pub fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// An `i64`.
    pub fn i64(&mut self, v: i64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// An `f32`.
    pub fn f32(&mut self, v: f32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    /// A byte string with a `u32` length prefix.
    pub fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.0.extend_from_slice(v);
    }
    /// Text with a `u32` length prefix.
    pub fn str(&mut self, v: &str) {
        self.bytes(v.as_bytes());
    }
    /// An optional byte string: a flag, then the string if present.
    pub fn opt_bytes(&mut self, v: Option<&[u8]>) {
        match v {
            None => self.bool(false),
            Some(b) => {
                self.bool(true);
                self.bytes(b);
            }
        }
    }
}

/// Byte reader for message payloads.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Over the bytes after the tag.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n).ok_or(DecodeError::Short)?;
        let s = self.data.get(self.pos..end).ok_or(DecodeError::Short)?;
        self.pos = end;
        Ok(s)
    }
    /// Nothing may follow a message.
    pub fn finish(self) -> Result<(), DecodeError> {
        let left = self.data.len() - self.pos;
        if left == 0 {
            Ok(())
        } else {
            Err(DecodeError::Trailing(left))
        }
    }
    /// One byte.
    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }
    /// A boolean byte; anything but 0 or 1 is an error.
    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::Value("bool")),
        }
    }
    /// A `u16`.
    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    /// A `u32`.
    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    /// An `i32`.
    pub fn i32(&mut self) -> Result<i32, DecodeError> {
        self.u32().map(|v| v as i32)
    }
    /// A `u64`.
    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }
    /// An `i64`.
    pub fn i64(&mut self) -> Result<i64, DecodeError> {
        self.u64().map(|v| v as i64)
    }
    /// An `f32`.
    pub fn f32(&mut self) -> Result<f32, DecodeError> {
        self.u32().map(f32::from_bits)
    }
    /// A length-prefixed byte string.
    pub fn bytes(&mut self) -> Result<&'a [u8], DecodeError> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    /// A length-prefixed byte string of at most `max` bytes; a longer one
    /// is a bad value for `what` (SPEC-9.3.2: the caps hold on this side
    /// of the pipe too, since the other side is not trusted).
    pub fn bytes_max(&mut self, max: usize, what: &'static str) -> Result<&'a [u8], DecodeError> {
        let n = self.u32()? as usize;
        if n > max {
            return Err(DecodeError::Value(what));
        }
        self.take(n)
    }
    /// A length-prefixed UTF-8 string.
    pub fn str(&mut self) -> Result<&'a str, DecodeError> {
        std::str::from_utf8(self.bytes()?).map_err(|_| DecodeError::Utf8)
    }
    /// A length-prefixed UTF-8 string of at most `max` bytes.
    pub fn str_max(&mut self, max: usize, what: &'static str) -> Result<&'a str, DecodeError> {
        std::str::from_utf8(self.bytes_max(max, what)?).map_err(|_| DecodeError::Utf8)
    }
    /// An optional byte string.
    pub fn opt_bytes(&mut self) -> Result<Option<&'a [u8]>, DecodeError> {
        if self.bool()? {
            self.bytes().map(Some)
        } else {
            Ok(None)
        }
    }
    /// A count for a list that follows; capped so a hostile length cannot
    /// reserve memory before the elements are read.
    fn count(&mut self, what: &'static str) -> Result<usize, DecodeError> {
        let n = self.u32()? as usize;
        if n > self.data.len() - self.pos {
            return Err(DecodeError::Value(what));
        }
        Ok(n)
    }
}

/// Write one frame: the length, then `body` (tag and payload).
pub fn write_frame(w: &mut impl Write, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len())
        .ok()
        .filter(|l| *l >= 1 && *l <= MAX_FRAME_BYTES)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "frame too large"))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(body)?;
    w.flush()
}

/// Read one frame: `Ok(None)` at a clean end of stream, an error for a
/// length outside `1..=MAX_FRAME_BYTES` or a stream cut mid-frame.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len);
    if len == 0 || len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("frame length {len} outside 1..={MAX_FRAME_BYTES}"),
        ));
    }
    let mut body = vec![0u8; len as usize];
    r.read_exact(&mut body)?;
    Ok(Some(body))
}

/// A two-way channel of frames, as the tab loop sees it. Pipes in M3
/// ([`PipeTransport`]); an in-process channel for tests
/// ([`ChannelTransport`]); a shared-memory ring buffer later.
pub trait IpcTransport {
    /// Send one frame body (tag and payload).
    fn send(&mut self, body: &[u8]) -> io::Result<()>;
    /// Receive the next frame body; `None` when the other side is gone.
    fn recv(&mut self) -> io::Result<Option<Vec<u8>>>;
}

/// Frames over a reader and a writer: a child's stdin and stdout.
pub struct PipeTransport<R: Read, W: Write> {
    reader: io::BufReader<R>,
    writer: io::BufWriter<W>,
}

impl<R: Read, W: Write> PipeTransport<R, W> {
    /// Over the given ends.
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader: io::BufReader::with_capacity(1 << 16, reader),
            writer: io::BufWriter::with_capacity(1 << 16, writer),
        }
    }
}

impl<R: Read, W: Write> IpcTransport for PipeTransport<R, W> {
    fn send(&mut self, body: &[u8]) -> io::Result<()> {
        write_frame(&mut self.writer, body)
    }
    fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        // A buffered reader with nothing pending and a closed pipe is a
        // clean end; otherwise a frame must be complete.
        if self.reader.fill_buf()?.is_empty() {
            return Ok(None);
        }
        read_frame(&mut self.reader)
    }
}

/// Frames over in-process channels: the tab loop on a thread, for tests
/// and for the headless driver.
pub struct ChannelTransport {
    /// Frames to the other side.
    pub tx: Sender<Vec<u8>>,
    /// Frames from the other side.
    pub rx: Receiver<Vec<u8>>,
}

impl IpcTransport for ChannelTransport {
    fn send(&mut self, body: &[u8]) -> io::Result<()> {
        self.tx
            .send(body.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "receiver gone"))
    }
    fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        Ok(self.rx.recv().ok())
    }
}

/// A storage entry: key and value (SPEC-5.2.9).
pub type Entry = (Vec<u8>, Vec<u8>);
/// A package resource as the tab loads it: the bytes and the manifest's
/// image size, if any (SPEC-8.1.2).
pub type ResourceBytes = (Vec<u8>, Option<(u32, u32)>);

/// A text style as the tab process registered it, so the main process
/// can register the same one under the same id (SPEC-9.1.2: the main
/// process rasterizes what the tab measured).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyleSpec {
    /// `vy_abi::font` role.
    pub font: i32,
    /// Size in logical pixels.
    pub size_px: f32,
    /// Requested weight, clamped to 1..=1000.
    pub weight: i32,
    /// `0xRRGGBBAA`.
    pub color: u32,
}

/// Everything a tab process needs to load a page.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadRequest {
    /// Module bytes.
    pub wasm: Vec<u8>,
    /// Its SHA-256 as the manifest states it (SPEC-9.4.1).
    pub sha256: [u8; 32],
    /// `memory_max` in pages (SPEC-4.2.4).
    pub memory_max_pages: u32,
    /// Granted capability ids (`vy_abi::cap`), one bit each: bit `id - 1`.
    pub capabilities: u8,
    /// Package resources by id (SPEC-4.5.1), with the manifest's image size.
    pub resources: Vec<ResourceBytes>,
    /// Storage the origin saved earlier (SPEC-5.2.9).
    pub storage: Vec<Entry>,
    /// Storage quota in bytes (SPEC-9.3.1).
    pub storage_quota_bytes: u64,
    /// The manifest's `net` origins (SPEC-5.2.8).
    pub net_origins: Vec<String>,
    /// The random source (SPEC-5.2.11): keyed from the OS in the client,
    /// the fixed seed for screenshots.
    pub random: RandomSource,
    /// Logical page size and scale for `vy_init` (SPEC-5.1.1).
    pub width: u32,
    /// Logical height.
    pub height: u32,
    /// Scale (SPEC-6.3.1).
    pub scale: f32,
    /// The clock at load (SPEC-5.2.11).
    pub clock_ms: i64,
    /// The shell's clipboard, if it has one (SPEC-5.2.12).
    pub clipboard: Option<Vec<u8>>,
}

/// Messages from the main process to a tab process.
#[derive(Clone, Debug, PartialEq)]
pub enum ToTab {
    /// Load, verify and `vy_init` a page; answered with [`ToShell::Loaded`].
    Load(Box<LoadRequest>),
    /// Run a frame if the module wants one (SPEC-6.2.1); always answered,
    /// with [`ToShell::Frame`] or [`ToShell::Idle`].
    Tick {
        /// The clock (SPEC-5.2.11).
        clock_ms: i64,
    },
    /// An input or host event (SPEC-5.1.3, SPEC-A.2.1).
    Event {
        /// Kind.
        kind: i32,
        /// Argument a.
        a: i32,
        /// Argument b.
        b: i32,
        /// Argument c.
        c: i32,
        /// Argument d.
        d: i32,
        /// The clock when it happened.
        clock_ms: i64,
    },
    /// Committed text (`TEXT_INPUT` through `vy_alloc`, SPEC-5.1.6).
    TextInput {
        /// The text.
        text: String,
        /// The clock when it happened.
        clock_ms: i64,
    },
    /// `vy_resize` (SPEC-5.1.4).
    Resize {
        /// Logical width.
        width: u32,
        /// Logical height.
        height: u32,
        /// Scale.
        scale: f32,
    },
    /// Hide (`vy_suspend`, then `VISIBILITY` 0) or show (`VISIBILITY` 1)
    /// the tab (SPEC-5.1.5, SPEC-6.2.1).
    Visibility {
        /// True when shown.
        visible: bool,
        /// The clock.
        clock_ms: i64,
    },
    /// Grant or revoke a capability (SPEC-9.2.1).
    Grant {
        /// `vy_abi::cap` id.
        cap: i32,
        /// True to grant.
        granted: bool,
    },
    /// The response to a [`ToShell::FetchRequest`] (SPEC-5.2.7).
    FetchResponse {
        /// The request.
        id: i32,
        /// HTTP status, or a negative code from appendix A.
        status: i32,
        /// The whole body.
        body: Vec<u8>,
    },
    /// `vy_suspend` now (the shell is about to persist); answered with
    /// [`ToShell::Storage`].
    Suspend,
    /// Leave: `vy_suspend`, [`ToShell::Storage`], then the process ends.
    Shutdown,
    /// The shell's clipboard changed (SPEC-5.2.12).
    Clipboard {
        /// Its bytes.
        bytes: Vec<u8>,
    },
    /// How the main process's rasterization of the last frame went
    /// (SPEC-5.3.3): a truncated list is a budget breach the module hears
    /// about, as `Runtime::raster_report` delivers it.
    Raster {
        /// True when the budget ran out before the list ended.
        truncated: bool,
        /// Ops drawn.
        ops_done: u32,
        /// Units charged.
        units: u64,
        /// The frame's budget.
        budget: u64,
    },
    /// The next [`ToTab::Event`] is a copy gesture the user made in the
    /// page (Ctrl+C or Ctrl+X): a `clipboard_write` while it is handled
    /// succeeds without the `clipboard` capability (SPEC-8.2.4). One-shot:
    /// it arms exactly the event that follows it.
    CopyGesture,
    /// The end of a recording the user stopped (SPEC-11.4.1, devtools):
    /// the main process writes it last to the file instead of sending it,
    /// and a tab that reads it in a replay does nothing.
    StopRecording,
}

/// Messages from a tab process to the main process.
#[derive(Clone, Debug, PartialEq)]
pub enum ToShell {
    /// The answer to [`ToTab::Load`].
    Loaded {
        /// True when the page runs; false with the reason in `error`.
        ok: bool,
        /// Why loading failed (SPEC-9.4.1, SPEC-4.2.*, `vy_init`).
        error: String,
        /// The tab process's id (SPEC-9.1.1).
        pid: u32,
    },
    /// A display list from `vy_frame` (SPEC-5.1.2); the answer to a tick
    /// the module wanted a frame for.
    Frame {
        /// The display list bytes (appendix C).
        display_list: Vec<u8>,
        /// Styles registered since the last frame sent, in id order.
        styles: Vec<StyleSpec>,
        /// True when the module already asked for another frame.
        wants_more: bool,
        /// True when the frame breached its budget (SPEC-5.3.3).
        over_budget: bool,
    },
    /// The answer to a tick without a frame.
    Idle,
    /// A resource the tab decoded for `IMAGE` (SPEC-6.5.1); sent once,
    /// before the first frame that draws it.
    Image {
        /// Resource id.
        id: u16,
        /// The pixels.
        image: RgbaImage,
    },
    /// The links of the last frame (SPEC-5.2.10); sent when they change.
    Links(Vec<Link>),
    /// The semantic tree of the last frame (SPEC-10.1.1); sent when it changes.
    Semantic(Vec<SemNode>),
    /// A log line from the module or the host about it.
    Log {
        /// True for the module's own `log` calls.
        module: bool,
        /// `vy_abi::level`.
        level: i32,
        /// Text.
        message: String,
    },
    /// The module called `navigate` (SPEC-5.2.10).
    Navigate {
        /// `vy://` or `https://`.
        url: String,
        /// Replace the history entry.
        replace: bool,
    },
    /// The module called `fetch_begin`; the main process performs it (SPEC-5.2.7).
    FetchRequest {
        /// Request id.
        id: i32,
        /// `vy_abi::method`.
        method: i32,
        /// The URL.
        url: String,
        /// Request body.
        body: Vec<u8>,
    },
    /// The page's storage (SPEC-5.2.9), after a suspend or at shutdown.
    Storage(Vec<Entry>),
    /// What the tab process's lockdown applied and what it could not
    /// (SPEC-9.1.3), sent once, just before its first [`ToTab::Load`] is
    /// handled: before any page code has run in the process.
    Sandbox(crate::sandbox::Report),
    /// The module wrote the clipboard (SPEC-5.2.12).
    Clipboard(Vec<u8>),
    /// The IME composition rectangle changed (`ime_begin`/`ime_end`, SPEC-5.2.6).
    Ime(Option<(f32, f32, f32, f32)>),
    /// The semantic node the host holds keyboard focus on changed
    /// (SPEC-10.3.1): focus is the runtime's, in the tab process; the main
    /// process mirrors it for the chrome's own decisions (Ctrl+A in a
    /// field) and the platform's accessibility tree (SPEC-10.2.1).
    Focus(Option<i32>),
    /// The page stopped for good: a trap, `exit`, the hard fuel limit or a
    /// forced suspend (SPEC-5.3.3). The process stays until told to leave.
    Stopped {
        /// What happened.
        reason: String,
    },
}

const T_LOAD: u8 = 1;
const T_TICK: u8 = 2;
const T_EVENT: u8 = 3;
const T_TEXT: u8 = 4;
const T_RESIZE: u8 = 5;
const T_VISIBILITY: u8 = 6;
const T_GRANT: u8 = 7;
const T_FETCH_RESPONSE: u8 = 8;
const T_SUSPEND: u8 = 9;
const T_SHUTDOWN: u8 = 10;
const T_CLIPBOARD: u8 = 11;
const T_RASTER: u8 = 12;
const T_COPY_GESTURE: u8 = 13;
const T_STOP_RECORDING: u8 = 14;

const S_LOADED: u8 = 64;
const S_FRAME: u8 = 65;
const S_IDLE: u8 = 66;
const S_IMAGE: u8 = 67;
const S_LINKS: u8 = 68;
const S_SEMANTIC: u8 = 69;
const S_LOG: u8 = 70;
const S_NAVIGATE: u8 = 71;
const S_FETCH_REQUEST: u8 = 72;
const S_STORAGE: u8 = 73;
const S_CLIPBOARD: u8 = 74;
const S_IME: u8 = 75;
const S_STOPPED: u8 = 76;
const S_FOCUS: u8 = 77;
const S_SANDBOX: u8 = 78;

fn put_random(w: &mut Writer, r: RandomSource) {
    match r {
        RandomSource::Xorshift(seed) => {
            w.u8(0);
            w.u64(seed);
        }
        RandomSource::ChaCha20(key) => {
            w.u8(1);
            w.0.extend_from_slice(&key);
        }
    }
}

fn get_random(r: &mut Reader<'_>) -> Result<RandomSource, DecodeError> {
    match r.u8()? {
        0 => Ok(RandomSource::Xorshift(r.u64()?)),
        1 => {
            let b = r.take(32)?;
            let mut key = [0u8; 32];
            key.copy_from_slice(b);
            Ok(RandomSource::ChaCha20(key))
        }
        _ => Err(DecodeError::Value("random source")),
    }
}

fn put_lines(w: &mut Writer, lines: &[String]) {
    w.u32(lines.len() as u32);
    for l in lines {
        w.str(l);
    }
}

/// A sandbox report's list: at most [`MAX_SANDBOX_LINES`] lines of at
/// most [`MAX_SANDBOX_LINE_BYTES`].
fn get_lines(r: &mut Reader<'_>, what: &'static str) -> Result<Vec<String>, DecodeError> {
    let n = r.count(what)?;
    if n > MAX_SANDBOX_LINES {
        return Err(DecodeError::Value(what));
    }
    (0..n)
        .map(|_| Ok(r.str_max(MAX_SANDBOX_LINE_BYTES, what)?.to_string()))
        .collect()
}

fn put_entries(w: &mut Writer, entries: &[Entry]) {
    w.u32(entries.len() as u32);
    for (k, v) in entries {
        w.bytes(k);
        w.bytes(v);
    }
}

/// Storage entries within the runtime's caps (SPEC-9.3.2: key 256 bytes,
/// value 1 MB) and the quota's ceiling in all (SPEC-5.2.9); more is not
/// a page's storage, whichever side sent it.
fn get_entries(r: &mut Reader<'_>) -> Result<Vec<Entry>, DecodeError> {
    let n = r.count("entries")?;
    let mut out = Vec::with_capacity(n.min(1024));
    let mut total = 0usize;
    for _ in 0..n {
        let k = r
            .bytes_max(limits::MAX_STATE_KEY_BYTES as usize, "storage key")?
            .to_vec();
        let v = r
            .bytes_max(limits::MAX_STATE_VALUE_BYTES as usize, "storage value")?
            .to_vec();
        total += k.len() + v.len();
        if total > MAX_STORAGE_BYTES {
            return Err(DecodeError::Value("storage size"));
        }
        out.push((k, v));
    }
    Ok(out)
}

impl ToTab {
    /// The frame body: tag and payload.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Load(l) => {
                let mut w = Writer::new(T_LOAD);
                w.bytes(&l.wasm);
                w.0.extend_from_slice(&l.sha256);
                w.u32(l.memory_max_pages);
                w.u8(l.capabilities);
                w.u32(l.resources.len() as u32);
                for (bytes, pixels) in &l.resources {
                    w.bytes(bytes);
                    match pixels {
                        None => w.bool(false),
                        Some((pw, ph)) => {
                            w.bool(true);
                            w.u32(*pw);
                            w.u32(*ph);
                        }
                    }
                }
                put_entries(&mut w, &l.storage);
                w.u64(l.storage_quota_bytes);
                w.u32(l.net_origins.len() as u32);
                for o in &l.net_origins {
                    w.str(o);
                }
                put_random(&mut w, l.random);
                w.u32(l.width);
                w.u32(l.height);
                w.f32(l.scale);
                w.i64(l.clock_ms);
                w.opt_bytes(l.clipboard.as_deref());
                w.finish()
            }
            Self::Tick { clock_ms } => {
                let mut w = Writer::new(T_TICK);
                w.i64(*clock_ms);
                w.finish()
            }
            Self::Event {
                kind,
                a,
                b,
                c,
                d,
                clock_ms,
            } => {
                let mut w = Writer::new(T_EVENT);
                for v in [kind, a, b, c, d] {
                    w.i32(*v);
                }
                w.i64(*clock_ms);
                w.finish()
            }
            Self::TextInput { text, clock_ms } => {
                let mut w = Writer::new(T_TEXT);
                w.str(text);
                w.i64(*clock_ms);
                w.finish()
            }
            Self::Resize {
                width,
                height,
                scale,
            } => {
                let mut w = Writer::new(T_RESIZE);
                w.u32(*width);
                w.u32(*height);
                w.f32(*scale);
                w.finish()
            }
            Self::Visibility { visible, clock_ms } => {
                let mut w = Writer::new(T_VISIBILITY);
                w.bool(*visible);
                w.i64(*clock_ms);
                w.finish()
            }
            Self::Grant { cap, granted } => {
                let mut w = Writer::new(T_GRANT);
                w.i32(*cap);
                w.bool(*granted);
                w.finish()
            }
            Self::FetchResponse { id, status, body } => {
                let mut w = Writer::new(T_FETCH_RESPONSE);
                w.i32(*id);
                w.i32(*status);
                w.bytes(body);
                w.finish()
            }
            Self::Suspend => Writer::new(T_SUSPEND).finish(),
            Self::Shutdown => Writer::new(T_SHUTDOWN).finish(),
            Self::Clipboard { bytes } => {
                let mut w = Writer::new(T_CLIPBOARD);
                w.bytes(bytes);
                w.finish()
            }
            Self::Raster {
                truncated,
                ops_done,
                units,
                budget,
            } => {
                let mut w = Writer::new(T_RASTER);
                w.bool(*truncated);
                w.u32(*ops_done);
                w.u64(*units);
                w.u64(*budget);
                w.finish()
            }
            Self::CopyGesture => Writer::new(T_COPY_GESTURE).finish(),
            Self::StopRecording => Writer::new(T_STOP_RECORDING).finish(),
        }
    }

    /// A message from a frame body.
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        let (&tag, payload) = body.split_first().ok_or(DecodeError::Short)?;
        let mut r = Reader::new(payload);
        let msg = match tag {
            T_LOAD => {
                let wasm = r.bytes()?.to_vec();
                let mut sha256 = [0u8; 32];
                sha256.copy_from_slice(r.take(32)?);
                let memory_max_pages = r.u32()?;
                let capabilities = r.u8()?;
                let n = r.count("resources")?;
                let mut resources = Vec::with_capacity(n.min(256));
                for _ in 0..n {
                    let bytes = r.bytes()?.to_vec();
                    let pixels = if r.bool()? {
                        Some((r.u32()?, r.u32()?))
                    } else {
                        None
                    };
                    resources.push((bytes, pixels));
                }
                let storage = get_entries(&mut r)?;
                let storage_quota_bytes = r.u64()?;
                let n = r.count("origins")?;
                let mut net_origins = Vec::with_capacity(n.min(64));
                for _ in 0..n {
                    net_origins.push(r.str()?.to_string());
                }
                let random = get_random(&mut r)?;
                let width = r.u32()?;
                let height = r.u32()?;
                let scale = r.f32()?;
                let clock_ms = r.i64()?;
                let clipboard = r.opt_bytes()?.map(<[u8]>::to_vec);
                Self::Load(Box::new(LoadRequest {
                    wasm,
                    sha256,
                    memory_max_pages,
                    capabilities,
                    resources,
                    storage,
                    storage_quota_bytes,
                    net_origins,
                    random,
                    width,
                    height,
                    scale,
                    clock_ms,
                    clipboard,
                }))
            }
            T_TICK => Self::Tick { clock_ms: r.i64()? },
            T_EVENT => Self::Event {
                kind: r.i32()?,
                a: r.i32()?,
                b: r.i32()?,
                c: r.i32()?,
                d: r.i32()?,
                clock_ms: r.i64()?,
            },
            T_TEXT => Self::TextInput {
                text: r.str()?.to_string(),
                clock_ms: r.i64()?,
            },
            T_RESIZE => Self::Resize {
                width: r.u32()?,
                height: r.u32()?,
                scale: r.f32()?,
            },
            T_VISIBILITY => Self::Visibility {
                visible: r.bool()?,
                clock_ms: r.i64()?,
            },
            T_GRANT => Self::Grant {
                cap: r.i32()?,
                granted: r.bool()?,
            },
            T_FETCH_RESPONSE => Self::FetchResponse {
                id: r.i32()?,
                status: r.i32()?,
                body: r.bytes()?.to_vec(),
            },
            T_SUSPEND => Self::Suspend,
            T_SHUTDOWN => Self::Shutdown,
            T_CLIPBOARD => Self::Clipboard {
                bytes: r.bytes()?.to_vec(),
            },
            T_RASTER => Self::Raster {
                truncated: r.bool()?,
                ops_done: r.u32()?,
                units: r.u64()?,
                budget: r.u64()?,
            },
            T_COPY_GESTURE => Self::CopyGesture,
            T_STOP_RECORDING => Self::StopRecording,
            other => return Err(DecodeError::Tag(other)),
        };
        r.finish()?;
        Ok(msg)
    }
}

impl ToShell {
    /// The frame body: tag and payload.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Self::Loaded { ok, error, pid } => {
                let mut w = Writer::new(S_LOADED);
                w.bool(*ok);
                w.str(error);
                w.u32(*pid);
                w.finish()
            }
            Self::Frame {
                display_list,
                styles,
                wants_more,
                over_budget,
            } => {
                let mut w = Writer::new(S_FRAME);
                w.bytes(display_list);
                w.u32(styles.len() as u32);
                for s in styles {
                    w.i32(s.font);
                    w.f32(s.size_px);
                    w.i32(s.weight);
                    w.u32(s.color);
                }
                w.bool(*wants_more);
                w.bool(*over_budget);
                w.finish()
            }
            Self::Idle => Writer::new(S_IDLE).finish(),
            Self::Image { id, image } => {
                let mut w = Writer::new(S_IMAGE);
                w.u16(*id);
                w.u32(image.width);
                w.u32(image.height);
                w.bytes(&image.data);
                w.finish()
            }
            Self::Links(links) => {
                let mut w = Writer::new(S_LINKS);
                w.u32(links.len() as u32);
                for l in links {
                    w.i32(l.id);
                    for v in [l.x, l.y, l.w, l.h] {
                        w.f32(v);
                    }
                    w.str(&l.url);
                }
                w.finish()
            }
            Self::Semantic(nodes) => {
                let mut w = Writer::new(S_SEMANTIC);
                w.u32(nodes.len() as u32);
                for n in nodes {
                    w.i32(n.id);
                    w.i32(n.parent_id);
                    w.i32(n.role);
                    for v in [n.x, n.y, n.w, n.h] {
                        w.f32(v);
                    }
                    w.str(&n.text);
                }
                w.finish()
            }
            Self::Log {
                module,
                level,
                message,
            } => {
                let mut w = Writer::new(S_LOG);
                w.bool(*module);
                w.i32(*level);
                w.str(message);
                w.finish()
            }
            Self::Navigate { url, replace } => {
                let mut w = Writer::new(S_NAVIGATE);
                w.str(url);
                w.bool(*replace);
                w.finish()
            }
            Self::FetchRequest {
                id,
                method,
                url,
                body,
            } => {
                let mut w = Writer::new(S_FETCH_REQUEST);
                w.i32(*id);
                w.i32(*method);
                w.str(url);
                w.bytes(body);
                w.finish()
            }
            Self::Storage(entries) => {
                let mut w = Writer::new(S_STORAGE);
                put_entries(&mut w, entries);
                w.finish()
            }
            Self::Sandbox(report) => {
                let mut w = Writer::new(S_SANDBOX);
                put_lines(&mut w, &report.layers);
                put_lines(&mut w, &report.warnings);
                w.finish()
            }
            Self::Clipboard(bytes) => {
                let mut w = Writer::new(S_CLIPBOARD);
                w.bytes(bytes);
                w.finish()
            }
            Self::Ime(rect) => {
                let mut w = Writer::new(S_IME);
                match rect {
                    None => w.bool(false),
                    Some((x, y, rw, rh)) => {
                        w.bool(true);
                        for v in [x, y, rw, rh] {
                            w.f32(*v);
                        }
                    }
                }
                w.finish()
            }
            Self::Focus(node) => {
                let mut w = Writer::new(S_FOCUS);
                match node {
                    None => w.bool(false),
                    Some(id) => {
                        w.bool(true);
                        w.i32(*id);
                    }
                }
                w.finish()
            }
            Self::Stopped { reason } => {
                let mut w = Writer::new(S_STOPPED);
                w.str(reason);
                w.finish()
            }
        }
    }

    /// A message from a frame body. The main process decodes what a tab
    /// process sent, and a tab process runs page code, so every length
    /// is held to the runtime's caps (SPEC-9.3.2) here as well: a message
    /// over them cannot have come from a runtime that kept them, and is
    /// refused before it is kept, drawn or acted on.
    pub fn decode(body: &[u8]) -> Result<Self, DecodeError> {
        let (&tag, payload) = body.split_first().ok_or(DecodeError::Short)?;
        let mut r = Reader::new(payload);
        let msg = match tag {
            S_LOADED => Self::Loaded {
                ok: r.bool()?,
                error: r.str_max(MAX_MESSAGE_TEXT_BYTES, "error")?.to_string(),
                pid: r.u32()?,
            },
            S_FRAME => {
                let display_list = r
                    .bytes_max(MAX_DISPLAY_LIST_BYTES, "display list")?
                    .to_vec();
                let n = r.count("styles")?;
                if n > vy_render::text::MAX_STYLES {
                    return Err(DecodeError::Value("styles"));
                }
                let mut styles = Vec::with_capacity(n.min(1024));
                for _ in 0..n {
                    styles.push(StyleSpec {
                        font: r.i32()?,
                        size_px: r.f32()?,
                        weight: r.i32()?,
                        color: r.u32()?,
                    });
                }
                Self::Frame {
                    display_list,
                    styles,
                    wants_more: r.bool()?,
                    over_budget: r.bool()?,
                }
            }
            S_IDLE => Self::Idle,
            S_IMAGE => {
                let id = r.u16()?;
                let width = r.u32()?;
                let height = r.u32()?;
                if width > limits::MAX_IMAGE_SIDE_PX || height > limits::MAX_IMAGE_SIDE_PX {
                    return Err(DecodeError::Value("image side"));
                }
                let data = r.bytes()?;
                let expect = (width as usize)
                    .checked_mul(height as usize)
                    .and_then(|p| p.checked_mul(4))
                    .ok_or(DecodeError::Value("image size"))?;
                if data.len() != expect {
                    return Err(DecodeError::Value("image size"));
                }
                Self::Image {
                    id,
                    image: RgbaImage {
                        width,
                        height,
                        data: data.to_vec(),
                    },
                }
            }
            S_LINKS => {
                let n = r.count("links")?;
                if n > limits::MAX_LINKS_PER_FRAME as usize {
                    return Err(DecodeError::Value("links"));
                }
                let mut links = Vec::with_capacity(n);
                for _ in 0..n {
                    links.push(Link {
                        id: r.i32()?,
                        x: r.f32()?,
                        y: r.f32()?,
                        w: r.f32()?,
                        h: r.f32()?,
                        url: r
                            .str_max(limits::MAX_URL_BYTES as usize, "link url")?
                            .to_string(),
                    });
                }
                Self::Links(links)
            }
            S_SEMANTIC => {
                let n = r.count("nodes")?;
                if n > limits::MAX_SEM_NODES_PER_FRAME as usize {
                    return Err(DecodeError::Value("nodes"));
                }
                let mut nodes = Vec::with_capacity(n.min(4096));
                for _ in 0..n {
                    nodes.push(SemNode {
                        id: r.i32()?,
                        parent_id: r.i32()?,
                        role: r.i32()?,
                        x: r.f32()?,
                        y: r.f32()?,
                        w: r.f32()?,
                        h: r.f32()?,
                        text: r
                            .str_max(limits::MAX_SEM_TEXT_BYTES as usize, "node text")?
                            .to_string(),
                    });
                }
                Self::Semantic(nodes)
            }
            S_LOG => Self::Log {
                module: r.bool()?,
                level: r.i32()?,
                message: r.str_max(MAX_MESSAGE_TEXT_BYTES, "log")?.to_string(),
            },
            S_NAVIGATE => Self::Navigate {
                url: r
                    .str_max(limits::MAX_URL_BYTES as usize, "navigate url")?
                    .to_string(),
                replace: r.bool()?,
            },
            S_FETCH_REQUEST => Self::FetchRequest {
                id: r.i32()?,
                method: r.i32()?,
                url: r
                    .str_max(limits::MAX_URL_BYTES as usize, "fetch url")?
                    .to_string(),
                body: r
                    .bytes_max(limits::MAX_FETCH_BODY_BYTES as usize, "fetch body")?
                    .to_vec(),
            },
            S_STORAGE => Self::Storage(get_entries(&mut r)?),
            S_SANDBOX => Self::Sandbox(crate::sandbox::Report {
                layers: get_lines(&mut r, "sandbox layers")?,
                warnings: get_lines(&mut r, "sandbox warnings")?,
            }),
            S_CLIPBOARD => Self::Clipboard(
                r.bytes_max(limits::MAX_CLIPBOARD_BYTES as usize, "clipboard")?
                    .to_vec(),
            ),
            S_IME => Self::Ime(if r.bool()? {
                Some((r.f32()?, r.f32()?, r.f32()?, r.f32()?))
            } else {
                None
            }),
            S_FOCUS => Self::Focus(if r.bool()? { Some(r.i32()?) } else { None }),
            S_STOPPED => Self::Stopped {
                reason: r.str_max(MAX_MESSAGE_TEXT_BYTES, "reason")?.to_string(),
            },
            other => return Err(DecodeError::Tag(other)),
        };
        r.finish()?;
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn load() -> LoadRequest {
        LoadRequest {
            wasm: vec![0, 0x61, 0x73, 0x6d],
            sha256: [7; 32],
            memory_max_pages: 64,
            capabilities: 0b10_0011,
            resources: vec![(vec![1, 2, 3], Some((320, 200))), (vec![], None)],
            storage: vec![(b"k".to_vec(), b"v".to_vec())],
            storage_quota_bytes: 1 << 20,
            net_origins: vec!["https://chat.example".into()],
            random: RandomSource::ChaCha20([9; 32]),
            width: 800,
            height: 560,
            scale: 1.5,
            clock_ms: 123_456_789_012,
            clipboard: Some(b"urklipp".to_vec()),
        }
    }

    fn to_tab_messages() -> Vec<ToTab> {
        vec![
            ToTab::Load(Box::new(load())),
            ToTab::Load(Box::new(LoadRequest {
                random: RandomSource::Xorshift(42),
                clipboard: None,
                resources: vec![],
                storage: vec![],
                net_origins: vec![],
                ..load()
            })),
            ToTab::Tick { clock_ms: -1 },
            ToTab::Event {
                kind: 2,
                a: 10_000,
                b: -5,
                c: 1,
                d: 8,
                clock_ms: 16,
            },
            ToTab::TextInput {
                text: "Hej från Vy – ﬁ".into(),
                clock_ms: 32,
            },
            ToTab::Resize {
                width: 1,
                height: u32::MAX,
                scale: 0.75,
            },
            ToTab::Visibility {
                visible: false,
                clock_ms: 0,
            },
            ToTab::Grant {
                cap: 3,
                granted: true,
            },
            ToTab::FetchResponse {
                id: 7,
                status: -31,
                body: vec![0; 70_000],
            },
            ToTab::Suspend,
            ToTab::Shutdown,
            ToTab::Clipboard { bytes: vec![] },
            ToTab::Raster {
                truncated: true,
                ops_done: 65,
                units: 1 << 40,
                budget: 64 << 20,
            },
            ToTab::CopyGesture,
            ToTab::StopRecording,
        ]
    }

    fn to_shell_messages() -> Vec<ToShell> {
        vec![
            ToShell::Loaded {
                ok: false,
                error: "module hash mismatch".into(),
                pid: 4242,
            },
            ToShell::Frame {
                display_list: vec![0xFF],
                styles: vec![StyleSpec {
                    font: -1,
                    size_px: 16.0,
                    weight: 700,
                    color: 0x1122_33FF,
                }],
                wants_more: true,
                over_budget: false,
            },
            ToShell::Idle,
            ToShell::Image {
                id: 3,
                image: RgbaImage {
                    width: 2,
                    height: 1,
                    data: vec![1, 2, 3, 4, 5, 6, 7, 8],
                },
            },
            ToShell::Links(vec![Link {
                id: 1,
                x: 1.5,
                y: -2.0,
                w: 100.0,
                h: 20.0,
                url: "vy://docs.example/a?b".into(),
            }]),
            ToShell::Semantic(vec![SemNode {
                id: -3,
                parent_id: 0,
                role: 2,
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
                text: "Rubrik \"citat\"".into(),
            }]),
            ToShell::Log {
                module: true,
                level: 3,
                message: "fel".into(),
            },
            ToShell::Navigate {
                url: "https://example.test/".into(),
                replace: true,
            },
            ToShell::FetchRequest {
                id: 1,
                method: 1,
                url: "https://chat.example/api".into(),
                body: b"{}".to_vec(),
            },
            ToShell::Storage(vec![(vec![], vec![0; 300])]),
            ToShell::Clipboard(b"x".to_vec()),
            ToShell::Ime(Some((1.0, 2.0, 3.0, 4.0))),
            ToShell::Ime(None),
            ToShell::Focus(Some(-10)),
            ToShell::Focus(None),
            ToShell::Stopped {
                reason: "module trapped: unreachable".into(),
            },
            ToShell::Sandbox(crate::sandbox::Report {
                layers: vec!["no_new_privs".into(), "seccomp allowlist".into()],
                warnings: vec!["landlock: Operation not supported (os error 95)".into()],
            }),
            ToShell::Sandbox(crate::sandbox::Report::default()),
        ]
    }

    /// SPEC-11.2.1: every message round-trips through its frame body, the
    /// two directions have disjoint tags, and the frame framing
    /// (`u32 len`, tag, payload) is what goes over a pipe and comes back.
    #[test]
    fn spec_11_2_1_messages_round_trip() {
        let mut tab_tags = std::collections::HashSet::new();
        let mut shell_tags = std::collections::HashSet::new();
        for m in to_tab_messages() {
            let body = m.encode();
            assert_eq!(ToTab::decode(&body).unwrap(), m, "{m:?}");
            assert!(ToShell::decode(&body).is_err(), "tags are disjoint: {m:?}");
            tab_tags.insert(body[0]);
        }
        for m in to_shell_messages() {
            let body = m.encode();
            assert_eq!(ToShell::decode(&body).unwrap(), m, "{m:?}");
            assert!(ToTab::decode(&body).is_err(), "tags are disjoint: {m:?}");
            shell_tags.insert(body[0]);
        }
        assert_eq!(tab_tags.len(), 14, "every ToTab variant is covered");
        assert_eq!(shell_tags.len(), 15, "every ToShell variant is covered");
        assert!(tab_tags.is_disjoint(&shell_tags));
        // Over a byte stream: lengths first, little-endian.
        let mut pipe = Vec::new();
        let msgs = to_shell_messages();
        for m in &msgs {
            write_frame(&mut pipe, &m.encode()).unwrap();
        }
        let first = ToShell::Loaded {
            ok: false,
            error: "module hash mismatch".into(),
            pid: 4242,
        }
        .encode();
        assert_eq!(pipe[..4], (first.len() as u32).to_le_bytes());
        assert_eq!(pipe[4], S_LOADED);
        let mut cursor = std::io::Cursor::new(pipe);
        let mut back = Vec::new();
        while let Some(body) = read_frame(&mut cursor).unwrap() {
            back.push(ToShell::decode(&body).unwrap());
        }
        assert_eq!(back, msgs);
    }

    /// SPEC-9.3.1, SPEC-11.2.2: a truncated, oversized, trailing or
    /// mis-tagged frame is an error, never a panic or a guess; a length
    /// over the cap is refused before anything is allocated; an image
    /// whose bytes do not match its size is refused.
    #[test]
    fn spec_11_2_2_bad_frames_are_errors() {
        for m in to_tab_messages() {
            let body = m.encode();
            for cut in 1..body.len() {
                assert!(ToTab::decode(&body[..cut]).is_err(), "{m:?} cut at {cut}");
            }
            let mut extra = body.clone();
            extra.push(0);
            assert_eq!(ToTab::decode(&extra), Err(DecodeError::Trailing(1)));
        }
        for m in to_shell_messages() {
            let body = m.encode();
            for cut in 1..body.len() {
                assert!(ToShell::decode(&body[..cut]).is_err(), "{m:?} cut at {cut}");
            }
        }
        assert_eq!(ToTab::decode(&[]), Err(DecodeError::Short));
        assert_eq!(ToTab::decode(&[200]), Err(DecodeError::Tag(200)));
        assert_eq!(
            ToShell::decode(&[T_TICK, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(DecodeError::Tag(T_TICK))
        );
        // A bool byte that is neither 0 nor 1.
        let mut grant = ToTab::Grant {
            cap: 1,
            granted: true,
        }
        .encode();
        *grant.last_mut().unwrap() = 2;
        assert_eq!(ToTab::decode(&grant), Err(DecodeError::Value("bool")));
        // Counts that promise more elements than there are bytes.
        let mut links = Writer::new(S_LINKS);
        links.u32(u32::MAX);
        assert_eq!(
            ToShell::decode(&links.finish()),
            Err(DecodeError::Value("links"))
        );
        // Image bytes must match width × height × 4.
        let mut img = Writer::new(S_IMAGE);
        img.u16(1);
        img.u32(2);
        img.u32(2);
        img.bytes(&[0; 15]);
        assert_eq!(
            ToShell::decode(&img.finish()),
            Err(DecodeError::Value("image size"))
        );
        let mut img = Writer::new(S_IMAGE);
        img.u16(1);
        img.u32(u32::MAX);
        img.u32(u32::MAX);
        img.bytes(&[]);
        assert_eq!(
            ToShell::decode(&img.finish()),
            Err(DecodeError::Value("image side")),
            "a side over SPEC-6.5.1's 8192 is refused before the size is even multiplied"
        );
        // Frame lengths: zero, over the cap, and cut mid-frame.
        let mut zero = std::io::Cursor::new(0u32.to_le_bytes().to_vec());
        assert!(read_frame(&mut zero).is_err());
        let mut huge = std::io::Cursor::new((MAX_FRAME_BYTES + 1).to_le_bytes().to_vec());
        assert!(read_frame(&mut huge).is_err());
        let mut cut = std::io::Cursor::new(vec![5, 0, 0, 0, S_IDLE, 1]);
        assert!(read_frame(&mut cut).is_err());
        let mut empty = std::io::Cursor::new(Vec::new());
        assert_eq!(read_frame(&mut empty).unwrap(), None);
        let mut partial_len = std::io::Cursor::new(vec![1, 0]);
        assert_eq!(read_frame(&mut partial_len).unwrap(), None);
        let mut out = Vec::new();
        assert!(write_frame(&mut out, &[]).is_err(), "a frame has a tag");
        assert!(write_frame(&mut out, &vec![0; MAX_FRAME_BYTES as usize + 1]).is_err());
    }

    /// SPEC-9.3.2, SPEC-9.3.1: the main process holds a tab's messages to
    /// the runtime's caps before keeping anything: more links, nodes,
    /// text, body, clipboard or storage than a page may produce, or a
    /// longer sandbox report than a lockdown makes, is a refused frame,
    /// not a bigger allocation. The caps are the same constants the
    /// runtime applies, so a conforming tab never hits them.
    #[test]
    fn spec_9_3_2_main_process_holds_tab_messages_to_the_caps() {
        use vy_abi::limits;
        let link = |url: &str| Link {
            id: 1,
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            url: url.into(),
        };
        let node = |text: &str| SemNode {
            id: 1,
            parent_id: 0,
            role: 1,
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            text: text.into(),
        };
        let max_url = "a".repeat(limits::MAX_URL_BYTES as usize);
        let over_url = "a".repeat(limits::MAX_URL_BYTES as usize + 1);
        let max_sem = "s".repeat(limits::MAX_SEM_TEXT_BYTES as usize);
        let over_sem = "s".repeat(limits::MAX_SEM_TEXT_BYTES as usize + 1);
        let ok: Vec<ToShell> = vec![
            ToShell::Links(vec![link(&max_url); limits::MAX_LINKS_PER_FRAME as usize]),
            ToShell::Semantic(vec![node(&max_sem); 1000]),
            ToShell::Log {
                module: false,
                level: 0,
                message: "m".repeat(MAX_MESSAGE_TEXT_BYTES),
            },
            ToShell::Navigate {
                url: max_url.clone(),
                replace: false,
            },
            ToShell::FetchRequest {
                id: 1,
                method: 0,
                url: max_url.clone(),
                body: vec![0; limits::MAX_FETCH_BODY_BYTES as usize],
            },
            ToShell::Clipboard(vec![0; limits::MAX_CLIPBOARD_BYTES as usize]),
            ToShell::Storage(vec![(
                vec![0; limits::MAX_STATE_KEY_BYTES as usize],
                vec![0; limits::MAX_STATE_VALUE_BYTES as usize],
            )]),
            ToShell::Stopped {
                reason: "r".repeat(MAX_MESSAGE_TEXT_BYTES),
            },
            ToShell::Image {
                id: 1,
                image: RgbaImage::new(limits::MAX_IMAGE_SIDE_PX, 1),
            },
            ToShell::Sandbox(crate::sandbox::Report {
                layers: vec!["l".repeat(MAX_SANDBOX_LINE_BYTES); MAX_SANDBOX_LINES],
                warnings: vec!["w".repeat(MAX_SANDBOX_LINE_BYTES); MAX_SANDBOX_LINES],
            }),
        ];
        for m in &ok {
            assert_eq!(&ToShell::decode(&m.encode()).unwrap(), m);
        }
        let over: Vec<(ToShell, &str)> = vec![
            (
                ToShell::Links(vec![
                    link("vy://x/");
                    limits::MAX_LINKS_PER_FRAME as usize + 1
                ]),
                "links",
            ),
            (ToShell::Links(vec![link(&over_url)]), "link url"),
            (
                ToShell::Semantic(vec![node(""); limits::MAX_SEM_NODES_PER_FRAME as usize + 1]),
                "nodes",
            ),
            (ToShell::Semantic(vec![node(&over_sem)]), "node text"),
            (
                ToShell::Log {
                    module: true,
                    level: 0,
                    message: "m".repeat(MAX_MESSAGE_TEXT_BYTES + 1),
                },
                "log",
            ),
            (
                ToShell::Navigate {
                    url: over_url.clone(),
                    replace: false,
                },
                "navigate url",
            ),
            (
                ToShell::FetchRequest {
                    id: 1,
                    method: 0,
                    url: over_url.clone(),
                    body: Vec::new(),
                },
                "fetch url",
            ),
            (
                ToShell::FetchRequest {
                    id: 1,
                    method: 0,
                    url: "https://x/".into(),
                    body: vec![0; limits::MAX_FETCH_BODY_BYTES as usize + 1],
                },
                "fetch body",
            ),
            (
                ToShell::Clipboard(vec![0; limits::MAX_CLIPBOARD_BYTES as usize + 1]),
                "clipboard",
            ),
            (
                ToShell::Storage(vec![(
                    vec![0; limits::MAX_STATE_KEY_BYTES as usize + 1],
                    Vec::new(),
                )]),
                "storage key",
            ),
            (
                ToShell::Storage(vec![(
                    Vec::new(),
                    vec![0; limits::MAX_STATE_VALUE_BYTES as usize + 1],
                )]),
                "storage value",
            ),
            (
                ToShell::Storage(vec![
                    (
                        Vec::new(),
                        vec![0; limits::MAX_STATE_VALUE_BYTES as usize]
                    );
                    MAX_STORAGE_BYTES
                        / limits::MAX_STATE_VALUE_BYTES as usize
                        + 1
                ]),
                "storage size",
            ),
            (
                ToShell::Stopped {
                    reason: "r".repeat(MAX_MESSAGE_TEXT_BYTES + 1),
                },
                "reason",
            ),
            (
                ToShell::Frame {
                    display_list: vec![0; MAX_DISPLAY_LIST_BYTES + 1],
                    styles: Vec::new(),
                    wants_more: false,
                    over_budget: false,
                },
                "display list",
            ),
            (
                ToShell::Frame {
                    display_list: vec![0xFF],
                    styles: vec![
                        StyleSpec {
                            font: -1,
                            size_px: 1.0,
                            weight: 1,
                            color: 0
                        };
                        vy_render::text::MAX_STYLES + 1
                    ],
                    wants_more: false,
                    over_budget: false,
                },
                "styles",
            ),
            (
                ToShell::Image {
                    id: 1,
                    image: RgbaImage::new(limits::MAX_IMAGE_SIDE_PX + 1, 1),
                },
                "image side",
            ),
            (
                ToShell::Sandbox(crate::sandbox::Report {
                    layers: vec!["l".into(); MAX_SANDBOX_LINES + 1],
                    warnings: vec![],
                }),
                "sandbox layers",
            ),
            (
                ToShell::Sandbox(crate::sandbox::Report {
                    layers: vec![],
                    warnings: vec!["w".repeat(MAX_SANDBOX_LINE_BYTES + 1)],
                }),
                "sandbox warnings",
            ),
        ];
        for (m, what) in &over {
            assert_eq!(
                ToShell::decode(&m.encode()),
                Err(DecodeError::Value(what)),
                "{what}"
            );
        }
        // The same storage caps hold for the file the shell reads back.
        let big = crate::storage::encode(&[(
            Vec::new(),
            vec![0; limits::MAX_STATE_VALUE_BYTES as usize + 1],
        )]);
        assert_eq!(crate::storage::decode(&big), None);
    }

    /// SPEC-11.2.1: the same frames travel over a pipe transport and an
    /// in-process channel transport; the loop is written against the
    /// trait, so the transport can be replaced.
    #[test]
    fn spec_11_2_1_transports_carry_the_same_frames() {
        let msgs = to_shell_messages();
        let (to_a, from_b) = mpsc::channel();
        let (to_b, from_a) = mpsc::channel();
        let mut a = ChannelTransport {
            tx: to_a,
            rx: from_a,
        };
        let mut b = ChannelTransport {
            tx: to_b,
            rx: from_b,
        };
        for m in &msgs {
            a.send(&m.encode()).unwrap();
        }
        drop(a);
        let mut got = Vec::new();
        while let Some(body) = b.recv().unwrap() {
            got.push(ToShell::decode(&body).unwrap());
        }
        assert_eq!(got, msgs);

        let mut wire = Vec::new();
        {
            let mut p = PipeTransport::new(std::io::empty(), &mut wire);
            for m in &msgs {
                p.send(&m.encode()).unwrap();
            }
        }
        let mut p = PipeTransport::new(std::io::Cursor::new(wire), std::io::sink());
        let mut got = Vec::new();
        while let Some(body) = p.recv().unwrap() {
            got.push(ToShell::decode(&body).unwrap());
        }
        assert_eq!(got, msgs);
        let mut cut =
            PipeTransport::new(std::io::Cursor::new(vec![9, 0, 0, 0, 1]), std::io::sink());
        assert!(
            cut.recv().is_err(),
            "a frame cut short is an error, not an end"
        );
    }
}
