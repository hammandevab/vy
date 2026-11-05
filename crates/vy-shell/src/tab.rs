//! The tab process (SPEC-9.1.1): one `Runtime` driven by messages from
//! the main process, answering with display lists and everything else the
//! page produces. It holds no window, no rasterizer and no network
//! (SPEC-9.1.2): pixels are the main process's, and a page's own
//! requests go back to it as [`ToShell::FetchRequest`].
//!
//! The loop is one message at a time, in order, and every call into the
//! module happens here. Nothing in it reads a clock or the operating
//! system's entropy: the clock comes with each message and the random
//! key with the load, so the inbound stream is the whole session. A
//! `--record` writes that stream to a file frame by frame and a
//! `--replay` reads it back in place of the transport, which reproduces
//! the session exactly (SPEC-11.4.1, SPEC-2.1.5).

use crate::ipc::{write_frame, IpcTransport, LoadRequest, StyleSpec, ToShell, ToTab};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter};
use std::path::Path;
use vy_abi::event;
use vy_runtime::{Capabilities, ClockSource, LogSource, Options, Resource, Runtime, Transport};

/// The tab loop's state around the runtime.
pub struct TabLoop<T: IpcTransport> {
    transport: T,
    record: Option<BufWriter<File>>,
    rt: Option<Runtime>,
    /// Set once the page is dead (trap, exit, hard limit, forced suspend).
    stopped: bool,
    styles_sent: usize,
    images_sent: Vec<bool>,
    logs_sent: usize,
    last_links: Vec<vy_runtime::Link>,
    last_sem: Vec<vy_runtime::SemNode>,
    last_ime: Option<(f32, f32, f32, f32)>,
    last_focus: Option<i32>,
    /// Set by [`ToTab::CopyGesture`] and spent by the next event: that
    /// event is delivered as a copy gesture (SPEC-8.2.4).
    copy_gesture_armed: bool,
}

/// Capability ids packed as bits for [`LoadRequest::capabilities`].
pub fn pack_capabilities(caps: Capabilities) -> u8 {
    (1..=vy_abi::cap::MAX)
        .filter(|id| caps.has(*id))
        .fold(0u8, |bits, id| bits | 1 << (id - 1))
}

/// The capabilities behind packed bits; bits outside 1.0's ids are ignored.
pub fn unpack_capabilities(bits: u8) -> Capabilities {
    let mut caps = Capabilities::default();
    for id in 1..=vy_abi::cap::MAX {
        if bits & (1 << (id - 1)) != 0 {
            caps.set(id, true);
        }
    }
    caps
}

impl<T: IpcTransport> TabLoop<T> {
    /// A loop over `transport`, recording every inbound frame to
    /// `record` when given.
    pub fn new(transport: T, record: Option<&Path>) -> io::Result<Self> {
        let record = match record {
            Some(p) => Some(BufWriter::new(File::create(p)?)),
            None => None,
        };
        Ok(Self {
            transport,
            record,
            rt: None,
            stopped: false,
            styles_sent: 0,
            images_sent: Vec::new(),
            logs_sent: 0,
            last_links: Vec::new(),
            last_sem: Vec::new(),
            last_ime: None,
            last_focus: None,
            copy_gesture_armed: false,
        })
    }

    /// Serve messages until the other side is gone or says
    /// [`ToTab::Shutdown`].
    pub fn run(&mut self) -> io::Result<()> {
        while let Some(body) = self.transport.recv()? {
            if let Some(rec) = &mut self.record {
                write_frame(rec, &body)?;
            }
            let msg = match ToTab::decode(&body) {
                Ok(m) => m,
                Err(e) => {
                    self.send(&ToShell::Stopped {
                        reason: format!("bad message from the shell: {e}"),
                    })?;
                    return Err(io::Error::new(io::ErrorKind::InvalidData, e.to_string()));
                }
            };
            if !self.handle(msg)? {
                break;
            }
        }
        if let Some(rec) = &mut self.record {
            use std::io::Write;
            rec.flush()?;
        }
        Ok(())
    }

    fn send(&mut self, msg: &ToShell) -> io::Result<()> {
        self.transport.send(&msg.encode())
    }

    /// Handle one message; false when the loop should end.
    fn handle(&mut self, msg: ToTab) -> io::Result<bool> {
        match msg {
            ToTab::Load(req) => self.load(*req)?,
            ToTab::Tick { clock_ms } => self.tick(clock_ms)?,
            ToTab::Event {
                kind,
                a,
                b,
                c,
                d,
                clock_ms,
            } => {
                // Spent whether or not the page is alive: the grant was
                // for this event only.
                let gesture = std::mem::take(&mut self.copy_gesture_armed);
                if let Some(rt) = self.alive() {
                    rt.set_clock_ms(clock_ms);
                    let r = if gesture {
                        rt.event_as_copy_gesture(kind, a, b, c, d)
                    } else {
                        rt.event(kind, a, b, c, d)
                    };
                    self.settle(r.map(|_| ()))?;
                }
            }
            ToTab::CopyGesture => self.copy_gesture_armed = true,
            ToTab::StopRecording => {
                // The frame itself is already in the file (see `run`); a
                // replay reads it and does nothing.
                if let Some(mut rec) = self.record.take() {
                    use std::io::Write;
                    rec.flush()?;
                }
            }
            ToTab::TextInput { text, clock_ms } => {
                if let Some(rt) = self.alive() {
                    rt.set_clock_ms(clock_ms);
                    let r = rt.text_input(&text);
                    self.settle(r.map(|_| ()))?;
                }
            }
            ToTab::Resize {
                width,
                height,
                scale,
            } => {
                if let Some(rt) = self.alive() {
                    let r = rt.resize(width, height, scale);
                    self.settle(r)?;
                }
            }
            ToTab::Visibility { visible, clock_ms } => {
                if let Some(rt) = self.alive() {
                    rt.set_clock_ms(clock_ms);
                    let r = if visible {
                        rt.event(event::VISIBILITY, 1, 0, 0, 0).map(|_| ())
                    } else {
                        // Hidden: the page may write its state away first (SPEC-5.1.5).
                        rt.suspend()
                            .and_then(|_| rt.event(event::VISIBILITY, 0, 0, 0, 0))
                            .map(|_| ())
                    };
                    self.settle(r)?;
                    if !visible {
                        self.send_storage()?;
                    }
                }
            }
            ToTab::Grant { cap, granted } => {
                if let Some(rt) = self.alive() {
                    let r = rt.set_capability(cap, granted);
                    self.settle(r.map(|_| ()))?;
                }
            }
            ToTab::FetchResponse { id, status, body } => {
                if let Some(rt) = self.alive() {
                    let r = rt.fetch_complete(id, status, body);
                    self.settle(r.map(|_| ()))?;
                }
            }
            ToTab::Suspend => {
                if let Some(rt) = self.alive() {
                    let r = rt.suspend();
                    self.settle(r.map(|_| ()))?;
                }
                self.send_storage()?;
            }
            ToTab::Shutdown => {
                if let Some(rt) = self.alive() {
                    let r = rt.suspend();
                    self.settle(r.map(|_| ()))?;
                }
                self.send_storage()?;
                return Ok(false);
            }
            ToTab::Clipboard { bytes } => {
                if let Some(rt) = &mut self.rt {
                    rt.set_clipboard(Some(bytes));
                }
            }
            ToTab::Raster {
                truncated,
                ops_done,
                units,
                budget,
            } => {
                if let Some(rt) = self.alive() {
                    let r = rt.raster_report(&vy_runtime::RasterReport {
                        truncated,
                        ops_done: ops_done as usize,
                        units,
                        budget,
                    });
                    self.settle(r)?;
                }
            }
        }
        Ok(true)
    }

    /// The runtime, when the page is alive.
    fn alive(&mut self) -> Option<&mut Runtime> {
        if self.stopped {
            None
        } else {
            self.rt.as_mut()
        }
    }

    fn load(&mut self, req: LoadRequest) -> io::Result<()> {
        self.rt = None;
        self.stopped = false;
        self.styles_sent = 0;
        self.images_sent = vec![false; req.resources.len() + 1];
        self.logs_sent = 0;
        self.last_links.clear();
        self.last_sem.clear();
        self.last_ime = None;
        self.last_focus = None;
        self.copy_gesture_armed = false;
        let options = Options {
            memory_max_pages: req.memory_max_pages,
            capabilities: unpack_capabilities(req.capabilities),
            resources: req
                .resources
                .into_iter()
                .map(|(bytes, pixels)| Resource { bytes, pixels })
                .collect(),
            storage: req.storage.into_iter().collect::<BTreeMap<_, _>>(),
            storage_quota_bytes: req.storage_quota_bytes,
            fetch: Vec::new(),
            net_origins: req.net_origins,
            clipboard: req.clipboard,
            clock: ClockSource::External,
            random: req.random,
            transport: Transport::External,
            navigation: true,
        };
        let pid = std::process::id();
        let outcome = Runtime::load(&req.wasm, &req.sha256, options).and_then(|mut rt| {
            rt.set_clock_ms(req.clock_ms);
            rt.init(req.width, req.height, req.scale).map(|()| rt)
        });
        match outcome {
            Ok(rt) => {
                self.rt = Some(rt);
                // Everything `vy_init` produced, then the answer: a message
                // that answers another is always the last of its batch.
                self.after_call()?;
                self.send(&ToShell::Loaded {
                    ok: true,
                    error: String::new(),
                    pid,
                })
            }
            Err(e) => self.send(&ToShell::Loaded {
                ok: false,
                error: e.to_string(),
                pid,
            }),
        }
    }

    fn tick(&mut self, clock_ms: i64) -> io::Result<()> {
        let Some(rt) = self.alive() else {
            return self.send(&ToShell::Idle);
        };
        rt.set_clock_ms(clock_ms);
        if !rt.wants_frame() {
            return self.send(&ToShell::Idle);
        }
        let outcome = rt.frame().map(<[u8]>::to_vec);
        match outcome {
            Ok(display_list) => {
                let rt = self.rt.as_mut().expect("alive");
                let stats = rt.last_frame();
                let wants_more = rt.wants_frame();
                let styles: Vec<StyleSpec> = {
                    let text = rt.text_engine_mut();
                    (self.styles_sent + 1..=text.style_count())
                        .filter_map(|id| text.get_style(id as u16))
                        .map(|s| StyleSpec {
                            font: s.font,
                            size_px: s.size_px,
                            weight: i32::from(s.weight),
                            color: s.color,
                        })
                        .collect()
                };
                self.styles_sent += styles.len();
                // What the frame produced first, the frame itself last, so
                // the shell can take `Frame` as the end of the tick's answer.
                self.after_call()?;
                self.send_new_images()?;
                self.send_tree_and_links()?;
                self.send(&ToShell::Frame {
                    display_list,
                    styles,
                    wants_more,
                    over_budget: stats.over_budget,
                })?;
                // A frame may have suspended the module (SPEC-5.3.3).
                if self.rt.as_ref().is_some_and(Runtime::is_suspended) {
                    self.stop("module suspended after repeated budget breaches (SPEC-5.3.3)")?;
                }
                Ok(())
            }
            Err(e) => {
                self.after_call()?;
                self.stop(&e.to_string())
            }
        }
    }

    /// After a call into the module: a trap or exit stops the page;
    /// otherwise everything the call produced goes to the shell.
    fn settle(&mut self, r: Result<(), vy_runtime::Error>) -> io::Result<()> {
        self.after_call()?;
        match r {
            Ok(()) => {
                if self.rt.as_ref().is_some_and(Runtime::is_suspended) {
                    self.stop("module suspended after repeated budget breaches (SPEC-5.3.3)")?;
                }
                Ok(())
            }
            Err(e) => self.stop(&e.to_string()),
        }
    }

    fn stop(&mut self, reason: &str) -> io::Result<()> {
        if self.stopped {
            return Ok(());
        }
        self.stopped = true;
        self.send(&ToShell::Stopped {
            reason: reason.to_string(),
        })
    }

    /// Images the runtime decoded since the last frame (SPEC-6.5.1).
    fn send_new_images(&mut self) -> io::Result<()> {
        let Some(rt) = self.rt.as_ref() else {
            return Ok(());
        };
        let count = rt.image_cache().resource_count();
        let mut ready = Vec::new();
        for id in 1..=count {
            if !self.images_sent.get(id).copied().unwrap_or(true) {
                if let Ok(img) = rt.image_cache().get(id as u16) {
                    ready.push((id as u16, img.clone()));
                }
            }
        }
        for (id, image) in ready {
            self.images_sent[usize::from(id)] = true;
            self.send(&ToShell::Image { id, image })?;
        }
        Ok(())
    }

    fn send_tree_and_links(&mut self) -> io::Result<()> {
        let Some(rt) = self.rt.as_ref() else {
            return Ok(());
        };
        if rt.links() != self.last_links.as_slice() {
            self.last_links = rt.links().to_vec();
            let links = self.last_links.clone();
            self.send(&ToShell::Links(links))?;
        }
        let rt = self.rt.as_ref().expect("checked");
        if rt.semantic_tree() != self.last_sem.as_slice() {
            self.last_sem = rt.semantic_tree().to_vec();
            let nodes = self.last_sem.clone();
            self.send(&ToShell::Semantic(nodes))?;
        }
        Ok(())
    }

    /// Logs, navigations, fetch requests, clipboard, IME and focus changes
    /// since the last call.
    fn after_call(&mut self) -> io::Result<()> {
        let Some(rt) = self.rt.as_mut() else {
            return Ok(());
        };
        let mut out = Vec::new();
        for entry in &rt.logs()[self.logs_sent.min(rt.logs().len())..] {
            out.push(ToShell::Log {
                module: entry.source == LogSource::Module,
                level: entry.level,
                message: entry.message.clone(),
            });
        }
        self.logs_sent = rt.logs().len();
        for n in rt.take_navigations() {
            out.push(ToShell::Navigate {
                url: n.url,
                replace: n.replace,
            });
        }
        for f in rt.take_fetch_requests() {
            out.push(ToShell::FetchRequest {
                id: f.id,
                method: f.method,
                url: f.url,
                body: f.body,
            });
        }
        // Every write, even of the same bytes: the shell puts it on the
        // system clipboard, which may have changed in between.
        if let Some(bytes) = rt.take_clipboard_write() {
            out.push(ToShell::Clipboard(bytes));
        }
        let ime = rt.ime_rect();
        if ime != self.last_ime {
            self.last_ime = ime;
            out.push(ToShell::Ime(ime));
        }
        let focus = rt.focus();
        if focus != self.last_focus {
            self.last_focus = focus;
            out.push(ToShell::Focus(focus));
        }
        for m in &out {
            self.send(m)?;
        }
        Ok(())
    }

    fn send_storage(&mut self) -> io::Result<()> {
        let entries = match self.rt.as_ref() {
            Some(rt) => rt
                .storage()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            None => Vec::new(),
        };
        self.send(&ToShell::Storage(entries))
    }
}

/// A transport whose inbound frames come from a recording instead of the
/// other side; outbound frames still go to `inner` (SPEC-11.4.1).
pub struct ReplayTransport<T: IpcTransport> {
    file: BufReader<File>,
    inner: T,
}

impl<T: IpcTransport> ReplayTransport<T> {
    /// Replay the frames of `path` over `inner`.
    pub fn open(path: &Path, inner: T) -> io::Result<Self> {
        Ok(Self {
            file: BufReader::new(File::open(path)?),
            inner,
        })
    }
}

impl<T: IpcTransport> IpcTransport for ReplayTransport<T> {
    fn send(&mut self, body: &[u8]) -> io::Result<()> {
        self.inner.send(body)
    }
    fn recv(&mut self) -> io::Result<Option<Vec<u8>>> {
        crate::ipc::read_frame(&mut self.file)
    }
}

/// The `--tab` entry point: serve the parent over stdin and stdout, with
/// an optional record or replay file. Stdout carries frames only; the
/// parent reads nothing else from it.
pub fn main(record: Option<&Path>, replay: Option<&Path>) -> io::Result<()> {
    let stdin = io::stdin().lock();
    let stdout = io::stdout().lock();
    let pipe = crate::ipc::PipeTransport::new(stdin, stdout);
    match replay {
        Some(path) => TabLoop::new(ReplayTransport::open(path, pipe)?, record)?.run(),
        None => TabLoop::new(pipe, record)?.run(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::ChannelTransport;
    use crate::process::{channel_sink, spawn_thread, Inbound, TabEvent};
    use std::sync::mpsc;
    use std::time::Duration;
    use vy_runtime::RandomSource;

    /// `target/wasm/<name>.wasm`, built by `tools/build_modules.sh`.
    pub(crate) fn module(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/wasm")
            .join(format!("{name}.wasm"));
        std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}\nrun tools/build_modules.sh before cargo test",
                path.display()
            )
        })
    }

    /// A load request for `wasm` at 800 × 600, no capabilities, the
    /// headless random seed.
    pub(crate) fn load_request(wasm: Vec<u8>, capabilities: u8) -> LoadRequest {
        LoadRequest {
            sha256: vy_runtime::sha256(&wasm),
            wasm,
            memory_max_pages: 64,
            capabilities,
            resources: Vec::new(),
            storage: Vec::new(),
            storage_quota_bytes: 1 << 20,
            net_origins: Vec::new(),
            random: RandomSource::default(),
            width: 800,
            height: 600,
            scale: 1.0,
            clock_ms: 0,
            clipboard: None,
        }
    }

    /// Run the loop on a thread over channels and return the shell's ends.
    fn start() -> (
        mpsc::Sender<Vec<u8>>,
        mpsc::Receiver<Vec<u8>>,
        std::thread::JoinHandle<()>,
    ) {
        let (to_tab, tab_rx) = mpsc::channel();
        let (tab_tx, from_tab) = mpsc::channel();
        let h = std::thread::spawn(move || {
            TabLoop::new(
                ChannelTransport {
                    tx: tab_tx,
                    rx: tab_rx,
                },
                None,
            )
            .unwrap()
            .run()
            .unwrap();
        });
        (to_tab, from_tab, h)
    }

    fn recv(rx: &mpsc::Receiver<Vec<u8>>) -> ToShell {
        ToShell::decode(&rx.recv_timeout(Duration::from_secs(20)).expect("a message")).unwrap()
    }

    /// Messages until one satisfies `stop`, that one included.
    fn recv_until(rx: &mpsc::Receiver<Vec<u8>>, stop: impl Fn(&ToShell) -> bool) -> Vec<ToShell> {
        let mut out = Vec::new();
        loop {
            let m = recv(rx);
            let done = stop(&m);
            out.push(m);
            if done {
                return out;
            }
        }
    }

    /// SPEC-9.1.1, SPEC-5.1.1, SPEC-5.1.2, SPEC-6.2.1: a load answers
    /// `Loaded`, the first tick after it carries a frame (init wants one)
    /// with the page's styles, the tree and the logs follow, a second tick
    /// is `Idle` for a static page, and a shutdown yields the storage and
    /// ends the loop.
    #[test]
    fn spec_9_1_1_tab_loop_serves_frames() {
        let (tx, rx, h) = start();
        let req = load_request(module("hello"), 0);
        tx.send(ToTab::Load(Box::new(req)).encode()).unwrap();
        let msgs = recv_until(&rx, |m| matches!(m, ToShell::Loaded { .. }));
        assert_eq!(
            msgs.last(),
            Some(&ToShell::Loaded {
                ok: true,
                error: String::new(),
                pid: std::process::id()
            })
        );
        assert!(msgs.iter().any(
            |m| matches!(m, ToShell::Log { module: true, message, .. } if message == "hello: init")
        ));
        tx.send(ToTab::Tick { clock_ms: 16 }.encode()).unwrap();
        let msgs = recv_until(&rx, |m| matches!(m, ToShell::Frame { .. }));
        let frame = msgs
            .iter()
            .find_map(|m| match m {
                ToShell::Frame {
                    display_list,
                    styles,
                    wants_more,
                    over_budget,
                } => Some((
                    display_list.clone(),
                    styles.clone(),
                    *wants_more,
                    *over_budget,
                )),
                _ => None,
            })
            .expect("a frame");
        assert!(vy_dl::decode(&frame.0).unwrap().len() > 2);
        assert!(!frame.1.is_empty(), "hello registers text styles");
        assert!(!frame.2, "hello is static");
        assert!(!frame.3);
        assert!(msgs
            .iter()
            .any(|m| matches!(m, ToShell::Semantic(n) if n.len() == 3)));
        tx.send(ToTab::Tick { clock_ms: 32 }.encode()).unwrap();
        assert_eq!(recv(&rx), ToShell::Idle);
        tx.send(ToTab::Shutdown.encode()).unwrap();
        assert_eq!(recv(&rx), ToShell::Storage(vec![]));
        assert!(rx.recv().is_err(), "the loop ended");
        h.join().unwrap();
    }

    /// SPEC-9.4.1, SPEC-11.2.2: a load that fails (wrong hash, not a
    /// module) answers `Loaded { ok: false }` with the reason and the
    /// loop keeps serving; a module that traps stops the page with the
    /// reason, after which ticks are idle, and the loop still answers.
    #[test]
    fn spec_11_2_2_failures_are_answered_not_fatal() {
        let (tx, rx, h) = start();
        let mut bad = load_request(module("hello"), 0);
        bad.sha256 = [0; 32];
        tx.send(ToTab::Load(Box::new(bad)).encode()).unwrap();
        match recv(&rx) {
            ToShell::Loaded { ok, error, .. } => {
                assert!(!ok);
                assert!(error.contains("SPEC-9.4.1"), "{error}");
            }
            other => panic!("{other:?}"),
        }
        tx.send(ToTab::Tick { clock_ms: 0 }.encode()).unwrap();
        assert_eq!(recv(&rx), ToShell::Idle);
        let garbage = load_request(b"not wasm".to_vec(), 0);
        tx.send(ToTab::Load(Box::new(garbage)).encode()).unwrap();
        assert!(matches!(recv(&rx), ToShell::Loaded { ok: false, .. }));

        let trapping = wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32) unreachable)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
        )
        .unwrap();
        tx.send(ToTab::Load(Box::new(load_request(trapping, 0))).encode())
            .unwrap();
        assert!(matches!(recv(&rx), ToShell::Loaded { ok: true, .. }));
        tx.send(ToTab::Tick { clock_ms: 0 }.encode()).unwrap();
        let msgs = recv_until(&rx, |m| matches!(m, ToShell::Stopped { .. }));
        assert!(matches!(
            msgs.last(),
            Some(ToShell::Stopped { reason }) if reason.contains("trapped")
        ));
        tx.send(ToTab::Tick { clock_ms: 16 }.encode()).unwrap();
        assert_eq!(recv(&rx), ToShell::Idle, "a dead page is idle");
        tx.send(
            ToTab::Event {
                kind: 2,
                a: 0,
                b: 0,
                c: 1,
                d: 0,
                clock_ms: 16,
            }
            .encode(),
        )
        .unwrap();
        tx.send(ToTab::Shutdown.encode()).unwrap();
        assert_eq!(recv(&rx), ToShell::Storage(vec![]));
        h.join().unwrap();
    }

    /// SPEC-5.2.7, SPEC-5.2.10, SPEC-5.2.9: the chat page's `fetch_begin`
    /// comes out as a `FetchRequest` the shell answers; `navigate` from
    /// the document page comes out as `Navigate`; storage written by the
    /// settings page comes back in `Storage` on suspend.
    #[test]
    fn spec_5_2_7_requests_cross_the_boundary() {
        let (tx, rx, h) = start();
        let mut req = load_request(
            module("chat"),
            pack_capabilities(Capabilities::from_names(["ime", "net"]).unwrap()),
        );
        req.net_origins = vec!["https://chat.example".into()];
        tx.send(ToTab::Load(Box::new(req)).encode()).unwrap();
        recv_until(&rx, |m| matches!(m, ToShell::Loaded { .. }));
        tx.send(ToTab::Tick { clock_ms: 0 }.encode()).unwrap();
        recv_until(&rx, |m| matches!(m, ToShell::Frame { .. }));
        // Tab to the composer, type, Enter: the page posts the message.
        let key = |code: i32, clock_ms: i64| ToTab::Event {
            kind: vy_abi::event::KEY_DOWN,
            a: code,
            b: 0,
            c: 0,
            d: 0,
            clock_ms,
        };
        tx.send(key(vy_abi::key::TAB, 16).encode()).unwrap();
        tx.send(
            ToTab::TextInput {
                text: "Hej".into(),
                clock_ms: 32,
            }
            .encode(),
        )
        .unwrap();
        tx.send(key(vy_abi::key::ENTER, 48).encode()).unwrap();
        tx.send(ToTab::Tick { clock_ms: 64 }.encode()).unwrap();
        let msgs = recv_until(&rx, |m| matches!(m, ToShell::Frame { .. }));
        let fetch = msgs
            .iter()
            .find_map(|m| match m {
                ToShell::FetchRequest {
                    id, method, url, ..
                } => Some((*id, *method, url.clone())),
                _ => None,
            })
            .expect("the page asked for a request");
        assert_eq!(
            fetch,
            (
                1,
                vy_abi::method::POST,
                "https://chat.example/api/messages".into()
            )
        );
        tx.send(
            ToTab::FetchResponse {
                id: 1,
                status: 200,
                body: b"ok".to_vec(),
            }
            .encode(),
        )
        .unwrap();
        tx.send(ToTab::Tick { clock_ms: 80 }.encode()).unwrap();
        let msgs = recv_until(&rx, |m| matches!(m, ToShell::Frame { .. }));
        assert!(msgs.iter().any(|m| matches!(
            m,
            ToShell::Semantic(nodes) if nodes.iter().any(|n| n.text.ends_with("levererat: Hej"))
        )));
        tx.send(ToTab::Shutdown.encode()).unwrap();
        recv_until(&rx, |m| matches!(m, ToShell::Storage(_)));
        h.join().unwrap();

        let (tx, rx, h) = start();
        let req = load_request(
            module("settings"),
            pack_capabilities(Capabilities::from_names(["storage"]).unwrap()),
        );
        tx.send(ToTab::Load(Box::new(req)).encode()).unwrap();
        recv_until(&rx, |m| matches!(m, ToShell::Loaded { .. }));
        tx.send(ToTab::Tick { clock_ms: 0 }.encode()).unwrap();
        recv_until(&rx, |m| matches!(m, ToShell::Frame { .. }));
        tx.send(
            ToTab::Event {
                kind: vy_abi::event::POINTER_DOWN,
                a: 35_400,
                b: 15_600,
                c: 1,
                d: 0,
                clock_ms: 16,
            }
            .encode(),
        )
        .unwrap();
        tx.send(ToTab::Suspend.encode()).unwrap();
        let msgs = recv_until(&rx, |m| matches!(m, ToShell::Storage(_)));
        assert!(matches!(msgs.last(), Some(ToShell::Storage(e)) if !e.is_empty()));
        tx.send(ToTab::Shutdown.encode()).unwrap();
        recv_until(&rx, |m| matches!(m, ToShell::Storage(_)));
        h.join().unwrap();

        let (tx, rx, h) = start();
        tx.send(ToTab::Load(Box::new(load_request(module("document"), 0))).encode())
            .unwrap();
        recv_until(&rx, |m| matches!(m, ToShell::Loaded { .. }));
        tx.send(ToTab::Tick { clock_ms: 0 }.encode()).unwrap();
        let msgs = recv_until(&rx, |m| matches!(m, ToShell::Frame { .. }));
        assert!(msgs
            .iter()
            .any(|m| matches!(m, ToShell::Links(l) if l.len() > 5)));
        // Tab to the first link, Enter: the page navigates.
        tx.send(key(vy_abi::key::TAB, 16).encode()).unwrap();
        tx.send(key(vy_abi::key::ENTER, 32).encode()).unwrap();
        tx.send(ToTab::Tick { clock_ms: 48 }.encode()).unwrap();
        let msgs = recv_until(&rx, |m| matches!(m, ToShell::Frame { .. } | ToShell::Idle));
        assert!(msgs.iter().any(|m| matches!(
            m,
            ToShell::Navigate { url, replace: false } if url == "vy://docs.example/personer/maja-ek"
        )), "{msgs:?}");
        tx.send(ToTab::Shutdown.encode()).unwrap();
        h.join().unwrap();
    }

    /// SPEC-8.2.4, SPEC-5.2.12, SPEC-5.3.1: a `clipboard_write` while the
    /// page handles a copy gesture (the `KEY_DOWN` that follows a
    /// `CopyGesture`) succeeds without the `clipboard` capability and
    /// comes out as `Clipboard`; the same write on any other event is
    /// `E_DENIED` and logged, and the grant is spent by the one event:
    /// a `CopyGesture` arms exactly the next event, whatever it is.
    #[test]
    fn spec_8_2_4_copy_gesture_grants_clipboard_write() {
        // On every event: `clipboard_write("kopia")`, then a redraw request.
        let wasm = wat::parse_str(
            r#"(module
                (import "vy" "clipboard_write" (func $cw (param i32 i32) (result i32)))
                (memory (export "memory") 1)
                (data (i32.const 16) "kopia")
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32) i32.const 0)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                    (drop (call $cw (i32.const 16) (i32.const 5)))
                    i32.const 1)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
        )
        .unwrap();
        let (tx, rx, h) = start();
        let mut req = load_request(wasm, 0);
        req.clipboard = Some(Vec::new());
        tx.send(ToTab::Load(Box::new(req)).encode()).unwrap();
        recv_until(&rx, |m| matches!(m, ToShell::Loaded { .. }));
        let key = |code: i32, modifiers: i32, clock_ms: i64| ToTab::Event {
            kind: vy_abi::event::KEY_DOWN,
            a: code,
            b: modifiers,
            c: 0,
            d: 0,
            clock_ms,
        };
        // Without a gesture: denied and logged, nothing written.
        tx.send(key(crate::shell::hid::C, vy_abi::modifier::CTRL, 16).encode())
            .unwrap();
        tx.send(ToTab::Tick { clock_ms: 32 }.encode()).unwrap();
        let msgs = recv_until(&rx, |m| matches!(m, ToShell::Frame { .. } | ToShell::Idle));
        assert!(
            msgs.iter().any(|m| matches!(
                m,
                ToShell::Log { module: false, message, .. }
                    if message.starts_with("clipboard_write: E_DENIED")
            )),
            "{msgs:?}"
        );
        assert!(!msgs.iter().any(|m| matches!(m, ToShell::Clipboard(_))));
        // With the gesture: written, and the write comes out.
        tx.send(ToTab::CopyGesture.encode()).unwrap();
        tx.send(key(crate::shell::hid::C, vy_abi::modifier::CTRL, 48).encode())
            .unwrap();
        tx.send(ToTab::Tick { clock_ms: 64 }.encode()).unwrap();
        let msgs = recv_until(&rx, |m| matches!(m, ToShell::Frame { .. } | ToShell::Idle));
        assert!(
            msgs.iter()
                .any(|m| matches!(m, ToShell::Clipboard(b) if b == b"kopia")),
            "{msgs:?}"
        );
        // The grant was for that one event: the next write is denied again.
        tx.send(key(crate::shell::hid::X, vy_abi::modifier::CTRL, 80).encode())
            .unwrap();
        tx.send(ToTab::Tick { clock_ms: 96 }.encode()).unwrap();
        let msgs = recv_until(&rx, |m| matches!(m, ToShell::Frame { .. } | ToShell::Idle));
        assert!(!msgs.iter().any(|m| matches!(m, ToShell::Clipboard(_))));
        assert_eq!(
            msgs.iter()
                .filter(|m| matches!(
                    m,
                    ToShell::Log { message, .. } if message.starts_with("clipboard_write: E_DENIED")
                ))
                .count(),
            1
        );
        // A gesture arms exactly the next event, whatever it is: a pointer
        // move spends it and the key after it is denied.
        tx.send(ToTab::CopyGesture.encode()).unwrap();
        tx.send(
            ToTab::Event {
                kind: vy_abi::event::POINTER_MOVE,
                a: 100,
                b: 100,
                c: 0,
                d: 0,
                clock_ms: 112,
            }
            .encode(),
        )
        .unwrap();
        tx.send(key(crate::shell::hid::C, vy_abi::modifier::CTRL, 128).encode())
            .unwrap();
        tx.send(ToTab::Tick { clock_ms: 144 }.encode()).unwrap();
        let msgs = recv_until(&rx, |m| matches!(m, ToShell::Frame { .. } | ToShell::Idle));
        assert_eq!(
            msgs.iter()
                .filter(|m| matches!(m, ToShell::Clipboard(_)))
                .count(),
            1,
            "the move wrote, the key did not: {msgs:?}"
        );
        assert!(msgs.iter().any(|m| matches!(
            m,
            ToShell::Log { message, .. } if message.starts_with("clipboard_write: E_DENIED")
        )));
        tx.send(ToTab::Shutdown.encode()).unwrap();
        recv_until(&rx, |m| matches!(m, ToShell::Storage(_)));
        h.join().unwrap();
    }

    /// `spawn_thread` wires the same loop to the shell's inbox.
    #[test]
    fn thread_tab_reports_to_the_inbox() {
        let (sink, inbox) = mpsc::channel::<TabEvent>();
        let conn = spawn_thread(7, channel_sink(sink)).unwrap();
        assert!(conn.send(&ToTab::Load(Box::new(load_request(module("hello"), 0)))));
        loop {
            let (id, ev) = inbox.recv_timeout(Duration::from_secs(20)).unwrap();
            assert_eq!(id, 7);
            match ev {
                Inbound::Message(ToShell::Loaded { ok, .. }) => {
                    assert!(ok);
                    break;
                }
                Inbound::Message(ToShell::Log { .. }) => {}
                other => panic!("{other:?}"),
            }
        }
        conn.shutdown();
        let mut gone = false;
        while let Ok((_, ev)) = inbox.recv_timeout(Duration::from_secs(5)) {
            if matches!(ev, Inbound::Gone { error: None }) {
                gone = true;
                break;
            }
        }
        assert!(gone);
    }
}
