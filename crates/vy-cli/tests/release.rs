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

/// A workflow under `.github/workflows/`, as text.
fn workflow(name: &str) -> String {
    let path = root().join(".github/workflows").join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The lines of a workflow that are not comments.
fn code(workflow: &str) -> String {
    workflow
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The names of a workflow's jobs, in order: the keys two spaces in
/// after the line `jobs:`.
fn jobs(workflow: &str) -> Vec<&str> {
    let (_, after) = workflow.split_once("\njobs:\n").expect("a jobs: key");
    after
        .lines()
        .filter(|l| l.starts_with("  ") && !l.starts_with("   ") && !l.starts_with("  #"))
        .filter_map(|l| l.trim().strip_suffix(':'))
        .collect()
}

/// The text of job `name`: from its key to the next job's.
fn job<'a>(workflow: &'a str, name: &str) -> &'a str {
    let start = workflow
        .find(&format!("\n  {name}:\n"))
        .unwrap_or_else(|| panic!("no job {name}"));
    let body = &workflow[start + 1..];
    let end = body
        .match_indices("\n  ")
        .map(|(i, _)| i + 1)
        .find(|i| {
            let line = body[*i..].lines().next().unwrap_or("");
            *i > 0 && !line.starts_with("   ") && !line.starts_with("  #") && line.ends_with(':')
        })
        .unwrap_or(body.len());
    &body[..end]
}

/// The steps of a job: each from its `      - ` to the next.
fn steps(job: &str) -> Vec<&str> {
    let mut starts: Vec<usize> = job
        .match_indices("\n      - ")
        .map(|(i, _)| i + 1)
        .collect();
    starts.push(job.len());
    starts.windows(2).map(|w| &job[w[0]..w[1]]).collect()
}

/// SPEC-12.3.1, SPEC-9.7.1 (granskning 0012:2): the size cap stops a
/// release on the day it is made, and the binaries of two platforms had
/// never been built by the workflow. A manual run with `dry_run` is the
/// rehearsal: it builds, measures, sums and writes the recipe as a
/// release does, and skips the one job that publishes.
#[test]
fn spec_12_3_1_a_release_can_be_rehearsed() {
    let release = workflow("release.yml");
    let text = code(&release);
    let (_, inputs) = text
        .split_once("  workflow_dispatch:\n    inputs:\n")
        .unwrap();
    let inputs = inputs.split_once("\n\n").unwrap().0;
    let (_, dry_run) = inputs
        .split_once("      dry_run:\n")
        .expect("a dry_run input");
    assert!(dry_run.contains("type: boolean") && dry_run.contains("default: false"));

    assert_eq!(
        jobs(&text)[..5],
        ["version", "binaries", "site", "sums", "release"]
    );
    // Only the release itself is left out of a rehearsal.
    for name in jobs(&text) {
        let job = job(&text, name);
        let skipped = job.contains("if: ${{ !inputs.dry_run }}");
        assert_eq!(skipped, name == "release", "{name}");
    }
    // The size check and the sums are in jobs a rehearsal runs.
    assert!(job(&text, "binaries").contains("over SPEC-12.3.1's 25000000"));
    let sums = job(&text, "sums");
    assert!(sums.contains("needs: [version, binaries, site]"));
    assert!(sums.contains("> SHA256SUMS") && sums.contains("> BUILD.md"));
    assert!(sums.contains("name: vy-sums"), "kept as an artifact");
    // The release publishes what that job made and makes nothing itself.
    let publish = job(&text, "release");
    assert!(publish.contains("needs: [version, sums]"));
    let steps = steps(publish);
    assert_eq!(steps.len(), 2, "download and publish");
    assert!(steps[0].contains("actions/download-artifact@"));
    assert!(steps[1].contains("softprops/action-gh-release@"));
    for file in ["dist/*.zip", "dist/SHA256SUMS", "dist/BUILD.md"] {
        assert!(steps[1].contains(file), "{file}");
    }
}
