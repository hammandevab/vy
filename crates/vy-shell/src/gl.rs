//! The window's GL context (spec-fråga 66; SPEC-9.1.2: in the main process
//! only): OpenGL 3.3 core, or OpenGL ES 3.0, on the winit window through
//! `glutin` – EGL on Linux (X11 and Wayland), WGL on Windows. macOS builds
//! without it and draws on the CPU. Everything GL draws is
//! `vy_render::gl`; this module makes the context and presents.
//!
//! A context lost to a graphics reset (granskning 0011:5): the context is
//! asked for with reset notification where the platform has it, and the
//! driver is asked before every frame whether it was lost
//! ([`GlContext::alive`]); the window then goes on with the CPU path. A
//! driver without the query, or one that loses a context without saying
//! so, leaves a window that no longer draws: that is not noticed.
//!
//! Every failure here returns the window (when one was made) and a
//! reason, and the caller goes on with the CPU path: no context, no
//! configuration, an old version, a shader that does not compile, a
//! framebuffer that is not complete (`vy_render::gl::GlRaster::new`), or
//! a panic anywhere in the setup, ours or a library's ([`guarded`];
//! granskning 0011:1).

use crate::shell::Shell;
use glow::HasContext;
use glutin::config::{Config, ConfigTemplateBuilder, GlConfig};
use glutin::context::{
    ContextApi, ContextAttributesBuilder, GlProfile, NotCurrentGlContext, PossiblyCurrentContext,
    Robustness, Version,
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

/// What [`GlContext::compose`] knows of the page framebuffer between
/// frames; the default is an empty framebuffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PageState {
    /// The page last handed to the pipeline.
    version: Option<PageVersion>,
    /// True when the framebuffer holds that page's picture. False when
    /// its list was refused: the framebuffer then still holds an earlier
    /// picture, of this tab or of another, which is not shown.
    shows: bool,
}

impl PageState {
    /// The page the framebuffer's picture is of, when it is shown.
    pub fn shown(&self) -> Option<PageVersion> {
        self.version.filter(|_| self.shows)
    }
}

/// A current GL context on a window, with the pipeline built on it.
pub struct GlContext {
    /// The functions, loaded from the context.
    pub gl: glow::Context,
    /// The pipeline.
    pub raster: GlRaster,
    context: PossiblyCurrentContext,
    surface: Surface<WindowSurface>,
    /// The context's `glGetGraphicsResetStatus`, when it has one.
    reset_status: Option<ResetStatus>,
    /// True when the context reports a graphics reset by being lost
    /// (made with reset notification, and the driver agreed).
    notified: bool,
}

/// `glGetGraphicsResetStatus`: no arguments, a status.
type ResetStatus = unsafe extern "system" fn() -> u32;

/// What a status of `glGetGraphicsResetStatus` says happened to the
/// context; `None` for a context that is whole.
fn reset_reason(status: u32) -> Option<&'static str> {
    match status {
        glow::NO_ERROR => None,
        glow::GUILTY_CONTEXT_RESET => Some("a graphics reset this context caused"),
        glow::INNOCENT_CONTEXT_RESET => Some("a graphics reset another context caused"),
        glow::UNKNOWN_CONTEXT_RESET => Some("a graphics reset of unknown cause"),
        _ => Some("a reset status the driver does not name"),
    }
}

/// The name of `glGetGraphicsResetStatus` in a context of this API and
/// version with these extensions, or `None` when it has no such
/// function: in OpenGL since 4.5 and in OpenGL ES since 3.2, before that
/// from `KHR_robustness`, `ARB_robustness` (OpenGL) or `EXT_robustness`
/// (OpenGL ES), under the extension's own name.
fn reset_status_name(
    embedded: bool,
    version: (u32, u32),
    has: impl Fn(&str) -> bool,
) -> Option<&'static std::ffi::CStr> {
    if embedded {
        if version >= (3, 2) {
            Some(c"glGetGraphicsResetStatus")
        } else if has("GL_KHR_robustness") {
            Some(c"glGetGraphicsResetStatusKHR")
        } else if has("GL_EXT_robustness") {
            Some(c"glGetGraphicsResetStatusEXT")
        } else {
            None
        }
    } else if version >= (4, 5) || has("GL_KHR_robustness") {
        Some(c"glGetGraphicsResetStatus")
    } else if has("GL_ARB_robustness") {
        Some(c"glGetGraphicsResetStatusARB")
    } else {
        None
    }
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
    /// `page` names. The page is cut to the page area: nothing of it
    /// shows outside (SPEC-9.6.1). A page whose list the reference
    /// refuses is not shown at all, as on the CPU path: what the
    /// framebuffer holds then is an earlier picture, perhaps another
    /// tab's (granskning 0011:3). `changed` is the rows of the frame (top
    /// down, `y0..y1`) that differ from the last one composited here.
    /// `Err`: leave the GL path.
    pub fn compose(
        &mut self,
        shell: &mut Shell,
        changed: Option<(u32, u32)>,
        page: &mut PageState,
    ) -> Result<(), GlError> {
        self.alive()?;
        shell.frame();
        let mut at = None;
        let mut report = None;
        let mut refused = None;
        if let Some(layer) = shell.page_layer() {
            let p = layer.place;
            if page.version != Some(layer.version) {
                let before = layer.raster.path();
                page.shows = match layer.raster.render_gl(
                    &self.gl,
                    &mut self.raster,
                    layer.list,
                    p.width,
                    p.height,
                    p.scale,
                    layer.text,
                    layer.images,
                ) {
                    Ok(r) => {
                        report = Some((p.tab, r, before));
                        true
                    }
                    // A list the reference refuses too: nothing to show,
                    // now or until the page changes.
                    Err(GlError::Render(e)) => {
                        refused = Some((p.tab, e.to_string()));
                        false
                    }
                    Err(e) => return Err(e),
                };
                page.version = Some(layer.version);
            }
            if page.shows {
                at = Some(PageAt {
                    x: p.at.0,
                    y: p.at.1,
                    clip: p.clip,
                });
            }
        }
        if let Some((tab, r, before)) = report {
            shell.page_drawn(tab, r, before);
        }
        if let Some((tab, why)) = refused {
            shell.page_not_drawn(tab, &why);
        }
        self.raster.present(&self.gl, shell.frame(), changed, at)
    }

    /// `Err` when the driver says the context was lost to a graphics
    /// reset (a driver crash or update, a GPU taken away): nothing drawn
    /// with it shows any more, and the window leaves the GL path. Cheap:
    /// one call without arguments. `Ok` also where the driver has no such
    /// query ([`GlContext::notices_loss`]).
    pub fn alive(&self) -> Result<(), GlError> {
        let Some(status) = self.reset_status else {
            return Ok(());
        };
        // SAFETY: the function is this context's own
        // `glGetGraphicsResetStatus`, taken from its loader only when its
        // version or extensions have one; it takes nothing and returns an
        // enum, and the context is current on this thread.
        match reset_reason(unsafe { status() }) {
            None => Ok(()),
            Some(why) => Err(GlError::Draw(format!("the GL context was lost to {why}"))),
        }
    }

    /// True when a context lost to a graphics reset is noticed
    /// ([`GlContext::alive`]): the driver has the query and the context
    /// was made with reset notification.
    pub fn notices_loss(&self) -> bool {
        self.reset_status.is_some() && self.notified
    }

    /// One line for the log.
    pub fn describe(&self) -> String {
        format!(
            "raster: {}, the fast software path for what it does not draw and if it fails; a context lost to a graphics reset is {}",
            self.raster.version(),
            if self.notices_loss() {
                "noticed"
            } else {
                "not noticed unless GL reports an error"
            }
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
    // OpenGL 3.3 core, else OpenGL ES 3.0; each first as a context that
    // is lost, and says so, at a graphics reset (granskning 0011:5), and
    // then without, where the platform cannot make one that way.
    let core = |robustness| {
        ContextAttributesBuilder::new()
            .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
            .with_profile(GlProfile::Core)
            .with_robustness(robustness)
            .build(Some(raw))
    };
    let es = |robustness| {
        ContextAttributesBuilder::new()
            .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
            .with_robustness(robustness)
            .build(Some(raw))
    };
    let (lose, plain) = (Robustness::RobustLoseContextOnReset, Robustness::NotRobust);
    // SAFETY: `raw` is the live window's handle, and the window outlives
    // the context (both are owned by the caller, the context dropped first).
    let not_current = unsafe {
        display
            .create_context(config, &core(lose))
            .or_else(|_| display.create_context(config, &core(plain)))
            .or_else(|_| display.create_context(config, &es(lose)))
            .or_else(|_| display.create_context(config, &es(plain)))
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
    // The driver's word on a lost context, where the context has the
    // query (never asked for by name alone: a loader may hand out a
    // function the context does not support).
    let v = gl.version();
    let name = reset_status_name(v.is_embedded, (v.major, v.minor), |e| {
        gl.supported_extensions().contains(e)
    });
    let reset_status = name
        .map(|n| display.get_proc_address(n))
        .filter(|p| !p.is_null())
        // SAFETY: a non-null address the context's loader gave for
        // `glGetGraphicsResetStatus` under the name its version or
        // extension defines, which has this signature.
        .map(|p| unsafe { std::mem::transmute::<*const std::ffi::c_void, ResetStatus>(p) });
    // SAFETY: the context is current; the parameter exists wherever the
    // query above does. An error it might leave is taken off, so that it
    // is not found after the first frame and read as a failure to draw.
    let notified = reset_status.is_some()
        && unsafe {
            let strategy =
                glow::HasContext::get_parameter_i32(&gl, glow::RESET_NOTIFICATION_STRATEGY);
            let _ = glow::HasContext::get_error(&gl);
            strategy == glow::LOSE_CONTEXT_ON_RESET as i32
        };
    Ok(GlContext {
        gl,
        raster,
        context,
        surface,
        reset_status,
        notified,
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

    /// SPEC-11.3.2 (granskning 0011:5): the GPU path hands over to the
    /// software path when its context is lost. What the driver's reset
    /// status means, and under which name a context has the query: core
    /// since OpenGL 4.5 and OpenGL ES 3.2, by extension before, and not at
    /// all in a context that has neither (where a loss is not noticed).
    #[test]
    fn spec_11_3_2_a_lost_context_is_asked_for() {
        assert_eq!(reset_reason(glow::NO_ERROR), None);
        for lost in [
            glow::GUILTY_CONTEXT_RESET,
            glow::INNOCENT_CONTEXT_RESET,
            glow::UNKNOWN_CONTEXT_RESET,
            0x1234,
        ] {
            assert!(reset_reason(lost).is_some(), "{lost:#x}");
        }
        let name = |embedded, version, exts: &[&str]| {
            reset_status_name(embedded, version, |e| exts.contains(&e)).map(|n| n.to_str().unwrap())
        };
        assert_eq!(name(false, (4, 5), &[]), Some("glGetGraphicsResetStatus"));
        assert_eq!(name(false, (4, 6), &[]), Some("glGetGraphicsResetStatus"));
        assert_eq!(
            name(false, (3, 3), &["GL_KHR_robustness", "GL_ARB_robustness"]),
            Some("glGetGraphicsResetStatus")
        );
        assert_eq!(
            name(false, (3, 3), &["GL_ARB_robustness"]),
            Some("glGetGraphicsResetStatusARB")
        );
        assert_eq!(name(false, (3, 3), &["GL_EXT_robustness"]), None);
        assert_eq!(name(false, (4, 4), &[]), None);
        assert_eq!(name(true, (3, 2), &[]), Some("glGetGraphicsResetStatus"));
        assert_eq!(
            name(true, (3, 0), &["GL_KHR_robustness"]),
            Some("glGetGraphicsResetStatusKHR")
        );
        assert_eq!(
            name(true, (3, 0), &["GL_EXT_robustness"]),
            Some("glGetGraphicsResetStatusEXT")
        );
        assert_eq!(name(true, (3, 1), &["GL_ARB_robustness"]), None);
        assert_eq!(name(true, (4, 5), &[]), Some("glGetGraphicsResetStatus"));
    }
}
