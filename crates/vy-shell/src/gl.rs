//! The window's GL context (spec-fråga 66; SPEC-9.1.2: in the main process
//! only): OpenGL 3.3 core, or OpenGL ES 3.0, on the winit window through
//! `glutin` – EGL on Linux (X11 and Wayland), WGL on Windows. macOS builds
//! without it and draws on the CPU. Everything GL draws is
//! `vy_render::gl`; this module makes the context and presents.
//!
//! Every failure here returns the window (when one was made) and a
//! reason, and the caller goes on with the CPU path: no context, no
//! configuration, an old version, a shader that does not compile, a
//! framebuffer that is not complete (`vy_render::gl::GlRaster::new`), or
//! a panic anywhere in the setup, ours or a library's ([`guarded`];
//! granskning 0011:1).

use crate::shell::Shell;
use glutin::config::{Config, ConfigTemplateBuilder, GlConfig};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentGlContext, PossiblyCurrentContext,
    Version,
};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use std::num::NonZeroU32;
use vy_render::gl::{GlError, GlRaster, PageAt};
use winit::event_loop::ActiveEventLoop;
use winit::raw_window_handle::HasWindowHandle;
use winit::window::{Window, WindowAttributes};

/// Which page the page framebuffer holds (`PageLayer::version`).
pub type PageVersion = (u32, u64, u32, u32, u32, u64);

/// A current GL context on a window, with the pipeline built on it.
pub struct GlContext {
    /// The functions, loaded from the context.
    pub gl: glow::Context,
    /// The pipeline.
    pub raster: GlRaster,
    context: PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
}

impl GlContext {
    /// Resize the window's surface to `width` × `height` physical pixels.
    pub fn resize(&self, width: u32, height: u32) {
        if let (Some(w), Some(h)) = (NonZeroU32::new(width), NonZeroU32::new(height)) {
            self.surface.resize(&self.context, w, h);
        }
    }

    /// Show what was drawn on the default framebuffer.
    pub fn swap(&self) -> Result<(), GlError> {
        self.surface
            .swap_buffers(&self.context)
            .map_err(|e| GlError::Draw(format!("swap: {e}")))
    }

    /// Composite the window on the default framebuffer, to be shown by
    /// [`GlContext::swap`]: the shell's frame (transparent where the page
    /// shows, [`Shell::set_page_layer`]) over the page, which is drawn
    /// into its framebuffer first when its [`PageVersion`] is not the one
    /// `drawn` names. The page is cut to the page area: nothing of it
    /// shows outside (SPEC-9.6.1). `changed` is the rows of the frame
    /// (top down, `y0..y1`) that differ from the last one composited here.
    /// `Err`: leave the GL path.
    pub fn compose(
        &mut self,
        shell: &mut Shell,
        changed: Option<(u32, u32)>,
        drawn: &mut Option<PageVersion>,
    ) -> Result<(), GlError> {
        shell.frame();
        let mut at = None;
        let mut report = None;
        if let Some(layer) = shell.page_layer() {
            let p = layer.place;
            if *drawn != Some(layer.version) {
                let before = layer.raster.path();
                match layer.raster.render_gl(
                    &self.gl,
                    &mut self.raster,
                    layer.list,
                    p.width,
                    p.height,
                    p.scale,
                    layer.text,
                    layer.images,
                ) {
                    Ok(r) => report = Some((p.tab, r, before)),
                    // A list the reference refuses too: nothing to show.
                    Err(GlError::Render(_)) => {}
                    Err(e) => return Err(e),
                }
                *drawn = Some(layer.version);
            }
            at = Some(PageAt {
                x: p.at.0,
                y: p.at.1,
                clip: p.clip,
            });
        }
        if let Some((tab, r, before)) = report {
            shell.page_drawn(tab, r, before);
        }
        self.raster.present(&self.gl, shell.frame(), changed, at)
    }

    /// One line for the log.
    pub fn describe(&self) -> String {
        format!(
            "raster: {}, the fast software path for what it does not draw and if it fails",
            self.raster.version()
        )
    }
}

/// The first config without multisampling, or the first; `None` when the
/// display offers none. glutin's EGL answers a search with an iterator
/// that may be empty, and `DisplayBuilder::build` calls its picker all
/// the same.
fn pick(configs: Box<dyn Iterator<Item = Config> + '_>) -> Option<Config> {
    let all: Vec<Config> = configs.collect();
    all.iter()
        .find(|c| c.num_samples() == 0)
        .or(all.first())
        .cloned()
}

/// What [`pick_or_unwind`] unwinds with when there is no configuration.
struct NoConfig;

/// [`pick`] as `DisplayBuilder::build` wants its picker: one that returns
/// a config. With none to return it unwinds to [`guarded`] (without the
/// panic hook: nothing went wrong in the program), which makes it the
/// reason there is no GL.
fn pick_or_unwind(configs: Box<dyn Iterator<Item = Config> + '_>) -> Config {
    pick(configs).unwrap_or_else(|| std::panic::resume_unwind(Box::new(NoConfig)))
}

/// Run a step of the GL setup. A panic in it is a reason there is no GL,
/// as an error is: the client goes on with the CPU path instead of ending
/// where the fallback is needed.
fn guarded<T>(step: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(step)) {
        Ok(result) => result,
        Err(payload) if payload.is::<NoConfig>() => {
            Err("the GL display offers no configuration".into())
        }
        Err(payload) => {
            let what = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "no message".into());
            Err(format!("the GL setup panicked: {what}"))
        }
    }
}

/// Why there is no GL context, and the window when one was made, so the
/// CPU path can draw on it.
pub struct NoGl {
    /// The window, if it was made before the failure.
    pub window: Option<Window>,
    /// Why, for the log.
    pub why: String,
}

fn no(window: Option<Window>, why: String) -> Box<NoGl> {
    Box::new(NoGl { window, why })
}

/// Open a window from `attributes` with a GL context on it (`VY_NO_GL`
/// set in the environment: never).
pub fn open(
    el: &ActiveEventLoop,
    attributes: WindowAttributes,
) -> Result<(Window, GlContext), Box<NoGl>> {
    if std::env::var_os("VY_NO_GL").is_some() {
        return Err(no(None, "VY_NO_GL is set".into()));
    }
    let template = ConfigTemplateBuilder::new().with_transparency(false);
    let built = guarded(|| {
        DisplayBuilder::new()
            .with_window_attributes(Some(attributes))
            .build(el, template, pick_or_unwind)
            .map_err(|e| format!("no GL display: {e}"))
    });
    let (window, config) = built.map_err(|why| no(None, why))?;
    let Some(window) = window else {
        return Err(no(None, "no window from the GL display".into()));
    };
    match guarded(|| context(&window, &config)) {
        Ok(c) => Ok((window, c)),
        Err(e) => Err(no(Some(window), e)),
    }
}

fn context(window: &Window, config: &Config) -> Result<GlContext, String> {
    let display = config.display();
    let raw = window
        .window_handle()
        .map_err(|e| format!("window handle: {e}"))?
        .as_raw();
    let core = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .with_profile(GlProfile::Core)
        .build(Some(raw));
    let es = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
        .build(Some(raw));
    // SAFETY: `raw` is the live window's handle, and the window outlives
    // the context (both are owned by the caller, the context dropped first).
    let not_current = unsafe {
        display
            .create_context(config, &core)
            .or_else(|_| display.create_context(config, &es))
    }
    .map_err(|e| format!("no OpenGL 3.3 or OpenGL ES 3.0 context: {e}"))?;
    let attrs = window
        .build_surface_attributes(SurfaceAttributesBuilder::new())
        .map_err(|e| format!("surface: {e}"))?;
    // SAFETY: as above, for the surface.
    let surface = unsafe { display.create_window_surface(config, &attrs) }
        .map_err(|e| format!("surface: {e}"))?;
    let context = not_current
        .make_current(&surface)
        .map_err(|e| format!("make current: {e}"))?;
    // SAFETY: the context is current on this thread, and the loader
    // returns its functions.
    let gl = unsafe {
        glow::Context::from_loader_function_cstr(|s| display.get_proc_address(s) as *const _)
    };
    // SAFETY: `gl` is current on this thread and stays so: the window's
    // loop is the only thread that draws.
    let raster = unsafe { GlRaster::new(&gl) }.map_err(|e| e.to_string())?;
    Ok(GlContext {
        gl,
        raster,
        context,
        surface,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-11.3.2 (granskning 0011:1): the GPU path hands over to the
    /// software path at every failure of its setup, and a display that
    /// offers no configuration is one: the lookup says so instead of
    /// panicking, the picker glutin calls unwinds to the guard, and the
    /// guard makes that, and any other panic in the setup, a reason for
    /// the CPU path.
    #[test]
    fn spec_11_3_2_no_gl_configuration_is_a_fallback() {
        let none = || -> Box<dyn Iterator<Item = Config>> { Box::new(std::iter::empty()) };
        assert!(pick(none()).is_none());
        let why = guarded(|| Ok(pick_or_unwind(none()))).err();
        assert_eq!(
            why.as_deref(),
            Some("the GL display offers no configuration")
        );
        let why = guarded(|| -> Result<(), String> { panic!("driver fault {}", 7) }).unwrap_err();
        assert_eq!(why, "the GL setup panicked: driver fault 7");
        let why = guarded(|| -> Result<(), String> { std::panic::panic_any(7u8) }).unwrap_err();
        assert_eq!(why, "the GL setup panicked: no message");
        assert_eq!(guarded(|| Ok::<_, String>(3)), Ok(3));
        assert_eq!(
            guarded(|| Err::<(), _>("no GL display: x".to_string())),
            Err("no GL display: x".into())
        );
    }
}
