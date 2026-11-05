//! cargo-fuzz targets for Vy's parsers of untrusted bytes (M4). The entry
//! points live in `entry.rs` so the stable regression test can replay the
//! corpora through exactly the same code.

pub mod entry;
