//! `tools/package_conformance.sh`: the suite as other clients get it
//! (SPEC-12.1.2), built twice and unpacked. Unix only: the script is run
//! under two umasks.
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

/// SPEC-9.7.1 (granskning 0010:3): the published suite is the same bytes
/// whoever builds it. The system `zip` stored each file's mode, so two
/// builders with different umasks got two packages; the zip is now
/// written by `tools/release_files.py`, as every other zip of a release:
/// sorted entries, one time, mode 0644, no directories. The package
/// unpacks to files that match its `SHA256SUMS`, and `vy-cli conformance
/// --root` runs the unpacked suite.
#[test]
fn spec_9_7_1_conformance_package_is_the_same_under_any_umask() {
    let root = root();
    let base = std::env::temp_dir().join(format!("vy-conformance-pkg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let name = "vy-conformance-0.0.0-test";
    let mut zips = Vec::new();
    for umask in ["022", "002"] {
        let out = base.join(umask);
        std::fs::create_dir_all(&out).unwrap();
        run(
            "tools/package_conformance.sh",
            Command::new("bash")
                .arg("-c")
                .arg(format!(
                    "umask {umask} && tools/package_conformance.sh 0.0.0-test \"$VY_TEST_OUT\""
                ))
                .env("VY_TEST_OUT", &out)
                .env("SOURCE_DATE_EPOCH", "1767225600")
                .env_remove("CARGO_TARGET_DIR")
                .env_remove("CARGO_BUILD_TARGET_DIR")
                .current_dir(&root),
        );
        zips.push(std::fs::read(out.join(format!("{name}.zip"))).unwrap());
    }
    assert!(zips[0].len() > 1_000_000, "{} bytes", zips[0].len());
    assert!(
        zips[0] == zips[1],
        "the package differs between umask 022 and 002 ({} and {} bytes)",
        zips[0].len(),
        zips[1].len()
    );

    // Every entry is a file with mode 0644 and the commit's time, in
    // sorted order, under the package's own directory.
    let listing = run(
        "listing the zip",
        Command::new("python3")
            .arg("-c")
            .arg(
                "import sys, zipfile\n\
                 for i in zipfile.ZipFile(sys.argv[1]).infolist():\n\
                 \x20   print(i.filename, oct(i.external_attr >> 16), i.date_time, len(i.extra))",
            )
            .arg(base.join("022").join(format!("{name}.zip"))),
    );
    let entries: Vec<&str> = listing.lines().collect();
    assert!(entries.len() > 150, "{} entries", entries.len());
    let names: Vec<&str> = entries
        .iter()
        .map(|e| e.split(' ').next().unwrap())
        .collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(names, sorted);
    for e in &entries {
        assert!(
            e.starts_with(&format!("{name}/")) && e.ends_with(" 0o100644 (2026, 1, 1, 0, 0, 0) 0"),
            "{e}"
        );
    }
    for part in [
        "SHA256SUMS",
        "README.md",
        "SPEC.md",
        "conformance/cases.toml",
        "conformance/golden/hello-800x600@1.png",
        "target/wasm/hello.wasm",
    ] {
        assert!(names.contains(&format!("{name}/{part}").as_str()), "{part}");
    }

    // Unpacked, the files are the ones `SHA256SUMS` lists, and the suite
    // runs from there.
    let unpacked = base.join("unpacked");
    run(
        "unpacking",
        Command::new("python3")
            .args(["-m", "zipfile", "-e"])
            .arg(base.join("022").join(format!("{name}.zip")))
            .arg(&unpacked),
    );
    let suite = unpacked.join(name);
    let sums = std::fs::read_to_string(suite.join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), entries.len() - 1, "all but itself");
    for line in sums.lines() {
        let (hash, file) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(suite.join(file)).unwrap();
        assert_eq!(vy_runtime::hex(&vy_runtime::sha256(&bytes)), hash, "{file}");
    }
    let ran = run(
        "vy-cli conformance --root <unpacked>",
        Command::new(env!("CARGO_BIN_EXE_vy-cli"))
            .args(["conformance", "--root"])
            .arg(&suite)
            .arg("hello-"),
    );
    assert_eq!(ran.lines().count(), 4, "{ran}");
    assert!(ran.lines().all(|l| l.starts_with("ok ")), "{ran}");
    let _ = std::fs::remove_dir_all(&base);
}
