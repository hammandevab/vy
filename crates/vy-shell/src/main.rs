//! `vy-shell`: the windowed client, its tab processes and its headless
//! screenshot mode.
//!
//! ```text
//! vy-shell [<vy-address | file.vyp | file.wasm>]          open a window
//! vy-shell --screenshot <png> --size WxH [--scale S] [--manifest <draft.json>] <address or file>
//! vy-shell --tab [--record <file>] [--replay <file>]      a tab process (started by the shell)
//! ```

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
    usage()
}
