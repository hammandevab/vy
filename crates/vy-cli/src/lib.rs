//! Headless tools for the Vy client: `check-abi`, `render`, `conformance` (SPEC §11.3, §12.2).
//!
//! Everything the binary does lives here so the gates can call it as a library.
#![deny(missing_docs)]

pub mod conformance;

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::Path;
use vy_render::RgbaImage;
use vy_runtime::{LogEntry, Options, Runtime};

pub use vy_runtime::{check_abi_bytes, AbiReport};

/// Milliseconds passed as `dt_ms` to every headless frame.
pub const HEADLESS_DT_MS: i32 = 16;

/// Validate a module file against the Vy ABI.
pub fn check_abi(path: &str) -> Result<AbiReport> {
    let bytes = fs::read(path).with_context(|| format!("reading {path}"))?;
    Ok(check_abi_bytes(&bytes)?)
}

/// Parse `WxH` into logical width and height.
pub fn parse_size(s: &str) -> Result<(u32, u32)> {
    let (w, h) = s
        .split_once('x')
        .ok_or_else(|| anyhow!("size must be WxH, got {s:?}"))?;
    let w: u32 = w.parse().with_context(|| format!("bad width in {s:?}"))?;
    let h: u32 = h.parse().with_context(|| format!("bad height in {s:?}"))?;
    if w == 0 || h == 0 {
        bail!("size must be at least 1x1");
    }
    Ok((w, h))
}

/// What to render.
#[derive(Clone, Copy, Debug)]
pub struct RenderRequest<'a> {
    /// Module bytes.
    pub wasm: &'a [u8],
    /// Logical window width.
    pub width: u32,
    /// Logical window height.
    pub height: u32,
    /// Logical-to-physical ratio (SPEC-6.3.1).
    pub scale: f32,
    /// Most frames to run; the last one run is rasterized. The first frame
    /// always runs (init is a window change); each later one only while the
    /// module wants a frame (SPEC-6.2.1), so a static page stops after one.
    pub frames: u32,
    /// `memory_max` in pages (SPEC-4.2.4).
    pub memory_max_pages: u32,
}

impl<'a> RenderRequest<'a> {
    /// One frame at scale 1 with the default memory cap.
    pub fn new(wasm: &'a [u8], width: u32, height: u32) -> Self {
        Self {
            wasm,
            width,
            height,
            scale: 1.0,
            frames: 1,
            memory_max_pages: vy_runtime::DEFAULT_MEMORY_MAX_PAGES,
        }
    }
}

/// Everything a headless render produces.
#[derive(Debug)]
pub struct RenderOutput {
    /// Physical-pixel image of the last frame.
    pub image: RgbaImage,
    /// Display list bytes of the last frame.
    pub display_list: Vec<u8>,
    /// Semantic tree of the last frame as JSON (SPEC-10.2.3).
    pub semantic_json: String,
    /// Module and host log lines.
    pub logs: Vec<LogEntry>,
    /// Frames actually run: 1 to `RenderRequest::frames` (SPEC-6.2.1).
    pub frames_run: u32,
}

/// Physical pixel size of a logical size at `scale`.
pub fn physical(width: u32, height: u32, scale: f32) -> (u32, u32) {
    (
        ((width as f32) * scale).round().max(1.0) as u32,
        ((height as f32) * scale).round().max(1.0) as u32,
    )
}

/// Load, init and run up to `frames` frames of a module, then rasterize the
/// last one (SPEC-11.3.1). No window, no wall clock, no system fonts.
///
/// `vy_frame` is called only while the runtime wants a frame (SPEC-6.2.1):
/// once after init, then after each frame that called `request_frame`.
pub fn render(req: &RenderRequest<'_>) -> Result<RenderOutput> {
    if req.frames == 0 {
        bail!("frames must be at least 1");
    }
    if !(req.scale.is_finite() && req.scale > 0.0 && req.scale <= 8.0) {
        bail!("scale must be in (0, 8]");
    }
    let hash = vy_runtime::sha256(req.wasm);
    let options = Options {
        memory_max_pages: req.memory_max_pages,
        ..Options::default()
    };
    let mut rt = Runtime::load(req.wasm, &hash, options).map_err(|e| anyhow!("{e}"))?;
    rt.init(req.width, req.height, req.scale)
        .map_err(|e| anyhow!("{e}"))?;
    let mut display_list = Vec::new();
    let mut frames_run = 0;
    while frames_run < req.frames && rt.wants_frame() {
        display_list = rt
            .frame(HEADLESS_DT_MS)
            .map_err(|e| anyhow!("frame {}: {e}", frames_run + 1))?
            .to_vec();
        frames_run += 1;
    }
    debug_assert!(frames_run >= 1, "init wants the first frame");
    let (pw, ph) = physical(req.width, req.height, req.scale);
    let image = vy_render::render(&display_list, pw, ph, req.scale, rt.text_engine_mut())
        .map_err(|e| anyhow!("{e}"))?;
    Ok(RenderOutput {
        image,
        display_list,
        semantic_json: rt.semantic_tree_json(),
        logs: rt.logs().to_vec(),
        frames_run,
    })
}

/// `render` for a module file; writes the PNG and optionally the semantic JSON.
pub fn render_to_files(
    module: &str,
    out: &Path,
    width: u32,
    height: u32,
    scale: f32,
    frames: u32,
    dump_semantic: Option<&Path>,
) -> Result<RenderOutput> {
    if module.starts_with("vy://") {
        bail!("fetching vy:// addresses arrives in M3; give a .wasm file");
    }
    let wasm = fs::read(module).with_context(|| format!("reading {module}"))?;
    let mut req = RenderRequest::new(&wasm, width, height);
    req.scale = scale;
    req.frames = frames;
    let output = render(&req)?;
    fs::write(out, output.image.encode_png()?)
        .with_context(|| format!("writing {}", out.display()))?;
    if let Some(path) = dump_semantic {
        fs::write(path, &output.semantic_json)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::time::Instant;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    /// `target/wasm/hello.wasm`, built by `tools/build_modules.sh`.
    fn hello() -> Vec<u8> {
        let path = root().join("target/wasm/hello.wasm");
        fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}\nrun tools/build_modules.sh before cargo test",
                path.display()
            )
        })
    }

    /// SPEC-11.3.1, SPEC-11.3.2: `render` produces a deterministic PNG through the shared
    /// render path, with no window.
    #[test]
    fn spec_11_3_1_headless_render() {
        let wasm = hello();
        let a = render(&RenderRequest::new(&wasm, 800, 600)).unwrap();
        let b = render(&RenderRequest::new(&wasm, 800, 600)).unwrap();
        assert_eq!((a.image.width, a.image.height), (800, 600));
        assert_eq!(a.display_list, b.display_list);
        assert_eq!(a.image, b.image);
        assert_eq!(a.image.encode_png().unwrap(), b.image.encode_png().unwrap());
        assert!(a.logs.iter().any(|l| l.message == "hello: init"));
        // The card is light on a dark background (modules/hello/src/lib.rs).
        assert_eq!(a.image.pixel(2, 2).unwrap(), [0x1A, 0x1A, 0x2E, 255]);
        assert_eq!(a.image.pixel(100, 30).unwrap(), [0xF5, 0xF5, 0xF0, 255]);
        let mut req = RenderRequest::new(&wasm, 800, 600);
        req.scale = 2.0;
        let two = render(&req).unwrap();
        assert_eq!((two.image.width, two.image.height), (1600, 1200));
    }

    /// SPEC-10.2.3: `--dump-semantic` writes the tree as JSON, matching the committed tree.
    #[test]
    fn spec_10_2_3_dump_semantic() {
        let wasm = hello();
        let out = render(&RenderRequest::new(&wasm, 800, 600)).unwrap();
        assert_eq!(out.semantic_json.matches("\"id\":").count(), 3);
        assert!(out.semantic_json.contains("\"role_name\": \"heading\""));
        assert!(out.semantic_json.contains("Hej från Vy"));
        let expected_path = root().join("conformance/semantic/hello.json");
        let expected = fs::read_to_string(&expected_path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}\nrun `vy-cli conformance --update`",
                expected_path.display()
            )
        });
        assert_eq!(
            out.semantic_json, expected,
            "semantic tree drifted from conformance/semantic/hello.json"
        );
        // The CLI path writes the same bytes.
        let dir = std::env::temp_dir().join(format!("vy-dump-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let png = dir.join("hello.png");
        let json = dir.join("hello.json");
        let wasm_path = root().join("target/wasm/hello.wasm");
        render_to_files(
            wasm_path.to_str().unwrap(),
            &png,
            800,
            600,
            1.0,
            1,
            Some(&json),
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&json).unwrap(), expected);
        assert_eq!(
            RgbaImage::decode_png(&fs::read(&png).unwrap()).unwrap(),
            out.image
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A module whose frame draws one rectangle per frame run so far and calls
    /// `request_frame` while fewer than `requests` frames have run.
    fn counting_module(requests: u32) -> Vec<u8> {
        wat::parse_str(format!(
            r#"(module
                (import "vy" "request_frame" (func $request_frame))
                (import "vy" "dl_begin" (func $dl_begin))
                (import "vy" "dl_end" (func $dl_end))
                (import "vy" "dl_rect" (func $dl_rect (param f32 f32 f32 f32 f32 i32)))
                (memory (export "memory") 1)
                (global $n (mut i32) (i32.const 0))
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32)
                    (local $i i32)
                    (global.set $n (i32.add (global.get $n) (i32.const 1)))
                    (call $dl_begin)
                    (block (loop
                        (br_if 1 (i32.ge_u (local.get $i) (global.get $n)))
                        (call $dl_rect (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (f32.const 0) (i32.const 0))
                        (local.set $i (i32.add (local.get $i) (i32.const 1)))
                        (br 0)))
                    (call $dl_end)
                    (if (i32.lt_u (global.get $n) (i32.const {requests})) (then (call $request_frame)))
                    global.get $n)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#
        ))
        .unwrap()
    }

    /// SPEC-6.2.1, SPEC-5.2.2: `render --frames N` calls `vy_frame` once after
    /// init and then only while the module asked for another frame; `N` is a
    /// ceiling, not a count.
    #[test]
    fn spec_6_2_1_render_frames_is_a_ceiling() {
        let rects = |out: &RenderOutput| vy_dl::decode(&out.display_list).unwrap().len();
        for (requests, max, expect) in [(0, 5, 1), (0, 1, 1), (3, 5, 3), (3, 2, 2), (10, 5, 5)] {
            let wasm = counting_module(requests);
            let mut req = RenderRequest::new(&wasm, 8, 8);
            req.frames = max;
            let out = render(&req).unwrap();
            assert_eq!(
                out.frames_run, expect,
                "requests {requests}, --frames {max}"
            );
            assert_eq!(
                rects(&out),
                expect as usize,
                "the last frame run is rasterized"
            );
        }
        // hello never calls request_frame: one frame however many are allowed.
        let wasm = hello();
        let mut req = RenderRequest::new(&wasm, 800, 600);
        req.frames = 5;
        let five = render(&req).unwrap();
        assert_eq!(five.frames_run, 1);
        assert_eq!(
            five.image,
            render(&RenderRequest::new(&wasm, 800, 600)).unwrap().image
        );
        assert!(render(&RenderRequest {
            frames: 0,
            ..RenderRequest::new(&wasm, 8, 8)
        })
        .is_err());
    }

    /// SPEC-13.1.4: compiling a 300 KB module takes at most 60 ms, P95 over
    /// 100 runs on the reference machine (SPEC §13), and 0 ms on a
    /// compilation-cache hit. A release-mode measurement for `vy-cli bench`
    /// (M3); `hello` stands in until a reference page of that size exists
    /// (M2), and no compilation cache is configured yet, so the 0 ms half
    /// stays open. Meaningless in a debug build, hence the guard.
    #[test]
    #[ignore = "M3: bench"]
    fn spec_13_1_4_compile_time() {
        if cfg!(debug_assertions) {
            panic!("SPEC-13.1.4 is a release-mode budget; run with --release");
        }
        let wasm = hello();
        let hash = vy_runtime::sha256(&wasm);
        let mut samples: Vec<_> = (0..100)
            .map(|_| {
                let t = Instant::now();
                let rt = Runtime::load(&wasm, &hash, Options::default()).unwrap();
                let elapsed = t.elapsed();
                drop(rt);
                elapsed
            })
            .collect();
        samples.sort();
        let p95 = samples[94];
        assert!(p95.as_millis() <= 60, "P95 compile time {p95:?} > 60 ms");
    }

    /// SPEC-11.3.1: `--size` parsing.
    #[test]
    fn spec_11_3_1_size_parsing() {
        assert_eq!(parse_size("800x600").unwrap(), (800, 600));
        assert!(parse_size("800").is_err());
        assert!(parse_size("0x600").is_err());
        assert_eq!(physical(800, 600, 2.0), (1600, 1200));
        assert_eq!(physical(401, 301, 1.5), (602, 452));
    }
}

#[cfg(test)]
mod bench_tests;
