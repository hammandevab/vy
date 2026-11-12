//! The window's GL context (spec-fråga 66; SPEC-9.1.2: in the main process
//! only): OpenGL 3.3 core, or OpenGL ES 3.0, on the winit window through
//! `glutin` – EGL on Linux (X11 and Wayland), WGL on Windows. macOS builds
//! without it and draws on the CPU. Everything GL draws is
//! `vy_render::gl`; this module makes the context and presents.
//!
//! Every failure here returns the window (when one was made) and a
//! reason, and the caller goes on with the CPU path: no context, an old
//! version, a shader that does not compile, a framebuffer that is not
//! complete (`vy_render::gl::GlRaster::new`).

use glutin::config::{Config, ConfigTemplateBuilder, GlConfig};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentGlContext, PossiblyCurrentContext,
    Version,
};
use glutin::display::{GetGlDisplay, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use std::num::NonZeroU32;
use vy_render::gl::{GlError, GlRaster};
use winit::event_loop::ActiveEventLoop;
use winit::raw_window_handle::HasWindowHandle;
use winit::window::{Window, WindowAttributes};

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

    /// One line for the log.
    pub fn describe(&self) -> String {
        format!(
            "raster: {}, the fast software path for what it does not draw and if it fails",
            self.raster.version()
        )
    }
}

/// The first config without multisampling, or the first.
fn pick(configs: Box<dyn Iterator<Item = Config> + '_>) -> Config {
    let all: Vec<Config> = configs.collect();
    all.iter()
        .find(|c| c.num_samples() == 0)
        .or(all.first())
        .cloned()
        .expect("glutin offers at least one config")
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
    let (window, config) = DisplayBuilder::new()
        .with_window_attributes(Some(attributes))
        .build(el, template, pick)
        .map_err(|e| no(None, format!("no GL display: {e}")))?;
    let Some(window) = window else {
        return Err(no(None, "no window from the GL display".into()));
    };
    match context(&window, &config) {
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
