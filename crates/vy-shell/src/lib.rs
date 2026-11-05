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
#[cfg(feature = "window")]
pub mod keymap;
pub mod markdown;
pub mod page;
pub mod paths;
pub mod process;
pub mod sandbox;
pub mod shell;
pub mod storage;
pub mod tab;
#[cfg(feature = "window")]
pub mod window;

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
    /// (SPEC-8.2.3); `extra` adds capabilities to the draft's request, and
    /// `net=<origin>` replaces the `net` origins with one treated as the
    /// page's own. The page is at `vy://<name>.example/`.
    fn trusted_page(name: &str, extra: &[&str]) -> LoadedPage {
        trusted_page_at(name, extra, &format!("{name}.example"))
    }

    /// [`trusted_page`] at `vy://<authority>/`: a `net` request for
    /// another authority than the page's own is then a question.
    fn trusted_page_at(name: &str, extra: &[&str], authority: &str) -> LoadedPage {
        let wasm = std::fs::read(module_path(name)).unwrap();
        let draft_path = root().join(format!("conformance/manifests/{name}.json"));
        let draft = Draft::from_json(&std::fs::read_to_string(&draft_path).unwrap()).unwrap();
        let inputs = draft.load_resources(draft_path.parent().unwrap()).unwrap();
        let mut manifest = draft.finish(&wasm, &inputs);
        let mut own = vec![format!("https://{authority}")];
        for cap in extra {
            match *cap {
                "clipboard" => manifest.capabilities.clipboard = Some(Empty {}),
                "persist_frame" => manifest.capabilities.persist_frame = Some(Empty {}),
                "hires_time" => manifest.capabilities.hires_time = Some(Empty {}),
                other => match other.strip_prefix("net=") {
                    Some(origin) => {
                        own = vec![origin.to_string()];
                        manifest.capabilities.net = Some(vy_manifest::NetRequest {
                            origins: own.clone(),
                        });
                    }
                    None => panic!("{other}"),
                },
            }
        }
        let key = SigningKey::from_seed(&[3; 32]);
        manifest.sign(&key);
        let trust = manifest.verify(&key.public()).expect("signed by the key");
        let verdicts = manifest
            .capabilities
            .default_grants(Some(&trust), |n| n.origins.iter().all(|o| own.contains(o)));
        let resources = inputs
            .into_iter()
            .zip(&manifest.resources)
            .map(|(i, r)| (i.bytes, r.w.zip(r.h)))
            .collect();
        LoadedPage {
            display: format!("vy://{authority}/"),
            origin: Some(format!("vy://{authority}")),
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
    /// by a page through `navigate` on a user gesture, or clicked as a
    /// link the page registered goes to the system browser; the shell
    /// never loads it, and the page keeps running.
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
        // The page's `navigate` on the user's key press (a gesture).
        shell.key(crate::shell::hid::END, true, 0, false);
        shell.key(crate::shell::hid::END, false, 0, false);
        shell.handle(Incoming::Tab(
            tab,
            shell.tab_generation(tab),
            Inbound::Message(ToShell::Navigate {
                url: "https://docs.example/vy/m1-bench.csv".into(),
                replace: false,
            }),
        ));
        assert_eq!(shell.external_opens().len(), 2);
        assert_eq!(shell.tab_status(tab), "running");
        // A registered https:// link, clicked in the page area: the
        // document's external links are in its notes, at the end (the
        // End key above scrolled there).
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

    /// SPEC-2.1.7, SPEC-5.2.10, SPEC-5.2.12: a page's `navigate` to an
    /// `https://` address opens the system browser only on a user
    /// gesture in its tab (a pointer release or a key press delivered to
    /// the page) within the last second of the shell's clock, and once
    /// per gesture: a thousand calls after one click open one window,
    /// a call without a gesture, after the second has passed, or from a
    /// hidden tab opens nothing and is logged with the address. The
    /// address bar, which the user types in, needs no gesture.
    #[test]
    fn spec_2_1_7_page_opens_externally_once_per_gesture() {
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let tab = shell.open_tab(Some(local("hello")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(tab), "running");
        let navigate = |shell: &mut crate::shell::Shell, tab, url: &str| {
            let generation = shell.tab_generation(tab);
            shell.handle(Incoming::Tab(
                tab,
                generation,
                Inbound::Message(ToShell::Navigate {
                    url: url.into(),
                    replace: false,
                }),
            ));
        };
        let dropped = |shell: &crate::shell::Shell, tab| {
            shell
                .tab_logs(tab)
                .iter()
                .filter(|l| l.starts_with("[host info] not opened: https://"))
                .count()
        };
        // No gesture yet: nothing opens.
        navigate(&mut shell, tab, "https://spam.example/0");
        assert_eq!(shell.external_opens(), &[] as &[String]);
        assert_eq!(dropped(&shell, tab), 1);
        assert!(shell
            .tab_logs(tab)
            .iter()
            .any(|l| l.contains("https://spam.example/0")));
        // Pointer movement and scrolling are not gestures.
        let page = shell.layout().page;
        shell.pointer_move(page.x + 50.0, page.y + 50.0);
        shell.scroll(0.0, 10.0, true);
        navigate(&mut shell, tab, "https://spam.example/1");
        assert_eq!(shell.external_opens().len(), 0);
        assert_eq!(dropped(&shell, tab), 2);
        // One click: a thousand calls open one window.
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        for i in 0..1000 {
            navigate(&mut shell, tab, &format!("https://spam.example/{i}"));
        }
        assert_eq!(shell.external_opens(), ["https://spam.example/0"]);
        assert!(
            shell
                .tab_logs(tab)
                .last()
                .is_some_and(|l| l.starts_with("[host info] not opened: https://spam.example/999 ")),
            "{:?}",
            shell.tab_logs(tab).last()
        );
        // A key press is a gesture, but not after a second has passed.
        shell.key(0x2C, true, 0, false);
        shell.key(0x2C, false, 0, false);
        shell.advance_clock(crate::shell::GESTURE_WINDOW_MS + 1);
        navigate(&mut shell, tab, "https://late.example/");
        assert_eq!(shell.external_opens().len(), 1);
        shell.key(0x2C, true, 0, false);
        shell.advance_clock(crate::shell::GESTURE_WINDOW_MS);
        navigate(&mut shell, tab, "https://in-time.example/");
        assert_eq!(
            shell.external_opens().last().map(String::as_str),
            Some("https://in-time.example/")
        );
        // A gesture in one tab is not a gesture in another: the hidden
        // tab gets no events and its page opens nothing.
        let other = shell.open_tab(Some(local("text")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert!(shell.tab_hidden(tab) && shell.active_tab() == Some(other));
        shell.key(0x2C, true, 0, false);
        shell.key(0x2C, false, 0, false);
        navigate(&mut shell, tab, "https://hidden.example/");
        assert_eq!(shell.external_opens().len(), 2);
        navigate(&mut shell, other, "https://shown.example/");
        assert_eq!(shell.external_opens().len(), 3);
        // Typed by the user, no gesture is needed; the page runs on.
        shell.navigate("https://typed.example/");
        assert_eq!(shell.external_opens().len(), 4);
        assert_eq!(shell.tab_status(other), "running");
        shell.shutdown();
    }

    /// SPEC-8.2.3, SPEC-9.6.1: the capability question lies over the top
    /// of the page area without moving the page, and a click on "Tillåt"
    /// counts only once the question has been up for `DIALOG_ARM_MS`;
    /// "Neka" counts at once. So a page that makes the shell ask right
    /// under the user's next click gets no grant from it, and a click
    /// aimed at page content lands where it was aimed.
    #[test]
    fn spec_8_2_3_capability_question_overlays_and_arms_late() {
        use crate::chrome::{Rect, ADDRESS_H, DIALOG_H, TAB_STRIP_H};
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let tab = shell.open_tab_loaded(trusted_page("chat", &["clipboard"]));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.pending_ask(), Some("clipboard"));
        let frame = shell.frame().clone();
        let layout = shell.layout().clone();
        // The page area is where it is without a question; the question
        // covers its top and holds both buttons.
        let page = Rect::new(
            0.0,
            TAB_STRIP_H + ADDRESS_H,
            800.0,
            600.0 - TAB_STRIP_H - ADDRESS_H,
        );
        assert_eq!(layout.page, page);
        let dialog = layout.dialog.unwrap();
        assert_eq!(dialog, Rect::new(page.x, page.y, page.w, DIALOG_H));
        let (allow, deny) = (layout.allow.unwrap(), layout.deny.unwrap());
        assert!(dialog.contains(allow.x, allow.y) && dialog.contains(deny.x, deny.y));
        // The question's pixels are the shell's, over the page's frame.
        let dialog_bg = [0xFF, 0xF4, 0xD6, 0xFF];
        assert_eq!(frame.pixel(4, dialog.y as u32 + 2), Some(dialog_bg));
        assert_eq!(
            frame.pixel(400, dialog.y as u32 + 2),
            Some(dialog_bg),
            "opaque: nothing of the page shows through"
        );
        assert_ne!(
            frame.pixel(4, dialog.bottom() as u32 + 2),
            Some(dialog_bg),
            "the page below the question"
        );
        // Over the question, the pointer is not in the page.
        shell.pointer_move(allow.x + 5.0, allow.y + 5.0);
        assert_eq!(shell.cursor(), Cursor::Arrow);
        // A click on "Tillåt" right away is ignored and logged...
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        assert_eq!(shell.pending_ask(), Some("clipboard"));
        assert_eq!(shell.tab_badges(tab)[1], ("clipboard", false));
        assert!(shell
            .tab_logs(tab)
            .iter()
            .any(|l| l.starts_with("[host info] click on Tillåt ignored")));
        // ...and again just before the arming time; then it counts.
        shell.advance_clock(crate::shell::DIALOG_ARM_MS - shell.now_ms() - 1);
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        assert_eq!(shell.pending_ask(), Some("clipboard"));
        shell.advance_clock(1);
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        assert_eq!(shell.pending_ask(), None);
        assert_eq!(shell.tab_badges(tab)[1], ("clipboard", true));
        let _ = shell.frame();
        assert!(shell.layout().dialog.is_none(), "the question is gone");
        assert_eq!(shell.layout().page, page, "and the page did not move");
        // "Neka" counts at once: a fresh question in a fresh tab.
        let other = shell.open_tab_loaded(trusted_page("chat", &["persist_frame"]));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.pending_ask(), Some("persist_frame"));
        let _ = shell.frame();
        let deny = shell.layout().deny.unwrap();
        shell.pointer_move(deny.x + 5.0, deny.y + 5.0);
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        assert_eq!(shell.pending_ask(), None);
        assert!(shell.tab_badges(other).contains(&("persist_frame", false)));
        // A page at another authority than its `net` origin has two
        // questions (SPEC-8.2.3). Switching to its tab shows the first
        // afresh: it arms from the switch, not from the load; and the
        // second arms from the moment the first was answered.
        let third = shell.open_tab_loaded(trusted_page_at(
            "chat",
            &["persist_frame"],
            "another.example",
        ));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.pending_ask(), Some("net"));
        shell.advance_clock(crate::shell::DIALOG_ARM_MS);
        shell.switch_tab(true);
        assert_ne!(shell.active_tab(), Some(third));
        assert_eq!(shell.pending_ask(), None, "the other tabs have no question");
        shell.switch_tab(false);
        assert_eq!(shell.active_tab(), Some(third));
        let _ = shell.frame();
        let allow = shell.layout().allow.unwrap();
        shell.pointer_move(allow.x + 5.0, allow.y + 5.0);
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        assert_eq!(
            shell.pending_ask(),
            Some("net"),
            "not armed since the switch"
        );
        shell.advance_clock(crate::shell::DIALOG_ARM_MS);
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        assert_eq!(
            shell.pending_ask(),
            Some("persist_frame"),
            "the next question"
        );
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        assert_eq!(shell.pending_ask(), Some("persist_frame"), "not armed yet");
        shell.advance_clock(crate::shell::DIALOG_ARM_MS);
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        assert_eq!(shell.pending_ask(), None);
        assert!(shell.tab_badges(third).contains(&("net", true)));
        assert!(shell.tab_badges(third).contains(&("persist_frame", true)));
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
        // Navigating a running tab starts a fresh process (SPEC-9.5.2:
        // nothing of the old page survives); the old process's end is
        // not mistaken for the new one's.
        shell.navigate_tab(b, local("hello"), true);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(b), "running");
        let pb2 = shell.tab_pid(b).unwrap();
        assert_ne!(pb2, pb);
        assert!(shell.tab_frames(b) >= 1);
        shell.drain();
        assert_eq!(
            shell.tab_status(b),
            "running",
            "a stale Gone changes nothing"
        );
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
        // Allow by clicking the button, once the question has been up
        // long enough to be armed (`spec_8_2_3_capability_question_overlays_and_arms_late`).
        let allow = layout.allow.unwrap();
        shell.advance_clock(crate::shell::DIALOG_ARM_MS);
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

    /// SPEC-5.2.8, SPEC-5.3.1, SPEC-9.3.1, SPEC-2.1.4: the main process
    /// decides a page's requests itself, since the tab process runs page
    /// code. A tab without `net` gets `E_DENIED`, one with `net` gets
    /// `E_ORIGIN` outside the manifest's origins, and neither reaches the
    /// loader; a request to a named origin does. A clipboard from a tab
    /// without `clipboard` is ignored, and storage from a tab without
    /// `storage` is not saved.
    #[test]
    fn spec_5_2_8_main_process_gates_page_requests() {
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let plain = shell.open_tab(Some(local("hello")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_badges(plain), vec![]);
        let inject = |shell: &mut crate::shell::Shell, tab, m: ToShell| {
            let generation = shell.tab_generation(tab);
            shell.handle(Incoming::Tab(tab, generation, Inbound::Message(m)));
        };
        // Anything the loader answers within `wait` (the inbox otherwise
        // only sees tab messages, which are handled on the way).
        let loader_answer =
            |shell: &mut crate::shell::Shell, wait: Duration| -> Option<(u32, i32, i32)> {
                let deadline = Instant::now() + wait;
                while Instant::now() < deadline {
                    match shell.inbox().recv_timeout(Duration::from_millis(20)) {
                        Ok(Incoming::Load(crate::page::Done::Request {
                            tab, id, status, ..
                        })) => return Some((tab, id, status)),
                        Ok(other) => shell.handle(other),
                        Err(_) => {}
                    }
                }
                None
            };
        let request = |url: &str| ToShell::FetchRequest {
            id: 7,
            method: vy_abi::method::GET,
            url: url.into(),
            body: Vec::new(),
        };
        inject(&mut shell, plain, request("https://127.0.0.1:1/anything"));
        assert!(
            shell
                .tab_logs(plain)
                .iter()
                .any(|l| l.contains("fetch 7: E_DENIED")),
            "{:?}",
            shell.tab_logs(plain)
        );
        assert_eq!(loader_answer(&mut shell, Duration::from_millis(300)), None);

        // A trusted page whose manifest names a loopback origin as its own.
        let tab = shell.open_tab_loaded(trusted_page("chat", &["net=https://127.0.0.1:1"]));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_badges(tab), vec![("net", true), ("ime", true)]);
        inject(&mut shell, tab, request("https://127.0.0.2:1/elsewhere"));
        assert!(shell
            .tab_logs(tab)
            .iter()
            .any(|l| l.contains("fetch 7: E_ORIGIN")));
        inject(&mut shell, tab, request("https://127.0.0.1:1@127.0.0.2:1/"));
        assert_eq!(
            shell
                .tab_logs(tab)
                .iter()
                .filter(|l| l.contains("E_ORIGIN"))
                .count(),
            2,
            "userinfo does not smuggle another host past the gate"
        );
        assert_eq!(loader_answer(&mut shell, Duration::from_millis(300)), None);
        inject(&mut shell, tab, request("https://127.0.0.1:1/api"));
        assert_eq!(
            loader_answer(&mut shell, Duration::from_secs(10)).map(|(t, id, _)| (t, id)),
            Some((tab, 7)),
            "a named origin reaches the loader (and port 1 refuses it)"
        );
        // Revoked, the same request is refused again.
        shell.toggle_capability(tab, "net");
        inject(&mut shell, tab, request("https://127.0.0.1:1/api"));
        assert!(shell
            .tab_logs(tab)
            .iter()
            .any(|l| l.contains("fetch 7: E_DENIED")));

        // A tab's `navigate` is `vy://` or `https://` (SPEC-5.2.10); a
        // path names a local file, which only the address bar opens.
        let generation = shell.tab_generation(plain);
        inject(
            &mut shell,
            plain,
            ToShell::Navigate {
                url: module_path("text").to_string_lossy().into_owned(),
                replace: false,
            },
        );
        assert_eq!(
            shell.tab_generation(plain),
            generation,
            "nothing was loaded"
        );
        assert!(shell
            .tab_logs(plain)
            .iter()
            .any(|l| l.starts_with("[host] navigate refused: ")));
        inject(
            &mut shell,
            plain,
            ToShell::Navigate {
                url: "http://x.example/".into(),
                replace: false,
            },
        );
        assert_eq!(shell.tab_generation(plain), generation);
        assert_eq!(shell.external_opens(), &[] as &[String]);

        // Clipboard and storage follow their grants.
        inject(
            &mut shell,
            plain,
            ToShell::Clipboard(b"not granted".to_vec()),
        );
        assert_eq!(shell.clipboard(), b"");
        inject(
            &mut shell,
            plain,
            ToShell::Storage(vec![(b"k".to_vec(), b"v".to_vec())]),
        );
        assert!(shell
            .tab_logs(plain)
            .iter()
            .any(|l| l.contains("storage of 2 bytes not saved")));
        let clip = shell.open_tab_loaded(trusted_page("chat", &["clipboard"]));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        shell.answer_ask(true);
        assert_eq!(shell.tab_badges(clip)[1], ("clipboard", true));
        inject(&mut shell, clip, ToShell::Clipboard(b"granted".to_vec()));
        assert_eq!(shell.clipboard(), b"granted");
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

    /// SPEC-9.7.1: reproducible builds publish hash and recipe.
    #[test]
    #[ignore = "M5: release"]
    fn spec_9_7_1_reproducible_build() {
        todo!()
    }

    /// SPEC-10.2.1: semantic tree exposed through AccessKit. The shell
    /// receives the tree per frame (`Shell::tab_semantic`); exposing it
    /// to the platform is M4.
    #[test]
    #[ignore = "M4: accessibility"]
    fn spec_10_2_1_accesskit() {
        todo!()
    }

    /// SPEC-10.2.2: text selection copies logical strings.
    #[test]
    #[ignore = "M4: accessibility"]
    fn spec_10_2_2_text_selection() {
        todo!()
    }

    /// SPEC-10.3.1: the shell feeds OS key and pointer events to the runtime, which owns
    /// the focus order (Tab/Shift+Tab become KIND_FOCUS; Enter/Escape stay KEY_DOWN).
    /// The keys reach the runtime as HID codes (`keymap`); the visible focus order is M4.
    #[test]
    #[ignore = "M4: accessibility"]
    fn spec_10_3_1_focus_order() {
        todo!()
    }

    /// SPEC-10.4.1: a page without a semantic tree is marked inaccessible.
    /// The chrome draws the marker ("otillgänglig") from `has_tree`; the
    /// test of the whole path is M4 with AccessKit.
    #[test]
    #[ignore = "M4: accessibility"]
    fn spec_10_4_1_inaccessible_marker() {
        todo!()
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
