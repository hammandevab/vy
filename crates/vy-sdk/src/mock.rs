//! A host stand-in for native tests of SDK and page logic.
//!
//! `MockHost` records every import call on the current thread and enforces
//! the same refusals as `vy-runtime` (SPEC-5.3.2, SPEC-6.1.4, SPEC-6.1.6,
//! SPEC-9.3.2), returning the same status codes (SPEC-5.3.1), so a test can
//! check both what a page drew and that the SDK refused before the host had
//! to. Text is "shaped" with a fixed model:
//! every character is `size × 0.6` wide and a line is `size × 1.25` tall,
//! wrapped greedily at word boundaries. Pixels are never produced; that is
//! what `vy-cli render` and the conformance suite are for.

use crate::app::App;
use crate::error::VyError;
use crate::frame::Frame;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;
use vy_abi::{err, fit, limits, ABI_VERSION};

/// Width of one character relative to the font size in the mock's text model.
pub const CHAR_WIDTH_FACTOR: f32 = 0.6;
/// Line height relative to the font size, as in `vy-render`.
pub const LINE_HEIGHT_FACTOR: f32 = 1.25;
/// Most links the host keeps for one frame (SPEC-9.3.2).
pub const MAX_LINKS_PER_FRAME: usize = 4_096;

/// One display-list call as the mock recorded it.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// `dl_begin`.
    Begin,
    /// `dl_end`.
    End,
    /// `dl_rect`.
    Rect {
        /// Left.
        x: f32,
        /// Top.
        y: f32,
        /// Width.
        w: f32,
        /// Height.
        h: f32,
        /// Corner radius.
        radius: f32,
        /// `0xRRGGBBAA`.
        color: u32,
    },
    /// `dl_text`.
    Text {
        /// Left.
        x: f32,
        /// Top.
        y: f32,
        /// Wrap width.
        max_w: f32,
        /// Style id.
        style_id: i32,
        /// The text.
        text: String,
    },
    /// `dl_image`.
    Image {
        /// Left.
        x: f32,
        /// Top.
        y: f32,
        /// Width.
        w: f32,
        /// Height.
        h: f32,
        /// Resource id.
        resource_id: i32,
        /// Fit mode.
        fit: i32,
    },
    /// `dl_path`.
    Path {
        /// Encoded commands (SPEC-C.1.2).
        bytes: Vec<u8>,
        /// `0xRRGGBBAA`.
        color: u32,
        /// 0 = fill.
        stroke_w: f32,
    },
    /// `dl_clip_push`.
    ClipPush {
        /// Left.
        x: f32,
        /// Top.
        y: f32,
        /// Width.
        w: f32,
        /// Height.
        h: f32,
        /// Corner radius.
        radius: f32,
    },
    /// `dl_clip_pop`.
    ClipPop,
}

/// One `sem_node` call as the mock recorded it.
#[derive(Clone, Debug, PartialEq)]
pub struct SemNode {
    /// Module-chosen id.
    pub id: i32,
    /// Parent id, 0 for roots.
    pub parent_id: i32,
    /// Role 1..=24.
    pub role: i32,
    /// Left.
    pub x: f32,
    /// Top.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
    /// Text content or accessible name.
    pub text: String,
}

/// A style registered with `text_style`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// Font resource id.
    pub font: i32,
    /// Size in logical pixels.
    pub size_px: f32,
    /// Weight 1..=1000.
    pub weight: i32,
    /// `0xRRGGBBAA`.
    pub color: u32,
}

/// Which capabilities the mock grants (SPEC-8.2.1); all false by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caps {
    /// `fetch_*`.
    pub net: bool,
    /// `state_*`.
    pub storage: bool,
    /// `clipboard_*`.
    pub clipboard: bool,
    /// `ime_*`.
    pub ime: bool,
}

/// The recording host. Fields are public so tests can inspect and preset them.
#[derive(Debug, Default)]
pub struct MockHost {
    /// Display-list calls that were accepted, in order.
    pub ops: Vec<Op>,
    /// Semantic nodes of the current frame, in order.
    pub sem: Vec<SemNode>,
    /// `(level, message)` from `log`.
    pub logs: Vec<(i32, String)>,
    /// Refusals the host made, as it would log them: `"<import>: <code>"`.
    pub refusals: Vec<String>,
    /// Styles by id − 1.
    pub styles: Vec<Style>,
    /// `request_frame` calls.
    pub frame_requests: u32,
    /// Code passed to `exit`.
    pub exit_code: Option<i32>,
    /// Keys currently down.
    pub keys_down: Vec<i32>,
    /// IME rectangle while composition is active.
    pub ime_rect: Option<(f32, f32, f32, f32)>,
    /// `(id, x, y, w, h, url)` from `link_register` since the last `dl_begin`.
    pub links: Vec<(i32, f32, f32, f32, f32, String)>,
    /// `(url, replace)` from `navigate`.
    pub navigations: Vec<(String, bool)>,
    /// Value `now_ms` returns.
    pub now_ms: i64,
    /// State of the random source (xorshift64*).
    pub rng: u64,
    /// Storage behind `state_*` when `caps.storage`.
    pub state: BTreeMap<Vec<u8>, Vec<u8>>,
    /// Clipboard behind `clipboard_*` when `caps.clipboard`.
    pub clipboard: String,
    /// Granted capabilities.
    pub caps: Caps,
    dl_open: bool,
    op_count: u32,
    clip_depth: u32,
}

std::thread_local! {
    static HOST: RefCell<MockHost> = RefCell::new(MockHost::new());
}

/// Run `f` against the current thread's mock host.
pub fn with<R>(f: impl FnOnce(&mut MockHost) -> R) -> R {
    HOST.with(|h| f(&mut h.borrow_mut()))
}

/// Replace the current thread's mock host with a fresh one.
pub fn reset() {
    with(|h| *h = MockHost::new());
}

/// Take the current thread's mock host, leaving a fresh one.
pub fn take() -> MockHost {
    with(core::mem::take)
}

/// Run one `vy_frame` of `app` against the mock as the export glue would:
/// open the list, let the page draw, close it. Returns the opcode count the
/// glue would report (SPEC-5.1.2), or the page's error, or `DlClip` for a
/// clip left open (SPEC-6.1.4). The only way to get a [`Frame`] outside the
/// glue, and only natively.
pub fn run_frame<A: App>(app: &mut A, dt_ms: i32) -> Result<u32, VyError> {
    let mut f = Frame::begin();
    let drawn = app.frame(&mut f, dt_ms);
    let ended = f.end();
    drawn.and(ended)
}

impl MockHost {
    /// A host with nothing recorded and no capabilities.
    pub fn new() -> Self {
        Self {
            rng: 0x5EED_5EED_0000_0001,
            ..Self::default()
        }
    }

    /// Accepted display-list ops between the last `Begin` and `End`,
    /// excluding both markers.
    pub fn frame_ops(&self) -> &[Op] {
        let start = self
            .ops
            .iter()
            .rposition(|o| *o == Op::Begin)
            .map_or(0, |i| i + 1);
        let end = self.ops[start..]
            .iter()
            .position(|o| *o == Op::End)
            .map_or(self.ops.len(), |i| start + i);
        &self.ops[start..end]
    }

    /// True between `dl_begin` and `dl_end`.
    pub fn dl_open(&self) -> bool {
        self.dl_open
    }

    fn refuse(&mut self, name: &str, code: i32) {
        let n = match code {
            err::E_DENIED => "E_DENIED",
            err::E_ARG => "E_ARG",
            err::E_DL_STATE => "E_DL_STATE",
            err::E_DL_FULL => "E_DL_FULL",
            err::E_DL_CLIP => "E_DL_CLIP",
            err::E_RESOURCE => "E_RESOURCE",
            err::E_STYLE => "E_STYLE",
            _ => "E_UNKNOWN",
        };
        self.refusals.push(alloc::format!("{name}: {n}"));
    }

    fn gate(&mut self, name: &str) -> Result<(), i32> {
        if !self.dl_open {
            self.refuse(name, err::E_DL_STATE);
            return Err(err::E_DL_STATE);
        }
        if self.op_count >= limits::MAX_OPS_PER_FRAME {
            self.refuse(name, err::E_DL_FULL);
            return Err(err::E_DL_FULL);
        }
        Ok(())
    }

    fn style(&self, id: i32) -> Option<Style> {
        usize::try_from(id)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| self.styles.get(i))
            .copied()
    }

    /// Lines of `text` wrapped at `max_w` under the mock model.
    fn lines(size_px: f32, max_w: f32, text: &str) -> Vec<f32> {
        let cw = size_px * CHAR_WIDTH_FACTOR;
        let mut lines = Vec::new();
        for para in text.split('\n') {
            let mut line_w = 0.0f32;
            let mut any = false;
            for word in para.split(' ') {
                let w = word.chars().count() as f32 * cw;
                let with_space = if any { line_w + cw + w } else { w };
                if any && max_w > 0.0 && with_space > max_w {
                    lines.push(line_w);
                    line_w = w;
                } else {
                    line_w = with_space;
                }
                any = true;
            }
            lines.push(line_w);
        }
        lines
    }

    fn measure(&self, style: Style, max_w: f32, text: &str) -> (f32, f32) {
        let lines = Self::lines(style.size_px, max_w, text);
        let w = lines.iter().copied().fold(0.0f32, f32::max);
        (w, lines.len() as f32 * style.size_px * LINE_HEIGHT_FACTOR)
    }

    fn text_arg(bytes: &[u8], cap: u32) -> Result<&str, i32> {
        if bytes.len() > cap as usize {
            return Err(err::E_ARG);
        }
        core::str::from_utf8(bytes).map_err(|_| err::E_ARG)
    }

    pub(crate) fn abi_version(&self) -> i32 {
        ABI_VERSION
    }

    pub(crate) fn log(&mut self, level: i32, bytes: &[u8]) -> i32 {
        if bytes.len() > limits::MAX_LOG_BYTES as usize {
            self.refuse("log", err::E_ARG);
            return err::E_ARG;
        }
        self.logs
            .push((level, String::from_utf8_lossy(bytes).into_owned()));
        err::OK
    }

    pub(crate) fn request_frame(&mut self) {
        self.frame_requests += 1;
    }

    pub(crate) fn exit(&mut self, code: i32) {
        self.exit_code = Some(code);
    }

    pub(crate) fn dl_begin(&mut self) -> i32 {
        if self.dl_open {
            self.refuse("dl_begin", err::E_DL_STATE);
            return err::E_DL_STATE;
        }
        self.dl_open = true;
        self.op_count = 0;
        self.clip_depth = 0;
        // Links live per frame (SPEC-5.2.10), and the runtime starts a new
        // semantic tree for every `vy_frame`, which a page enters through
        // `Frame::begin` (SPEC-10.1.1).
        self.links.clear();
        self.new_frame();
        self.ops.push(Op::Begin);
        err::OK
    }

    pub(crate) fn dl_end(&mut self) -> i32 {
        if !self.dl_open {
            self.refuse("dl_end", err::E_DL_STATE);
            return err::E_DL_STATE;
        }
        self.dl_open = false;
        self.ops.push(Op::End);
        if self.clip_depth != 0 {
            self.refuse("dl_end", err::E_DL_CLIP);
            return err::E_DL_CLIP;
        }
        err::OK
    }

    pub(crate) fn dl_rect(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        radius: f32,
        color: i32,
    ) -> i32 {
        if let Err(c) = self.gate("dl_rect") {
            return c;
        }
        self.op_count += 1;
        self.ops.push(Op::Rect {
            x,
            y,
            w,
            h,
            radius,
            color: color as u32,
        });
        err::OK
    }

    pub(crate) fn dl_text(&mut self, x: f32, y: f32, w: f32, style_id: i32, bytes: &[u8]) -> i64 {
        if let Err(c) = self.gate("dl_text") {
            return i64::from(c);
        }
        let Some(style) = self.style(style_id) else {
            self.refuse("dl_text", err::E_STYLE);
            return i64::from(err::E_STYLE);
        };
        let text = match Self::text_arg(bytes, limits::MAX_TEXT_BYTES) {
            Ok(t) => t,
            Err(c) => {
                self.refuse("dl_text", c);
                return i64::from(c);
            }
        };
        let (tw, th) = self.measure(style, w, text);
        self.op_count += 1;
        self.ops.push(Op::Text {
            x,
            y,
            max_w: w,
            style_id,
            text: text.into(),
        });
        pack(tw, th)
    }

    pub(crate) fn dl_image(
        &mut self,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        resource_id: i32,
        f: i32,
    ) -> i32 {
        if let Err(c) = self.gate("dl_image") {
            return c;
        }
        if !(fit::COVER..=fit::MAX).contains(&f) {
            self.refuse("dl_image", err::E_ARG);
            return err::E_ARG;
        }
        if resource_id <= 0 || resource_id > i32::from(u16::MAX) {
            self.refuse("dl_image", err::E_RESOURCE);
            return err::E_RESOURCE;
        }
        self.op_count += 1;
        self.ops.push(Op::Image {
            x,
            y,
            w,
            h,
            resource_id,
            fit: f,
        });
        err::OK
    }

    pub(crate) fn dl_path(&mut self, bytes: &[u8], color: i32, stroke_w: f32) -> i32 {
        if let Err(c) = self.gate("dl_path") {
            return c;
        }
        if bytes.len() > limits::MAX_PATH_BYTES as usize || !bytes.len().is_multiple_of(9) {
            self.refuse("dl_path", err::E_ARG);
            return err::E_ARG;
        }
        self.op_count += 1;
        self.ops.push(Op::Path {
            bytes: bytes.to_vec(),
            color: color as u32,
            stroke_w,
        });
        err::OK
    }

    pub(crate) fn dl_clip_push(&mut self, x: f32, y: f32, w: f32, h: f32, radius: f32) -> i32 {
        if let Err(c) = self.gate("dl_clip_push") {
            return c;
        }
        if self.clip_depth >= limits::MAX_CLIP_DEPTH {
            self.refuse("dl_clip_push", err::E_DL_CLIP);
            return err::E_DL_CLIP;
        }
        self.clip_depth += 1;
        self.op_count += 1;
        self.ops.push(Op::ClipPush { x, y, w, h, radius });
        err::OK
    }

    pub(crate) fn dl_clip_pop(&mut self) -> i32 {
        if let Err(c) = self.gate("dl_clip_pop") {
            return c;
        }
        if self.clip_depth == 0 {
            self.refuse("dl_clip_pop", err::E_DL_CLIP);
            return err::E_DL_CLIP;
        }
        self.clip_depth -= 1;
        self.op_count += 1;
        self.ops.push(Op::ClipPop);
        err::OK
    }

    pub(crate) fn text_style(&mut self, font: i32, size_px: f32, weight: i32, color: i32) -> i32 {
        if !(-5..=-1).contains(&font) && !(1..=i32::from(u16::MAX)).contains(&font) {
            self.refuse("text_style", err::E_RESOURCE);
            return err::E_RESOURCE;
        }
        if !(size_px > 0.0 && size_px <= 512.0) || !(1..=1000).contains(&weight) {
            self.refuse("text_style", err::E_ARG);
            return err::E_ARG;
        }
        self.styles.push(Style {
            font,
            size_px,
            weight,
            color: color as u32,
        });
        self.styles.len() as i32
    }

    pub(crate) fn text_measure(&mut self, style_id: i32, max_w: f32, bytes: &[u8]) -> i64 {
        let Some(style) = self.style(style_id) else {
            self.refuse("text_measure", err::E_STYLE);
            return i64::from(err::E_STYLE);
        };
        match Self::text_arg(bytes, limits::MAX_TEXT_BYTES) {
            Ok(text) => {
                let (w, h) = self.measure(style, max_w, text);
                pack(w, h)
            }
            Err(c) => {
                self.refuse("text_measure", c);
                i64::from(c)
            }
        }
    }

    pub(crate) fn text_caret(&mut self, style_id: i32, bytes: &[u8], byte_offset: i32) -> i64 {
        let Some(style) = self.style(style_id) else {
            self.refuse("text_caret", err::E_STYLE);
            return i64::from(err::E_STYLE);
        };
        let text = match Self::text_arg(bytes, limits::MAX_TEXT_BYTES) {
            Ok(t) => t,
            Err(c) => {
                self.refuse("text_caret", c);
                return i64::from(c);
            }
        };
        if byte_offset < 0 {
            self.refuse("text_caret", err::E_ARG);
            return i64::from(err::E_ARG);
        }
        let off = (byte_offset as usize).min(text.len());
        let before = &text[..text.floor_char_boundary(off)];
        let line = before.rsplit('\n').next().unwrap_or("");
        let lines_before = before.matches('\n').count() as f32;
        let x = line.chars().count() as f32 * style.size_px * CHAR_WIDTH_FACTOR;
        pack(x, lines_before * style.size_px * LINE_HEIGHT_FACTOR)
    }

    pub(crate) fn key_down(&self, code: i32) -> i32 {
        i32::from(self.keys_down.contains(&code))
    }

    pub(crate) fn ime_begin(&mut self, x: f32, y: f32, w: f32, h: f32) -> i32 {
        if self.caps.ime {
            self.ime_rect = Some((x, y, w, h));
            err::OK
        } else {
            self.refuse("ime_begin", err::E_DENIED);
            err::E_DENIED
        }
    }

    pub(crate) fn ime_end(&mut self) -> i32 {
        if self.caps.ime {
            self.ime_rect = None;
            err::OK
        } else {
            self.refuse("ime_end", err::E_DENIED);
            err::E_DENIED
        }
    }

    pub(crate) fn fetch_begin(&mut self, _method: i32, _url: &[u8], _body: &[u8]) -> i32 {
        self.refuse("fetch_begin", err::E_DENIED);
        err::E_DENIED
    }

    pub(crate) fn fetch_poll(&mut self, _req: i32) -> i32 {
        self.refuse("fetch_poll", err::E_DENIED);
        err::E_DENIED
    }

    pub(crate) fn fetch_read(&mut self, _req: i32, _dst: &mut [u8]) -> i32 {
        self.refuse("fetch_read", err::E_DENIED);
        err::E_DENIED
    }

    pub(crate) fn fetch_cancel(&mut self, _req: i32) -> i32 {
        self.refuse("fetch_cancel", err::E_DENIED);
        err::E_DENIED
    }

    pub(crate) fn state_put(&mut self, key: &[u8], val: &[u8]) -> i32 {
        if !self.caps.storage {
            self.refuse("state_put", err::E_DENIED);
            return err::E_DENIED;
        }
        if key.len() > limits::MAX_STATE_KEY_BYTES as usize
            || val.len() > limits::MAX_STATE_VALUE_BYTES as usize
        {
            self.refuse("state_put", err::E_ARG);
            return err::E_ARG;
        }
        self.state.insert(key.to_vec(), val.to_vec());
        err::OK
    }

    pub(crate) fn state_get(&mut self, key: &[u8], dst: &mut [u8]) -> i32 {
        if !self.caps.storage {
            self.refuse("state_get", err::E_DENIED);
            return err::E_DENIED;
        }
        if key.len() > limits::MAX_STATE_KEY_BYTES as usize {
            self.refuse("state_get", err::E_ARG);
            return err::E_ARG;
        }
        match self.state.get(key) {
            Some(v) => {
                let n = v.len().min(dst.len());
                dst[..n].copy_from_slice(&v[..n]);
                v.len() as i32
            }
            None => err::E_RESOURCE,
        }
    }

    pub(crate) fn state_delete(&mut self, key: &[u8]) -> i32 {
        if !self.caps.storage {
            self.refuse("state_delete", err::E_DENIED);
            return err::E_DENIED;
        }
        if key.len() > limits::MAX_STATE_KEY_BYTES as usize {
            self.refuse("state_delete", err::E_ARG);
            return err::E_ARG;
        }
        self.state.remove(key);
        err::OK
    }

    pub(crate) fn navigate(&mut self, url: &[u8], replace: i32) -> i32 {
        let url = String::from_utf8_lossy(url).into_owned();
        if url.starts_with("vy://") || url.starts_with("https://") {
            self.navigations.push((url, replace != 0));
            err::OK
        } else {
            self.refuse("navigate", err::E_ARG);
            err::E_ARG
        }
    }

    pub(crate) fn link_register(
        &mut self,
        id: i32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        url: &[u8],
    ) -> i32 {
        if url.len() > limits::MAX_URL_BYTES as usize {
            self.refuse("link_register", err::E_ARG);
            return err::E_ARG;
        }
        let url = String::from_utf8_lossy(url).into_owned();
        if let Some(l) = self.links.iter_mut().find(|l| l.0 == id) {
            *l = (id, x, y, w, h, url);
        } else if self.links.len() < MAX_LINKS_PER_FRAME {
            self.links.push((id, x, y, w, h, url));
        } else {
            self.refuse("link_register", err::E_DL_FULL);
            return err::E_DL_FULL;
        }
        err::OK
    }

    pub(crate) fn now_ms(&self) -> i64 {
        self.now_ms
    }

    pub(crate) fn random(&mut self, dst: &mut [u8]) -> i32 {
        for chunk in dst.chunks_mut(8) {
            let mut x = self.rng;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            self.rng = x;
            let v = x.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
        err::OK
    }

    pub(crate) fn clipboard_read(&mut self, dst: &mut [u8]) -> i32 {
        if !self.caps.clipboard {
            self.refuse("clipboard_read", err::E_DENIED);
            return err::E_DENIED;
        }
        let b = self.clipboard.as_bytes();
        let n = b.len().min(dst.len());
        dst[..n].copy_from_slice(&b[..n]);
        b.len() as i32
    }

    pub(crate) fn clipboard_write(&mut self, bytes: &[u8]) -> i32 {
        if !self.caps.clipboard {
            self.refuse("clipboard_write", err::E_DENIED);
            return err::E_DENIED;
        }
        if bytes.len() > limits::MAX_CLIPBOARD_BYTES as usize {
            self.refuse("clipboard_write", err::E_ARG);
            return err::E_ARG;
        }
        self.clipboard = String::from_utf8_lossy(bytes).into_owned();
        err::OK
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn sem_node(
        &mut self,
        id: i32,
        parent_id: i32,
        role: i32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        bytes: &[u8],
    ) -> i32 {
        if vy_abi::role::name(role).is_none() {
            self.refuse("sem_node", err::E_ARG);
            return err::E_ARG;
        }
        if self.sem.len() >= limits::MAX_SEM_NODES_PER_FRAME as usize {
            self.refuse("sem_node", err::E_DL_FULL);
            return err::E_DL_FULL;
        }
        // One node per id and frame (SPEC-5.2.13).
        if self.sem.iter().any(|n| n.id == id) {
            self.refuse("sem_node", err::E_ARG);
            return err::E_ARG;
        }
        let text = match Self::text_arg(bytes, limits::MAX_SEM_TEXT_BYTES) {
            Ok(t) => t,
            Err(c) => {
                self.refuse("sem_node", c);
                return c;
            }
        };
        self.sem.push(SemNode {
            id,
            parent_id,
            role,
            x,
            y,
            w,
            h,
            text: text.into(),
        });
        err::OK
    }

    /// Start a new frame as the runtime does before `vy_frame`: the semantic
    /// tree of the previous frame is dropped (SPEC-10.1.1). `dl_begin` does
    /// this too; call it directly for a tree built from events alone.
    pub fn new_frame(&mut self) {
        self.sem.clear();
    }
}

/// Pack a measurement as `dl_text` and `text_measure` do: each half at most
/// 2³¹ − 1, so the value is never negative (SPEC-5.2.4, SPEC-5.2.5).
fn pack(w: f32, h: f32) -> i64 {
    let half = |v: f32| (f64::from(v) * 100.0).round().clamp(0.0, 2_147_483_647.0) as i64;
    (half(w) << 32) | half(h)
}
