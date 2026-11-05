//! The shell without a window: the same [`Shell`], tab processes and
//! compositing, driven to a settled state and read out as pixels. This is
//! `vy-shell --screenshot` and what the conformance suite's shell cases
//! and the tests look at (SPEC-11.3.2: one render path, window or not).
//!
//! Deterministic by construction (SPEC-2.1.5): the clock is the
//! headless one (16 ms per tick), the random source the fixed seed,
//! nothing is persisted, and the drive stops at the first frame of a
//! static page or after a fixed number of ticks of an animating one.

use crate::clipboard::MemoryClipboard;
use crate::external::RecordingOpener;
use crate::page::{LoadedPage, Source};
use crate::paths::scratch_dir;
use crate::shell::{Config, Shell, Waker};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vy_render::RgbaImage;

/// How a screenshot is taken.
#[derive(Clone, Debug)]
pub struct Options {
    /// Logical window width.
    pub width: f32,
    /// Logical window height.
    pub height: f32,
    /// Scale (SPEC-6.3.1).
    pub scale: f32,
    /// Most ticks to send; a static page settles after one.
    pub max_ticks: u32,
    /// The tab executable; `None` runs the tab on a thread (tests).
    pub exe: Option<PathBuf>,
    /// How long to wait for the tab in all.
    pub timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            width: 800.0,
            height: 600.0,
            scale: 1.0,
            max_ticks: 1,
            exe: None,
            timeout: Duration::from_secs(60),
        }
    }
}

/// A shell set up for headless use: in-memory grants and clipboard, a
/// scratch data directory, a recording opener and no waker.
pub fn headless_shell(
    exe: Option<PathBuf>,
    width: f32,
    height: f32,
    scale: f32,
) -> std::io::Result<Shell> {
    Shell::new(
        Config {
            exe,
            data_dir: scratch_dir("headless"),
            headless: true,
            opener: Box::new(RecordingOpener::default()),
            clipboard: Box::new(MemoryClipboard::new()),
            waker: Waker::none(),
        },
        width,
        height,
        scale,
    )
}

/// Drive `shell` until its active tab is settled: its page failed,
/// stopped or went away; or it has drawn and asks for nothing more; or
/// `max_ticks` ticks have been sent and answered. Returns the number of
/// frames received.
pub fn settle(shell: &mut Shell, max_ticks: u32, timeout: Duration) -> Result<u64, String> {
    let deadline = Instant::now() + timeout;
    let mut ticks = 0u32;
    loop {
        let Some(id) = shell.active_tab() else {
            return Ok(0);
        };
        if shell.tab_terminal(id) {
            return Ok(shell.tab_frames(id));
        }
        if !shell.tick_pending() && ticks < max_ticks && shell.wants_tick() {
            shell.tick();
            ticks += 1;
        }
        let frames = shell.tab_frames(id);
        // Settled: drawn and asking for nothing more, out of ticks, or
        // hidden under its `min_size` where only the fallback shows.
        if !shell.tick_pending()
            && (frames > 0 && !shell.wants_tick()
                || ticks >= max_ticks
                || (shell.tab_status(id) == "running" && shell.tab_hidden(id)))
        {
            return Ok(frames);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(format!(
                "timed out after {timeout:?} waiting for the tab ({})",
                shell.tab_status(id)
            ));
        }
        match shell.inbox().recv_timeout(deadline - now) {
            Ok(m) => shell.handle(m),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err("the shell's inbox closed".into())
            }
        }
    }
}

/// Load `source` in a fresh headless shell and return the composited
/// frame once the page has settled.
pub fn screenshot(source: Source, options: &Options) -> Result<RgbaImage, String> {
    let mut shell = fresh(options)?;
    shell.open_tab(Some(source));
    capture(shell, options)
}

/// [`screenshot`] of a page already in hand (see
/// `Shell::open_tab_loaded`): one the caller fetched and judged, trusted
/// or not, so the frame can show grants and the capability question.
pub fn screenshot_loaded(page: LoadedPage, options: &Options) -> Result<RgbaImage, String> {
    let mut shell = fresh(options)?;
    shell.open_tab_loaded(page);
    capture(shell, options)
}

fn fresh(options: &Options) -> Result<Shell, String> {
    headless_shell(
        options.exe.clone(),
        options.width,
        options.height,
        options.scale,
    )
    .map_err(|e| e.to_string())
}

fn capture(mut shell: Shell, options: &Options) -> Result<RgbaImage, String> {
    settle(&mut shell, options.max_ticks, options.timeout)?;
    let image = shell.frame().clone();
    shell.shutdown();
    Ok(image)
}
