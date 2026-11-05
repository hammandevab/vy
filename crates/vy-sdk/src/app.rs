//! The page as a type: [`App`] plus the glue `export!` wraps around it to
//! produce the six exports of SPEC-5.1.
//!
//! The glue owns the lifecycle the spec prescribes: `vy_init` constructs the
//! app and returns 0, or the status code of the error that stopped it, in
//! which case the host rejects the page (SPEC-5.1.1); `vy_frame` opens a [`Frame`], lets the app draw, closes
//! it and returns the opcode count (SPEC-5.1.2); `vy_event` decodes the
//! arguments and returns 1 when the app wants a redraw (SPEC-5.1.3);
//! `vy_resize` and `vy_suspend` forward (SPEC-5.1.4, SPEC-5.1.5);
//! `vy_alloc` serves the arena (SPEC-5.1.6). Calls before `vy_init` are
//! ignored and return 0.

use crate::api::{log, Level};
use crate::arena;
use crate::error::VyError;
use crate::event::Event;
use crate::frame::Frame;
use core::cell::UnsafeCell;

/// The window the page draws into (SPEC-5.1.1, SPEC-5.1.4).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Window {
    /// Width in logical pixels.
    pub width: f32,
    /// Height in logical pixels.
    pub height: f32,
    /// Logical-to-physical ratio (SPEC-6.3.1).
    pub scale: f32,
}

/// A Vy page. Implement it and hand the type to `vy_sdk::export!`.
pub trait App: Sized + 'static {
    /// `vy_init`: build the page for a window; `abi_minor` is the host's
    /// ABI minor version (SPEC-5.1.1, SPEC-4.3.2). An `Err` (a style the
    /// host refused, say) becomes `vy_init`'s return value, its status
    /// code, and the host does not run the page; there is no page without
    /// what `init` asked for, so no `unwrap_or` is needed.
    fn init(window: Window, abi_minor: i32) -> Result<Self, VyError>;

    /// `vy_frame`: describe the whole page into `f` (SPEC-6.1.2); the glue
    /// opened it and will close it. An `Err` is logged at `Level::Error` and
    /// the frame is still ended.
    fn frame(&mut self, f: &mut Frame, dt_ms: i32) -> Result<(), VyError>;

    /// `vy_event`: return `true` to ask for a frame (SPEC-5.1.3, SPEC-6.2.1).
    fn event(&mut self, event: Event<'_>) -> bool;

    /// `vy_resize`: the window changed size or DPI (SPEC-5.1.4).
    fn resize(&mut self, window: Window);

    /// `vy_suspend`: the tab is being hidden; return value is passed through
    /// (SPEC-5.1.5). Default: 0.
    fn suspend(&mut self) -> i32 {
        0
    }
}

/// Storage for the one app instance; declared by `export!`.
pub struct Slot<A>(UnsafeCell<Option<A>>);

// SAFETY: modules are single-threaded (SPEC-4.2.3) and the host never
// re-enters an export (SPEC-5.3.4), so no two references to the app exist
// at once.
unsafe impl<A> Sync for Slot<A> {}

impl<A> Slot<A> {
    /// An empty slot.
    pub const fn new() -> Self {
        Self(UnsafeCell::new(None))
    }

    #[allow(clippy::mut_from_ref)]
    pub(crate) fn get(&self) -> Option<&mut A> {
        // SAFETY: see `Sync`; the glue holds this reference only for the
        // duration of one export call.
        unsafe { (*self.0.get()).as_mut() }
    }
}

impl<A> Default for Slot<A> {
    fn default() -> Self {
        Self::new()
    }
}

/// `vy_init` (SPEC-5.1.1): 0, or the status code of the error `App::init`
/// returned, logged at `Level::Error`; the slot stays empty then and every
/// later export is ignored.
pub fn init<A: App>(slot: &Slot<A>, abi_minor: i32, width: i32, height: i32, scale: f32) -> i32 {
    let window = Window {
        width: width as f32,
        height: height as f32,
        scale,
    };
    match A::init(window, abi_minor) {
        Ok(app) => {
            // SAFETY: see `Slot`; no reference into the slot is live here.
            unsafe { *slot.0.get() = Some(app) };
            0
        }
        Err(e) => {
            let _ = log(Level::Error, e.name());
            e.code()
        }
    }
}

/// `vy_frame` (SPEC-5.1.2): returns the opcodes written.
pub fn frame<A: App>(slot: &Slot<A>, dt_ms: i32) -> i32 {
    let Some(app) = slot.get() else {
        return 0;
    };
    let mut f = Frame::begin();
    if let Err(e) = app.frame(&mut f, dt_ms) {
        let _ = log(Level::Error, e.name());
    }
    let ops = f.ops();
    match f.end() {
        Ok(n) => n as i32,
        Err(_) => ops as i32,
    }
}

/// `vy_event` (SPEC-5.1.3): 1 when the app asks for a redraw.
pub fn event<A: App>(slot: &Slot<A>, kind: i32, a: i32, b: i32, c: i32, d: i32) -> i32 {
    let Some(app) = slot.get() else {
        return 0;
    };
    let ev = Event::decode(kind, a, b, c, d, |ptr, len| {
        arena::bytes(ptr, len).and_then(|b| core::str::from_utf8(b).ok())
    });
    i32::from(app.event(ev))
}

/// `vy_resize` (SPEC-5.1.4).
pub fn resize<A: App>(slot: &Slot<A>, width: i32, height: i32, scale: f32) {
    if let Some(app) = slot.get() {
        app.resize(Window {
            width: width as f32,
            height: height as f32,
            scale,
        });
    }
}

/// `vy_suspend` (SPEC-5.1.5).
pub fn suspend<A: App>(slot: &Slot<A>) -> i32 {
    slot.get().map_or(0, App::suspend)
}

/// `vy_alloc` (SPEC-5.1.6).
pub fn alloc(size: i32) -> i32 {
    arena::alloc(size)
}

/// Generate the six exports of SPEC-5.1 (and nothing else, SPEC-4.2.1)
/// around an [`App`] type.
///
/// ```
/// use vy_sdk::prelude::*;
/// struct Page;
/// impl App for Page {
///     fn init(_: Window, _: i32) -> Result<Self, VyError> { Ok(Page) }
///     fn frame(&mut self, _: &mut Frame, _: i32) -> Result<(), VyError> { Ok(()) }
///     fn event(&mut self, _: Event<'_>) -> bool { false }
///     fn resize(&mut self, _: Window) {}
/// }
/// vy_sdk::export!(Page);
/// assert_eq!(vy_init(0, 100, 100, 1.0), 0);
/// assert_eq!(vy_frame(16), 0);
/// ```
#[macro_export]
macro_rules! export {
    ($app:ty) => {
        static __VY_APP: $crate::app::Slot<$app> = $crate::app::Slot::new();

        #[no_mangle]
        pub extern "C" fn vy_init(abi_minor: i32, width: i32, height: i32, scale: f32) -> i32 {
            $crate::app::init(&__VY_APP, abi_minor, width, height, scale)
        }

        #[no_mangle]
        pub extern "C" fn vy_frame(dt_ms: i32) -> i32 {
            $crate::app::frame(&__VY_APP, dt_ms)
        }

        #[no_mangle]
        pub extern "C" fn vy_event(kind: i32, a: i32, b: i32, c: i32, d: i32) -> i32 {
            $crate::app::event(&__VY_APP, kind, a, b, c, d)
        }

        #[no_mangle]
        pub extern "C" fn vy_resize(width: i32, height: i32, scale: f32) {
            $crate::app::resize(&__VY_APP, width, height, scale)
        }

        #[no_mangle]
        pub extern "C" fn vy_suspend() -> i32 {
            $crate::app::suspend(&__VY_APP)
        }

        #[no_mangle]
        pub extern "C" fn vy_alloc(size: i32) -> i32 {
            $crate::app::alloc(size)
        }
    };
}

/// A panic is `unreachable`: the host reports the trap and the tab stops
/// (SPEC-4.1.4). Nothing is printed; the module has no output but the log.
#[cfg(all(target_arch = "wasm32", not(test)))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    core::arch::wasm32::unreachable()
}
