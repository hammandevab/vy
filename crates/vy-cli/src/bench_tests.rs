//! Performance budget tests (SPEC §13) and later conformance gates. Enabled milestone by milestone.
//! M1 tests live in `lib.rs` and `conformance.rs`.
//!
//! The §13 budgets are release-mode budgets on the reference machine
//! (SPEC §13: P95 over 100 runs). In a debug build these tests report and
//! return; in a release build (`cargo test --release`, the CI `bench` job)
//! they measure with the same code as `vy-cli bench` and assert. A budget
//! this machine does not meet is not loosened: the test is ignored with the
//! measured value in its reason, for the owner to decide.

use crate::bench::{self, Report, Row, Selection};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Runs per measurement, as SPEC §13 asks.
const RUNS: usize = 100;

/// The repository root, or `None` (and a note) in a debug build, where the
/// budgets are not measured.
fn release_root(spec: &str) -> Option<PathBuf> {
    if cfg!(debug_assertions) {
        eprintln!("{spec} is a release-mode budget; run with --release");
        return None;
    }
    Some(bench::root().expect("inside the repository"))
}

fn assert_within(row: &Row) {
    eprintln!(
        "{} ({}): P50 {:.2} P95 {:.2} {} (budget {:?})",
        row.id, row.spec, row.p50, row.p95, row.unit, row.budget
    );
    assert!(row.within(), "{} over budget: {row:?}", row.id);
}

fn exe() -> PathBuf {
    crate::conformance::tab_executable().expect("the vy-cli binary")
}

/// SPEC-1.1.3, SPEC-13.1.2: Enter to first frame on cache hit <= 100 ms:
/// a local file, so nothing is fetched, through the headless shell with a
/// tab process, for `hello` and the largest reference page.
#[test]
fn spec_13_1_2_first_frame_cache_hit() {
    let Some(root) = release_root("SPEC-13.1.2") else {
        return;
    };
    for name in ["hello", "text"] {
        assert_within(&bench::first_frame(&exe(), &root, name, RUNS).unwrap());
    }
}

/// A package of a reference module served on loopback, as `vy-publish`
/// builds and `vy-serve` serves it.
struct Served {
    _server: vy_serve::Handle,
    site: PathBuf,
    address: String,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.site);
    }
}

fn scratch(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("vy-bench-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn serve(root: &Path, name: &str) -> Served {
    let wasm = std::fs::read(root.join(format!("target/wasm/{name}.wasm"))).unwrap();
    let draft = vy_manifest::Draft::from_json(
        &std::fs::read_to_string(root.join(format!("conformance/manifests/{name}.json"))).unwrap(),
    )
    .unwrap();
    let key = vy_manifest::SigningKey::from_seed(&[0x13; 32]);
    let package = vy_publish::build(&draft, wasm, Vec::new(), &key).unwrap();
    let site = scratch("site");
    std::fs::write(site.join(format!("{name}.vyp")), package.pack().unwrap()).unwrap();
    let key_file = site.join("dev.key.pub");
    std::fs::write(&key_file, vy_manifest::key_file(&[key.public()])).unwrap();
    let server = vy_serve::start(
        vy_serve::Config {
            public_key_file: Some(key_file),
            ..vy_serve::Config::local(&site)
        },
        false,
    )
    .unwrap();
    let address = format!("vy://localhost:{}/{name}", server.port());
    Served {
        _server: server,
        site,
        address,
    }
}

/// Enter to the first composited frame for a page fetched with empty
/// caches: a new HTTP cache and key store, the manifest, the module and
/// the key fetched over loopback, a new tab process that compiles without a
/// compilation cache, the first frame rasterized.
fn first_frame_cold(exe: &Path, served: &Served, runs: usize) -> Vec<Duration> {
    let options = vy_shell::headless::Options {
        width: 1024.0,
        height: 720.0,
        scale: 1.0,
        max_ticks: 1,
        exe: Some(exe.to_path_buf()),
        timeout: Duration::from_secs(60),
    };
    let source = vy_shell::page::Source::parse(&served.address).unwrap();
    (0..runs)
        .map(|_| {
            let cache = scratch("cache");
            let t = Instant::now();
            let mut fetcher =
                vy_proto::Fetcher::new(vy_proto::Options::new(&cache)).expect("a cache");
            let page = vy_shell::page::load(&mut fetcher, &source)
                .unwrap_or_else(|e| panic!("{}: {e:?}", served.address));
            assert!(page.trusted, "signed and pinned at first visit");
            vy_shell::headless::screenshot_loaded(page, &options).unwrap();
            let d = t.elapsed();
            let _ = std::fs::remove_dir_all(&cache);
            d
        })
        .collect()
}

/// SPEC-13.1.3: cold fetch of a 300 KB module to first frame <= 400 ms.
/// The largest reference module (`text`, 31 KB) over loopback, every cache
/// empty; the reference machine's 50 Mbit/30 ms link adds about 50 ms for
/// 300 KB, which loopback does not (docs/notes/m4-perf.md).
#[test]
fn spec_13_1_3_first_frame_cold() {
    let Some(root) = release_root("SPEC-13.1.3") else {
        return;
    };
    let served = serve(&root, "text");
    let samples = first_frame_cold(&exe(), &served, RUNS);
    let row = bench::time_row(
        "first-frame-cold.text",
        "Enter till första bild, kall hämtning över loopback (text)",
        "SPEC-13.1.3",
        Some(400.0),
        samples,
    );
    assert_within(&row);
}

/// SPEC-13.2.2: input to pixel <= 16 ms: a pointer event into a running
/// `dashboard` tab, its frame back over the pipe, rasterized and composited.
#[test]
fn spec_13_2_2_input_latency() {
    let Some(root) = release_root("SPEC-13.2.2") else {
        return;
    };
    assert_within(&bench::input_to_pixel(&exe(), &root, RUNS).unwrap());
}

/// SPEC-13.2.3: scrolling a text page holds 60 fps with > 95 % glyph cache hits:
/// SCROLL, frame and rasterization per step within 16.7 ms, and the
/// shaped-text cache missing at most 5 % on both sides.
#[test]
fn spec_13_2_3_scroll_fps() {
    let Some(root) = release_root("SPEC-13.2.3") else {
        return;
    };
    let rows = bench::scroll_sequence(
        &root,
        &Selection {
            runs: RUNS,
            only: None,
        },
    )
    .unwrap();
    assert_eq!(rows.len(), 3);
    for row in &rows {
        assert_within(row);
    }
}

/// SPEC-13.4.1: SDK refuses to publish a module over budget without --allow-slow.
/// `vy-publish build` runs the module headless (init, first frame, a
/// resize, pointer, key and scroll events) and refuses a package whose
/// frame uses more than the 8 ms fuel budget, naming the frame and the fuel
/// (`--allow-slow` publishes it anyway; the command is tested in
/// vy-publish's `tests/cli.rs`). Fuel, not time: the verdict is the same on
/// every machine, in debug and in release. Every reference page passes.
#[test]
fn spec_13_4_1_sdk_refuses_slow_module() {
    let key = vy_manifest::SigningKey::from_seed(&[0x13; 32]);
    let root = bench::root().unwrap();
    let draft = |name: &str| {
        vy_manifest::Draft::from_json(
            &std::fs::read_to_string(root.join(format!("conformance/manifests/{name}.json")))
                .unwrap(),
        )
        .unwrap()
    };
    // Slow from its first frame: two hundred million steps would be cut at
    // the hard limit (64 ms); the check reports at least that much.
    let slow = wat::parse_str(
        r#"(module (memory (export "memory") 2)
            (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
            (func (export "vy_frame") (param i32) (result i32)
                (local $i i32)
                (loop $spin
                    (local.set $i (i32.add (local.get $i) (i32.const 1)))
                    (br_if $spin (i32.lt_u (local.get $i) (i32.const 200000000))))
                i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
            (func (export "vy_resize") (param i32 i32 f32))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
    )
    .unwrap();
    let package = vy_publish::build(&draft("hello"), slow, Vec::new(), &key).unwrap();
    let err = vy_publish::check_frames(&package).unwrap_err();
    let message = err.to_string();
    match err {
        vy_publish::BuildError::OverBudget {
            frame,
            fuel,
            budget,
        } => {
            assert_eq!(frame, "frame 1 (first, 800 × 600 at scale 1)");
            assert_eq!(budget, vy_runtime::FUEL_PER_MS * 8);
            assert!(fuel >= vy_runtime::CALL_HARD_LIMIT_FUEL, "{fuel}");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        message.contains("frame 1") && message.contains("--allow-slow"),
        "{message}"
    );

    // The reference pages publish.
    for name in ["hello", "text", "chat", "dashboard", "game"] {
        let wasm = std::fs::read(root.join(format!("target/wasm/{name}.wasm"))).unwrap();
        let package = vy_publish::build(&draft(name), wasm, Vec::new(), &key).unwrap();
        let costs = vy_publish::check_frames(&package).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(costs.len() >= 2, "{name}: {costs:?}");
    }
}

fn row(id: &str, unit: &str, p50: f64, p95: f64) -> Row {
    Row {
        id: id.into(),
        what: id.into(),
        spec: "SPEC-13.2.1".into(),
        unit: unit.into(),
        budget: None,
        p50,
        p95,
        n: 100,
    }
}

/// SPEC-12.2.3: no performance regression over 5 %. The gate compares a
/// bench report with a committed baseline for the same runner class
/// (`conformance/bench/<class>.json`, changed only by a `bench:` commit):
/// a row regresses when its P50 is more than 5 % plus a noise floor over
/// the baseline's. Deterministic here: the comparison itself, on reports
/// made up for it, and every committed baseline parses as a release
/// report with the §13 rows.
#[test]
fn spec_12_2_3_perf_regression_gate() {
    let base = Report {
        runs: 100,
        profile: "release".into(),
        rows: vec![
            row("raster.page.text", "ms", 10.0, 12.0),
            row("raster.page.hello", "ms", 0.3, 0.4),
            row("scroll.text.miss", "%", 2.6, 2.6),
            row("compile.hello", "ms", 14.0, 15.0),
        ],
    };
    let with = |changes: &[(&str, f64)]| {
        let mut r = base.clone();
        for (id, p50) in changes {
            r.rows.iter_mut().find(|r| r.id == *id).unwrap().p50 = *p50;
        }
        r
    };
    let ids = |r: Vec<bench::Regression>| r.into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert!(bench::compare(&base, &base).unwrap().is_empty());
    // 4 % slower is within 5 %; faster is never a regression.
    assert!(bench::compare(&base, &with(&[("raster.page.text", 10.4)]))
        .unwrap()
        .is_empty());
    assert!(bench::compare(&base, &with(&[("compile.hello", 7.0)]))
        .unwrap()
        .is_empty());
    // 11 % slower is a regression: 10 × 1.05 + 0.5 ms floor = 11.0.
    assert_eq!(
        ids(bench::compare(&base, &with(&[("raster.page.text", 11.1)])).unwrap()),
        ["raster.page.text"]
    );
    assert!(bench::compare(&base, &with(&[("raster.page.text", 10.9)]))
        .unwrap()
        .is_empty());
    // A row far under a millisecond moves within the noise floor.
    assert!(bench::compare(&base, &with(&[("raster.page.hello", 0.7)]))
        .unwrap()
        .is_empty());
    // Percentages have their own floor, in points.
    assert_eq!(
        ids(bench::compare(&base, &with(&[("scroll.text.miss", 4.0)])).unwrap()),
        ["scroll.text.miss"]
    );
    // Rows only one side has are not compared; a debug report never is.
    let mut fewer = base.clone();
    fewer.rows.truncate(1);
    assert!(bench::compare(&base, &fewer).unwrap().is_empty());
    let mut debug = base.clone();
    debug.profile = "debug".into();
    assert!(bench::compare(&base, &debug).is_err());

    // The committed baselines.
    let dir = bench::root().unwrap().join("conformance/bench");
    let mut found = 0;
    for entry in std::fs::read_dir(&dir).expect("conformance/bench") {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let report = Report::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(report.profile, "release", "{}", path.display());
        for id in ["raster.ops-2000", "scroll.text.frame", "compile.hello"] {
            assert!(report.row(id).is_some(), "{}: {id}", path.display());
        }
        found += 1;
    }
    assert!(found >= 1, "at least one committed baseline");
}

/// SPEC-12.3.1: client binary <= 25 MB; whole codebase < 400 000 tokens.
#[test]
#[ignore = "M5: release"]
fn spec_12_3_1_size_budget() {
    todo!()
}
