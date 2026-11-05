//! Typed bindings for the imports outside the display list (SPEC-5.2.1,
//! SPEC-5.2.5..5.2.13). The eight `dl_*` imports are [`crate::Frame`].
//!
//! Lengths are checked against `vy_abi::limits` before the call (SPEC-9.3.2),
//! so an over-long string is `Err(VyError::Arg)` here and never reaches the
//! host. Every import that can refuse a call returns its status code
//! (SPEC-5.3.1), which these bindings turn into `Err`; the host logs the
//! refusal as well.

use crate::error::{check, check64, VyError};
use crate::raw::{self, pl};
use crate::{from_x100, Color, Font, Rect, Size, StyleId};
use vy_abi::{level, limits, method};

/// Log level for [`log`], numbered as `vy_abi::level` (SPEC-5.2.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Level {
    /// Development chatter.
    Debug = level::DEBUG,
    /// Normal operation.
    Info = level::INFO,
    /// Something is off but the page goes on.
    Warn = level::WARN,
    /// The page cannot do what it meant to.
    Error = level::ERROR,
}

/// HTTP method for [`fetch_begin`], numbered as `vy_abi::method`
/// (SPEC-5.2.7); the host refuses any other number with `Arg`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Method {
    /// `GET`.
    Get = method::GET,
    /// `POST`.
    Post = method::POST,
    /// `PUT`.
    Put = method::PUT,
    /// `DELETE`.
    Delete = method::DELETE,
}

/// A network request started with [`fetch_begin`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request(pub i32);

fn capped(bytes: &[u8], cap: u32) -> Result<(*const u8, i32), VyError> {
    if bytes.len() > cap as usize {
        return Err(VyError::Arg);
    }
    Ok(pl(bytes))
}

fn len_i32(len: usize) -> Result<i32, VyError> {
    i32::try_from(len).map_err(|_| VyError::Arg)
}

/// `abi_version`: the host's ABI as `MAJOR × 100 + MINOR` (SPEC-5.2.1).
pub fn abi_version() -> i32 {
    // SAFETY: no arguments.
    unsafe { raw::abi_version() }
}

/// `log`: a line to the host's log (SPEC-5.2.1). Longer than 4 KB is `Arg`
/// (SPEC-9.3.2).
pub fn log(level: Level, message: &str) -> Result<(), VyError> {
    let (p, l) = capped(message.as_bytes(), limits::MAX_LOG_BYTES)?;
    // SAFETY: `p` is valid for `l` bytes for the duration of the call.
    check(unsafe { raw::log(level as i32, p, l) }).map(|_| ())
}

/// `request_frame`: ask for a `vy_frame` outside events and window changes
/// (SPEC-5.2.2).
pub fn request_frame() {
    // SAFETY: no arguments.
    unsafe { raw::request_frame() }
}

/// `exit`: stop the page with `code` once the current export returns (SPEC-5.2.1).
pub fn exit(code: i32) {
    // SAFETY: no pointers.
    unsafe { raw::exit(code) }
}

/// `text_style`: register a style (SPEC-5.2.5). `weight` is 1..=1000.
pub fn text_style(font: Font, size_px: f32, weight: u16, color: Color) -> Result<StyleId, VyError> {
    // SAFETY: no pointers.
    let id = unsafe {
        raw::text_style(
            font.resource_id(),
            size_px,
            i32::from(weight),
            color.as_i32(),
        )
    };
    check(id).map(StyleId)
}

/// `text_measure`: width of the widest line and height after wrapping at
/// `max_w` (SPEC-5.2.5, SPEC-6.4.3); `max_w <= 0` means no wrapping.
pub fn text_measure(style: StyleId, max_w: f32, text: &str) -> Result<Size, VyError> {
    let (p, l) = capped(text.as_bytes(), limits::MAX_TEXT_BYTES)?;
    // SAFETY: `p` is valid for `l` bytes for the duration of the call.
    let packed = check64(unsafe { raw::text_measure(style.0, max_w, p, l) })?;
    Ok(Size {
        w: from_x100((packed >> 32) as i32),
        h: from_x100(packed as i32),
    })
}

/// `text_caret`: position of the caret before `byte_offset` in an unwrapped
/// layout (SPEC-5.2.5).
pub fn text_caret(style: StyleId, text: &str, byte_offset: usize) -> Result<(f32, f32), VyError> {
    let (p, l) = capped(text.as_bytes(), limits::MAX_TEXT_BYTES)?;
    let off = len_i32(byte_offset)?;
    // SAFETY: `p` is valid for `l` bytes for the duration of the call.
    let packed = check64(unsafe { raw::text_caret(style.0, p, l, off) })?;
    Ok((from_x100((packed >> 32) as i32), from_x100(packed as i32)))
}

/// `key_down`: true while the USB HID key `code` is held (SPEC-5.2.6). The
/// pointer has no such query: a page keeps the last `POINTER_MOVE` itself.
pub fn key_down(code: i32) -> bool {
    // SAFETY: no pointers.
    unsafe { raw::key_down(code) != 0 }
}

/// `ime_begin`: start text composition over `rect` (SPEC-5.2.6; capability
/// `ime`, else `Denied`).
pub fn ime_begin(rect: Rect) -> Result<(), VyError> {
    // SAFETY: no pointers.
    check(unsafe { raw::ime_begin(rect.x, rect.y, rect.w, rect.h) }).map(|_| ())
}

/// `ime_end`: end text composition (SPEC-5.2.6; capability `ime`, else `Denied`).
pub fn ime_end() -> Result<(), VyError> {
    // SAFETY: no arguments.
    check(unsafe { raw::ime_end() }).map(|_| ())
}

/// `fetch_begin` (SPEC-5.2.7; capability `net`). URL over 2 KB or body over
/// 1 MB is `Arg` (SPEC-9.3.2); an origin outside the manifest is `Origin`
/// (SPEC-5.2.8).
pub fn fetch_begin(method: Method, url: &str, body: &[u8]) -> Result<Request, VyError> {
    let (up, ul) = capped(url.as_bytes(), limits::MAX_URL_BYTES)?;
    let (bp, bl) = capped(body, limits::MAX_FETCH_BODY_BYTES)?;
    // SAFETY: both pointers are valid for their lengths during the call.
    check(unsafe { raw::fetch_begin(method as i32, up, ul, bp, bl) }).map(Request)
}

/// `fetch_poll`: 0 while the response is on its way, 1 once `FETCH_DONE`
/// has been delivered and the body can be read (SPEC-5.2.7); `Arg` for a
/// request that is not open.
pub fn fetch_poll(req: Request) -> Result<i32, VyError> {
    // SAFETY: no pointers.
    check(unsafe { raw::fetch_poll(req.0) })
}

/// `fetch_read`: copy the next bytes of the buffered response body into
/// `dst`; returns how many, and 0 only at the end of the body, which closes
/// the request and frees the body in the host (SPEC-5.2.7). The whole body
/// is buffered before `FETCH_DONE`, so a loop until 0 reads all of it.
pub fn fetch_read(req: Request, dst: &mut [u8]) -> Result<usize, VyError> {
    let max = len_i32(dst.len())?;
    // SAFETY: `dst` is valid for `max` bytes during the call.
    check(unsafe { raw::fetch_read(req.0, dst.as_mut_ptr(), max) }).map(|n| n as usize)
}

/// `fetch_cancel`: drop a request and the response body the host holds for
/// it (SPEC-5.2.7).
pub fn fetch_cancel(req: Request) -> Result<(), VyError> {
    // SAFETY: no pointers.
    check(unsafe { raw::fetch_cancel(req.0) }).map(|_| ())
}

/// `state_put` (SPEC-5.2.9; capability `storage`). Key over 256 bytes or
/// value over 1 MB is `Arg` (SPEC-9.3.2).
pub fn state_put(key: &str, value: &[u8]) -> Result<(), VyError> {
    let (kp, kl) = capped(key.as_bytes(), limits::MAX_STATE_KEY_BYTES)?;
    let (vp, vl) = capped(value, limits::MAX_STATE_VALUE_BYTES)?;
    // SAFETY: both pointers are valid for their lengths during the call.
    check(unsafe { raw::state_put(kp, kl, vp, vl) }).map(|_| ())
}

/// `state_get`: copy a value into `dst`; returns the value's full length,
/// which may exceed `dst.len()`; a key with no value is `Resource`
/// (SPEC-5.2.9).
pub fn state_get(key: &str, dst: &mut [u8]) -> Result<usize, VyError> {
    let (kp, kl) = capped(key.as_bytes(), limits::MAX_STATE_KEY_BYTES)?;
    let max = len_i32(dst.len())?;
    // SAFETY: `kp` and `dst` are valid for their lengths during the call.
    check(unsafe { raw::state_get(kp, kl, dst.as_mut_ptr(), max) }).map(|n| n as usize)
}

/// `state_delete` (SPEC-5.2.9).
pub fn state_delete(key: &str) -> Result<(), VyError> {
    let (kp, kl) = capped(key.as_bytes(), limits::MAX_STATE_KEY_BYTES)?;
    // SAFETY: `kp` is valid for `kl` bytes during the call.
    check(unsafe { raw::state_delete(kp, kl) }).map(|_| ())
}

/// `navigate` to a `vy://` or `https://` address (SPEC-5.2.10); `replace`
/// swaps the current history entry.
pub fn navigate(url: &str, replace: bool) -> Result<(), VyError> {
    if !(url.starts_with("vy://") || url.starts_with("https://")) {
        return Err(VyError::Arg);
    }
    let (p, l) = capped(url.as_bytes(), limits::MAX_URL_BYTES)?;
    // SAFETY: `p` is valid for `l` bytes during the call.
    check(unsafe { raw::navigate(p, l, i32::from(replace)) }).map(|_| ())
}

/// `link_register`: tell the host that `rect` opens `url` for the current
/// frame (SPEC-5.2.10); `dl_begin` forgets every link, so a page registers
/// its links while it draws. The same `id` twice in a frame replaces the
/// rectangle; the 4 097th link of a frame is `DlFull` (SPEC-9.3.2).
pub fn link_register(id: i32, rect: Rect, url: &str) -> Result<(), VyError> {
    let (p, l) = capped(url.as_bytes(), limits::MAX_URL_BYTES)?;
    // SAFETY: `p` is valid for `l` bytes during the call.
    check(unsafe { raw::link_register(id, rect.x, rect.y, rect.w, rect.h, p, l) }).map(|_| ())
}

/// `now_ms`: the host's monotonic clock (SPEC-5.2.11).
pub fn now_ms() -> i64 {
    // SAFETY: no arguments.
    unsafe { raw::now_ms() }
}

/// `random`: fill `dst` from the host's random source (SPEC-5.2.11).
pub fn random(dst: &mut [u8]) -> Result<(), VyError> {
    let len = len_i32(dst.len())?;
    // SAFETY: `dst` is valid for `len` bytes during the call.
    check(unsafe { raw::random(dst.as_mut_ptr(), len) }).map(|_| ())
}

/// `clipboard_read`: copy the clipboard into `dst`; returns its full length
/// (SPEC-5.2.12; capability `clipboard`, user gesture within 1 s).
pub fn clipboard_read(dst: &mut [u8]) -> Result<usize, VyError> {
    let max = len_i32(dst.len())?;
    // SAFETY: `dst` is valid for `max` bytes during the call.
    check(unsafe { raw::clipboard_read(dst.as_mut_ptr(), max) }).map(|n| n as usize)
}

/// `clipboard_write` (SPEC-5.2.12; capability `clipboard`). Over 1 MB is `Arg`.
pub fn clipboard_write(text: &str) -> Result<(), VyError> {
    let (p, l) = capped(text.as_bytes(), limits::MAX_CLIPBOARD_BYTES)?;
    // SAFETY: `p` is valid for `l` bytes during the call.
    check(unsafe { raw::clipboard_write(p, l) }).map(|_| ())
}
