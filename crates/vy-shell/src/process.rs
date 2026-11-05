//! Tab processes as the main process holds them (SPEC-9.1.1, SPEC-11.2.2).
//!
//! A tab is the same executable started with `--tab`, talking frames over
//! its stdin and stdout. The main process keeps, per tab, a writer thread
//! that feeds the child's stdin from a channel (so the UI thread never
//! blocks on a slow tab) and a reader thread that decodes the child's
//! stdout into [`Inbound`] events on the shell's one inbox. A child that
//! dies, crashes or is killed is just an end of its stdout: the reader
//! reports [`Inbound::Gone`], the shell shows an error page in that tab,
//! and nothing else notices.
//!
//! For tests and the headless driver the same loop can run on a thread in
//! this process over an in-memory channel ([`spawn_thread`]); the shell
//! code is the same either way.

use crate::ipc::{read_frame, write_frame, ChannelTransport, ToShell, ToTab};
use crate::tab::TabLoop;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// A tab's number in the shell, stable for its life.
pub type TabId = u32;

/// What a tab's reader reports to the shell.
#[derive(Debug)]
pub enum Inbound {
    /// A decoded message.
    Message(ToShell),
    /// The tab's stream ended: it left, crashed or was killed; or it sent
    /// a frame that does not decode, which counts the same.
    Gone {
        /// Why, when it is not a clean end.
        error: Option<String>,
    },
}

/// A message to the shell from one tab.
pub type TabEvent = (TabId, Inbound);

/// Where a tab's reader delivers: returns false once the shell is gone.
pub trait Sink: Fn(TabId, Inbound) -> bool + Send + 'static {}
impl<F: Fn(TabId, Inbound) -> bool + Send + 'static> Sink for F {}

enum Kind {
    Process(Child),
    Thread(Option<JoinHandle<()>>),
}

/// The main process's end of one tab.
pub struct TabConnection {
    id: TabId,
    tx: Option<Sender<Vec<u8>>>,
    kind: Kind,
    writer: Option<JoinHandle<()>>,
    reader: Option<JoinHandle<()>>,
}

/// Where `--tab` children are started from: this executable.
pub fn tab_executable() -> io::Result<PathBuf> {
    std::env::current_exe()
}

/// Start a tab process from `exe` (`exe --tab`), delivering its messages
/// to `sink` as `(id, ..)`. `record` and `replay` are passed on as the
/// child's `--record`/`--replay` files (SPEC-11.4.1).
pub fn spawn_process(
    exe: &Path,
    id: TabId,
    sink: impl Sink,
    record: Option<&Path>,
    replay: Option<&Path>,
) -> io::Result<TabConnection> {
    let mut cmd = Command::new(exe);
    cmd.arg("--tab");
    if let Some(r) = record {
        cmd.arg("--record").arg(r);
    }
    if let Some(r) = replay {
        cmd.arg("--replay").arg(r);
    }
    // The child's stderr (panics) goes where the shell's goes; a Windows
    // release build is a GUI program without one, so nowhere.
    let stderr = if cfg!(all(windows, not(debug_assertions))) {
        Stdio::null()
    } else {
        Stdio::inherit()
    };
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()?;
    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let writer = thread::Builder::new()
        .name(format!("tab-{id}-writer"))
        .spawn(move || {
            let mut w = BufWriter::new(stdin);
            while let Ok(body) = rx.recv() {
                if write_frame(&mut w, &body).is_err() {
                    break;
                }
            }
            let _ = w.flush();
        })?;
    let reader = thread::Builder::new()
        .name(format!("tab-{id}-reader"))
        .spawn(move || {
            let mut r = BufReader::new(stdout);
            let error = loop {
                match read_frame(&mut r) {
                    Ok(Some(body)) => match ToShell::decode(&body) {
                        Ok(m) => {
                            if !sink(id, Inbound::Message(m)) {
                                return;
                            }
                        }
                        Err(e) => break Some(format!("undecodable message from the tab: {e}")),
                    },
                    Ok(None) => break None,
                    Err(e) => break Some(e.to_string()),
                }
            };
            sink(id, Inbound::Gone { error });
        })?;
    Ok(TabConnection {
        id,
        tx: Some(tx),
        kind: Kind::Process(child),
        writer: Some(writer),
        reader: Some(reader),
    })
}

/// Run the tab loop on a thread of this process over an in-memory
/// channel: the same loop, no process boundary. For tests and tools.
pub fn spawn_thread(id: TabId, sink: impl Sink) -> io::Result<TabConnection> {
    let (tx, to_tab) = mpsc::channel::<Vec<u8>>();
    let (from_tab, rx) = mpsc::channel::<Vec<u8>>();
    let transport = ChannelTransport {
        tx: from_tab,
        rx: to_tab,
    };
    let worker = thread::Builder::new()
        .name(format!("tab-{id}"))
        .spawn(move || {
            if let Ok(mut l) = TabLoop::new(transport, None) {
                let _ = l.run();
            }
        })?;
    let reader = thread::Builder::new()
        .name(format!("tab-{id}-reader"))
        .spawn(move || {
            let error = loop {
                match rx.recv() {
                    Ok(body) => match ToShell::decode(&body) {
                        Ok(m) => {
                            if !sink(id, Inbound::Message(m)) {
                                return;
                            }
                        }
                        Err(e) => break Some(format!("undecodable message from the tab: {e}")),
                    },
                    Err(_) => break None,
                }
            };
            sink(id, Inbound::Gone { error });
        })?;
    Ok(TabConnection {
        id,
        tx: Some(tx),
        kind: Kind::Thread(Some(worker)),
        writer: None,
        reader: Some(reader),
    })
}

impl TabConnection {
    /// The tab's number.
    pub fn id(&self) -> TabId {
        self.id
    }

    /// Queue a message for the tab; false once the tab is gone.
    pub fn send(&self, msg: &ToTab) -> bool {
        self.tx
            .as_ref()
            .is_some_and(|tx| tx.send(msg.encode()).is_ok())
    }

    /// The tab's process id, for a process-backed tab.
    pub fn pid(&self) -> Option<u32> {
        match &self.kind {
            Kind::Process(c) => Some(c.id()),
            Kind::Thread(_) => None,
        }
    }

    /// True when the tab runs in its own process.
    pub fn is_process(&self) -> bool {
        matches!(self.kind, Kind::Process(_))
    }

    /// Kill the tab outright (a test of crash containment, or a tab that
    /// does not answer a shutdown).
    pub fn kill(&mut self) {
        if let Kind::Process(c) = &mut self.kind {
            let _ = c.kill();
        }
    }

    /// The exit status once the process has ended, as a short text;
    /// `None` while it runs or for a thread-backed tab. Waits up to
    /// `timeout` for the exit.
    pub fn exit_status(&mut self, timeout: Duration) -> Option<String> {
        let Kind::Process(c) = &mut self.kind else {
            return None;
        };
        let start = Instant::now();
        loop {
            match c.try_wait() {
                Ok(Some(status)) => return Some(status.to_string()),
                Ok(None) if start.elapsed() < timeout => thread::sleep(Duration::from_millis(5)),
                Ok(None) => return None,
                Err(e) => return Some(e.to_string()),
            }
        }
    }

    /// Ask the tab to leave, wait briefly, then make sure it has.
    pub fn shutdown(mut self) {
        let _ = self.send(&ToTab::Shutdown);
        self.tx = None;
        match &mut self.kind {
            Kind::Process(c) => {
                let start = Instant::now();
                while start.elapsed() < Duration::from_millis(500) {
                    if matches!(c.try_wait(), Ok(Some(_))) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                let _ = c.kill();
                let _ = c.wait();
            }
            Kind::Thread(h) => {
                if let Some(h) = h.take() {
                    let _ = h.join();
                }
            }
        }
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}

impl Drop for TabConnection {
    fn drop(&mut self) {
        if let Kind::Process(c) = &mut self.kind {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// A sink that sends to a channel.
pub fn channel_sink(tx: std::sync::mpsc::Sender<TabEvent>) -> impl Sink {
    move |id, inbound| tx.send((id, inbound)).is_ok()
}

/// Receive the next event for `id` within `timeout`, passing over events
/// of other tabs to `others`. For drivers that wait for one tab's answer.
pub fn wait_for(
    rx: &Receiver<TabEvent>,
    id: TabId,
    timeout: Duration,
    others: &mut Vec<TabEvent>,
) -> Option<Inbound> {
    let deadline = Instant::now() + timeout;
    loop {
        let now = Instant::now();
        if now >= deadline {
            return None;
        }
        match rx.recv_timeout(deadline - now) {
            Ok((tab, inbound)) if tab == id => return Some(inbound),
            Ok(other) => others.push(other),
            Err(_) => return None,
        }
    }
}
