//! The 33 imports of namespace `vy` (SPEC-5.2.1..5.2.13), untyped.
//!
//! On `wasm32-unknown-unknown` these are the real imports; natively they
//! dispatch to [`crate::mock::MockHost`] on the current thread. The signatures
//! are exactly `vy_abi::IMPORTS`; `tests::spec_5_2_raw_signatures` keeps the
//! native copy honest. Every function is `unsafe` on both targets because the
//! pointer arguments must be valid for `len` bytes.

#[cfg(target_arch = "wasm32")]
mod ffi {
    #[link(wasm_import_module = "vy")]
    extern "C" {
        pub fn abi_version() -> i32;
        pub fn log(level: i32, ptr: *const u8, len: i32);
        pub fn request_frame();
        pub fn exit(code: i32);
        pub fn dl_begin();
        pub fn dl_end();
        pub fn dl_rect(x: f32, y: f32, w: f32, h: f32, radius: f32, color: i32);
        pub fn dl_text(x: f32, y: f32, w: f32, style_id: i32, ptr: *const u8, len: i32) -> i32;
        pub fn dl_image(x: f32, y: f32, w: f32, h: f32, resource_id: i32, fit: i32);
        pub fn dl_path(ptr: *const u8, len: i32, color: i32, stroke_w: f32);
        pub fn dl_clip_push(x: f32, y: f32, w: f32, h: f32);
        pub fn dl_clip_pop();
        pub fn text_style(font_resource_id: i32, size_px: f32, weight: i32, color: i32) -> i32;
        pub fn text_measure(style_id: i32, max_w: f32, ptr: *const u8, len: i32) -> i64;
        pub fn text_caret(style_id: i32, ptr: *const u8, len: i32, byte_offset: i32) -> i64;
        pub fn pointer_get() -> i64;
        pub fn key_down(code: i32) -> i32;
        pub fn ime_begin(x: f32, y: f32, w: f32, h: f32);
        pub fn ime_end();
        pub fn fetch_begin(
            method: i32,
            url_ptr: *const u8,
            url_len: i32,
            body_ptr: *const u8,
            body_len: i32,
        ) -> i32;
        pub fn fetch_poll(req_id: i32) -> i32;
        pub fn fetch_read(req_id: i32, dst_ptr: *mut u8, max: i32) -> i32;
        pub fn fetch_cancel(req_id: i32);
        pub fn state_put(key_ptr: *const u8, key_len: i32, val_ptr: *const u8, val_len: i32)
            -> i32;
        pub fn state_get(key_ptr: *const u8, key_len: i32, dst_ptr: *mut u8, max: i32) -> i32;
        pub fn state_delete(key_ptr: *const u8, key_len: i32);
        pub fn navigate(url_ptr: *const u8, url_len: i32, replace: i32);
        pub fn link_register(
            id: i32,
            x: f32,
            y: f32,
            w: f32,
            h: f32,
            url_ptr: *const u8,
            url_len: i32,
        );
        pub fn now_ms() -> i64;
        pub fn random(dst_ptr: *mut u8, len: i32);
        pub fn clipboard_read(dst_ptr: *mut u8, max: i32) -> i32;
        pub fn clipboard_write(ptr: *const u8, len: i32);
        pub fn sem_node(
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
}

#[cfg(not(target_arch = "wasm32"))]
mod ffi {
    //! Native stand-ins: each forwards to the thread's `MockHost`, reading
    //! `(ptr, len)` as a slice exactly as the host would read module memory.
    #![allow(clippy::missing_safety_doc)]

    use crate::mock::with;
    use core::slice;

    unsafe fn bytes<'a>(ptr: *const u8, len: i32) -> &'a [u8] {
        if ptr.is_null() || len <= 0 {
            &[]
        } else {
            slice::from_raw_parts(ptr, len as usize)
        }
    }

    unsafe fn bytes_mut<'a>(ptr: *mut u8, len: i32) -> &'a mut [u8] {
        if ptr.is_null() || len <= 0 {
            &mut []
        } else {
            slice::from_raw_parts_mut(ptr, len as usize)
        }
    }

    pub unsafe fn abi_version() -> i32 {
        with(|h| h.abi_version())
    }
    pub unsafe fn log(level: i32, ptr: *const u8, len: i32) {
        let b = bytes(ptr, len);
        with(|h| h.log(level, b))
    }
    pub unsafe fn request_frame() {
        with(|h| h.request_frame())
    }
    pub unsafe fn exit(code: i32) {
        with(|h| h.exit(code))
    }
    pub unsafe fn dl_begin() {
        with(|h| h.dl_begin())
    }
    pub unsafe fn dl_end() {
        with(|h| h.dl_end())
    }
    pub unsafe fn dl_rect(x: f32, y: f32, w: f32, h: f32, radius: f32, color: i32) {
        with(|m| m.dl_rect(x, y, w, h, radius, color))
    }
    pub unsafe fn dl_text(x: f32, y: f32, w: f32, style_id: i32, ptr: *const u8, len: i32) -> i32 {
        let b = bytes(ptr, len);
        with(|m| m.dl_text(x, y, w, style_id, b))
    }
    pub unsafe fn dl_image(x: f32, y: f32, w: f32, h: f32, resource_id: i32, fit: i32) {
        with(|m| m.dl_image(x, y, w, h, resource_id, fit))
    }
    pub unsafe fn dl_path(ptr: *const u8, len: i32, color: i32, stroke_w: f32) {
        let b = bytes(ptr, len);
        with(|m| m.dl_path(b, color, stroke_w))
    }
    pub unsafe fn dl_clip_push(x: f32, y: f32, w: f32, h: f32) {
        with(|m| m.dl_clip_push(x, y, w, h))
    }
    pub unsafe fn dl_clip_pop() {
        with(|m| m.dl_clip_pop())
    }
    pub unsafe fn text_style(font_resource_id: i32, size_px: f32, weight: i32, color: i32) -> i32 {
        with(|m| m.text_style(font_resource_id, size_px, weight, color))
    }
    pub unsafe fn text_measure(style_id: i32, max_w: f32, ptr: *const u8, len: i32) -> i64 {
        let b = bytes(ptr, len);
        with(|m| m.text_measure(style_id, max_w, b))
    }
    pub unsafe fn text_caret(style_id: i32, ptr: *const u8, len: i32, byte_offset: i32) -> i64 {
        let b = bytes(ptr, len);
        with(|m| m.text_caret(style_id, b, byte_offset))
    }
    pub unsafe fn pointer_get() -> i64 {
        with(|m| m.pointer_get())
    }
    pub unsafe fn key_down(code: i32) -> i32 {
        with(|m| m.key_down(code))
    }
    pub unsafe fn ime_begin(x: f32, y: f32, w: f32, h: f32) {
        with(|m| m.ime_begin(x, y, w, h))
    }
    pub unsafe fn ime_end() {
        with(|m| m.ime_end())
    }
    pub unsafe fn fetch_begin(
        method: i32,
        url_ptr: *const u8,
        url_len: i32,
        body_ptr: *const u8,
        body_len: i32,
    ) -> i32 {
        let (u, b) = (bytes(url_ptr, url_len), bytes(body_ptr, body_len));
        with(|m| m.fetch_begin(method, u, b))
    }
    pub unsafe fn fetch_poll(req_id: i32) -> i32 {
        with(|m| m.fetch_poll(req_id))
    }
    pub unsafe fn fetch_read(req_id: i32, dst_ptr: *mut u8, max: i32) -> i32 {
        let d = bytes_mut(dst_ptr, max);
        with(|m| m.fetch_read(req_id, d))
    }
    pub unsafe fn fetch_cancel(req_id: i32) {
        with(|m| m.fetch_cancel(req_id))
    }
    pub unsafe fn state_put(
        key_ptr: *const u8,
        key_len: i32,
        val_ptr: *const u8,
        val_len: i32,
    ) -> i32 {
        let (k, v) = (bytes(key_ptr, key_len), bytes(val_ptr, val_len));
        with(|m| m.state_put(k, v))
    }
    pub unsafe fn state_get(key_ptr: *const u8, key_len: i32, dst_ptr: *mut u8, max: i32) -> i32 {
        let k = bytes(key_ptr, key_len);
        let d = bytes_mut(dst_ptr, max);
        with(|m| m.state_get(k, d))
    }
    pub unsafe fn state_delete(key_ptr: *const u8, key_len: i32) {
        let k = bytes(key_ptr, key_len);
        with(|m| m.state_delete(k))
    }
    pub unsafe fn navigate(url_ptr: *const u8, url_len: i32, replace: i32) {
        let u = bytes(url_ptr, url_len);
        with(|m| m.navigate(u, replace))
    }
    pub unsafe fn link_register(
        id: i32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        url_ptr: *const u8,
        url_len: i32,
    ) {
        let u = bytes(url_ptr, url_len);
        with(|m| m.link_register(id, x, y, w, h, u))
    }
    pub unsafe fn now_ms() -> i64 {
        with(|m| m.now_ms())
    }
    pub unsafe fn random(dst_ptr: *mut u8, len: i32) {
        let d = bytes_mut(dst_ptr, len);
        with(|m| m.random(d))
    }
    pub unsafe fn clipboard_read(dst_ptr: *mut u8, max: i32) -> i32 {
        let d = bytes_mut(dst_ptr, max);
        with(|m| m.clipboard_read(d))
    }
    pub unsafe fn clipboard_write(ptr: *const u8, len: i32) {
        let b = bytes(ptr, len);
        with(|m| m.clipboard_write(b))
    }
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn sem_node(
        id: i32,
        parent_id: i32,
        role: i32,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        ptr: *const u8,
        len: i32,
    ) {
        let b = bytes(ptr, len);
        with(|m| m.sem_node(id, parent_id, role, x, y, w, h, b))
    }
}

pub(crate) use ffi::*;

/// `(ptr, len)` of a byte string as the imports take it. The ABI carries
/// lengths as `i32`; every typed binding caps the length against
/// `vy_abi::limits` before calling, so the cast cannot truncate.
pub(crate) fn pl(bytes: &[u8]) -> (*const u8, i32) {
    (bytes.as_ptr(), bytes.len() as i32)
}
