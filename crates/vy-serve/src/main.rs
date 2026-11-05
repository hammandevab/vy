//! `vy-serve`: serve a directory of `.vyp` packages.
//!
//! Flags override environment: `--dir` (`VY_DIR`, default `.`), `--port`
//! (`PORT`, default 8080; Railway sets `PORT`), `--bind` (`VY_BIND`,
//! default `0.0.0.0`), `--host-name` (`VY_HOST_NAME`, then Railway's
//! `RAILWAY_PUBLIC_DOMAIN`, the host in `vy://` links, default
//! `localhost:<port>`), `--public-key` (`VY_PUBLIC_KEY`, a `vy-key` file
//! served at `/.well-known/vy-key`). At startup it prints the DNS TXT
//! records a client's first visit needs (SPEC-7.4.5), which are also
//! served at `/.well-known/vy-dns`.

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
            .clone()
            .unwrap_or_else(|| format!("localhost:{}", handle.port()))
    );
    for p in packages {
        eprintln!("  /{p}");
    }
    // The DNS half of the trust chain (SPEC-7.4.2, SPEC-7.4.5) is the
    // operator's to publish; without it every first visit is restricted.
    let host = config
        .host_name
        .clone()
        .unwrap_or_else(|| format!("localhost:{}", handle.port()));
    let lines = handle.site().dns_zone_lines(&host);
    if lines.is_empty() {
        eprintln!("no --public-key: every page runs in restricted mode (SPEC-7.4.3)");
    } else if vy_serve::is_loopback_host(&host) {
        eprintln!("DNS: none needed for {host}, loopback has no DNS (SPEC-7.4.5)");
    } else {
        eprintln!(
            "DNS TXT record(s) to publish at the DNS provider for {host} (SPEC-7.4.5; also at /.well-known/vy-dns);"
        );
        eprintln!("without them a client's first visit runs every page in restricted mode:");
        for l in lines {
            eprintln!("  {l}");
        }
        if host.ends_with(".up.railway.app") {
            eprintln!(
                "note: {host} is a platform subdomain whose DNS you cannot change; serve the site \
                 under a domain you control (VY_HOST_NAME) and publish the record there"
            );
        }
    }
    // The key log (SPEC-7.6.1) is optional in 1.0 (SPEC-7.6.3); a broken
    // one is served as is, and clients will refuse it (SPEC-7.6.2).
    match handle.site().key_log() {
        None => eprintln!(
            "no {} in the site directory: no key log is served (SPEC-7.6.1)",
            vy_serve::LOG_FILE_NAME
        ),
        Some(Ok(log)) => eprintln!(
            "key log: {} entr{} at {}, head {}",
            log.entries.len(),
            if log.entries.len() == 1 { "y" } else { "ies" },
            vy_serve::WELL_KNOWN_LOG_PATH,
            vy_manifest::hex(&log.head())
        ),
        Some(Err(e)) => eprintln!(
            "warning: {}: {e}; clients will refuse it",
            vy_serve::LOG_FILE_NAME
        ),
    }
    // The workers run until the process is stopped.
    loop {
        std::thread::park();
    }
}
