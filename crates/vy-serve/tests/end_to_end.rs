//! End to end on loopback: publish the hello module with `vy-publish`,
//! serve it with `vy-serve`, fetch it with `vy-proto` as `vy://localhost`,
//! verify hashes and signature, run it in `vy-runtime` and render a frame.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use vy_manifest::{Draft, Grant, SigningKey};
use vy_proto::{Address, Fetcher, Form, Options};
use vy_runtime::{Capabilities, Runtime};
use vy_serve::{start, Config};

/// RFC 8032 §7.1 test vector 1: a publicly known key, never a secret.
const SEED: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vy-e2e-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Render the first frame of a fetched page in the runtime, with the
/// capabilities the default policy grants automatically and the package's
/// resources, each with the size the manifest declares (SPEC-8.1.2).
fn first_frame(fetched: &vy_proto::Fetched) -> Vec<u8> {
    let manifest = &fetched.package.manifest;
    let granted = fetched
        .grants()
        .into_iter()
        .filter(|(_, g)| *g == Grant::Auto)
        .map(|(name, _)| name);
    let resources = fetched
        .package
        .resources
        .iter()
        .zip(&manifest.resources)
        .map(|(bytes, r)| vy_runtime::Resource {
            bytes: bytes.clone(),
            pixels: r.w.zip(r.h),
        })
        .collect();
    let options = vy_runtime::Options {
        memory_max_pages: manifest.memory_max,
        capabilities: Capabilities::from_names(granted).unwrap(),
        resources,
        net_origins: manifest
            .capabilities
            .net
            .as_ref()
            .map(|n| n.origins.clone())
            .unwrap_or_default(),
        storage_quota_bytes: manifest
            .capabilities
            .storage
            .as_ref()
            .map(|s| u64::from(s.quota_kb) * 1024)
            .unwrap_or(vy_runtime::DEFAULT_STORAGE_QUOTA_BYTES),
        ..vy_runtime::Options::default()
    };
    let mut rt = Runtime::load(&fetched.package.module, &manifest.module.hash.0, options)
        .expect("module loads under its manifest");
    rt.init(800, 600, 1.0).unwrap();
    rt.frame().unwrap().to_vec()
}

/// SPEC-7.2.1 (and SPEC-7.1.3, 7.3.1, 7.3.2, 7.4.1, 9.4.1): a page
/// published, served and fetched over localhost renders identically from
/// the network and from the cache, with the module served exactly once.
#[test]
fn spec_7_2_1_end_to_end_localhost() {
    let wasm_path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/wasm/hello.wasm");
    let Ok(wasm) = std::fs::read(wasm_path) else {
        eprintln!("skipped: {wasm_path} is missing; run tools/build_modules.sh first");
        return;
    };

    // Publish.
    let key = SigningKey::from_seed(&SEED);
    let draft =
        Draft::from_json(include_str!("../../vy-manifest/tests/fixtures/hello.json")).unwrap();
    let package = vy_publish::build(&draft, wasm, vec![], &key).unwrap();
    let site_dir = temp_dir("site");
    std::fs::write(site_dir.join("hello.vyp"), package.pack().unwrap()).unwrap();
    let pub_path = site_dir.join("dev.key.pub");
    std::fs::write(&pub_path, vy_manifest::key_file(&[key.public()])).unwrap();

    // Serve.
    let server = start(
        Config {
            public_key_file: Some(pub_path),
            ..Config::local(&site_dir)
        },
        false,
    )
    .unwrap();

    // Fetch, twice, with one cache.
    let cache_dir = temp_dir("cache");
    let mut fetcher = Fetcher::new(Options::new(&cache_dir)).unwrap();
    let address = Address::parse(&format!("vy://localhost:{}/hello", server.port())).unwrap();
    assert!(address.is_localhost());
    let first = fetcher.fetch(&address).unwrap();
    assert_eq!(
        first.form,
        Form::Manifest,
        "both forms accepted: the server sends the manifest"
    );
    assert!(first.trusted, "signature verifies with the published key");
    assert_eq!(first.identity, package.identity());
    assert_eq!(first.package, package);
    // Manifest, module and key file.
    assert_eq!(first.requests, 3);

    let second = fetcher.fetch(&address).unwrap();
    assert_eq!(
        second.requests, 0,
        "fresh manifest, cached module: no network"
    );
    assert!(second.trusted);
    assert_eq!(second.package, package);

    let stats = server.stats();
    assert_eq!(stats.blobs, 1, "the module was served exactly once");
    assert_eq!(stats.manifests, 1);
    assert_eq!(stats.keys, 1);
    assert_eq!(stats.packages, 0);

    // Run.
    let frame_a = first_frame(&first);
    let frame_b = first_frame(&second);
    assert!(!frame_a.is_empty());
    assert_eq!(
        frame_a, frame_b,
        "same module, same input, same display list"
    );
    assert!(first.grants().is_empty(), "hello requests no capabilities");

    server.stop();
    let _ = std::fs::remove_dir_all(site_dir);
    let _ = std::fs::remove_dir_all(cache_dir);
}
