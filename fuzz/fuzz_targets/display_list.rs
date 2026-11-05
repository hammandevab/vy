#![no_main]
//! `cargo +nightly fuzz run display_list`: see `vy_fuzz::entry::display_list`.

libfuzzer_sys::fuzz_target!(|data: &[u8]| vy_fuzz::entry::display_list(data));
