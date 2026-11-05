//! Headless tools for the Vy client: `check-abi`, `render`, `conformance` (SPEC §11.3, §12.2).
//!
//! Everything the binary does lives here so the gates can call it as a library.
#![deny(missing_docs)]

pub mod abi_doc;
pub mod conformance;

use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use vy_abi::event as kind;
use vy_render::RgbaImage;
use vy_runtime::{Capabilities, LogEntry, Options, Runtime, SemNode, Storage};

pub use vy_runtime::{check_abi_bytes, AbiReport, CannedResponse, Resource};

/// An input or host event replayed before the final frame (SPEC-5.1.3,
/// SPEC-A.2.1), in logical pixels: what a conformance case needs to reach a
/// focus, hover, scroll, paused, reflowed or persisted state. The shell
/// delivers the same events from a window in M3.
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
    /// `FOCUS` set directly on a node (SPEC-10.3.1): the host's focus moves
    /// there as if its own order had reached it.
    Focus {
        /// Semantic node id.
        node: i32,
        /// True when the node gains focus.
        gained: bool,
        /// True when the change came from the keyboard.
        keyboard: bool,
    },
    /// `VISIBILITY`: the tab is hidden or shown (SPEC-6.2.1); hidden, no
    /// frames run until it is shown again.
    Visibility {
        /// True when shown.
        visible: bool,
    },
    /// The next frame is reported as over its budget (SPEC-5.3.3) whatever
    /// it costs: the module gets `BUDGET`, and a second in a row suspends
    /// it, after which the case runs no more frames or events.
    Budget,
    /// `vy_resize` (SPEC-5.1.4); the final image has the last size.
    Resize {
        /// New logical width.
        width: u32,
        /// New logical height.
        height: u32,
        /// New scale.
        scale: f32,
    },
    /// `vy_suspend` (SPEC-5.1.5): the page may write its state away.
    Suspend,
    /// A fresh instance of the module with the storage the old one saved
    /// (SPEC-5.2.9): `vy_init` again at the current size, as after the tab
    /// was evicted and reopened; the old instance's logs are kept.
    Reinit,
    /// `FETCH_DONE` for the oldest request still waiting for it, answered
    /// from the case's canned responses (SPEC-5.2.7); an error when no
    /// request is waiting.
    FetchDone,
}

/// One scripted step: an event, and how many frames may run before it.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    /// The event.
    pub event: ScriptedEvent,
    /// Most frames to run between the previous step (or `vy_init`) and
    /// this event, while the module keeps asking for them; `None` runs
    /// until it stops asking or the ceiling is reached. This is how a case
    /// reaches a page mid-animation: `after_frames = 20` on a `visibility`
    /// event pauses a game twenty frames after the serve.
    pub after_frames: Option<u32>,
}

impl From<ScriptedEvent> for Step {
    fn from(event: ScriptedEvent) -> Self {
        Self {
            event,
            after_frames: None,
        }
    }
}

impl ScriptedEvent {
    /// Deliver an input event to the runtime; true when the module asked
    /// for a redraw. The host-side events are the runner's (see [`render`]).
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
            Self::Visibility { visible } => {
                rt.event(kind::VISIBILITY, i32::from(*visible), 0, 0, 0)
            }
            Self::Budget => {
                rt.force_budget_breach();
                Ok(false)
            }
            Self::Suspend => rt.suspend().map(|_| false),
            Self::FetchDone => match rt.fetch_done()? {
                Some(redraw) => Ok(redraw),
                None => Ok(false),
            },
            Self::Resize { .. } | Self::Reinit => {
                unreachable!("handled by the runner, which owns the size and the instance")
            }
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
    /// Steps taken after the first frame, in order; each event the module
    /// answers with a redraw gets a frame while `frames` allows (SPEC-6.2.1).
    pub events: &'a [Step],
    /// Responses the headless host serves for `fetch_begin` (SPEC-5.2.7).
    pub fetch: &'a [CannedResponse],
    /// Origins the manifest lists under `net` (SPEC-5.2.8); empty skips the gate.
    pub net_origins: &'a [String],
    /// Storage quota in bytes (SPEC-9.3.1): the manifest's `quota_kb` × 1024.
    pub storage_quota_bytes: u64,
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
            fetch: &[],
            net_origins: &[],
            storage_quota_bytes: vy_runtime::DEFAULT_STORAGE_QUOTA_BYTES,
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
    /// Module and host log lines, across a `reinit`.
    pub logs: Vec<LogEntry>,
    /// Frames actually run: 1 to `RenderRequest::frames` (SPEC-6.2.1).
    pub frames_run: u32,
    /// True when the module was force-suspended by the budget (SPEC-5.3.3).
    pub suspended: bool,
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
/// in all, and never after a forced suspend (SPEC-5.3.3). Before each step
/// the frames run are capped by its `after_frames`. A `resize` changes the
/// size the final image has; a `reinit` replaces the instance with a fresh
/// one that starts with the old one's storage.
pub fn render(req: &RenderRequest<'_>) -> Result<RenderOutput> {
    if req.frames == 0 {
        bail!("frames must be at least 1");
    }
    let check_scale = |scale: f32| -> Result<()> {
        if !(scale.is_finite() && scale > 0.0 && scale <= 8.0) {
            bail!("scale must be in (0, 8]");
        }
        Ok(())
    };
    check_scale(req.scale)?;
    let hash = vy_runtime::sha256(req.wasm);
    let load = |storage: Storage| -> Result<Runtime> {
        let options = Options {
            memory_max_pages: req.memory_max_pages,
            capabilities: req.capabilities,
            resources: req.resources.to_vec(),
            storage,
            storage_quota_bytes: req.storage_quota_bytes,
            fetch: req.fetch.to_vec(),
            net_origins: req.net_origins.to_vec(),
        };
        Runtime::load(req.wasm, &hash, options).map_err(|e| anyhow!("{e}"))
    };
    let (mut width, mut height, mut scale) = (req.width, req.height, req.scale);
    let mut rt = load(Storage::new())?;
    rt.init(width, height, scale).map_err(|e| anyhow!("{e}"))?;
    let mut display_list = Vec::new();
    let mut frames_run = 0;
    let mut earlier_logs: Vec<LogEntry> = Vec::new();
    let mut run_frames =
        |rt: &mut Runtime, display_list: &mut Vec<u8>, cap: Option<u32>| -> Result<()> {
            let mut ran = 0;
            while frames_run < req.frames
                && rt.wants_frame()
                && !rt.is_suspended()
                && cap.is_none_or(|c| ran < c)
            {
                *display_list = rt
                    .frame()
                    .map_err(|e| anyhow!("frame {}: {e}", frames_run + 1))?
                    .to_vec();
                frames_run += 1;
                ran += 1;
            }
            Ok(())
        };
    let mut steps = req.events.iter().enumerate().peekable();
    loop {
        let cap = steps.peek().and_then(|(_, s)| s.after_frames);
        run_frames(&mut rt, &mut display_list, cap)?;
        let Some((i, step)) = steps.next() else {
            break;
        };
        if rt.is_suspended() {
            bail!(
                "event {}: the module is suspended after two budget breaches (SPEC-5.3.3)",
                i + 1
            );
        }
        match &step.event {
            ScriptedEvent::Resize {
                width: w,
                height: h,
                scale: s,
            } => {
                check_scale(*s)?;
                (width, height, scale) = (*w, *h, *s);
                rt.resize(width, height, scale)
                    .map_err(|e| anyhow!("event {}: {e}", i + 1))?;
            }
            ScriptedEvent::Reinit => {
                earlier_logs.extend(rt.logs().iter().cloned());
                let storage = rt.take_storage();
                rt = load(storage)?;
                rt.init(width, height, scale)
                    .map_err(|e| anyhow!("event {}: reinit: {e}", i + 1))?;
            }
            ScriptedEvent::FetchDone if rt.pending_fetches() == 0 => {
                bail!(
                    "event {}: fetch_done with no request waiting for FETCH_DONE",
                    i + 1
                );
            }
            other => {
                other
                    .deliver(&mut rt)
                    .map_err(|e| anyhow!("event {}: {e}", i + 1))?;
            }
        }
    }
    debug_assert!(frames_run >= 1, "init wants the first frame");
    let (pw, ph) = physical(width, height, scale);
    let (text, images) = rt.raster_parts();
    let image = vy_render::render(&display_list, pw, ph, scale, text, images)
        .map_err(|e| anyhow!("{e}"))?;
    earlier_logs.extend(rt.logs().iter().cloned());
    Ok(RenderOutput {
        image,
        display_list,
        semantic_json: rt.semantic_tree_json(),
        semantic: rt.semantic_tree().to_vec(),
        logs: earlier_logs,
        frames_run,
        suspended: rt.is_suspended(),
    })
}

/// What `vy-cli render` renders a module file with.
#[derive(Clone, Debug)]
pub struct FileRender {
    /// Logical width.
    pub width: u32,
    /// Logical height.
    pub height: u32,
    /// Scale (SPEC-6.3.1).
    pub scale: f32,
    /// Most frames to run (SPEC-6.2.1).
    pub frames: u32,
    /// Capabilities granted (`--grant`).
    pub capabilities: Capabilities,
    /// Resources by id (`--resource`, `--resource-size`; SPEC-4.5.1).
    pub resources: Vec<Resource>,
    /// Where to write the semantic tree (`--dump-semantic`, SPEC-10.2.3).
    pub dump_semantic: Option<PathBuf>,
}

/// `render` for a module file; writes the PNG and optionally the semantic JSON.
pub fn render_to_files(module: &str, out: &Path, settings: &FileRender) -> Result<RenderOutput> {
    if module.starts_with("vy://") {
        bail!("fetching vy:// addresses arrives in M3; give a .wasm file");
    }
    let wasm = fs::read(module).with_context(|| format!("reading {module}"))?;
    let mut req = RenderRequest::new(&wasm, settings.width, settings.height);
    req.scale = settings.scale;
    req.frames = settings.frames;
    req.capabilities = settings.capabilities;
    req.resources = &settings.resources;
    let output = render(&req)?;
    fs::write(out, output.image.encode_png()?)
        .with_context(|| format!("writing {}", out.display()))?;
    if let Some(path) = &settings.dump_semantic {
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
            &FileRender {
                width: 800,
                height: 600,
                scale: 1.0,
                frames: 1,
                capabilities: Capabilities::default(),
                resources: Vec::new(),
                dump_semantic: Some(json.clone()),
            },
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
        let events: Vec<Step> = events.into_iter().map(Step::from).collect();
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
        let events: Vec<Step> = [tab.clone(), tab.clone(), tab]
            .into_iter()
            .map(Step::from)
            .collect();
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

    fn module(name: &str) -> Vec<u8> {
        fs::read(root().join(format!("target/wasm/{name}.wasm"))).unwrap()
    }

    fn key(code: i32) -> ScriptedEvent {
        ScriptedEvent::KeyDown {
            code,
            modifiers: 0,
            repeat: false,
        }
    }

    fn after(event: ScriptedEvent, frames: u32) -> Step {
        Step {
            event,
            after_frames: Some(frames),
        }
    }

    const SPACE: i32 = 0x2C;

    /// SPEC-5.1.4, SPEC-11.3.1: a scripted `resize` reaches the module as
    /// `vy_resize` and the final image has the new size; the document page
    /// reflows so completely that pixels and tree equal a fresh instance
    /// started at that size.
    #[test]
    fn spec_5_1_4_resize_step_reflows_like_a_fresh_init() {
        let wasm = module("document");
        let steps = [Step::from(ScriptedEvent::Resize {
            width: 400,
            height: 700,
            scale: 1.0,
        })];
        let mut req = RenderRequest::new(&wasm, 1280, 800);
        req.events = &steps;
        req.frames = 2;
        let resized = render(&req).unwrap();
        assert_eq!(resized.frames_run, 2, "a window change wants a frame");
        assert_eq!((resized.image.width, resized.image.height), (400, 700));
        let fresh = render(&RenderRequest::new(&wasm, 400, 700)).unwrap();
        assert_eq!(resized.image, fresh.image);
        assert_eq!(resized.semantic_json, fresh.semantic_json);
    }

    /// SPEC-6.2.1, SPEC-5.1.2, SPEC-A.2.1: `after_frames` caps the frames
    /// before a step; a hidden tab runs no frames however much the game
    /// asks; shown again it draws once (the pause overlay) with `dt_ms` 0.
    #[test]
    fn spec_6_2_1_visibility_step_runs_no_frames_hidden() {
        let wasm = module("game");
        let steps = [
            Step::from(key(SPACE)),
            after(ScriptedEvent::Visibility { visible: false }, 20),
            Step::from(ScriptedEvent::Visibility { visible: true }),
        ];
        let mut req = RenderRequest::new(&wasm, 800, 600);
        req.events = &steps;
        req.frames = 100;
        let out = render(&req).unwrap();
        assert_eq!(
            out.frames_run,
            1 + 20 + 1,
            "init, twenty in flight, one for the overlay; none while hidden"
        );
        assert!(out.semantic.iter().any(|n| n.text.starts_with("Pausad")));
        assert!(
            out.semantic.iter().any(|n| n.text.contains("0,3 s")),
            "20 frames of 16 ms"
        );
        assert!(out.logs.iter().any(|l| l.message == "game: hidden, paused"));
    }

    /// SPEC-5.3.3: a `budget` step makes the next frame a reported breach:
    /// the game gets `KIND_BUDGET` and drops detail; two in a row suspend
    /// the module, after which no frame runs and a further event is an
    /// error rather than a call into a suspended module.
    #[test]
    fn spec_5_3_3_budget_step_delivers_kind_budget() {
        let wasm = module("game");
        let steps = [Step::from(key(SPACE)), after(ScriptedEvent::Budget, 10)];
        let mut req = RenderRequest::new(&wasm, 800, 600);
        req.events = &steps;
        req.frames = 30;
        let out = render(&req).unwrap();
        assert_eq!(
            out.frames_run, 30,
            "the ceiling; one breach does not stop it"
        );
        assert!(!out.suspended);
        assert!(out
            .logs
            .iter()
            .any(|l| l.message == "game: 9 ms frame, dropping stars and trail"));

        let twice = [
            Step::from(key(SPACE)),
            after(ScriptedEvent::Budget, 5),
            after(ScriptedEvent::Budget, 1),
        ];
        req.events = &twice;
        let out = render(&req).unwrap();
        assert!(
            out.suspended,
            "second consecutive breach: forced vy_suspend"
        );
        assert_eq!(
            out.frames_run,
            1 + 5 + 1 + 1,
            "init, five, the first breach, the second; none after the suspend"
        );
        let too_late = [
            Step::from(key(SPACE)),
            after(ScriptedEvent::Budget, 5),
            after(ScriptedEvent::Budget, 1),
            Step::from(key(SPACE)),
        ];
        req.events = &too_late;
        let err = render(&req).unwrap_err().to_string();
        assert!(err.contains("SPEC-5.3.3"), "{err}");
    }

    /// SPEC-5.2.9, SPEC-5.1.5, SPEC-5.1.1: a `reinit` step starts a fresh
    /// instance with the storage the old one saved on change and in
    /// `vy_suspend`, so the settings page comes back with its values; the
    /// logs of both instances are kept.
    #[test]
    fn spec_5_2_9_reinit_step_keeps_storage() {
        let wasm = module("settings");
        let press = |x: f32, y: f32| ScriptedEvent::PointerDown {
            x,
            y,
            button: 1,
            modifiers: 0,
        };
        let steps: Vec<Step> = [
            press(354.0, 156.0),
            press(354.0, 200.0),
            ScriptedEvent::Suspend,
            ScriptedEvent::Reinit,
        ]
        .into_iter()
        .map(Step::from)
        .collect();
        let mut req = RenderRequest::new(&wasm, 800, 600);
        req.capabilities = Capabilities::from_names(["storage"]).unwrap();
        req.events = &steps;
        req.frames = 10;
        let out = render(&req).unwrap();
        let status = out
            .semantic
            .iter()
            .find(|n| n.text.contains("laddade"))
            .unwrap();
        assert_eq!(status.text, "Sparade inställningar laddade.");
        assert!(out.semantic.iter().any(|n| n.text == "Mörkt tema: på"));
        assert!(out.semantic.iter().any(|n| n.text == "Kompakt läge: på"));
        assert_eq!(
            out.logs
                .iter()
                .filter(|l| l.message == "settings: init")
                .count(),
            2,
            "both instances logged their init"
        );
        // Without storage the second instance starts from the defaults.
        req.capabilities = Capabilities::default();
        let out = render(&req).unwrap();
        assert!(out
            .semantic
            .iter()
            .any(|n| n.text.starts_with("Lagring otillgänglig (E_DENIED)")));
        assert!(out.semantic.iter().any(|n| n.text == "Mörkt tema: av"));
    }

    /// SPEC-5.2.7, SPEC-A.2.1: with a canned response `fetch_begin` is
    /// answered, a `fetch_done` step delivers `FETCH_DONE` and the chat page
    /// marks the message delivered after reading the body to its end; a
    /// `fetch_done` with nothing waiting is an error in the case.
    #[test]
    fn spec_5_2_7_fetch_done_step() {
        let wasm = module("chat");
        let canned = [CannedResponse {
            url: "https://chat.example/api/messages".into(),
            status: 200,
            body: b"ok".to_vec(),
        }];
        let origins = ["https://chat.example".to_string()];
        let steps: Vec<Step> = [
            key(vy_abi::key::TAB),
            ScriptedEvent::TextInput("Hej".into()),
            key(vy_abi::key::ENTER),
            ScriptedEvent::FetchDone,
        ]
        .into_iter()
        .map(Step::from)
        .collect();
        let mut req = RenderRequest::new(&wasm, 800, 600);
        req.capabilities = Capabilities::from_names(["ime", "net"]).unwrap();
        req.fetch = &canned;
        req.net_origins = &origins;
        req.events = &steps;
        req.frames = 10;
        let out = render(&req).unwrap();
        assert!(out
            .semantic
            .iter()
            .any(|n| n.text.ends_with("levererat: Hej")));
        assert!(out
            .logs
            .iter()
            .any(|l| l.message == "chat: delivered, read 2 of 2 bytes"));
        req.fetch = &[];
        let err = render(&req).unwrap_err().to_string();
        assert!(err.contains("fetch_done with no request waiting"), "{err}");
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
