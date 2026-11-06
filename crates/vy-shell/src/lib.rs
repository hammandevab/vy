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

pub mod a11y;
pub mod address;
pub mod chrome;
pub mod clipboard;
pub mod compose;
pub mod devtools;
pub mod external;
pub mod find;
pub mod grants;
pub mod headless;
pub mod history;
pub mod ipc;
#[cfg(feature = "window")]
pub mod keymap;
pub mod markdown;
pub mod netlimit;
pub mod page;
pub mod paths;
pub mod process;
pub mod sandbox;
pub mod selection;
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
            restriction: None,
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
            restriction: None,
            inaccessible: true,
            loading: false,
            badges: &[],
            ask: None,
            page,
            zoom: 1.0,
            find: false,
            devtools: false,
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
        assert_eq!(shell.clipboard_text(), None);
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
        assert_eq!(shell.clipboard_text().as_deref(), Some("granted"));
        shell.shutdown();
    }

    /// SPEC-5.2.12, SPEC-8.2.4, SPEC-5.1.6: the clipboard is the system's
    /// (memory here). Ctrl+V with focus in the page delivers its text to
    /// the page as `TEXT_INPUT`, as if typed, with `\n` line endings, and
    /// the form page's focused field takes it; Ctrl+V, Ctrl+C and Ctrl+X in
    /// the address bar paste, copy and cut through the same clipboard.
    /// Ctrl+C or Ctrl+X in the page is a copy gesture: the page's
    /// `clipboard_write` while handling that key succeeds without the
    /// capability and lands on the clipboard, once per gesture; a write on
    /// any other key is refused by the tab's runtime, and a second write
    /// the tab claims for the same gesture is refused by the shell.
    #[test]
    fn spec_5_2_12_clipboard_copy_paste_through_the_shell() {
        use crate::shell::hid;
        use vy_abi::modifier::CTRL;
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let form = shell.open_tab(Some(local("form")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(form), "running");
        // Tab to the name field, paste: the field holds the text, line
        // endings normalised and nothing stripped.
        shell.key(key::TAB, true, 0, false);
        shell.key(key::TAB, false, 0, false);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        shell.set_clipboard_text("Anna\r\nLind ");
        assert!(
            shell.key(hid::V, true, CTRL, false),
            "consumed by the shell"
        );
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let name_field = |shell: &crate::shell::Shell| {
            shell
                .tab_semantic(form)
                .into_iter()
                .find(|n| n.id == -10)
                .map(|n| n.text)
        };
        assert_eq!(name_field(&shell).as_deref(), Some("Anna\nLind "));
        // An empty clipboard pastes nothing.
        shell.set_clipboard_text("");
        shell.key(hid::V, true, CTRL, false);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(name_field(&shell).as_deref(), Some("Anna\nLind "));
        // The address bar: paste replaces the selection, copy and cut
        // write the clipboard.
        shell.set_clipboard_text("vy://docs.example/rapport");
        shell.key(hid::L, true, CTRL, false);
        shell.key(hid::V, true, CTRL, false);
        assert_eq!(shell.address().text(), "vy://docs.example/rapport");
        shell.key(hid::A, true, CTRL, false);
        shell.key(hid::C, true, CTRL, false);
        assert_eq!(
            shell.clipboard_text().as_deref(),
            Some("vy://docs.example/rapport")
        );
        shell.set_clipboard_text("other");
        shell.key(hid::X, true, CTRL, false);
        assert_eq!(shell.address().text(), "", "cut");
        assert_eq!(
            shell.clipboard_text().as_deref(),
            Some("vy://docs.example/rapport")
        );
        // A whole document pasted into the address bar is cut short.
        shell.set_clipboard_text(&"x".repeat(100_000));
        shell.key(hid::V, true, CTRL, false);
        assert_eq!(
            shell.address().text().len(),
            crate::clipboard::MAX_ADDRESS_PASTE_BYTES
        );
        shell.key(key::ESCAPE, true, 0, false);
        assert!(!shell.address().focused());

        // A page that writes the clipboard on every key it gets.
        let wasm = wat::parse_str(
            r#"(module
                (import "vy" "clipboard_write" (func $cw (param i32 i32) (result i32)))
                (memory (export "memory") 1)
                (data (i32.const 16) "kopia")
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32) i32.const 0)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                    (if (i32.eq (local.get 0) (i32.const 5))
                        (then (drop (call $cw (i32.const 16) (i32.const 5)))))
                    i32.const 0)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join(format!("vy-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("copier.wasm");
        std::fs::write(&path, &wasm).unwrap();
        let copier = shell.open_tab(Some(Source::File { path, draft: None }));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(copier), "running");
        assert_eq!(shell.tab_badges(copier), vec![], "no capabilities at all");
        // A plain key: the write is refused in the tab, the clipboard stays.
        shell.set_clipboard_text("before");
        shell.key(0x05, true, 0, false);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        shell.drain();
        assert_eq!(shell.clipboard_text().as_deref(), Some("before"));
        assert!(shell
            .tab_logs(copier)
            .iter()
            .any(|l| l.contains("clipboard_write: E_DENIED")));
        // Ctrl+C: a copy gesture; the page's write reaches the clipboard.
        assert!(
            !shell.key(hid::C, true, CTRL, false),
            "the key went to the page"
        );
        settle(&mut shell, 1, TIMEOUT).unwrap();
        shell.drain();
        assert_eq!(shell.clipboard_text().as_deref(), Some("kopia"));
        // The same again with Ctrl+X, after the clipboard changed elsewhere.
        shell.set_clipboard_text("elsewhere");
        shell.key(hid::X, true, CTRL, false);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        shell.drain();
        assert_eq!(shell.clipboard_text().as_deref(), Some("kopia"));
        // One write per gesture: a second `Clipboard` the tab claims for
        // the same gesture is refused by the shell and logged.
        shell.set_clipboard_text("elsewhere");
        let generation = shell.tab_generation(copier);
        shell.handle(Incoming::Tab(
            copier,
            generation,
            Inbound::Message(ToShell::Clipboard(b"forged".to_vec())),
        ));
        assert_eq!(shell.clipboard_text().as_deref(), Some("elsewhere"));
        assert!(shell
            .tab_logs(copier)
            .iter()
            .any(|l| l.starts_with("[host] clipboard ignored")));
        // And a gesture is not a grant for long: after a second, nothing.
        shell.key(hid::C, true, CTRL, false);
        shell.advance_clock(crate::shell::GESTURE_WINDOW_MS + 1);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        shell.drain();
        assert_eq!(shell.clipboard_text().as_deref(), Some("elsewhere"));
        shell.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-5.2.9, SPEC-5.1.5, SPEC-9.5.1: what a page put in its storage
    /// is saved when the tab's process is let go of, not only when the tab
    /// is hidden: on a reload (F5) and a navigation in the same tab, whose
    /// answer arrives under the old generation, and on Ctrl+W, after which
    /// the tab is gone, as well as when the window closes. The tab runs in
    /// the in-process transport; the shell persists as in a window (not
    /// headless), to a scratch data directory.
    #[test]
    fn spec_5_2_9_storage_saved_on_reload_and_close() {
        use crate::clipboard::MemoryClipboard;
        use crate::external::RecordingOpener;
        use crate::shell::{Config, Shell, Waker};
        // On every key: `state_put("k", [keycode])`.
        let wasm = wat::parse_str(
            r#"(module
                (import "vy" "state_put" (func $put (param i32 i32 i32 i32) (result i32)))
                (memory (export "memory") 1)
                (data (i32.const 16) "k")
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32) i32.const 0)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32)
                    (if (i32.eq (local.get 0) (i32.const 5))
                        (then
                            (i32.store8 (i32.const 32) (local.get 1))
                            (drop (call $put (i32.const 16) (i32.const 1) (i32.const 32) (i32.const 1)))))
                    i32.const 0)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
        )
        .unwrap();
        let data = std::env::temp_dir().join(format!("vy-storage-flush-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        std::fs::create_dir_all(&data).unwrap();
        let origin = "vy://store.example";
        let page = LoadedPage {
            display: format!("{origin}/"),
            origin: Some(origin.into()),
            manifest: None,
            sha256: vy_runtime::sha256(&wasm),
            wasm,
            resources: Vec::new(),
            trusted: true,
            restriction: None,
            verdicts: vec![("storage", vy_manifest::Grant::Auto)],
            is_localhost: false,
        };
        let mut shell = Shell::new(
            Config {
                exe: None,
                data_dir: data.clone(),
                headless: false,
                opener: Box::new(RecordingOpener::default()),
                clipboard: Box::new(MemoryClipboard::new()),
                waker: Waker::none(),
            },
            800.0,
            600.0,
            1.0,
        )
        .unwrap();
        let saved = || crate::storage::load(&data.join("storage"), origin);
        let key = |shell: &mut Shell, code: i32| {
            shell.key(code, true, 0, false);
            shell.key(code, false, 0, false);
        };
        // F5: the old process's answer comes under the old generation.
        let tab = shell.open_tab_loaded(page.clone());
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(tab), "running");
        assert_eq!(shell.tab_badges(tab), vec![("storage", true)]);
        key(&mut shell, 0x04);
        assert!(saved().is_empty(), "nothing saved while the page runs");
        shell.reload();
        shell.drain();
        assert_eq!(saved(), vec![(b"k".to_vec(), vec![0x04])], "saved on F5");
        // A navigation in the same tab: the same.
        let tab = shell.open_tab_loaded(page.clone());
        settle(&mut shell, 1, TIMEOUT).unwrap();
        key(&mut shell, 0x05);
        shell.navigate_tab(
            tab,
            Source::File {
                path: PathBuf::from("no-such-page.wasm"),
                draft: None,
            },
            true,
        );
        shell.drain();
        assert_eq!(
            saved(),
            vec![(b"k".to_vec(), vec![0x05])],
            "saved on navigation"
        );
        // Ctrl+W: the tab is gone when its answer is handled.
        shell.open_tab_loaded(page.clone());
        settle(&mut shell, 1, TIMEOUT).unwrap();
        key(&mut shell, 0x06);
        shell.key(crate::shell::hid::W, true, vy_abi::modifier::CTRL, false);
        shell.drain();
        assert_eq!(
            saved(),
            vec![(b"k".to_vec(), vec![0x06])],
            "saved on Ctrl+W"
        );
        // The window closing, with the tab still open.
        shell.open_tab_loaded(page);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        key(&mut shell, 0x07);
        shell.shutdown();
        assert_eq!(saved(), vec![(b"k".to_vec(), vec![0x07])], "saved on close");
        let _ = std::fs::remove_dir_all(&data);
    }

    /// SPEC-10.1.1, SPEC-9.6.1: find on page (Ctrl+F) searches the page's
    /// semantic tree, case-insensitively, and counts the matches ("1 av
    /// N"); Enter and Shift+Enter cycle and wrap; the bar is the shell's,
    /// over the top right of the page area; a match below the viewport is
    /// brought into view with synthetic `SCROLL` events (the page owns
    /// scrolling); the matches are highlighted over the page and the
    /// highlight goes with the bar on Escape; the bar's buttons work.
    #[test]
    fn spec_10_1_1_find_on_page() {
        use crate::shell::hid;
        use vy_abi::modifier::{CTRL, SHIFT};
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let tab = shell.open_tab(Some(local("document")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let page = shell.layout().page;
        let count = |shell: &crate::shell::Shell, word: &str| -> usize {
            shell
                .tab_semantic(tab)
                .iter()
                .map(|n| n.text.to_lowercase().matches(word).count())
                .sum()
        };
        // Ctrl+F: the bar, focused; typing searches.
        assert!(shell.key(hid::F, true, CTRL, false));
        assert!(shell.find_open());
        shell.frame();
        let bar = shell.layout().find.expect("laid out");
        assert!(bar.y >= page.y && bar.right() <= page.right() && bar.x > page.w / 2.0);
        let expected = count(&shell, "macos");
        assert!(expected >= 2, "{expected}");
        shell.text_input("MACOS");
        assert_eq!(shell.find_status().1, expected);
        assert_eq!(shell.find_label(), format!("1 av {expected}"));
        // Enter, Shift+Enter, wrapping both ways.
        shell.key(key::ENTER, true, 0, false);
        assert_eq!(shell.find_label(), format!("2 av {expected}"));
        shell.key(key::ENTER, true, SHIFT, false);
        shell.key(key::ENTER, true, SHIFT, false);
        assert_eq!(shell.find_label(), format!("{expected} av {expected}"));
        shell.key(key::ENTER, true, 0, false);
        assert_eq!(shell.find_label(), format!("1 av {expected}"));
        // Nothing found.
        shell.key(hid::A, true, CTRL, false);
        shell.text_input("qqqxyz");
        assert_eq!(shell.find_label(), "Inga träffar");
        // A match far below the viewport is scrolled into view.
        let word = "diffkvot";
        assert_eq!(count(&shell, word), 1);
        let target = shell
            .tab_semantic(tab)
            .into_iter()
            .find(|n| n.text.to_lowercase().contains(word))
            .expect("the last note");
        assert!(target.y > page.h, "starts below the viewport: {target:?}");
        shell.key(hid::A, true, CTRL, false);
        shell.text_input("Diffkvot");
        settle(&mut shell, 20, TIMEOUT).unwrap();
        let (current, total) = shell.find_status();
        assert_eq!(total, 1);
        let node = shell
            .tab_semantic(tab)
            .into_iter()
            .find(|n| n.id == current.unwrap().id)
            .unwrap();
        assert!(
            node.y >= 0.0 && node.y + node.h <= page.h,
            "scrolled into view: {node:?}"
        );
        // Highlighted over the page; Escape closes the bar and the
        // highlight with it.
        let with = shell.frame().clone();
        assert!(shell.key(key::ESCAPE, true, 0, false), "the bar's Escape");
        assert!(!shell.find_open());
        let without = shell.frame().clone();
        let rows = (page.y + node.y) as u32..(page.y + node.y + node.h) as u32;
        let differ = rows
            .flat_map(|y| (0..page.w as u32).map(move |x| (x, y)))
            .filter(|&(x, y)| with.pixel(x, y) != without.pixel(x, y))
            .count();
        assert!(differ > 20, "{differ} pixels tinted by the match");
        // The bar's buttons: next and close; a click in the page area
        // under the bar is the shell's, not the page's.
        shell.key(hid::F, true, CTRL, false);
        shell.text_input("macos");
        shell.frame();
        let next = shell.layout().find_next.unwrap();
        // The page is scrolled to its end now: the fresh search starts at
        // the first match in view, and "next" goes on from there.
        let at = |label: String| -> usize { label.split(' ').next().unwrap().parse().unwrap() };
        let before = at(shell.find_label());
        shell.pointer_move(next.x + 4.0, next.y + 4.0);
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        assert_eq!(at(shell.find_label()), before % expected + 1);
        let close = shell.layout().find_close.unwrap();
        shell.pointer_move(close.x + 4.0, close.y + 4.0);
        shell.pointer_button(button::PRIMARY, true);
        assert!(!shell.find_open());
        shell.shutdown();
    }

    /// SPEC-11.4.1, SPEC-9.6.1, SPEC-C.1.1: F12 opens the devtools panel
    /// at the right of the window; the page area narrows (the page gets
    /// the new size) and the panel is the shell's. "Display list" lists the
    /// frame's ops with counts and bytes, and selecting one outlines its
    /// rectangle over the page; the pane tabs are hit by the pointer;
    /// "Semantik" marks the focused node; "Logg" carries frame numbers;
    /// "Inspelning" needs a tab process; "Studio" turns live reload on and
    /// opens Studio. F12 again closes it.
    #[test]
    fn spec_11_4_1_devtools_panel() {
        use crate::chrome::DevHit;
        use crate::devtools::{Command, Pane};
        use crate::shell::hid;
        let mut shell = headless_shell(None, 900.0, 600.0, 1.0).unwrap();
        let form = shell.open_tab(Some(local("form")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let full = shell.layout().page;
        assert!(shell.key(hid::F12, true, 0, false));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        shell.frame();
        let panel = shell.layout().devtools.expect("open");
        let page = shell.layout().page;
        assert_eq!(page.right(), panel.x);
        assert_eq!(panel.right(), 900.0);
        assert_eq!(panel.w, crate::devtools::PANEL_W);
        assert!(page.w < full.w);
        // The page laid itself out again for the narrower area (SPEC-6.2.1):
        // its tree fits it.
        assert!(shell
            .tab_semantic(form)
            .iter()
            .all(|n| n.x + n.w <= page.w + 0.5));
        // Display list: the frame's ops.
        assert_eq!(shell.devtools_pane(), Some(Pane::DisplayList));
        let list = shell.tab_display_list(form);
        let ops = crate::devtools::ops(&list);
        let lines = shell.devtools_lines();
        assert_eq!(
            lines[0].text,
            format!("{} opkoder, {} byte", ops.len(), list.len())
        );
        let (i, op) = ops
            .iter()
            .enumerate()
            .find(|(_, o)| {
                o.rect.is_some_and(|r| {
                    r.w > 4.0
                        && r.h > 4.0
                        && r.x >= 1.0
                        && r.y >= 1.0
                        && r.right() < page.w - 1.0
                        && r.bottom() < page.h - 1.0
                })
            })
            .unwrap_or_else(|| panic!("a rectangle to outline: {ops:?}"));
        let before = shell.frame().clone();
        shell.devtools_run(Command::SelectOp(i));
        let after = shell.frame().clone();
        let r = op.rect.unwrap();
        let (px, py) = (
            (page.x + r.x + 0.5) as u32,
            (page.y + r.y + r.h / 2.0) as u32,
        );
        assert_ne!(after.pixel(px, py), before.pixel(px, py), "outlined");
        assert!(shell
            .devtools_lines()
            .iter()
            .any(|l| l.strong && l.command == Some(Command::SelectOp(i))));
        assert_eq!(
            shell.tab_display_list(form),
            list,
            "the page's list is untouched"
        );
        // The pane tabs, by pointer.
        let hit = |shell: &crate::shell::Shell, want: DevHit| {
            shell
                .devtools_hits()
                .into_iter()
                .find(|(_, h)| *h == want)
                .map(|(r, _)| r)
                .unwrap()
        };
        let click = |shell: &mut crate::shell::Shell, r: crate::chrome::Rect| {
            shell.pointer_move(r.x + 3.0, r.y + 3.0);
            shell.pointer_button(button::PRIMARY, true);
            shell.pointer_button(button::PRIMARY, false);
            shell.frame();
        };
        let r = hit(&shell, DevHit::Pane(Pane::Semantic));
        click(&mut shell, r);
        assert_eq!(shell.devtools_pane(), Some(Pane::Semantic));
        shell.key(key::TAB, true, 0, false);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let focused = shell.tab_focus(form).unwrap();
        assert!(
            shell
                .devtools_lines()
                .iter()
                .any(|l| l.strong && l.text.contains(&format!("▶ textbox #{focused}"))),
            "{:?}",
            shell.devtools_lines()
        );
        // Logg: with the frame it came at.
        shell.set_devtools_pane(Pane::Log);
        let generation = shell.tab_generation(form);
        shell.handle(Incoming::Tab(
            form,
            generation,
            Inbound::Message(ToShell::Log {
                module: true,
                level: vy_abi::level::INFO,
                message: "hej från sidan".into(),
            }),
        ));
        let frames = shell.tab_frames(form);
        assert!(shell
            .devtools_lines()
            .iter()
            .any(|l| l.text == format!("frame {frames} [module info] hej från sidan")));
        // Inspelning: a thread-backed tab cannot record.
        shell.set_devtools_pane(Pane::Recording);
        assert!(shell
            .devtools_lines()
            .iter()
            .all(|l| l.command != Some(Command::StartRecording)));
        // Studio: live reload on, and Studio in a new tab, by pointer.
        shell.set_devtools_pane(Pane::Studio);
        shell.frame();
        assert!(!shell.live_reload());
        let lines = shell.devtools_lines();
        let at = |cmd: Command| {
            lines
                .iter()
                .position(|l| l.command == Some(cmd.clone()))
                .unwrap()
        };
        let toggle = hit(&shell, DevHit::Line(at(Command::ToggleLiveReload)));
        click(&mut shell, toggle);
        assert!(shell.live_reload());
        assert!(shell.poll_in().is_some());
        let studio = hit(&shell, DevHit::Line(at(Command::OpenStudio)));
        click(&mut shell, studio);
        assert_eq!(shell.tab_ids().len(), 2);
        assert_eq!(shell.address().committed(), crate::devtools::STUDIO_ADDRESS);
        // Closed again: the page has the whole width.
        shell.key(hid::F12, true, 0, false);
        shell.frame();
        assert!(shell.layout().devtools.is_none());
        assert_eq!(shell.layout().page.w, 900.0);
        shell.shutdown();
    }

    /// SPEC-11.4.1, SPEC-2.1.5: "Inspelning" records the active tab's
    /// session from a fresh load (`--record`), stops on request with the
    /// file complete, lists it, and replays it in a new tab (`--replay`):
    /// the replayed session ends on the same display list, byte for byte,
    /// as the recorded one when it was stopped.
    #[test]
    fn spec_11_4_1_devtools_record_and_replay() {
        use crate::devtools::{Command, Pane};
        let exe = shell_exe();
        let mut shell = headless_shell(Some(exe), 800.0, 600.0, 1.0).unwrap();
        let game = shell.open_tab(Some(local("game")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        shell.toggle_devtools();
        shell.set_devtools_pane(Pane::Recording);
        assert!(shell
            .devtools_lines()
            .iter()
            .any(|l| l.command == Some(Command::StartRecording)));
        shell.devtools_run(Command::StartRecording);
        let path = shell.recording().expect("recording");
        assert!(path.starts_with(shell.recordings_dir()));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(game), "running");
        shell.key(0x2C, true, 0, false);
        shell.key(0x2C, false, 0, false);
        settle(&mut shell, 6, TIMEOUT).unwrap();
        let last = shell.tab_display_list(game);
        shell.devtools_run(Command::StopRecording);
        assert!(shell.recording().is_none());
        // The tab handles its messages in order: once it has answered a
        // tick sent after the stop (a key makes one due), the file is
        // flushed and complete.
        shell.key(0x04, true, 0, false);
        shell.key(0x04, false, 0, false);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let size = std::fs::metadata(&path).map_or(0, |m| m.len());
        assert!(size > 0, "{}", path.display());
        assert!(shell
            .devtools_lines()
            .iter()
            .any(|l| l.command == Some(Command::Replay(path.clone()))));
        shell.devtools_run(Command::Replay(path.clone()));
        let replay = shell.active_tab().unwrap();
        assert_ne!(replay, game);
        let deadline = Instant::now() + TIMEOUT;
        while !shell.tab_title(replay).starts_with("Uppspelning klar") {
            assert!(Instant::now() < deadline, "{}", shell.tab_status(replay));
            assert!(!shell.tab_terminal(replay), "{}", shell.tab_status(replay));
            shell.tick();
            if let Ok(m) = shell.inbox().recv_timeout(Duration::from_millis(100)) {
                shell.handle(m);
            }
        }
        assert_eq!(shell.tab_display_list(replay), last, "the same session");
        shell.shutdown();
        let _ = std::fs::remove_file(&path);
    }

    /// A one-path HTTP server for live reload: `/` answers with the
    /// current body, its hash as `ETag` and `max-age=60`, and 304 to a
    /// matching `If-None-Match`; everything else is 404. Counts the
    /// conditional requests.
    struct PageServer {
        port: u16,
        body: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
        conditional: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl PageServer {
        fn start(body: Vec<u8>) -> Self {
            use std::io::{BufRead, BufReader, Write};
            use std::sync::atomic::Ordering;
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let body = std::sync::Arc::new(std::sync::Mutex::new(body));
            let conditional = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let (b, c) = (body.clone(), conditional.clone());
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let mut reader = BufReader::new(stream);
                    let mut first = String::new();
                    let _ = reader.read_line(&mut first);
                    let mut inm = None;
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                            break;
                        }
                        if let Some((k, v)) = line.split_once(':') {
                            if k.eq_ignore_ascii_case("if-none-match") {
                                inm = Some(v.trim().to_string());
                            }
                        }
                    }
                    let path = first.split(' ').nth(1).unwrap_or("").to_string();
                    let body = b.lock().unwrap().clone();
                    let etag = format!("\"{}\"", &hex(&vy_runtime::sha256(&body))[..16]);
                    let mut out = reader.into_inner();
                    let head = if path != "/" {
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_string()
                    } else if inm.is_some() {
                        c.fetch_add(1, Ordering::SeqCst);
                        if inm.as_deref() == Some(etag.as_str()) {
                            format!("HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\nCache-Control: max-age=60\r\nConnection: close\r\n\r\n")
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    if !head.is_empty() {
                        let _ = out.write_all(head.as_bytes());
                        continue;
                    }
                    let _ = out.write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/vy-package+zstd\r\nContent-Length: {}\r\nETag: {etag}\r\nCache-Control: max-age=60\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    );
                    let _ = out.write_all(&body);
                }
            });
            Self {
                port,
                body,
                conditional,
            }
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The hello page as a package whose manifest is named `name`.
    fn hello_package(name: &str) -> Vec<u8> {
        let wasm = std::fs::read(module_path("hello")).unwrap();
        let draft_path = root().join("conformance/manifests/hello.json");
        let draft = Draft::from_json(&std::fs::read_to_string(&draft_path).unwrap()).unwrap();
        let inputs = draft.load_resources(draft_path.parent().unwrap()).unwrap();
        let mut manifest = draft.finish(&wasm, &inputs);
        manifest.name = name.into();
        manifest.sign(&SigningKey::from_seed(&[5; 32]));
        vy_manifest::Package {
            manifest,
            module: wasm,
            resources: inputs.into_iter().map(|i| i.bytes).collect(),
        }
        .pack()
        .unwrap()
    }

    /// SPEC-7.4.3, SPEC-7.4.5: the chrome says why a fetched page is
    /// restricted, in Swedish beside the marker, from the fetch's own
    /// reason (`Fetched::restriction`): through the loader for a signed
    /// page whose origin serves no key file, and for a first visit whose
    /// `_vy.<host>` has no TXT record; the tab's log keeps the detail. A
    /// local file has no origin to trust and shows the marker alone.
    #[test]
    fn spec_7_4_3_restriction_reason_in_the_chrome() {
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let shown = |shell: &mut crate::shell::Shell| texts(&shell.chrome_display_list());

        // Fetched through the loader: no /.well-known/vy-key on the origin.
        let server = PageServer::start(hello_package("Utan nyckelfil"));
        let address = format!("vy://127.0.0.1:{}/", server.port);
        let tab = shell.open_tab(Some(Source::parse(&address).unwrap()));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(tab), "running");
        let chrome = shown(&mut shell);
        assert!(
            chrome.contains(&"begränsat läge: nyckelfilen kunde inte hämtas".to_string()),
            "{chrome:?}"
        );
        assert!(
            shell
                .tab_logs(tab)
                .iter()
                .any(|l| l.starts_with("[host info] restricted mode: /.well-known/vy-key")),
            "{:?}",
            shell.tab_logs(tab)
        );

        // A first visit without a TXT record, as the fetch reports it.
        let mut page = trusted_page("hello", &[]);
        page.trusted = false;
        page.restriction = Some(vy_proto::Restriction::DnsNoRecord {
            name: "_vy.hello.example".into(),
        });
        for (_, grant) in &mut page.verdicts {
            *grant = vy_manifest::Grant::Denied;
        }
        let tab = shell.open_tab_loaded(page);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(tab), "running");
        let chrome = shown(&mut shell);
        assert!(
            chrome.contains(&"begränsat läge: nyckeln saknas i DNS".to_string()),
            "{chrome:?}"
        );
        assert!(shell.tab_logs(tab).iter().any(|l| {
            l
            == "[host info] restricted mode: no v=vy1 TXT record at _vy.hello.example (SPEC-7.4.5)"
        }));

        // A local file: the marker alone.
        let file = shell.open_tab(Some(local("hello")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(file), "running");
        let chrome = shown(&mut shell);
        assert!(chrome.contains(&"begränsat läge".to_string()), "{chrome:?}");
        assert!(!chrome.iter().any(|t| t.starts_with("begränsat läge:")));
        shell.shutdown();
    }

    /// SPEC-7.3.2, SPEC-11.4.1: live reload (devtools "Studio") asks the
    /// server of a localhost page every 500 ms on the shell's clock with a
    /// conditional GET, even while the stored manifest is fresh
    /// (`max-age=60`): an unchanged page (304) is left alone, a changed one
    /// is reloaded, and the reload shows the new version.
    #[test]
    fn spec_7_3_2_live_reload_localhost() {
        use crate::devtools::Command;
        use std::sync::atomic::Ordering;
        let server = PageServer::start(hello_package("Version 1"));
        let mut shell = headless_shell(None, 640.0, 400.0, 1.0).unwrap();
        let address = format!("vy://127.0.0.1:{}/", server.port);
        let tab = shell.open_tab(Some(Source::parse(&address).unwrap()));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(tab), "running");
        assert_eq!(shell.tab_title(tab), "Version 1");
        shell.toggle_devtools();
        shell.devtools_run(Command::ToggleLiveReload);
        let wait = |shell: &mut crate::shell::Shell,
                    done: &dyn Fn(&crate::shell::Shell) -> bool| {
            let deadline = Instant::now() + TIMEOUT;
            while !done(shell) {
                assert!(Instant::now() < deadline, "{}", shell.tab_status(tab));
                shell.tick();
                if let Ok(m) = shell.inbox().recv_timeout(Duration::from_millis(50)) {
                    shell.handle(m);
                }
            }
        };
        // The first check: conditional, answered 304; nothing reloads.
        let generation = shell.tab_generation(tab);
        shell.poll();
        wait(&mut shell, &|_| {
            server.conditional.load(Ordering::SeqCst) >= 1
        });
        shell.advance_clock(crate::devtools::LIVE_RELOAD_MS);
        std::thread::sleep(Duration::from_millis(200));
        shell.drain();
        shell.poll();
        wait(&mut shell, &|_| {
            server.conditional.load(Ordering::SeqCst) >= 2
        });
        std::thread::sleep(Duration::from_millis(200));
        shell.drain();
        assert_eq!(
            shell.tab_generation(tab),
            generation,
            "unchanged: no reload"
        );
        // Not before 500 ms have passed on the shell's clock.
        shell.poll();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(server.conditional.load(Ordering::SeqCst), 2);
        // The page changes on its server: the next check reloads it.
        *server.body.lock().unwrap() = hello_package("Version 2");
        shell.advance_clock(crate::devtools::LIVE_RELOAD_MS);
        shell.poll();
        wait(&mut shell, &|s| s.tab_title(tab) == "Version 2");
        assert!(shell.tab_generation(tab) > generation);
        assert!(shell
            .tab_logs(tab)
            .iter()
            .any(|l| l.contains("live reload")));
        shell.shutdown();
    }

    /// SPEC-A.5.1: Ctrl with Alt is AltGr, a character and not a shortcut:
    /// AltGr+0 (`}`) and AltGr+`+` (`\`) on a Swedish keyboard leave the
    /// zoom alone and reach the address bar as typed text, while Ctrl+0
    /// and Cmd+0 still reset it.
    #[test]
    fn spec_a_5_1_altgr_is_not_a_shortcut() {
        use crate::shell::hid;
        use vy_abi::modifier::{ALT, CTRL, META};
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        shell.open_tab(None);
        shell.zoom_step(1);
        assert_eq!(shell.zoom(), 1.1);
        shell.key(hid::DIGIT0, true, CTRL | ALT, false);
        shell.key(hid::MINUS, true, CTRL | ALT, false);
        assert_eq!(shell.zoom(), 1.1, "AltGr is no zoom shortcut");
        shell.key(hid::L, true, CTRL, false);
        shell.key(hid::DIGIT0, true, CTRL | ALT, false);
        shell.text_input("}");
        assert!(shell.address().text().ends_with('}'));
        shell.key(key::ESCAPE, true, 0, false);
        shell.key(hid::DIGIT0, true, META, false);
        assert_eq!(shell.zoom(), 1.0, "Cmd+0");
        shell.zoom_step(1);
        shell.key(hid::DIGIT0, true, CTRL, false);
        assert_eq!(shell.zoom(), 1.0, "Ctrl+0");
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

    /// SPEC-10.2.2, SPEC-2.1.5: the host owns text selection and copies
    /// logical strings, never pixels. On the document page: a drag over a
    /// paragraph selects its text exactly; a shorter drag on its first line
    /// selects a prefix that grows with the drag (glyph granularity from the
    /// renderer's measurements); a drag into the next paragraph takes both,
    /// joined with a line break; Ctrl+C puts that on the clipboard. The
    /// highlight is the shell's overlay: the frame is tinted under it, the
    /// page's own display list is byte for byte what it was, and Escape
    /// restores the pixels. Ctrl+A selects every text node, a press away
    /// from text clears; in a focused field Ctrl+A is the page's.
    #[test]
    fn spec_10_2_2_text_selection() {
        use crate::shell::hid;
        use vy_abi::modifier::CTRL;
        use vy_runtime::SemNode;
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let tab = shell.open_tab(Some(local("document")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(tab), "running");
        let page = shell.layout().page;
        let sem = shell.tab_semantic(tab);
        let shown = |n: &&SemNode| {
            n.role == vy_abi::role::TEXT
                && n.text.len() > 60
                && n.x >= 0.0
                && n.y >= 0.0
                && n.x + n.w <= page.w
                && n.y + n.h <= page.h
        };
        let paragraphs: Vec<SemNode> = sem.iter().filter(shown).cloned().collect();
        assert!(paragraphs.len() >= 2, "{sem:?}");
        let (a, b) = (&paragraphs[0], &paragraphs[1]);
        let at = |n: &SemNode, dx: f32, dy: f32| (page.x + n.x + dx, page.y + n.y + dy);
        let drag = |shell: &mut crate::shell::Shell, from: (f32, f32), to: (f32, f32)| {
            shell.pointer_move(from.0, from.1);
            shell.pointer_button(button::PRIMARY, true);
            shell.pointer_move((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
            shell.pointer_move(to.0, to.1);
            shell.pointer_button(button::PRIMARY, false);
        };
        let list_before = shell.tab_display_list(tab);
        let frame_before = shell.frame().clone();
        // The whole paragraph, corner to corner.
        drag(&mut shell, at(a, 0.5, 2.0), at(a, a.w - 0.5, a.h - 2.0));
        assert_eq!(shell.selected_text(), a.text);
        // Part of its first line: a proper prefix, longer the further the
        // drag goes, cut at a character boundary.
        drag(&mut shell, at(a, 0.5, 2.0), at(a, a.w * 0.25, 2.0));
        let short = shell.selected_text();
        drag(&mut shell, at(a, 0.5, 2.0), at(a, a.w * 0.5, 2.0));
        let longer = shell.selected_text();
        assert!(!short.is_empty() && a.text.starts_with(&short), "{short:?}");
        assert!(longer.len() > short.len() && a.text.starts_with(&longer));
        assert!(longer.len() < a.text.len(), "the first line only");
        // Into the next paragraph: both, a line break between.
        drag(&mut shell, at(a, 0.5, 2.0), at(b, b.w * 0.3, 2.0));
        let both = shell.selected_text();
        let tail = both.rsplit('\n').next().unwrap();
        assert!(both.starts_with(&format!("{}\n", a.text)), "{both:?}");
        assert!(!tail.is_empty() && b.text.starts_with(tail), "{both:?}");
        // Copied as text.
        assert!(shell.key(hid::C, true, CTRL, false), "the shell's copy");
        assert_eq!(shell.clipboard_text().as_deref(), Some(both.as_str()));
        // The highlight is drawn by the shell over the page: the frame is
        // tinted at the selection's first line, the page's list unchanged.
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let (px, py) = at(a, 1.0, 1.0);
        let tinted = shell.frame().pixel(px as u32, py as u32);
        assert_ne!(tinted, frame_before.pixel(px as u32, py as u32));
        assert_eq!(shell.tab_display_list(tab), list_before);
        // Escape clears it and the pixels are the page's again.
        assert!(
            !shell.key(key::ESCAPE, true, 0, false),
            "Escape reaches the page"
        );
        assert_eq!(shell.selected_text(), "");
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(
            shell.frame().pixel(px as u32, py as u32),
            frame_before.pixel(px as u32, py as u32)
        );
        // Ctrl+A: every text node in document order.
        assert!(shell.key(hid::A, true, CTRL, false));
        let all = crate::selection::selected_text(
            &shell.tab_semantic(tab),
            &crate::selection::all(&shell.tab_semantic(tab)),
        );
        assert_eq!(shell.selected_text(), all);
        assert!(all.contains(&a.text) && all.contains(&b.text));
        // A press away from text clears.
        shell.pointer_move(page.x + page.w - 2.0, page.bottom() - 2.0);
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        assert_eq!(shell.selected_text(), "");
        shell.shutdown();

        // In a focused field, Ctrl+A belongs to the page.
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let form = shell.open_tab(Some(local("form")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        shell.key(key::TAB, true, 0, false);
        shell.key(key::TAB, false, 0, false);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let focused = shell.tab_focus(form).expect("Tab focused a field");
        assert!(shell
            .tab_semantic(form)
            .iter()
            .any(|n| n.id == focused && n.role == vy_abi::role::TEXTBOX));
        assert!(!shell.key(hid::A, true, CTRL, false), "went to the page");
        assert_eq!(shell.selected_text(), "");
        shell.shutdown();
    }

    /// A page with a fixed tree (text 1, button 2, textbox 3, link 4, in
    /// document order) that logs every `FOCUS` it gets as `F<b><a><c>`
    /// (in/out, node, keyboard/pointer) and every `POINTER_DOWN` as `P`,
    /// so a test sees the deliveries themselves, in order.
    fn focus_logger(dir: &Path) -> Source {
        let wasm = wat::parse_str(
            r#"(module
                (import "vy" "log" (func $log (param i32 i32 i32) (result i32)))
                (import "vy" "dl_begin" (func $dl_begin (result i32)))
                (import "vy" "dl_end" (func $dl_end (result i32)))
                (import "vy" "dl_rect" (func $dl_rect (param f32 f32 f32 f32 f32 i32) (result i32)))
                (import "vy" "sem_node" (func $sem (param i32 i32 i32 f32 f32 f32 f32 i32 i32) (result i32)))
                (memory (export "memory") 1)
                (data (i32.const 64) "Rubrik")
                (data (i32.const 72) "Skicka")
                (data (i32.const 80) "Namn")
                (data (i32.const 88) "Mer")
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32)
                    (drop (call $dl_begin))
                    (drop (call $dl_rect (f32.const 0) (f32.const 0) (f32.const 50) (f32.const 5) (f32.const 0) (i32.const 0x336699FF)))
                    (drop (call $sem (i32.const 1) (i32.const 0) (i32.const 1) (f32.const 10) (f32.const 10) (f32.const 200) (f32.const 20) (i32.const 64) (i32.const 6)))
                    (drop (call $sem (i32.const 2) (i32.const 0) (i32.const 4) (f32.const 10) (f32.const 40) (f32.const 100) (f32.const 30) (i32.const 72) (i32.const 6)))
                    (drop (call $sem (i32.const 3) (i32.const 0) (i32.const 5) (f32.const 10) (f32.const 80) (f32.const 200) (f32.const 30) (i32.const 80) (i32.const 4)))
                    (drop (call $sem (i32.const 4) (i32.const 0) (i32.const 3) (f32.const 10) (f32.const 120) (f32.const 100) (f32.const 20) (i32.const 88) (i32.const 3)))
                    (drop (call $dl_end))
                    i32.const 1)
                (func (export "vy_event") (param $k i32) (param $a i32) (param $b i32) (param $c i32) (param $d i32) (result i32)
                    (if (i32.eq (local.get $k) (i32.const 10))
                        (then
                            (i32.store8 (i32.const 32) (i32.const 70))
                            (i32.store8 (i32.const 33) (i32.add (i32.const 48) (local.get $b)))
                            (i32.store8 (i32.const 34) (i32.add (i32.const 48) (local.get $a)))
                            (i32.store8 (i32.const 35) (i32.add (i32.const 48) (local.get $c)))
                            (drop (call $log (i32.const 1) (i32.const 32) (i32.const 4)))))
                    (if (i32.eq (local.get $k) (i32.const 2))
                        (then
                            (i32.store8 (i32.const 32) (i32.const 80))
                            (drop (call $log (i32.const 1) (i32.const 32) (i32.const 1)))))
                    i32.const 1)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
        )
        .unwrap();
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("focus.wasm");
        std::fs::write(&path, &wasm).unwrap();
        Source::File { path, draft: None }
    }

    /// The module's own log lines since `from`, without the prefix.
    fn module_logs(shell: &crate::shell::Shell, tab: u32, from: usize) -> Vec<String> {
        shell.tab_logs(tab)[from..]
            .iter()
            .filter_map(|l| l.strip_prefix("[module info] ").map(str::to_string))
            .collect()
    }

    /// SPEC-10.2.1, SPEC-10.3.1: the page's tree goes to the platform
    /// through AccessKit, without a window here: every node with its
    /// platform role, its text as name or value, and its rectangle in
    /// physical window pixels under the page area; the host's focus is the
    /// update's focus (the address field while it is focused, the window
    /// when nothing is); the digest changes with the tree and focus, not
    /// otherwise. Actions: Focus moves the host's focus the way Tab does
    /// (`FOCUS` out and in, `c = 1`), Click presses and releases at the
    /// node's centre (the press moves focus first, `c = 0`), SetValue is
    /// not supported.
    #[test]
    fn spec_10_2_1_accesskit() {
        use crate::a11y::{self, node_id};
        use accesskit::{Action, ActionData, ActionRequest, Role, TreeId};
        let mut shell = headless_shell(None, 800.0, 600.0, 2.0).unwrap();
        let form = shell.open_tab(Some(local("form")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let sem = shell.tab_semantic(form);
        assert!(!sem.is_empty());
        let snap = shell.a11y_snapshot();
        assert_eq!(snap.sem, sem);
        let update = a11y::tree_update(&snap);
        assert_eq!(update.tree.as_ref().unwrap().root, a11y::WINDOW_ID);
        assert_eq!(update.tree_id, TreeId::ROOT);
        assert_eq!(update.focus, a11y::WINDOW_ID, "nothing focused");
        let by_id: std::collections::HashMap<_, _> =
            update.nodes.iter().map(|(i, n)| (*i, n)).collect();
        assert_eq!(update.nodes.len(), sem.len() + 3, "window, address, page");
        assert_eq!(by_id[&a11y::PAGE_ID].role(), Role::Document);
        let page = shell.layout().page;
        for n in &sem {
            let node = by_id[&node_id(n.id)];
            assert_eq!(node.role(), a11y::role_of(n.role), "{n:?}");
            let text = match node.role() {
                Role::Label | Role::TextInput => node.value(),
                _ => node.label(),
            };
            assert_eq!(text.unwrap_or(""), n.text, "{n:?}");
            let b = node.bounds().unwrap();
            assert_eq!(b.x0, f64::from((page.x + n.x) * 2.0));
            assert_eq!(b.y0, f64::from((page.y + n.y) * 2.0));
            assert_eq!(
                node.supports_action(Action::Focus),
                vy_abi::role::is_focusable(n.role)
            );
        }
        let fields: Vec<i32> = sem
            .iter()
            .filter(|n| n.role == vy_abi::role::TEXTBOX)
            .map(|n| n.id)
            .collect();
        assert!(fields.len() >= 2, "{sem:?}");
        // The digest follows what is shown.
        assert_eq!(shell.a11y_snapshot().digest(), snap.digest());
        shell.key(key::TAB, true, 0, false);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let focused = shell.tab_focus(form).unwrap();
        let snap2 = shell.a11y_snapshot();
        assert_ne!(snap2.digest(), snap.digest());
        assert_eq!(a11y::tree_update(&snap2).focus, node_id(focused));
        shell.key(crate::shell::hid::L, true, vy_abi::modifier::CTRL, false);
        assert_eq!(
            a11y::tree_update(&shell.a11y_snapshot()).focus,
            a11y::ADDRESS_ID
        );
        shell.key(key::ESCAPE, true, 0, false);
        // Focus action on the second field: the host's focus moves there.
        let request = |action, node| ActionRequest {
            action,
            target_tree: TreeId::ROOT,
            target_node: node,
            data: None,
        };
        let second = fields[1];
        assert!(shell.a11y_action(&request(Action::Focus, node_id(second))));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_focus(form), Some(second));
        // SetValue is the page's business.
        let mut set = request(Action::SetValue, node_id(second));
        set.data = Some(ActionData::Value("x".into()));
        assert!(!shell.a11y_action(&set));
        assert!(!shell.a11y_action(&request(Action::Focus, node_id(987_654))));
        // Focus on the address field.
        assert!(shell.a11y_action(&request(Action::Focus, a11y::ADDRESS_ID)));
        assert!(shell.address().focused());
        shell.key(key::ESCAPE, true, 0, false);
        shell.shutdown();

        // The deliveries, seen by a page that logs them.
        let dir = std::env::temp_dir().join(format!("vy-a11y-{}", std::process::id()));
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let tab = shell.open_tab(Some(focus_logger(&dir)));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let from = shell.tab_logs(tab).len();
        assert!(shell.a11y_action(&request(Action::Focus, node_id(2))));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert!(shell.a11y_action(&request(Action::Focus, node_id(3))));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert!(
            !shell.a11y_action(&request(Action::Focus, node_id(1))),
            "text is not focusable"
        );
        assert!(shell.a11y_action(&request(Action::Click, node_id(4))));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(
            module_logs(&shell, tab, from),
            ["F121", "F021", "F131", "F030", "F140", "P"],
            "keyboard focus in and out, then a click: focus moves by pointer before the press"
        );
        assert_eq!(shell.tab_focus(tab), Some(4));
        shell.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-10.3.1, SPEC-10.3.2: focus is the host's. On the form page,
    /// Tab walks the focusable nodes in document order and wraps to the
    /// first; Shift+Tab walks back and wraps to the last; a press on a
    /// focusable node focuses it and a press elsewhere takes focus away.
    /// The module draws its own focus mark from the deliveries, so its
    /// display list changes with focus. On a page that logs its events the
    /// deliveries themselves: out before in, `c = 1` for keys and `c = 0`
    /// for the pointer, and the focus change before the press.
    #[test]
    fn spec_10_3_1_focus_order() {
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let form = shell.open_tab(Some(local("form")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let order: Vec<i32> = shell
            .tab_semantic(form)
            .iter()
            .filter(|n| vy_abi::role::is_focusable(n.role))
            .map(|n| n.id)
            .collect();
        assert!(order.len() >= 3, "{order:?}");
        let tab_key = |shell: &mut crate::shell::Shell, shift: bool| {
            let m = if shift { vy_abi::modifier::SHIFT } else { 0 };
            shell.key(key::TAB, true, m, false);
            shell.key(key::TAB, false, m, false);
            settle(shell, 1, TIMEOUT).unwrap();
            shell.tab_focus(form)
        };
        let unfocused = shell.tab_display_list(form);
        let mut walked = Vec::new();
        for _ in 0..order.len() {
            walked.push(tab_key(&mut shell, false).unwrap());
        }
        assert_eq!(walked, order, "document order");
        assert_ne!(shell.tab_display_list(form), unfocused, "the mark is drawn");
        assert_eq!(tab_key(&mut shell, false), Some(order[0]), "wraps");
        assert_eq!(tab_key(&mut shell, true), order.last().copied(), "back");
        assert_eq!(
            tab_key(&mut shell, true),
            Some(order[order.len() - 2]),
            "back again"
        );
        // Pointer focus.
        let page = shell.layout().page;
        let sem = shell.tab_semantic(form);
        let target = sem.iter().find(|n| n.id == order[1]).unwrap().clone();
        shell.pointer_move(
            page.x + target.x + target.w / 2.0,
            page.y + target.y + target.h / 2.0,
        );
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_focus(form), Some(order[1]));
        let outside = |x: f32, y: f32| {
            !sem.iter().any(|n| {
                vy_abi::role::is_focusable(n.role)
                    && x >= n.x
                    && y >= n.y
                    && x < n.x + n.w
                    && y < n.y + n.h
            })
        };
        let (bx, by) = (0..40)
            .flat_map(|i| (0..40).map(move |j| (i as f32 * 10.0 + 3.0, j as f32 * 10.0 + 3.0)))
            .find(|&(x, y)| outside(x, y) && y < page.h)
            .unwrap();
        shell.pointer_move(page.x + bx, page.y + by);
        shell.pointer_button(button::PRIMARY, true);
        shell.pointer_button(button::PRIMARY, false);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_focus(form), None, "a press away from it");
        shell.shutdown();

        // The deliveries.
        let dir = std::env::temp_dir().join(format!("vy-focus-{}", std::process::id()));
        let mut shell = headless_shell(None, 800.0, 600.0, 1.0).unwrap();
        let tab = shell.open_tab(Some(focus_logger(&dir)));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        let from = shell.tab_logs(tab).len();
        for shift in [false, false, false, false, true] {
            let m = if shift { vy_abi::modifier::SHIFT } else { 0 };
            shell.key(key::TAB, true, m, false);
            settle(&mut shell, 1, TIMEOUT).unwrap();
        }
        let page = shell.layout().page;
        // The button (id 2) at (10, 40, 100, 30), pressed with the pointer.
        shell.pointer_move(page.x + 50.0, page.y + 55.0);
        shell.pointer_button(button::PRIMARY, true);
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(
            module_logs(&shell, tab, from),
            [
                "F121", // Tab: in 2
                "F021", "F131", // Tab: out 2, in 3
                "F031", "F141", // Tab: out 3, in 4
                "F041", "F121", // Tab: wraps from 4 to 2
                "F021", "F141", // Shift+Tab: back to 4
                "F040", "F120", "P", // press on 2: out 4, in 2 (pointer), then the press
            ]
        );
        shell.shutdown();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// SPEC-10.4.1: a page without a semantic tree runs, and the chrome
    /// marks it "otillgänglig" next to the address; a page with a tree is
    /// not marked; the accessibility tree says so too.
    #[test]
    fn spec_10_4_1_inaccessible_marker() {
        let marked = |shell: &mut crate::shell::Shell| {
            texts(&shell.chrome_display_list()).contains(&"otillgänglig".to_string())
        };
        let mut shell = headless_shell(None, 640.0, 400.0, 1.0).unwrap();
        let hello = shell.open_tab(Some(local("hello")));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(hello), "running");
        assert!(!marked(&mut shell), "hello has a tree");
        assert!(!shell.a11y_snapshot().inaccessible);
        // A module that draws but registers no node.
        let wasm = wat::parse_str(
            r#"(module
                (import "vy" "dl_begin" (func $dl_begin (result i32)))
                (import "vy" "dl_end" (func $dl_end (result i32)))
                (import "vy" "dl_rect" (func $dl_rect (param f32 f32 f32 f32 f32 i32) (result i32)))
                (memory (export "memory") 1)
                (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
                (func (export "vy_frame") (param i32) (result i32)
                    (drop (call $dl_begin))
                    (drop (call $dl_rect (f32.const 10) (f32.const 10) (f32.const 100) (f32.const 50) (f32.const 0) (i32.const 0x2F5BD6FF)))
                    (drop (call $dl_end))
                    i32.const 1)
                (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
                (func (export "vy_resize") (param i32 i32 f32))
                (func (export "vy_suspend") (result i32) i32.const 0)
                (func (export "vy_alloc") (param i32) (result i32) i32.const 0))"#,
        )
        .unwrap();
        let dir = std::env::temp_dir().join(format!("vy-notree-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notree.wasm");
        std::fs::write(&path, &wasm).unwrap();
        let bare = shell.open_tab(Some(Source::File { path, draft: None }));
        settle(&mut shell, 1, TIMEOUT).unwrap();
        assert_eq!(shell.tab_status(bare), "running");
        assert!(shell.tab_semantic(bare).is_empty());
        assert!(marked(&mut shell), "otillgänglig");
        let snap = shell.a11y_snapshot();
        assert!(snap.inaccessible);
        let update = crate::a11y::tree_update(&snap);
        assert!(update
            .nodes
            .iter()
            .any(|(id, n)| *id == crate::a11y::INACCESSIBLE_ID
                && n.value().is_some_and(|v| v.contains("otillgänglig"))));
        // Back to hello: the marker goes with the page.
        shell.switch_tab(true);
        assert_eq!(shell.active_tab(), Some(hello));
        assert!(!marked(&mut shell));
        shell.shutdown();
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
