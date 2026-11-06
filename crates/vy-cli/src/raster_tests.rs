//! SPEC-11.3.2 and SPEC-2.1.5 over the reference pages: the window's
//! rasterizer (`vy_render::Renderer` on its fast path: text runs kept
//! between frames, damage tracking, bands on threads) draws every frame of
//! every reference page byte for byte as `vy_render::render`, the
//! reference and the conformance suite's path, does.

use std::path::{Path, PathBuf};
use vy_abi::{button, event as kind};
use vy_render::{RasterPath, Renderer};
use vy_runtime::{Options, Runtime};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// What a user does to a page between frames, in logical pixels: pointer
/// moves over a grid (hover), presses and releases, scrolls down and back,
/// keys. Scaled to the window by the caller.
fn script(w: f32, h: f32) -> Vec<(i32, i32, i32, i32, i32)> {
    let at = |fx: f32, fy: f32| ((w * fx * 100.0) as i32, (h * fy * 100.0) as i32);
    let mut steps = Vec::new();
    for (fx, fy) in [
        (0.1, 0.1),
        (0.3, 0.2),
        (0.5, 0.35),
        (0.7, 0.5),
        (0.2, 0.6),
        (0.5, 0.8),
    ] {
        let (x, y) = at(fx, fy);
        steps.push((kind::POINTER_MOVE, x, y, 0, 0));
    }
    for (fx, fy) in [(0.25, 0.3), (0.5, 0.5), (0.15, 0.85)] {
        let (x, y) = at(fx, fy);
        steps.push((kind::POINTER_DOWN, x, y, button::PRIMARY, 0));
        steps.push((kind::POINTER_UP, x, y, button::PRIMARY, 0));
    }
    for dy in [2_400, 2_400, 4_800, -2_400] {
        steps.push((kind::SCROLL, 0, dy, 0, 0));
    }
    // Arrow down, arrow up, space (USB HID).
    for code in [81, 82, 44] {
        steps.push((kind::KEY_DOWN, code, 0, 0, 0));
        steps.push((kind::KEY_UP, code, 0, 0, 0));
    }
    steps
}

/// SPEC-11.3.2, SPEC-2.1.5, SPEC-13.2.1: the window may redraw only what
/// changed and draw in bands, but every frame it draws is the headless
/// frame byte for byte. Every reference module, at two sizes and scales
/// and in one and three bands, through its first frame,
/// a script of hovers, clicks, scrolls and keys and up to five frames of
/// animation after each: each frame drawn by one `Renderer` (which redraws
/// only what changed since its last frame) is the reference's frame, byte
/// for byte and report for report, and many of them are partial redraws.
#[test]
fn spec_11_3_2_reference_pages_through_both_paths() {
    let dir = root().join("target/wasm");
    let mut modules: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            panic!(
                "{}: {e}\nrun tools/build_modules.sh before cargo test",
                dir.display()
            )
        })
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "wasm"))
        .collect();
    modules.sort();
    assert!(modules.len() >= 10, "{} modules", modules.len());
    let (mut frames, mut partial, mut unchanged) = (0, 0, 0);
    for module in &modules {
        let name = module.file_stem().unwrap().to_string_lossy().into_owned();
        let wasm = std::fs::read(module).unwrap();
        for (w, h, scale, bands) in [(640u32, 400u32, 1.0f32, 1usize), (420, 300, 1.5, 3)] {
            let hash = vy_runtime::sha256(&wasm);
            let mut rt = Runtime::load(&wasm, &hash, Options::default()).unwrap();
            rt.init(w, h, scale).unwrap();
            let (pw, ph) = crate::physical(w, h, scale);
            let mut fast = Renderer::with_bands(bands);
            assert_eq!(fast.path(), RasterPath::Fast);
            let mut check = |rt: &mut Runtime, list: Vec<u8>, what: &str| {
                let (text, images) = rt.raster_parts();
                let want = vy_render::render(&list, pw, ph, scale, text, images).unwrap();
                let report = fast.render(&list, pw, ph, scale, text, images).unwrap();
                assert_eq!(report, want.report, "{name} {w}x{h}@{scale}: {what}");
                if fast.image().data != want.image.data {
                    let i = fast
                        .image()
                        .data
                        .chunks_exact(4)
                        .zip(want.image.data.chunks_exact(4))
                        .position(|(a, b)| a != b)
                        .unwrap();
                    panic!(
                        "{name} {w}x{h}@{scale}, {bands} bands, {what}: pixel ({}, {}) differs ({:?})",
                        i % pw as usize,
                        i / pw as usize,
                        fast.info()
                    );
                }
                frames += 1;
                match fast.info() {
                    i if i.redrawn.is_none() => unchanged += 1,
                    i if !i.full => partial += 1,
                    _ => {}
                }
            };
            let list = rt.frame().unwrap().to_vec();
            check(&mut rt, list, "first frame");
            for (k, (ev, a, b, c, d)) in script(w as f32, h as f32).into_iter().enumerate() {
                let Ok(redraw) = rt.event(ev, a, b, c, d) else {
                    break;
                };
                let mut n = 0;
                let mut want_frame = redraw;
                while (want_frame || rt.wants_frame()) && n < 5 && !rt.is_suspended() {
                    let Ok(list) = rt.frame().map(<[u8]>::to_vec) else {
                        break;
                    };
                    check(&mut rt, list, &format!("step {k} ({ev}), frame {n}"));
                    want_frame = false;
                    n += 1;
                }
            }
        }
    }
    eprintln!("{frames} frames: {partial} partial redraws, {unchanged} unchanged");
    assert!(
        partial >= 10,
        "{partial} of {frames} frames were partial redraws"
    );
}
