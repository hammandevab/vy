//! The fuzzed entry points, one per target. Shared by the cargo-fuzz
//! targets in `fuzz_targets/` (nightly, libFuzzer) and by the stable
//! regression test `crates/vy-shell/tests/fuzz_regressions.rs`, which
//! replays `fuzz/corpus/<target>/*` and `fuzz/artifacts/<target>/*`
//! through the same functions, so a crash found once is a gate forever.
//!
//! Each function takes arbitrary bytes and must neither panic nor hang;
//! where a format has one canonical encoding, an accepted input must
//! encode back to exactly its bytes.

/// A fuzzed entry point.
pub type Entry = fn(&[u8]);

/// The targets, in the order of `fuzz/Cargo.toml`.
pub const TARGETS: &[(&str, Entry)] = &[
    ("manifest", manifest),
    ("display_list", display_list),
    ("ipc", ipc),
    ("address", address),
    ("png", png),
    ("markdown", markdown),
    ("dns", dns),
];

/// Manifest canonical CBOR (SPEC-4.1.3) decode and validation, the key
/// log entry (SPEC-7.6.1) and the log's text form, and the package
/// container (SPEC-4.4.1).
pub fn manifest(data: &[u8]) {
    if let Ok(m) = vy_manifest::Manifest::decode(data) {
        let _ = m.identity();
        if m.sig.is_some() {
            let again = m.encode().expect("a decoded manifest with sig encodes");
            assert_eq!(again, data, "canonical CBOR re-encodes to the same bytes");
        }
    }
    if let Ok(e) = vy_manifest::LogEntry::decode(data) {
        assert_eq!(e.encode(), data, "a log entry re-encodes to the same bytes");
    }
    let _ = vy_manifest::KeyLog::parse(&String::from_utf8_lossy(data));
    if let Ok(p) = vy_manifest::Package::unpack(data) {
        let _ = p.validate();
    }
}

/// Display list decoding (SPEC-6.1.3, bilaga C), path commands included.
pub fn display_list(data: &[u8]) {
    let _ = vy_dl::decode(data);
    let _ = vy_dl::decode_path_cmds(data);
}

/// IPC messages between shell and tab (SPEC-11.2.1): both directions,
/// and an accepted message encodes back to the same body.
pub fn ipc(data: &[u8]) {
    if let Ok(m) = vy_shell::ipc::ToShell::decode(data) {
        assert_eq!(m.encode(), data, "ToShell re-encodes to the same body");
    }
    if let Ok(m) = vy_shell::ipc::ToTab::decode(data) {
        assert_eq!(m.encode(), data, "ToTab re-encodes to the same body");
    }
    let mut r = data;
    let _ = vy_shell::ipc::read_frame(&mut r);
}

/// `vy://` addresses (SPEC-7.1.1, 7.1.2, 9.5.1) and URL resolution: a
/// parsed address prints as one that parses to itself.
pub fn address(data: &[u8]) {
    let text = String::from_utf8_lossy(data);
    if let Ok(a) = vy_proto::Address::parse(&text) {
        let again = vy_proto::Address::parse(&a.to_string()).expect("printed address parses");
        assert_eq!(again, a, "an address round-trips through its text");
        let _ = (a.origin(), a.to_https(), a.fetch_url(), a.is_localhost());
    }
    if let Some((base, rel)) = text.split_once('\n') {
        let _ = vy_proto::resolve(base, rel);
    }
    let _ = vy_proto::HttpUrl::parse(&text);
}

/// PNG decoding of resources (SPEC-6.5.1, `vy-render` image.rs).
pub fn png(data: &[u8]) {
    let _ = vy_render::RgbaImage::decode_png(data);
    let _ = vy_manifest::png_dimensions(data);
}

/// The Markdown subset of the fallback page (SPEC-8.3.1).
pub fn markdown(data: &[u8]) {
    let _ = vy_shell::markdown::parse(&String::from_utf8_lossy(data));
}

/// The DNS answer parser (SPEC-7.4.5): the first two bytes are the id the
/// answer must carry, the rest the packet; and the query builder on text.
pub fn dns(data: &[u8]) {
    if data.len() >= 2 {
        let id = u16::from_be_bytes([data[0], data[1]]);
        let _ = vy_proto::dns::parse_answer(id, "_vy.exempel.se", &data[2..]);
        let _ = vy_proto::dns::parse_answer(id, "_vy.exempel.se", data);
    }
    let _ = vy_proto::dns::build_query(0, &String::from_utf8_lossy(data));
}
