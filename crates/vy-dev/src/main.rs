//! `vy-dev`: `new`, `build` and `serve` for a page project.

use anyhow::{anyhow, bail, Result};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use vy_dev::scaffold::{derive_name, lib_name, Scaffold};
use vy_dev::serve::{DEFAULT_PORT, DEFAULT_STUDIO_PORT};
use vy_dev::{build_project, paths, Project, ServeConfig};

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  \
         vy-dev new <dir> [--name <name>] [--sdk <vy-checkout>]\n  \
         vy-dev build [<dir>] [--key <dev.key>] [--out <page.vyp>]\n  \
         vy-dev serve [<dir>] [--port {DEFAULT_PORT}] [--studio-port {DEFAULT_STUDIO_PORT}] [--key <dev.key>] [--agent-cmd <cmd>] [--quiet]"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("new") => cmd_new(&args[1..]),
        Some("build") => cmd_build(&args[1..]),
        Some("serve") => cmd_serve(&args[1..]),
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

    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == name)
    }

    fn port(&self, name: &str, default: u16) -> Result<u16> {
        match self.value(name) {
            Some(p) => p
                .parse()
                .map_err(|_| anyhow!("--{name} {p:?} is not a port number")),
            None => Ok(default),
        }
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
    println!(
        "  vy-dev serve      # builds on every change, serves vy://localhost:{DEFAULT_PORT}/{name} and Studio"
    );
    println!("or by hand:");
    println!("  cargo build --release --target wasm32-unknown-unknown");
    println!(
        "  vy-cli render target/wasm32-unknown-unknown/release/{}.wasm --out preview.png --size 800x600",
        lib_name(&name)
    );
    println!("CLAUDE.md in the project has the whole loop.");
    Ok(ExitCode::SUCCESS)
}

fn cmd_build(args: &[String]) -> Result<ExitCode> {
    let a = Args::parse(args, &["key", "out"], &[])?;
    let dir = a.positional.first().map(String::as_str).unwrap_or(".");
    let project = Project::open(Path::new(dir))?;
    let key_path = paths::key_path(a.value("key").map(Path::new))?;
    let (key, created) = paths::load_or_create_key(&key_path)?;
    if created {
        eprintln!("new developer key: {}", key_path.display());
    }
    let out = a
        .value("out")
        .map(PathBuf::from)
        .unwrap_or_else(|| project.scratch_dir().join(format!("{}.vyp", project.name)));
    let outcome = build_project(&project, &key, &out, &mut |line| println!("{line}"));
    if outcome.ok() {
        println!("wrote {}", out.display());
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

fn cmd_serve(args: &[String]) -> Result<ExitCode> {
    let a = Args::parse(
        args,
        &["port", "studio-port", "key", "agent-cmd"],
        &["quiet"],
    )?;
    let dir = a.positional.first().map(String::as_str).unwrap_or(".");
    let config = ServeConfig {
        project_dir: PathBuf::from(dir),
        port: a.port("port", DEFAULT_PORT)?,
        studio_port: a.port("studio-port", DEFAULT_STUDIO_PORT)?,
        key_path: a.value("key").map(PathBuf::from),
        target_dir: None,
        agent_cmd: a.value("agent-cmd").map(String::from),
        log: !a.has("quiet"),
    };
    let handle = vy_dev::start(config)?;
    if handle.key_created {
        eprintln!(
            "new developer key: {} (keep it; its .pub is served at /.well-known/vy-key)",
            handle.key_path.display()
        );
    }
    let project = &handle.dev.project;
    eprintln!("vy-dev: {} in {}", project.name, project.dir.display());
    eprintln!("  page:   {}", handle.address());
    eprintln!("  studio: {}", handle.studio_address());
    eprintln!("  api:    {}", handle.api_url("/_dev/state"));
    eprintln!(
        "  key:    {} ({})",
        handle.key_id,
        handle.key_path.display()
    );
    eprintln!("  listening on 127.0.0.1 only; Ctrl+C stops");
    // The threads run until the process is stopped.
    loop {
        std::thread::park();
    }
}
