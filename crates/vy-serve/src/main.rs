//! `vy-serve`: serve a directory of `.vyp` packages.
//!
//! Flags override environment: `--dir` (`VY_DIR`, default `.`), `--port`
//! (`PORT`, default 8080; Railway sets `PORT`), `--bind` (`VY_BIND`,
//! default `0.0.0.0`), `--host-name` (`VY_HOST_NAME`, then Railway's
//! `RAILWAY_PUBLIC_DOMAIN`, the host in `vy://` links, default
//! `localhost:<port>`), `--public-key` (`VY_PUBLIC_KEY`, a `vy-key` file
//! served at `/.well-known/vy-key`).

use std::path::PathBuf;
use std::process::ExitCode;
use vy_serve::{start, Config};

fn usage() -> ExitCode {
    eprintln!(
        "usage: vy-serve [--dir <dir>] [--port <n>] [--bind <addr>] [--host-name <host[:port]>] [--public-key <dev.key.pub>] [--quiet]"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    let mut dir = env("VY_DIR").unwrap_or_else(|| ".".into());
    let mut port = env("PORT").unwrap_or_else(|| "8080".into());
    let mut bind = env("VY_BIND").unwrap_or_else(|| "0.0.0.0".into());
    let mut host_name = env("VY_HOST_NAME").or_else(|| env("RAILWAY_PUBLIC_DOMAIN"));
    let mut public_key = env("VY_PUBLIC_KEY");
    let mut quiet = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = || it.next().cloned();
        match a.as_str() {
            "--dir" => dir = value().unwrap_or_default(),
            "--port" => port = value().unwrap_or_default(),
            "--bind" => bind = value().unwrap_or_default(),
            "--host-name" => host_name = value(),
            "--public-key" => public_key = value(),
            "--quiet" => quiet = true,
            _ => return usage(),
        }
    }
    let Ok(port) = port.parse::<u16>() else {
        eprintln!("error: port {port:?} is not a number");
        return usage();
    };
    let config = Config {
        dir: PathBuf::from(dir),
        bind,
        port,
        host_name,
        public_key_file: public_key.map(PathBuf::from),
    };
    let handle = match start(config.clone(), !quiet) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let packages = handle.site().packages();
    eprintln!(
        "vy-serve: {} package(s) from {} on {}:{} as vy://{}",
        packages.len(),
        config.dir.display(),
        config.bind,
        handle.port(),
        config
            .host_name
            .unwrap_or_else(|| format!("localhost:{}", handle.port()))
    );
    for p in packages {
        eprintln!("  /{p}");
    }
    // The workers run until the process is stopped.
    loop {
        std::thread::park();
    }
}
