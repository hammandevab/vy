//! Vy reference module 0 (`hello`).
//!
//! Draws a background, a card, a heading and a paragraph, and registers the
//! two text runs in the semantic tree. No capabilities are requested, so this
//! module runs in restricted mode without any difference.
//!
//! Exports exactly the six ABI functions (SPEC-4.2.1); imports only from `vy`
//! (SPEC-4.2.2); no WASI, no threads (SPEC-4.2.3).
#![no_std]
#![allow(clippy::missing_safety_doc)]

use core::cell::UnsafeCell;
use vy_abi::role;

#[link(wasm_import_module = "vy")]
extern "C" {
    fn abi_version() -> i32;
    fn log(level: i32, ptr: *const u8, len: i32);
    fn dl_begin();
    fn dl_end();
    fn dl_rect(x: f32, y: f32, w: f32, h: f32, radius: f32, color: i32);
    fn dl_text(x: f32, y: f32, w: f32, style_id: i32, ptr: *const u8, len: i32) -> i32;
    fn text_style(font_resource_id: i32, size_px: f32, weight: i32, color: i32) -> i32;
    fn text_measure(style_id: i32, max_w: f32, ptr: *const u8, len: i32) -> i64;
    #[allow(clippy::too_many_arguments)]
    fn sem_node(
        id: i32,
        parent_id: i32,
        role: i32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        ptr: *const u8,
        len: i32,
    );
}

const FONT_SANS: i32 = -1; // SPEC-6.4.2
const BG: i32 = 0x1A1A_2EFFu32 as i32;
const CARD: i32 = 0xF5F5_F0FFu32 as i32;
const INK: i32 = 0x1A1A_2EFFu32 as i32;
const MUTED: i32 = 0x5F5E_5AFFu32 as i32;

const TITLE: &str = "Hej från Vy";
const BODY: &str = "Den här sidan är en WebAssembly-modul. Den ritar sig själv via sex importer och äger inte en enda pixel.";

struct State {
    width: f32,
    height: f32,
    style_title: i32,
    style_body: i32,
    arena_used: usize,
    arena: [u8; 64 * 1024],
}

struct Cell(UnsafeCell<State>);
// Modules are single-threaded by spec (SPEC-4.2.3).
unsafe impl Sync for Cell {}

static STATE: Cell = Cell(UnsafeCell::new(State {
    width: 0.0,
    height: 0.0,
    style_title: 0,
    style_body: 0,
    arena_used: 0,
    arena: [0; 64 * 1024],
}));

#[allow(clippy::mut_from_ref)]
fn state() -> &'static mut State {
    // SAFETY: single-threaded module; the host never re-enters an export (SPEC-5.3.4).
    unsafe { &mut *STATE.0.get() }
}

#[no_mangle]
pub unsafe extern "C" fn vy_init(_abi_minor: i32, width: i32, height: i32, _scale: f32) -> i32 {
    let s = state();
    s.width = width as f32;
    s.height = height as f32;
    s.style_title = text_style(FONT_SANS, 28.0, 600, INK);
    s.style_body = text_style(FONT_SANS, 16.0, 400, MUTED);
    let msg = "hello: init";
    log(0, msg.as_ptr(), msg.len() as i32);
    let _ = abi_version();
    0
}

#[no_mangle]
pub unsafe extern "C" fn vy_frame(_dt_ms: i32) -> i32 {
    let s = state();
    let margin = 24.0;
    let card_w = (s.width - 2.0 * margin).max(0.0);
    let pad = 20.0;

    // Measure first (SPEC-5.2.5: width*100 in the high 32 bits, height*100 in the low),
    // then emit background, card and text in paint order (SPEC-6.1.3).
    let text_w = card_w - 2.0 * pad;
    let title_h = (text_measure(s.style_title, text_w, TITLE.as_ptr(), TITLE.len() as i32)
        & 0xFFFF_FFFF) as f32
        / 100.0;
    let body_h = (text_measure(s.style_body, text_w, BODY.as_ptr(), BODY.len() as i32)
        & 0xFFFF_FFFF) as f32
        / 100.0;
    let title_y = margin + pad;
    let body_y = title_y + title_h + 12.0;
    let card_h = pad + title_h + 12.0 + body_h + pad;

    dl_begin();
    dl_rect(0.0, 0.0, s.width, s.height, 0.0, BG);
    dl_rect(margin, margin, card_w, card_h, 12.0, CARD);
    dl_text(
        margin + pad,
        title_y,
        text_w,
        s.style_title,
        TITLE.as_ptr(),
        TITLE.len() as i32,
    );
    dl_text(
        margin + pad,
        body_y,
        text_w,
        s.style_body,
        BODY.as_ptr(),
        BODY.len() as i32,
    );
    dl_end();

    sem_node(
        1,
        0,
        role::MAIN,
        margin,
        margin,
        card_w,
        card_h,
        core::ptr::null(),
        0,
    );
    sem_node(
        2,
        1,
        role::HEADING,
        margin + pad,
        title_y,
        text_w,
        title_h,
        TITLE.as_ptr(),
        TITLE.len() as i32,
    );
    sem_node(
        3,
        1,
        role::TEXT,
        margin + pad,
        body_y,
        text_w,
        body_h,
        BODY.as_ptr(),
        BODY.len() as i32,
    );

    4
}

#[no_mangle]
pub unsafe extern "C" fn vy_event(_kind: i32, _a: i32, _b: i32, _c: i32, _d: i32) -> i32 {
    0
}

#[no_mangle]
pub unsafe extern "C" fn vy_resize(width: i32, height: i32, _scale: f32) {
    let s = state();
    s.width = width as f32;
    s.height = height as f32;
}

#[no_mangle]
pub unsafe extern "C" fn vy_suspend() -> i32 {
    0
}

/// Bump allocator over a static arena; enough for host writes in this module.
#[no_mangle]
pub unsafe extern "C" fn vy_alloc(size: i32) -> i32 {
    let s = state();
    let size = size.max(0) as usize;
    let start = (s.arena_used + 7) & !7;
    if start + size > s.arena.len() {
        return 0;
    }
    s.arena_used = start + size;
    s.arena.as_ptr().add(start) as i32
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
