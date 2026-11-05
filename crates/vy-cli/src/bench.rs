//! `vy-cli bench`: what of SPEC §13 can be measured without a window, as
//! P95 (and P50) over `runs` runs against the budgets, printed as a table
//! or as JSON for `spec_12_2_3_perf_regression_gate`, which compares the
//! JSON with a committed baseline per runner class (SPEC-12.2.3).
//!
//! Measured, with the row id in brackets:
//! - compile time of every reference module (SPEC-13.1.4, `Runtime::load`
//!   with the ABI check and hash) [`compile.<module>`], and the time to
//!   get the compiled module on a compilation-cache hit
//!   [`compile-hit.<module>`];
//! - rasterization of 2 000-op frames at 1280 × 800 (SPEC-13.2.1): the
//!   frame of `vy_render::bench` in the proportions of the reference
//!   pages, warm [`raster.ops-2000`] and with a fresh text engine per run
//!   [`raster.ops-2000-cold`], three pathological lists, rectangles
//!   alone, text alone and both [`raster.rects-2000`, `raster.text-200`,
//!   `raster.mix-1800-200`], and 2 000 rounded cards of a push, a
//!   rectangle and a pop each [`raster.cards-2000`];
//! - rasterization of every reference page's first conformance case
//!   [`raster.page.<module>`];
//! - a scroll sequence through the `text` page: SCROLL event, frame and
//!   rasterization per step [`scroll.text.frame`], with the miss rate of
//!   the shaped-text cache on the rasterizing side [`scroll.text.miss`]
//!   and on the module's side, where `dl_text` measures
//!   [`scroll.text.measure-miss`] (SPEC-13.2.3);
//! - a tab process's cold start: spawn `--tab`, load `hello`, first
//!   display list back (the process half of SPEC-13.1.1) [`tab.cold-start`];
//! - Enter to first composited frame headless for `hello` and `text`
//!   from a local file, the cache-hit path of SPEC-13.1.2 minus the
//!   window [`first-frame.hello`, `first-frame.text`];
//! - input to pixel (SPEC-13.2.2): a pointer event into a running
//!   `dashboard` tab, its frame back and rasterized [`input.dashboard`].

use crate::conformance::{self, CasePackage};
use crate::{drive, Driven, RenderRequest};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use vy_abi::event as kind;
use vy_render::TextEngine;
use vy_runtime::{Options, Runtime};
use vy_shell::ipc::{LoadRequest, ToShell, ToTab};
use vy_shell::process::{channel_sink, spawn_process, Inbound};

/// Runs per measurement unless `--runs` says otherwise: §13 asks for
/// P95 over 100.
pub const DEFAULT_RUNS: usize = 100;
/// One frame at 60 fps, the budget of a scroll step (SPEC-13.2.3).
pub const FRAME_60FPS_MS: f64 = 1000.0 / 60.0;
/// Largest miss rate of the shaped-text cache on a scroll (SPEC-13.2.3:
/// hits over 95 %).
pub const MAX_TEXT_MISS_PERCENT: f64 = 5.0;
/// Logical pixels one scroll step moves the text page.
pub const SCROLL_STEP_PX: f32 = 24.0;

/// One measured row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Row {
    /// Stable machine key, `section.name`.
    pub id: String,
    /// What was measured, for people.
    pub what: String,
    /// The spec sentence.
    pub spec: String,
    /// Unit of `p50`, `p95` and `budget`: `ms` or `%`.
    pub unit: String,
    /// The budget, when the spec gives one; the P95 must stay at or under it.
    pub budget: Option<f64>,
    /// Median.
    pub p50: f64,
    /// 95th percentile.
    pub p95: f64,
    /// Samples behind the percentiles.
    pub n: usize,
}

impl Row {
    /// True when the P95 is within the budget (or there is none).
    pub fn within(&self) -> bool {
        self.budget.is_none_or(|b| self.p95 <= b)
    }
}

/// A whole bench run, as `--json` prints it and the baseline files hold it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// Runs per measurement.
    pub runs: usize,
    /// `release` or `debug`; only a release build measures the budgets.
    pub profile: String,
    /// The rows, in a fixed order.
    pub rows: Vec<Row>,
}

impl Report {
    /// The rows measured now with `runs` runs.
    pub fn new(runs: usize, rows: Vec<Row>) -> Self {
        Self {
            runs,
            profile: if cfg!(debug_assertions) {
                "debug".into()
            } else {
                "release".into()
            },
            rows,
        }
    }

    /// Pretty JSON with a trailing newline.
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("rows serialize");
        s.push('\n');
        s
    }

    /// Parse what [`Report::to_json`] wrote.
    pub fn from_json(text: &str) -> Result<Self> {
        serde_json::from_str(text).context("bench report JSON")
    }

    /// The row with this id.
    pub fn row(&self, id: &str) -> Option<&Row> {
        self.rows.iter().find(|r| r.id == id)
    }
}

fn percentile(samples: &mut [f64], p: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    let i = ((samples.len() as f64 - 1.0) * p).round() as usize;
    samples[i.min(samples.len() - 1)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn row_of(
    id: impl Into<String>,
    what: impl Into<String>,
    spec: &str,
    unit: &str,
    budget: Option<f64>,
    mut samples: Vec<f64>,
) -> Row {
    let n = samples.len();
    assert!(n > 0, "a row needs samples");
    Row {
        id: id.into(),
        what: what.into(),
        spec: spec.into(),
        unit: unit.into(),
        budget,
        p50: percentile(&mut samples, 0.5),
        p95: percentile(&mut samples, 0.95),
        n,
    }
}

pub(crate) fn time_row(
    id: impl Into<String>,
    what: impl Into<String>,
    spec: &str,
    budget_ms: Option<f64>,
    samples: Vec<Duration>,
) -> Row {
    row_of(
        id,
        what,
        spec,
        "ms",
        budget_ms,
        samples.into_iter().map(ms).collect(),
    )
}

/// Which rows to measure: every one, or those whose id starts with a prefix.
#[derive(Clone, Copy, Debug)]
pub struct Selection<'a> {
    /// Runs per measurement.
    pub runs: usize,
    /// Only rows whose id starts with this.
    pub only: Option<&'a str>,
}

impl Selection<'_> {
    fn wants(&self, id: &str) -> bool {
        self.only.is_none_or(|p| id.starts_with(p))
    }
}

/// The modules under `target/wasm`, by name.
fn modules(root: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    let dir = root.join("target/wasm");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .with_context(|| format!("{} (run tools/build_modules.sh)", dir.display()))?
    {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "wasm") {
            let name = path.file_stem().unwrap().to_string_lossy().into_owned();
            out.push((name, std::fs::read(&path)?));
        }
    }
    out.sort();
    Ok(out)
}

/// SPEC-13.1.4: compile (load) time per module without a cache, and the
/// time to get the compiled module on a compilation-cache hit.
fn compile_times(mods: &[(String, Vec<u8>)], sel: &Selection<'_>) -> Vec<Row> {
    let mut rows = Vec::new();
    for (name, wasm) in mods {
        let hash = vy_runtime::sha256(wasm);
        if sel.wants(&format!("compile.{name}")) {
            let samples = (0..sel.runs)
                .map(|_| {
                    let t = Instant::now();
                    let rt = Runtime::load(wasm, &hash, Options::default());
                    let d = t.elapsed();
                    drop(rt);
                    d
                })
                .collect();
            rows.push(time_row(
                format!("compile.{name}"),
                format!("kompilering {name} ({} KB)", wasm.len() / 1024),
                "SPEC-13.1.4",
                Some(60.0),
                samples,
            ));
        }
        let id = format!("compile-hit.{name}");
        if sel.wants(&id) {
            if let Ok(samples) = cache_hit_times(wasm, sel.runs) {
                rows.push(time_row(
                    id,
                    format!("kompileringscache-träff {name}: modulen ur cachen"),
                    "SPEC-13.1.4",
                    Some(CACHE_HIT_BUDGET_MS),
                    samples,
                ));
            }
        }
    }
    rows
}

/// "0 ms vid kompileringscache-träff" (SPEC-13.1.4) in whole milliseconds:
/// under one.
pub const CACHE_HIT_BUDGET_MS: f64 = 1.0;

/// The time [`Runtime::load`] spends getting the compiled module when the
/// compilation cache holds it, `runs` times: one load writes the entry,
/// then each read-only load takes it from there (SPEC-13.1.4).
pub(crate) fn cache_hit_times(wasm: &[u8], runs: usize) -> Result<Vec<Duration>> {
    let dir = std::env::temp_dir().join(format!(
        "vy-bench-code-{}-{}",
        std::process::id(),
        vy_runtime::hex(&vy_runtime::sha256(wasm)[..8])
    ));
    let hash = vy_runtime::sha256(wasm);
    let load = |cache: vy_runtime::CompileCache| {
        let options = Options {
            compile_cache: cache,
            ..Options::default()
        };
        Runtime::load(wasm, &hash, options).map_err(|e| anyhow!("{e}"))
    };
    let result = (|| {
        load(vy_runtime::CompileCache::ReadWrite(dir.clone()))?;
        (0..runs)
            .map(|_| {
                let rt = load(vy_runtime::CompileCache::ReadOnly(dir.clone()))?;
                let stats = rt.load_stats();
                if stats.source != vy_runtime::CompileSource::CacheHit {
                    return Err(anyhow!("the compilation cache missed"));
                }
                Ok(stats.module_time)
            })
            .collect()
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Rasterize `list` `runs` times at `w` × `h` physical pixels.
fn time_raster(
    list: &[u8],
    w: u32,
    h: u32,
    scale: f32,
    text: &mut TextEngine,
    images: &vy_render::ImageCache,
    runs: usize,
) -> Vec<Duration> {
    (0..runs)
        .map(|_| {
            let t = Instant::now();
            let r = vy_render::render(list, w, h, scale, text, images);
            let d = t.elapsed();
            drop(r);
            d
        })
        .collect()
}

/// SPEC-13.2.1: 2 000-op frames rasterized at 1280 × 800. The frame in the
/// reference pages' proportions twice: warm, one text engine for every run
/// as a tab keeps it across frames (the budget row), and cold, a fresh
/// engine per run as after Enter (no budget: shaping and glyph
/// rasterization from nothing). Then M3's pathological lists, rectangles
/// alone, text alone (200 runs of 40 characters) and the mix of 1 800
/// rectangles and 200 runs, so the shaping cost shows by itself, and the
/// 2 000 rounded cards of the vy-render tests (6 000 ops, no budget).
fn raster_times(sel: &Selection<'_>) -> Result<Vec<Row>> {
    let mut rows = Vec::new();
    if sel.wants("raster.ops-2000") {
        let mut text = TextEngine::new();
        let frame = vy_render::bench::frame_2000(&mut text, 1280.0, 800.0);
        let samples = time_raster(
            &frame.list,
            1280,
            800,
            1.0,
            &mut text,
            &frame.images,
            sel.runs,
        );
        rows.push(time_row(
            "raster.ops-2000",
            "rasterisering 2 000 opkoder i sidornas proportioner 1280×800",
            "SPEC-13.2.1",
            Some(2.0),
            samples,
        ));
    }
    if sel.wants("raster.ops-2000-cold") {
        let samples = (0..sel.runs)
            .map(|_| {
                let mut text = TextEngine::new();
                let frame = vy_render::bench::frame_2000(&mut text, 1280.0, 800.0);
                time_raster(&frame.list, 1280, 800, 1.0, &mut text, &frame.images, 1)[0]
            })
            .collect();
        rows.push(time_row(
            "raster.ops-2000-cold",
            "dito med ny textmotor varje körning (kall formning och glyfer)",
            "SPEC-13.2.1",
            None,
            samples,
        ));
    }
    let images = vy_render::ImageCache::empty(0);
    for (id, what, rects, runs) in [
        (
            "raster.rects-2000",
            "rasterisering 2 000 rundade genomskinliga rektanglar 1280×800",
            2_000,
            0,
        ),
        (
            "raster.text-200",
            "rasterisering 200 textrader à 40 tecken 1280×800",
            0,
            200,
        ),
        (
            "raster.mix-1800-200",
            "rasterisering 1 800 rektanglar + 200 textrader 1280×800",
            1_800,
            200,
        ),
    ] {
        if !sel.wants(id) {
            continue;
        }
        let mut text = TextEngine::new();
        let bytes = vy_render::bench::grid(&mut text, rects, runs);
        let samples = time_raster(&bytes, 1280, 800, 1.0, &mut text, &images, sel.runs);
        rows.push(time_row(id, what, "SPEC-13.2.1", Some(2.0), samples));
    }
    if sel.wants("raster.cards-2000") {
        let mut text = TextEngine::new();
        let bytes = vy_render::bench::rounded_cards();
        let samples = time_raster(&bytes, 1280, 800, 1.0, &mut text, &images, sel.runs);
        rows.push(time_row(
            "raster.cards-2000",
            "rasterisering 2 000 rundade klipp med rektangel (6 000 opkoder)",
            "SPEC-13.2.1",
            None,
            samples,
        ));
    }
    Ok(rows)
}

/// A conformance case driven to its last frame: the module instance with
/// its text engine and images, and the display list to rasterize.
fn drive_case(root: &Path, case: &conformance::Case) -> Result<Driven> {
    let CasePackage {
        wasm,
        manifest,
        resources,
    } = conformance::load_case_package(root, case)?;
    let net_origins = manifest
        .capabilities
        .net
        .as_ref()
        .map(|n| n.origins.clone())
        .unwrap_or_default();
    let storage_quota_bytes = manifest
        .capabilities
        .storage
        .as_ref()
        .map(|s| u64::from(s.quota_kb) * 1024)
        .unwrap_or(vy_runtime::DEFAULT_STORAGE_QUOTA_BYTES);
    let req = RenderRequest {
        wasm: &wasm,
        width: case.width,
        height: case.height,
        scale: case.scale,
        frames: case.frames,
        memory_max_pages: manifest.memory_max,
        capabilities: case.capabilities,
        resources: &resources,
        events: &case.events,
        fetch: &case.fetch,
        net_origins: &net_origins,
        storage_quota_bytes,
    };
    drive(&req).with_context(|| format!("case {}", case.name))
}

/// SPEC-13.2.1 on real frames: the first headless conformance case of
/// every module, driven to its last frame, then rasterized `runs` times.
fn page_raster_times(root: &Path, sel: &Selection<'_>) -> Result<Vec<Row>> {
    let cases = conformance::load_cases(root)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut rows = Vec::new();
    for case in cases.iter().filter(|c| !c.shell) {
        if !seen.insert(case.module.clone()) {
            continue;
        }
        let id = format!("raster.page.{}", case.module);
        if !sel.wants(&id) {
            continue;
        }
        let mut driven = drive_case(root, case)?;
        let ops = vy_dl::decode(&driven.display_list)
            .map_err(|e| anyhow!("{}: {e:?}", case.name))?
            .len();
        let (pw, ph) = crate::physical(driven.width, driven.height, driven.scale);
        let (text, images) = driven.rt.raster_parts();
        let samples = time_raster(
            &driven.display_list,
            pw,
            ph,
            driven.scale,
            text,
            images,
            sel.runs,
        );
        rows.push(time_row(
            id,
            format!("rasterisering {} ({ops} opkoder)", case.name),
            "SPEC-13.2.1",
            Some(2.0),
            samples,
        ));
    }
    Ok(rows)
}

/// Register in `into` the styles of `from` it does not have yet, as the
/// shell's main process mirrors a tab's styles before rasterizing its
/// frames (SPEC-9.1.2).
fn mirror_styles(from: &mut TextEngine, into: &mut TextEngine) -> Result<()> {
    for id in into.style_count() + 1..=from.style_count() {
        let s = from.get_style(id as u16).expect("a registered style");
        let got = into
            .style(s.font, s.size_px, i32::from(s.weight), s.color)
            .map_err(|c| anyhow!("style: {c}"))?;
        if usize::from(got) != id {
            return Err(anyhow!("style ids diverged: {got} for {id}"));
        }
    }
    Ok(())
}

/// SPEC-13.2.3: a scroll through the `text` page at 1280 × 800, one
/// SCROLL event, frame and rasterization per step, the frame rasterized by
/// a second text engine as the shell's main process does; the miss rates
/// of the shaped-text cache on both sides over the whole sequence.
pub(crate) fn scroll_sequence(root: &Path, sel: &Selection<'_>) -> Result<Vec<Row>> {
    let ids = [
        "scroll.text.frame",
        "scroll.text.miss",
        "scroll.text.measure-miss",
    ];
    if !ids.iter().any(|id| sel.wants(id)) {
        return Ok(Vec::new());
    }
    let wasm = std::fs::read(root.join("target/wasm/text.wasm"))
        .context("target/wasm/text.wasm (run tools/build_modules.sh)")?;
    let hash = vy_runtime::sha256(&wasm);
    let mut rt = Runtime::load(&wasm, &hash, Options::default()).map_err(|e| anyhow!("{e}"))?;
    rt.init(1280, 800, 1.0).map_err(|e| anyhow!("{e}"))?;
    let mut raster_text = TextEngine::new();
    let images = vy_render::ImageCache::empty(0);
    // The first frame: everything is a miss, as after Enter.
    let first = rt.frame().map_err(|e| anyhow!("{e}"))?.to_vec();
    mirror_styles(rt.text_engine_mut(), &mut raster_text)?;
    vy_render::render(&first, 1280, 800, 1.0, &mut raster_text, &images)
        .map_err(|e| anyhow!("{e}"))?;
    let raster_before = raster_text.cache_stats();
    let measure_before = rt.text_engine_mut().cache_stats();
    // Down, and back up from the end of the page when a run is longer
    // than the article (about 450 steps at 1280 × 800).
    let mut dy = (SCROLL_STEP_PX * 100.0) as i32;
    let mut samples = Vec::with_capacity(sel.runs);
    for _ in 0..sel.runs {
        let t = Instant::now();
        let mut redraw = rt
            .event(kind::SCROLL, 0, dy, 0, 0)
            .map_err(|e| anyhow!("{e}"))?;
        if !redraw {
            dy = -dy;
            redraw = rt
                .event(kind::SCROLL, 0, dy, 0, 0)
                .map_err(|e| anyhow!("{e}"))?;
        }
        if !redraw {
            return Err(anyhow!("the text page does not scroll"));
        }
        let list = rt.frame().map_err(|e| anyhow!("{e}"))?.to_vec();
        mirror_styles(rt.text_engine_mut(), &mut raster_text)?;
        let r = vy_render::render(&list, 1280, 800, 1.0, &mut raster_text, &images)
            .map_err(|e| anyhow!("{e}"))?;
        samples.push(t.elapsed());
        drop(r);
    }
    let miss_percent = |before: vy_render::CacheStats, after: vy_render::CacheStats| {
        let lookups = after.lookups() - before.lookups();
        let misses = after.misses - before.misses;
        (
            misses as f64 / lookups.max(1) as f64 * 100.0,
            lookups as usize,
        )
    };
    let (raster_miss, raster_lookups) = miss_percent(raster_before, raster_text.cache_stats());
    let (measure_miss, measure_lookups) =
        miss_percent(measure_before, rt.text_engine_mut().cache_stats());
    let mut rows = Vec::new();
    if sel.wants(ids[0]) {
        rows.push(time_row(
            ids[0],
            format!("rullning text 1280×800: SCROLL {SCROLL_STEP_PX} px, frame och rasterisering"),
            "SPEC-13.2.3",
            Some(FRAME_60FPS_MS),
            samples,
        ));
    }
    if sel.wants(ids[1]) {
        rows.push(row_of(
            ids[1],
            "textcache-missar vid rullning, rasteriserande sida",
            "SPEC-13.2.3",
            "%",
            Some(MAX_TEXT_MISS_PERCENT),
            vec![raster_miss; raster_lookups.max(1)],
        ));
    }
    if sel.wants(ids[2]) {
        rows.push(row_of(
            ids[2],
            "textcache-missar vid rullning, modulens dl_text-mätning",
            "SPEC-13.2.3",
            "%",
            Some(MAX_TEXT_MISS_PERCENT),
            vec![measure_miss; measure_lookups.max(1)],
        ));
    }
    Ok(rows)
}

/// A tab process: spawn, load `wasm`, tick, first display list back.
fn tab_cold_start(exe: &Path, wasm: &[u8], runs: usize) -> Result<Row> {
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let (sink, inbox) = mpsc::channel();
        let t = Instant::now();
        let conn = spawn_process(exe, 1, channel_sink(sink), None, None)?;
        let request = LoadRequest {
            sha256: vy_runtime::sha256(wasm),
            wasm: wasm.to_vec(),
            memory_max_pages: 64,
            capabilities: 0,
            resources: Vec::new(),
            storage: Vec::new(),
            storage_quota_bytes: 1 << 20,
            net_origins: Vec::new(),
            random: vy_runtime::RandomSource::default(),
            width: 1024,
            height: 640,
            scale: 1.0,
            clock_ms: 0,
            clipboard: None,
        };
        conn.send(&ToTab::Load(Box::new(request)));
        conn.send(&ToTab::Tick { clock_ms: 16 });
        loop {
            match inbox.recv_timeout(Duration::from_secs(60)) {
                Ok((_, Inbound::Message(ToShell::Frame { .. }))) => break,
                Ok((
                    _,
                    Inbound::Message(ToShell::Loaded {
                        ok: false, error, ..
                    }),
                )) => {
                    return Err(anyhow!("tab: {error}"));
                }
                Ok((_, Inbound::Message(_))) => {}
                Ok((_, Inbound::Gone { error })) => {
                    return Err(anyhow!("tab gone: {error:?}"));
                }
                Err(_) => return Err(anyhow!("tab silent")),
            }
        }
        samples.push(t.elapsed());
        conn.shutdown();
    }
    Ok(time_row(
        "tab.cold-start",
        "flikprocess: start, laddning, första display list (hello)",
        "SPEC-13.1.1",
        Some(150.0),
        samples,
    ))
}

/// Enter to the first composited frame, headless, for a local file.
pub(crate) fn first_frame(exe: &Path, root: &Path, name: &str, runs: usize) -> Result<Row> {
    let source = vy_shell::page::Source::File {
        path: root.join(format!("target/wasm/{name}.wasm")),
        draft: Some(root.join(format!("conformance/manifests/{name}.json"))),
    };
    let options = vy_shell::headless::Options {
        width: 1024.0,
        height: 720.0,
        scale: 1.0,
        max_ticks: 1,
        exe: Some(exe.to_path_buf()),
        timeout: Duration::from_secs(60),
    };
    let samples = (0..runs)
        .map(|_| {
            let t = Instant::now();
            let r = vy_shell::headless::screenshot(source.clone(), &options);
            let d = t.elapsed();
            r.map(|_| d).map_err(|e| anyhow!("{e}"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(time_row(
        format!("first-frame.{name}"),
        format!("Enter till första bild, huvudlöst ({name}, lokal fil)"),
        "SPEC-13.1.2",
        Some(100.0),
        samples,
    ))
}

/// SPEC-13.2.2: a pointer event into a running tab, its frame back and
/// rasterized into the composited image. The dashboard redraws on every
/// hover change (the tooltip over the bars).
pub(crate) fn input_to_pixel(exe: &Path, root: &Path, runs: usize) -> Result<Row> {
    let mut shell =
        vy_shell::headless::headless_shell(Some(exe.to_path_buf()), 1280.0, 900.0, 1.0)?;
    let tab = shell.open_tab(Some(vy_shell::page::Source::File {
        path: root.join("target/wasm/dashboard.wasm"),
        draft: Some(root.join("conformance/manifests/dashboard.json")),
    }));
    vy_shell::headless::settle(&mut shell, 1, Duration::from_secs(60))
        .map_err(|e| anyhow!("{e}"))?;
    let page = shell.layout().page;
    let mut samples = Vec::with_capacity(runs);
    for i in 0..runs {
        let frames = shell.tab_frames(tab);
        let t = Instant::now();
        // Alternating between the line chart and the Wednesday bar, so
        // every move changes the hover state (the conformance case's points).
        let (x, y) = if i.is_multiple_of(2) {
            (521.0, 400.0)
        } else {
            (973.0, 526.0)
        };
        shell.pointer_move(page.x + x, page.y + y);
        shell.tick();
        let deadline = Instant::now() + Duration::from_secs(10);
        while shell.tab_frames(tab) == frames {
            if Instant::now() > deadline {
                return Err(anyhow!("no frame after the pointer event"));
            }
            if let Ok(m) = shell.inbox().recv_timeout(Duration::from_millis(50)) {
                shell.handle(m);
            }
        }
        let _ = shell.frame();
        samples.push(t.elapsed());
    }
    shell.shutdown();
    Ok(time_row(
        "input.dashboard",
        "pekarhändelse till pixel (dashboard)",
        "SPEC-13.2.2",
        Some(16.0),
        samples,
    ))
}

/// Run every selected measurement.
pub fn run(root: &Path, sel: &Selection<'_>) -> Result<Vec<Row>> {
    let mods = modules(root)?;
    let mut rows = compile_times(&mods, sel);
    rows.extend(raster_times(sel)?);
    rows.extend(page_raster_times(root, sel)?);
    rows.extend(scroll_sequence(root, sel)?);
    let shell_rows = [
        "tab.cold-start",
        "first-frame.hello",
        "first-frame.text",
        "input.dashboard",
    ];
    if shell_rows.iter().any(|id| sel.wants(id)) {
        let exe = conformance::tab_executable()?;
        let hello = mods
            .iter()
            .find(|(name, _)| name == "hello")
            .map(|(_, w)| w.clone())
            .ok_or_else(|| anyhow!("no hello module"))?;
        if sel.wants(shell_rows[0]) {
            rows.push(tab_cold_start(&exe, &hello, sel.runs)?);
        }
        if sel.wants(shell_rows[1]) {
            rows.push(first_frame(&exe, root, "hello", sel.runs)?);
        }
        if sel.wants(shell_rows[2]) {
            rows.push(first_frame(&exe, root, "text", sel.runs)?);
        }
        if sel.wants(shell_rows[3]) {
            rows.push(input_to_pixel(&exe, root, sel.runs)?);
        }
    }
    Ok(rows)
}

/// Largest slowdown of a row's P50 over the baseline before it counts as
/// a regression (SPEC-12.2.3: 5 %).
pub const REGRESSION_TOLERANCE: f64 = 0.05;
/// Added to the tolerance for rows in milliseconds: a row well under a
/// millisecond moves by more than 5 % with the scheduler alone.
pub const NOISE_FLOOR_MS: f64 = 0.5;
/// Added to the tolerance for rows in percent (the text cache's misses),
/// in percentage points.
pub const NOISE_FLOOR_PERCENT: f64 = 1.0;

/// A row slower than the baseline allows.
#[derive(Clone, Debug, PartialEq)]
pub struct Regression {
    /// The row.
    pub id: String,
    /// The baseline's P50.
    pub baseline: f64,
    /// The P50 now.
    pub now: f64,
    /// The largest P50 the gate accepts.
    pub limit: f64,
    /// `ms` or `%`.
    pub unit: String,
}

impl std::fmt::Display for Regression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: P50 {:.2} {} against {:.2} in the baseline (at most {:.2}, SPEC-12.2.3)",
            self.id, self.now, self.unit, self.baseline, self.limit
        )
    }
}

/// SPEC-12.2.3: the rows of `now` whose P50 exceeds the baseline's by more
/// than [`REGRESSION_TOLERANCE`] plus the noise floor of their unit. P50,
/// not P95: on a shared CI runner the tail is the neighbours'. Rows only
/// one report has are not compared. Both reports must come from release
/// builds; the baseline is for one runner class and changes only in a
/// `bench:` commit.
pub fn compare(baseline: &Report, now: &Report) -> Result<Vec<Regression>> {
    for (what, r) in [("baseline", baseline), ("report", now)] {
        if r.profile != "release" {
            return Err(anyhow!(
                "the {what} is from a {} build; compare release builds",
                r.profile
            ));
        }
    }
    let mut out = Vec::new();
    for row in &now.rows {
        let Some(base) = baseline.row(&row.id) else {
            continue;
        };
        let floor = if row.unit == "%" {
            NOISE_FLOOR_PERCENT
        } else {
            NOISE_FLOOR_MS
        };
        let limit = base.p50 * (1.0 + REGRESSION_TOLERANCE) + floor;
        if row.p50 > limit {
            out.push(Regression {
                id: row.id.clone(),
                baseline: base.p50,
                now: row.p50,
                limit,
                unit: row.unit.clone(),
            });
        }
    }
    Ok(out)
}

/// The table as text.
pub fn table(rows: &[Row]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<64} {:<12} {:>9} {:>9} {:>9}  {}\n",
        "mätning", "spec", "p50", "p95", "budget", "n"
    ));
    for r in rows {
        let budget = r
            .budget
            .map(|b| format!("{b:.2} {}", r.unit))
            .unwrap_or_else(|| "-".into());
        let mark = if r.within() { "" } else { "  ÖVER" };
        out.push_str(&format!(
            "{:<64} {:<12} {:>9.2} {:>9.2} {:>9}  {}{mark}\n",
            r.what, r.spec, r.p50, r.p95, budget, r.n
        ));
    }
    out
}

/// Repository root for the bench: the first ancestor holding `conformance/cases.toml`.
pub fn root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join(conformance::CASES_FILE).is_file() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err(anyhow!(
                "not inside the repository (no conformance/cases.toml)"
            ));
        }
    }
}
