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

/// SPEC-9.7.1 (granskning 0012:3): `vy-dev new` pins a new project's SDK
/// to the tag of the release that `vy-dev` came with, which does not
/// exist until the release does; the test of that pin tests a string.
/// The last job of the workflow does what someone outside does with the
/// released `vy-dev`: a new project in an empty directory, built. After
/// a release, without `--sdk`, so it fails when the tag cannot be fetched
/// or the page does not build; in a dry run, on the checkout.
#[test]
fn spec_9_7_1_a_release_ends_with_a_new_project_built() {
    let release = workflow("release.yml");
    let text = code(&release);
    assert_eq!(jobs(&text).last(), Some(&"smoke"));
    let smoke = job(&text, "smoke");
    // After the release exists; and in a dry run, where that job is
    // skipped, all the same.
    assert!(smoke.contains("needs: [version, sums, release]"));
    assert!(smoke.contains(
        "if: ${{ !cancelled() && needs.sums.result == 'success' && (inputs.dry_run || needs.release.result == 'success') }}"
    ));
    assert!(smoke.contains("DRY_RUN: ${{ inputs.dry_run }}"));
    // The released binary, not one built here.
    assert!(smoke.contains("name: vy-x86_64-unknown-linux-gnu"));
    assert!(!smoke.contains("cargo build") && !smoke.contains("cargo run"));
    let steps = steps(smoke);
    let run = steps.last().unwrap();
    let (rehearsal, rest) = run
        .split_once("if [ \"$DRY_RUN\" = true ]; then\n")
        .and_then(|(_, after)| after.split_once("else\n"))
        .expect("a branch for the dry run");
    let (released, after) = rest.split_once("\n          fi\n").unwrap();
    assert!(rehearsal.contains("\"$vy_dev\" new \"$project\" --sdk \"$GITHUB_WORKSPACE\""));
    assert!(released.contains("\"$vy_dev\" new \"$project\"\n"));
    assert!(!released.contains("--sdk"), "the tag is what is tried");
    assert!(
        released.contains("tag = \\\"$TAG\\\""),
        "and it is this release's"
    );
    assert!(after.contains("\"$vy_dev\" build \"$project\""));
    // The pin the released `vy-dev` writes is the workspace's version,
    // which the first job requires the tag to be.
    let scaffold = std::fs::read_to_string(root().join("crates/vy-dev/src/scaffold.rs")).unwrap();
    assert!(scaffold.contains("tag = \\\"{SDK_TAG}\\\""));
    assert!(job(&text, "version").contains("[workspace.package] version in Cargo.toml"));
}

/// SPEC-9.7.1 (granskning 0012:4): the workflows hold no more rights than
/// they use. Every job reads the repository and only `release` writes to
/// it; and the package signing key is in the environment of one step,
/// which compiles nothing: the tools and the modules are built by the
/// step before it, so no dependency's build script runs with the key.
#[test]
fn spec_9_7_1_the_release_gives_rights_and_the_key_only_where_used() {
    let release = workflow("release.yml");
    let text = code(&release);
    let (head, _) = text.split_once("\njobs:\n").unwrap();
    assert!(head.contains("\npermissions:\n  contents: read\n"));
    for name in jobs(&text) {
        let writes = job(&text, name).contains("contents: write");
        assert_eq!(writes, name == "release", "{name}");
    }
    let ci = code(&workflow("ci.yml"));
    let (head, _) = ci.split_once("\njobs:\n").unwrap();
    assert!(head.contains("\npermissions:\n  contents: read\n"));
    assert!(!ci.contains("contents: write") && !ci.contains("secrets."));

    let site = job(&text, "site");
    let steps = steps(site);
    let with_key: Vec<usize> = (0..steps.len())
        .filter(|i| steps[*i].contains("secrets.VY_KEY"))
        .collect();
    assert_eq!(with_key.len(), 1, "one step holds the key");
    let sign = steps[with_key[0]];
    assert!(sign.contains("VY_SITE_PREBUILT: 1"));
    assert!(!sign.contains("cargo ") && !sign.contains("build_modules"));
    let build = steps[with_key[0] - 1];
    assert!(build.contains("cargo build --release --locked -p vy-publish -p vy-cli"));
    assert!(build.contains("tools/build_modules.sh"));
    assert!(!build.contains("VY_KEY"));
    // The script compiles nothing when told that everything is built.
    let script = std::fs::read_to_string(root().join("tools/publish_site.sh")).unwrap();
    let (_, prebuilt) = script
        .split_once("if [ \"${VY_SITE_PREBUILT:-}\" = 1 ]; then\n")
        .expect("a prebuilt branch");
    let (prebuilt, rest) = prebuilt.split_once("\nelse\n").unwrap();
    assert!(!prebuilt.contains("cargo") && !prebuilt.contains("build_modules"));
    let (built_here, _) = rest.split_once("\nfi\n").unwrap();
    assert!(built_here.contains("cargo build") && built_here.contains("tools/build_modules.sh"));

    // What is signed is built from the checkout alone: the jobs that
    // build the binaries and the site restore no cache, which a run on
    // `main` could have left. And every action, in every workflow, is the
    // commit it was read at, not a tag that can be moved (granskning
    // 0016:3).
    for name in ["binaries", "site"] {
        assert!(!job(&text, name).contains("rust-cache"), "{name}");
    }
    let mut files = vec![
        ".github/workflows/ci.yml".to_owned(),
        ".github/workflows/fuzz.yml".to_owned(),
        ".github/workflows/release.yml".to_owned(),
    ];
    for entry in std::fs::read_dir(root().join(".github/actions")).unwrap() {
        let action = entry.unwrap().path().join("action.yml");
        let action = action.strip_prefix(root()).unwrap().to_owned();
        files.push(action.to_string_lossy().into_owned());
    }
    let mut checked = 0;
    for file in &files {
        let text = code(&std::fs::read_to_string(root().join(file)).unwrap());
        for line in text.lines() {
            let Some((_, what)) = line.split_once("uses: ") else {
                continue;
            };
            let what = what.trim();
            let pinned = what.starts_with("./")
                || what.split_once('@').is_some_and(|(_, at)| {
                    let commit = at.split_whitespace().next().unwrap_or("");
                    commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit())
                });
            assert!(pinned, "{file}: {what}");
            checked += 1;
        }
    }
    assert!(checked >= 25, "{checked} actions checked");
}

/// SPEC-9.7.1 (granskning 0012:5): `SHA256SUMS` has a line for every
/// file a release attaches. `BUILD.md` is written first and summed with
/// the site and conformance zips; the platform zips and the files in
/// them come from each build's own sums.
#[test]
fn spec_9_7_1_sha256sums_covers_every_release_asset() {
    let release = workflow("release.yml");
    let text = code(&release);
    let sums = job(&text, "sums");
    let recipe = sums.find("build-md ../tools/BUILD.md.in").unwrap();
    let summed = sums
        .find("sums \"\" vy-site-*.zip vy-conformance-*.zip BUILD.md")
        .expect("BUILD.md is summed with the zips");
    assert!(recipe < summed, "the recipe exists before it is summed");
    assert!(sums.contains("cat vy-*.sha256"));
    // What the release attaches is what was summed, and nothing else.
    let attached = job(&text, "release");
    for file in ["dist/*.zip", "dist/SHA256SUMS", "dist/BUILD.md"] {
        assert!(attached.contains(file), "{file}");
    }
    assert_eq!(attached.matches("dist/").count(), 3);
}
