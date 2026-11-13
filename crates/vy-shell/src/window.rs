//! The window: `winit` for the window, input and DPI, and two ways to
//! present (SPEC-9.1.2: the only pixels, and the only GL context, are the
//! main process's), decided when the window opens and left for good on
//! any GL failure (docs/notes/m5-gpu.md, spec-fråga 66):
//!
//! 1. **OpenGL** (`gl` feature; Linux and Windows): the shell composites
//!    chrome and overlays with the page area transparent, the page is
//!    drawn by `vy_render::gl` into a framebuffer when its list changes,
//!    and both are put on the window (`crate::gl`). Within SPEC-11.3.2's
//!    tolerance of the reference.
//! 2. **CPU** (`softbuffer`): the page rasterized by
//!    `vy_render::Renderer`'s fast path – the reference's bytes, with
//!    damage tracking and bands on threads, itself falling back to the
//!    reference – into the shell's frame, of which only the rectangle that
//!    changed since the last present is converted and sent ([`Presented`]).
//!
//! The path is logged on stderr and in every tab's log (devtools).
//! Everything the window does is a call into [`Shell`]; nothing here
//! decides anything.

use crate::compose::to_xrgb;
use crate::external::SystemOpener;
use crate::keymap::{hid_code, modifier_bits};
use crate::page::Source;
use crate::paths::{data_dir, scratch_dir};
use crate::shell::{Config, Cursor, Shell, Waker, FRAME_MS};
use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};
use vy_render::RgbaImage;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalSize};
use winit::event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{CursorIcon, Window, WindowAttributes, WindowId};

/// The user events of the loop.
#[derive(Debug)]
enum UserEvent {
    /// Something arrived in the shell's inbox.
    Wake,
    /// The platform's accessibility layer (SPEC-10.2.1).
    A11y(accesskit_winit::Event),
}

impl From<accesskit_winit::Event> for UserEvent {
    fn from(e: accesskit_winit::Event) -> Self {
        Self::A11y(e)
    }
}

struct App {
    /// What tab processes are started from ([`tab_exe`]).
    exe: std::path::PathBuf,
    shell: Option<Shell>,
    /// The GL path while it works.
    gpu: Option<gpu::Gpu>,
    /// What the GL page framebuffer holds.
    gpu_page: gpu::Page,
    proxy: EventLoopProxy<UserEvent>,
    /// The AccessKit adapter (SPEC-10.2.1), made before the window shows.
    adapter: Option<accesskit_winit::Adapter>,
    /// True while an assistive technology listens: only then is the tree
    /// built, and only when its digest changed.
    a11y_active: bool,
    a11y_digest: Option<u64>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    presented: Presented,
    initial: Vec<Source>,
    modifiers: ModifiersState,
    held: Vec<i32>,
    next_tick: Instant,
    title: String,
    cursor: Cursor,
    ime_allowed: bool,
}

impl App {
    fn scale(&self) -> f64 {
        self.window.as_ref().map_or(1.0, |w| w.scale_factor())
    }

    /// Hand the shell what is pending, send a tick when due, and ask for
    /// a redraw and the right control flow.
    fn settle(&mut self, el: &ActiveEventLoop) {
        let Some(shell) = self.shell.as_mut() else {
            return;
        };
        shell.drain();
        shell.poll();
        let now = Instant::now();
        if shell.wants_tick() && !shell.tick_pending() {
            if shell.animating() && now < self.next_tick {
                el.set_control_flow(ControlFlow::WaitUntil(self.next_tick));
            } else {
                shell.tick();
                self.next_tick = now + Duration::from_millis(FRAME_MS as u64);
                el.set_control_flow(if shell.animating() {
                    ControlFlow::WaitUntil(self.next_tick)
                } else {
                    ControlFlow::Wait
                });
            }
        } else {
            el.set_control_flow(ControlFlow::Wait);
        }
        // Live reload's next check (devtools).
        if let Some(d) = shell.poll_in() {
            let at = Instant::now() + d.max(Duration::from_millis(1));
            el.set_control_flow(match el.control_flow() {
                ControlFlow::WaitUntil(t) => ControlFlow::WaitUntil(t.min(at)),
                _ => ControlFlow::WaitUntil(at),
            });
        }
        if shell.quit() {
            // The last tab was closed: its process may still answer with
            // its storage; `shutdown` waits for that (one grace period at
            // most) and saves it.
            if let Some(shell) = self.shell.take() {
                shell.shutdown();
            }
            el.exit();
            return;
        }
        let Some(window) = &self.window else {
            return;
        };
        if shell.needs_redraw() {
            window.request_redraw();
        }
        let title = shell.title();
        if title != self.title {
            self.title = title;
            window.set_title(&self.title);
        }
        if self.a11y_active {
            if let Some(adapter) = self.adapter.as_mut() {
                let snap = shell.a11y_snapshot();
                let digest = snap.digest();
                if self.a11y_digest != Some(digest) {
                    self.a11y_digest = Some(digest);
                    adapter.update_if_active(|| crate::a11y::tree_update(&snap));
                }
            }
        }
        let cursor = shell.cursor();
        if cursor != self.cursor {
            self.cursor = cursor;
            window.set_cursor(match cursor {
                Cursor::Arrow => CursorIcon::Default,
                Cursor::Hand => CursorIcon::Pointer,
                Cursor::Text => CursorIcon::Text,
            });
        }
        let ime = shell.ime_area();
        let allowed = ime.is_some();
        if allowed != self.ime_allowed {
            self.ime_allowed = allowed;
            window.set_ime_allowed(allowed);
        }
        if let Some(r) = ime {
            window.set_ime_cursor_area(
                LogicalPosition::new(f64::from(r.x), f64::from(r.y)),
                LogicalSize::new(f64::from(r.w), f64::from(r.h)),
            );
        }
    }

    /// Present the shell's frame. Only what changed since the last
    /// present is converted (as much as the buffer's age needs) and
    /// handed to the window system as damage; a frame that changed
    /// nothing is presented whole, for an expose.
    fn present(&mut self) {
        if let (Some(g), Some(shell)) = (self.gpu.as_mut(), self.shell.as_mut()) {
            match gpu::present(g, shell, &mut self.presented, &mut self.gpu_page) {
                Ok(()) => return,
                Err(e) => self.leave_gpu(e),
            }
        }
        let (Some(shell), Some(surface)) = (self.shell.as_mut(), self.surface.as_mut()) else {
            return;
        };
        let frame = shell.frame();
        let (Some(w), Some(h)) = (NonZeroU32::new(frame.width), NonZeroU32::new(frame.height))
        else {
            return;
        };
        // A new size (a resize, a DPI change) gives a new buffer, age 0.
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        if buffer.len() != (frame.width * frame.height) as usize {
            return;
        }
        let changed = self.presented.update(frame);
        match self.presented.stale(buffer.age(), frame) {
            Some(a) if a == Area::whole(frame) => to_xrgb(frame, &mut buffer),
            Some(a) => to_xrgb_area(frame, &mut buffer, a),
            None => {}
        }
        let _ = match changed.and_then(Area::rect) {
            Some(rect) => buffer.present_with_damage(&[rect]),
            None => buffer.present(),
        };
    }
}

impl App {
    /// Leave the GL path for the CPU path, for good: the shell draws the
    /// page into its frame again and `softbuffer` presents it.
    fn leave_gpu(&mut self, why: String) {
        eprintln!("vy-shell: GL failed ({why}); presenting on the CPU");
        self.gpu = None;
        self.gpu_page = gpu::Page::default();
        self.presented = Presented::default();
        if let Some(shell) = self.shell.as_mut() {
            shell.set_page_layer(false);
            shell.set_raster_path(format!(
                "GL failed ({why}); {}",
                vy_render::Renderer::new(vy_render::RasterPath::Fast).describe()
            ));
        }
        if let Some(window) = &self.window {
            self.surface = cpu_surface(window);
        }
    }
}

/// A `softbuffer` surface on `window`, or `None` with a line on stderr.
fn cpu_surface(window: &Rc<Window>) -> Option<softbuffer::Surface<Rc<Window>, Rc<Window>>> {
    let context = match softbuffer::Context::new(window.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("vy-shell: no drawing surface: {e}");
            return None;
        }
    };
    match softbuffer::Surface::new(&context, window.clone()) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("vy-shell: no drawing surface: {e}");
            None
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // Invisible until the accessibility adapter exists (AccessKit
        // requires it).
        let attributes = WindowAttributes::default()
            .with_visible(false)
            .with_title("Vy")
            .with_inner_size(LogicalSize::new(1024.0, 720.0))
            .with_min_inner_size(LogicalSize::new(320.0, 240.0));
        let (window, gpu) = match gpu::open(el, attributes) {
            Ok((w, g)) => (Rc::new(w), g),
            Err(e) => {
                eprintln!("vy-shell: cannot open a window: {e}");
                el.exit();
                return;
            }
        };
        self.adapter = Some(accesskit_winit::Adapter::with_event_loop_proxy(
            el,
            &window,
            self.proxy.clone(),
        ));
        window.set_visible(true);
        let raster_line = match &gpu {
            Some(g) => gpu::describe(g),
            None => vy_render::Renderer::new(vy_render::RasterPath::Fast).describe(),
        };
        eprintln!(
            "vy-shell: presentation: {}; {raster_line}",
            if gpu.is_some() {
                "OpenGL"
            } else {
                "softbuffer, damage only"
            }
        );
        if gpu.is_none() {
            self.surface = cpu_surface(&window);
            if self.surface.is_none() {
                el.exit();
                return;
            }
        }
        let scale = window.scale_factor();
        let size = window.inner_size();
        let (w, h) = (
            (f64::from(size.width) / scale) as f32,
            (f64::from(size.height) / scale) as f32,
        );
        let proxy = self.proxy.clone();
        let waker = Waker::new(move || {
            let _ = proxy.send_event(UserEvent::Wake);
        });
        let data_dir = data_dir().unwrap_or_else(|| scratch_dir("data"));
        let config = Config {
            exe: Some(self.exe.clone()),
            // The compilation cache is on on every platform, Windows
            // included (granskning 0014:4, spec-fråga 62; decided by the
            // coordinator on the owner's delegation). An entry is keyed
            // by the module's own bytes and filled only by a tab process
            // loaded with them, before any code of the page runs: a
            // poisoned entry can only come from a module that breaks out
            // while it is compiled, and it only runs again for that same
            // module, which could repeat its break-out at every visit
            // without a cache. An entry never outlives the client version
            // that kept it (granskning 0014:1). That Windows's default
            // sandbox lets a tab process read files and reach the network
            // is a property of that sandbox, which strict mode closes,
            // not of the cache: the code runs in the same tab process
            // with or without it.
            code_cache: Some(crate::paths::code_cache_dir(&data_dir)),
            data_dir,
            headless: false,
            opener: Box::new(SystemOpener),
            clipboard: crate::clipboard::system_or_memory(),
            waker,
        };
        let mut shell = match Shell::new(config, w, h, scale as f32) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("vy-shell: {e}");
                el.exit();
                return;
            }
        };
        shell.set_raster_path(raster_line);
        shell.set_page_layer(gpu.is_some());
        self.gpu = gpu;
        let sources = std::mem::take(&mut self.initial);
        if sources.is_empty() {
            shell.open_tab(None);
            shell.key(crate::shell::hid::L, true, vy_abi::modifier::CTRL, false);
        } else {
            for s in sources {
                shell.open_tab(Some(s));
            }
        }
        self.shell = Some(shell);
        self.window = Some(window);
        self.settle(el);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, event: UserEvent) {
        if let UserEvent::A11y(e) = event {
            match e.window_event {
                accesskit_winit::WindowEvent::InitialTreeRequested => {
                    // A full tree now, whatever was sent before.
                    self.a11y_active = true;
                    self.a11y_digest = None;
                }
                accesskit_winit::WindowEvent::ActionRequested(req) => {
                    if let Some(shell) = self.shell.as_mut() {
                        shell.a11y_action(&req);
                    }
                }
                accesskit_winit::WindowEvent::AccessibilityDeactivated => {
                    self.a11y_active = false;
                }
            }
        }
        self.settle(el);
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        self.settle(el);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let (Some(adapter), Some(window)) = (self.adapter.as_mut(), self.window.as_ref()) {
            adapter.process_event(window, &event);
        }
        let scale = self.scale();
        let Some(shell) = self.shell.as_mut() else {
            if let WindowEvent::CloseRequested = event {
                el.exit();
            }
            return;
        };
        match event {
            WindowEvent::CloseRequested => {
                if let Some(shell) = self.shell.take() {
                    shell.shutdown();
                }
                el.exit();
                return;
            }
            WindowEvent::Resized(PhysicalSize { width, height }) => {
                if let Some(g) = &self.gpu {
                    gpu::resize(g, width, height);
                }
                shell.resize(
                    (f64::from(width) / scale) as f32,
                    (f64::from(height) / scale) as f32,
                    scale as f32,
                );
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let size = self.window.as_ref().map(|w| w.inner_size());
                if let Some(size) = size {
                    if let Some(g) = &self.gpu {
                        gpu::resize(g, size.width, size.height);
                    }
                    shell.resize(
                        (f64::from(size.width) / scale_factor) as f32,
                        (f64::from(size.height) / scale_factor) as f32,
                        scale_factor as f32,
                    );
                }
            }
            WindowEvent::RedrawRequested => {
                self.present();
                return;
            }
            WindowEvent::CursorMoved { position, .. } => {
                shell.pointer_move((position.x / scale) as f32, (position.y / scale) as f32);
            }
            WindowEvent::CursorLeft { .. } => shell.pointer_leave(),
            WindowEvent::MouseInput { state, button, .. } => {
                let button = match button {
                    MouseButton::Left => vy_abi::button::PRIMARY,
                    MouseButton::Right => vy_abi::button::SECONDARY,
                    MouseButton::Middle => vy_abi::button::MIDDLE,
                    MouseButton::Back => vy_abi::button::BACK,
                    MouseButton::Forward => vy_abi::button::FORWARD,
                    MouseButton::Other(_) => return,
                };
                shell.pointer_button(button, state == ElementState::Pressed);
            }
            WindowEvent::MouseWheel { delta, .. } => match delta {
                // winit: positive means the content moves right and down;
                // the page's SCROLL is the distance the view moves.
                MouseScrollDelta::LineDelta(x, y) => shell.scroll(-x, -y, true),
                MouseScrollDelta::PixelDelta(p) => {
                    shell.scroll(-(p.x / scale) as f32, -(p.y / scale) as f32, false)
                }
            },
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                shell.set_modifiers(modifier_bits(self.modifiers));
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let mods = modifier_bits(self.modifiers);
                let down = event.state == ElementState::Pressed;
                if let Some(code) = hid_code(event.physical_key) {
                    if down {
                        if !self.held.contains(&code) {
                            self.held.push(code);
                        }
                    } else {
                        self.held.retain(|c| *c != code);
                    }
                    shell.key(code, down, mods, event.repeat);
                }
                // Committed text, when the key is not a shortcut chord.
                if down && crate::keymap::produces_text(mods) {
                    if let Some(text) = event.text.as_deref() {
                        if !text.is_empty() && !text.chars().any(char::is_control) {
                            shell.text_input(text);
                        }
                    }
                }
            }
            WindowEvent::Ime(Ime::Commit(text)) => shell.text_input(&text),
            WindowEvent::Ime(_) => {}
            WindowEvent::Focused(false) => {
                // Held keys are released for the page (SPEC-5.2.6): the
                // host clears `key_down` on focus loss.
                let mods = modifier_bits(self.modifiers);
                for code in std::mem::take(&mut self.held) {
                    shell.key(code, false, mods, false);
                }
            }
            _ => {}
        }
        self.settle(el);
    }
}

/// What has been presented, so a frame that changes a little converts
/// and sends a little (SPEC-13.2.2): the last frame's pixels and the
/// changed rectangle of each of the last few presents, for buffers that
/// come back some presents old (double buffering).
#[derive(Default)]
struct Presented {
    last: Option<RgbaImage>,
    /// Newest last: the rectangle that changed in each present (`None`:
    /// nothing changed), at most [`Presented::HISTORY`].
    changes: VecDeque<Option<Area>>,
}

/// Physical pixels `x0..x1` × `y0..y1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Area {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl Area {
    fn whole(img: &RgbaImage) -> Self {
        Self {
            x0: 0,
            y0: 0,
            x1: img.width,
            y1: img.height,
        }
    }

    fn union(self, o: Area) -> Area {
        Area {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    fn rect(self) -> Option<softbuffer::Rect> {
        Some(softbuffer::Rect {
            x: self.x0,
            y: self.y0,
            width: NonZeroU32::new(self.x1 - self.x0)?,
            height: NonZeroU32::new(self.y1 - self.y0)?,
        })
    }
}

impl Presented {
    const HISTORY: usize = 3;

    /// Record `frame` as presented and return what changed since the last
    /// present: the whole frame when there was none or its size differs,
    /// `None` when every pixel is the same.
    fn update(&mut self, frame: &RgbaImage) -> Option<Area> {
        let changed = match &self.last {
            Some(last) if (last.width, last.height) == (frame.width, frame.height) => {
                changed_area(last, frame)
            }
            _ => {
                self.changes.clear();
                Some(Area::whole(frame))
            }
        };
        match (&mut self.last, changed) {
            (Some(last), Some(a)) if (last.width, last.height) == (frame.width, frame.height) => {
                let w = frame.width as usize * 4;
                let (x0, x1) = (a.x0 as usize * 4, a.x1 as usize * 4);
                for y in a.y0 as usize..a.y1 as usize {
                    last.data[y * w + x0..y * w + x1]
                        .copy_from_slice(&frame.data[y * w + x0..y * w + x1]);
                }
            }
            (_, Some(_)) => self.last = Some(frame.clone()),
            (_, None) => {}
        }
        if self.changes.len() == Self::HISTORY {
            self.changes.pop_front();
        }
        self.changes.push_back(changed);
        changed
    }

    /// What a buffer last presented `age` presents ago lacks: the union
    /// of the changes since, or the whole frame when that is not known
    /// (`age` 0: a new buffer).
    fn stale(&self, age: u8, frame: &RgbaImage) -> Option<Area> {
        let age = usize::from(age);
        if age == 0 || age > self.changes.len() {
            return Some(Area::whole(frame));
        }
        self.changes
            .iter()
            .rev()
            .take(age)
            .flatten()
            .copied()
            .reduce(Area::union)
    }
}

/// The bounding box of the pixels that differ between two frames of the
/// same size, or `None`.
fn changed_area(a: &RgbaImage, b: &RgbaImage) -> Option<Area> {
    let w = a.width as usize * 4;
    let mut area: Option<Area> = None;
    for (y, (ra, rb)) in a
        .data
        .chunks_exact(w)
        .zip(b.data.chunks_exact(w))
        .enumerate()
    {
        if ra == rb {
            continue;
        }
        let first = ra.iter().zip(rb).position(|(p, q)| p != q).unwrap_or(0) / 4;
        let last = w / 4
            - ra.iter()
                .rev()
                .zip(rb.iter().rev())
                .position(|(p, q)| p != q)
                .unwrap_or(0)
                / 4;
        let row = Area {
            x0: first as u32,
            y0: y as u32,
            x1: last as u32,
            y1: y as u32 + 1,
        };
        area = Some(area.map_or(row, |u| u.union(row)));
    }
    area
}

/// [`to_xrgb`] over `area` of the frame only.
fn to_xrgb_area(img: &RgbaImage, out: &mut [u32], area: Area) {
    let w = img.width as usize;
    let (x0, x1) = (area.x0 as usize, area.x1 as usize);
    for y in area.y0 as usize..area.y1 as usize {
        let src = &img.data[(y * w + x0) * 4..(y * w + x1) * 4];
        for (px, dst) in src.chunks_exact(4).zip(&mut out[y * w + x0..y * w + x1]) {
            *dst = (u32::from(px[0]) << 16) | (u32::from(px[1]) << 8) | u32::from(px[2]);
        }
    }
}

/// The GL path (`gl` feature, not macOS).
#[cfg(all(feature = "gl", not(target_os = "macos")))]
mod gpu {
    use super::Presented;
    use crate::shell::Shell;
    use winit::event_loop::ActiveEventLoop;
    use winit::window::{Window, WindowAttributes};

    pub type Gpu = crate::gl::GlContext;
    /// What the page framebuffer holds.
    pub type Page = crate::gl::PageState;

    /// The window, with a GL context when one can be made.
    pub fn open(
        el: &ActiveEventLoop,
        attributes: WindowAttributes,
    ) -> Result<(Window, Option<Gpu>), String> {
        match crate::gl::open(el, attributes.clone()) {
            Ok((w, g)) => Ok((w, Some(g))),
            Err(no) => {
                eprintln!("vy-shell: no GL ({}); the CPU path", no.why);
                match no.window {
                    Some(w) => Ok((w, None)),
                    None => el
                        .create_window(attributes)
                        .map(|w| (w, None))
                        .map_err(|e| e.to_string()),
                }
            }
        }
    }

    pub fn describe(g: &Gpu) -> String {
        g.describe()
    }

    pub fn resize(g: &Gpu, width: u32, height: u32) {
        g.resize(width, height);
    }

    /// Composite the shell's frame (transparent where the page shows),
    /// draw the page when it changed, present. `Err`: leave the GL path.
    pub fn present(
        g: &mut Gpu,
        shell: &mut Shell,
        presented: &mut Presented,
        page: &mut Page,
    ) -> Result<(), String> {
        let changed = presented.update(shell.frame()).map(|a| (a.y0, a.y1));
        g.compose(shell, changed, page).map_err(|e| e.to_string())?;
        g.swap().map_err(|e| e.to_string())
    }
}

/// No GL path in this build: the CPU path only.
#[cfg(not(all(feature = "gl", not(target_os = "macos"))))]
mod gpu {
    use super::Presented;
    use crate::shell::Shell;
    use winit::event_loop::ActiveEventLoop;
    use winit::window::{Window, WindowAttributes};

    pub enum Gpu {}
    pub type Page = ();

    pub fn open(
        el: &ActiveEventLoop,
        attributes: WindowAttributes,
    ) -> Result<(Window, Option<Gpu>), String> {
        el.create_window(attributes)
            .map(|w| (w, None))
            .map_err(|e| e.to_string())
    }

    pub fn describe(g: &Gpu) -> String {
        match *g {}
    }

    pub fn resize(g: &Gpu, _: u32, _: u32) {
        match *g {}
    }

    pub fn present(
        g: &mut Gpu,
        _: &mut Shell,
        _: &mut Presented,
        _: &mut Page,
    ) -> Result<(), String> {
        match *g {}
    }
}

/// The executable tab processes are started from, or why the window does
/// not open. Without one every page would run on a thread of this
/// process, outside the OS sandbox (SPEC-9.1.1, SPEC-9.1.3), and kept
/// code would be deserialized here (SPEC-9.4.2; granskning 0014:2): the
/// window refuses to start instead, as `--screenshot` does.
fn tab_exe(found: std::io::Result<std::path::PathBuf>) -> Result<std::path::PathBuf, String> {
    found.map_err(|e| {
        format!("cannot find the client's own executable to start tab processes from: {e}")
    })
}

/// Open the window with `sources` in tabs (an empty tab when none) and
/// run until it closes.
pub fn run(sources: Vec<Source>) -> Result<(), String> {
    let exe = tab_exe(crate::process::self_exe())?;
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    let proxy = event_loop.create_proxy();
    let mut app = App {
        exe,
        shell: None,
        gpu: None,
        gpu_page: gpu::Page::default(),
        proxy,
        adapter: None,
        a11y_active: false,
        a11y_digest: None,
        window: None,
        surface: None,
        presented: Presented::default(),
        initial: sources,
        modifiers: ModifiersState::empty(),
        held: Vec::new(),
        next_tick: Instant::now(),
        title: String::new(),
        cursor: Cursor::Arrow,
        ime_allowed: false,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                img.data[i..i + 4].copy_from_slice(&f(x, y));
            }
        }
        img
    }

    /// SPEC-9.1.1, SPEC-9.4.2 (granskning 0014:2): the window does not
    /// open without an executable to start tab processes from; it never
    /// falls back to running pages on threads of the main process.
    #[test]
    fn spec_9_1_1_no_window_without_a_tab_executable() {
        let gone = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");
        let why = tab_exe(Err(gone)).unwrap_err();
        assert!(
            why.contains("tab processes") && why.contains("no such file"),
            "{why}"
        );
        let exe = std::path::PathBuf::from("/proc/self/exe");
        assert_eq!(tab_exe(Ok(exe.clone())), Ok(exe));
    }

    /// SPEC-13.2.2, SPEC-9.6.1: the window converts and sends what changed
    /// since the last present – the bounding box of the changed pixels –
    /// and, for a buffer some presents old, everything changed since it
    /// was last shown; a new size or a new buffer is the whole frame; the
    /// pixels a buffer ends up with are the frame's, whatever its age.
    #[test]
    fn spec_13_2_2_present_converts_what_changed() {
        let white = |_, _| [255, 255, 255, 255];
        let a = frame(40, 30, white);
        let mut b = a.clone();
        for (x, y) in [(5, 7), (12, 9)] {
            b.data[((y * 40 + x) * 4) as usize] = 0;
        }
        let mut c = b.clone();
        c.data[((20 * 40 + 30) * 4 + 1) as usize] = 7;
        let mut p = Presented::default();
        assert_eq!(p.update(&a), Some(Area::whole(&a)), "first present");
        let ab = p.update(&b);
        assert_eq!(
            ab,
            Some(Area {
                x0: 5,
                y0: 7,
                x1: 13,
                y1: 10
            })
        );
        assert_eq!(p.update(&b), None, "nothing changed");
        let bc = p.update(&c).unwrap();
        assert_eq!(
            bc,
            Area {
                x0: 30,
                y0: 20,
                x1: 31,
                y1: 21
            }
        );
        assert_eq!(p.stale(1, &c), Some(bc));
        assert_eq!(p.stale(2, &c), Some(bc), "the frame before changed nothing");
        assert_eq!(p.stale(3, &c), Some(ab.unwrap().union(bc)));
        assert_eq!(p.stale(0, &c), Some(Area::whole(&c)), "a new buffer");
        assert_eq!(p.stale(9, &c), Some(Area::whole(&c)), "older than known");
        // A buffer two presents old ends up with exactly the frame.
        let mut words = vec![0u32; 40 * 30];
        to_xrgb(&a, &mut words);
        to_xrgb_area(&c, &mut words, p.stale(3, &c).unwrap());
        let mut want = vec![0u32; 40 * 30];
        to_xrgb(&c, &mut want);
        assert_eq!(words, want);
        // A new size starts over.
        let small = frame(10, 10, white);
        assert_eq!(p.update(&small), Some(Area::whole(&small)));
        assert_eq!(p.stale(2, &small), Some(Area::whole(&small)));
        assert!(Area::whole(&small).rect().is_some());
        assert!(Area {
            x0: 3,
            y0: 3,
            x1: 3,
            y1: 4
        }
        .rect()
        .is_none());
    }
}
