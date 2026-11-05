//! `vy-cli bench`: what of SPEC §13 can be measured without a window,
//! printed as a table against the budgets. Informational in M3 (no gate
//! yet, SPEC-12.2.3 is the M3 gate once numbers exist on the reference
//! machine); P95 over the iterations, as §13 asks, and the P50 beside it.
//!
//! Measured:
//! - compile time of every reference module (SPEC-13.1.4, `Runtime::load`
//!   with the ABI check and hash);
//! - rasterization of 2 000-op frames at 1280 × 800 (SPEC-13.2.1):
//!   rectangles, text, and both;
//! - a tab process's cold start: spawn `--tab`, load `hello`, first
//!   display list back (the process half of SPEC-13.1.1 and 13.1.2);
//! - Enter to first composited frame headless for `hello` and `text`
//!   from a local file, which is the cache-hit path of SPEC-13.1.2 minus
//!   the window;
//! - input to pixel (SPEC-13.2.2): a pointer event into a running
//!   `shop` tab, its frame back and rasterized.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use vy_runtime::{Options, Runtime};
use vy_shell::ipc::{LoadRequest, ToShell, ToTab};
use vy_shell::process::{channel_sink, spawn_process, Inbound};

/// One measured row.
#[derive(Clone, Debug)]
pub struct Row {
    /// What was measured.
    pub what: String,
    /// The spec sentence.
    pub spec: &'static str,
    /// The budget in milliseconds, when the spec gives one.
    pub budget_ms: Option<f64>,
    /// Median.
    pub p50_ms: f64,
    /// 95th percentile.
    pub p95_ms: f64,
    /// Iterations.
    pub n: usize,
}

impl Row {
    /// True when the P95 is within the budget (or there is none).
    pub fn within(&self) -> bool {
        self.budget_ms.is_none_or(|b| self.p95_ms <= b)
    }
}

fn percentile(samples: &mut [Duration], p: f64) -> f64 {
    samples.sort();
    let i = ((samples.len() as f64 - 1.0) * p).round() as usize;
    samples[i.min(samples.len() - 1)].as_secs_f64() * 1000.0
}

fn row(
    what: String,
    spec: &'static str,
    budget_ms: Option<f64>,
    mut samples: Vec<Duration>,
) -> Row {
    let n = samples.len();
    Row {
        what,
        spec,
        budget_ms,
        p50_ms: percentile(&mut samples, 0.5),
        p95_ms: percentile(&mut samples, 0.95),
        n,
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

/// SPEC-13.1.4: compile (load) time per module.
fn compile_times(mods: &[(String, Vec<u8>)], n: usize) -> Vec<Row> {
    mods.iter()
        .map(|(name, wasm)| {
            let hash = vy_runtime::sha256(wasm);
            let samples = (0..n)
                .map(|_| {
                    let t = Instant::now();
                    let rt = Runtime::load(wasm, &hash, Options::default());
                    let d = t.elapsed();
                    drop(rt);
                    d
                })
                .collect();
            row(
                format!("kompilering {name} ({} KB)", wasm.len() / 1024),
                "SPEC-13.1.4",
                Some(60.0),
                samples,
            )
        })
        .collect()
}

/// SPEC-13.2.1: 2 000-op frames rasterized at 1280 × 800: rectangles
/// alone, text alone (200 runs of 40 characters), and the mix of 1 800
/// rectangles and 200 runs, so the shaping cost shows by itself.
fn raster_times(n: usize) -> Result<Vec<Row>> {
    let mut text = vy_render::TextEngine::new();
    let style = text
        .style(vy_abi::font::SANS, 14.0, 400, 0x2221_1FFF)
        .map_err(|c| anyhow!("style: {c}"))?;
    let list = |rects: u32, runs: u32| -> Result<Vec<u8>> {
        let mut dl = vy_dl::DisplayList::new();
        dl.begin().map_err(|c| anyhow!("dl: {c}"))?;
        for i in 0..rects {
            let (x, y) = ((i % 40) as f32 * 32.0, (i / 40) as f32 * 17.0);
            dl.rect(x, y, 30.0, 15.0, 3.0, 0x3366_9980 + (i % 5) * 0x0010_0000)
                .map_err(|c| anyhow!("rect: {c}"))?;
        }
        for i in 0..runs {
            let (x, y) = ((i % 4) as f32 * 320.0, (i / 4) as f32 * 16.0);
            dl.text(
                x,
                y,
                300.0,
                style,
                "Vy ritar text i värden, aldrig i modulen".as_bytes(),
            )
            .map_err(|c| anyhow!("text: {c}"))?;
        }
        dl.end().map_err(|c| anyhow!("end: {c}"))?;
        Ok(dl.bytes().to_vec())
    };
    let images = vy_render::ImageCache::empty(0);
    let mut rows = Vec::new();
    for (what, rects, runs) in [
        ("rasterisering 2 000 rektanglar 1280×800", 2_000, 0),
        ("rasterisering 200 textrader à 40 tecken 1280×800", 0, 200),
        (
            "rasterisering 1 800 rektanglar + 200 textrader 1280×800",
            1_800,
            200,
        ),
    ] {
        let bytes = list(rects, runs)?;
        let samples = (0..n)
            .map(|_| {
                let t = Instant::now();
                let r = vy_render::render(&bytes, 1280, 800, 1.0, &mut text, &images);
                let d = t.elapsed();
                drop(r);
                d
            })
            .collect();
        rows.push(row(what.into(), "SPEC-13.2.1", Some(2.0), samples));
    }
    Ok(rows)
}

/// A tab process: spawn, load `wasm`, tick, first display list back.
fn tab_cold_start(exe: &Path, wasm: &[u8], n: usize) -> Result<Row> {
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
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
    Ok(row(
        "flikprocess: start, laddning, första display list (hello)".into(),
        "SPEC-13.1.1",
        Some(150.0),
        samples,
    ))
}

/// Enter to the first composited frame, headless, for a local file.
fn first_frame(exe: &Path, root: &Path, name: &str, n: usize) -> Result<Row> {
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
    let samples = (0..n)
        .map(|_| {
            let t = Instant::now();
            let r = vy_shell::headless::screenshot(source.clone(), &options);
            let d = t.elapsed();
            r.map(|_| d).map_err(|e| anyhow!("{e}"))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(row(
        format!("Enter till första bild, huvudlöst ({name}, lokal fil)"),
        "SPEC-13.1.2",
        Some(100.0),
        samples,
    ))
}

/// SPEC-13.2.2: a pointer event into a running tab, its frame back and
/// rasterized into the composited image. The dashboard redraws on every
/// hover change (the tooltip over the bars).
fn input_to_pixel(exe: &Path, root: &Path, n: usize) -> Result<Row> {
    let mut shell =
        vy_shell::headless::headless_shell(Some(exe.to_path_buf()), 1280.0, 900.0, 1.0)?;
    let tab = shell.open_tab(Some(vy_shell::page::Source::File {
        path: root.join("target/wasm/dashboard.wasm"),
        draft: Some(root.join("conformance/manifests/dashboard.json")),
    }));
    vy_shell::headless::settle(&mut shell, 1, Duration::from_secs(60))
        .map_err(|e| anyhow!("{e}"))?;
    let page = shell.layout().page;
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let frames = shell.tab_frames(tab);
        let t = Instant::now();
        // Alternating between the line chart and the Wednesday bar, so
        // every move changes the hover state (the conformance case's points).
        let (x, y) = if i % 2 == 0 {
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
    Ok(row(
        "pekarhändelse till pixel (dashboard)".into(),
        "SPEC-13.2.2",
        Some(16.0),
        samples,
    ))
}

/// Run every measurement with `n` iterations.
pub fn run(root: &Path, n: usize) -> Result<Vec<Row>> {
    let exe = crate::conformance::tab_executable()?;
    let mods = modules(root)?;
    let mut rows = compile_times(&mods, n);
    rows.extend(raster_times(n)?);
    let hello = mods
        .iter()
        .find(|(name, _)| name == "hello")
        .map(|(_, w)| w.clone())
        .ok_or_else(|| anyhow!("no hello module"))?;
    rows.push(tab_cold_start(&exe, &hello, n)?);
    rows.push(first_frame(&exe, root, "hello", n)?);
    rows.push(first_frame(&exe, root, "text", n)?);
    rows.push(input_to_pixel(&exe, root, n)?);
    Ok(rows)
}

/// The table as text.
pub fn table(rows: &[Row]) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{:<58} {:<12} {:>9} {:>9} {:>9}  {}\n",
        "mätning", "spec", "p50 ms", "p95 ms", "budget", "n"
    ));
    for r in rows {
        let budget = r
            .budget_ms
            .map(|b| format!("{b:.0}"))
            .unwrap_or_else(|| "-".into());
        let mark = if r.within() { "" } else { "  ÖVER" };
        out.push_str(&format!(
            "{:<58} {:<12} {:>9.2} {:>9.2} {:>9}  {}{mark}\n",
            r.what, r.spec, r.p50_ms, r.p95_ms, budget, r.n
        ));
    }
    out
}

/// Repository root for the bench: the first ancestor holding `conformance/cases.toml`.
pub fn root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join(crate::conformance::CASES_FILE).is_file() {
            return Ok(dir);
        }
        if !dir.pop() {
            return Err(anyhow!(
                "not inside the repository (no conformance/cases.toml)"
            ));
        }
    }
}
