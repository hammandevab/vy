//! The main process's side of the compilation cache (SPEC-13.1.4,
//! SPEC-9.4.2).
//!
//! A tab process has no file system (SPEC-9.1.3), so the cache is here:
//! before a load the shell looks the module up ([`CodeCache::offer`]) and
//! sends what it finds ahead of the load; the tab takes the module from
//! that code or compiles and hands its code back, and says which, once,
//! before the page runs ([`CodeCache::settle`]).
//!
//! The rule that makes this safe to keep (docs/notes/m5-polish.md,
//! "Hotbild"): the key of an entry is computed here, from the module
//! bytes this process is about to send the tab, and carried here from the
//! offer to the report. Nothing a tab says names a key, a hash or a path,
//! so the code a tab hands over can only become the entry of the module
//! that tab was loaded with. The bytes are never deserialized in this
//! process.
//!
//! Lookups are a hash and a file read and run where they are asked for;
//! writes, removals and the eviction that follows a write run on a thread
//! of the cache's own, in the order they were asked for.

use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread;
use vy_runtime::{CodeKey, CodeStore};

/// What the main process holds for one load while the cache is on for
/// it: the key it made for the module and whether it offered code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    key: CodeKey,
    module_len: usize,
    /// The code the store held for the module, to send ahead of the load.
    pub code: Option<Vec<u8>>,
}

/// The same once the code has been sent: what a report is settled against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    key: CodeKey,
    module_len: usize,
    offered: bool,
}

impl Offer {
    /// Split into the code for the tab and what the main process keeps.
    pub fn send(self) -> (Option<Vec<u8>>, Pending) {
        let pending = Pending {
            key: self.key,
            module_len: self.module_len,
            offered: self.code.is_some(),
        };
        (self.code, pending)
    }
}

impl Pending {
    /// True when code was sent ahead of the load.
    pub fn offered(&self) -> bool {
        self.offered
    }
}

/// What became of a tab's report ([`CodeCache::settle`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Settled {
    /// The tab used the code it was offered.
    Hit,
    /// The tab compiled and its code is being kept.
    Kept,
    /// The tab compiled and handed over nothing that can be kept.
    Compiled,
    /// The tab did not use the code it was offered: the entry is dropped,
    /// and replaced when the tab handed over code of its own.
    Refused,
    /// The report cannot be true (a hit without an offer); nothing is done.
    Ignored,
}

enum Op {
    Put(CodeKey, usize, Vec<u8>),
    Remove(CodeKey),
    Clear(Sender<usize>),
    Sync(Sender<()>),
}

/// The compilation cache as the shell uses it.
pub struct CodeCache {
    store: CodeStore,
    ops: Sender<Op>,
}

impl CodeCache {
    /// The cache at `dir`, or `None` when there is to be none: the
    /// directory cannot be made or is not the user's own
    /// ([`CodeStore::open`]), or the writer thread cannot start.
    pub fn open(dir: &Path) -> Option<CodeCache> {
        Self::over(CodeStore::open(dir)?)
    }

    /// The cache over an opened store.
    pub fn over(store: CodeStore) -> Option<CodeCache> {
        let (ops, rx) = mpsc::channel::<Op>();
        let writer = store.clone();
        thread::Builder::new()
            .name("vy-code-cache".into())
            .spawn(move || {
                // The first key builds this process's wasmtime engine, for
                // its settings; done here so the first page load need not.
                let _ = CodeKey::of(&[]);
                while let Ok(op) = rx.recv() {
                    match op {
                        Op::Put(key, module_len, code) => {
                            if let Err(e) = writer.put(&key, module_len, &code) {
                                eprintln!("vy-shell: compiled code not kept: {e}");
                            }
                        }
                        Op::Remove(key) => writer.remove(&key),
                        Op::Clear(done) => {
                            let _ = done.send(writer.clear());
                        }
                        Op::Sync(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })
            .ok()?;
        Some(CodeCache { store, ops })
    }

    /// The store underneath.
    pub fn store(&self) -> &CodeStore {
        &self.store
    }

    /// Look up the module this process is about to load in a tab: the key
    /// is made from `wasm`, the bytes that will be sent.
    pub fn offer(&self, wasm: &[u8]) -> Offer {
        let key = CodeKey::of(wasm);
        Offer {
            code: self.store.get(&key),
            key,
            module_len: wasm.len(),
        }
    }

    /// Act on what the tab reported for the load `pending` belongs to:
    /// `cached` when it used the offered code, otherwise `code` is what it
    /// compiled. The key is `pending`'s, whatever the tab was really
    /// doing; code over the store's caps is not kept.
    pub fn settle(&self, pending: Pending, cached: bool, code: Vec<u8>) -> Settled {
        match (cached, pending.offered) {
            (true, true) => Settled::Hit,
            (true, false) => Settled::Ignored,
            (false, offered) => {
                if offered {
                    self.forget(&pending);
                }
                let keep = !code.is_empty();
                if keep {
                    let _ = self
                        .ops
                        .send(Op::Put(pending.key, pending.module_len, code));
                }
                match (offered, keep) {
                    (true, _) => Settled::Refused,
                    (false, true) => Settled::Kept,
                    (false, false) => Settled::Compiled,
                }
            }
        }
    }

    /// Drop the entry of `pending`'s module: the code offered from it was
    /// refused, or the process that was running it died.
    pub fn forget(&self, pending: &Pending) {
        let _ = self.ops.send(Op::Remove(pending.key.clone()));
    }

    /// Remove every entry (devtools, "Rensa cache"), after the writes and
    /// removals asked for so far, and say how many went. Waits for it:
    /// when this returns the cache is empty.
    pub fn clear(&self) -> usize {
        let (done, wait) = mpsc::channel();
        if self.ops.send(Op::Clear(done)).is_err() {
            return 0;
        }
        wait.recv().unwrap_or(0)
    }

    /// Wait until every write and removal asked for so far is done.
    pub fn flush(&self) {
        let (done, wait) = mpsc::channel();
        if self.ops.send(Op::Sync(done)).is_ok() {
            let _ = wait.recv();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::MemoryClipboard;
    use crate::external::RecordingOpener;
    use crate::headless::settle;
    use crate::ipc::{read_frame, write_frame, ToShell, ToTab};
    use crate::page::{LoadedPage, Source};
    use crate::process::{Inbound, TabId};
    use crate::shell::{Config, Incoming, Shell, Waker};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use vy_runtime::{CompileCache, Options, Runtime};

    const TIMEOUT: Duration = Duration::from_secs(60);

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    /// `target/wasm/<name>.wasm`, built by `tools/build_modules.sh`.
    fn module(name: &str) -> Vec<u8> {
        let path = root().join(format!("target/wasm/{name}.wasm"));
        std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}\nrun tools/build_modules.sh before cargo test",
                path.display()
            )
        })
    }

    fn local(name: &str) -> Source {
        Source::File {
            path: root().join(format!("target/wasm/{name}.wasm")),
            draft: None,
        }
    }

    /// The built `vy-shell` binary beside the test binary (`tests/bin.rs`
    /// makes cargo build it).
    fn shell_exe() -> PathBuf {
        let mut dir = std::env::current_exe().unwrap();
        dir.pop();
        if dir.ends_with("deps") {
            dir.pop();
        }
        let exe = dir.join(format!("vy-shell{}", std::env::consts::EXE_SUFFIX));
        assert!(exe.is_file(), "{}: cargo build -p vy-shell", exe.display());
        exe
    }

    /// A directory of the test's own.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("vy-codecache-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A deterministic shell (the headless clock and random source) with
    /// the compilation cache at `cache`, tabs in processes of `exe` or on
    /// threads.
    fn shell(exe: Option<PathBuf>, data: &Path, cache: Option<&Path>) -> Shell {
        Shell::new(
            Config {
                exe,
                data_dir: data.join("data"),
                headless: true,
                opener: Box::new(RecordingOpener::default()),
                clipboard: Box::new(MemoryClipboard::new()),
                waker: Waker::none(),
                code_cache: cache.map(Path::to_path_buf),
            },
            800.0,
            600.0,
            1.0,
        )
        .unwrap()
    }

    /// A page of a bare module, as a local file gives it.
    fn page(wasm: Vec<u8>) -> LoadedPage {
        LoadedPage {
            display: "module.wasm".into(),
            origin: None,
            manifest: None,
            sha256: vy_runtime::sha256(&wasm),
            wasm,
            resources: Vec::new(),
            trusted: false,
            restriction: None,
            verdicts: Vec::new(),
            is_localhost: false,
            net_class: vy_proto::NetClass::Public,
        }
    }

    /// What a tab hands over after compiling `wasm`.
    fn compiled(wasm: &[u8]) -> Vec<u8> {
        let options = Options {
            compile_cache: CompileCache::Host { code: None },
            ..Options::default()
        };
        Runtime::load(wasm, &vy_runtime::sha256(wasm), options)
            .unwrap()
            .take_code()
            .unwrap()
    }

    /// Everything in a cache directory but its version stamp, by name.
    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|name| name != vy_runtime::STAMP_FILE)
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// The version stamp of a cache directory.
    fn stamp(dir: &Path) -> String {
        std::fs::read_to_string(dir.join(vy_runtime::STAMP_FILE)).unwrap_or_default()
    }

    fn logged(shell: &Shell, tab: TabId, what: &str) -> bool {
        shell.tab_logs(tab).iter().any(|l| l.contains(what))
    }

    /// SPEC-13.1.4, SPEC-9.1.3: with the cache on, the first load of a
    /// module compiles in the tab process and the main process keeps the
    /// code under the module's key; a second load in a fresh tab process,
    /// a reload, and a load after the client restarted take the module
    /// from that code and do not compile, as the tab reports before the
    /// page runs, and draw the same frame. The tab processes are locked
    /// down as ever: the probe, started as a tab is started, still cannot
    /// write a file, and the cache directory holds only what the main
    /// process wrote. Without the cache nothing is offered or reported.
    #[test]
    fn spec_13_1_4_a_second_load_does_not_compile() {
        let dir = scratch("hit");
        let code_dir = dir.join("code");
        let exe = shell_exe();
        let wasm = module("hello");
        let key = CodeKey::of(&wasm);
        let mut sh = shell(Some(exe.clone()), &dir, Some(&code_dir));
        let a = sh.open_tab(Some(local("hello")));
        settle(&mut sh, 1, TIMEOUT).unwrap();
        assert_eq!(sh.tab_status(a), "running");
        assert_eq!(
            sh.tab_code(a).map(|(cached, _)| cached),
            Some(false),
            "the first load compiles"
        );
        assert!(logged(&sh, a, "module code: compiled"));
        let cache = sh.code_cache().expect("the cache is on");
        cache.flush();
        let kept = cache
            .store()
            .get(&key)
            .expect("kept under the module's key");
        assert_eq!(entries(&code_dir), [key.file_name()]);
        assert_eq!(
            stamp(&code_dir),
            format!("VYCODE01 {}\n", env!("CARGO_PKG_VERSION")),
            "the directory is this client version's"
        );

        // A fresh process, the same module: not compiled.
        let b = sh.open_tab(Some(local("hello")));
        settle(&mut sh, 1, TIMEOUT).unwrap();
        assert_eq!(sh.tab_status(b), "running");
        assert_ne!(sh.tab_pid(a), sh.tab_pid(b));
        assert_eq!(sh.tab_code(b).map(|(cached, _)| cached), Some(true));
        assert!(logged(&sh, b, "module code: from the compilation cache"));
        assert!(!sh.tab_display_list(a).is_empty());
        assert_eq!(sh.tab_display_list(a), sh.tab_display_list(b));
        // A reload: a new process again, and again not compiled.
        let before = sh.tab_pid(b);
        sh.reload();
        settle(&mut sh, 1, TIMEOUT).unwrap();
        assert_ne!(sh.tab_pid(b), before);
        assert_eq!(sh.tab_code(b).map(|(cached, _)| cached), Some(true));
        // The processes that compiled and that took the code are sandboxed
        // like any tab, and reported it before their loads were answered.
        for tab in [a, b] {
            let sandbox = sh.tab_sandbox(tab).expect("a process-backed tab reports");
            assert_eq!(sandbox.marker(), None, "{sandbox}");
        }
        let probe = crate::process::probe_process(&exe).unwrap();
        if cfg!(target_os = "linux") && probe.contains("seccomp allowlist") {
            assert!(
                probe.lines().any(|l| l.starts_with("file-write: refused")),
                "{probe}"
            );
        }
        sh.code_cache().unwrap().flush();
        assert_eq!(entries(&code_dir), [key.file_name()], "one entry, no more");
        assert_eq!(sh.code_cache().unwrap().store().get(&key), Some(kept));
        sh.shutdown();

        // The client started again: the entry is still good.
        let mut sh = shell(Some(exe.clone()), &dir, Some(&code_dir));
        let c = sh.open_tab(Some(local("hello")));
        settle(&mut sh, 1, TIMEOUT).unwrap();
        assert_eq!(sh.tab_code(c).map(|(cached, _)| cached), Some(true));
        sh.shutdown();

        // No cache: the load compiles and says nothing about it.
        let mut sh = shell(Some(exe), &dir, None);
        let d = sh.open_tab(Some(local("hello")));
        settle(&mut sh, 1, TIMEOUT).unwrap();
        assert_eq!(sh.tab_status(d), "running");
        assert!(sh.code_cache().is_none());
        assert_eq!(sh.tab_code(d), None);
        assert!(!logged(&sh, d, "module code"));
        sh.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-9.4.2: compiled code is kept by the main process alone, under
    /// a key it makes itself from the module bytes it gave the tab; a tab
    /// process can hand over code only for the module it was loaded with,
    /// once per load, and kept code runs only in a tab process loaded
    /// with the same module.
    ///
    /// A tab loaded with `hello` (a stand-in process that never answers)
    /// hands over the compiled code of another module, `text`: it is kept
    /// under `hello`'s key, the one the main process made, and `text` has
    /// no entry. A second report for the same load, a report after the
    /// load was answered, a claimed hit when nothing was offered, code
    /// over the size cap and a report from a tab the cache is not on for
    /// change nothing. `text`, loaded for real afterwards, is offered
    /// nothing and compiles.
    #[test]
    fn spec_9_4_2_code_is_kept_under_the_main_process_key() {
        let dir = scratch("key");
        let code_dir = dir.join("code");
        let hello = module("hello");
        let text = module("text");
        let text_code = compiled(&text);
        let exe = shell_exe();
        let mut sh = shell(Some(exe.clone()), &dir, Some(&code_dir));
        let report = |sh: &mut Shell, tab: TabId, m: ToShell| {
            let generation = sh.tab_generation(tab);
            sh.handle(Incoming::Tab(tab, generation, Inbound::Message(m)));
            sh.code_cache().unwrap().flush();
        };
        let code = |code: &[u8]| ToShell::Code {
            cached: false,
            micros: 1,
            code: code.to_vec(),
        };
        let store = sh.code_cache().unwrap().store().clone();

        sh.stall_next_tab();
        let tab = sh.open_tab_loaded(page(hello.clone()));
        assert_eq!(sh.tab_status(tab), "loading");
        report(&mut sh, tab, code(&text_code));
        assert_eq!(
            store.get(&CodeKey::of(&hello)).as_ref(),
            Some(&text_code),
            "under the key of the module the main process sent"
        );
        assert_eq!(store.get(&CodeKey::of(&text)), None);
        assert_eq!(entries(&code_dir), [CodeKey::of(&hello).file_name()]);
        assert_eq!(sh.tab_code(tab), Some((false, 1)));

        // Once per load.
        report(&mut sh, tab, code(b"a second answer"));
        assert!(logged(&sh, tab, "compiled code ignored"));
        assert_eq!(store.get(&CodeKey::of(&hello)).as_ref(), Some(&text_code));
        assert_eq!(sh.tab_code(tab), Some((false, 1)));

        // Not after the load is answered: the page's code has run by then.
        sh.stall_next_tab();
        let late = sh.open_tab_loaded(page(text.clone()));
        report(
            &mut sh,
            late,
            ToShell::Loaded {
                ok: true,
                error: String::new(),
                pid: 0,
            },
        );
        assert_eq!(sh.tab_status(late), "running");
        report(&mut sh, late, code(b"after the page ran"));
        assert!(logged(&sh, late, "compiled code ignored"));
        assert_eq!(store.get(&CodeKey::of(&text)), None);
        assert_eq!(sh.tab_code(late), None);

        // A hit claimed when nothing was offered, and code over the cap
        // for its module (256 bytes per byte of it).
        sh.stall_next_tab();
        let liar = sh.open_tab_loaded(page(text.clone()));
        report(
            &mut sh,
            liar,
            ToShell::Code {
                cached: true,
                micros: 0,
                code: Vec::new(),
            },
        );
        assert!(logged(&sh, liar, "report ignored"));
        sh.stall_next_tab();
        let big = sh.open_tab_loaded(page(text.clone()));
        let over = vec![7u8; text.len() * vy_runtime::MAX_CODE_PER_MODULE_BYTE + 1];
        report(&mut sh, big, code(&over));
        assert_eq!(store.get(&CodeKey::of(&text)), None, "not kept");
        assert_eq!(entries(&code_dir).len(), 1);

        // The real thing: `text` is offered nothing, compiles, and its own
        // code is kept under its own key; `hello`'s entry is as it was.
        let real = sh.open_tab_loaded(page(text.clone()));
        settle(&mut sh, 1, TIMEOUT).unwrap();
        assert_eq!(sh.tab_status(real), "running");
        assert_eq!(sh.tab_code(real).map(|(cached, _)| cached), Some(false));
        sh.code_cache().unwrap().flush();
        assert_eq!(store.get(&CodeKey::of(&text)).as_ref(), Some(&text_code));
        assert_eq!(store.get(&CodeKey::of(&hello)).as_ref(), Some(&text_code));
        sh.shutdown();

        // A shell without the cache keeps nothing a tab hands it.
        let mut sh = shell(Some(exe), &dir, None);
        sh.stall_next_tab();
        let tab = sh.open_tab_loaded(page(module("game")));
        let generation = sh.tab_generation(tab);
        sh.handle(Incoming::Tab(
            tab,
            generation,
            Inbound::Message(code(b"unasked")),
        ));
        assert!(logged(&sh, tab, "compiled code ignored"));
        assert_eq!(entries(&code_dir).len(), 2);
        sh.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-13.1.4, SPEC-9.4.2: an entry that cannot be used costs a
    /// compilation and is gone. A damaged or truncated file is removed by
    /// the main process when it looks the module up, and nothing is
    /// offered; code that is whole on disk but does not deserialize is
    /// offered, refused by the tab and dropped; either way the tab compiles
    /// and the fresh code is kept. An entry a tab process died on is not
    /// offered again.
    #[test]
    fn spec_13_1_4_unusable_code_is_dropped_and_compiled() {
        let dir = scratch("bad");
        let code_dir = dir.join("code");
        let wasm = module("hello");
        let key = CodeKey::of(&wasm);
        let good = compiled(&wasm);
        let mut sh = shell(Some(shell_exe()), &dir, Some(&code_dir));
        let store = sh.code_cache().unwrap().store().clone();
        let file = code_dir.join(key.file_name());
        let load = |sh: &mut Shell| {
            let tab = sh.open_tab_loaded(page(wasm.clone()));
            settle(sh, 1, TIMEOUT).unwrap();
            assert_eq!(sh.tab_status(tab), "running");
            sh.code_cache().unwrap().flush();
            tab
        };
        let first = load(&mut sh);
        assert_eq!(sh.tab_code(first).map(|c| c.0), Some(false));
        assert_eq!(store.get(&key).as_ref(), Some(&good));
        let reference = sh.tab_display_list(first);

        // Damaged on disk: a flipped byte, half the file.
        let whole = std::fs::read(&file).unwrap();
        let mut flipped = whole.clone();
        let at = flipped.len() - 9;
        flipped[at] ^= 1;
        for damaged in [flipped, whole[..whole.len() / 2].to_vec()] {
            std::fs::write(&file, &damaged).unwrap();
            let tab = load(&mut sh);
            assert_eq!(sh.tab_code(tab).map(|c| c.0), Some(false), "compiled");
            assert!(
                logged(&sh, tab, "compiled") && !logged(&sh, tab, "not usable"),
                "nothing was offered: {:?}",
                sh.tab_logs(tab)
            );
            assert_eq!(sh.tab_display_list(tab), reference);
            assert_eq!(std::fs::read(&file).unwrap(), whole, "kept afresh");
        }

        // Whole on disk, but not a compiled module.
        for unusable in [
            b"not machine code".to_vec(),
            good[..good.len() / 2].to_vec(),
        ] {
            store.put(&key, wasm.len(), &unusable).unwrap();
            let tab = load(&mut sh);
            assert_eq!(sh.tab_code(tab).map(|c| c.0), Some(false), "compiled");
            assert!(logged(&sh, tab, "not usable"), "{:?}", sh.tab_logs(tab));
            assert_eq!(sh.tab_display_list(tab), reference);
            assert_eq!(store.get(&key).as_ref(), Some(&good), "replaced");
        }
        // Refused with nothing to put in its place: dropped all the same.
        store.put(&key, wasm.len(), b"refused").unwrap();
        sh.stall_next_tab();
        let tab = sh.open_tab_loaded(page(wasm.clone()));
        let generation = sh.tab_generation(tab);
        sh.handle(Incoming::Tab(
            tab,
            generation,
            Inbound::Message(ToShell::Code {
                cached: false,
                micros: 0,
                code: Vec::new(),
            }),
        ));
        sh.code_cache().unwrap().flush();
        assert!(!file.exists());

        // A process that dies with the offered code in it.
        let tab = load(&mut sh);
        assert_eq!(sh.tab_code(tab).map(|c| c.0), Some(false));
        let tab = load(&mut sh);
        assert_eq!(sh.tab_code(tab).map(|c| c.0), Some(true));
        assert!(file.exists());
        sh.kill_tab_process(tab);
        let deadline = Instant::now() + TIMEOUT;
        while sh.tab_status(tab) == "running" {
            assert!(Instant::now() < deadline);
            if let Ok(m) = sh.inbox().recv_timeout(Duration::from_millis(100)) {
                sh.handle(m);
            }
        }
        sh.code_cache().unwrap().flush();
        assert!(!file.exists(), "not offered again");
        sh.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-9.4.2 (granskning 0014:1): kept code does not outlive the
    /// client version that kept it, and the user can throw it away.
    ///
    /// A cache directory another client version kept, holding code for
    /// the very module that is loaded, is emptied when the client starts,
    /// and the module compiles. An entry under another version's key that
    /// turns up while the client runs is never offered: the version is in
    /// the key. "Rensa cache" in the devtools panel removes every entry;
    /// a load that was unanswered when the cache was cleared hands over
    /// nothing that is kept; the next load compiles, and the one after it
    /// takes the module from the cache as ever. Without a cache the panel
    /// offers no clearing.
    #[test]
    fn spec_9_4_2_the_cache_is_cleared_on_update_and_on_request() {
        use crate::chrome::DevHit;
        use crate::devtools::{Command, Pane};
        let dir = scratch("version");
        let code_dir = dir.join("code");
        let exe = shell_exe();
        let wasm = module("hello");
        let good = compiled(&wasm);
        let key = CodeKey::of(&wasm);
        assert_eq!(
            vy_runtime::CLIENT_VERSION,
            env!("CARGO_PKG_VERSION"),
            "the version in the key is the client's"
        );
        let old = "0.0.1";
        let old_key = CodeKey::of_client(&wasm, old);
        let kept = CodeStore::open_for(&code_dir, old).unwrap();
        kept.put(&old_key, wasm.len(), &good).unwrap();
        assert_eq!(entries(&code_dir), [old_key.file_name()]);
        assert_eq!(stamp(&code_dir), "VYCODE01 0.0.1\n");
        let load = |sh: &mut Shell| {
            let tab = sh.open_tab(Some(local("hello")));
            settle(sh, 1, TIMEOUT).unwrap();
            assert_eq!(sh.tab_status(tab), "running");
            sh.code_cache().unwrap().flush();
            sh.tab_code(tab).map(|(cached, _)| cached)
        };

        // The client starts on another version's directory.
        let mut sh = shell(Some(exe.clone()), &dir, Some(&code_dir));
        assert!(entries(&code_dir).is_empty(), "emptied at startup");
        assert_eq!(
            stamp(&code_dir),
            format!("VYCODE01 {}\n", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(load(&mut sh), Some(false), "compiled");
        assert_eq!(entries(&code_dir), [key.file_name()]);

        // Another version's entry beside this version's stamp.
        let store = sh.code_cache().unwrap().store().clone();
        std::fs::remove_file(code_dir.join(key.file_name())).unwrap();
        store.put(&old_key, wasm.len(), &good).unwrap();
        assert_eq!(load(&mut sh), Some(false), "not offered: compiled");
        let mut both = vec![key.file_name().to_string(), old_key.file_name().to_string()];
        both.sort();
        assert_eq!(entries(&code_dir), both);

        // "Rensa cache", by pointer.
        sh.toggle_devtools();
        sh.set_devtools_pane(Pane::Studio);
        sh.frame();
        let line = sh
            .devtools_lines()
            .iter()
            .position(|l| l.command == Some(Command::ClearCache))
            .expect("Rensa cache");
        let (r, _) = sh
            .devtools_hits()
            .into_iter()
            .find(|(_, h)| *h == DevHit::Line(line))
            .expect("the line is on the panel");
        sh.pointer_move(r.x + 3.0, r.y + 3.0);
        sh.pointer_button(vy_abi::button::PRIMARY, true);
        sh.pointer_button(vy_abi::button::PRIMARY, false);
        assert!(entries(&code_dir).is_empty(), "every entry is gone");
        assert!(!stamp(&code_dir).is_empty(), "the stamp stays");
        assert!(
            sh.devtools_lines()
                .iter()
                .any(|l| l.text == "Kompileringscachen rensad: 2 poster borttagna."),
            "{:?}",
            sh.devtools_lines()
        );

        // A load under way when the cache is cleared: its code is not kept.
        sh.stall_next_tab();
        let slow = sh.open_tab_loaded(page(wasm.clone()));
        assert_eq!(sh.tab_status(slow), "loading");
        sh.devtools_run(Command::ClearCache);
        let generation = sh.tab_generation(slow);
        sh.handle(Incoming::Tab(
            slow,
            generation,
            Inbound::Message(ToShell::Code {
                cached: false,
                micros: 1,
                code: good.clone(),
            }),
        ));
        sh.code_cache().unwrap().flush();
        assert!(logged(&sh, slow, "compiled code ignored"));
        assert!(entries(&code_dir).is_empty());
        assert!(sh
            .devtools_lines()
            .iter()
            .any(|l| l.text == "Kompileringscachen rensad: 0 poster borttagna."));

        // And on it goes: compiled and kept, then taken from the cache.
        assert_eq!(load(&mut sh), Some(false));
        assert_eq!(entries(&code_dir), [key.file_name()]);
        assert_eq!(load(&mut sh), Some(true));
        sh.shutdown();

        // No cache, nothing to clear.
        let mut sh = shell(Some(exe), &dir, None);
        sh.toggle_devtools();
        sh.set_devtools_pane(Pane::Studio);
        assert!(sh
            .devtools_lines()
            .iter()
            .all(|l| l.command != Some(Command::ClearCache)));
        assert_eq!(sh.clear_code_cache(), None);
        sh.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-9.4.2, SPEC-9.1.3 (granskning 0014:2): kept code runs only in
    /// a tab process, never in the main process. A shell without a tab
    /// executable runs its pages on threads of the main process; given a
    /// cache directory that holds good code for the module it loads, it
    /// opens no cache, offers the page nothing (the tab says nothing of
    /// its module, as when the cache is off), keeps nothing of a module
    /// that has no entry, and leaves the directory as it was. A report a
    /// thread-backed tab is made to send is ignored. The same directory in
    /// a shell with tab processes is a hit.
    #[test]
    fn spec_9_4_2_kept_code_never_runs_in_the_main_process() {
        let dir = scratch("thread");
        let code_dir = dir.join("code");
        let hello = module("hello");
        let text = module("text");
        let key = CodeKey::of(&hello);
        let store = CodeStore::open(&code_dir).unwrap();
        store.put(&key, hello.len(), &compiled(&hello)).unwrap();
        let before = std::fs::read(code_dir.join(key.file_name())).unwrap();

        let mut sh = shell(None, &dir, Some(&code_dir));
        assert!(sh.code_cache().is_none(), "no cache without tab processes");
        for wasm in [&hello, &text] {
            let tab = sh.open_tab_loaded(page(wasm.clone()));
            settle(&mut sh, 1, TIMEOUT).unwrap();
            assert_eq!(sh.tab_status(tab), "running");
            assert_eq!(sh.tab_pid(tab), None, "a thread of this process");
            assert_eq!(sh.tab_code(tab), None, "nothing offered, nothing said");
            assert!(!logged(&sh, tab, "module code"), "{:?}", sh.tab_logs(tab));
            // What such a tab might send all the same.
            let generation = sh.tab_generation(tab);
            sh.handle(Incoming::Tab(
                tab,
                generation,
                Inbound::Message(ToShell::Code {
                    cached: false,
                    micros: 1,
                    code: b"from a thread".to_vec(),
                }),
            ));
            assert!(logged(&sh, tab, "compiled code ignored"));
        }
        sh.shutdown();
        assert_eq!(entries(&code_dir), [key.file_name()], "nothing was kept");
        assert_eq!(
            std::fs::read(code_dir.join(key.file_name())).unwrap(),
            before,
            "and the entry was not rewritten"
        );

        // The same directory with tab processes: the entry is used.
        let mut sh = shell(Some(shell_exe()), &dir, Some(&code_dir));
        let tab = sh.open_tab_loaded(page(hello.clone()));
        settle(&mut sh, 1, TIMEOUT).unwrap();
        assert!(sh.tab_pid(tab).is_some());
        assert_eq!(sh.tab_code(tab).map(|(cached, _)| cached), Some(true));
        sh.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-9.4.2: without a directory of the user's own there is no
    /// cache: one that group or others may write is not opened, nor one
    /// that is a link to another directory, and one that turns writable
    /// for others later is neither read nor written.
    #[cfg(unix)]
    #[test]
    fn spec_9_4_2_no_cache_in_a_directory_others_may_write() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("perm");
        let code_dir = dir.join("code");
        let mode = |m: u32| {
            std::fs::set_permissions(&code_dir, std::fs::Permissions::from_mode(m)).unwrap()
        };
        std::fs::create_dir_all(&code_dir).unwrap();
        mode(0o777);
        assert!(CodeCache::open(&code_dir).is_none());
        let wasm = module("hello");
        let exe = shell_exe();
        let mut sh = shell(Some(exe.clone()), &dir, Some(&code_dir));
        assert!(sh.code_cache().is_none());
        let tab = sh.open_tab_loaded(page(wasm.clone()));
        settle(&mut sh, 1, TIMEOUT).unwrap();
        assert_eq!(sh.tab_status(tab), "running");
        assert_eq!(sh.tab_code(tab), None);
        assert!(entries(&code_dir).is_empty());
        sh.shutdown();

        // Nor in a link to another directory of the user's (granskning
        // 0014:3): the cache is not followed out of its place.
        let elsewhere = dir.join("elsewhere");
        vy_runtime::compile_cache::create_dir(&elsewhere).unwrap();
        std::fs::write(elsewhere.join("notes.tmp"), b"not the cache's").unwrap();
        let link = dir.join("code-link");
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        assert!(CodeCache::open(&link).is_none());
        let mut sh = shell(Some(exe.clone()), &dir, Some(&link));
        assert!(sh.code_cache().is_none());
        let tab = sh.open_tab_loaded(page(wasm.clone()));
        settle(&mut sh, 1, TIMEOUT).unwrap();
        assert_eq!(sh.tab_status(tab), "running");
        assert_eq!(sh.tab_code(tab), None);
        sh.shutdown();
        assert_eq!(entries(&elsewhere), ["notes.tmp"], "untouched");

        // Opened while private, opened up afterwards.
        mode(0o700);
        let mut sh = shell(Some(exe), &dir, Some(&code_dir));
        let load = |sh: &mut Shell| {
            let tab = sh.open_tab_loaded(page(wasm.clone()));
            settle(sh, 1, TIMEOUT).unwrap();
            sh.code_cache().unwrap().flush();
            sh.tab_code(tab).map(|c| c.0)
        };
        assert_eq!(load(&mut sh), Some(false));
        assert_eq!(load(&mut sh), Some(true));
        mode(0o777);
        assert_eq!(load(&mut sh), Some(false), "not read");
        std::fs::remove_file(code_dir.join(CodeKey::of(&wasm).file_name())).unwrap();
        assert_eq!(load(&mut sh), Some(false));
        assert!(entries(&code_dir).is_empty(), "not written");
        mode(0o700);
        sh.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-9.4.2, SPEC-11.4.1: a recorded tab compiles its module itself
    /// even when the cache holds it, so the recording has no machine code
    /// in it and replays the same anywhere; and a replaying tab is never
    /// fed code from its file: a recording with a code frame put before
    /// its load replays as if the frame were not there.
    #[test]
    fn spec_9_4_2_recordings_hold_no_code() {
        use crate::devtools::{Command, Pane};
        let dir = scratch("rec");
        let code_dir = dir.join("code");
        let mut sh = shell(Some(shell_exe()), &dir, Some(&code_dir));
        let tab = sh.open_tab(Some(local("hello")));
        settle(&mut sh, 1, TIMEOUT).unwrap();
        sh.code_cache().unwrap().flush();
        assert_eq!(entries(&code_dir).len(), 1, "the module is in the cache");
        sh.toggle_devtools();
        sh.set_devtools_pane(Pane::Recording);
        sh.devtools_run(Command::StartRecording);
        let path = sh.recording().expect("recording");
        settle(&mut sh, 1, TIMEOUT).unwrap();
        assert_eq!(sh.tab_status(tab), "running");
        assert_eq!(sh.tab_code(tab), None, "the cache is off under a recording");
        let last = sh.tab_display_list(tab);
        sh.devtools_run(Command::StopRecording);
        sh.key(0x04, true, 0, false);
        sh.key(0x04, false, 0, false);
        settle(&mut sh, 1, TIMEOUT).unwrap();
        let mut frames = Vec::new();
        let mut file = std::io::BufReader::new(std::fs::File::open(&path).unwrap());
        while let Some(body) = read_frame(&mut file).unwrap() {
            frames.push(body);
        }
        assert!(matches!(ToTab::decode(&frames[0]), Ok(ToTab::Load(_))));
        assert!(!frames.iter().any(|f| ToTab::is_code_cache(f)));

        // The same recording with a code frame before the load.
        let forged = dir.join("forged.vyrec");
        let mut out = Vec::new();
        let code = ToTab::CodeCache {
            code: Some(b"code from a file".to_vec()),
        };
        write_frame(&mut out, &code.encode()).unwrap();
        for f in &frames {
            write_frame(&mut out, f).unwrap();
        }
        std::fs::write(&forged, out).unwrap();
        for recording in [path.clone(), forged] {
            let replay = sh.open_replay(recording).expect("a replay tab");
            let deadline = Instant::now() + TIMEOUT;
            while !sh.tab_title(replay).starts_with("Uppspelning klar") {
                assert!(Instant::now() < deadline, "{}", sh.tab_status(replay));
                assert!(!sh.tab_terminal(replay), "{}", sh.tab_status(replay));
                sh.tick();
                if let Ok(m) = sh.inbox().recv_timeout(Duration::from_millis(100)) {
                    sh.handle(m);
                }
            }
            assert_eq!(sh.tab_display_list(replay), last);
            assert_eq!(sh.tab_code(replay), None);
            assert!(
                !logged(&sh, replay, "compiled code ignored"),
                "the tab was not fed the code frame: {:?}",
                sh.tab_logs(replay)
            );
        }
        sh.code_cache().unwrap().flush();
        assert_eq!(entries(&code_dir).len(), 1, "nothing new was kept");
        sh.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
