//! Headless tools for the Vy client: `check-abi`, `render`, `conformance` (SPEC §11.3, §12.2).
//!
//! Everything the binary does lives here so the gates can call it as a library.
#![deny(missing_docs)]

pub mod conformance;

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::Path;
use vy_abi::event as kind;
use vy_render::RgbaImage;
use vy_runtime::{Capabilities, LogEntry, Options, Runtime, SemNode};

pub use vy_runtime::{check_abi_bytes, AbiReport, Resource};

/// An input event replayed before the final frame (SPEC-5.1.3, SPEC-A.2.1),
/// in logical pixels. The minimal subset a conformance case needs to reach a
/// focus, hover or scroll state; the shell delivers the rest in M3.
#[derive(Clone, Debug, PartialEq)]
pub enum ScriptedEvent {
    /// `POINTER_MOVE`.
    PointerMove {
        /// X.
        x: f32,
        /// Y.
        y: f32,
        /// Buttons held.
        buttons: i32,
        /// Modifier bits (`vy_abi::modifier`).
        modifiers: i32,
    },
    /// `POINTER_DOWN`.
    PointerDown {
        /// X.
        x: f32,
        /// Y.
        y: f32,
        /// The button.
        button: i32,
        /// Modifier bits.
        modifiers: i32,
    },
    /// `POINTER_UP`.
    PointerUp {
        /// X.
        x: f32,
        /// Y.
        y: f32,
        /// The button.
        button: i32,
        /// Modifier bits.
        modifiers: i32,
    },
    /// `POINTER_LEAVE`: the pointer left the page.
    PointerLeave,
    /// `SCROLL`.
    Scroll {
        /// Horizontal delta.
        dx: f32,
        /// Vertical delta.
        dy: f32,
        /// Deltas are lines, not pixels.
        lines: bool,
        /// Modifier bits.
        modifiers: i32,
    },
    /// `KEY_DOWN`.
    KeyDown {
        /// USB HID code.
        code: i32,
        /// Modifier bits.
        modifiers: i32,
        /// Auto-repeat.
        repeat: bool,
    },
    /// `KEY_UP`.
    KeyUp {
        /// USB HID code.
        code: i32,
        /// Modifier bits.
        modifiers: i32,
    },
    /// `TEXT_INPUT` through `vy_alloc` (SPEC-5.1.6).
    TextInput(String),
    /// `FOCUS` as the host's focus order would deliver it (SPEC-10.3.1); the
    /// headless host keeps no focus order, so a case names the node itself.
    Focus {
        /// Semantic node id.
        node: i32,
        /// True when the node gains focus.
        gained: bool,
        /// True when the change came from the keyboard.
        keyboard: bool,
    },
}

impl ScriptedEvent {
    /// Deliver to the runtime; true when the module asked for a redraw.
    fn deliver(&self, rt: &mut Runtime) -> Result<bool, vy_runtime::Error> {
        let x100 = |v: f32| (v * 100.0).round() as i32;
        match self {
            Self::PointerMove {
                x,
                y,
                buttons,
                modifiers,
            } => rt.event(kind::POINTER_MOVE, x100(*x), x100(*y), *buttons, *modifiers),
            Self::PointerDown {
                x,
                y,
                button,
                modifiers,
            } => rt.event(kind::POINTER_DOWN, x100(*x), x100(*y), *button, *modifiers),
            Self::PointerUp {
                x,
                y,
                button,
                modifiers,
            } => rt.event(kind::POINTER_UP, x100(*x), x100(*y), *button, *modifiers),
            Self::PointerLeave => rt.event(kind::POINTER_LEAVE, 0, 0, 0, 0),
            Self::Scroll {
                dx,
                dy,
                lines,
                modifiers,
            } => rt.event(
                kind::SCROLL,
                x100(*dx),
                x100(*dy),
                i32::from(*lines),
                *modifiers,
            ),
            Self::KeyDown {
                code,
                modifiers,
                repeat,
            } => rt.event(kind::KEY_DOWN, *code, *modifiers, i32::from(*repeat), 0),
            Self::KeyUp { code, modifiers } => rt.event(kind::KEY_UP, *code, *modifiers, 0, 0),
            Self::TextInput(text) => rt.text_input(text),
            Self::Focus {
                node,
                gained,
                keyboard,
            } => rt.event(
                kind::FOCUS,
                *node,
                i32::from(*gained),
                i32::from(*keyboard),
                0,
            ),
        }
    }
}

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
    /// Capabilities the headless host grants (SPEC-8.2.1); none by default.
    pub capabilities: Capabilities,
    /// Package resources, `resources[i]` being resource id `i + 1`
    /// (SPEC-4.5.1), each with the size the manifest declares when known
    /// (SPEC-8.1.2).
    pub resources: &'a [Resource],
    /// Events delivered after the first frame, in order; each one the module
    /// answers with a redraw gets a frame while `frames` allows (SPEC-6.2.1).
    pub events: &'a [ScriptedEvent],
}

impl<'a> RenderRequest<'a> {
    /// One frame at scale 1 with the default memory cap, no capabilities and
    /// no resources.
    pub fn new(wasm: &'a [u8], width: u32, height: u32) -> Self {
        Self {
            wasm,
            width,
            height,
            scale: 1.0,
            frames: 1,
            memory_max_pages: vy_runtime::DEFAULT_MEMORY_MAX_PAGES,
            capabilities: Capabilities::default(),
            resources: &[],
            events: &[],
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
    /// Semantic tree of the last frame.
    pub semantic: Vec<SemNode>,
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
/// once after init, after each scripted event the module answered with a
/// redraw, and after each frame that called `request_frame`, up to `frames`
/// in all.
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
        capabilities: req.capabilities,
        resources: req.resources.to_vec(),
        ..Options::default()
    };
    let mut rt = Runtime::load(req.wasm, &hash, options).map_err(|e| anyhow!("{e}"))?;
    rt.init(req.width, req.height, req.scale)
        .map_err(|e| anyhow!("{e}"))?;
    let mut display_list = Vec::new();
    let mut frames_run = 0;
    let mut run_frames = |rt: &mut Runtime, display_list: &mut Vec<u8>| -> Result<()> {
        while frames_run < req.frames && rt.wants_frame() {
            *display_list = rt
                .frame()
                .map_err(|e| anyhow!("frame {}: {e}", frames_run + 1))?
                .to_vec();
            frames_run += 1;
        }
        Ok(())
    };
    run_frames(&mut rt, &mut display_list)?;
    for (i, ev) in req.events.iter().enumerate() {
        ev.deliver(&mut rt)
            .map_err(|e| anyhow!("event {}: {e}", i + 1))?;
        run_frames(&mut rt, &mut display_list)?;
    }
    debug_assert!(frames_run >= 1, "init wants the first frame");
    let (pw, ph) = physical(req.width, req.height, req.scale);
    let (text, images) = rt.raster_parts();
    let image = vy_render::render(&display_list, pw, ph, req.scale, text, images)
        .map_err(|e| anyhow!("{e}"))?;
    Ok(RenderOutput {
        image,
        display_list,
        semantic_json: rt.semantic_tree_json(),
        semantic: rt.semantic_tree().to_vec(),
        logs: rt.logs().to_vec(),
        frames_run,
    })
}

/// `render` for a module file; writes the PNG and optionally the semantic JSON.
#[allow(clippy::too_many_arguments)]
pub fn render_to_files(
    module: &str,
    out: &Path,
    width: u32,
    height: u32,
    scale: f32,
    frames: u32,
    capabilities: Capabilities,
    dump_semantic: Option<&Path>,
) -> Result<RenderOutput> {
    if module.starts_with("vy://") {
        bail!("fetching vy:// addresses arrives in M3; give a .wasm file");
    }
    let wasm = fs::read(module).with_context(|| format!("reading {module}"))?;
    let mut req = RenderRequest::new(&wasm, width, height);
    req.scale = scale;
    req.frames = frames;
    req.capabilities = capabilities;
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
            Capabilities::default(),
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
                (import "vy" "dl_begin" (func $dl_begin (result i32)))
                (import "vy" "dl_end" (func $dl_end (result i32)))
                (import "vy" "dl_rect" (func $dl_rect (param f32 f32 f32 f32 f32 i32) (result i32)))
                (memory (export "memory") 1)
                (global $n (mut i32) (i32.const 0))
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32)
                    (local $i i32)
                    (global.set $n (i32.add (global.get $n) (i32.const 1)))
                    (drop (call $dl_begin))
                    (block (loop
                        (br_if 1 (i32.ge_u (local.get $i) (global.get $n)))
                        (drop (call $dl_rect (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (f32.const 0) (i32.const 0)))
                        (local.set $i (i32.add (local.get $i) (i32.const 1)))
                        (br 0)))
                    (drop (call $dl_end))
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

    /// A module that counts pointer downs and committed text bytes in
    /// `vy_event` (returning 1 for both) and draws one rectangle per count.
    fn event_module() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (import "vy" "dl_begin" (func $dl_begin (result i32)))
                (import "vy" "dl_end" (func $dl_end (result i32)))
                (import "vy" "dl_rect" (func $dl_rect (param f32 f32 f32 f32 f32 i32) (result i32)))
                (memory (export "memory") 1)
                (global $n (mut i32) (i32.const 0))
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32)
                    (local $i i32)
                    (drop (call $dl_begin))
                    (block (loop
                        (br_if 1 (i32.ge_u (local.get $i) (global.get $n)))
                        (drop (call $dl_rect (f32.const 0) (f32.const 0) (f32.const 1) (f32.const 1) (f32.const 0) (i32.const 0)))
                        (local.set $i (i32.add (local.get $i) (i32.const 1)))
                        (br 0)))
                    (drop (call $dl_end))
                    global.get $n)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                    (if (i32.eq (local.get 0) (i32.const 2)) (then
                        (global.set $n (i32.add (global.get $n) (i32.const 1)))
                        (return (i32.const 1))))
                    (if (i32.eq (local.get 0) (i32.const 7)) (then
                        (global.set $n (i32.add (global.get $n) (local.get 2)))
                        (return (i32.const 1))))
                    i32.const 0)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 1024))"#,
        )
        .unwrap()
    }

    /// SPEC-5.1.3, SPEC-6.2.1: scripted events are delivered in order after
    /// the first frame; each one answered with 1 gets a frame while the
    /// ceiling allows; the last frame run is what gets rasterized.
    #[test]
    fn spec_5_1_3_render_replays_events() {
        let wasm = event_module();
        let events = [
            ScriptedEvent::PointerMove {
                x: 1.0,
                y: 1.0,
                buttons: 0,
                modifiers: 0,
            },
            ScriptedEvent::PointerDown {
                x: 1.0,
                y: 1.0,
                button: 1,
                modifiers: 0,
            },
            ScriptedEvent::TextInput("abc".into()),
            ScriptedEvent::PointerDown {
                x: 1.0,
                y: 1.0,
                button: 1,
                modifiers: 0,
            },
        ];
        let rects = |out: &RenderOutput| vy_dl::decode(&out.display_list).unwrap().len();
        let mut req = RenderRequest::new(&wasm, 8, 8);
        req.events = &events;
        req.frames = 10;
        let out = render(&req).unwrap();
        assert_eq!(
            out.frames_run, 4,
            "init plus the three events answered with 1"
        );
        assert_eq!(rects(&out), 1 + 3 + 1);
        req.frames = 2;
        let out = render(&req).unwrap();
        assert_eq!(out.frames_run, 2, "the ceiling stops at the first redraw");
        assert_eq!(rects(&out), 1, "later events were delivered but not drawn");
        req.events = &[];
        req.frames = 10;
        assert_eq!(render(&req).unwrap().frames_run, 1);
    }

    /// SPEC-10.3.1, SPEC-10.3.3, SPEC-10.1.1: three Tabs reach the document
    /// page as `FOCUS` events through the host's own order over the whole
    /// tree, never as `KEY_DOWN` 43, and the focus ring the golden shows is
    /// drawn by the module: it is a `RECT` in the display list, and the
    /// pixels are nothing but that list rasterized.
    #[test]
    fn spec_10_3_3_focus_ring_comes_from_the_module() {
        let wasm = fs::read(root().join("target/wasm/document.wasm")).unwrap();
        let tab = ScriptedEvent::KeyDown {
            code: vy_abi::key::TAB,
            modifiers: 0,
            repeat: false,
        };
        let events = [tab.clone(), tab.clone(), tab];
        let mut req = RenderRequest::new(&wasm, 1280, 800);
        req.events = &events;
        req.frames = 4;
        let out = render(&req).unwrap();
        assert_eq!(
            out.frames_run, 4,
            "each Tab moved focus and asked for a frame"
        );
        // The third link (Metod) carries the ring: the page's RING colour.
        const RING: u32 = 0x2F5B_D6FF;
        let ops = vy_dl::decode(&out.display_list).unwrap();
        let rings: Vec<_> = ops
            .iter()
            .filter(|op| matches!(op, vy_dl::Op::Rect { color, .. } if *color == RING))
            .collect();
        assert_eq!(
            rings.len(),
            1,
            "exactly one focus ring, drawn by the module"
        );
        let link = out
            .semantic
            .iter()
            .filter(|n| n.role == vy_abi::role::LINK)
            .nth(2)
            .expect("third link");
        let vy_dl::Op::Rect { x, y, .. } = rings[0] else {
            unreachable!()
        };
        assert!((x - (link.x - 4.0)).abs() < 0.01 && (y - (link.y - 4.0)).abs() < 0.01);
        // No host overlay: the pixels are the display list and nothing else.
        let (pw, ph) = physical(1280, 800, 1.0);
        let golden = RgbaImage::decode_png(
            &fs::read(root().join("conformance/golden/document-1280x800@1-tab3.png")).unwrap(),
        )
        .unwrap();
        assert_eq!((pw, ph), (golden.width, golden.height));
        assert_eq!(out.image, golden, "the golden is the module's own drawing");
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
