#![no_main]
//! `cargo +nightly fuzz run ipc`: see `vy_fuzz::entry::ipc`.

libfuzzer_sys::fuzz_target!(|data: &[u8]| vy_fuzz::entry::ipc(data));
