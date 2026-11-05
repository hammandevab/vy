//! The `vy-publish` command line, run as a binary.

use std::path::PathBuf;
use std::process::Command;
use vy_manifest::SigningKey;

fn scratch(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("vy-publish-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A module whose second frame, after the publish-time resize to 1 280 px,
/// loops five million times: about 30 ms of logical time.
fn slow_module() -> Vec<u8> {
    wat::parse_str(
        r#"(module (memory (export "memory") 2)
            (global $w (mut i32) (i32.const 0))
            (func (export "vy_init") (param i32 i32 i32 f32) (result i32)
                (global.set $w (local.get 1)) i32.const 0)
            (func (export "vy_frame") (param i32) (result i32)
                (local $i i32)
                (if (i32.gt_s (global.get $w) (i32.const 1000)) (then
                    (loop $spin
                        (local.set $i (i32.add (local.get $i) (i32.const 1)))
                        (br_if $spin (i32.lt_u (local.get $i) (i32.const 5000000))))))
                i32.const 0)
            (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
            (func (export "vy_resize") (param i32 i32 f32) (global.set $w (local.get 0)))
            (func (export "vy_suspend") (result i32) i32.const 0)
            (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
    )
    .unwrap()
}

/// SPEC-13.4.1: `vy-publish build` refuses a module whose frame breaks the
/// 8 ms budget, naming the frame and its fuel, and writes nothing; with
/// `--allow-slow` it warns and publishes.
#[test]
fn build_refuses_a_slow_module_without_allow_slow() {
    let dir = scratch("slow");
    let manifest = dir.join("page.json");
    std::fs::write(
        &manifest,
        include_str!("../../../conformance/manifests/hello.json"),
    )
    .unwrap();
    let module = dir.join("page.wasm");
    std::fs::write(&module, slow_module()).unwrap();
    let key = dir.join("dev.key");
    vy_publish::write_key(&key, &SigningKey::from_seed(&[7; 32])).unwrap();
    let out = dir.join("page.vyp");
    let run = |extra: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_vy-publish"));
        cmd.arg("build")
            .arg("--manifest")
            .arg(&manifest)
            .arg("--module")
            .arg(&module)
            .arg("--key")
            .arg(&key)
            .arg("--out")
            .arg(&out)
            .args(extra);
        cmd.output().unwrap()
    };
    let refused = run(&[]);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "{stderr}");
    assert!(
        stderr.contains("frame 2 (after a resize to 1280 × 800 at scale 2) used"),
        "{stderr}"
    );
    assert!(
        stderr.contains("fuel") && stderr.contains("--allow-slow"),
        "{stderr}"
    );
    assert!(!out.exists(), "nothing published");

    let allowed = run(&["--allow-slow"]);
    let stderr = String::from_utf8_lossy(&allowed.stderr);
    assert!(allowed.status.success(), "{stderr}");
    assert!(stderr.starts_with("warning: frame 2"), "{stderr}");
    assert!(out.is_file());
    let _ = std::fs::remove_dir_all(&dir);
}
