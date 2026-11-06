//! The build pipeline of a page, in one step: `cargo build` for wasm32, the
//! ABI check `vy-cli check-abi` runs, the manifest draft against the module
//! and its resources (`vy_publish::build`), the 8 ms frame check
//! `vy-publish build` makes (SPEC-13.4.1, a warning here), and a signed
//! package written whole or not at all. Every stage writes to one log; a
//! failure stops the pipeline and is reported, never raised past it.

use crate::project::Project;
use crate::scaffold::lib_name;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::fmt::Write as _;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;
use vy_manifest::{Package, SigningKey};

/// Most bytes of a build log that are kept; the head goes first, since the
/// errors come last.
pub const MAX_LOG_BYTES: usize = 64 * 1024;
/// The target pages are built for.
pub const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// What one run of the pipeline produced.
#[derive(Debug)]
pub struct BuildOutcome {
    /// The log of every stage, most recent last, at most
    /// [`MAX_LOG_BYTES`].
    pub log: String,
    /// The package, written to the output path, when every stage passed.
    pub package: Option<Package>,
    /// The module cargo produced, when cargo got that far.
    pub wasm_path: Option<PathBuf>,
}

impl BuildOutcome {
    /// True when the package was written.
    pub fn ok(&self) -> bool {
        self.package.is_some()
    }

    /// The line that says why the build failed: the first `error` line of
    /// the log, else its last line.
    pub fn first_error(&self) -> Option<&str> {
        if self.ok() {
            return None;
        }
        self.log
            .lines()
            .find(|l| l.starts_with("error"))
            .or_else(|| self.log.lines().rev().find(|l| !l.trim().is_empty()))
    }
}

/// Cut `log` to its last [`MAX_LOG_BYTES`] bytes on a character boundary.
pub fn trim_log(log: &mut String) {
    if log.len() > MAX_LOG_BYTES {
        let mut cut = log.len() - MAX_LOG_BYTES;
        while !log.is_char_boundary(cut) {
            cut += 1;
        }
        log.replace_range(..cut, "[...]\n");
    }
}

/// The cargo to run: `CARGO` when this process was started by cargo (the
/// same toolchain), else `cargo` on `PATH` (`cargo.exe` on Windows).
pub fn cargo_program() -> PathBuf {
    std::env::var_os("CARGO")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("cargo"))
}

/// Run the pipeline for `project`, signing with `key` and writing the
/// package to `out`. `progress` gets every log line as it is written,
/// cargo's output included, so a caller can show a long first build.
pub fn build_project(
    project: &Project,
    key: &SigningKey,
    out: &Path,
    progress: &mut dyn FnMut(&str),
) -> BuildOutcome {
    let mut log = Log {
        text: String::new(),
        progress,
    };
    let started = Instant::now();
    let mut wasm_path = None;
    let package = match run(project, key, out, &mut log, &mut wasm_path) {
        Ok(p) => {
            log.line(&format!("ok: {:.1} s", started.elapsed().as_secs_f32()));
            Some(p)
        }
        Err(e) => {
            log.line(&format!("error: {e:#}"));
            None
        }
    };
    let mut text = log.text;
    trim_log(&mut text);
    BuildOutcome {
        log: text,
        package,
        wasm_path,
    }
}

struct Log<'a> {
    text: String,
    progress: &'a mut dyn FnMut(&str),
}

impl Log<'_> {
    fn line(&mut self, line: &str) {
        self.text.push_str(line);
        self.text.push('\n');
        (self.progress)(line);
        // Keep memory bounded even for a runaway cargo.
        if self.text.len() > 2 * MAX_LOG_BYTES {
            trim_log(&mut self.text);
        }
    }
}

fn run(
    project: &Project,
    key: &SigningKey,
    out: &Path,
    log: &mut Log<'_>,
    wasm_path: &mut Option<PathBuf>,
) -> Result<Package> {
    // 1. cargo. Diagnostics and progress are rendered on stderr (the log);
    //    stdout carries the JSON messages that name the artifact, so the
    //    module is found wherever the target directory is.
    log.line(&format!(
        "$ cargo build --release --target {WASM_TARGET}   ({})",
        project.dir.display()
    ));
    let mut command = Command::new(cargo_program());
    command
        .args([
            "build",
            "--release",
            "--target",
            WASM_TARGET,
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(&project.dir)
        .env("CARGO_TERM_COLOR", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(dir) = &project.target_dir {
        command.env("CARGO_TARGET_DIR", dir);
    }
    let mut child = command
        .spawn()
        .context("starting cargo (is Rust installed and cargo on PATH?)")?;
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let artifacts = std::thread::spawn(move || {
        BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
            .collect::<Vec<Value>>()
    });
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        log.line(&line);
    }
    let status = child.wait().context("waiting for cargo")?;
    let messages = artifacts.join().unwrap_or_default();
    if !status.success() {
        bail!("cargo build failed ({status})");
    }
    let wasm = find_wasm(&messages, &lib_name(&project.name)).ok_or_else(|| {
        anyhow!("cargo built no .wasm (is crate-type = [\"cdylib\"] in Cargo.toml?)")
    })?;
    *wasm_path = Some(wasm.clone());

    // 2. The ABI check, the same code as `vy-cli check-abi`.
    let module = std::fs::read(&wasm).with_context(|| format!("reading {}", wasm.display()))?;
    log.line(&format!(
        "$ vy-cli check-abi {}   ({} bytes)",
        wasm.display(),
        module.len()
    ));
    let report = vy_runtime::check_abi_bytes(&module).map_err(|e| anyhow!("check-abi: {e}"))?;
    for line in report.to_string().lines() {
        log.line(line);
    }
    if !report.ok() {
        bail!("check-abi found {} problem(s)", report.problems.len());
    }

    // 3. The draft against the module and its resources, signed.
    log.line("$ vy-publish build --manifest manifest.json");
    let draft = project.draft()?;
    let resources = vy_publish::load_resources(&project.manifest_path(), &draft, &[])
        .map_err(|e| anyhow!("{e}"))?;
    let package = vy_publish::build(&draft, module, resources, key).map_err(|e| anyhow!("{e}"))?;

    // 4. The 8 ms frame check `vy-publish build` makes (SPEC-13.4.1).
    frame_check(&package, log);

    // 5. The package, written whole or not at all.
    let bytes = package.pack().map_err(|e| anyhow!("packing: {e}"))?;
    write_atomically(out, &bytes)?;
    log.line(&format!(
        "package: {} bytes, {} resources, identity {}",
        bytes.len(),
        package.resources.len(),
        package.identity()
    ));
    Ok(package)
}

/// Run the publish-time frame check (`vy_publish::check_frames`,
/// SPEC-13.4.1) and log its verdict. A frame over the 8 ms budget, or a
/// module the check cannot run, is a warning and not a failure: the dev
/// build is served on loopback only and the page must keep running while
/// it is fixed, but `vy-publish build` refuses such a module, so the agent
/// in Studio is told now.
fn frame_check(package: &Package, log: &mut Log<'_>) {
    let ms = |fuel: u64| fuel as f64 / vy_runtime::FUEL_PER_MS as f64;
    let budget_ms = vy_runtime::FRAME_BUDGET_FUEL / vy_runtime::FUEL_PER_MS;
    log.line(&format!(
        "$ vy-publish build: frame check, {budget_ms} ms per frame (SPEC-13.4.1)"
    ));
    match vy_publish::check_frames(package) {
        Ok(costs) => {
            let worst = costs.iter().map(|c| c.fuel).max().unwrap_or(0);
            log.line(&format!(
                "frames: {} run headless, the costliest {:.1} ms of logical time ({worst} fuel)",
                costs.len(),
                ms(worst)
            ));
        }
        Err(vy_publish::BuildError::OverBudget { frame, fuel, .. }) => log.line(&format!(
            "warning: {frame} used {:.1} ms of logical time ({fuel} fuel), over the {budget_ms} ms \
             frame budget (SPEC-13.4.1); served here, but vy-publish build refuses it without \
             --allow-slow",
            ms(fuel)
        )),
        Err(e) => log.line(&format!(
            "warning: the frame check (SPEC-13.4.1) could not run the module: {e}"
        )),
    }
}

/// Write `bytes` to `path` through a temporary file next to it and a
/// rename, so a reader sees the old file or the new one.
pub fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("renaming to {}", path.display()))
}

/// The `.wasm` of the `cdylib` named `lib` among cargo's JSON messages,
/// else of any `cdylib`.
pub fn find_wasm(messages: &[Value], lib: &str) -> Option<PathBuf> {
    let wasm_of = |m: &Value| -> Option<PathBuf> {
        if m["reason"] != "compiler-artifact" {
            return None;
        }
        let kinds = m["target"]["kind"].as_array()?;
        if !kinds.iter().any(|k| k == "cdylib") {
            return None;
        }
        m["filenames"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .find(|f| f.ends_with(".wasm"))
            .map(PathBuf::from)
    };
    messages
        .iter()
        .filter(|m| m["target"]["name"] == lib)
        .find_map(wasm_of)
        .or_else(|| messages.iter().find_map(wasm_of))
}

/// The summary line of a log for a person: what the outcome was, with the
/// reason when it failed.
pub fn summary(outcome: &BuildOutcome) -> String {
    let mut s = String::new();
    match outcome.first_error() {
        None => {
            let _ = write!(s, "bygget gick igenom");
        }
        Some(e) => {
            let _ = write!(s, "bygget misslyckades: {e}");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn log_keeps_the_tail() {
        let mut log = "x".repeat(MAX_LOG_BYTES + 10);
        log.push_str("end");
        trim_log(&mut log);
        assert!(log.starts_with("[...]\n"));
        assert!(log.ends_with("end"));
        assert!(log.len() <= MAX_LOG_BYTES + 6);
        let mut short = String::from("short");
        trim_log(&mut short);
        assert_eq!(short, "short");
        let mut multibyte = "å".repeat(MAX_LOG_BYTES);
        trim_log(&mut multibyte);
        assert!(multibyte.starts_with("[...]\n"));
    }

    #[test]
    fn finds_the_module_among_cargo_messages() {
        let messages = vec![
            json!({"reason": "compiler-artifact", "target": {"name": "vy_sdk", "kind": ["lib"]},
                   "filenames": ["/t/libvy_sdk.rlib"]}),
            json!({"reason": "compiler-artifact", "target": {"name": "other", "kind": ["cdylib"]},
                   "filenames": ["/t/other.wasm"]}),
            json!({"reason": "compiler-artifact", "target": {"name": "min_sida", "kind": ["cdylib"]},
                   "filenames": ["/t/min_sida.d", "/t/min_sida.wasm"]}),
            json!({"reason": "build-finished", "success": true}),
        ];
        assert_eq!(
            find_wasm(&messages, "min_sida"),
            Some(PathBuf::from("/t/min_sida.wasm"))
        );
        assert_eq!(
            find_wasm(&messages, "nope"),
            Some(PathBuf::from("/t/other.wasm"))
        );
        assert_eq!(find_wasm(&messages[..1], "vy_sdk"), None);
    }

    #[test]
    fn writes_atomically() {
        let dir = std::env::temp_dir().join(format!("vy-dev-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("a/b.vyp");
        write_atomically(&path, b"one").unwrap();
        write_atomically(&path, b"two").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(dir.join("a")).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
