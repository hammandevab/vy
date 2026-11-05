//! Shell: window, tabs, address bar, history, capability dialogs (SPEC §9, §11).
//!
//! The main process draws the chrome and rasterizes (SPEC-9.1.2,
//! SPEC-9.6.1); every tab is a child process running one `vy_runtime::Runtime`
//! (SPEC-9.1.1), reached over the frames of [`ipc`]. Everything but the
//! window itself is platform-independent and runs without one:
//! [`headless::screenshot`] drives the same shell, tab processes and
//! compositing to a PNG. The client has no extension system (SPEC-11.5.1)
//! and never renders HTML (SPEC-1.1.1).
#![deny(missing_docs)]

pub mod address;
pub mod chrome;
pub mod compose;
pub mod external;
pub mod grants;
pub mod headless;
pub mod history;
pub mod ipc;
pub mod markdown;
pub mod page;
pub mod paths;
pub mod process;
pub mod shell;
pub mod storage;
pub mod tab;

#[cfg(test)]
mod tests {
    use crate::headless::{headless_shell, settle};
    use crate::ipc::{ToShell, ToTab};
    use crate::page::{LoadedPage, Source};
    use crate::process::{channel_sink, spawn_process, Inbound, TabEvent};
    use crate::shell::{Cursor, Incoming};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use vy_abi::{button, event, key};
    use vy_manifest::{Draft, Empty, SigningKey};

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    fn module_path(name: &str) -> PathBuf {
        let p = root().join(format!("target/wasm/{name}.wasm"));
        assert!(p.is_file(), "{}: run tools/build_modules.sh", p.display());
        p
    }

    /// The built `vy-shell` binary in the test binary's profile directory
    /// (`target/<profile>/deps/..`); `tests/bin.rs` makes cargo build it.
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

    fn local(name: &str) -> Source {
        Source::File {
            path: module_path(name),
            draft: Some(root().join(format!("conformance/manifests/{name}.json"))),
        }
    }

    const TIMEOUT: Duration = Duration::from_secs(60);

    /// A page from a module and its draft, signed with a test key and
    /// verified, so it is trusted and the default policy applies
    /// (SPEC-8.2.3); `extra` adds capabilities to the draft's request.
    fn trusted_page(name: &str, extra: &[&str]) -> LoadedPage {
        let wasm = std::fs::read(module_path(name)).unwrap();
        let draft_path = root().join(format!("conformance/manifests/{name}.json"));
        let draft = Draft::from_json(&std::fs::read_to_string(&draft_path).unwrap()).unwrap();
        let inputs = draft.load_resources(draft_path.parent().unwrap()).unwrap();
        let mut manifest = draft.finish(&wasm, &inputs);
        for cap in extra {
            match *cap {
                "clipboard" => manifest.capabilities.clipboard = Some(Empty {}),
                "persist_frame" => manifest.capabilities.persist_frame = Some(Empty {}),
                "hires_time" => manifest.capabilities.hires_time = Some(Empty {}),
                other => panic!("{other}"),
            }
        }
        let key = SigningKey::from_seed(&[3; 32]);
        manifest.sign(&key);
        let trust = manifest.verify(&key.public()).expect("signed by the key");
        let verdicts = manifest.capabilities.default_grants(Some(&trust), |n| {
            n.origins.iter().all(|o| o == "https://chat.example")
        });
        let resources = inputs
            .into_iter()
            .zip(&manifest.resources)
            .map(|(i, r)| (i.bytes, r.w.zip(r.h)))
            .collect();
        LoadedPage {
            display: format!("vy://{name}.example/"),
            origin: Some(format!("vy://{name}.example")),
            sha256: manifest.module.hash.0,
            manifest: Some(manifest),
            wasm,
            resources,
            trusted: true,
            verdicts,
            is_localhost: false,
        }
    }

    fn texts(dl: &[u8]) -> Vec<String> {
        vy_dl::decode(dl)
            .unwrap()
            .iter()
            .filter_map(|op| match op {
                vy_dl::Op::Text { text, .. } => Some(String::from_utf8_lossy(text).into_owned()),
                _ => None,
            })
            .collect()
    }

    /// The chrome alone for one page view, as a display list.
    fn chrome_for(page: crate::chrome::PageView<'_>, address: &str) -> Vec<u8> {
        let mut text = vy_render::TextEngine::new();
        let styles = crate::chrome::Styles::register(&mut text);
        let tabs = [crate::chrome::TabInfo {
            id: 1,
            title: "x".into(),
            loading: false,
        }];
        let mut field = crate::address::AddressField::new();
        field.set_committed(address);
        let view = crate::chrome::View {
            width: 800.0,
            height: 600.0,
            tabs: &tabs,
            active: 0,
            address: &field,
            can_back: false,
            can_forward: false,
            restricted: true,
            inaccessible: true,
            loading: false,
            badges: &[],
            ask: None,
            page,
            zoom: 1.0,
        };
        crate::chrome::draw(&view, &mut text, &styles).0
    }

    /// SPEC-1.1.2, SPEC-11.1.1: the crates are the ones the spec names
    /// with the dependencies CLAUDE.md gives them: the shell uses the
    /// runtime, renderer, protocol and manifest crates and never the SDK
    /// or a module; nothing below the shell depends on it; the SDK
    /// depends on the ABI alone.
    #[test]
    fn spec_11_1_1_crate_boundaries() {
        let deps = |name: &str| -> Vec<String> {
            let text =
                std::fs::read_to_string(root().join(format!("crates/{name}/Cargo.toml"))).unwrap();
            let mut out = Vec::new();
            let mut in_deps = false;
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with('[') {
                    in_deps = line == "[dependencies]";
                    continue;
                }
                if in_deps {
                    if let Some((k, _)) = line.split_once('=') {
                        let k = k.trim();
                        let name = k.split(['.', ' ']).next().unwrap_or(k);
                        out.push(name.to_string());
                    }
                }
            }
            out
        };
        for name in [
            "vy-abi",
            "vy-manifest",
            "vy-proto",
            "vy-runtime",
            "vy-dl",
            "vy-render",
            "vy-shell",
            "vy-cli",
        ] {
            assert!(root().join("crates").join(name).is_dir(), "{name}");
        }
        let shell = deps("vy-shell");
        for needed in [
            "vy-abi",
            "vy-dl",
            "vy-manifest",
            "vy-proto",
            "vy-render",
            "vy-runtime",
        ] {
            assert!(
                shell.contains(&needed.to_string()),
                "vy-shell needs {needed}"
            );
        }
        assert!(
            !shell.contains(&"vy-sdk".to_string()),
            "no SDK in the client"
        );
        assert!(!shell.iter().any(|d| d.contains("modules")));
        for below in [
            "vy-abi",
            "vy-dl",
            "vy-manifest",
            "vy-proto",
            "vy-runtime",
            "vy-render",
        ] {
            assert!(
                !deps(below).contains(&"vy-shell".to_string()),
                "{below} must not depend on vy-shell"
            );
        }
        assert_eq!(
            deps("vy-abi"),
            Vec::<String>::new(),
            "the ABI crate stands alone"
        );
        assert_eq!(deps("vy-sdk"), vec!["vy-abi"], "the SDK sees only the ABI");
        assert!(!deps("vy-render").contains(&"vy-runtime".to_string()));
    }

    /// SPEC-2.1.7: an `https://` address typed in the address bar, called
    /// by a page through `navigate`, or clicked as a link the page
    /// registered goes to the system browser; the shell never loads it,
    /// and the page keeps running.
    #[test]
    fn spec_2_1_7_https_opens_externally() {
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let tab = shell.open_tab(None);
        shell.navigate("https://example.test/a?b=c");
        assert_eq!(shell.external_opens(), ["https://example.test/a?b=c"]);
        assert_eq!(shell.tab_status(tab), "empty", "nothing was loaded");
        shell.navigate("http://example.test/");
        assert_eq!(shell.external_opens().len(), 1, "plain http is not opened");
        assert!(shell.tab_status(tab).starts_with("failed"));

        shell.navigate_tab(tab, local("document"), true);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(tab), "running");
        shell.handle(Incoming::Tab(
            tab,
            Inbound::Message(ToShell::Navigate {
                url: "https://docs.example/vy/m1-bench.csv".into(),
                replace: false,
            }),
        ));
        assert_eq!(shell.external_opens().len(), 2);
        assert_eq!(shell.tab_status(tab), "running");
        // A registered https:// link, clicked in the page area: the
        // document's external links are in its notes, at the end.
        shell.key(crate::shell::hid::END, true, 0, false);
        shell.key(crate::shell::hid::END, false, 0, false);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let page = shell.layout().page;
        let link = shell
            .tab_links(tab)
            .into_iter()
            .find(|l| l.url.starts_with("https://") && l.y >= 0.0 && l.y + l.h <= page.h)
            .expect("the document has a visible external link");
        let (x, y) = (
            page.x + link.x + link.w / 2.0,
            page.y + link.y + link.h / 2.0,
        );
        shell.pointer_move(x, y);
        assert_eq!(shell.cursor(), Cursor::Hand);
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        assert_eq!(
            shell.external_opens().last().map(String::as_str),
            Some(link.url.as_str())
        );
        assert_eq!(shell.tab_status(tab), "running");
        assert_eq!(shell.address().committed(), "document.wasm");
        shell.shutdown();
    }

    /// SPEC-9.1.1, SPEC-9.1.2: every tab is its own OS process, distinct
    /// from the shell's and from each other's; what crosses the boundary
    /// is a display list (bytes of appendix C), rasterized here.
    #[test]
    fn spec_9_1_1_process_isolation() {
        let exe = shell_exe();
        let mut shell = headless_shell(Some(exe), 800.0, 600.0, 1.0).unwrap();
        let a = shell.open_tab(Some(local("hello")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let b = shell.open_tab(Some(local("text")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let (pa, pb) = (shell.tab_pid(a).unwrap(), shell.tab_pid(b).unwrap());
        assert_ne!(pa, pb);
        assert_ne!(pa, std::process::id());
        assert_ne!(pb, std::process::id());
        assert_eq!(shell.tab_status(a), "running");
        assert_eq!(shell.tab_status(b), "running");
        assert!(shell.tab_frames(b) >= 1);
        // The frame on screen is this process's rasterization of the
        // text page's list: an opaque page below the chrome, different
        // from the chrome's own pixels.
        let frame = shell.frame().clone();
        let chrome_h = shell.layout().chrome_h as u32;
        assert_eq!(frame.pixel(400, chrome_h + 300).map(|p| p[3]), Some(255));
        assert_ne!(frame.pixel(400, chrome_h + 300), frame.pixel(400, 10));
        shell.shutdown();
    }

    /// SPEC-9.2.1, SPEC-8.2.3: a trusted page's requested capabilities
    /// are shown as badges in the address bar, an "ask" capability is a
    /// question the user answers, and any one of them can be revoked (and
    /// re-granted) by itself: the page hears `CAPABILITY` and the chat
    /// page goes offline when `net` is taken away.
    #[test]
    fn spec_9_2_1_capability_ui() {
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let tab = shell.open_tab_loaded(trusted_page("chat", &["clipboard"]));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(tab), "running");
        assert_eq!(
            shell.tab_badges(tab),
            vec![("net", true), ("clipboard", false), ("ime", true)]
        );
        assert_eq!(shell.pending_ask(), Some("clipboard"));
        let _ = shell.frame();
        let layout = shell.layout().clone();
        assert!(layout.allow.is_some() && layout.deny.is_some());
        assert_eq!(
            layout.badges.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            ["net", "clipboard", "ime"]
        );
        // Allow by clicking the button.
        let allow = layout.allow.unwrap();
        shell.pointer_move(allow.x + 5.0, allow.y + 5.0);
        shell.pointer_button(button::PRIMARY, true);
        assert_eq!(shell.pending_ask(), None);
        assert_eq!(shell.tab_badges(tab)[1], ("clipboard", true));
        let _ = shell.frame();
        assert!(shell.layout().allow.is_none(), "the question is gone");
        // Revoke net with its badge: the page is told and goes offline.
        let net_badge = shell.layout().badges[0].1;
        shell.pointer_move(net_badge.x + 3.0, net_badge.y + 3.0);
        shell.pointer_button(button::PRIMARY, true);
        assert_eq!(shell.tab_badges(tab)[0], ("net", false));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert!(
            shell
                .tab_semantic(tab)
                .iter()
                .any(|n| n.text.ends_with("· offline")),
            "{:?}",
            shell.tab_semantic(tab)
        );
        // And back.
        shell.toggle_capability(tab, "net");
        assert_eq!(shell.tab_badges(tab)[0], ("net", true));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert!(shell
            .tab_semantic(tab)
            .iter()
            .any(|n| n.text.ends_with("· ansluten")));
        // An untrusted page has no grants and no question (SPEC-7.4.3).
        let local_tab = shell.open_tab(Some(local("chat")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(
            shell.tab_badges(local_tab),
            vec![("net", false), ("ime", false)]
        );
        assert_eq!(shell.pending_ask(), None);
        shell.shutdown();
    }

    /// SPEC-9.6.1: the address bar is the client's: a page that paints
    /// far outside its area (a rectangle from -10 000 to +10 000) colours
    /// the page area only; the chrome keeps its own pixels and draws the
    /// address itself, whatever the page draws; the font scale changes
    /// what the page sees, not the chrome.
    #[test]
    fn spec_9_6_1_chrome_owned_by_client() {
        let wasm = wat::parse_str(
            r#"(module
                (import "vy" "dl_begin" (func $dl_begin (result i32)))
                (import "vy" "dl_end" (func $dl_end (result i32)))
                (import "vy" "dl_rect" (func $dl_rect (param f32 f32 f32 f32 f32 i32) (result i32)))
                (memory (export "memory") 1)
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32)
                    (drop (call $dl_begin))
                    (drop (call $dl_rect (f32.const -10000) (f32.const -10000) (f32.const 20000) (f32.const 20000) (f32.const 0) (i32.const 0xFF0000FF)))
                    (drop (call $dl_end))
                    i32.const 1)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join(format!("vy-chrome-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("red.wasm");
        std::fs::write(&path, &wasm).unwrap();
        let mut shell = headless_shell(None, 640.0, 400.0, 1.0).unwrap();
        let tab = shell.open_tab(Some(Source::File {
            path: path.clone(),
            draft: None,
        }));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(tab), "running");
        let chrome_h = shell.layout().chrome_h as u32;
        let frame = shell.frame().clone();
        let red = [255, 0, 0, 255];
        for y in 0..chrome_h {
            for x in [0, 100, 320, 639] {
                assert_ne!(
                    frame.pixel(x, y),
                    Some(red),
                    "chrome pixel ({x}, {y}) painted by the page"
                );
            }
        }
        for (x, y) in [(0, chrome_h), (320, chrome_h + 100), (639, 399)] {
            assert_eq!(frame.pixel(x, y), Some(red), "page pixel ({x}, {y})");
        }
        assert_eq!(shell.address().committed(), "red.wasm");
        assert!(
            texts(&chrome_for(crate::chrome::PageView::Page, "red.wasm"))
                .contains(&"red.wasm".to_string())
        );
        shell.zoom_step(1);
        assert_eq!(shell.zoom(), 1.1);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let zoomed = shell.frame().clone();
        assert_eq!(zoomed.pixel(320, chrome_h + 100), Some(red));
        assert_eq!(zoomed.pixel(2, 2), frame.pixel(2, 2));
        shell.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-11.2.1, SPEC-11.2.2: display lists cross the process boundary
    /// as frames of the per-tab transport (pipes in M3, behind the same
    /// framing a shared-memory ring buffer will carry); a tab process
    /// killed outright leaves the shell and the other tabs untouched: the
    /// dead tab shows the shell's own error view, the other still draws,
    /// and F5 starts a fresh process.
    #[test]
    fn spec_11_2_1_ring_buffer_and_crash_containment() {
        let exe = shell_exe();
        let mut shell = headless_shell(Some(exe), 800.0, 600.0, 1.0).unwrap();
        let a = shell.open_tab(Some(local("game")));
        settle(&mut shell, 3, TIMEOUT).unwrap();
        let b = shell.open_tab(Some(local("hello")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert!(shell.tab_frames(b) >= 1);
        shell.kill_tab_process(a);
        let deadline = Instant::now() + TIMEOUT;
        while !shell.tab_status(a).starts_with("gone") {
            assert!(Instant::now() < deadline, "{}", shell.tab_status(a));
            if let Ok(m) = shell.inbox().recv_timeout(Duration::from_millis(100)) {
                shell.handle(m);
            }
        }
        assert_eq!(shell.tab_status(b), "running", "the other tab is untouched");
        assert!(shell.tab_pid(a).is_none());
        assert!(shell.tab_pid(b).is_some());
        // The dead tab, activated, shows the shell's own view.
        shell.switch_tab(true);
        assert_eq!(shell.active_tab(), Some(a));
        let frame = shell.frame().clone();
        assert_eq!((frame.width, frame.height), (800, 600));
        assert!(texts(&chrome_for(
            crate::chrome::PageView::Stopped("Flikprocessen avslutades."),
            "game.wasm"
        ))
        .contains(&"Sidan stannade".to_string()));
        // Back to the live tab: it still answers ticks with frames.
        shell.switch_tab(true);
        assert_eq!(shell.active_tab(), Some(b));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(b), "running");
        // F5 on the dead tab starts a fresh process.
        shell.switch_tab(true);
        shell.reload();
        settle(&mut shell, 3, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(a), "running");
        assert!(shell.tab_pid(a).is_some());
        shell.shutdown();
    }

    /// Drive a recorded or replayed tab process and collect what it sends
    /// until the shutdown's storage answer.
    fn drive_tab(
        exe: &Path,
        record: Option<&Path>,
        replay: Option<&Path>,
        script: &[ToTab],
    ) -> Vec<ToShell> {
        let (sink, inbox) = mpsc::channel::<TabEvent>();
        let conn = spawn_process(exe, 1, channel_sink(sink), record, replay).unwrap();
        if replay.is_none() {
            for m in script {
                assert!(conn.send(m));
            }
        }
        let mut out = Vec::new();
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let now = Instant::now();
            assert!(now < deadline, "tab did not finish: {} messages", out.len());
            match inbox.recv_timeout(deadline - now) {
                Ok((_, Inbound::Message(m))) => {
                    let storage = matches!(m, ToShell::Storage(_));
                    out.push(m);
                    if storage {
                        break;
                    }
                }
                Ok((_, Inbound::Gone { .. })) => break,
                Err(_) => panic!("tab silent"),
            }
        }
        conn.shutdown();
        out
    }

    /// SPEC-11.4.1, SPEC-2.1.5: a tab process started with `--record`
    /// writes every inbound frame; started with `--replay` of that file
    /// it reproduces the session exactly: the same display lists, byte
    /// for byte, from a page that uses `random` and `now_ms` (the game),
    /// since the random key and every clock reading are part of the
    /// inbound stream.
    #[test]
    fn spec_11_4_1_record_replay() {
        let exe = shell_exe();
        let dir = std::env::temp_dir().join(format!("vy-record-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let record = dir.join("session.vyrec");
        let wasm = std::fs::read(module_path("game")).unwrap();
        let mut load = crate::ipc::LoadRequest {
            sha256: vy_runtime::sha256(&wasm),
            wasm,
            memory_max_pages: 64,
            capabilities: 0,
            resources: Vec::new(),
            storage: Vec::new(),
            storage_quota_bytes: 1 << 20,
            net_origins: Vec::new(),
            random: vy_runtime::RandomSource::ChaCha20([42; 32]),
            width: 800,
            height: 600,
            scale: 1.0,
            clock_ms: 1_000,
            clipboard: None,
        };
        let mut script = vec![ToTab::Load(Box::new(load.clone()))];
        let mut clock = 1_000;
        script.push(ToTab::Tick { clock_ms: clock });
        script.push(ToTab::Event {
            kind: event::KEY_DOWN,
            a: 0x2C,
            b: 0,
            c: 0,
            d: 0,
            clock_ms: clock,
        });
        for _ in 0..12 {
            clock += 17;
            script.push(ToTab::Tick { clock_ms: clock });
        }
        script.push(ToTab::Event {
            kind: event::KEY_DOWN,
            a: key::ESCAPE,
            b: 0,
            c: 0,
            d: 0,
            clock_ms: clock,
        });
        script.push(ToTab::Tick {
            clock_ms: clock + 16,
        });
        script.push(ToTab::Shutdown);
        let recorded = drive_tab(&exe, Some(&record), None, &script);
        let frames = |msgs: &[ToShell]| -> Vec<Vec<u8>> {
            msgs.iter()
                .filter_map(|m| match m {
                    ToShell::Frame { display_list, .. } => Some(display_list.clone()),
                    _ => None,
                })
                .collect()
        };
        let first = frames(&recorded);
        assert!(first.len() >= 10, "{} frames", first.len());
        assert!(first.windows(2).any(|w| w[0] != w[1]), "the game moves");
        let replayed = drive_tab(&exe, None, Some(&record), &[]);
        assert_eq!(frames(&replayed), first, "replay reproduces every frame");
        assert_eq!(
            recorded
                .iter()
                .filter(|m| matches!(m, ToShell::Semantic(_)))
                .count(),
            replayed
                .iter()
                .filter(|m| matches!(m, ToShell::Semantic(_)))
                .count()
        );
        // Another key gives another session (the serve's direction is one
        // random byte modulo a few serves, so a handful of keys are tried):
        // the inbound stream, key included, is the session.
        let mut differed = false;
        for seed in 43u8..49 {
            load.random = vy_runtime::RandomSource::ChaCha20([seed; 32]);
            script[0] = ToTab::Load(Box::new(load.clone()));
            let other = drive_tab(&exe, Some(&dir.join("other.vyrec")), None, &script);
            if frames(&other) != first {
                differed = true;
                break;
            }
        }
        assert!(differed, "a different random key, a different serve");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-13.1.1: cold start to empty window <= 150 ms.
    #[test]
    #[ignore = "M3: needs a window; a tab process's cold start to its first display list is measured by `vy-cli bench`"]
    fn spec_13_1_1_cold_start() {
        todo!()
    }

    /// CPU time of a process in clock ticks, from `/proc` (Linux only).
    fn cpu_ticks(pid: u32) -> Option<u64> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let after = stat.rsplit(')').next()?;
        let fields: Vec<&str> = after.split_whitespace().collect();
        let utime: u64 = fields.get(11)?.parse().ok()?;
        let stime: u64 = fields.get(12)?.parse().ok()?;
        Some(utime + stime)
    }

    /// Resident set of a process in bytes, from `/proc` (Linux only).
    fn rss_bytes(pid: u32) -> Option<u64> {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let line = status.lines().find(|l| l.starts_with("VmRSS:"))?;
        let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
        Some(kb * 1024)
    }

    /// SPEC-13.3.1, SPEC-13.3.2, SPEC-6.2.2, SPEC-9.3.1: ten text tabs
    /// stay within 400 MB of resident memory (the shell and its tab
    /// processes together; asserted in release builds, reported in debug
    /// ones), and a hidden tab runs no frames and spends no CPU, however
    /// much its page animates: the game, hidden behind another tab, gets
    /// no tick and its process's CPU time does not move.
    #[test]
    fn spec_13_3_1_memory_and_background() {
        let exe = shell_exe();
        let mut shell = headless_shell(Some(exe), 800.0, 600.0, 1.0).unwrap();
        let game = shell.open_tab(Some(local("game")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        // Space serves; from then on the game asks for a frame per frame.
        shell.key(0x2C, true, 0, false);
        shell.key(0x2C, false, 0, false);
        settle(&mut shell, 3, TIMEOUT).unwrap();
        assert!(shell.animating(), "the game asks for frames");
        let mut tabs = vec![game];
        for _ in 0..10 {
            tabs.push(shell.open_tab(Some(local("text"))));
            settle(&mut shell, 1, TIMEOUT).unwrap();
        }
        for t in &tabs {
            assert_eq!(shell.tab_status(*t), "running", "tab {t}");
        }
        assert!(shell.tab_hidden(game));
        let frames_before = shell.tab_frames(game);
        let pid = shell.tab_pid(game).unwrap();
        let ticks_before = cpu_ticks(pid);
        // Drive the active (text) tab for a while; the hidden game gets nothing.
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(400) {
            shell.tick();
            if let Ok(m) = shell.inbox().recv_timeout(Duration::from_millis(50)) {
                shell.handle(m);
            }
        }
        shell.drain();
        assert_eq!(
            shell.tab_frames(game),
            frames_before,
            "no frames while hidden (SPEC-6.2.1)"
        );
        if let (Some(before), Some(after)) = (ticks_before, cpu_ticks(pid)) {
            assert!(
                after <= before + 1,
                "hidden tab used CPU: {before} -> {after} ticks"
            );
        }
        let mut total = rss_bytes(std::process::id()).unwrap_or(0);
        let mut per_tab = Vec::new();
        for t in &tabs {
            if let Some(r) = shell.tab_pid(*t).and_then(rss_bytes) {
                per_tab.push(r);
                total += r;
            }
        }
        if !per_tab.is_empty() {
            eprintln!(
                "RSS: shell + {} tabs = {:.1} MB (tabs {:.1}-{:.1} MB each)",
                per_tab.len(),
                total as f64 / 1e6,
                per_tab.iter().min().copied().unwrap_or(0) as f64 / 1e6,
                per_tab.iter().max().copied().unwrap_or(0) as f64 / 1e6
            );
            if !cfg!(debug_assertions) {
                assert!(
                    total <= 400 << 20,
                    "{total} bytes for 10 text tabs (SPEC-13.3.1)"
                );
            }
        }
        // Shown again, the game draws once more.
        while shell.active_tab() != Some(game) {
            shell.switch_tab(true);
        }
        assert!(!shell.tab_hidden(game));
        settle(&mut shell, 2, TIMEOUT).unwrap();
        assert!(shell.tab_frames(game) > frames_before);
        shell.shutdown();
    }
}
