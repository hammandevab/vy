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
    /// Frames to run; the last one is rasterized.
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
}

/// Physical pixel size of a logical size at `scale`.
pub fn physical(width: u32, height: u32, scale: f32) -> (u32, u32) {
    (
        ((width as f32) * scale).round().max(1.0) as u32,
        ((height as f32) * scale).round().max(1.0) as u32,
    )
}

/// Load, init and run `frames` frames of a module, then rasterize the last one
/// (SPEC-11.3.1). No window, no wall clock, no system fonts.
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
    for i in 0..req.frames {
        display_list = rt
            .frame(HEADLESS_DT_MS)
            .map_err(|e| anyhow!("frame {}: {e}", i + 1))?
            .to_vec();
    }
    let (pw, ph) = physical(req.width, req.height, req.scale);
    let image = vy_render::render(&display_list, pw, ph, req.scale, rt.text_engine_mut())
        .map_err(|e| anyhow!("{e}"))?;
    Ok(RenderOutput {
        image,
        display_list,
        semantic_json: rt.semantic_tree_json(),
        logs: rt.logs().to_vec(),
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

    /// SPEC-13.1.4: compiling the reference module is fast. The 60 ms figure is a
    /// release-mode budget on the reference machine (`vy-cli bench`, M3); here a
    /// debug build must stay under 2 s so the gate never flakes.
    #[test]
    fn spec_13_1_4_compile_time() {
        let wasm = hello();
        let hash = vy_runtime::sha256(&wasm);
        let t = Instant::now();
        let rt = Runtime::load(&wasm, &hash, Options::default()).unwrap();
        let elapsed = t.elapsed();
        drop(rt);
        assert!(elapsed.as_millis() < 2000, "compile took {elapsed:?}");
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
