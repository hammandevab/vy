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
//!
//! Recording and replay (SPEC-11.4.1) are this side's too, so a tab
//! process never opens a file and is locked down like every other
//! (SPEC-9.1.3): the writer thread of a recorded tab writes each frame it
//! sends the tab to the recording as well, and the writer thread of a
//! replaying tab feeds it the frames of a recording instead of the
//! shell's. The file holds exactly the frames the tab was sent.
//!
//! A tab the shell lets go of (a navigation, a closed tab, the window
//! closing) is sent `Shutdown` and handed to a [`Reaper`], which ends it
//! by its deadline on a thread of its own; the UI thread never waits for
//! a tab process to leave, and the answer to the shutdown (the page's
//! storage) arrives like any other message.

use crate::ipc::{read_frame, write_frame, ChannelTransport, ToShell, ToTab};
use crate::tab::TabLoop;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// How long a tab process let go of has to answer its `Shutdown` (with its
/// storage, SPEC-5.2.9) and leave before it is killed.
pub const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);
/// How often a waiting end is looked at.
const POLL: Duration = Duration::from_millis(5);

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
    /// A stand-in for tests ([`stalled`]).
    #[cfg(test)]
    Stalled(StalledProc),
}

impl Proc {
    fn id(&self) -> u32 {
        match self {
            Proc::Std(c, _) => c.id(),
            #[cfg(windows)]
            Proc::Strict(c) => c.id(),
            #[cfg(test)]
            Proc::Stalled(_) => 0,
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
            #[cfg(test)]
            Proc::Stalled(p) => p.kill(),
        }
    }

    fn try_wait(&mut self) -> io::Result<Option<String>> {
        match self {
            Proc::Std(c, _) => Ok(c.try_wait()?.map(|s| s.to_string())),
            #[cfg(windows)]
            Proc::Strict(c) => c.try_wait(),
            #[cfg(test)]
            Proc::Stalled(p) => Ok(p.killed().map(|_| "killed".into())),
        }
    }

    fn wait(&mut self) {
        match self {
            Proc::Std(c, _) => {
                let _ = c.wait();
            }
            #[cfg(windows)]
            Proc::Strict(c) => c.wait(),
            #[cfg(test)]
            Proc::Stalled(_) => {}
        }
    }

    #[cfg(windows)]
    fn job(&self) -> Option<&Job> {
        match self {
            Proc::Std(_, j) => j.as_ref(),
            Proc::Strict(c) => Some(&c.job),
            #[cfg(test)]
            Proc::Stalled(_) => None,
        }
    }
}

/// A tab process stand-in for tests: the shell's ends are real pipes, but
/// nothing reads what it is sent or answers, and it leaves only when it
/// is killed, which closes its ends and notes the instant.
#[cfg(test)]
pub(crate) struct StalledProc {
    ends: Option<(io::PipeReader, io::PipeWriter)>,
    killed: Arc<std::sync::Mutex<Option<Instant>>>,
}

#[cfg(test)]
impl StalledProc {
    fn kill(&mut self) {
        self.killed.lock().unwrap().get_or_insert_with(Instant::now);
        self.ends = None;
    }

    fn killed(&self) -> Option<Instant> {
        *self.killed.lock().unwrap()
    }
}

/// A connection to a [`StalledProc`] for tab `id`, and the cell that holds
/// the instant it is killed.
#[cfg(test)]
pub(crate) fn stalled(
    id: TabId,
    sink: impl Sink,
) -> io::Result<(TabConnection, Arc<std::sync::Mutex<Option<Instant>>>)> {
    let (child_in, shell_in) = io::pipe()?;
    let (shell_out, child_out) = io::pipe()?;
    let killed = Arc::new(std::sync::Mutex::new(None));
    let process = Proc::Stalled(StalledProc {
        ends: Some((child_in, child_out)),
        killed: killed.clone(),
    });
    let pipes = (
        process,
        Box::new(shell_in) as Box<dyn Write + Send>,
        Box::new(shell_out) as Box<dyn io::Read + Send>,
        crate::sandbox::Report::default(),
    );
    Ok((connect(id, sink, pipes, Source::Shell(None))?, killed))
}

/// What a process-backed tab's writer thread is told.
enum Out {
    /// A frame for the tab.
    Frame(Vec<u8>),
    /// Stop recording: the file ends with [`ToTab::StopRecording`].
    StopRecording,
}

/// Where [`TabConnection::send`] puts a message.
enum Feed {
    /// The writer thread of a process-backed tab.
    Writer(Sender<Out>),
    /// The tab loop's channel, for a thread-backed tab.
    Channel(Sender<Vec<u8>>),
    /// Nowhere: a replaying tab's inbound stream is its recording.
    Replay,
}

/// The main process's end of one tab.
pub struct TabConnection {
    id: TabId,
    /// What the main process put around the tab's process (SPEC-9.1.3).
    host_sandbox: crate::sandbox::Report,
    feed: Option<Feed>,
    kind: Kind,
    writer: Option<JoinHandle<()>>,
    reader: Option<JoinHandle<()>>,
}

/// Where `--tab` children are started from: this executable ([`self_exe`]).
pub fn tab_executable() -> io::Result<PathBuf> {
    self_exe()
}

/// A path that starts the image this process is running, even after the
/// file it was started from has been replaced on disk (an update while
/// the client runs): tab processes must speak the same IPC as their shell.
///
/// On Linux that is `/proc/self/exe`: the child execs it before it has its
/// own image, so it names the parent's inode, whatever the path now holds.
/// Windows does not let a running executable be replaced (it can only be
/// renamed, and the rename keeps the image), so `current_exe` is the same
/// file. On macOS `current_exe` is the path, and a replaced file there
/// would start the new binary: a residual risk until tabs are started
/// another way.
pub fn self_exe() -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let proc_exe = Path::new("/proc/self/exe");
        if proc_exe.exists() {
            return Ok(proc_exe.to_path_buf());
        }
    }
    std::env::current_exe()
}

/// Start a tab process from `exe` (`exe --tab`), delivering its messages
/// to `sink` as `(id, ..)`. The child locks itself down before it reads
/// anything (SPEC-9.1.3, [`crate::sandbox`]); on Windows the shell also
/// puts it in a job object, or with `VY_SANDBOX=strict` starts it in an
/// AppContainer, falling back to the default with a message when that
/// cannot be done.
///
/// With `record`, every frame sent to the tab is also written to that
/// file, which is created here (SPEC-11.4.1). With `replay`, the tab is
/// fed the frames of that recording, read here, and then its input ends;
/// what the shell sends it is dropped. Either way the child is the same
/// locked-down `exe --tab`: it never sees the file.
pub fn spawn_process(
    exe: &Path,
    id: TabId,
    sink: impl Sink,
    record: Option<&Path>,
    replay: Option<&Path>,
) -> io::Result<TabConnection> {
    let source = match (record, replay) {
        (_, Some(path)) => Source::Replay(BufReader::new(File::open(path)?)),
        (Some(path), None) => Source::Shell(Some(BufWriter::new(File::create(path)?))),
        (None, None) => Source::Shell(None),
    };
    let (process, stdin, stdout, host) = start(exe, &["--tab".into()])?;
    connect(id, sink, (process, stdin, stdout, host), source)
}

/// Start `exe --tab --sandbox-probe` exactly as a tab is started (job
/// object, strict mode) and return what the probe reports (SPEC-9.1.3).
/// A recorded or replayed tab is started the same way ([`spawn_process`]).
pub fn probe_process(exe: &Path) -> io::Result<String> {
    use std::io::Read;
    let args = ["--tab".into(), "--sandbox-probe".into()];
    let (mut process, stdin, mut stdout, _) = start(exe, &args)?;
    drop(stdin);
    let mut text = String::new();
    stdout.read_to_string(&mut text)?;
    process.wait();
    Ok(text)
}

/// The child, the shell's ends of its pipes, and what the shell itself put
/// around the child (SPEC-9.1.3).
type Pipes = (
    Proc,
    Box<dyn Write + Send>,
    Box<dyn io::Read + Send>,
    crate::sandbox::Report,
);

/// Start a tab child with `args`: on Windows with `VY_SANDBOX=strict` in
/// an AppContainer, falling back with a message; otherwise through std
/// and, on Windows, into a job object. The report names those layers and
/// any that failed (nothing outside Windows).
fn start(exe: &Path, args: &[std::ffi::OsString]) -> io::Result<Pipes> {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut host = crate::sandbox::Report::default();
    #[cfg(windows)]
    if crate::sandbox::Mode::from_env() == crate::sandbox::Mode::Strict {
        let tail: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        match crate::sandbox::spawn_strict(exe, &tail.join(" ")) {
            Ok((child, stdin, stdout)) => {
                host.layers.push("AppContainer".into());
                host.layers.push("job object".into());
                return Ok((Proc::Strict(child), Box::new(stdin), Box::new(stdout), host));
            }
            Err(e) => {
                eprintln!(
                    "vy-shell: VY_SANDBOX=strict: {e}; this tab starts with the default sandbox"
                );
                host.warnings.push(format!("AppContainer: {e}"));
            }
        }
    }
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
        Ok(job) => {
            host.layers.push("job object".into());
            Some(job)
        }
        Err(e) => {
            eprintln!("vy-shell: a tab runs without a job object: {e}");
            host.warnings.push(format!("job object: {e}"));
            None
        }
    };
    #[cfg(not(windows))]
    let job = None;
    Ok((
        Proc::Std(child, job),
        Box::new(stdin),
        Box::new(stdout),
        host,
    ))
}

/// What a process-backed tab's writer thread feeds it.
enum Source {
    /// What the shell sends, also written to the recording when there is one.
    Shell(Option<BufWriter<File>>),
    /// The frames of a recording, then the end of the input.
    Replay(BufReader<File>),
}

/// The writer thread's loop for [`Source::Shell`]: each frame to the tab
/// and, once it is written there, to the recording. The tab loop ends at
/// `Shutdown`, so the recording does too: it holds what the tab read.
fn write_from_shell(
    w: &mut impl Write,
    rx: &Receiver<Out>,
    mut record: Option<BufWriter<File>>,
) -> io::Result<()> {
    let shutdown = ToTab::Shutdown.encode();
    while let Ok(out) = rx.recv() {
        match out {
            Out::Frame(body) => {
                write_frame(w, &body)?;
                if let Some(rec) = &mut record {
                    if let Err(e) = write_frame(rec, &body) {
                        eprintln!("vy-shell: recording stopped: {e}");
                        record = None;
                    } else if body == shutdown {
                        record = None;
                    }
                }
            }
            Out::StopRecording => {
                if let Some(mut rec) = record.take() {
                    if let Err(e) = write_frame(&mut rec, &ToTab::StopRecording.encode()) {
                        eprintln!("vy-shell: recording not completed: {e}");
                    }
                }
            }
        }
    }
    Ok(())
}

/// The writer and reader threads of a process-backed tab.
fn connect(id: TabId, sink: impl Sink, pipes: Pipes, source: Source) -> io::Result<TabConnection> {
    let (process, stdin, stdout, host_sandbox) = pipes;
    let (tx, rx) = mpsc::channel::<Out>();
    let feed = match &source {
        Source::Shell(_) => Feed::Writer(tx),
        Source::Replay(_) => Feed::Replay,
    };
    let writer = thread::Builder::new()
        .name(format!("tab-{id}-writer"))
        .spawn(move || {
            let mut w = BufWriter::new(stdin);
            let _ = match source {
                Source::Shell(record) => write_from_shell(&mut w, &rx, record),
                // The whole recording, frame by frame; a frame that does
                // not read ends the input there, as the end of the file does.
                Source::Replay(mut file) => {
                    loop {
                        match read_frame(&mut file) {
                            Ok(Some(body)) => {
                                if write_frame(&mut w, &body).is_err() {
                                    break;
                                }
                            }
                            Ok(None) => break,
                            Err(e) => {
                                eprintln!("vy-shell: replay ends early: {e}");
                                break;
                            }
                        }
                    }
                    Ok(())
                }
            };
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
        host_sandbox,
        feed: Some(feed),
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
            let _ = TabLoop::new(transport).run();
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
        host_sandbox: crate::sandbox::Report::default(),
        feed: Some(Feed::Channel(tx)),
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

    /// Queue a message for the tab; false once the tab is gone, and for a
    /// replaying tab, whose input is its recording.
    pub fn send(&self, msg: &ToTab) -> bool {
        match &self.feed {
            Some(Feed::Writer(tx)) => tx.send(Out::Frame(msg.encode())).is_ok(),
            Some(Feed::Channel(tx)) => tx.send(msg.encode()).is_ok(),
            Some(Feed::Replay) | None => false,
        }
    }

    /// Stop the recording `spawn_process` was given, after every frame
    /// sent before this call: the file is then complete and ends with
    /// [`ToTab::StopRecording`]. Nothing for a tab that does not record.
    pub fn stop_recording(&self) {
        if let Some(Feed::Writer(tx)) = &self.feed {
            let _ = tx.send(Out::StopRecording);
        }
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

    /// What the main process put around the tab's process and what of it
    /// failed (SPEC-9.1.3): on Windows the job object, or the AppContainer
    /// in strict mode; nothing elsewhere, and nothing for a thread. The
    /// process reports its own lockdown itself (`ToShell::Sandbox`).
    pub fn host_sandbox(&self) -> &crate::sandbox::Report {
        &self.host_sandbox
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

    /// Ask the tab to leave: [`ToTab::Shutdown`], then the end of its
    /// input. Its answer (the page's storage) still comes through the
    /// reader.
    fn ask_to_leave(&mut self) {
        let _ = self.send(&ToTab::Shutdown);
        self.feed = None;
    }

    /// True once the tab has ended: its process exited, or its thread
    /// returned.
    fn has_ended(&mut self) -> bool {
        match &mut self.kind {
            Kind::Process(c) => !matches!(c.try_wait(), Ok(None)),
            Kind::Thread(h) => h.as_ref().is_none_or(JoinHandle::is_finished),
        }
    }

    /// End the tab now: kill a process that still runs (a thread cannot
    /// be, so it is waited for), then wait for the writer and the reader,
    /// so everything the tab sent has been delivered.
    fn end(&mut self) {
        match &mut self.kind {
            Kind::Process(c) => {
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

    /// Ask the tab to leave, wait up to [`SHUTDOWN_GRACE`], then make sure
    /// it has; everything it sent has been delivered when this returns.
    /// This waits on the calling thread: the shell lets its tabs go through
    /// a [`Reaper`] instead.
    pub fn shutdown(mut self) {
        self.ask_to_leave();
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        while !self.has_ended() && Instant::now() < deadline {
            thread::sleep(POLL);
        }
        self.end();
    }
}

/// Tab connections being let go of, ended on a thread of their own so the
/// caller never waits (SPEC-5.2.9). Each is sent `Shutdown` when it is
/// handed over and has until its deadline to leave; then it is killed.
/// Their readers keep delivering meanwhile, so a page's storage answer
/// reaches the sink whenever it comes, before its process ends. Many let
/// go of at once with one deadline take that one deadline in all.
pub struct Reaper {
    tx: Option<Sender<(TabConnection, Instant)>>,
    pending: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl Reaper {
    /// A reaper and its thread.
    pub fn new() -> io::Result<Reaper> {
        let (tx, rx) = mpsc::channel();
        let pending = Arc::new(AtomicUsize::new(0));
        let count = pending.clone();
        let thread = thread::Builder::new()
            .name("tab-reaper".into())
            .spawn(move || reap(&rx, &count))?;
        Ok(Reaper {
            tx: Some(tx),
            pending,
            thread: Some(thread),
        })
    }

    /// Send `conn` its `Shutdown` now and have it ended by `deadline`.
    /// Returns at once.
    pub fn retire(&self, mut conn: TabConnection, deadline: Instant) {
        conn.ask_to_leave();
        self.pending.fetch_add(1, Ordering::SeqCst);
        if let Some(tx) = &self.tx {
            if let Err(mpsc::SendError((mut conn, _))) = tx.send((conn, deadline)) {
                conn.end();
                self.pending.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    /// Connections handed over that have not ended yet: their answers may
    /// still come. Zero means everything they sent has been delivered.
    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }

    /// Wait until every connection handed over has ended, each by its
    /// deadline, and stop the thread.
    pub fn finish(&mut self) {
        self.tx = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Reaper {
    /// Without [`Reaper::finish`] the thread still ends what it holds, at
    /// their deadlines, on its own.
    fn drop(&mut self) {
        self.tx = None;
    }
}

/// The reaper's thread: take connections as they come, end each once it
/// has ended by itself or its deadline has passed, until no more can come
/// and none is left.
fn reap(rx: &Receiver<(TabConnection, Instant)>, pending: &AtomicUsize) {
    let mut live: Vec<(TabConnection, Instant)> = Vec::new();
    let mut open = true;
    while open || !live.is_empty() {
        if live.is_empty() {
            match rx.recv() {
                Ok(c) => live.push(c),
                Err(_) => open = false,
            }
        } else if open {
            match rx.recv_timeout(POLL) {
                Ok(c) => live.push(c),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => open = false,
            }
        } else {
            thread::sleep(POLL);
        }
        live.extend(rx.try_iter());
        let now = Instant::now();
        live.retain_mut(|(conn, deadline)| {
            if conn.has_ended() || now >= *deadline {
                conn.end();
                pending.fetch_sub(1, Ordering::SeqCst);
                false
            } else {
                true
            }
        });
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    const STAGE: &str = "VY_SELF_EXE_STAGE";
    const MARK: &str = "self-exe: original image";

    /// The child half of `spec_9_1_1_tabs_start_the_running_image`: a
    /// no-op in the normal run. As stage 1 it waits for the parent's
    /// signal (its own file has been replaced by then) and starts
    /// [`self_exe`] as stage 2, which prints the mark only the original
    /// test binary knows.
    #[test]
    fn self_exe_child_stage() {
        match std::env::var(STAGE).as_deref() {
            Ok("1") => {
                let go = PathBuf::from(std::env::var("VY_SELF_EXE_GO").unwrap());
                let start = Instant::now();
                while !go.exists() && start.elapsed() < Duration::from_secs(20) {
                    thread::sleep(Duration::from_millis(10));
                }
                let out = Command::new(self_exe().unwrap())
                    .args([
                        "--exact",
                        "process::tests::self_exe_child_stage",
                        "--nocapture",
                    ])
                    .env(STAGE, "2")
                    .output()
                    .unwrap();
                print!("{}", String::from_utf8_lossy(&out.stdout));
            }
            Ok("2") => println!("{MARK}"),
            _ => {}
        }
    }

    /// SPEC-9.1.1, SPEC-11.2.2: a tab process is the shell's own image.
    /// A copy of this test binary is started from a temporary path, the
    /// file at that path is replaced by a script while it runs, and the
    /// child it then starts through [`self_exe`] is still the original
    /// image, not the script.
    #[test]
    fn spec_9_1_1_tabs_start_the_running_image() {
        assert_eq!(self_exe().unwrap(), Path::new("/proc/self/exe"));
        let dir = std::env::temp_dir().join(format!("vy-self-exe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("shell");
        std::fs::copy(std::env::current_exe().unwrap(), &copy).unwrap();
        let go = dir.join("go");
        let child = Command::new(&copy)
            .args([
                "--exact",
                "process::tests::self_exe_child_stage",
                "--nocapture",
            ])
            .env(STAGE, "1")
            .env("VY_SELF_EXE_GO", &go)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        // The update: a different file takes the path while the copy runs.
        let script = dir.join("next");
        std::fs::write(&script, "#!/bin/sh\necho replaced\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(&script, &copy).unwrap();
        std::fs::write(&go, b"").unwrap();
        let out = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains(MARK), "{text}");
        assert!(!text.contains("replaced"), "{text}");
        let direct = Command::new(&copy).output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&direct.stdout),
            "replaced\n",
            "the path did change"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
