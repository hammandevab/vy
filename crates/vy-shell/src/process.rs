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
    Process(Proc),
    Thread(Option<JoinHandle<()>>),
}

/// The job a tab process runs in on Windows (SPEC-9.1.3); nothing elsewhere.
#[cfg(windows)]
type Job = crate::sandbox::TabJob;
#[cfg(not(windows))]
type Job = ();

/// A tab process: started by std, or on Windows with `VY_SANDBOX=strict`
/// in an AppContainer ([`crate::sandbox::spawn_strict`]).
enum Proc {
    Std(
        Child,
        #[cfg_attr(not(windows), allow(dead_code))] Option<Job>,
    ),
    #[cfg(windows)]
    Strict(crate::sandbox::StrictChild),
}

impl Proc {
    fn id(&self) -> u32 {
        match self {
            Proc::Std(c, _) => c.id(),
            #[cfg(windows)]
            Proc::Strict(c) => c.id(),
        }
    }

    fn kill(&mut self) {
        match self {
            Proc::Std(c, _) => {
                let _ = c.kill();
            }
            #[cfg(windows)]
            Proc::Strict(c) => {
                let _ = c.kill();
            }
        }
    }

    fn try_wait(&mut self) -> io::Result<Option<String>> {
        match self {
            Proc::Std(c, _) => Ok(c.try_wait()?.map(|s| s.to_string())),
            #[cfg(windows)]
            Proc::Strict(c) => c.try_wait(),
        }
    }

    fn wait(&mut self) {
        match self {
            Proc::Std(c, _) => {
                let _ = c.wait();
            }
            #[cfg(windows)]
            Proc::Strict(c) => c.wait(),
        }
    }

    #[cfg(windows)]
    fn job(&self) -> Option<&Job> {
        match self {
            Proc::Std(_, j) => j.as_ref(),
            Proc::Strict(c) => Some(&c.job),
        }
    }
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
/// child's `--record`/`--replay` files (SPEC-11.4.1). The child locks
/// itself down before it reads anything (SPEC-9.1.3, [`crate::sandbox`]);
/// on Windows the shell also puts it in a job object, or with
/// `VY_SANDBOX=strict` starts it in an AppContainer, falling back to the
/// default with a message when that cannot be done.
pub fn spawn_process(
    exe: &Path,
    id: TabId,
    sink: impl Sink,
    record: Option<&Path>,
    replay: Option<&Path>,
) -> io::Result<TabConnection> {
    let mut args = vec![std::ffi::OsString::from("--tab")];
    if let Some(r) = record {
        args.push("--record".into());
        args.push(r.into());
    }
    if let Some(r) = replay {
        args.push("--replay".into());
        args.push(r.into());
    }
    let (process, stdin, stdout) = start(exe, &args, record.is_none() && replay.is_none())?;
    connect(id, sink, process, stdin, stdout)
}

/// Start `exe --tab --sandbox-probe` exactly as a tab is started (job
/// object, strict mode) and return what the probe reports (SPEC-9.1.3).
pub fn probe_process(exe: &Path) -> io::Result<String> {
    use std::io::Read;
    let args = ["--tab".into(), "--sandbox-probe".into()];
    let (mut process, stdin, mut stdout) = start(exe, &args, true)?;
    drop(stdin);
    let mut text = String::new();
    stdout.read_to_string(&mut text)?;
    process.wait();
    Ok(text)
}

type Pipes = (Proc, Box<dyn Write + Send>, Box<dyn io::Read + Send>);

/// Start a tab child with `args`: on Windows with `VY_SANDBOX=strict`
/// (when `strict_ok`) in an AppContainer, falling back with a message;
/// otherwise through std and, on Windows, into a job object.
fn start(exe: &Path, args: &[std::ffi::OsString], strict_ok: bool) -> io::Result<Pipes> {
    #[cfg(windows)]
    if strict_ok && crate::sandbox::Mode::from_env() == crate::sandbox::Mode::Strict {
        let tail: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        match crate::sandbox::spawn_strict(exe, &tail.join(" ")) {
            Ok((child, stdin, stdout)) => {
                return Ok((Proc::Strict(child), Box::new(stdin), Box::new(stdout)))
            }
            Err(e) => eprintln!(
                "vy-shell: VY_SANDBOX=strict: {e}; this tab starts with the default sandbox"
            ),
        }
    }
    let _ = strict_ok;
    let mut cmd = Command::new(exe);
    cmd.args(args);
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
    #[cfg(windows)]
    let job = match crate::sandbox::contain(&child) {
        Ok(job) => Some(job),
        Err(e) => {
            eprintln!("vy-shell: a tab runs without a job object: {e}");
            None
        }
    };
    #[cfg(not(windows))]
    let job = None;
    Ok((Proc::Std(child, job), Box::new(stdin), Box::new(stdout)))
}

/// The writer and reader threads of a process-backed tab.
fn connect(
    id: TabId,
    sink: impl Sink,
    process: Proc,
    stdin: impl Write + Send + 'static,
    stdout: impl io::Read + Send + 'static,
) -> io::Result<TabConnection> {
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
        kind: Kind::Process(process),
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

    /// Bound the tab process's memory to `memory_max_pages` of linear
    /// memory plus headroom once the page is known (SPEC-9.3.1): the job's
    /// commit limit on Windows; elsewhere wasmtime's own limit is the
    /// bound and this does nothing.
    pub fn limit_memory(&self, memory_max_pages: u32) -> io::Result<()> {
        #[cfg(windows)]
        if let Kind::Process(p) = &self.kind {
            if let Some(job) = p.job() {
                return job.limit_memory(memory_max_pages);
            }
        }
        let _ = memory_max_pages;
        Ok(())
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
            c.kill();
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
                Ok(Some(status)) => return Some(status),
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
                c.kill();
                c.wait();
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
            c.kill();
            c.wait();
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
