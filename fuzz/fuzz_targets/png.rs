#![no_main]
//! `cargo +nightly fuzz run png`: see `vy_fuzz::entry::png`.

libfuzzer_sys::fuzz_target!(|data: &[u8]| vy_fuzz::entry::png(data));
