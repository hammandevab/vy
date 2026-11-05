#![no_main]
//! `cargo +nightly fuzz run address`: see `vy_fuzz::entry::address`.

libfuzzer_sys::fuzz_target!(|data: &[u8]| vy_fuzz::entry::address(data));
