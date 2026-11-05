//! `vy-dev`: scaffold a page project.

use anyhow::{anyhow, bail, Result};
use std::path::PathBuf;
use std::process::ExitCode;
use vy_dev::scaffold::{derive_name, Scaffold};

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  \
         vy-dev new <dir> [--name <name>] [--sdk <vy-checkout>]"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("new") => cmd_new(&args[1..]),
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

/// Minimal `--flag value` parsing, as in `vy-cli`.
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
}

fn cmd_new(args: &[String]) -> Result<ExitCode> {
    let a = Args::parse(args, &["name", "sdk"], &[])?;
    let Some(dir) = a.positional.first() else {
        return Ok(usage());
    };
    let dir = PathBuf::from(dir);
    let name = match a.value("name") {
        Some(n) => n.to_string(),
        None => derive_name(
            &std::path::absolute(&dir)?
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ),
    };
    let scaffold = Scaffold {
        dir: dir.clone(),
        name: name.clone(),
        sdk: a.value("sdk").map(PathBuf::from),
    };
    for f in scaffold.create()? {
        println!("wrote {}", f.display());
    }
    println!();
    println!("Next:");
    println!("  cd {}", dir.display());
    println!("  cargo build --release --target wasm32-unknown-unknown");
    println!(
        "  vy-cli render target/wasm32-unknown-unknown/release/{}.wasm --out preview.png --size 800x600",
        vy_dev::scaffold::lib_name(&name)
    );
    println!("CLAUDE.md in the project has the whole loop.");
    Ok(ExitCode::SUCCESS)
}
