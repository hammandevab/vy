//! The release as `.github/workflows/release.yml` and the scripts under
//! `tools/` make it (SPEC-9.7.1, docs/RELEASING.md): what can be checked
//! of them without a runner. Unix only: the scripts are run under bash.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn run(what: &str, command: &mut Command) -> String {
    let out = command.output().unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(
        out.status.success(),
        "{what}: {}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// SPEC-9.7.1 (granskning 0012:1): the flags `tools/repro_env.sh` and
/// `tools/build_release.sh` hand to cargo through the environment hold
/// `/Brepro` and the targets of the path remapping, `/vy`, `/cargo` and
/// `/target`. Git Bash on Windows rewrites such values into paths under
/// its own directory on the way to a native program unless the variable
/// is listed in `MSYS2_ENV_CONV_EXCL`; the script lists the two it sets
/// flags in, after whatever was listed already.
#[test]
fn spec_9_7_1_the_build_flags_are_kept_from_git_bash() {
    let sourced = |listed: Option<&str>| -> Vec<String> {
        let mut bash = Command::new("bash");
        bash.arg("-c")
            .arg(". tools/repro_env.sh && printf '%s\\n%s' \"$MSYS2_ENV_CONV_EXCL\" \"$CARGO_ENCODED_RUSTFLAGS\"")
            .env_remove("MSYS2_ENV_CONV_EXCL")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTFLAGS")
            .current_dir(root());
        if let Some(listed) = listed {
            bash.env("MSYS2_ENV_CONV_EXCL", listed);
        }
        let out = run("tools/repro_env.sh", &mut bash);
        out.lines().map(str::to_string).collect()
    };
    let out = sourced(None);
    let excluded: Vec<&str> = out[0].split(';').collect();
    assert_eq!(excluded, ["CARGO_ENCODED_RUSTFLAGS", "CFLAGS"]);
    // What the exclusion is for: values that begin a path at `/`.
    assert!(
        out[1].contains("=/vy") && out[1].contains("=/cargo"),
        "{}",
        out[1]
    );
    let release = std::fs::read_to_string(root().join("tools/build_release.sh")).unwrap();
    assert!(release.contains("-Clink-arg=/Brepro") && release.contains(". tools/repro_env.sh"));
    assert_eq!(
        sourced(Some("MY_VAR;OTHER"))[0],
        "MY_VAR;OTHER;CARGO_ENCODED_RUSTFLAGS;CFLAGS",
        "what was listed stays listed"
    );
}
