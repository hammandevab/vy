//! Studio end to end, through the client's own code: `vy-dev serve` fits
//! the Studio package to this run (origin and token in the module, `net`
//! to that origin, signed with the developer key); `vy-proto` fetches it
//! from `vy://localhost:<studio-port>/` and trusts it; its own origin is
//! granted without asking (SPEC-8.2.3); `vy-runtime` runs the real module,
//! and every request it makes is checked by the rule the host process
//! applies and performed by `Fetcher::request`, as the shell's loader
//! does. A mission typed into the field and run with Kör reaches the agent
//! (`tools/fake_agent.sh`), whose log and the build after it show in
//! Studio's tree.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use vy_abi::event as kind;
use vy_dev::dev::BuildStatus;
use vy_dev::serve::{start, ServeConfig};
use vy_manifest::{Draft, Grant, SigningKey};
use vy_proto::{Address, Fetcher, Options};
use vy_runtime::{Capabilities, Runtime, Transport};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Run Studio until `done` holds for its semantic tree: frames when it
/// asks, its requests performed one at a time like the shell's loader.
fn run_until(
    rt: &mut Runtime,
    fetcher: &Fetcher,
    authority: &str,
    done: impl Fn(&[vy_runtime::SemNode]) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        rt.frame().unwrap();
        if done(rt.semantic_tree()) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{:#?}",
            rt.semantic_tree()
                .iter()
                .map(|n| n.text.clone())
                .collect::<Vec<_>>()
        );
        for req in rt.take_fetch_requests() {
            let origin = vy_runtime::origin_of(&req.url).unwrap();
            assert!(
                vy_runtime::plain_http_allowed(&origin, authority),
                "{origin} from {authority}"
            );
            let method = vy_abi::method::name(req.method).unwrap();
            let (status, body) = match fetcher.request(method, &req.url, &req.body, true, 1 << 26) {
                Ok((s, b)) => (i32::from(s), b),
                Err(e) => panic!("{}: {e}", req.url),
            };
            rt.fetch_complete(req.id, status, body).unwrap();
        }
    }
}

fn has(tree: &[vy_runtime::SemNode], text: &str) -> bool {
    tree.iter().any(|n| n.text.contains(text))
}

#[test]
fn studio_runs_a_mission_through_the_client_path() {
    let wasm_path = root().join("target/wasm/studio.wasm");
    let Ok(wasm) = std::fs::read(&wasm_path) else {
        eprintln!(
            "skipped: {} is missing; run tools/build_modules.sh first",
            wasm_path.display()
        );
        return;
    };
    let tmp = std::env::temp_dir().join(format!("vy-dev-studio-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // The Studio package as tools/build_studio.sh makes it: a throwaway key.
    let draft_path = root().join("conformance/manifests/studio.json");
    let draft = Draft::from_json(&std::fs::read_to_string(&draft_path).unwrap()).unwrap();
    let throwaway = SigningKey::from_seed(&[5; 32]);
    let package = vy_publish::build(&draft, wasm.clone(), vec![], &throwaway).unwrap();
    let studio_vyp = tmp.join("studio.vyp");
    std::fs::write(&studio_vyp, package.pack().unwrap()).unwrap();

    let project = tmp.join("studio-sida");
    vy_dev::scaffold::Scaffold {
        dir: project.clone(),
        name: "studio-sida".into(),
        sdk: Some(root()),
    }
    .create()
    .unwrap();
    let fake = std::path::absolute(root().join(if cfg!(windows) {
        "tools/fake_agent.ps1"
    } else {
        "tools/fake_agent.sh"
    }))
    .unwrap();
    let agent_cmd = if cfg!(windows) {
        format!(
            "powershell -NoProfile -ExecutionPolicy Bypass -File \"{}\"",
            fake.display()
        )
    } else {
        format!("\"{}\"", fake.display())
    };
    let handle = start(ServeConfig {
        port: 0,
        studio_port: 0,
        key_path: Some(tmp.join("dev.key")),
        target_dir: Some(root().join("target").join("vy-dev-tests")),
        agent_cmd: Some(agent_cmd),
        studio_package: Some(studio_vyp.clone()),
        ..ServeConfig::new(&project)
    })
    .unwrap();
    assert_eq!(handle.studio_package.as_deref(), Some(studio_vyp.as_path()));
    let (a, b) = (handle.port, handle.studio_port);
    let dev = &handle.dev;
    let token = dev.access().token.clone();
    assert!(dev.wait_until(Duration::from_secs(600), |s| s.builds >= 1
        && s.build != BuildStatus::Building
        && !s.build_requested));

    // The client fetches Studio and trusts it with the developer key; it is
    // fitted to this run, and its `net` is its own origin, granted.
    let mut fetcher = Fetcher::new(Options::new(tmp.join("cache"))).unwrap();
    let studio = fetcher
        .fetch(&Address::parse(&format!("vy://localhost:{b}/")).unwrap())
        .unwrap();
    assert!(studio.trusted());
    let own = format!("http://localhost:{b}");
    let manifest = &studio.package.manifest;
    assert_eq!(
        manifest.capabilities.net.as_ref().unwrap().origins,
        std::slice::from_ref(&own)
    );
    let module = &studio.package.module;
    let has_bytes = |needle: &[u8]| module.windows(needle.len()).any(|w| w == needle);
    assert!(has_bytes(
        format!("VYDEV-STUDIO-CONFIG {own} {token}").as_bytes()
    ));
    assert!(
        !has_bytes(b"00000000000000000000000000000000"),
        "the placeholder is gone"
    );
    assert_eq!(module.len(), wasm.len(), "patched in place");
    let grants = studio.grants();
    for cap in ["ime", "net", "storage"] {
        assert!(grants.contains(&(cap, Grant::Auto)), "{cap}: {grants:?}");
    }

    // Run it as the shell would.
    let options = vy_runtime::Options {
        memory_max_pages: manifest.memory_max,
        capabilities: Capabilities::from_names(["ime", "net", "storage"]).unwrap(),
        net_origins: vec![own.clone()],
        transport: Transport::External,
        ..vy_runtime::Options::default()
    };
    let mut rt = Runtime::load(module, &manifest.module.hash.0, options).unwrap();
    rt.init(800, 600, 1.0).unwrap();
    let authority = format!("localhost:{b}");
    let page_address = format!("vy://localhost:{a}/studio-sida");
    run_until(&mut rt, &fetcher, &authority, |t| {
        has(t, &page_address) && has(t, "Status: klar")
    });
    assert!(
        rt.links().iter().any(|l| l.url == page_address),
        "the address is a link"
    );

    // Type a mission and press Kör: the agent runs, Studio shows its log.
    let x100 = |v: f32| (v * 100.0) as i32;
    rt.event(kind::POINTER_DOWN, x100(282.0), x100(124.0), 1, 0)
        .unwrap();
    rt.text_input("gör en sida som hälsar från agenten")
        .unwrap();
    rt.event(kind::POINTER_DOWN, x100(608.0), x100(124.0), 1, 0)
        .unwrap();
    run_until(&mut rt, &fetcher, &authority, |t| {
        has(t, "Klart: sidan hälsar från agenten.")
            && has(t, "Status: klar")
            && has(t, "agenten är klar")
    });
    assert_eq!(
        std::fs::read_to_string(project.join("fake-agent-mission.txt")).unwrap(),
        "gör en sida som hälsar från agenten"
    );
    assert!(has(rt.semantic_tree(), "› Edit src/lib.rs"));
    assert!(rt.storage().contains_key("studio.mission".as_bytes()));
    handle.stop();
    let _ = std::fs::remove_dir_all(&tmp);
}
