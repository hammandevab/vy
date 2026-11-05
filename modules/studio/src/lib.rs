//! # studio
//!
//! Vy Studio: the page a person builds pages from. A mission is typed
//! ("gör en sida som …"), **Kör** hands it to `vy-dev serve`, which runs
//! Claude Code in the page's project, and Studio shows what the agent says
//! and does, the build log, a status line and the address of the page
//! under development as a link. It is an ordinary ABI 1.0 page; the client
//! knows nothing about it.
//!
//! What the page exercises in the ABI:
//! - `fetch_begin` / `fetch_poll` / `fetch_read` (SPEC-5.2.7) against the
//!   dev API on its own origin, which the manifest lists under `net`
//!   (SPEC-5.2.8) and the default policy grants without asking because it
//!   is the page's own (SPEC-8.2.3). `GET /_dev/state` gives name,
//!   address, build status and log; `GET /_dev/agent/events` is a
//!   long-poll for the log entries, re-issued as soon as one answers, so a
//!   still page waits on the network and spends no frames (SPEC-6.2.2);
//!   `POST /_dev/agent`, `/_dev/agent/stop` and `/_dev/build` are the
//!   buttons. A refusal or a lost connection is shown, never fatal
//!   (SPEC-5.3.1); a lost connection is retried after a pause.
//! - Where the API is and this run's token are not known when the module
//!   is built: `vy-dev serve` writes them over the placeholder in
//!   [`CONFIG`] before it signs the package it serves. Built as is, the
//!   page talks to `http://localhost:8081` with a token of zeros, which no
//!   run accepts (the conformance cases answer it from canned responses).
//! - `link_register` (SPEC-5.2.10) for the address of the page under
//!   development, so the client can open it in a tab of its own.
//! - `state_put` / `state_get` (SPEC-5.2.9, capability `storage`) keep the
//!   last mission across visits.
//! - `ime_begin` / `ime_end` (SPEC-5.2.6) follow the host's focus on the
//!   mission field (SPEC-10.3.1); requires capability `ime` (SPEC-10.4.2).
//! - `SCROLL` moves the agent log, which otherwise follows its newest entry.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod json;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use vy_sdk::api::Request;
use vy_sdk::event::key;
use vy_sdk::prelude::*;
use vy_sdk::{
    fetch_begin, fetch_cancel, fetch_poll, fetch_read, ime_begin, ime_end, link_register, now_ms,
    state_get, state_put, text_caret, Method,
};

/// Length of [`CONFIG`].
pub const CONFIG_LEN: usize = 128;
/// What `vy-dev serve` looks for in the module to find [`CONFIG`]; it
/// occurs exactly once.
pub const CONFIG_MARKER: &[u8] = b"VYDEV-STUDIO-CONFIG ";
/// The origin of the dev API when nothing was written over the placeholder.
pub const DEFAULT_ORIGIN: &str = "http://localhost:8081";

const fn config_block() -> [u8; CONFIG_LEN] {
    let src = b"VYDEV-STUDIO-CONFIG http://localhost:8081 00000000000000000000000000000000";
    let mut out = [b' '; CONFIG_LEN];
    let mut i = 0;
    while i < src.len() {
        out[i] = src[i];
        i += 1;
    }
    out
}

/// `VYDEV-STUDIO-CONFIG <origin> <token>`, padded with spaces. `vy-dev
/// serve` rewrites the bytes after the marker in the module before it signs
/// the package it serves; the page reads them with volatile loads, so the
/// optimizer cannot fold the placeholder into the code.
#[used]
static CONFIG: [u8; CONFIG_LEN] = config_block();

/// The dev API's origin and this run's token, from [`CONFIG`].
fn read_config() -> (String, String) {
    let mut raw = [0u8; CONFIG_LEN];
    let base = CONFIG.as_ptr();
    for (i, b) in raw.iter_mut().enumerate() {
        // SAFETY: `i < CONFIG_LEN`, so the read stays inside the static,
        // which lives for the whole program and is never written.
        *b = unsafe { core::ptr::read_volatile(base.add(i)) };
    }
    parse_config(&raw)
}

/// `(origin, token)` of a config block; the defaults for anything else.
fn parse_config(raw: &[u8]) -> (String, String) {
    let text = raw
        .strip_prefix(CONFIG_MARKER)
        .and_then(|rest| core::str::from_utf8(rest).ok())
        .unwrap_or("");
    let mut words = text.split_whitespace();
    let origin = words
        .next()
        .filter(|o| o.starts_with("http://") || o.starts_with("https://"))
        .unwrap_or(DEFAULT_ORIGIN);
    let token = words.next().unwrap_or("");
    (origin.to_string(), token.to_string())
}

const BG: Color = Color::rgb(0xF2, 0xF3, 0xF5);
const PANEL: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const INK: Color = Color::rgb(0x1C, 0x1E, 0x26);
const MUTED: Color = Color::rgb(0x6B, 0x70, 0x7C);
const RULE: Color = Color::rgb(0xD8, 0xDB, 0xE2);
const ACCENT: Color = Color::rgb(0x2F, 0x5B, 0xD6);
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const RING: Color = Color::rgb(0xF2, 0x9F, 0x05);
const GREEN: Color = Color::rgb(0x1E, 0x7F, 0x4F);
const AMBER: Color = Color::rgb(0xB4, 0x5F, 0x06);
const RED: Color = Color::rgb(0xC0, 0x2B, 0x2B);
const TERM_BG: Color = Color::rgb(0x1C, 0x1E, 0x26);
const TERM_INK: Color = Color::rgb(0xD8, 0xDB, 0xE2);
const TERM_ERR: Color = Color::rgb(0xFF, 0x8A, 0x80);
const DISABLED: Color = Color::rgb(0xA9, 0xAE, 0xB8);
const BAR: Color = Color::rgba(0x1C, 0x1E, 0x26, 0x50);

const TITLE: &str = "Studio";
const MISSION_LABEL: &str = "Uppdrag";
const PLACEHOLDER: &str = "Gör en sida som …";
const RUN: &str = "Kör";
const STOP: &str = "Stoppa";
const REBUILD: &str = "Bygg om";
const AGENT_TITLE: &str = "Agenten";
const BUILD_TITLE: &str = "Bygglogg";
const AGENT_HINT: &str =
    "Skriv ett uppdrag och tryck Kör. Claude Code bygger sidan i projektet, och det agenten säger och gör syns här.";
const NO_LOG: &str = "Inget bygge än.";
const CONNECTING: &str = "ansluter till vy-dev …";

/// Storage key of the last mission.
const MISSION_KEY: &str = "studio.mission";
/// Longest mission the field takes, in bytes (a semantic node holds 4 KB,
/// SPEC-9.3.2).
const MAX_MISSION: usize = 4000;
/// Entries the agent log keeps.
const MAX_ENTRIES: usize = 300;
/// Entries asked for per poll.
const POLL_LIMIT: u32 = 200;
/// How long the dev API may hold a poll open, in ms. Short, because a
/// client may perform requests one at a time.
const POLL_WAIT_MS: u32 = 1000;
/// Pause before reconnecting after a lost connection, in ms.
const RETRY_MS: i64 = 2000;
/// Longest text one semantic node carries.
const MAX_NODE_TEXT: usize = 4000;

const MARGIN: f32 = 20.0;
const FIELD_H: f32 = 44.0;
const BUTTON_W: f32 = 108.0;
const GAP: f32 = 10.0;
const PANEL_HEAD: f32 = 32.0;
const PANEL_PAD: f32 = 12.0;
const ENTRY_GAP: f32 = 8.0;
const LOG_LINE_H: f32 = 15.0;
const LINE_PX: f32 = 24.0;
/// Windows at least this wide get the two logs side by side.
const TWO_COLUMNS_MIN: f32 = 720.0;
/// Windows at least this wide get the buttons next to the field.
const ONE_ROW_MIN: f32 = 560.0;

/// Fixed semantic ids of the controls (SPEC-10.3.1).
const ID_LINK: i32 = -1;
const ID_MISSION: i32 = -2;
const ID_RUN: i32 = -3;
const ID_BUILD: i32 = -4;
/// `link_register` id of the address.
const LINK_ADDRESS: i32 = 1;

/// Where the last build is, as the dev API says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Build {
    Waiting,
    Building,
    Ok,
    Error,
}

impl Build {
    fn from_name(name: &str) -> Build {
        match name {
            "building" => Build::Building,
            "ok" => Build::Ok,
            "error" => Build::Error,
            _ => Build::Waiting,
        }
    }

    /// The word in the status line.
    fn word(self) -> &'static str {
        match self {
            Build::Waiting => "väntar",
            Build::Building => "bygger",
            Build::Ok => "klar",
            Build::Error => "fel",
        }
    }

    fn color(self) -> Color {
        match self {
            Build::Waiting => MUTED,
            Build::Building => AMBER,
            Build::Ok => GREEN,
            Build::Error => RED,
        }
    }
}

/// What a log entry is (the dev API's `kind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Agent,
    Assistant,
    Tool,
    Result,
    Build(Build),
}

struct Entry {
    kind: Kind,
    text: String,
    /// Height at the current width.
    h: f32,
}

/// Which request a `FETCH_DONE` closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    State,
    Events,
    Action(Action),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Run,
    Stop,
    Rebuild,
}

/// Every rectangle of the page, computed from the window.
struct Layout {
    title: Rect,
    status: Rect,
    address: Rect,
    label: Rect,
    field: Rect,
    run: Rect,
    build: Rect,
    notice: Rect,
    agent: Rect,
    log: Rect,
}

impl Layout {
    fn agent_body(&self) -> Rect {
        body_of(self.agent)
    }

    fn log_body(&self) -> Rect {
        body_of(self.log)
    }
}

fn body_of(panel: Rect) -> Rect {
    Rect::new(
        panel.x + PANEL_PAD,
        panel.y + PANEL_HEAD,
        (panel.w - 2.0 * PANEL_PAD).max(0.0),
        (panel.h - PANEL_HEAD - PANEL_PAD).max(0.0),
    )
}

struct Styles {
    title: StyleId,
    pill: StyleId,
    label: StyleId,
    link: StyleId,
    field: StyleId,
    placeholder: StyleId,
    button: StyleId,
    head: StyleId,
    notice: StyleId,
    error: StyleId,
    assistant: StyleId,
    tool: StyleId,
    agent: StyleId,
    result: StyleId,
    build_ok: StyleId,
    build_err: StyleId,
    log: StyleId,
    log_err: StyleId,
}

impl Styles {
    fn new() -> Result<Self, VyError> {
        let s = text_style;
        Ok(Self {
            title: s(Font::Sans, 22.0, 700, INK)?,
            pill: s(Font::Sans, 13.0, 600, WHITE)?,
            label: s(Font::Sans, 13.0, 400, MUTED)?,
            link: s(Font::Sans, 13.0, 600, ACCENT)?,
            field: s(Font::Sans, 15.0, 400, INK)?,
            placeholder: s(Font::Sans, 15.0, 400, MUTED)?,
            button: s(Font::Sans, 15.0, 600, WHITE)?,
            head: s(Font::Sans, 13.0, 700, INK)?,
            notice: s(Font::Sans, 13.0, 400, MUTED)?,
            error: s(Font::Sans, 13.0, 600, RED)?,
            assistant: s(Font::Sans, 14.0, 400, INK)?,
            tool: s(Font::Mono, 12.0, 400, MUTED)?,
            agent: s(Font::Sans, 12.0, 600, MUTED)?,
            result: s(Font::Sans, 14.0, 700, INK)?,
            build_ok: s(Font::Sans, 12.0, 600, GREEN)?,
            build_err: s(Font::Sans, 12.0, 600, RED)?,
            log: s(Font::Mono, 12.0, 400, TERM_INK)?,
            log_err: s(Font::Mono, 12.0, 400, TERM_ERR)?,
        })
    }
}

/// `text` cut to at most `max` bytes on a character boundary.
fn cut(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// The last `max` bytes of `text` on a character boundary.
fn tail(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut start = text.len() - max;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

struct Studio {
    window: Window,
    styles: Styles,
    origin: String,
    token: String,
    name: String,
    address: String,
    build: Build,
    build_log: String,
    agent_running: bool,
    entries: Vec<Entry>,
    /// Width the entry heights were measured at.
    measured_w: f32,
    /// The sequence number to poll from.
    after: u64,
    /// The first events answer (the history) has been read.
    history: bool,
    mission: String,
    focused: Option<i32>,
    /// A line under the field: what the last button did, or why not.
    notice: Option<(String, bool)>,
    /// Why the dev API cannot be reached, while it cannot.
    offline: Option<String>,
    retry_at: Option<i64>,
    pending: Vec<(Request, Pending)>,
    scroll: f32,
    stick: bool,
    content_h: f32,
}

impl Studio {
    fn layout(&self) -> Layout {
        let w = self.window.width;
        let h = self.window.height;
        let inner = (w - 2.0 * MARGIN).max(0.0);
        let title = Rect::new(MARGIN, 16.0, inner, 28.0);
        let status = Rect::new(MARGIN + inner - 96.0, 18.0, 96.0, 24.0);
        let address = Rect::new(MARGIN, 52.0, inner, 18.0);
        let label = Rect::new(MARGIN, 82.0, inner, 18.0);
        let top = 102.0;
        let (field, run, build, below) = if w >= ONE_ROW_MIN {
            let fw = (inner - 2.0 * (BUTTON_W + GAP)).max(40.0);
            let field = Rect::new(MARGIN, top, fw, FIELD_H);
            let run = Rect::new(field.right() + GAP, top, BUTTON_W, FIELD_H);
            let build = Rect::new(run.right() + GAP, top, BUTTON_W, FIELD_H);
            (field, run, build, top + FIELD_H)
        } else {
            let field = Rect::new(MARGIN, top, inner, FIELD_H);
            let bw = ((inner - GAP) / 2.0).max(0.0);
            let y = field.bottom() + GAP;
            let run = Rect::new(MARGIN, y, bw, FIELD_H);
            let build = Rect::new(run.right() + GAP, y, bw, FIELD_H);
            (field, run, build, y + FIELD_H)
        };
        let notice = Rect::new(MARGIN, below + 8.0, inner, 18.0);
        let panels_top = notice.bottom() + 10.0;
        let avail = (h - MARGIN - panels_top).max(0.0);
        let (agent, log) = if w >= TWO_COLUMNS_MIN {
            let aw = ((inner - 16.0) * 0.58).max(0.0);
            let agent = Rect::new(MARGIN, panels_top, aw, avail);
            let log = Rect::new(
                agent.right() + 16.0,
                panels_top,
                (inner - 16.0 - aw).max(0.0),
                avail,
            );
            (agent, log)
        } else {
            let ah = ((avail - 12.0) * 0.6).max(0.0);
            let agent = Rect::new(MARGIN, panels_top, inner, ah);
            let log = Rect::new(
                MARGIN,
                agent.bottom() + 12.0,
                inner,
                (avail - 12.0 - ah).max(0.0),
            );
            (agent, log)
        };
        Layout {
            title,
            status,
            address,
            label,
            field,
            run,
            build,
            notice,
            agent,
            log,
        }
    }

    fn url(&self, path: &str) -> String {
        let sep = if path.contains('?') { '&' } else { '?' };
        format!("{}{path}{sep}token={}", self.origin, self.token)
    }

    fn is_pending(&self, what: Pending) -> bool {
        self.pending.iter().any(|(_, p)| *p == what)
    }

    fn begin(&mut self, method: Method, path: &str, body: &[u8], what: Pending) {
        match fetch_begin(method, &self.url(path), body) {
            Ok(req) => self.pending.push((req, what)),
            Err(e) => {
                let _ = log(Level::Warn, &format!("studio: fetch_begin refused: {e}"));
                match what {
                    Pending::Action(_) => {
                        self.notice = Some((format!("kunde inte skicka ({})", e.name()), true))
                    }
                    _ => self.lost(e),
                }
            }
        }
    }

    fn poll_state(&mut self) {
        if !self.is_pending(Pending::State) {
            self.begin(Method::Get, "/_dev/state", b"", Pending::State);
        }
    }

    fn poll_events(&mut self) {
        if self.is_pending(Pending::Events) {
            return;
        }
        let wait = if self.history { POLL_WAIT_MS } else { 0 };
        let path = format!(
            "/_dev/agent/events?after={}&wait={wait}&raw=0&limit={POLL_LIMIT}",
            self.after
        );
        self.begin(Method::Get, &path, b"", Pending::Events);
    }

    /// The dev API cannot be reached: say why and, unless the client
    /// refused the request itself, try again after a pause.
    fn lost(&mut self, e: VyError) {
        self.offline = Some(match e {
            VyError::Denied => String::from("nätet är inte beviljat för Studio (E_DENIED)"),
            e => format!("ingen kontakt med vy-dev ({})", e.name()),
        });
        if e != VyError::Denied && self.retry_at.is_none() {
            self.retry_at = Some(now_ms() + RETRY_MS);
            request_frame();
        }
    }

    fn reconnect(&mut self) {
        self.retry_at = None;
        self.poll_state();
        self.poll_events();
    }

    /// `GET /_dev/state` answered.
    fn on_state(&mut self, body: &[u8]) {
        let Some(v) = json::parse(body) else {
            self.notice = Some((String::from("kunde inte läsa svaret på /_dev/state"), true));
            return;
        };
        if let Some(n) = v.get("name").str() {
            self.name = n.to_string();
        }
        if let Some(a) = v.get("address").str() {
            self.address = a.to_string();
        }
        let build = v.get("build");
        if let Some(s) = build.get("status").str() {
            self.build = Build::from_name(s);
        }
        if let Some(l) = build.get("log").str() {
            self.build_log = l.to_string();
        }
        if let Some(r) = v.get("agent").get("running").bool() {
            self.agent_running = r;
        }
    }

    /// `GET /_dev/agent/events` answered. Returns whether the state should
    /// be read again (a build finished and its log changed).
    fn on_events(&mut self, body: &[u8]) -> bool {
        let Some(v) = json::parse(body) else {
            self.notice = Some((String::from("kunde inte läsa loggen från vy-dev"), true));
            return false;
        };
        if let Some(n) = v.get("next").num() {
            if n >= 0.0 {
                self.after = n as u64;
            }
        }
        let initial = !self.history;
        self.history = true;
        let mut refresh = false;
        for e in v.get("entries").arr() {
            let text = e.get("text").str().unwrap_or("").to_string();
            let kind = match e.get("kind").str().unwrap_or("") {
                "agent" => {
                    if text.starts_with("uppdrag:") {
                        self.agent_running = true;
                    } else if text.starts_with("agenten ") || text.starts_with("kunde inte starta")
                    {
                        self.agent_running = false;
                    }
                    Kind::Agent
                }
                "assistant" => Kind::Assistant,
                "tool" => Kind::Tool,
                "result" => Kind::Result,
                "build" => {
                    let b = Build::from_name(e.get("status").str().unwrap_or(""));
                    self.build = b;
                    refresh |= !initial && matches!(b, Build::Ok | Build::Error);
                    Kind::Build(b)
                }
                _ => continue,
            };
            self.entries.push(Entry { kind, text, h: 0.0 });
        }
        if self.entries.len() > MAX_ENTRIES {
            let excess = self.entries.len() - MAX_ENTRIES;
            self.entries.drain(..excess);
        }
        self.relayout(false);
        refresh
    }

    /// A button's request answered.
    fn on_action(&mut self, action: Action, status: i32, body: &[u8]) {
        let error = || {
            json::parse(body)
                .and_then(|v| v.get("error").str().map(String::from))
                .unwrap_or_default()
        };
        self.notice = Some(match (action, status) {
            (Action::Run, 200..=299) => (String::from("uppdraget skickat till agenten"), false),
            (Action::Stop, 200..=299) => (String::from("agenten stoppas"), false),
            (Action::Rebuild, 200..=299) => (String::from("bygget begärt"), false),
            (Action::Run, 409) => (String::from("en agent arbetar redan"), true),
            (_, 403) => (
                String::from("dev-API:t nekade (403): öppna Studio från vy-dev serve"),
                true,
            ),
            (_, s) if s < 0 => (
                format!(
                    "ingen kontakt med vy-dev ({})",
                    VyError::from_code(s).unwrap_or(VyError::Net).name()
                ),
                true,
            ),
            (_, s) => (format!("vy-dev svarade {s}: {}", error()), true),
        });
    }

    /// The body of a finished request: read to the 0 that frees it
    /// (SPEC-5.2.7), or cancelled when it is not readable.
    fn read_all(req: Request) -> Vec<u8> {
        let mut out = Vec::new();
        if fetch_poll(req) != Ok(1) {
            let _ = fetch_cancel(req);
            return out;
        }
        let mut buf = [0u8; 4096];
        loop {
            match fetch_read(req, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
        }
        out
    }

    fn finish(&mut self, request: i32, status: i32) -> bool {
        let Some(i) = self.pending.iter().position(|(r, _)| r.0 == request) else {
            return false;
        };
        let (req, what) = self.pending.remove(i);
        let body = Self::read_all(req);
        if let Pending::Action(a) = what {
            self.on_action(a, status, &body);
            return true;
        }
        if !(200..300).contains(&status) {
            match VyError::from_code(status) {
                Some(e) if status < 0 => self.lost(e),
                _ => {
                    self.offline = Some(if status == 403 {
                        String::from("dev-API:t nekade (403): öppna Studio från vy-dev serve")
                    } else {
                        format!("vy-dev svarade {status}")
                    });
                }
            }
            return true;
        }
        self.offline = None;
        match what {
            Pending::State => self.on_state(&body),
            Pending::Events => {
                if self.on_events(&body) {
                    self.poll_state();
                }
                self.poll_events();
            }
            Pending::Action(_) => {}
        }
        true
    }

    fn run(&mut self) -> bool {
        if self.agent_running {
            if !self.is_pending(Pending::Action(Action::Stop)) {
                self.begin(
                    Method::Post,
                    "/_dev/agent/stop",
                    b"",
                    Pending::Action(Action::Stop),
                );
            }
            return true;
        }
        let mission = self.mission.trim();
        if mission.is_empty() {
            self.notice = Some((String::from("skriv ett uppdrag först"), true));
            return true;
        }
        if self.is_pending(Pending::Action(Action::Run)) {
            return false;
        }
        let mission = String::from(mission);
        if let Err(e) = state_put(MISSION_KEY, mission.as_bytes()) {
            let _ = log(Level::Debug, &format!("studio: mission not kept: {e}"));
        }
        self.begin(
            Method::Post,
            "/_dev/agent",
            mission.as_bytes(),
            Pending::Action(Action::Run),
        );
        true
    }

    fn rebuild(&mut self) -> bool {
        if !self.is_pending(Pending::Action(Action::Rebuild)) {
            self.begin(
                Method::Post,
                "/_dev/build",
                b"",
                Pending::Action(Action::Rebuild),
            );
        }
        true
    }

    fn set_focus(&mut self, node: i32, gained: bool) -> bool {
        let before = self.focused;
        if gained {
            self.focused = Some(node);
        } else if self.focused == Some(node) {
            self.focused = None;
        }
        let was = before == Some(ID_MISSION);
        let is = self.focused == Some(ID_MISSION);
        if is && !was {
            let _ = ime_begin(self.layout().field);
        } else if was && !is {
            let _ = ime_end();
        }
        before != self.focused
    }

    fn entry_style(&self, kind: Kind) -> StyleId {
        let s = &self.styles;
        match kind {
            Kind::Agent | Kind::Build(Build::Waiting | Build::Building) => s.agent,
            Kind::Assistant => s.assistant,
            Kind::Tool => s.tool,
            Kind::Result => s.result,
            Kind::Build(Build::Ok) => s.build_ok,
            Kind::Build(Build::Error) => s.build_err,
        }
    }

    fn entry_text(kind: Kind, text: &str) -> String {
        match kind {
            Kind::Tool => format!("› {text}"),
            Kind::Result => format!("Resultat: {text}"),
            Kind::Build(_) => format!("Bygge: {text}"),
            Kind::Agent | Kind::Assistant => String::from(text),
        }
    }

    /// Measure the entries at the agent log's width: all of them when the
    /// width changed (or `all`), else the new ones.
    fn relayout(&mut self, all: bool) {
        let w = self.layout().agent_body().w;
        let all = all || w != self.measured_w;
        self.measured_w = w;
        let mut content = 0.0;
        for i in 0..self.entries.len() {
            if all || self.entries[i].h == 0.0 {
                let style = self.entry_style(self.entries[i].kind);
                let text = Self::entry_text(self.entries[i].kind, &self.entries[i].text);
                let h = text_measure(style, w, cut(&text, 60_000)).map_or(16.0, |s| s.h);
                self.entries[i].h = h.max(1.0);
            }
            content += self.entries[i].h + ENTRY_GAP;
        }
        self.content_h = content;
        let target = if self.stick { f32::MAX } else { self.scroll };
        self.scroll_to(target);
    }

    fn max_scroll(&self) -> f32 {
        (self.content_h - self.layout().agent_body().h).max(0.0)
    }

    fn scroll_to(&mut self, y: f32) -> bool {
        let max = self.max_scroll();
        let clamped = y.clamp(0.0, max);
        let changed = clamped != self.scroll;
        self.scroll = clamped;
        self.stick = clamped >= max;
        changed
    }

    fn status_text(&self) -> String {
        let mut s = format!("Status: {}", self.build.word());
        if self.agent_running {
            s.push_str(", agenten arbetar");
        }
        s
    }

    fn draw_button(
        &self,
        f: &mut Frame,
        r: Rect,
        label: &str,
        id: i32,
        enabled: bool,
    ) -> Result<(), VyError> {
        if self.focused == Some(id) {
            f.rect(r.inset(-3.0), 9.0, RING)?;
        }
        f.rect(r, 6.0, if enabled { ACCENT } else { DISABLED })?;
        let tw = text_measure(self.styles.button, 0.0, label)?.w;
        f.text(
            r.x + (r.w - tw) / 2.0,
            r.y + (r.h - 19.0) / 2.0,
            0.0,
            self.styles.button,
            label,
        )?;
        f.sem.node_with_id(id, Role::Button, r, label)?;
        Ok(())
    }

    fn draw_header(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let s = &self.styles;
        let title = if self.name.is_empty() {
            String::from(TITLE)
        } else {
            format!("{TITLE} – {}", self.name)
        };
        let max_w = (l.status.x - l.title.x - GAP).max(0.0);
        f.clipped(
            Rect::new(l.title.x, l.title.y, max_w, l.title.h),
            0.0,
            |f| {
                f.text(l.title.x, l.title.y, 0.0, s.title, &title)
                    .map(|_| ())
            },
        )?;
        f.sem.node(Role::Heading, l.title, &title)?;

        let word = self.build.word();
        let pw = text_measure(s.pill, 0.0, word)?.w + 24.0;
        let pill = Rect::new(l.status.right() - pw, l.status.y, pw, l.status.h);
        f.rect(pill, 12.0, self.build.color())?;
        f.text(pill.x + 12.0, pill.y + 4.0, 0.0, s.pill, word)?;
        f.sem.node(Role::Text, pill, &self.status_text())?;

        let prefix = "Sidan under utveckling: ";
        let pre_w = f.text(l.address.x, l.address.y, 0.0, s.label, prefix)?.w;
        if self.address.is_empty() {
            let w = f
                .text(l.address.x + pre_w, l.address.y, 0.0, s.label, CONNECTING)?
                .w;
            f.sem.node(
                Role::Text,
                Rect::new(l.address.x, l.address.y, pre_w + w, l.address.h),
                CONNECTING,
            )?;
        } else {
            let x = l.address.x + pre_w;
            let max_w = (l.address.right() - x).max(0.0);
            let size = text_measure(s.link, 0.0, &self.address)?;
            let link = Rect::new(x, l.address.y, size.w.min(max_w), size.h.max(16.0));
            f.clipped(
                Rect::new(x, l.address.y - 2.0, max_w, l.address.h + 4.0),
                0.0,
                |f| {
                    f.text(x, l.address.y, 0.0, s.link, &self.address)?;
                    f.rect(Rect::new(x, link.bottom() - 1.0, link.w, 1.0), 0.0, ACCENT)
                },
            )?;
            if self.focused == Some(ID_LINK) {
                f.rect(
                    Rect::new(link.x - 2.0, link.bottom() + 1.0, link.w + 4.0, 2.0),
                    0.0,
                    RING,
                )?;
            }
            link_register(LINK_ADDRESS, link, &self.address)?;
            f.sem
                .node_with_id(ID_LINK, Role::Link, link, &self.address)?;
        }
        Ok(())
    }

    fn draw_controls(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let s = &self.styles;
        f.text(l.label.x, l.label.y, 0.0, s.label, MISSION_LABEL)?;
        f.sem.node(Role::Text, l.label, MISSION_LABEL)?;

        let r = l.field;
        let focused = self.focused == Some(ID_MISSION);
        let line = if focused { 2.0 } else { 1.0 };
        f.rect(r, 8.0, if focused { ACCENT } else { RULE })?;
        f.rect(r.inset(line), 8.0 - line, PANEL)?;
        let inner = r.inset(12.0);
        let ty = r.y + (r.h - 19.0) / 2.0;
        if self.mission.is_empty() {
            f.text(inner.x, ty, 0.0, s.placeholder, PLACEHOLDER)?;
        } else {
            // One line, scrolled so the end, where typing happens, shows.
            let w = text_measure(s.field, 0.0, &self.mission)?.w;
            let shift = (w - inner.w + 2.0).max(0.0);
            f.clipped(Rect::new(inner.x, r.y, inner.w, r.h), 0.0, |f| {
                f.text(inner.x - shift, ty, 0.0, s.field, &self.mission)
                    .map(|_| ())
            })?;
        }
        if focused {
            let (cx, _) = text_caret(s.field, &self.mission, self.mission.len())?;
            let w = text_measure(s.field, 0.0, &self.mission)?.w;
            let shift = (w - inner.w + 2.0).max(0.0);
            f.rect(
                Rect::new(inner.x + cx - shift, ty + 1.0, 1.0, 17.0),
                0.0,
                INK,
            )?;
        }
        f.sem
            .node_with_id(ID_MISSION, Role::Textbox, r, &self.mission)?;

        let run_label = if self.agent_running { STOP } else { RUN };
        let can_run = self.agent_running || !self.mission.trim().is_empty();
        self.draw_button(f, l.run, run_label, ID_RUN, can_run)?;
        self.draw_button(f, l.build, REBUILD, ID_BUILD, true)?;

        let line = match (&self.offline, &self.notice) {
            (Some(why), _) => Some((why.as_str(), true)),
            (None, Some((text, err))) => Some((text.as_str(), *err)),
            _ => None,
        };
        if let Some((text, err)) = line {
            let style = if err { s.error } else { s.notice };
            f.clipped(l.notice, 0.0, |f| {
                f.text(l.notice.x, l.notice.y, 0.0, style, text).map(|_| ())
            })?;
            f.sem.node(Role::Text, l.notice, text)?;
        }
        Ok(())
    }

    fn draw_agent(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let s = &self.styles;
        let p = l.agent;
        f.rect(p, 10.0, PANEL)?;
        let head = if self.agent_running {
            format!("{AGENT_TITLE} · arbetar")
        } else {
            String::from(AGENT_TITLE)
        };
        let hr = Rect::new(p.x + PANEL_PAD, p.y + 9.0, p.w - 2.0 * PANEL_PAD, 18.0);
        f.text(hr.x, hr.y, 0.0, s.head, &head)?;
        f.sem.node(Role::Heading, hr, &head)?;
        let body = l.agent_body();
        if self.entries.is_empty() {
            let h = f.text(body.x, body.y, body.w, s.notice, AGENT_HINT)?.h;
            f.sem
                .node(Role::Text, Rect::new(body.x, body.y, body.w, h), AGENT_HINT)?;
            return Ok(());
        }
        f.clipped(body, 0.0, |f| {
            f.group(Role::List, body, "Agentens logg", |f| {
                let mut y = body.y - self.scroll;
                for e in &self.entries {
                    let text = Self::entry_text(e.kind, &e.text);
                    if y + e.h > body.y && y < body.bottom() {
                        f.text(
                            body.x,
                            y,
                            body.w,
                            self.entry_style(e.kind),
                            cut(&text, 60_000),
                        )?;
                    }
                    let item = Rect::new(body.x, y, body.w, e.h);
                    f.sem
                        .node(Role::ListItem, item, cut(&text, MAX_NODE_TEXT))?;
                    y += e.h + ENTRY_GAP;
                }
                Ok(())
            })?;
            if self.max_scroll() > 0.0 {
                let track = body.h;
                let thumb_h = (track * body.h / self.content_h).max(24.0);
                let thumb_y = body.y + (track - thumb_h) * (self.scroll / self.max_scroll());
                f.rect(
                    Rect::new(body.right() - 4.0, thumb_y, 4.0, thumb_h),
                    2.0,
                    BAR,
                )?;
            }
            Ok(())
        })
    }

    fn draw_log(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let s = &self.styles;
        let p = l.log;
        f.rect(p, 10.0, TERM_BG)?;
        let hr = Rect::new(p.x + PANEL_PAD, p.y + 9.0, p.w - 2.0 * PANEL_PAD, 18.0);
        f.text(hr.x, hr.y, 0.0, s.log, BUILD_TITLE)?;
        f.sem.node(Role::Heading, hr, BUILD_TITLE)?;
        let body = l.log_body();
        let shown = if self.build_log.trim().is_empty() {
            NO_LOG
        } else {
            self.build_log.trim_end()
        };
        // The tail that fits: the errors come last.
        let rows = (body.h / LOG_LINE_H).max(0.0) as usize;
        let lines: Vec<&str> = shown.lines().collect();
        let first = lines.len().saturating_sub(rows);
        f.clipped(body, 0.0, |f| {
            for (i, line) in lines[first..].iter().enumerate() {
                let style = if line.starts_with("error") {
                    s.log_err
                } else {
                    s.log
                };
                f.text(
                    body.x,
                    body.y + i as f32 * LOG_LINE_H,
                    0.0,
                    style,
                    cut(line, 2000),
                )?;
            }
            Ok(())
        })?;
        f.sem
            .node(Role::Text, body, tail(shown, MAX_NODE_TEXT))
            .map(|_| ())
    }
}

impl App for Studio {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let (origin, token) = read_config();
        let mut mission = String::new();
        let mut buf = [0u8; MAX_MISSION];
        if let Ok(n) = state_get(MISSION_KEY, &mut buf) {
            if let Ok(text) = core::str::from_utf8(&buf[..n.min(MAX_MISSION)]) {
                mission.push_str(text);
            }
        }
        let mut page = Self {
            window,
            styles: Styles::new()?,
            origin,
            token,
            name: String::new(),
            address: String::new(),
            build: Build::Waiting,
            build_log: String::new(),
            agent_running: false,
            entries: Vec::new(),
            measured_w: 0.0,
            after: 0,
            history: false,
            mission,
            focused: None,
            notice: None,
            offline: None,
            retry_at: None,
            pending: Vec::new(),
            scroll: 0.0,
            stick: true,
            content_h: 0.0,
        };
        page.poll_state();
        page.poll_events();
        Ok(page)
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        if let Some(at) = self.retry_at {
            if now_ms() >= at {
                self.reconnect();
            } else {
                request_frame();
            }
        }
        let l = self.layout();
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, BG)?;
        f.group(Role::Main, window, "", |f| {
            self.draw_header(f, &l)?;
            self.draw_controls(f, &l)?;
            f.group(Role::Region, l.agent, AGENT_TITLE, |f| {
                self.draw_agent(f, &l)
            })?;
            f.group(Role::Region, l.log, BUILD_TITLE, |f| self.draw_log(f, &l))
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::FetchDone {
                request, status, ..
            } => self.finish(request, status),
            Event::PointerDown { x, y, .. } => {
                let l = self.layout();
                if l.run.contains(x, y) {
                    self.run()
                } else if l.build.contains(x, y) {
                    self.rebuild()
                } else {
                    false
                }
            }
            Event::TextInput { text } => {
                if self.focused == Some(ID_MISSION)
                    && self.mission.len() + text.len() <= MAX_MISSION
                {
                    // One line: a pasted newline becomes a space.
                    self.mission
                        .extend(text.chars().map(|c| if c.is_control() { ' ' } else { c }));
                    true
                } else {
                    false
                }
            }
            Event::KeyDown { code, .. } => match (code, self.focused) {
                (key::ENTER, Some(ID_MISSION | ID_RUN)) => self.run(),
                (key::SPACE, Some(ID_RUN)) => self.run(),
                (key::ENTER | key::SPACE, Some(ID_BUILD)) => self.rebuild(),
                (key::BACKSPACE, Some(ID_MISSION)) => self.mission.pop().is_some(),
                (key::PAGE_DOWN, _) => self.scroll_to(self.scroll + self.layout().agent_body().h),
                (key::PAGE_UP, _) => self.scroll_to(self.scroll - self.layout().agent_body().h),
                _ => false,
            },
            // The agent log is the one thing that scrolls.
            Event::Scroll { dy, lines, .. } => {
                let px = if lines { dy * LINE_PX } else { dy };
                self.scroll_to(self.scroll + px)
            }
            Event::Focus { node, gained, .. } => self.set_focus(node.0, gained),
            Event::Capability {
                cap: Capability::Net,
                granted,
            } => {
                if granted {
                    self.offline = None;
                    self.reconnect();
                } else {
                    self.offline =
                        Some(String::from("nätet är inte beviljat för Studio (E_DENIED)"));
                }
                true
            }
            _ => false,
        }
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
        self.relayout(true);
        if self.focused == Some(ID_MISSION) {
            // The field moved; tell the host where composition happens now.
            let _ = ime_begin(self.layout().field);
        }
    }
}

vy_sdk::export!(Studio);

#[cfg(test)]
mod tests {
    use super::*;
    use vy_sdk::mock;

    fn window(width: f32, height: f32) -> Window {
        Window {
            width,
            height,
            scale: 1.0,
        }
    }

    const STATE: &[u8] = br#"{"name":"min-sida","address":"vy://localhost:8080/min-sida","build":{"status":"ok","log":"$ cargo build\nABI 1.0: OK\nok: 1.2 s\n","builds":1},"agent":{"running":false,"last_exit":null},"seq":2}"#;
    const EVENTS: &[u8] = r#"{"next":7,"entries":[{"seq":3,"kind":"agent","text":"uppdrag: gör en sida"},{"seq":4,"kind":"assistant","text":"Jag ändrar rubriken."},{"seq":5,"kind":"tool","text":"Edit src/lib.rs"},{"seq":6,"kind":"build","text":"bygger","status":"building"},{"seq":7,"kind":"unknown","text":"x"}]}"#.as_bytes();

    #[test]
    fn config_placeholder_and_patched() {
        assert_eq!(CONFIG.len(), CONFIG_LEN);
        assert!(CONFIG.starts_with(CONFIG_MARKER));
        let (origin, token) = read_config();
        assert_eq!(origin, DEFAULT_ORIGIN);
        assert_eq!(token, "0".repeat(32));
        let mut patched = [b' '; CONFIG_LEN];
        let text = b"VYDEV-STUDIO-CONFIG http://localhost:9000 abc123";
        patched[..text.len()].copy_from_slice(text);
        assert_eq!(
            parse_config(&patched),
            ("http://localhost:9000".into(), "abc123".into())
        );
        assert_eq!(parse_config(b"garbage").0, DEFAULT_ORIGIN);
        assert_eq!(
            parse_config(b"VYDEV-STUDIO-CONFIG ftp://x t").0,
            DEFAULT_ORIGIN
        );
    }

    /// SPEC-5.3.1: in a host without `net` the first requests are refused;
    /// the page says so and does not retry until the capability comes.
    #[test]
    fn spec_5_3_1_without_net_studio_says_so() {
        mock::reset();
        let mut page = Studio::init(window(800.0, 600.0), 0).unwrap();
        assert!(page.pending.is_empty());
        assert!(page.offline.as_deref().unwrap().contains("E_DENIED"));
        assert_eq!(page.retry_at, None, "a refusal is not retried");
        assert!(mock::run_frame(&mut page, 16).unwrap() > 10);
        mock::with(|h| {
            assert!(h.sem.iter().any(|n| n.text.contains("E_DENIED")));
            assert_eq!(h.frame_requests, 0, "a still page asks for no frames");
        });
    }

    #[test]
    fn answers_become_state_and_log() {
        mock::reset();
        let mut page = Studio::init(window(800.0, 600.0), 0).unwrap();
        page.on_state(STATE);
        assert_eq!(page.name, "min-sida");
        assert_eq!(page.address, "vy://localhost:8080/min-sida");
        assert_eq!(page.build, Build::Ok);
        assert!(page.build_log.contains("ABI 1.0: OK"));
        assert!(!page.on_events(EVENTS), "the history needs no new state");
        assert_eq!(page.after, 7);
        assert!(page.history);
        assert!(page.agent_running, "uppdrag: starts a run");
        assert_eq!(page.build, Build::Building);
        assert_eq!(page.entries.len(), 4, "the unknown kind is skipped");
        assert_eq!(page.status_text(), "Status: bygger, agenten arbetar");
        let done = r#"{"next":9,"entries":[{"seq":8,"kind":"agent","text":"agenten är klar"},{"seq":9,"kind":"build","text":"klar: vy://localhost:8080/min-sida","status":"ok"}]}"#.as_bytes();
        assert!(page.on_events(done), "a finished build is read again");
        assert!(!page.agent_running);
        assert_eq!(page.build, Build::Ok);
        page.on_events(b"not json");
        assert!(page.notice.as_ref().unwrap().1);
        assert_eq!(page.after, 9, "a bad answer moves nothing");
        page.on_action(Action::Run, 409, br#"{"error":"busy"}"#);
        assert_eq!(page.notice.as_ref().unwrap().0, "en agent arbetar redan");
        page.on_action(Action::Rebuild, 500, br#"{"error":"trasigt"}"#);
        assert_eq!(
            page.notice.as_ref().unwrap().0,
            "vy-dev svarade 500: trasigt"
        );
        page.on_action(Action::Stop, -11, b"");
        assert!(page.notice.as_ref().unwrap().0.starts_with("ingen kontakt"));
    }

    /// SPEC-10.1.1: every visible text and control is a node, the address
    /// is a link registered for the frame (SPEC-5.2.10), the field is a
    /// textbox, and the log entries are list items.
    #[test]
    fn spec_10_1_1_the_tree_names_everything() {
        mock::reset();
        mock::with(|h| h.caps.storage = true);
        let mut page = Studio::init(window(800.0, 600.0), 0).unwrap();
        page.on_state(STATE);
        page.on_events(EVENTS);
        assert!(mock::run_frame(&mut page, 16).unwrap() > 20);
        mock::with(|h| {
            let has = |role: Role, text: &str| {
                h.sem
                    .iter()
                    .any(|n| n.role == role as i32 && n.text.contains(text))
            };
            assert!(has(Role::Heading, "Studio – min-sida"));
            assert!(has(Role::Text, "Status: bygger, agenten arbetar"));
            assert!(has(Role::Link, "vy://localhost:8080/min-sida"));
            assert!(has(Role::Textbox, ""));
            assert!(has(Role::Button, "Stoppa"), "Kör stops a running agent");
            assert!(has(Role::Button, "Bygg om"));
            assert!(has(Role::ListItem, "› Edit src/lib.rs"));
            assert!(has(Role::ListItem, "Jag ändrar rubriken."));
            assert!(has(Role::Text, "ABI 1.0: OK"));
            assert_eq!(h.links.len(), 1);
            assert_eq!(h.links[0].5, "vy://localhost:8080/min-sida");
        });
    }

    /// Typing goes to the focused field only; the mission is kept in
    /// storage when it is run and comes back in a new instance.
    #[test]
    fn spec_5_2_9_typing_and_the_kept_mission() {
        mock::reset();
        mock::with(|h| {
            h.caps.storage = true;
            h.caps.ime = true;
        });
        let mut page = Studio::init(window(800.0, 600.0), 0).unwrap();
        assert!(
            !page.event(Event::TextInput { text: "x" }),
            "no focus, no text"
        );
        assert!(page.event(Event::Focus {
            node: NodeId(ID_MISSION),
            gained: true,
            keyboard: false,
        }));
        mock::with(|h| assert!(h.ime_rect.is_some(), "ime follows focus"));
        assert!(page.event(Event::TextInput {
            text: "gör en sida\nsom hälsar",
        }));
        assert_eq!(page.mission, "gör en sida som hälsar");
        assert!(page.event(Event::KeyDown {
            code: key::BACKSPACE,
            modifiers: Modifiers(0),
            repeat: false,
        }));
        assert!(
            page.run(),
            "the request is refused in the mock, the mission kept"
        );
        let kept = mock::with(|h| h.state.get(MISSION_KEY.as_bytes()).cloned()).unwrap();
        assert_eq!(kept, "gör en sida som hälsa".as_bytes());
        assert!(page.notice.as_ref().unwrap().1, "the refusal is shown");
        let again = Studio::init(window(800.0, 600.0), 0).unwrap();
        assert_eq!(again.mission, "gör en sida som hälsa");
        assert!(page.event(Event::Focus {
            node: NodeId(ID_MISSION),
            gained: false,
            keyboard: false,
        }));
        mock::with(|h| assert!(h.ime_rect.is_none()));
    }

    /// The layout keeps the controls and both logs inside the window and
    /// apart, wide and narrow.
    #[test]
    fn layout_fits() {
        mock::reset();
        for (w, h) in [(800.0, 600.0), (1280.0, 800.0), (400.0, 700.0)] {
            let page = Studio::init(window(w, h), 0).unwrap();
            let l = page.layout();
            let inside = |r: Rect| r.x >= 0.0 && r.y >= 0.0 && r.right() <= w && r.bottom() <= h;
            for r in [l.title, l.address, l.field, l.run, l.build, l.agent, l.log] {
                assert!(inside(r), "{w}x{h}: {r:?}");
            }
            let apart = |a: Rect, b: Rect| {
                a.right() <= b.x || b.right() <= a.x || a.bottom() <= b.y || b.bottom() <= a.y
            };
            assert!(apart(l.field, l.run) && apart(l.run, l.build) && apart(l.agent, l.log));
            assert!(apart(l.notice, l.agent) && apart(l.field, l.notice));
            assert!(l.agent_body().h > 60.0, "{w}x{h}");
        }
    }
}
