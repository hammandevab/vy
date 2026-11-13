//! SPEC-11.3.2: the window's GL path (spec-fråga 66) against the
//! reference, on a hidden window's context: the frame of SPEC-13.2.1,
//! random display lists and every frame of every reference page through a
//! script of hovers, clicks, scrolls and keys, each drawn by
//! `Renderer::render_gl`, read back and compared pixel by pixel with
//! `vy_render::render`. The tolerance asserted is SPEC-11.3.2's.
//!
//! SPEC-9.6.1 in the GL window (granskning 0011:2): the composition the
//! window shows, read back from the default framebuffer. `GlRaster::present`
//! with a page larger than its area under a frame of known pixels, and
//! `GlContext::compose` with a real shell (a page, the devtools panel, a
//! zoomed page, a page that paints far outside its canvas): the chrome's
//! pixels byte for byte the CPU composition's, and the page nowhere but
//! inside the page area. And a page whose list does not decode shows no
//! page, not the picture another tab left in the framebuffer (granskning
//! 0011:3).
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
    use vy_render::gl::PageAt;
    use vy_render::{ImageCache, RasterPath, Renderer, RgbaImage, TextEngine};
    use vy_runtime::{Options, Runtime};
    use vy_shell::gl::{GlContext, PageState};
    use vy_shell::headless::{headless_shell, settle};
    use vy_shell::page::Source;
    use vy_shell::shell::Shell;
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
    /// The hidden window's size in physical pixels: at least the largest
    /// window composited in the SPEC-9.6.1 checks.
    const WINDOW: (u32, u32) = (960, 640);

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

    /// Physical pixels `(left, top, width, height)`.
    type Area = (i64, i64, i64, i64);

    fn inside(a: Area, x: u32, y: u32) -> bool {
        let (x, y) = (i64::from(x), i64::from(y));
        x >= a.0 && y >= a.1 && x < a.0 + a.2 && y < a.1 + a.3
    }

    fn pixel(img: &RgbaImage, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * img.width + x) * 4) as usize;
        [
            img.data[i],
            img.data[i + 1],
            img.data[i + 2],
            img.data[i + 3],
        ]
    }

    /// `area` of `img`, cut to the image.
    fn crop(img: &RgbaImage, area: Area) -> RgbaImage {
        let x0 = area.0.clamp(0, i64::from(img.width)) as u32;
        let y0 = area.1.clamp(0, i64::from(img.height)) as u32;
        let x1 = (area.0 + area.2).clamp(0, i64::from(img.width)) as u32;
        let y1 = (area.1 + area.3).clamp(0, i64::from(img.height)) as u32;
        let mut out = RgbaImage::new((x1 - x0).max(1), (y1 - y0).max(1));
        for y in y0..y1 {
            for x in x0..x1 {
                let o = (((y - y0) * out.width + (x - x0)) * 4) as usize;
                out.data[o..o + 4].copy_from_slice(&pixel(img, x, y));
            }
        }
        out
    }

    /// The window as `present` left it. The window has no transparency:
    /// its alpha is read as opaque.
    fn window(ctx: &GlContext, w: u32, h: u32) -> RgbaImage {
        let mut img = ctx.raster.read_window(&ctx.gl, w, h);
        for p in img.data.chunks_exact_mut(4) {
            p[3] = 255;
        }
        img
    }

    /// A premultiplied frame pixel over an opaque one, as the window blends.
    fn over(top: [u8; 4], under: [u8; 4]) -> [u8; 3] {
        let a = u32::from(top[3]);
        let ch = |i: usize| {
            ((u32::from(top[i]) * 255 + u32::from(under[i]) * (255 - a) + 127) / 255).min(255) as u8
        };
        [ch(0), ch(1), ch(2)]
    }

    /// SPEC-9.6.1 (granskning 0011:2): `GlRaster::present` itself. A page
    /// canvas larger than the window, placed so that it reaches past the
    /// page area on every side, under a frame whose every pixel is known:
    /// opaque chrome around the page area, one translucent strip of chrome
    /// (where a page that was not cut to its area would show through), a
    /// transparent page area and a translucent overlay inside it. Read
    /// back, the chrome is the frame's bytes, the translucent chrome is
    /// over white and never over the page, the page area is the page's own
    /// pixels, and the overlay is blended over them. Then the same with
    /// only some rows of the frame sent again, and without a page.
    fn presents_the_page_inside_its_area(
        ctx: &mut GlContext,
        failures: &mut Vec<String>,
    ) -> Result<String, String> {
        let (w, h) = (640u32, 480u32);
        let clip: Area = (40, 60, 500, 300);
        let at = (-20i64, -30i64);
        let overlay: Area = (120, 140, 200, 60);
        let strip: Area = (0, 400, 640, 16);
        let chrome = |x: u32, y: u32, salt: u32| -> [u8; 4] {
            if inside(overlay, x, y) {
                [0, 20, 100, 128]
            } else if inside(clip, x, y) {
                [0, 0, 0, 0]
            } else if inside(strip, x, y) {
                [60, 30, 0, 128]
            } else {
                [
                    (x * 7 + y * 3 + salt) as u8,
                    (x ^ y) as u8,
                    (x + y * 5) as u8,
                    255,
                ]
            }
        };
        let frame = |salt: u32| {
            let mut img = RgbaImage::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let i = ((y * w + x) * 4) as usize;
                    img.data[i..i + 4].copy_from_slice(&chrome(x, y, salt));
                }
            }
            img
        };
        // The page: magenta to every edge of a canvas larger than the
        // window, with shapes and text on it.
        let (pw, ph) = (700u32, 560u32);
        let mut t = TextEngine::new();
        let style = t
            .style(font::SANS, 15.0, weight::NORMAL, 0x1020_30FF)
            .unwrap();
        let mut dl = DisplayList::new();
        dl.begin().unwrap();
        dl.rect(-50.0, -50.0, 900.0, 800.0, 0.0, 0xFF00_FFFF)
            .unwrap();
        dl.rect(30.5, 40.25, 300.0, 200.0, 12.0, 0x2060_A0C0)
            .unwrap();
        dl.rect(500.0, 300.0, 400.0, 400.0, 0.0, 0x00C0_40FF)
            .unwrap();
        dl.text(70.0, 110.0, 0.0, style, "sidan ritar hit".as_bytes())
            .unwrap();
        dl.end().unwrap();
        let images = ImageCache::empty(0);
        let mut r = Renderer::new(RasterPath::Fast);
        r.render_gl(
            &ctx.gl,
            &mut ctx.raster,
            dl.bytes(),
            pw,
            ph,
            1.0,
            &mut t,
            &images,
        )
        .map_err(|e| e.to_string())?;
        let page = ctx.raster.read_page(&ctx.gl).ok_or("no page")?;
        let place = Some(PageAt {
            x: at.0,
            y: at.1,
            clip,
        });
        let mut counts = [0u64; 4];
        let mut check = |ctx: &mut GlContext,
                         f: &RgbaImage,
                         changed: Option<(u32, u32)>,
                         place: Option<PageAt>,
                         what: &str|
         -> Result<(), String> {
            ctx.raster
                .present(&ctx.gl, f, changed, place)
                .map_err(|e| e.to_string())?;
            let got = window(ctx, w, h);
            let mut bad = Vec::new();
            for y in 0..h {
                for x in 0..w {
                    let top = pixel(f, x, y);
                    let g = pixel(&got, x, y);
                    let in_area = place.is_some() && inside(clip, x, y);
                    let under = if in_area {
                        pixel(
                            &page,
                            (i64::from(x) - at.0) as u32,
                            (i64::from(y) - at.1) as u32,
                        )
                    } else {
                        [255, 255, 255, 255]
                    };
                    let want = over(top, under);
                    // What is opaque, and what is fully transparent, is
                    // exact; a blend may round one level either way.
                    let slack = u8::from(top[3] != 0 && top[3] != 255);
                    let ok = (0..3).all(|i| g[i].abs_diff(want[i]) <= slack);
                    counts[match (in_area, top[3]) {
                        (false, 255) => 0,
                        (false, _) => 1,
                        (true, 0) => 2,
                        (true, _) => 3,
                    }] += 1;
                    if !ok {
                        bad.push(format!("({x}, {y}): {g:?}, want {want:?}"));
                    }
                }
            }
            if !bad.is_empty() {
                failures.push(format!(
                    "present, {what}: {} pixels are not the composition's, the first {}",
                    bad.len(),
                    bad[0]
                ));
            }
            Ok(())
        };
        let first = frame(0);
        check(ctx, &first, None, place, "a page larger than its area")?;
        // Only rows 8..24 and 150..170 of the frame change and are sent.
        let mut second = first.clone();
        let other = frame(91);
        let row = w as usize * 4;
        for (y0, y1) in [(8usize, 24usize), (150, 170)] {
            second.data[y0 * row..y1 * row].copy_from_slice(&other.data[y0 * row..y1 * row]);
        }
        check(ctx, &second, Some((8, 170)), place, "changed rows only")?;
        check(ctx, &second, None, None, "no page")?;
        Ok(format!(
            "present    {w}x{h}, page {pw}x{ph} at {at:?} cut to {clip:?}, three frames: {} chrome pixels byte for byte, {} of translucent chrome and of the area without a page over white, {} page pixels exact, {} overlay pixels within 1",
            counts[0], counts[1], counts[2], counts[3]
        ))
    }

    /// The page area of `shell` in physical pixels, as its layout has it.
    fn page_area(shell: &Shell, scale: f32) -> Area {
        let p = shell.layout().page;
        (
            (p.x * scale).round() as i64,
            (p.y * scale).round() as i64,
            (p.w * scale).round() as i64,
            (p.h * scale).round() as i64,
        )
    }

    /// Composite `shell`'s window with GL, as its window would with what
    /// it knows of the page framebuffer in `page`, and read it back.
    fn composed(
        ctx: &mut GlContext,
        shell: &mut Shell,
        page: &mut PageState,
        what: &str,
    ) -> Result<RgbaImage, String> {
        shell.set_page_layer(true);
        let (w, h) = (shell.frame().width, shell.frame().height);
        // Every row of the frame is sent: this is not the frame the
        // context composited last.
        ctx.compose(shell, Some((0, h)), page)
            .map_err(|e| format!("{what}: {e}"))?;
        Ok(window(ctx, w, h))
    }

    /// Composite `shell`'s window with GL and on the CPU and compare:
    /// outside the page area the two are the same bytes; inside it they
    /// are within the tolerance of SPEC-11.3.2.
    fn compare_window(
        ctx: &mut GlContext,
        shell: &mut Shell,
        page: &mut PageState,
        scale: f32,
        what: &str,
        failures: &mut Vec<String>,
    ) -> Result<(Diff, u64), String> {
        let got = composed(ctx, shell, page, what)?;
        let (w, h) = (got.width, got.height);
        // The same state composited on the CPU, with the reference.
        shell.set_page_layer(false);
        let want = shell.frame().clone();
        shell.set_page_layer(true);
        let area = page_area(shell, scale);
        let mut chrome = 0u64;
        let mut bad = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if inside(area, x, y) {
                    continue;
                }
                chrome += 1;
                let (g, c) = (pixel(&got, x, y), pixel(&want, x, y));
                if g[..3] != c[..3] {
                    bad.push(format!("({x}, {y}): {g:?}, the chrome's {c:?}"));
                }
            }
        }
        if !bad.is_empty() {
            failures.push(format!(
                "{what}: {} pixels outside the page area {area:?} are not the chrome's, the first {}",
                bad.len(),
                bad[0]
            ));
        }
        let d = Diff::of(&crop(&got, area), &crop(&want, area));
        if d.share() > DIFFERING_SHARE {
            failures.push(format!(
                "{what}: {:.3} % of the page area differs by more than {CHANNEL_DELTA} (max {})",
                d.share() * 100.0,
                d.max
            ));
        }
        Ok((d, chrome))
    }

    /// A module that paints one colour (`rgba`, opaque) from far outside
    /// its canvas to far outside it, as `<name>.wasm` in `dir`.
    fn solid_page(dir: &Path, name: &str, rgba: u32) -> Result<Source, String> {
        let wasm = wat::parse_str(format!(
            r#"(module
                (import "vy" "dl_begin" (func $dl_begin (result i32)))
                (import "vy" "dl_end" (func $dl_end (result i32)))
                (import "vy" "dl_rect" (func $dl_rect (param f32 f32 f32 f32 f32 i32) (result i32)))
                (memory (export "memory") 1)
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32)
                    (drop (call $dl_begin))
                    (drop (call $dl_rect (f32.const -10000) (f32.const -10000) (f32.const 20000) (f32.const 20000) (f32.const 0) (i32.const {})))
                    (drop (call $dl_end))
                    i32.const 0)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
            rgba as i32
        ))
        .map_err(|e| e.to_string())?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("{name}.wasm"));
        std::fs::write(&path, wasm).map_err(|e| e.to_string())?;
        Ok(Source::File { path, draft: None })
    }

    /// How many pixels of `area` in `img` are exactly `rgb`.
    fn count(img: &RgbaImage, area: Area, rgb: [u8; 3]) -> u64 {
        let mut n = 0;
        for y in 0..img.height {
            for x in 0..img.width {
                n += u64::from(inside(area, x, y) && pixel(img, x, y)[..3] == rgb);
            }
        }
        n
    }

    /// SPEC-9.6.1, SPEC-11.3.2 (granskning 0011:3): a page whose display
    /// list the reference refuses shows no page in the GL window, as in
    /// the CPU window, and never the picture the framebuffer still holds
    /// of another tab. A red page and a blue page in two tabs; while the
    /// red one is shown (and is what the page framebuffer holds), the
    /// blue tab's process sends a list that does not decode; the blue tab
    /// is switched to. Its area is then empty, twice in a row (the second
    /// composition is of the same page version), nothing red is under
    /// the blue tab's address, the window is the CPU composition's, and
    /// the tab's log says why. Loaded again, the blue page is shown.
    fn shows_no_page_for_a_refused_list(
        ctx: &mut GlContext,
        failures: &mut Vec<String>,
    ) -> Result<String, String> {
        use vy_shell::ipc::ToShell;
        use vy_shell::process::Inbound;
        use vy_shell::shell::Incoming;
        const RED: [u8; 3] = [255, 0, 0];
        const BLUE: [u8; 3] = [0, 0, 255];
        const WHITE: [u8; 3] = [255, 255, 255];
        let timeout = std::time::Duration::from_secs(60);
        let exe = PathBuf::from(env!("CARGO_BIN_EXE_vy-shell"));
        let dir = std::env::temp_dir().join(format!("vy-gl-refused-{}", std::process::id()));
        let mut shell = headless_shell(Some(exe), 640.0, 480.0, 1.0).map_err(|e| e.to_string())?;
        let mut page = PageState::default();
        let red = shell.open_tab(Some(solid_page(&dir, "red", 0xFF00_00FF)?));
        settle(&mut shell, 2, timeout)?;
        let blue = shell.open_tab(Some(solid_page(&dir, "blue", 0x0000_FFFF)?));
        settle(&mut shell, 2, timeout)?;
        let area = page_area(&shell, 1.0);
        let all = (area.2 * area.3) as u64;
        let expect = |got: &RgbaImage, rgb: [u8; 3], what: &str, failures: &mut Vec<String>| {
            let (r, b, w) = (
                count(got, area, RED),
                count(got, area, BLUE),
                count(got, area, WHITE),
            );
            if count(got, area, rgb) != all {
                failures.push(format!(
                    "{what}: of {all} pixels in the page area {r} are red, {b} blue, {w} white"
                ));
            }
        };
        let got = composed(ctx, &mut shell, &mut page, "the blue tab")?;
        expect(&got, BLUE, "the blue tab", failures);
        shell.switch_tab(false);
        settle(&mut shell, 2, timeout)?;
        let got = composed(ctx, &mut shell, &mut page, "the red tab")?;
        expect(&got, RED, "the red tab", failures);
        if shell.active_tab() != Some(red) || page.shown().is_none() {
            return Err("the red tab is not the one shown".into());
        }

        // The blue tab's process, in the background, sends a list that
        // does not decode; then the user switches to the tab.
        let generation = shell.tab_generation(blue);
        shell.handle(Incoming::Tab(
            blue,
            generation,
            Inbound::Message(ToShell::Frame {
                // No opcode 0x7E (appendix C), and no END.
                display_list: vec![0x7E, 0x01, 0x02, 0x03],
                styles: Vec::new(),
                wants_more: false,
                over_budget: false,
            }),
        ));
        shell.switch_tab(true);
        if shell.active_tab() != Some(blue) {
            return Err("the blue tab is not active".into());
        }
        for round in ["first", "second"] {
            let what = format!("a refused list after a tab switch, {round} composition");
            let got = composed(ctx, &mut shell, &mut page, &what)?;
            expect(&got, WHITE, &what, failures);
            if page.shown().is_some() {
                failures.push(format!("{what}: the framebuffer's picture counts as shown"));
            }
        }
        let (d, _) = compare_window(
            ctx,
            &mut shell,
            &mut page,
            1.0,
            "a refused list against the CPU window",
            failures,
        )?;
        if d.any != 0 {
            failures.push(format!(
                "a refused list: the page area is not the CPU window's: {d:?}"
            ));
        }
        if !shell
            .tab_logs(blue)
            .iter()
            .any(|l| l.contains("display list not rendered"))
        {
            failures.push(format!(
                "a refused list is not in the tab's log: {:?}",
                shell.tab_logs(blue)
            ));
        }
        // The page loaded again draws, and is shown.
        shell.reload();
        settle(&mut shell, 2, timeout)?;
        let got = composed(ctx, &mut shell, &mut page, "the blue tab reloaded")?;
        expect(&got, BLUE, "the blue tab reloaded", failures);
        if page.shown().is_none() {
            failures.push("the reloaded page does not count as shown".into());
        }
        shell.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
        Ok(format!(
            "refused    a list that does not decode, after a tab switch: no page in the {all} pixels of the page area (not the other tab's), twice; shown again after a reload"
        ))
    }

    /// SPEC-9.6.1, SPEC-11.3.2 (granskning 0011:2): the GL window of a
    /// real shell, through `GlContext::compose` as `window.rs` calls it. A
    /// reference page; the same with the devtools panel open, with an op
    /// outlined, and zoomed (the canvas is then not the page area's
    /// size); a second window at scale 1.5; and a page that paints red
    /// from far outside its canvas to far outside it. Every time the
    /// pixels outside the page area are the CPU composition's bytes, and
    /// the page area is within the tolerance of it; the red page is red
    /// to the edge of its area and nowhere past it.
    fn composes_the_shell_window(
        ctx: &mut GlContext,
        failures: &mut Vec<String>,
    ) -> Result<String, String> {
        use vy_shell::devtools::Command;
        let timeout = std::time::Duration::from_secs(60);
        let exe = PathBuf::from(env!("CARGO_BIN_EXE_vy-shell"));
        let local = |name: &str| Source::File {
            path: root().join(format!("target/wasm/{name}.wasm")),
            draft: Some(root().join(format!("conformance/manifests/{name}.json"))),
        };
        let mut sum = Diff::default();
        let mut chrome = 0u64;
        let mut cases = 0;
        let mut step = |ctx: &mut GlContext,
                        shell: &mut Shell,
                        page: &mut PageState,
                        scale: f32,
                        what: &str,
                        failures: &mut Vec<String>|
         -> Result<Diff, String> {
            settle(shell, 2, timeout)?;
            let (d, c) = compare_window(ctx, shell, page, scale, what, failures)?;
            if page.shown().is_none() {
                failures.push(format!("{what}: no page was drawn"));
            }
            sum.add(&d);
            chrome += c;
            cases += 1;
            Ok(d)
        };

        let mut shell =
            headless_shell(Some(exe.clone()), 900.0, 600.0, 1.0).map_err(|e| e.to_string())?;
        // What the window keeps of the page framebuffer, for this shell.
        let mut page = PageState::default();
        shell.open_tab(Some(local("shop")));
        step(ctx, &mut shell, &mut page, 1.0, "shop 900x600@1", failures)?;
        let whole = page_area(&shell, 1.0);
        shell.toggle_devtools();
        step(
            ctx,
            &mut shell,
            &mut page,
            1.0,
            "shop, devtools open",
            failures,
        )?;
        let narrow = page_area(&shell, 1.0);
        if narrow.2 >= whole.2 || narrow.0 + narrow.2 >= 900 {
            failures.push(format!(
                "the devtools panel did not narrow the page area: {whole:?}, then {narrow:?}"
            ));
        }
        shell.devtools_run(Command::SelectOp(2));
        step(
            ctx,
            &mut shell,
            &mut page,
            1.0,
            "shop, devtools, an op outlined",
            failures,
        )?;
        shell.zoom_step(1);
        step(
            ctx,
            &mut shell,
            &mut page,
            1.0,
            "shop, devtools, zoom 110 %",
            failures,
        )?;
        shell.zoom_step(1);
        step(
            ctx,
            &mut shell,
            &mut page,
            1.0,
            "shop, devtools, zoom 125 %",
            failures,
        )?;

        // A page that paints everywhere it can name.
        let dir = std::env::temp_dir().join(format!("vy-gl-compose-{}", std::process::id()));
        let tab = shell.open_tab(Some(solid_page(&dir, "red", 0xFF00_00FF)?));
        let d = step(
            ctx,
            &mut shell,
            &mut page,
            1.0,
            "a page painting past its canvas",
            failures,
        )?;
        if shell.tab_status(tab) != "running" {
            failures.push(format!("the red page: {}", shell.tab_status(tab)));
        }
        if d.any != 0 {
            failures.push(format!("the red page's area is not the reference's: {d:?}"));
        }
        let area = page_area(&shell, 1.0);
        let got = window(ctx, 900, 600);
        let mut red_inside = 0u64;
        let mut red_outside = 0u64;
        for y in 0..600 {
            for x in 0..900 {
                let is_red = pixel(&got, x, y)[..3] == [255, 0, 0];
                red_inside += u64::from(is_red && inside(area, x, y));
                red_outside += u64::from(is_red && !inside(area, x, y));
            }
        }
        if red_inside != (area.2 * area.3) as u64 || red_outside != 0 {
            failures.push(format!(
                "the red page: {red_inside} red pixels in its area of {}, {red_outside} outside it",
                area.2 * area.3
            ));
        }
        shell.shutdown();
        let _ = std::fs::remove_dir_all(&dir);

        // Another window, at scale 1.5.
        let mut shell = headless_shell(Some(exe), 600.0, 400.0, 1.5).map_err(|e| e.to_string())?;
        // What the window keeps of the page framebuffer, for this shell.
        let mut page = PageState::default();
        shell.open_tab(Some(local("text")));
        step(
            ctx,
            &mut shell,
            &mut page,
            1.5,
            "text 600x400@1.5",
            failures,
        )?;
        shell.toggle_devtools();
        step(
            ctx,
            &mut shell,
            &mut page,
            1.5,
            "text @1.5, devtools open",
            failures,
        )?;
        shell.shutdown();
        Ok(format!(
            "window     {cases} compositions of a shell (devtools, zoom, scale 1.5, a page painting past its canvas): {chrome} chrome pixels byte for byte the CPU composition's; page area {:.3} % of pixels differ at all, {:.3} % by more than {CHANNEL_DELTA}, max {}",
            sum.any as f64 / sum.pixels.max(1) as f64 * 100.0,
            sum.share() * 100.0,
            sum.max
        ))
    }

    struct Outcome {
        lines: Vec<String>,
        failures: Vec<String>,
    }

    fn run(ctx: &mut GlContext) -> Result<Outcome, String> {
        let mut lines = vec![format!("GL: {}", ctx.raster.version())];
        let mut failures = Vec::new();
        let mut budget = None;
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

        // SPEC-13.2.1 on this context, in release: the frame drawn whole 100
        // times, each waited out (`glFinish`). A software GL (llvmpipe) is
        // no GPU: the line is the report, and the budget is asserted only
        // with VY_GL_BUDGET set (the owner's PC, docs/notes/m5-gpu.md).
        if !cfg!(debug_assertions) {
            let mut t = TextEngine::new();
            let f = vy_render::bench::frame_2000(&mut t, 1280.0, 800.0);
            let mut r = Renderer::new(RasterPath::Fast);
            let mut wall = Vec::new();
            let mut prep = Vec::new();
            for i in 0..110 {
                let start = std::time::Instant::now();
                r.render_gl(
                    &ctx.gl,
                    &mut ctx.raster,
                    &f.list,
                    1280,
                    800,
                    1.0,
                    &mut t,
                    &f.images,
                )
                .map_err(|e| e.to_string())?;
                // SAFETY: the context is current on this thread.
                unsafe { glow::HasContext::finish(&ctx.gl) };
                if i >= 10 {
                    wall.push(start.elapsed());
                    prep.push(ctx.raster.stats().prepare);
                }
            }
            wall.sort();
            prep.sort();
            let stats = ctx.raster.stats();
            lines.push(format!(
                "SPEC-13.2.1 on this context: P50 {:?}, P95 {:?} per frame to glFinish; CPU before the GPU (instances, clip table, paths, GL calls) P50 {:?}; {} instances, {} draw calls, {} bytes uploaded per frame",
                wall[49], wall[94], prep[49], stats.instances, stats.draws, stats.upload_bytes
            ));
            if std::env::var_os("VY_GL_BUDGET").is_some()
                && wall[94] > std::time::Duration::from_millis(2)
            {
                budget = Some(format!("SPEC-13.2.1: P95 {:?} over 2 ms", wall[94]));
            }
        }

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
        failures.extend(budget);

        // The window's composition (SPEC-9.6.1).
        lines.push(presents_the_page_inside_its_area(ctx, &mut failures)?);
        lines.push(composes_the_shell_window(ctx, &mut failures)?);
        lines.push(shows_no_page_for_a_refused_list(ctx, &mut failures)?);
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
            // Large enough for every window composited here.
            let attrs = WindowAttributes::default()
                .with_visible(false)
                .with_inner_size(winit::dpi::PhysicalSize::new(WINDOW.0, WINDOW.1))
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
