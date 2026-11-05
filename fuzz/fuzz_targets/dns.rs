#![no_main]
//! `cargo +nightly fuzz run dns`: see `vy_fuzz::entry::dns`.

libfuzzer_sys::fuzz_target!(|data: &[u8]| vy_fuzz::entry::dns(data));
