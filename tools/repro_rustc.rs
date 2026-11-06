//! A rustc wrapper for the module builds (tools/build_modules.sh), so that the
//! same commit gives the same `.wasm` bytes in any checkout (SPEC-9.7.1).
//!
//! Every module is a cargo workspace of its own, which makes `crates/vy-sdk`
//! and `crates/vy-abi` path dependencies outside the workspace, and for those
//! cargo hashes the absolute path of the package into `-C metadata`. That value
//! is the crate's StableCrateId: it is part of every symbol name, and rustc
//! orders the SDK's generic functions instantiated in a module by symbol name,
//! so two checkouts gave two orders of the same functions. This wrapper
//! replaces cargo's `-C metadata=<hash>` with a value made only of what does
//! not depend on where the checkout is: the crate name, the package version,
//! the crate types, the target and the `--cfg` values (features). Nothing else
//! is changed; `-C extra-filename`, which only names files in `target/`, stays
//! as cargo made it.
//!
//! tools/build_modules.sh compiles this file with plain rustc (std only) into
//! `target/tools/` and sets it as `RUSTC_WRAPPER`; cargo then runs
//! `repro-rustc <rustc> <args...>`.

use std::env;
use std::ffi::OsString;
use std::process::{exit, Command};

fn main() {
    let mut argv = env::args_os().skip(1);
    let Some(rustc) = argv.next() else {
        eprintln!("usage: repro-rustc <rustc> [args...]");
        exit(2);
    };
    let mut args: Vec<OsString> = argv.collect();
    if let Some(metadata) = stable_metadata(&args) {
        for i in 1..args.len() {
            let replace = args[i - 1] == "-C"
                && args[i]
                    .to_str()
                    .is_some_and(|a| a.starts_with("metadata="));
            if replace {
                args[i] = format!("metadata={metadata}").into();
            }
        }
    }
    match Command::new(&rustc).args(&args).status() {
        Ok(status) => exit(status.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("repro-rustc: cannot run {}: {e}", rustc.to_string_lossy());
            exit(1);
        }
    }
}

/// The replacement for `-C metadata`, or `None` when this is not a crate
/// compilation (`rustc -vV`, cargo's target queries) or an argument is not
/// UTF-8.
fn stable_metadata(args: &[OsString]) -> Option<String> {
    let args: Vec<&str> = args.iter().map(|a| a.to_str()).collect::<Option<_>>()?;
    let mut name = None;
    let mut types = Vec::new();
    let mut target = "";
    let mut cfgs = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "--crate-name" => name = it.next().copied(),
            "--crate-type" => types.extend(it.next().copied()),
            "--target" => target = it.next().copied().unwrap_or(""),
            "--cfg" => cfgs.extend(it.next().copied()),
            _ => {}
        }
    }
    let name = name?;
    types.sort_unstable();
    cfgs.sort_unstable();
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    Some(format!(
        "vy-repro:{name}:{version}:{}:{target}:{}",
        types.join(","),
        cfgs.join(",")
    ))
}
