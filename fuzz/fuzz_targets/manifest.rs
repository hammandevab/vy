#![no_main]
//! `cargo +nightly fuzz run manifest`: see `vy_fuzz::entry::manifest`.

libfuzzer_sys::fuzz_target!(|data: &[u8]| vy_fuzz::entry::manifest(data));
