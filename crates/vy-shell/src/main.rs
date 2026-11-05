//! `vy-shell`: the windowed client, its tab processes and its headless
//! screenshot mode.
//!
//! ```text
//! vy-shell [<vy-address | file.vyp | file.wasm>]          open a window
//! vy-shell --screenshot <png> --size WxH [--scale S] [--manifest <draft.json>] <address or file>
//! vy-shell --tab [--record <file>] [--replay <file>]      a tab process (started by the shell)
//! ```
//!
//! A release build on Windows is a GUI program (no console window on a
//! double-click); its tab processes talk over pipes, which need no console.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  \
         vy-shell [<vy://address | page.vyp | module.wasm>]\n  \
         vy-shell --screenshot <out.png> --size WxH [--scale S] [--manifest <draft.json>] <address or file>\n  \
         vy-shell --tab [--record <file>] [--replay <file>]   (started by the shell itself)"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--tab") {
        let mut record = None;
        let mut replay = None;
        let mut it = args[1..].iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--record" => record = it.next().map(PathBuf::from),
                "--replay" => replay = it.next().map(PathBuf::from),
                _ => return usage(),
            }
        }
        return match vy_shell::tab::main(record.as_deref(), replay.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("vy-shell --tab: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if args.iter().any(|a| a == "--screenshot") {
        return match screenshot(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("vy-shell --screenshot: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if args.iter().any(|a| a.starts_with("--")) {
        return usage();
    }
    let mut sources = Vec::new();
    for a in &args {
        match vy_shell::page::Source::parse(a) {
            Ok(s) => sources.push(s),
            Err(e) => {
                eprintln!("vy-shell: {e}");
                return ExitCode::from(2);
            }
        }
    }
    match window(sources) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("vy-shell: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(feature = "window")]
fn window(sources: Vec<vy_shell::page::Source>) -> Result<(), String> {
    vy_shell::window::run(sources)
}

#[cfg(not(feature = "window"))]
fn window(_sources: Vec<vy_shell::page::Source>) -> Result<(), String> {
    Err("built without the window feature; use --screenshot".into())
}

/// `--screenshot <png> --size WxH [--scale S] [--ticks N] [--manifest <draft>] <source>`.
fn screenshot(args: &[String]) -> Result<(), String> {
    let mut out = None;
    let mut size = None;
    let mut scale = 1.0f32;
    let mut ticks = 1u32;
    let mut manifest = None;
    let mut source = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| -> Result<String, String> {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "--screenshot" => out = Some(PathBuf::from(value("--screenshot")?)),
            "--size" => {
                let v = value("--size")?;
                let (w, h) = v
                    .split_once('x')
                    .ok_or_else(|| format!("--size must be WxH, got {v:?}"))?;
                let w: u32 = w.parse().map_err(|_| format!("bad width in {v:?}"))?;
                let h: u32 = h.parse().map_err(|_| format!("bad height in {v:?}"))?;
                size = Some((w, h));
            }
            "--scale" => {
                scale = value("--scale")?
                    .parse()
                    .map_err(|_| "--scale must be a number".to_string())?;
            }
            "--ticks" => {
                ticks = value("--ticks")?
                    .parse()
                    .map_err(|_| "--ticks must be a number".to_string())?;
            }
            "--manifest" => manifest = Some(PathBuf::from(value("--manifest")?)),
            other if other.starts_with("--") => return Err(format!("unknown flag {other}")),
            other => source = Some(other.to_string()),
        }
    }
    let (Some(out), Some((w, h)), Some(source)) = (out, size, source) else {
        return Err("--screenshot <png> --size WxH <address or file> are required".into());
    };
    if !(scale.is_finite() && scale > 0.0 && scale <= 8.0) {
        return Err("--scale must be in (0, 8]".into());
    }
    let mut source = vy_shell::page::Source::parse(&source)?;
    if let (vy_shell::page::Source::File { draft, .. }, Some(m)) = (&mut source, manifest) {
        *draft = Some(m);
    }
    let options = vy_shell::headless::Options {
        width: w as f32,
        height: h as f32,
        scale,
        max_ticks: ticks.max(1),
        exe: Some(std::env::current_exe().map_err(|e| e.to_string())?),
        timeout: std::time::Duration::from_secs(60),
    };
    let image = vy_shell::headless::screenshot(source, &options)?;
    let png = image.encode_png().map_err(|e| e.to_string())?;
    std::fs::write(&out, png).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("{}: {}x{} px", out.display(), image.width, image.height);
    Ok(())
}
