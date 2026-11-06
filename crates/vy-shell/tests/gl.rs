//! SPEC-11.3.2: the window's GL path (spec-fråga 66) against the
//! reference, on a hidden window's context: the frame of SPEC-13.2.1,
//! random display lists and every frame of every reference page through a
//! script of hovers, clicks, scrolls and keys, each drawn by
//! `Renderer::render_gl`, read back and compared pixel by pixel with
//! `vy_render::render`. The tolerance asserted is SPEC-11.3.2's.
//!
//! Without a test harness (`harness = false`), so winit's event loop runs
//! on the main thread. Where no GL context can be made (no display, no
//! driver, GL older than 3.3) it says so on stderr and passes, unless
//! `VY_GL_REQUIRED` is set, as in CI's Mesa step.

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::path::{Path, PathBuf};
    use vy_abi::{button, event as kind, font, weight};
    use vy_dl::{DisplayList, Fit, PathCmd};
    use vy_render::{ImageCache, RasterPath, Renderer, RgbaImage, TextEngine};
    use vy_runtime::{Options, Runtime};
    use vy_shell::gl::GlContext;
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::window::{Window, WindowAttributes, WindowId};

    /// SPEC-11.3.2's tolerance: a pixel differs when a channel is more than
    /// [`CHANNEL_DELTA`] from the reference's; at most [`DIFFERING_SHARE`] of
    /// the pixels of a frame may (measured with Mesa llvmpipe: the reference
    /// pages at most 0.25 %, the frame of SPEC-13.2.1 0.15 %, random lists
    /// 2.0 %; the differences are anti-aliased edges and blending rounding).
    const CHANNEL_DELTA: u8 = 2;
    /// See [`CHANNEL_DELTA`].
    const DIFFERING_SHARE: f64 = 0.03;

    /// How far one frame is from the reference.
    #[derive(Clone, Copy, Default, Debug)]
    struct Diff {
        pixels: u64,
        /// Pixels with any channel off by at least one.
        any: u64,
        /// Pixels with a channel off by more than [`CHANNEL_DELTA`].
        over: u64,
        max: u8,
    }

    impl Diff {
        fn of(a: &RgbaImage, b: &RgbaImage) -> Self {
            assert_eq!((a.width, a.height), (b.width, b.height));
            let mut d = Diff {
                pixels: u64::from(a.width) * u64::from(a.height),
                ..Diff::default()
            };
            for (p, q) in a.data.chunks_exact(4).zip(b.data.chunks_exact(4)) {
                let m = p.iter().zip(q).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
                d.any += u64::from(m > 0);
                d.over += u64::from(m > CHANNEL_DELTA);
                d.max = d.max.max(m);
            }
            d
        }

        fn share(&self) -> f64 {
            self.over as f64 / self.pixels.max(1) as f64
        }

        fn add(&mut self, o: &Diff) {
            self.pixels += o.pixels;
            self.any += o.any;
            self.over += o.over;
            self.max = self.max.max(o.max);
        }
    }

    /// Per class of input: the sum, the worst frame and how many frames.
    #[derive(Default)]
    struct Class {
        sum: Diff,
        worst: Diff,
        worst_what: String,
        frames: usize,
        gpu_frames: usize,
        instances: usize,
        draws: usize,
    }

    impl Class {
        fn add(&mut self, d: Diff, what: String, gpu: bool, stats: vy_render::gl::GlStats) {
            self.sum.add(&d);
            if d.share() >= self.worst.share() {
                self.worst = d;
                self.worst_what = what;
            }
            self.frames += 1;
            self.gpu_frames += usize::from(gpu);
            self.instances += stats.instances;
            self.draws += stats.draws;
        }

        fn line(&self, name: &str) -> String {
            let pct = |n: u64, d: &Diff| n as f64 / d.pixels.max(1) as f64 * 100.0;
            format!(
                "{name:<10} {:>4} frames ({} on GL): {:.3} % of pixels differ at all, {:.3} % by more than {CHANNEL_DELTA}, max {}; worst frame {:.3} % ({}); {} instances, {} draws in all",
                self.frames,
                self.gpu_frames,
                pct(self.sum.any, &self.sum),
                pct(self.sum.over, &self.sum),
                self.sum.max,
                self.worst.share() * 100.0,
                self.worst_what,
                self.instances,
                self.draws,
            )
        }
    }

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    /// Draw `list` both ways and compare.
    #[allow(clippy::too_many_arguments)]
    fn compare(
        ctx: &mut GlContext,
        r: &mut Renderer,
        list: &[u8],
        w: u32,
        h: u32,
        scale: f32,
        text: &mut TextEngine,
        images: &ImageCache,
    ) -> Result<(Diff, bool, vy_render::gl::GlStats), String> {
        let want = vy_render::render(list, w, h, scale, text, images).map_err(|e| e.to_string())?;
        let report = r
            .render_gl(&ctx.gl, &mut ctx.raster, list, w, h, scale, text, images)
            .map_err(|e| e.to_string())?;
        assert_eq!(
            report, want.report,
            "the GL path's budget report is the reference's"
        );
        let got = ctx.raster.read_page(&ctx.gl).ok_or("no page")?;
        // VY_GL_DUMP=<dir> writes both pictures of every frame over 0.5 %.
        if let Some(dir) = std::env::var_os("VY_GL_DUMP") {
            let d = Diff::of(&got, &want.image);
            if d.share() > 0.005 {
                let n = std::fs::read_dir(&dir).map(|r| r.count()).unwrap_or(0);
                let dir = PathBuf::from(dir);
                std::fs::write(
                    dir.join(format!("{n:04}-gl.png")),
                    got.encode_png().unwrap(),
                )
                .unwrap();
                std::fs::write(
                    dir.join(format!("{n:04}-ref.png")),
                    want.image.encode_png().unwrap(),
                )
                .unwrap();
            }
        }
        let gpu = r.info().gpu;
        let stats = if gpu {
            ctx.raster.stats()
        } else {
            Default::default()
        };
        Ok((Diff::of(&got, &want.image), gpu, stats))
    }

    /// A deterministic generator (xorshift).
    struct Gen(u64);

    impl Gen {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        fn coord(&mut self, max: f32) -> f32 {
            let whole = self.below(max as u64 + 20) as f32 - 10.0;
            match self.below(4) {
                0 | 1 => whole,
                2 => whole + 0.5,
                _ => whole + self.below(1000) as f32 / 1000.0,
            }
        }
        fn length(&mut self, max: f32) -> f32 {
            match self.below(3) {
                0 => (1 + self.below(max as u64)) as f32,
                _ => self.below((max * 1000.0) as u64) as f32 / 1000.0 + 0.5,
            }
        }
        fn color(&mut self) -> u32 {
            let rgb = (self.next() as u32) & 0xFFFF_FF00;
            match self.below(10) {
                0..=5 => rgb | 0xFF,
                _ => rgb | (1 + self.below(254)) as u32,
            }
        }
    }

    /// A random page-like list: rectangles plain and rounded, nested plain and
    /// rounded clips, text in three styles, images in three fits, paths.
    fn random_list(g: &mut Gen, styles: &[u16; 3], w: f32, h: f32) -> Vec<u8> {
        const TEXTS: [&str; 3] = ["Vy ritar text", "Lägesrapport: allt grönt", "0x2F5B 12 x 7"];
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.rect(0.0, 0.0, w, h, 0.0, g.color() | 0xFF).unwrap();
        let mut depth = 0;
        for _ in 0..(20 + g.below(60)) {
            match g.below(20) {
                0..=6 => {
                    let radius = if g.below(5) < 3 { 0.0 } else { g.length(14.0) };
                    let (x, y) = (g.coord(w), g.coord(h));
                    dl.rect(
                        x,
                        y,
                        g.length(w / 2.0),
                        g.length(h / 2.0),
                        radius,
                        g.color(),
                    )
                    .unwrap();
                }
                7..=8 if depth < 4 => {
                    let radius = if g.below(2) == 0 { 0.0 } else { g.length(12.0) };
                    let (x, y) = (g.coord(w), g.coord(h));
                    dl.clip_push(x, y, g.length(w), g.length(h), radius)
                        .unwrap();
                    depth += 1;
                }
                9..=10 if depth > 0 => {
                    dl.clip_pop().unwrap();
                    depth -= 1;
                }
                11..=15 => {
                    let style = styles[g.below(3) as usize];
                    let text = TEXTS[g.below(3) as usize];
                    dl.text(
                        g.coord(w),
                        g.coord(h),
                        [0.0, 80.0][g.below(2) as usize],
                        style,
                        text.as_bytes(),
                    )
                    .unwrap();
                }
                16..=17 => {
                    let fit = [Fit::Cover, Fit::Contain, Fit::Stretch][g.below(3) as usize];
                    let (x, y) = (g.coord(w), g.coord(h));
                    dl.image(x, y, g.length(w / 2.0), g.length(h / 2.0), 1, fit)
                        .unwrap();
                }
                _ => {
                    let stroke = if g.below(2) == 0 { 0.0 } else { 2.0 };
                    let (x, y) = (g.coord(w), g.coord(h));
                    let cmds = [
                        PathCmd::MoveTo(x, y),
                        PathCmd::LineTo(x + g.length(40.0), y + g.length(8.0)),
                        PathCmd::QuadTo {
                            ctrl: (x + 30.0, y + 40.0),
                            to: (x + g.length(12.0), y + g.length(30.0)),
                        },
                        PathCmd::Close,
                    ];
                    dl.path(g.color(), stroke, &cmds).unwrap();
                }
            }
        }
        for _ in 0..depth {
            dl.clip_pop().unwrap();
        }
        dl.end().unwrap();
        dl.bytes().to_vec()
    }

    /// A 9 × 7 picture with every kind of alpha, decoded as resource 1.
    fn picture() -> ImageCache {
        let mut img = RgbaImage::new(9, 7);
        for (i, p) in img.data.chunks_exact_mut(4).enumerate() {
            let a = [255u8, 0, 128, 1, 254, 77][i % 6];
            p.copy_from_slice(&[(i * 29) as u8, (i * 53) as u8, (i * 91) as u8, a]);
        }
        let mut c = ImageCache::from_bytes(vec![img.encode_png().unwrap()]);
        c.decode(1).unwrap();
        c
    }

    /// The user's script of vy-cli's page test (`raster_tests.rs`).
    fn script(w: f32, h: f32) -> Vec<(i32, i32, i32, i32, i32)> {
        let at = |fx: f32, fy: f32| ((w * fx * 100.0) as i32, (h * fy * 100.0) as i32);
        let mut steps = Vec::new();
        for (fx, fy) in [(0.1, 0.1), (0.3, 0.2), (0.5, 0.35), (0.7, 0.5), (0.2, 0.6)] {
            let (x, y) = at(fx, fy);
            steps.push((kind::POINTER_MOVE, x, y, 0, 0));
        }
        for (fx, fy) in [(0.25, 0.3), (0.5, 0.5), (0.15, 0.85)] {
            let (x, y) = at(fx, fy);
            steps.push((kind::POINTER_DOWN, x, y, button::PRIMARY, 0));
            steps.push((kind::POINTER_UP, x, y, button::PRIMARY, 0));
        }
        for dy in [2_400, 4_800, -2_400] {
            steps.push((kind::SCROLL, 0, dy, 0, 0));
        }
        for code in [81, 44] {
            steps.push((kind::KEY_DOWN, code, 0, 0, 0));
            steps.push((kind::KEY_UP, code, 0, 0, 0));
        }
        steps
    }

    struct Outcome {
        lines: Vec<String>,
        failures: Vec<String>,
    }

    fn run(ctx: &mut GlContext) -> Result<Outcome, String> {
        let mut lines = vec![format!("GL: {}", ctx.raster.version())];
        let mut failures = Vec::new();
        let mut check = |class: &str, d: &Diff, what: &str| {
            if d.share() > DIFFERING_SHARE {
                failures.push(format!(
                    "{class} {what}: {:.3} % of pixels differ by more than {CHANNEL_DELTA} (max {})",
                    d.share() * 100.0,
                    d.max
                ));
            }
        };

        // The frame of SPEC-13.2.1, at two scales.
        let mut frame = Class::default();
        for (w, h, scale) in [(1280u32, 800u32, 1.0f32), (1280, 800, 2.0)] {
            let mut t = TextEngine::new();
            let f = vy_render::bench::frame_2000(&mut t, w as f32 / scale, h as f32 / scale);
            let mut r = Renderer::new(RasterPath::Fast);
            // Twice: the second frame finds its glyphs in the atlas.
            for pass in 0..2 {
                let (d, gpu, stats) =
                    compare(ctx, &mut r, &f.list, w, h, scale, &mut t, &f.images)?;
                let what = format!("{w}x{h}@{scale} pass {pass}");
                check("frame_2000", &d, &what);
                frame.add(d, what, gpu, stats);
            }
        }
        lines.push(frame.line("frame_2000"));

        // Random lists.
        let images = picture();
        let mut random = Class::default();
        let mut g = Gen(0x9E37_79B9_7F4A_7C15);
        let mut t = TextEngine::new();
        let styles = [
            t.style(font::SANS, 13.0, weight::NORMAL, 0x2221_1FFF)
                .unwrap(),
            t.style(font::SERIF, 16.0, weight::BOLD, 0x3366_99C0)
                .unwrap(),
            t.style(font::MONO, 11.0, weight::NORMAL, 0xD64B_2FFF)
                .unwrap(),
        ];
        let mut r = Renderer::new(RasterPath::Fast);
        for case in 0..120 {
            let (w, h) = [(320u32, 200u32), (257, 129), (160, 300)][case % 3];
            let scale = [1.0f32, 1.5, 2.0][(case / 3) % 3];
            let list = random_list(&mut g, &styles, w as f32 / scale, h as f32 / scale);
            let (d, gpu, stats) = compare(ctx, &mut r, &list, w, h, scale, &mut t, &images)?;
            let what = format!("case {case} {w}x{h}@{scale}");
            check("random", &d, &what);
            random.add(d, what, gpu, stats);
        }
        lines.push(random.line("random"));

        // Every reference page, frame by frame.
        let dir = root().join("target/wasm");
        let mut modules: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|e| format!("{}: {e} (run tools/build_modules.sh)", dir.display()))?
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "wasm"))
            .collect();
        modules.sort();
        let mut pages = Class::default();
        for module in &modules {
            let name = module.file_stem().unwrap().to_string_lossy().into_owned();
            let wasm = std::fs::read(module).map_err(|e| e.to_string())?;
            for (w, h, scale) in [(800u32, 600u32, 1.0f32), (480, 320, 2.0)] {
                let hash = vy_runtime::sha256(&wasm);
                let mut rt =
                    Runtime::load(&wasm, &hash, Options::default()).map_err(|e| e.to_string())?;
                rt.init(w, h, scale).map_err(|e| e.to_string())?;
                let (pw, ph) = (
                    (w as f32 * scale).round() as u32,
                    (h as f32 * scale).round() as u32,
                );
                let mut r = Renderer::new(RasterPath::Fast);
                let mut lists = vec![rt.frame().map_err(|e| e.to_string())?.to_vec()];
                for (ev, a, b, c, d) in script(w as f32, h as f32) {
                    let Ok(redraw) = rt.event(ev, a, b, c, d) else {
                        break;
                    };
                    if (redraw || rt.wants_frame()) && !rt.is_suspended() {
                        if let Ok(l) = rt.frame() {
                            lists.push(l.to_vec());
                        }
                    }
                }
                for (k, list) in lists.iter().enumerate() {
                    let (text, images) = rt.raster_parts();
                    let (d, gpu, stats) = compare(ctx, &mut r, list, pw, ph, scale, text, images)?;
                    let what = format!("{name} {w}x{h}@{scale} frame {k}");
                    check("pages", &d, &what);
                    pages.add(d, what, gpu, stats);
                }
            }
        }
        lines.push(pages.line("pages"));
        Ok(Outcome { lines, failures })
    }

    /// What the hidden window found.
    enum Found {
        /// No context: a skip, unless one is required.
        NoContext(String),
        /// A context, and an error while drawing on it: a failure.
        Broke(String),
        Ran(Outcome),
    }

    #[derive(Default)]
    struct App {
        window: Option<Window>,
        outcome: Option<Found>,
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, el: &ActiveEventLoop) {
            if self.outcome.is_some() {
                return;
            }
            let attrs = WindowAttributes::default()
                .with_visible(false)
                .with_title("vy gl test");
            self.outcome = Some(match vy_shell::gl::open(el, attrs) {
                Ok((window, mut ctx)) => {
                    let out = run(&mut ctx);
                    drop(ctx);
                    self.window = Some(window);
                    match out {
                        Ok(o) => Found::Ran(o),
                        Err(e) => Found::Broke(e),
                    }
                }
                Err(e) => Found::NoContext(format!("no GL context: {}", e.why)),
            });
            el.exit();
        }

        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }

    pub fn main() {
        let required = std::env::var_os("VY_GL_REQUIRED").is_some();
        let skip = |why: String| {
            if required {
                eprintln!("gl: VY_GL_REQUIRED is set and {why}");
                std::process::exit(1);
            }
            eprintln!("gl: skipped, {why}");
            std::process::exit(0);
        };
        // A plain `cargo test` filter that names other tests skips this one.
        let args: Vec<String> = std::env::args()
            .skip(1)
            .filter(|a| !a.starts_with('-'))
            .collect();
        if !args.is_empty()
            && !args
                .iter()
                .any(|a| "spec_11_3_2_gl_path".contains(a.as_str()))
        {
            return;
        }
        let el = match EventLoop::new() {
            Ok(el) => el,
            Err(e) => return skip(format!("no event loop: {e}")),
        };
        let mut app = App::default();
        if let Err(e) = el.run_app(&mut app) {
            return skip(format!("event loop: {e}"));
        }
        match app.outcome {
            None => skip("the event loop never resumed".into()),
            Some(Found::NoContext(why)) => skip(why),
            Some(Found::Broke(why)) => {
                println!("test spec_11_3_2_gl_path ... FAILED: {why}");
                std::process::exit(1);
            }
            Some(Found::Ran(out)) => {
                println!("test spec_11_3_2_gl_path ...");
                for l in &out.lines {
                    println!("  {l}");
                }
                if out.failures.is_empty() {
                    println!("test spec_11_3_2_gl_path ... ok");
                } else {
                    for f in &out.failures {
                        println!("  over the tolerance: {f}");
                    }
                    println!("test spec_11_3_2_gl_path ... FAILED");
                    std::process::exit(1);
                }
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    imp::main();
}

/// macOS draws on the CPU path (docs/notes/m5-gpu.md): nothing to test.
#[cfg(target_os = "macos")]
fn main() {
    eprintln!("gl: skipped, macOS has no GL path");
}
