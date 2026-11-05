//! `vy-cli`: headless tools for the Vy client.
//!
//! M0 shipped `check-abi`; M1 adds `render` and `conformance`; `bench` arrives with M3.

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  \
         vy-cli check-abi <module.wasm>\n  \
         vy-cli render <file.wasm> --out <png> --size WxH [--scale S] [--frames MAX] [--grant cap,..]\n           \
         [--resource <file> [--resource-size WxH]].. [--dump-semantic <json>]\n  \
         vy-cli conformance [--update] [--root <dir>]\n  \
         vy-cli abi-dump      (the frozen ABI 1.0 listing, docs/ABI-1.0.md)\n  \
         vy-cli bench         (M3)"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("check-abi") => check_abi(&args[1..]),
        Some("render") => render(&args[1..]),
        Some("conformance") => conformance(&args[1..]),
        Some("abi-dump") => {
            print!("{}", vy_cli::abi_doc::abi_listing());
            return ExitCode::SUCCESS;
        }
        Some("bench") => {
            eprintln!("bench: not implemented before M3 (see CLAUDE.md)");
            return ExitCode::from(2);
        }
        _ => return usage(),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Minimal `--flag value` parsing: positionals and a map of flags.
struct Args {
    positional: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    fn parse(args: &[String], value_flags: &[&str], bool_flags: &[&str]) -> Result<Self> {
        let mut positional = Vec::new();
        let mut flags = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--") {
                if value_flags.contains(&name) {
                    let v = it.next().ok_or_else(|| anyhow!("--{name} needs a value"))?;
                    flags.push((name.to_string(), Some(v.clone())));
                } else if bool_flags.contains(&name) {
                    flags.push((name.to_string(), None));
                } else {
                    bail!("unknown flag --{name}");
                }
            } else {
                positional.push(a.clone());
            }
        }
        Ok(Self { positional, flags })
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_deref())
    }

    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == name)
    }
}

fn check_abi(args: &[String]) -> Result<ExitCode> {
    let Some(path) = args.first() else {
        return Ok(usage());
    };
    let report = vy_cli::check_abi(path)?;
    print!("{report}");
    Ok(if report.ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn render(args: &[String]) -> Result<ExitCode> {
    let a = Args::parse(
        args,
        &[
            "out",
            "size",
            "scale",
            "frames",
            "grant",
            "dump-semantic",
            "resource",
            "resource-size",
        ],
        &[],
    )?;
    let (Some(module), Some(out), Some(size)) =
        (a.positional.first(), a.value("out"), a.value("size"))
    else {
        return Ok(usage());
    };
    let (width, height) = vy_cli::parse_size(size)?;
    let scale: f32 = a.value("scale").unwrap_or("1").parse()?;
    let frames: u32 = a.value("frames").unwrap_or("1").parse()?;
    let names: Vec<&str> = a
        .value("grant")
        .map(|v| {
            v.split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let capabilities = vy_runtime::Capabilities::from_names(names)
        .map_err(|n| anyhow!("--grant: unknown capability {n:?} (SPEC-8.2.1)"))?;
    // `--resource <file>` in order gives resource ids 1, 2, .. (SPEC-4.5.1);
    // `--resource-size WxH` states the manifest's size for the one before it
    // (SPEC-8.1.2).
    let mut resources: Vec<vy_cli::Resource> = Vec::new();
    for (name, value) in &a.flags {
        let value = value.as_deref().unwrap_or_default();
        match name.as_str() {
            "resource" => {
                let bytes =
                    std::fs::read(value).with_context(|| format!("reading resource {value}"))?;
                resources.push(vy_cli::Resource::new(bytes));
            }
            "resource-size" => {
                let last = resources
                    .last_mut()
                    .ok_or_else(|| anyhow!("--resource-size must follow a --resource"))?;
                last.pixels = Some(vy_cli::parse_size(value)?);
            }
            _ => {}
        }
    }
    let settings = vy_cli::FileRender {
        width,
        height,
        scale,
        frames,
        capabilities,
        resources,
        dump_semantic: a.value("dump-semantic").map(PathBuf::from),
    };
    let output = vy_cli::render_to_files(module, Path::new(out), &settings)?;
    for log in &output.logs {
        eprintln!("[{:?} {}] {}", log.source, log.level, log.message);
    }
    println!(
        "{out}: {}x{} px, {} display list bytes, {} of at most {frames} frame(s) run",
        output.image.width,
        output.image.height,
        output.display_list.len(),
        output.frames_run
    );
    Ok(ExitCode::SUCCESS)
}

/// Repository root: `--root`, else the first ancestor of the working directory
/// holding `conformance/cases.toml`.
fn find_root(explicit: Option<&str>) -> Result<PathBuf> {
    if let Some(r) = explicit {
        return Ok(PathBuf::from(r));
    }
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join(vy_cli::conformance::CASES_FILE).is_file() {
            return Ok(dir);
        }
        if !dir.pop() {
            bail!(
                "{} not found in the working directory or its parents; pass --root",
                vy_cli::conformance::CASES_FILE
            );
        }
    }
}

fn conformance(args: &[String]) -> Result<ExitCode> {
    let a = Args::parse(args, &["root"], &["update"])?;
    let root = find_root(a.value("root"))?;
    let results = vy_cli::conformance::run(&root, a.has("update"))?;
    let mut failed = 0;
    for r in &results {
        let diff = r
            .diff
            .map(|d| format!("{:.3} %", d * 100.0))
            .unwrap_or_else(|| "-".into());
        let sem = match r.semantic_ok {
            Some(true) => " semantic ok",
            Some(false) => " semantic DIFFERS",
            None => "",
        };
        match &r.problem {
            None => println!("ok    {:<24} diff {diff}{sem}", r.name),
            Some(p) => {
                failed += 1;
                println!("FAIL  {:<24} {p}", r.name);
            }
        }
    }
    if a.has("update") {
        println!("{} golden image(s) written", results.len());
    }
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        println!("{failed} of {} case(s) failed (SPEC-12.2.2)", results.len());
        ExitCode::FAILURE
    })
}
