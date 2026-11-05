//! The built binary: `--screenshot` through the whole pipeline (shell,
//! a real tab process, chrome, compositing) and `--tab` as a child. This
//! integration test is also what makes `cargo test` build the binary the
//! unit tests spawn.

use std::path::PathBuf;
use std::process::Command;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// SPEC-11.3.2, SPEC-9.6.1: `vy-shell --screenshot` writes the composited
/// frame with the chrome above the page, deterministically.
#[test]
fn screenshot_command_writes_the_composited_frame() {
    let exe = env!("CARGO_BIN_EXE_vy-shell");
    let dir = std::env::temp_dir().join(format!("vy-shell-bin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("hello.png");
    let module = root().join("target/wasm/hello.wasm");
    assert!(
        module.is_file(),
        "{}: run tools/build_modules.sh first",
        module.display()
    );
    let run = |out: &std::path::Path| {
        let status = Command::new(exe)
            .args(["--screenshot"])
            .arg(out)
            .args(["--size", "640x480"])
            .arg(&module)
            .status()
            .unwrap();
        assert!(status.success());
        vy_render::RgbaImage::decode_png(&std::fs::read(out).unwrap()).unwrap()
    };
    let a = run(&out);
    let b = run(&dir.join("hello2.png"));
    assert_eq!((a.width, a.height), (640, 480));
    assert_eq!(a, b, "the same state is the same pixels (SPEC-2.1.5)");
    // The chrome is on top: the strip colour at the top-left corner, the
    // page's dark background below the chrome.
    assert_eq!(a.pixel(2, 2), Some([0xDE, 0xDC, 0xD6, 255]));
    assert_eq!(a.pixel(2, 100), Some([0x1A, 0x1A, 0x2E, 255]));
    assert!(Command::new(exe)
        .args(["--screenshot"])
        .arg(dir.join("none.png"))
        .args(["--size", "640x480"])
        .status()
        .map(|s| !s.success())
        .unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}
