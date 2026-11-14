//! SPEC-13.1.1, SPEC-12.2.3: how long the client's processes take to
//! start, on real processes, and two builds of the client against each
//! other (granskning 0012:7: the cold start with AT-SPI in the Linux
//! client had not been measured).
//!
//! Three starts of a `vy-shell` binary:
//!
//! - **tab**: `vy-shell --tab`, spawned as the shell spawns it (and locked
//!   down as its tabs are), loaded with `hello` and ticked, until its
//!   first display list is back. The process half of SPEC-13.1.1, and
//!   what `vy-cli bench` calls `tab.cold-start` (there with `vy-cli
//!   --tab`, which has no window code in it).
//! - **shell**: `vy-shell --screenshot` of `hello`, from start to exit:
//!   the main process, its tab process, the first frame, composited and
//!   written as a PNG.
//! - **window** (only when two builds are compared, and only where there
//!   is a display): the windowed client, from start until it says on
//!   stderr how it presents, which it does once the window, its GL
//!   context and the accessibility adapter exist and the window is shown.
//!
//! As a test it starts the client cargo built for it a few times the
//! first two ways and prints the medians: that the starts work, not a
//! budget (a debug build on a shared machine says little about 150 ms).
//! With `VY_COLD_START_A` and `VY_COLD_START_B` naming two `vy-shell`
//! binaries it starts them in turn `VY_COLD_START_RUNS` times (40 unless
//! set), each of the three ways, and prints P50 and P95 of each and B
//! against A, which is how docs/notes/m5-fix-b.md compares the release
//! client without and with `a11y-linux`.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use vy_shell::ipc::{LoadRequest, ToShell, ToTab};
use vy_shell::process::{channel_sink, spawn_process, Inbound};

const TIMEOUT: Duration = Duration::from_secs(60);

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn hello() -> PathBuf {
    let path = root().join("target/wasm/hello.wasm");
    assert!(
        path.is_file(),
        "{}: run tools/build_modules.sh",
        path.display()
    );
    path
}

/// A tab process of `exe`: spawned, loaded with `wasm`, ticked, its first
/// display list back.
fn tab_start(exe: &Path, wasm: &[u8]) -> Duration {
    let (sink, inbox) = mpsc::channel();
    let started = Instant::now();
    let conn = spawn_process(exe, 1, channel_sink(sink), None, None).expect("a tab process");
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
        match inbox.recv_timeout(TIMEOUT) {
            Ok((_, Inbound::Message(ToShell::Frame { .. }))) => break,
            Ok((
                _,
                Inbound::Message(ToShell::Loaded {
                    ok: false, error, ..
                }),
            )) => {
                panic!("the tab did not load hello: {error}")
            }
            Ok((_, Inbound::Message(_))) => {}
            Ok((_, Inbound::Gone { error })) => panic!("the tab process is gone: {error:?}"),
            Err(_) => panic!("the tab process is silent"),
        }
    }
    let took = started.elapsed();
    conn.shutdown();
    took
}

/// `exe --screenshot` of `page`, from start to exit.
fn shell_start(exe: &Path, page: &Path, png: &Path) -> Duration {
    let started = Instant::now();
    let out = Command::new(exe)
        .arg("--screenshot")
        .arg(png)
        .args(["--size", "800x600"])
        .arg(page)
        .output()
        .expect("the client starts");
    let took = started.elapsed();
    assert!(
        out.status.success(),
        "{} --screenshot: {}",
        exe.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    took
}

/// The windowed `exe` with `page`, from start until it reports how it
/// presents; `None` without a display. Its files go under `data`.
fn window_start(exe: &Path, page: &Path, data: &Path) -> Option<Duration> {
    let display = ["DISPLAY", "WAYLAND_DISPLAY"]
        .iter()
        .any(|v| std::env::var_os(v).is_some_and(|d| !d.is_empty()));
    if !display {
        return None;
    }
    let started = Instant::now();
    let mut child = Command::new(exe)
        .arg(page)
        .env("VY_DATA_DIR", data)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the client starts");
    let stderr = child.stderr.take().expect("piped");
    let (said, heard) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.starts_with("vy-shell: presentation:") {
                let _ = said.send(Instant::now());
                break;
            }
        }
    });
    let at = heard.recv_timeout(TIMEOUT).ok();
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    Some(
        at.expect("the windowed client never said how it presents")
            .duration_since(started),
    )
}

/// The starts of `exe`: its tab process, its headless shell and, when
/// `window` and there is a display, its window.
fn starts(exe: &Path, wasm: &[u8], scratch: &Path, window: bool) -> [Option<Duration>; 3] {
    [
        Some(tab_start(exe, wasm)),
        Some(shell_start(exe, &hello(), &scratch.join("shot.png"))),
        window
            .then(|| window_start(exe, &hello(), &scratch.join("data")))
            .flatten(),
    ]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// P50 and P95 of `samples`, in milliseconds.
fn percentiles(samples: &mut [Duration]) -> (f64, f64) {
    samples.sort();
    let n = samples.len();
    (ms(samples[n / 2]), ms(samples[(n * 95 / 100).min(n - 1)]))
}

const KINDS: [&str; 3] = [
    "tab    (spawn, load hello, first display list)",
    "shell  (--screenshot of hello, start to exit)",
    "window (start until the presentation is reported)",
];

/// SPEC-13.1.1, SPEC-12.2.3: the client's processes start, and how long
/// they take is measured where it can be compared. See the file's docs.
#[test]
fn spec_13_1_1_the_clients_processes_start() {
    let wasm = std::fs::read(hello()).unwrap();
    let scratch = std::env::temp_dir().join(format!("vy-cold-start-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    let named = |var: &str| std::env::var_os(var).map(PathBuf::from);
    let builds: Vec<(String, PathBuf)> = match (named("VY_COLD_START_A"), named("VY_COLD_START_B"))
    {
        (Some(a), Some(b)) => vec![("A".into(), a), ("B".into(), b)],
        _ => vec![(
            "this build".into(),
            PathBuf::from(env!("CARGO_BIN_EXE_vy-shell")),
        )],
    };
    let compared = builds.len() == 2;
    let runs: usize = std::env::var("VY_COLD_START_RUNS")
        .ok()
        .and_then(|r| r.parse().ok())
        .unwrap_or(if compared { 40 } else { 3 })
        .max(1);
    for (name, exe) in &builds {
        assert!(exe.is_file(), "{name}: {} is not a file", exe.display());
        eprintln!(
            "cold start: {name} = {} ({} bytes)",
            exe.display(),
            std::fs::metadata(exe).unwrap().len()
        );
    }
    // In turn, and in the other order every second round, so that what
    // the machine is doing meanwhile falls on both alike. One round first
    // that is not counted: the binaries into the page cache.
    let mut samples = vec![[const { Vec::new() }; 3]; builds.len()];
    for round in 0..=runs {
        let mut order: Vec<usize> = (0..builds.len()).collect();
        if round % 2 == 1 {
            order.reverse();
        }
        for b in order {
            let took = starts(&builds[b].1, &wasm, &scratch, compared);
            if round > 0 {
                for (kind, t) in took.into_iter().enumerate() {
                    samples[b][kind].extend(t);
                }
            }
        }
    }
    let mut medians = vec![[None; 3]; builds.len()];
    for (b, (name, _)) in builds.iter().enumerate() {
        for kind in 0..3 {
            let s = &mut samples[b][kind];
            if s.is_empty() {
                if compared {
                    eprintln!(
                        "cold start: {name}: {}: no display, not measured",
                        KINDS[kind]
                    );
                }
                continue;
            }
            let (p50, p95) = percentiles(s);
            medians[b][kind] = Some(p50);
            eprintln!(
                "cold start: {name}: {}: P50 {p50:.1} ms, P95 {p95:.1} ms over {} runs",
                KINDS[kind],
                s.len()
            );
        }
        assert!(
            medians[b][0].is_some() && medians[b][1].is_some(),
            "{name}: the tab process and the headless shell start anywhere"
        );
    }
    if let [a, b] = medians[..] {
        for kind in 0..3 {
            if let (Some(a), Some(b)) = (a[kind], b[kind]) {
                eprintln!(
                    "cold start: B against A: {}: {:+.1} % at the median ({a:.1} -> {b:.1} ms)",
                    KINDS[kind],
                    (b / a - 1.0) * 100.0
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
}
