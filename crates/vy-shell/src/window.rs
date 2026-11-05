//! The window: `winit` for the window, input and DPI, `softbuffer` to
//! present the frame the shell composited on the CPU (SPEC-9.1.2: the
//! only pixels are the main process's; the GPU path with `wgpu` is the
//! planned M4/M5 upgrade, behind the same `vy-render` code path,
//! SPEC-11.3.2). Everything the window does is a call into [`Shell`];
//! nothing here decides anything.

use crate::compose::to_xrgb;
use crate::external::SystemOpener;
use crate::keymap::{hid_code, modifier_bits};
use crate::page::Source;
use crate::paths::{data_dir, scratch_dir};
use crate::shell::{Config, Cursor, Shell, Waker, FRAME_MS};
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};
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
    shell: Option<Shell>,
    proxy: EventLoopProxy<UserEvent>,
    /// The AccessKit adapter (SPEC-10.2.1), made before the window shows.
    adapter: Option<accesskit_winit::Adapter>,
    /// True while an assistive technology listens: only then is the tree
    /// built, and only when its digest changed.
    a11y_active: bool,
    a11y_digest: Option<u64>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
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
            // The last tab was closed: what its process answered (its
            // storage) is still in the inbox; `shutdown` handles it.
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

    fn present(&mut self) {
        let (Some(shell), Some(surface)) = (self.shell.as_mut(), self.surface.as_mut()) else {
            return;
        };
        let frame = shell.frame();
        let (Some(w), Some(h)) = (NonZeroU32::new(frame.width), NonZeroU32::new(frame.height))
        else {
            return;
        };
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        if buffer.len() == (frame.width * frame.height) as usize {
            to_xrgb(frame, &mut buffer);
            let _ = buffer.present();
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
        let window = match el.create_window(attributes) {
            Ok(w) => Rc::new(w),
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
        let context = match softbuffer::Context::new(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("vy-shell: no drawing surface: {e}");
                el.exit();
                return;
            }
        };
        match softbuffer::Surface::new(&context, window.clone()) {
            Ok(s) => self.surface = Some(s),
            Err(e) => {
                eprintln!("vy-shell: no drawing surface: {e}");
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
            exe: std::env::current_exe().ok(),
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
                shell.resize(
                    (f64::from(width) / scale) as f32,
                    (f64::from(height) / scale) as f32,
                    scale as f32,
                );
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let size = self.window.as_ref().map(|w| w.inner_size());
                if let Some(size) = size {
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

/// Open the window with `sources` in tabs (an empty tab when none) and
/// run until it closes.
pub fn run(sources: Vec<Source>) -> Result<(), String> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    let proxy = event_loop.create_proxy();
    let mut app = App {
        shell: None,
        proxy,
        adapter: None,
        a11y_active: false,
        a11y_digest: None,
        window: None,
        surface: None,
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
