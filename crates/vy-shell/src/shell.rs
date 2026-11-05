//! The shell's model: tabs and their processes, the address bar, history,
//! capability decisions, navigation, input routing, ticks and
//! compositing. Everything the window does goes through here, and
//! nothing here needs a window: the headless screenshot and the tests
//! drive the same object.
//!
//! Threads: the UI thread owns the [`Shell`]; every tab has a reader
//! thread and the loader thread does the fetching; all of them report
//! into one inbox ([`Incoming`]) and call the [`Waker`], which the window
//! turns into a wake-up of its event loop.

use crate::address::{AddressField, Edit};
use crate::chrome::{self, Layout, PageView, Rect, Styles, TabInfo, View};
use crate::compose::{blend, blit, physical};
use crate::external::{is_external, Opener};
use crate::grants::{GrantStore, Resolved};
use crate::history::History;
use crate::ipc::{LoadRequest, ToShell, ToTab};
use crate::markdown::{self, Block};
use crate::page::{self, Done, Job, LoadError, LoadedPage, Source};
use crate::process::{self, Inbound, TabConnection, TabId};
use crate::storage;
use crate::tab::pack_capabilities;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::Instant;
use vy_abi::{cap, event, key, modifier};
use vy_manifest::Manifest;
use vy_render::{ImageCache, RgbaImage, TextEngine};
use vy_runtime::{Capabilities, Link, RandomSource, SemNode};

/// Milliseconds between frames of an animating page, and the headless
/// clock's step per tick.
pub const FRAME_MS: i64 = 16;
/// Font scale steps (Ctrl+Plus/Minus).
const ZOOM_STEPS: [f32; 9] = [0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 2.0];
/// Bytes a tab may fetch per minute (SPEC-9.3.1).
const NET_BYTES_PER_MINUTE: u64 = 256 << 20;
/// Log lines kept per tab.
const MAX_TAB_LOGS: usize = 200;

/// Something that wakes the UI thread after a message was put in the inbox.
#[derive(Clone)]
pub struct Waker(Arc<dyn Fn() + Send + Sync>);

impl Waker {
    /// A waker from a closure.
    pub fn new(f: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }
    /// A waker that does nothing (a driver that polls the inbox itself).
    pub fn none() -> Self {
        Self(Arc::new(|| {}))
    }
    /// Wake.
    pub fn wake(&self) {
        (self.0)();
    }
}

/// What arrives in the shell's inbox from its threads.
#[derive(Debug)]
pub enum Incoming {
    /// From a tab's reader: the tab, the navigation the process belongs
    /// to (a message from an earlier process of the same tab is dropped),
    /// and the message.
    Tab(TabId, u64, Inbound),
    /// From the loader.
    Load(Done),
}

/// How the shell is set up.
pub struct Config {
    /// The executable started as a tab process; `None` runs tabs on
    /// threads of this process (tests only; no isolation).
    pub exe: Option<PathBuf>,
    /// Grants, page storage, caches and keys live here.
    pub data_dir: PathBuf,
    /// Deterministic clock and random source, for screenshots and tests.
    pub headless: bool,
    /// Opens `https://` addresses (SPEC-2.1.7).
    pub opener: Box<dyn Opener>,
    /// Wakes the UI thread.
    pub waker: Waker,
}

/// The pointer the window should show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cursor {
    /// The arrow.
    Arrow,
    /// A hand, over a link.
    Hand,
    /// An I-beam, over the address field.
    Text,
}

#[derive(Clone, Debug, PartialEq)]
enum State {
    Empty,
    Loading,
    Running,
    Failed(LoadError),
    Rejected(String),
    Stopped(String),
    Gone(String),
}

/// One tab.
struct Tab {
    id: TabId,
    conn: Option<TabConnection>,
    /// Counts navigations, so a late result for an earlier one is dropped.
    generation: u64,
    source: Option<Source>,
    display: String,
    origin: Option<String>,
    manifest: Option<Manifest>,
    title: String,
    state: State,
    history: History,
    trusted: bool,
    caps: Vec<Resolved>,
    asks: VecDeque<&'static str>,
    is_localhost: bool,
    /// The manifest's `net` origins as `vy_runtime::origin_of` writes
    /// them (SPEC-5.2.8): the only origins a request may go to. Decided
    /// here, in the main process; the tab process is not trusted to.
    net_origins: Vec<String>,
    /// The page's storage quota in bytes (SPEC-9.3.1).
    storage_quota_bytes: u64,
    /// The page's styles, mirrored from the tab (SPEC-9.1.2).
    text: TextEngine,
    images: ImageCache,
    display_list: Vec<u8>,
    rendered: Option<(u32, u32, f32, RgbaImage)>,
    links: Vec<Link>,
    sem: Vec<SemNode>,
    has_tree: Option<bool>,
    loaded: bool,
    tick_pending: bool,
    wants_more: bool,
    tick_due: bool,
    frames: u64,
    sent_size: (u32, u32, f32),
    zoom: f32,
    ime: Option<(f32, f32, f32, f32)>,
    logs: VecDeque<String>,
    net_window: (i64, u64),
    pressed_link: Option<String>,
    hidden: bool,
    under_min: bool,
}

impl Tab {
    fn new(id: TabId) -> Self {
        Self {
            id,
            conn: None,
            generation: 0,
            source: None,
            display: String::new(),
            origin: None,
            manifest: None,
            title: String::new(),
            state: State::Empty,
            history: History::new(),
            trusted: false,
            caps: Vec::new(),
            asks: VecDeque::new(),
            is_localhost: false,
            net_origins: Vec::new(),
            storage_quota_bytes: 0,
            text: TextEngine::new(),
            images: ImageCache::empty(0),
            display_list: Vec::new(),
            rendered: None,
            links: Vec::new(),
            sem: Vec::new(),
            has_tree: None,
            loaded: false,
            tick_pending: false,
            wants_more: false,
            tick_due: false,
            frames: 0,
            sent_size: (0, 0, 0.0),
            zoom: 1.0,
            ime: None,
            logs: VecDeque::new(),
            net_window: (0, 0),
            pressed_link: None,
            hidden: false,
            under_min: false,
        }
    }

    /// Whether a request the page wants made may be made (SPEC-5.2.8,
    /// SPEC-5.3.1): `net` must be granted now, and the URL's origin must
    /// be one the manifest named. The tab process applies the same rule
    /// before asking, but it runs page code, so the main process decides
    /// (SPEC-2.1.4: no ambient authority).
    fn request_verdict(&self, url: &str) -> Result<(), i32> {
        if !self.capabilities().has(cap::NET) {
            return Err(vy_abi::err::E_DENIED);
        }
        match vy_runtime::origin_of(url) {
            Some(origin) if self.net_origins.contains(&origin) => Ok(()),
            _ => Err(vy_abi::err::E_ORIGIN),
        }
    }

    fn running(&self) -> bool {
        self.loaded && self.state == State::Running && self.conn.is_some()
    }

    fn send(&self, msg: &ToTab) {
        if let Some(c) = &self.conn {
            c.send(msg);
        }
    }

    fn log(&mut self, line: String) {
        if self.logs.len() == MAX_TAB_LOGS {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
    }

    fn badges(&self) -> Vec<(&'static str, bool)> {
        self.caps.iter().map(|r| (r.name, r.granted)).collect()
    }

    fn capabilities(&self) -> Capabilities {
        let mut caps = Capabilities::default();
        for r in &self.caps {
            if r.granted {
                caps.grant(r.name);
            }
        }
        caps
    }

    fn link_at(&self, x: f32, y: f32) -> Option<&Link> {
        self.links
            .iter()
            .rev()
            .find(|l| x >= l.x && y >= l.y && x < l.x + l.w && y < l.y + l.h)
    }
}

/// The shell.
pub struct Shell {
    config: Config,
    inbox_rx: Receiver<Incoming>,
    inbox_tx: Sender<Incoming>,
    loader: Sender<Job>,
    grants: GrantStore,
    tabs: Vec<Tab>,
    active: usize,
    next_id: TabId,
    address: AddressField,
    width: f32,
    height: f32,
    scale: f32,
    chrome_text: TextEngine,
    styles: Styles,
    layout: Layout,
    frame: RgbaImage,
    frame_dirty: bool,
    /// The chrome as last rasterized: its list and pixels, reused while
    /// the list is the same (a page frame changes nothing in it).
    chrome_cache: Option<(Vec<u8>, u32, u32, RgbaImage)>,
    pointer: (f32, f32),
    in_page: bool,
    buttons: i32,
    modifiers: i32,
    hover: Option<String>,
    clipboard: Vec<u8>,
    clock_headless: i64,
    started: Instant,
    quit: bool,
    external_opens: Vec<String>,
}

impl Shell {
    /// A shell with a window of `width` × `height` logical pixels at `scale`.
    pub fn new(config: Config, width: f32, height: f32, scale: f32) -> std::io::Result<Self> {
        let (inbox_tx, inbox_rx) = std::sync::mpsc::channel();
        let waker = config.waker.clone();
        let report_tx = inbox_tx.clone();
        let loader = page::start_loader(config.data_dir.join("cache"), move |done| {
            let _ = report_tx.send(Incoming::Load(done));
            waker.wake();
        })?;
        let grants = if config.headless {
            GrantStore::in_memory()
        } else {
            GrantStore::open(&config.data_dir.join("grants.txt"))
        };
        let mut chrome_text = TextEngine::new();
        let styles = Styles::register(&mut chrome_text);
        let mut shell = Self {
            config,
            inbox_rx,
            inbox_tx,
            loader,
            grants,
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            address: AddressField::new(),
            width: width.max(1.0),
            height: height.max(1.0),
            scale: if scale.is_finite() && scale > 0.0 {
                scale
            } else {
                1.0
            },
            chrome_text,
            styles,
            layout: Layout::default(),
            frame: RgbaImage::new(1, 1),
            frame_dirty: true,
            chrome_cache: None,
            pointer: (-1.0, -1.0),
            in_page: false,
            buttons: 0,
            modifiers: 0,
            hover: None,
            clipboard: Vec::new(),
            clock_headless: 0,
            started: Instant::now(),
            quit: false,
            external_opens: Vec::new(),
        };
        shell.relayout();
        Ok(shell)
    }

    /// The inbox, for a driver that blocks on it.
    pub fn inbox(&self) -> &Receiver<Incoming> {
        &self.inbox_rx
    }

    /// Handle every message waiting in the inbox.
    pub fn drain(&mut self) {
        while let Ok(m) = self.inbox_rx.try_recv() {
            self.handle(m);
        }
    }

    /// The clock the tabs see (SPEC-5.2.11): fixed steps headless, the
    /// monotonic clock in a window.
    pub fn now_ms(&self) -> i64 {
        if self.config.headless {
            self.clock_headless
        } else {
            self.started.elapsed().as_millis() as i64
        }
    }

    /// True when the last tab was closed.
    pub fn quit(&self) -> bool {
        self.quit
    }

    /// The window title.
    pub fn title(&self) -> String {
        match self.tabs.get(self.active) {
            Some(t) if !t.title.is_empty() => format!("{} – Vy", t.title),
            Some(t) if !t.display.is_empty() => format!("{} – Vy", t.display),
            _ => "Vy".into(),
        }
    }

    /// `https://` addresses handed to the opener so far, in order.
    pub fn external_opens(&self) -> &[String] {
        &self.external_opens
    }

    /// The shell's clipboard (SPEC-5.2.12), as the last page or address
    /// bar left it.
    pub fn clipboard(&self) -> &[u8] {
        &self.clipboard
    }

    /// The active tab's id.
    pub fn active_tab(&self) -> Option<TabId> {
        self.tabs.get(self.active).map(|t| t.id)
    }

    /// The tabs' ids in strip order.
    pub fn tab_ids(&self) -> Vec<TabId> {
        self.tabs.iter().map(|t| t.id).collect()
    }

    /// The process id of a tab's process, when it has one (SPEC-9.1.1).
    pub fn tab_pid(&self, id: TabId) -> Option<u32> {
        self.tab(id)
            .and_then(|t| t.conn.as_ref())
            .and_then(|c| c.pid())
    }

    /// Kill a tab's process outright (crash containment tests, SPEC-11.2.2).
    pub fn kill_tab_process(&mut self, id: TabId) {
        if let Some(c) = self.tab_mut(id).and_then(|t| t.conn.as_mut()) {
            c.kill();
        }
    }

    /// A tab's state, for tests and the status line.
    pub fn tab_status(&self, id: TabId) -> String {
        match self.tab(id).map(|t| &t.state) {
            None => "no such tab".into(),
            Some(State::Empty) => "empty".into(),
            Some(State::Loading) => "loading".into(),
            Some(State::Running) => "running".into(),
            Some(State::Failed(e)) => format!("failed: {}", e.title()),
            Some(State::Rejected(r)) => format!("rejected: {r}"),
            Some(State::Stopped(r)) => format!("stopped: {r}"),
            Some(State::Gone(r)) => format!("gone: {r}"),
        }
    }

    /// A tab's navigation count: the generation its current process belongs to.
    pub fn tab_generation(&self, id: TabId) -> u64 {
        self.tab(id).map_or(0, |t| t.generation)
    }

    /// Frames a tab has received since its page loaded.
    pub fn tab_frames(&self, id: TabId) -> u64 {
        self.tab(id).map_or(0, |t| t.frames)
    }

    /// True when a tab's page cannot produce anything more: it failed to
    /// load, was rejected, stopped, or its process is gone.
    pub fn tab_terminal(&self, id: TabId) -> bool {
        self.tab(id).is_some_and(|t| {
            matches!(
                t.state,
                State::Failed(_) | State::Rejected(_) | State::Stopped(_) | State::Gone(_)
            )
        })
    }

    /// A tab's capability badges: name and granted (SPEC-9.2.1).
    pub fn tab_badges(&self, id: TabId) -> Vec<(&'static str, bool)> {
        self.tab(id).map(Tab::badges).unwrap_or_default()
    }

    /// The capability the active tab is asking about, if any.
    pub fn pending_ask(&self) -> Option<&'static str> {
        self.tabs
            .get(self.active)
            .and_then(|t| t.asks.front().copied())
    }

    /// The active tab's log lines.
    pub fn tab_logs(&self, id: TabId) -> Vec<String> {
        self.tab(id)
            .map(|t| t.logs.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// The links the active tab registered (SPEC-5.2.10).
    pub fn tab_links(&self, id: TabId) -> Vec<Link> {
        self.tab(id).map(|t| t.links.clone()).unwrap_or_default()
    }

    /// A tab's semantic tree as last sent (SPEC-10.1.1).
    pub fn tab_semantic(&self, id: TabId) -> Vec<SemNode> {
        self.tab(id).map(|t| t.sem.clone()).unwrap_or_default()
    }

    /// True when a tab is hidden (not active, or under its `min_size`).
    pub fn tab_hidden(&self, id: TabId) -> bool {
        self.tab(id).is_some_and(|t| t.hidden)
    }

    /// The address field, for tests.
    pub fn address(&self) -> &AddressField {
        &self.address
    }

    /// The layout of the chrome as last drawn.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// The active tab's font scale.
    pub fn zoom(&self) -> f32 {
        self.tabs.get(self.active).map_or(1.0, |t| t.zoom)
    }

    /// True while the active tab animates (asked for another frame).
    pub fn animating(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|t| t.running() && t.wants_more && !t.hidden)
    }

    /// True when a tick is pending or due in the active tab: a driver
    /// should keep ticking.
    pub fn wants_tick(&self) -> bool {
        self.tabs
            .get(self.active)
            .is_some_and(|t| t.running() && !t.hidden && (t.tick_due || t.wants_more))
    }

    /// True when the active tab waits for the answer to a tick.
    pub fn tick_pending(&self) -> bool {
        self.tabs.get(self.active).is_some_and(|t| t.tick_pending)
    }

    /// The cursor to show at the pointer.
    pub fn cursor(&self) -> Cursor {
        if self.hover.is_some() {
            Cursor::Hand
        } else if self.layout.address.contains(self.pointer.0, self.pointer.1) {
            Cursor::Text
        } else {
            Cursor::Arrow
        }
    }

    /// Where the IME should compose, in logical window pixels: the
    /// address field, or the page's `ime_begin` rectangle.
    pub fn ime_area(&self) -> Option<Rect> {
        if self.address.focused() {
            return Some(self.layout.address);
        }
        let tab = self.tabs.get(self.active)?;
        let (x, y, w, h) = tab.ime?;
        let page = self.layout.page;
        Some(Rect::new(
            page.x + x * tab.zoom,
            page.y + y * tab.zoom,
            w * tab.zoom,
            h * tab.zoom,
        ))
    }

    fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    fn index_of(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    // ---- tabs ----------------------------------------------------------

    /// Open a tab, loading `source` in it when given, and make it active.
    pub fn open_tab(&mut self, source: Option<Source>) -> TabId {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab::new(id));
        let index = self.tabs.len() - 1;
        self.activate(index);
        if let Some(s) = source {
            self.navigate_tab(id, s, true);
        }
        id
    }

    /// Open a tab with a page that is already in hand (a package another
    /// part of the client fetched and judged), skipping the loader.
    pub fn open_tab_loaded(&mut self, page: LoadedPage) -> TabId {
        let id = self.open_tab(None);
        if let Some(tab) = find_tab(&mut self.tabs, id) {
            tab.generation += 1;
            tab.history.push(&page.display);
            tab.source = Some(Source::File {
                path: PathBuf::from(&page.display),
                draft: None,
            });
            tab.state = State::Loading;
        }
        self.start_page(id, page);
        id
    }

    /// Close a tab; closing the last one quits.
    pub fn close_tab(&mut self, id: TabId) {
        let Some(index) = self.index_of(id) else {
            return;
        };
        let mut tab = self.tabs.remove(index);
        if let Some(conn) = tab.conn.take() {
            conn.shutdown();
        }
        if self.tabs.is_empty() {
            self.quit = true;
            self.frame_dirty = true;
            return;
        }
        let next = index.min(self.tabs.len() - 1);
        self.active = usize::MAX;
        self.activate(next);
    }

    /// Make the tab at `index` active: the old one is hidden, the new one
    /// shown (SPEC-6.2.1).
    fn activate(&mut self, index: usize) {
        if index >= self.tabs.len() || index == self.active {
            return;
        }
        let now = self.now_ms();
        if let Some(old) = self.tabs.get_mut(self.active) {
            if old.running() && !old.hidden {
                old.hidden = true;
                old.send(&ToTab::Visibility {
                    visible: false,
                    clock_ms: now,
                });
            }
        }
        self.active = index;
        let display = self.tabs[index].display.clone();
        self.address.set_committed(&display);
        self.address.blur();
        self.hover = None;
        self.relayout();
        self.show_active();
        self.frame_dirty = true;
    }

    /// Send the active tab its size if it changed and show it unless the
    /// window is under its `min_size` (SPEC-8.3.1).
    fn show_active(&mut self) {
        let page = self.layout.page;
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if !tab.running() {
            return;
        }
        let want_size = page_size(page, tab.zoom, self.scale);
        if tab.sent_size != want_size {
            tab.sent_size = want_size;
            tab.rendered = None;
            tab.send(&ToTab::Resize {
                width: want_size.0,
                height: want_size.1,
                scale: want_size.2,
            });
        }
        let under = tab
            .manifest
            .as_ref()
            .is_some_and(|m| m.needs_fallback(false, want_size.0, want_size.1));
        tab.under_min = under;
        let now = if self.config.headless {
            self.clock_headless
        } else {
            self.started.elapsed().as_millis() as i64
        };
        if under {
            if !tab.hidden {
                tab.hidden = true;
                tab.send(&ToTab::Visibility {
                    visible: false,
                    clock_ms: now,
                });
            }
        } else if tab.hidden {
            tab.hidden = false;
            tab.send(&ToTab::Visibility {
                visible: true,
                clock_ms: now,
            });
            tab.tick_due = true;
        }
    }

    /// Ctrl+Tab / Ctrl+Shift+Tab.
    pub fn switch_tab(&mut self, forward: bool) {
        if self.tabs.len() < 2 {
            return;
        }
        let n = self.tabs.len();
        let next = if forward {
            (self.active + 1) % n
        } else {
            (self.active + n - 1) % n
        };
        self.activate(next);
    }

    // ---- navigation ----------------------------------------------------

    /// What the user typed in the address bar: an `https://` address
    /// opens externally (SPEC-2.1.7), a `vy://` address or a file path
    /// loads in the active tab, anything else is shown as an error.
    pub fn navigate(&mut self, text: &str) {
        let text = text.trim();
        if is_external(text) {
            self.open_external(text);
            return;
        }
        let Some(id) = self.active_tab() else {
            return;
        };
        match Source::parse(text) {
            Ok(source) => self.navigate_tab(id, source, true),
            Err(e) => {
                self.fail(id, LoadError::Fetch(e));
            }
        }
    }

    /// Load `source` in tab `id`, pushing or replacing its history entry.
    pub fn navigate_tab(&mut self, id: TabId, source: Source, push: bool) {
        let generation;
        {
            let Some(tab) = self.tab_mut(id) else {
                return;
            };
            if let Some(conn) = tab.conn.take() {
                conn.shutdown();
            }
            tab.generation += 1;
            generation = tab.generation;
            let display = source.display();
            if push {
                tab.history.push(&display);
            } else {
                tab.history.replace(&display);
            }
            tab.display = display;
            tab.source = Some(source.clone());
            tab.origin = source.origin();
            tab.manifest = None;
            tab.title = String::new();
            tab.state = State::Loading;
            tab.trusted = false;
            tab.caps.clear();
            tab.asks.clear();
            tab.net_origins.clear();
            tab.storage_quota_bytes = 0;
            tab.text = TextEngine::new();
            tab.images = ImageCache::empty(0);
            tab.display_list.clear();
            tab.rendered = None;
            tab.links.clear();
            tab.sem.clear();
            tab.has_tree = None;
            tab.loaded = false;
            tab.tick_pending = false;
            tab.wants_more = false;
            tab.tick_due = false;
            tab.frames = 0;
            tab.sent_size = (0, 0, 0.0);
            tab.ime = None;
            tab.hidden = false;
            tab.under_min = false;
            tab.pressed_link = None;
        }
        if Some(id) == self.active_tab() {
            let display = self.tab(id).map(|t| t.display.clone()).unwrap_or_default();
            self.address.set_committed(&display);
            self.hover = None;
        }
        let _ = self.loader.send(Job::Page {
            tab: id,
            generation,
            source,
        });
        self.frame_dirty = true;
    }

    /// Alt+Left.
    pub fn back(&mut self) {
        let Some(id) = self.active_tab() else {
            return;
        };
        let target = self
            .tab_mut(id)
            .and_then(|t| t.history.back().map(str::to_string));
        if let Some(address) = target {
            if let Ok(source) = Source::parse(&address) {
                self.navigate_tab(id, source, false);
            }
        }
    }

    /// Alt+Right.
    pub fn forward(&mut self) {
        let Some(id) = self.active_tab() else {
            return;
        };
        let target = self
            .tab_mut(id)
            .and_then(|t| t.history.forward().map(str::to_string));
        if let Some(address) = target {
            if let Ok(source) = Source::parse(&address) {
                self.navigate_tab(id, source, false);
            }
        }
    }

    /// F5.
    pub fn reload(&mut self) {
        let Some(id) = self.active_tab() else {
            return;
        };
        if let Some(source) = self.tab(id).and_then(|t| t.source.clone()) {
            self.navigate_tab(id, source, false);
        }
    }

    fn open_external(&mut self, url: &str) {
        match self.config.opener.open(url) {
            Ok(()) => self.external_opens.push(url.to_string()),
            Err(e) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.log(format!("[host] could not open {url}: {e}"));
                }
            }
        }
    }

    fn fail(&mut self, id: TabId, error: LoadError) {
        if let Some(tab) = self.tab_mut(id) {
            tab.state = State::Failed(error);
            tab.loaded = false;
        }
        self.frame_dirty = true;
    }

    // ---- inbox ---------------------------------------------------------

    /// Handle one message from the inbox.
    pub fn handle(&mut self, incoming: Incoming) {
        if let Incoming::Tab(id, generation, _) = &incoming {
            if self.tab(*id).is_none_or(|t| t.generation != *generation) {
                return;
            }
        }
        match incoming {
            Incoming::Tab(id, _, Inbound::Message(m)) => self.tab_message(id, m),
            Incoming::Tab(id, _, Inbound::Gone { error }) => {
                let active = Some(id) == self.active_tab();
                if let Some(tab) = self.tab_mut(id) {
                    let status = tab
                        .conn
                        .as_mut()
                        .and_then(|c| c.exit_status(std::time::Duration::from_millis(200)));
                    if !matches!(
                        tab.state,
                        State::Loading | State::Running | State::Stopped(_)
                    ) {
                        return;
                    }
                    let reason = match (error, status) {
                        (Some(e), _) => format!("Flikprocessen försvann: {e}"),
                        (None, Some(s)) => format!("Flikprocessen avslutades ({s})."),
                        (None, None) => "Flikprocessen avslutades.".into(),
                    };
                    tab.conn = None;
                    tab.loaded = false;
                    tab.tick_pending = false;
                    tab.wants_more = false;
                    tab.state = State::Gone(reason);
                }
                if active {
                    self.hover = None;
                }
                self.frame_dirty = true;
            }
            Incoming::Load(Done::Page {
                tab,
                generation,
                result,
            }) => {
                if self.tab(tab).is_some_and(|t| t.generation == generation) {
                    match *result {
                        Ok(page) => self.start_page(tab, page),
                        Err(e) => self.fail(tab, e),
                    }
                }
            }
            Incoming::Load(Done::Request {
                tab,
                generation,
                id,
                status,
                body,
            }) => {
                if let Some(t) = find_tab(&mut self.tabs, tab) {
                    if t.generation == generation && t.running() {
                        // The response counts against the same window as
                        // the request (SPEC-9.3.1: 256 MB/min in all).
                        t.net_window.1 = t.net_window.1.saturating_add(body.len() as u64);
                        t.send(&ToTab::FetchResponse { id, status, body });
                        t.tick_due = true;
                    }
                }
            }
        }
    }

    /// A loaded page: decide its grants, start its process and load it.
    fn start_page(&mut self, id: TabId, page: LoadedPage) {
        let resolved = match page.origin.as_deref() {
            Some(o) if page.trusted => self.grants.resolve(o, &page.verdicts),
            _ => page
                .verdicts
                .iter()
                .map(|(name, _)| Resolved {
                    name,
                    granted: false,
                    ask: false,
                })
                .collect(),
        };
        let stored = match &page.origin {
            Some(o) if !self.config.headless => {
                storage::load(&self.config.data_dir.join("storage"), o)
            }
            _ => Vec::new(),
        };
        let page_rect = self.layout.page;
        let scale = self.scale;
        let sink = self.inbox_tx.clone();
        let waker = self.config.waker.clone();
        let exe = self.config.exe.clone();
        let (clock_ms, random) = if self.config.headless {
            (self.clock_headless, RandomSource::default())
        } else {
            let mut keybytes = [0u8; 32];
            if getrandom::getrandom(&mut keybytes).is_err() {
                // Without OS entropy there is no CSPRNG; the page runs
                // with a clock-derived key rather than not at all.
                let t = self.started.elapsed().as_nanos() as u64;
                keybytes[..8].copy_from_slice(&t.to_le_bytes());
            }
            (
                self.started.elapsed().as_millis() as i64,
                RandomSource::ChaCha20(keybytes),
            )
        };
        let clipboard = self.clipboard.clone();
        let (net_origins, storage_quota_bytes, memory_max_pages) = match &page.manifest {
            Some(m) => (
                m.capabilities
                    .net
                    .as_ref()
                    .map(|n| n.origins.clone())
                    .unwrap_or_default(),
                m.capabilities
                    .storage
                    .as_ref()
                    .map(|s| u64::from(s.quota_kb) * 1024)
                    .unwrap_or(vy_runtime::DEFAULT_STORAGE_QUOTA_BYTES),
                m.memory_max,
            ),
            None => (
                Vec::new(),
                vy_runtime::DEFAULT_STORAGE_QUOTA_BYTES,
                vy_runtime::DEFAULT_MEMORY_MAX_PAGES,
            ),
        };
        let generation = self.tab(id).map_or(0, |t| t.generation);
        let forward = move |tab_id: TabId, inbound: Inbound| {
            let ok = sink
                .send(Incoming::Tab(tab_id, generation, inbound))
                .is_ok();
            waker.wake();
            ok
        };
        let conn = match &exe {
            Some(exe) => process::spawn_process(exe, id, forward, None, None),
            None => process::spawn_thread(id, forward),
        };
        let active = Some(id) == self.active_tab();
        let Some(tab) = find_tab(&mut self.tabs, id) else {
            return;
        };
        tab.title = page
            .manifest
            .as_ref()
            .map(|m| m.name.clone())
            .unwrap_or_else(|| page.display.clone());
        tab.trusted = page.trusted;
        tab.is_localhost = page.is_localhost;
        tab.net_origins = net_origins
            .iter()
            .filter_map(|o| vy_runtime::origin_of(o))
            .collect();
        tab.storage_quota_bytes = storage_quota_bytes;
        tab.asks = resolved.iter().filter(|r| r.ask).map(|r| r.name).collect();
        tab.caps = resolved;
        tab.images = ImageCache::empty(page.resources.len());
        tab.manifest = page.manifest;
        tab.origin = page.origin;
        tab.display = page.display;
        let want_size = page_size(page_rect, tab.zoom, scale);
        tab.sent_size = want_size;
        match conn {
            Ok(conn) => {
                let request = LoadRequest {
                    wasm: page.wasm,
                    sha256: page.sha256,
                    memory_max_pages,
                    capabilities: pack_capabilities(tab.capabilities()),
                    resources: page.resources,
                    storage: stored,
                    storage_quota_bytes,
                    net_origins,
                    random,
                    width: want_size.0,
                    height: want_size.1,
                    scale: want_size.2,
                    clock_ms,
                    clipboard: Some(clipboard),
                };
                conn.send(&ToTab::Load(Box::new(request)));
                tab.conn = Some(conn);
                tab.state = State::Loading;
            }
            Err(e) => {
                tab.state = State::Gone(format!("Flikprocessen kunde inte startas: {e}"));
            }
        }
        if active {
            let display = tab.display.clone();
            self.address.set_committed(&display);
        }
        self.frame_dirty = true;
    }

    fn tab_message(&mut self, id: TabId, m: ToShell) {
        let active = Some(id) == self.active_tab();
        match m {
            ToShell::Loaded { ok, error, .. } => {
                if let Some(tab) = self.tab_mut(id) {
                    if ok {
                        tab.loaded = true;
                        tab.state = State::Running;
                        tab.tick_due = true;
                    } else {
                        // A rejected module shows the fallback when the
                        // manifest has one (SPEC-8.3.1), else the reason.
                        tab.state = State::Rejected(error);
                        tab.loaded = false;
                    }
                }
                if ok && active {
                    self.show_active();
                }
                self.frame_dirty = true;
            }
            ToShell::Frame {
                display_list,
                styles,
                wants_more,
                over_budget,
            } => {
                if let Some(tab) = self.tab_mut(id) {
                    for s in styles {
                        let _ = tab.text.style(s.font, s.size_px, s.weight, s.color);
                    }
                    tab.display_list = display_list;
                    tab.rendered = None;
                    tab.wants_more = wants_more;
                    tab.tick_pending = false;
                    tab.frames += 1;
                    if over_budget {
                        tab.log("[host] frame over budget (SPEC-5.3.3)".into());
                    }
                }
                if active {
                    self.frame_dirty = true;
                }
            }
            ToShell::Idle => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.tick_pending = false;
                    tab.wants_more = false;
                }
            }
            ToShell::Image { id: rid, image } => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.images.set_decoded(rid, image);
                    tab.rendered = None;
                }
            }
            ToShell::Links(links) => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.links = links;
                }
                if active {
                    self.update_hover();
                }
            }
            ToShell::Semantic(nodes) => {
                let had = self.tab(id).and_then(|t| t.has_tree);
                let has = !nodes.is_empty();
                if let Some(tab) = self.tab_mut(id) {
                    tab.has_tree = Some(has);
                    tab.sem = nodes;
                }
                if had != Some(has) && active {
                    self.frame_dirty = true;
                }
            }
            ToShell::Log {
                module,
                level,
                message,
            } => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.log(format!(
                        "[{} {}] {message}",
                        if module { "module" } else { "host" },
                        vy_abi::level::name(level)
                    ));
                }
            }
            ToShell::Navigate { url, replace } => {
                if is_external(&url) {
                    self.open_external(&url);
                } else if let Ok(source) = Source::parse(&url) {
                    self.navigate_tab(id, source, !replace);
                } else if let Some(tab) = self.tab_mut(id) {
                    tab.log(format!("[host] navigate refused: {url}"));
                }
            }
            ToShell::FetchRequest {
                id: req,
                method,
                url,
                body,
            } => self.fetch_for(id, req, method, url, body),
            ToShell::Storage(entries) => {
                let Some(tab) = self.tab_mut(id) else {
                    return;
                };
                // Only a page granted `storage` has anything to save, and
                // only within its quota (SPEC-5.2.9, SPEC-9.3.1); the tab
                // process is not trusted to have kept either.
                let size: u64 = entries
                    .iter()
                    .map(|(k, v)| (k.len() + v.len()) as u64)
                    .sum();
                if !entries.is_empty()
                    && (!tab.capabilities().has(cap::STORAGE) || size > tab.storage_quota_bytes)
                {
                    tab.log(format!(
                        "[host] storage of {size} bytes not saved: not granted or over the quota"
                    ));
                    return;
                }
                let (origin, headless) = (tab.origin.clone(), self.config.headless);
                if let (Some(origin), false) = (origin, headless) {
                    if !origin.starts_with("file:") {
                        let dir = self.config.data_dir.join("storage");
                        if let Err(e) = storage::save(&dir, &origin, &entries) {
                            if let Some(tab) = self.tab_mut(id) {
                                tab.log(format!("[host] storage not saved: {e}"));
                            }
                        }
                    }
                }
            }
            ToShell::Clipboard(bytes) => {
                // The clipboard is written through `clipboard_write`, which
                // needs the capability (SPEC-5.2.12, SPEC-5.3.1): a tab
                // without it has nothing to say to the other tabs.
                let Some(tab) = self.tab_mut(id) else {
                    return;
                };
                if !tab.capabilities().has(cap::CLIPBOARD) {
                    tab.log("[host] clipboard ignored: `clipboard` is not granted".into());
                    return;
                }
                self.clipboard = bytes.clone();
                for tab in &self.tabs {
                    if tab.id != id && tab.running() {
                        tab.send(&ToTab::Clipboard {
                            bytes: bytes.clone(),
                        });
                    }
                }
            }
            ToShell::Ime(rect) => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.ime = rect;
                }
            }
            ToShell::Stopped { reason } => {
                if let Some(tab) = self.tab_mut(id) {
                    tab.state = State::Stopped(reason);
                    tab.loaded = false;
                    tab.tick_pending = false;
                    tab.wants_more = false;
                }
                if active {
                    self.hover = None;
                }
                self.frame_dirty = true;
            }
        }
    }

    /// A page's own request (SPEC-5.2.7): with `net` granted, to an origin
    /// the manifest named (SPEC-5.2.8) and within the per-minute budget
    /// (SPEC-9.3.1) it goes to the loader; otherwise the request is
    /// answered with the code and never reaches the network.
    fn fetch_for(&mut self, id: TabId, req: i32, method: i32, url: String, body: Vec<u8>) {
        let now = self.now_ms();
        let Some(tab) = find_tab(&mut self.tabs, id) else {
            return;
        };
        let refuse = |tab: &mut Tab, code: i32| {
            tab.log(format!(
                "[host] fetch {req}: {} for {url} (decided by the main process)",
                vy_abi::err::name(code).unwrap_or("E_UNKNOWN")
            ));
            tab.send(&ToTab::FetchResponse {
                id: req,
                status: code,
                body: Vec::new(),
            });
            tab.tick_due = true;
        };
        if let Err(code) = tab.request_verdict(&url) {
            refuse(tab, code);
            return;
        }
        if now - tab.net_window.0 >= 60_000 {
            tab.net_window = (now, 0);
        }
        tab.net_window.1 += body.len() as u64;
        if tab.net_window.1 > NET_BYTES_PER_MINUTE {
            refuse(tab, vy_abi::err::E_RATE);
            return;
        }
        let Some(method) = vy_abi::method::name(method) else {
            refuse(tab, vy_abi::err::E_ARG);
            return;
        };
        let _ = self.loader.send(Job::Request {
            tab: id,
            generation: tab.generation,
            id: req,
            method,
            url,
            body,
            page_is_localhost: tab.is_localhost,
        });
    }

    // ---- capabilities (SPEC-9.2.1) ---------------------------------------

    /// Answer the capability question of the active tab.
    pub fn answer_ask(&mut self, allow: bool) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(name) = tab.asks.pop_front() else {
            return;
        };
        let origin = tab.origin.clone();
        let id = tab.id;
        if let Some(o) = &origin {
            self.grants.set(o, name, allow);
        }
        self.set_capability(id, name, allow);
        self.frame_dirty = true;
    }

    /// Grant or revoke `name` for tab `id` now and for the origin from
    /// now on (SPEC-9.2.1); a click on a badge.
    pub fn toggle_capability(&mut self, id: TabId, name: &'static str) {
        let Some(tab) = self.tab(id) else {
            return;
        };
        let Some(r) = tab.caps.iter().find(|r| r.name == name) else {
            return;
        };
        let allow = !r.granted;
        if let Some(o) = tab.origin.clone() {
            self.grants.set(&o, name, allow);
        }
        self.set_capability(id, name, allow);
        self.frame_dirty = true;
    }

    fn set_capability(&mut self, id: TabId, name: &'static str, allow: bool) {
        let Some(tab) = self.tab_mut(id) else {
            return;
        };
        if let Some(r) = tab.caps.iter_mut().find(|r| r.name == name) {
            r.granted = allow;
            r.ask = false;
        }
        if let Some(cap_id) = cap::id(name) {
            if tab.running() {
                tab.send(&ToTab::Grant {
                    cap: cap_id,
                    granted: allow,
                });
                tab.tick_due = true;
            }
        }
    }

    // ---- window ----------------------------------------------------------

    /// The window changed size or DPI.
    pub fn resize(&mut self, width: f32, height: f32, scale: f32) {
        self.width = width.max(1.0);
        self.height = height.max(1.0);
        if scale.is_finite() && scale > 0.0 {
            self.scale = scale;
        }
        self.relayout();
        self.show_active();
        self.frame_dirty = true;
    }

    /// Ctrl+Plus / Ctrl+Minus / Ctrl+0: the page's font scale, which is
    /// the `scale` the module sees (SPEC-6.3.1).
    pub fn zoom_step(&mut self, step: i32) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let at = ZOOM_STEPS
            .iter()
            .position(|z| (*z - tab.zoom).abs() < 1e-3)
            .unwrap_or(4) as i32;
        let next = if step == 0 {
            4
        } else {
            (at + step).clamp(0, ZOOM_STEPS.len() as i32 - 1)
        };
        tab.zoom = ZOOM_STEPS[next as usize];
        tab.rendered = None;
        self.relayout();
        self.show_active();
        self.frame_dirty = true;
    }

    /// Recompute the chrome layout for the current state.
    fn relayout(&mut self) {
        let (dl, layout) = self.draw_chrome();
        let _ = dl;
        self.layout = layout;
    }

    fn draw_chrome(&mut self) -> (Vec<u8>, Layout) {
        let tabs: Vec<TabInfo> = self
            .tabs
            .iter()
            .map(|t| TabInfo {
                id: t.id,
                title: if t.title.is_empty() {
                    t.display.clone()
                } else {
                    t.title.clone()
                },
                loading: t.state == State::Loading,
            })
            .collect();
        let tab = self.tabs.get(self.active);
        let blocks: Vec<Block>;
        let detail: String;
        let page_view = match tab {
            None => PageView::Empty,
            Some(t) => match &t.state {
                State::Empty => PageView::Empty,
                State::Loading => PageView::Loading,
                State::Running => {
                    if t.under_min {
                        blocks = markdown::parse(
                            t.manifest
                                .as_ref()
                                .and_then(|m| m.fallback.as_deref())
                                .unwrap_or(""),
                        );
                        PageView::Fallback(&blocks)
                    } else {
                        PageView::Page
                    }
                }
                State::Failed(e) => {
                    detail = e.detail();
                    PageView::Error {
                        title: e.title(),
                        detail: &detail,
                        external: e.external_url(),
                    }
                }
                State::Rejected(reason) => {
                    if let Some(fb) = t.manifest.as_ref().and_then(|m| m.fallback.as_deref()) {
                        blocks = markdown::parse(fb);
                        PageView::Fallback(&blocks)
                    } else {
                        detail = reason.clone();
                        PageView::Error {
                            title: "Modulen avvisades",
                            detail: &detail,
                            external: None,
                        }
                    }
                }
                State::Stopped(reason) | State::Gone(reason) => PageView::Stopped(reason),
            },
        };
        let badges = tab.map(Tab::badges).unwrap_or_default();
        let view = View {
            width: self.width,
            height: self.height,
            tabs: &tabs,
            active: self.active.min(tabs.len().saturating_sub(1)),
            address: &self.address,
            can_back: tab.is_some_and(|t| t.history.can_back()),
            can_forward: tab.is_some_and(|t| t.history.can_forward()),
            restricted: tab
                .is_some_and(|t| !t.trusted && matches!(t.state, State::Running | State::Loading)),
            inaccessible: tab
                .is_some_and(|t| t.state == State::Running && t.has_tree == Some(false)),
            loading: tab.is_some_and(|t| t.state == State::Loading),
            badges: &badges,
            ask: tab.and_then(|t| t.asks.front().copied()),
            page: page_view,
            zoom: tab.map_or(1.0, |t| t.zoom),
        };
        chrome::draw(&view, &mut self.chrome_text, &self.styles)
    }

    /// True when the composited frame changed since it was last taken.
    pub fn needs_redraw(&self) -> bool {
        self.frame_dirty
    }

    /// The composited frame: chrome, the active page in its area, and the
    /// status pill. Recomposited only when something changed.
    pub fn frame(&mut self) -> &RgbaImage {
        if self.frame_dirty {
            self.compose();
            self.frame_dirty = false;
        }
        &self.frame
    }

    fn compose(&mut self) {
        let (chrome_dl, layout) = self.draw_chrome();
        self.layout = layout;
        let (pw, ph) = physical(self.width, self.height, self.scale);
        let none = ImageCache::empty(0);
        // The chrome is rasterized again only when its list changed: a
        // page frame alone reuses the cached pixels.
        let cached = match &self.chrome_cache {
            Some((dl, w, h, img)) if *dl == chrome_dl && (*w, *h) == (pw, ph) => Some(img.clone()),
            _ => None,
        };
        self.frame = match cached {
            Some(img) => img,
            None => {
                let img = match vy_render::render(
                    &chrome_dl,
                    pw,
                    ph,
                    self.scale,
                    &mut self.chrome_text,
                    &none,
                ) {
                    Ok(r) => r.image,
                    Err(_) => RgbaImage::new(pw, ph),
                };
                self.chrome_cache = Some((chrome_dl, pw, ph, img.clone()));
                img
            }
        };
        let page = self.layout.page;
        let scale = self.scale;
        let mut raster_due = None;
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if tab.state == State::Running && !tab.under_min && !tab.display_list.is_empty() {
                let (lw, lh, s) = page_size(page, tab.zoom, scale);
                let (ppw, pph) = physical(lw as f32, lh as f32, s);
                let fresh =
                    !matches!(&tab.rendered, Some((w, h, rs, _)) if (*w, *h, *rs) == (ppw, pph, s));
                if fresh {
                    match vy_render::render(
                        &tab.display_list,
                        ppw,
                        pph,
                        s,
                        &mut tab.text,
                        &tab.images,
                    ) {
                        Ok(r) => {
                            if r.report.truncated {
                                raster_due = Some(r.report);
                            }
                            tab.rendered = Some((ppw, pph, s, r.image));
                        }
                        Err(e) => tab.log(format!("[host] display list not rendered: {e}")),
                    }
                }
                if let Some((_, _, _, img)) = &tab.rendered {
                    let (px, py) = (
                        (page.x * scale).round() as i64,
                        (page.y * scale).round() as i64,
                    );
                    let clip = (
                        px,
                        py,
                        (page.w * scale).round() as i64,
                        (page.h * scale).round() as i64,
                    );
                    blit(&mut self.frame, img, px, py, Some(clip));
                }
            }
            if let Some(report) = raster_due {
                tab.send(&ToTab::Raster {
                    truncated: report.truncated,
                    ops_done: report.ops_done as u32,
                    units: report.units,
                    budget: report.budget,
                });
            }
        }
        if let Some(url) = self.hover.clone() {
            let (dl, w, h) = chrome::draw_status(&mut self.chrome_text, &self.styles, &url, page.w);
            let (sw, sh) = physical(w, h, scale);
            if let Ok(r) = vy_render::render(&dl, sw, sh, scale, &mut self.chrome_text, &none) {
                let x = (page.x * scale).round() as i64;
                let y = ((page.bottom() - h) * scale).round() as i64;
                blend(&mut self.frame, &r.image, x, y);
            }
        }
    }

    // ---- ticks -----------------------------------------------------------

    /// Send a tick to the active tab when one is due and none is pending
    /// (SPEC-6.2.1); the headless clock advances by a frame.
    pub fn tick(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        if !tab.running() || tab.hidden || tab.tick_pending || !(tab.tick_due || tab.wants_more) {
            return;
        }
        if self.config.headless {
            self.clock_headless += FRAME_MS;
        }
        let now = if self.config.headless {
            self.clock_headless
        } else {
            self.started.elapsed().as_millis() as i64
        };
        let tab = &mut self.tabs[self.active];
        tab.tick_due = false;
        tab.tick_pending = true;
        tab.send(&ToTab::Tick { clock_ms: now });
    }

    // ---- input -----------------------------------------------------------

    fn page_point(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let page = self.layout.page;
        let tab = self.tabs.get(self.active)?;
        if !page.contains(x, y) || !tab.running() || tab.under_min {
            return None;
        }
        Some(((x - page.x) / tab.zoom, (y - page.y) / tab.zoom))
    }

    fn send_page_event(&mut self, kind: i32, a: i32, b: i32, c: i32, d: i32) {
        let now = self.now_ms();
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if tab.running() {
                tab.send(&ToTab::Event {
                    kind,
                    a,
                    b,
                    c,
                    d,
                    clock_ms: now,
                });
                tab.tick_due = true;
            }
        }
    }

    fn update_hover(&mut self) {
        let was = self.hover.clone();
        self.hover = self
            .page_point(self.pointer.0, self.pointer.1)
            .and_then(|(px, py)| {
                self.tabs
                    .get(self.active)
                    .and_then(|t| t.link_at(px, py))
                    .map(|l| l.url.clone())
            });
        if was != self.hover {
            self.frame_dirty = true;
        }
    }

    /// The pointer moved to `(x, y)` logical window pixels.
    pub fn pointer_move(&mut self, x: f32, y: f32) {
        self.pointer = (x, y);
        if let Some((px, py)) = self.page_point(x, y) {
            self.in_page = true;
            self.send_page_event(
                event::POINTER_MOVE,
                x100(px),
                x100(py),
                self.buttons,
                self.modifiers,
            );
        } else if self.in_page {
            self.in_page = false;
            self.send_page_event(event::POINTER_LEAVE, 0, 0, 0, 0);
        }
        self.update_hover();
    }

    /// The pointer left the window.
    pub fn pointer_leave(&mut self) {
        self.pointer = (-1.0, -1.0);
        if self.in_page {
            self.in_page = false;
            self.send_page_event(event::POINTER_LEAVE, 0, 0, 0, 0);
        }
        self.update_hover();
    }

    /// Modifier keys changed (`vy_abi::modifier` bits).
    pub fn set_modifiers(&mut self, modifiers: i32) {
        self.modifiers = modifiers;
    }

    /// A pointer button (`vy_abi::button`) went down or up at the last
    /// pointer position.
    pub fn pointer_button(&mut self, button: i32, down: bool) {
        let mask = vy_abi::button::mask(button);
        if down {
            self.buttons |= mask;
        } else {
            self.buttons &= !mask;
        }
        let (x, y) = self.pointer;
        if let Some((px, py)) = self.page_point(x, y) {
            if down && self.address.focused() {
                self.address.blur();
                self.frame_dirty = true;
            }
            let link = self
                .tabs
                .get(self.active)
                .and_then(|t| t.link_at(px, py))
                .map(|l| l.url.clone());
            let kind = if down {
                event::POINTER_DOWN
            } else {
                event::POINTER_UP
            };
            self.send_page_event(kind, x100(px), x100(py), button, self.modifiers);
            if button == vy_abi::button::PRIMARY {
                if down {
                    if let Some(tab) = self.tabs.get_mut(self.active) {
                        tab.pressed_link = link;
                    }
                } else {
                    let pressed = self
                        .tabs
                        .get_mut(self.active)
                        .and_then(|t| t.pressed_link.take());
                    if let (Some(p), Some(l)) = (pressed, link) {
                        if p == l {
                            self.follow_link(&l);
                        }
                    }
                }
            } else if button == vy_abi::button::BACK && down {
                self.back();
            } else if button == vy_abi::button::FORWARD && down {
                self.forward();
            }
            return;
        }
        if !down || button != vy_abi::button::PRIMARY {
            return;
        }
        self.click_chrome(x, y);
    }

    /// A link the page registered was clicked (SPEC-5.2.10, SPEC-2.1.7).
    fn follow_link(&mut self, url: &str) {
        if is_external(url) {
            self.open_external(url);
        } else if let Ok(source) = Source::parse(url) {
            if let Some(id) = self.active_tab() {
                self.navigate_tab(id, source, true);
            }
        }
    }

    fn click_chrome(&mut self, x: f32, y: f32) {
        let layout = self.layout.clone();
        let hit_tab = layout
            .tabs
            .iter()
            .find(|(_, r, _)| r.contains(x, y))
            .copied();
        if let Some((id, _, close)) = hit_tab {
            if close.contains(x, y) {
                self.close_tab(id);
            } else if let Some(i) = self.index_of(id) {
                self.address.blur();
                self.activate(i);
            }
            self.frame_dirty = true;
            return;
        }
        if layout.new_tab.contains(x, y) {
            self.open_tab(None);
            self.address.focus_all();
            self.frame_dirty = true;
            return;
        }
        if layout.back.contains(x, y) {
            self.back();
            return;
        }
        if layout.forward.contains(x, y) {
            self.forward();
            return;
        }
        if layout.reload.contains(x, y) {
            self.reload();
            return;
        }
        if let Some((name, _)) = layout.badges.iter().find(|(_, r)| r.contains(x, y)) {
            if let Some(id) = self.active_tab() {
                self.toggle_capability(id, name);
            }
            return;
        }
        if layout.allow.is_some_and(|r| r.contains(x, y)) {
            self.answer_ask(true);
            return;
        }
        if layout.deny.is_some_and(|r| r.contains(x, y)) {
            self.answer_ask(false);
            return;
        }
        if let Some(r) = layout.external {
            if r.contains(x, y) {
                let url = self.tabs.get(self.active).and_then(|t| match &t.state {
                    State::Failed(e) => e.external_url().map(str::to_string),
                    _ => None,
                });
                if let Some(u) = url {
                    self.open_external(&u);
                }
                return;
            }
        }
        if layout.address.contains(x, y) {
            let offset = self.address_offset_at(x);
            if self.address.focused() {
                self.address.focus_at(offset);
            } else {
                self.address.focus_all();
            }
            self.frame_dirty = true;
            return;
        }
        if self.address.focused() {
            self.address.blur();
            self.frame_dirty = true;
        }
    }

    /// The byte offset in the shown address nearest to window x.
    fn address_offset_at(&mut self, x: f32) -> usize {
        let text = self.address.text().to_string();
        let rel = x - self.layout.address_text_x;
        let style = self.styles.address();
        let mut best = (0usize, f32::MAX);
        for (i, _) in text
            .char_indices()
            .chain(std::iter::once((text.len(), ' ')))
        {
            let cx = self.chrome_text.caret(style, &text, i).map_or(0.0, |c| c.0);
            let d = (cx - rel).abs();
            if d < best.1 {
                best = (i, d);
            }
        }
        best.0
    }

    /// Scrolling over the window.
    pub fn scroll(&mut self, dx: f32, dy: f32, lines: bool) {
        let (x, y) = self.pointer;
        if self.page_point(x, y).is_some() {
            self.send_page_event(
                event::SCROLL,
                x100(dx),
                x100(dy),
                i32::from(lines),
                self.modifiers,
            );
        }
    }

    /// A key (USB HID `code`) went down or up. Returns true when the shell
    /// consumed it; otherwise it went to the page.
    pub fn key(&mut self, code: i32, down: bool, modifiers: i32, repeat: bool) -> bool {
        self.modifiers = modifiers;
        let ctrl = modifiers & modifier::CTRL != 0 || modifiers & modifier::META != 0;
        let shift = modifiers & modifier::SHIFT != 0;
        let alt = modifiers & modifier::ALT != 0;
        if down {
            match (code, ctrl, alt) {
                (hid::L, true, _) => {
                    self.address.focus_all();
                    self.frame_dirty = true;
                    return true;
                }
                (hid::T, true, _) => {
                    self.open_tab(None);
                    self.address.focus_all();
                    self.frame_dirty = true;
                    return true;
                }
                (hid::W, true, _) => {
                    if let Some(id) = self.active_tab() {
                        self.close_tab(id);
                    }
                    return true;
                }
                (key::TAB, true, _) => {
                    self.switch_tab(!shift);
                    return true;
                }
                (hid::F5, _, _) => {
                    self.reload();
                    return true;
                }
                (hid::LEFT, _, true) => {
                    self.back();
                    return true;
                }
                (hid::RIGHT, _, true) => {
                    self.forward();
                    return true;
                }
                (hid::EQUAL | hid::KP_PLUS, true, _) => {
                    self.zoom_step(1);
                    return true;
                }
                (hid::MINUS | hid::KP_MINUS, true, _) => {
                    self.zoom_step(-1);
                    return true;
                }
                (hid::DIGIT0 | hid::KP_0, true, _) => {
                    self.zoom_step(0);
                    return true;
                }
                _ => {}
            }
        }
        if self.address.focused() {
            if down {
                self.address_key(code, ctrl, shift);
            }
            return true;
        }
        let kind = if down { event::KEY_DOWN } else { event::KEY_UP };
        self.send_page_event(kind, code, modifiers, i32::from(repeat && down), 0);
        false
    }

    fn address_key(&mut self, code: i32, ctrl: bool, shift: bool) {
        match code {
            key::ENTER => {
                if let Some(text) = self.address.commit() {
                    self.navigate(&text);
                }
            }
            key::ESCAPE => self.address.cancel(),
            hid::LEFT => self.address.edit(Edit::Left {
                select: shift,
                word: ctrl,
            }),
            hid::RIGHT => self.address.edit(Edit::Right {
                select: shift,
                word: ctrl,
            }),
            hid::HOME => self.address.edit(Edit::Home { select: shift }),
            hid::END => self.address.edit(Edit::End { select: shift }),
            hid::BACKSPACE => self.address.edit(Edit::Backspace { word: ctrl }),
            hid::DELETE => self.address.edit(Edit::Delete { word: ctrl }),
            hid::A if ctrl => self.address.edit(Edit::SelectAll),
            hid::C if ctrl => self.clipboard = self.address.selected_text().as_bytes().to_vec(),
            hid::X if ctrl => {
                self.clipboard = self.address.selected_text().as_bytes().to_vec();
                self.address.insert("");
            }
            hid::V if ctrl => {
                let paste = String::from_utf8_lossy(&self.clipboard).into_owned();
                self.address.insert(&paste);
            }
            _ => return,
        }
        self.frame_dirty = true;
    }

    /// Committed text from the keyboard or the IME.
    pub fn text_input(&mut self, text: &str) {
        if self.address.focused() {
            self.address.insert(text);
            self.frame_dirty = true;
            return;
        }
        let now = self.now_ms();
        if let Some(tab) = self.tabs.get_mut(self.active) {
            if tab.running() {
                tab.send(&ToTab::TextInput {
                    text: text.to_string(),
                    clock_ms: now,
                });
                tab.tick_due = true;
            }
        }
    }

    /// The window is closing: every tab leaves, saving its storage on the
    /// way (the tab answers the shutdown with it; see `ToShell::Storage`).
    pub fn shutdown(mut self) {
        let ids: Vec<TabId> = self.tabs.iter().map(|t| t.id).collect();
        for id in ids {
            if let Some(conn) = find_tab(&mut self.tabs, id).and_then(|t| t.conn.take()) {
                conn.shutdown();
            }
        }
        self.drain();
    }
}

/// The tab with `id` among `tabs`.
fn find_tab(tabs: &mut [Tab], id: TabId) -> Option<&mut Tab> {
    tabs.iter_mut().find(|t| t.id == id)
}

/// The page area's logical size and scale for a tab at `zoom`.
fn page_size(page: Rect, zoom: f32, scale: f32) -> (u32, u32, f32) {
    (
        (page.w / zoom).round().max(1.0) as u32,
        (page.h / zoom).round().max(1.0) as u32,
        scale * zoom,
    )
}

fn x100(v: f32) -> i32 {
    (v * 100.0).round() as i32
}

/// USB HID usage codes the shell's own shortcuts use (`vy_abi::key` has
/// Tab, Enter and Escape).
pub mod hid {
    /// A.
    pub const A: i32 = 0x04;
    /// C.
    pub const C: i32 = 0x06;
    /// L.
    pub const L: i32 = 0x0F;
    /// T.
    pub const T: i32 = 0x17;
    /// V.
    pub const V: i32 = 0x19;
    /// W.
    pub const W: i32 = 0x1A;
    /// X.
    pub const X: i32 = 0x1B;
    /// 0.
    pub const DIGIT0: i32 = 0x27;
    /// Backspace.
    pub const BACKSPACE: i32 = 0x2A;
    /// Minus.
    pub const MINUS: i32 = 0x2D;
    /// Equal (Plus unshifted).
    pub const EQUAL: i32 = 0x2E;
    /// F5.
    pub const F5: i32 = 0x3E;
    /// Home.
    pub const HOME: i32 = 0x4A;
    /// Delete forward.
    pub const DELETE: i32 = 0x4C;
    /// End.
    pub const END: i32 = 0x4D;
    /// Right arrow.
    pub const RIGHT: i32 = 0x4F;
    /// Left arrow.
    pub const LEFT: i32 = 0x50;
    /// Keypad minus.
    pub const KP_MINUS: i32 = 0x56;
    /// Keypad plus.
    pub const KP_PLUS: i32 = 0x57;
    /// Keypad 0.
    pub const KP_0: i32 = 0x62;
}
