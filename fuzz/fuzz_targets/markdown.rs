#![no_main]
//! `cargo +nightly fuzz run markdown`: see `vy_fuzz::entry::markdown`.

libfuzzer_sys::fuzz_target!(|data: &[u8]| vy_fuzz::entry::markdown(data));
