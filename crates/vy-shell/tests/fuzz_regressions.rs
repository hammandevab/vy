//! The fuzz corpora on stable Rust (M4): every input under
//! `fuzz/corpus/<target>/` and `fuzz/artifacts/<target>/` goes through the
//! same entry points the cargo-fuzz targets use (`fuzz/src/entry.rs`), so
//! a crash a fuzzer once found, saved as an artifact, fails `cargo test`
//! forever. A deterministic mutation pass over the seeds runs the parsers
//! a few thousand times more on every test run. The parsers are the ones
//! that take bytes from outside: the manifest and its log (SPEC-4.1.3,
//! SPEC-7.6.1), display lists (SPEC-6.1.3), IPC (SPEC-11.2.1), `vy://`
//! addresses (SPEC-7.1.1), PNG (SPEC-6.5.1), the Markdown fallback
//! (SPEC-8.3.1) and DNS answers (SPEC-7.4.5).
//!
//! `VY_WRITE_FUZZ_SEEDS=1 cargo test -p vy-shell --test fuzz_regressions`
//! writes the generated seeds into `fuzz/corpus/` first.

#[allow(dead_code)]
#[path = "../../../fuzz/src/entry.rs"]
mod entry;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every saved input of a target, corpus first, in name order.
fn inputs(target: &str) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for kind in ["corpus", "artifacts"] {
        let dir = root().join("fuzz").join(kind).join(target);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        for p in files {
            let bytes = std::fs::read(&p).unwrap();
            out.push((p, bytes));
        }
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Every corpus and artifact input of every target runs without a panic,
/// and every target has seeds.
#[test]
fn fuzz_corpora_replay() {
    if std::env::var_os("VY_WRITE_FUZZ_SEEDS").is_some() {
        write_seeds();
    }
    for (name, run) in entry::TARGETS {
        let saved = inputs(name);
        assert!(
            !saved.is_empty(),
            "fuzz/corpus/{name} has no seeds (VY_WRITE_FUZZ_SEEDS=1)"
        );
        for (path, bytes) in &saved {
            let r = catch_unwind(AssertUnwindSafe(|| run(bytes)));
            assert!(r.is_ok(), "{name}: {} panics", path.display());
        }
    }
}

/// xorshift64*: the same mutations on every run and machine.
fn next(state: &mut u64) -> u64 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    state.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// One mutation of `seed`: flip bits, overwrite with an extreme byte,
/// truncate, duplicate a slice, or insert random bytes.
fn mutate(seed: &[u8], state: &mut u64) -> Vec<u8> {
    let mut m = seed.to_vec();
    for _ in 0..1 + next(state) % 4 {
        let at = if m.is_empty() {
            0
        } else {
            (next(state) % m.len() as u64) as usize
        };
        match next(state) % 5 {
            0 if !m.is_empty() => m[at] ^= 1 << (next(state) % 8),
            1 if !m.is_empty() => m[at] = [0x00, 0xFF, 0x7F, 0x80][(next(state) % 4) as usize],
            2 => m.truncate(at),
            3 if !m.is_empty() => {
                let end = (at + 1 + (next(state) % 16) as usize).min(m.len());
                let piece = m[at..end].to_vec();
                m.splice(at..at, piece);
            }
            _ => {
                let n = 1 + next(state) % 8;
                for _ in 0..n {
                    m.insert(at.min(m.len()), next(state) as u8);
                }
            }
        }
    }
    m
}

/// Deterministic mutations of the first seeds of every target: no panic.
/// `VY_FUZZ_SMOKE_ROUNDS` (default 250 per seed) runs more by hand.
#[test]
fn fuzz_mutation_smoke() {
    let rounds: usize = std::env::var("VY_FUZZ_SMOKE_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(250);
    let mut state = std::env::var("VY_FUZZ_SMOKE_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0x9E37_79B9_7F4A_7C15u64);
    for (name, run) in entry::TARGETS {
        for (path, seed) in inputs(name).iter().take(6) {
            for _ in 0..rounds {
                let m = mutate(seed, &mut state);
                let r = catch_unwind(AssertUnwindSafe(|| run(&m)));
                assert!(
                    r.is_ok(),
                    "{name}: a mutation of {} panics; input {}",
                    path.display(),
                    hex(&m)
                );
            }
        }
    }
}

fn write(target: &str, name: &str, bytes: &[u8]) {
    let dir = root().join("fuzz/corpus").join(target);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(name), bytes).unwrap();
}

fn module(name: &str) -> Vec<u8> {
    let p = root().join(format!("target/wasm/{name}.wasm"));
    std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}; run tools/build_modules.sh", p.display()))
}

/// Seeds from the conformance material: the reference manifests signed
/// with a fixed test key, a packed package, a key log, display lists the
/// reference modules draw, IPC messages, addresses, small PNGs, the
/// fallback texts and DNS answers.
fn write_seeds() {
    use vy_manifest::{Draft, KeyLog, Package, SigningKey};
    let key = SigningKey::from_seed(&[7; 32]);
    let manifests = root().join("conformance/manifests");
    let mut names: Vec<String> = std::fs::read_dir(&manifests)
        .unwrap()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()?
                .strip_suffix(".json")
                .map(str::to_string)
        })
        .collect();
    names.sort();
    for name in &names {
        let path = manifests.join(format!("{name}.json"));
        let draft = Draft::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let wasm = module(name);
        let resources = draft.load_resources(Path::new(&manifests)).unwrap();
        let mut m = draft.finish(&wasm, &resources);
        m.sign(&key);
        write("manifest", &format!("{name}.cbor"), &m.encode().unwrap());
        if let Some(text) = &draft.fallback {
            write("markdown", &format!("{name}.md"), text.as_bytes());
        }
        if name == "hello" {
            let p = Package {
                manifest: m.clone(),
                module: wasm.clone(),
                resources: resources.into_iter().map(|r| r.bytes).collect(),
            };
            write("manifest", "hello.vyp", &p.pack().unwrap());
        }
    }
    let mut log = KeyLog::default();
    log.append("exempel.se", key.public(), "2026-11-05");
    log.append(
        "localhost:8080",
        SigningKey::from_seed(&[8; 32]).public(),
        "2026-11-06",
    );
    write("manifest", "log-entry.cbor", &log.entries[0].encode());
    write("manifest", "vy-log.txt", log.text().as_bytes());

    // Display lists as the reference modules draw them, and IPC frames.
    use vy_shell::ipc::{ToShell, ToTab};
    for name in ["hello", "shop", "text", "dashboard"] {
        let wasm = module(name);
        let mut rt = vy_runtime::Runtime::load(
            &wasm,
            &vy_runtime::sha256(&wasm),
            vy_runtime::Options::default(),
        )
        .unwrap();
        rt.init(400, 300, 1.0).unwrap();
        let dl = rt.frame().unwrap().to_vec();
        write("display_list", &format!("{name}.dl"), &dl);
        if name == "hello" {
            let frame = ToShell::Frame {
                display_list: dl,
                styles: Vec::new(),
                wants_more: false,
                over_budget: false,
            };
            write("ipc", "frame.bin", &frame.encode());
        }
    }
    let messages: Vec<(&str, Vec<u8>)> = vec![
        ("tick.bin", ToTab::Tick { clock_ms: 16 }.encode()),
        (
            "event.bin",
            ToTab::Event {
                kind: 3,
                a: 1,
                b: 2,
                c: 1,
                d: 0,
                clock_ms: 32,
            }
            .encode(),
        ),
        (
            "text.bin",
            ToTab::TextInput {
                text: "hej å".into(),
                clock_ms: 48,
            }
            .encode(),
        ),
        (
            "resize.bin",
            ToTab::Resize {
                width: 800,
                height: 600,
                scale: 1.5,
            }
            .encode(),
        ),
        (
            "fetch-response.bin",
            ToTab::FetchResponse {
                id: 1,
                status: 200,
                body: b"{}".to_vec(),
            }
            .encode(),
        ),
        (
            "fetch-request.bin",
            ToShell::FetchRequest {
                id: 1,
                method: 0,
                url: "https://chat.example/api".into(),
                body: Vec::new(),
            }
            .encode(),
        ),
        (
            "navigate.bin",
            ToShell::Navigate {
                url: "vy://exempel.se/".into(),
                replace: false,
            }
            .encode(),
        ),
        (
            "storage.bin",
            ToShell::Storage(vec![(b"k".to_vec(), b"v".to_vec())]).encode(),
        ),
        (
            "log.bin",
            ToShell::Log {
                module: true,
                level: 1,
                message: "init".into(),
            }
            .encode(),
        ),
        ("shutdown.bin", ToTab::Shutdown.encode()),
        // m4/ux: the copy gesture (SPEC-8.2.4), devtools recording, focus.
        ("copy-gesture.bin", ToTab::CopyGesture.encode()),
        ("stop-recording.bin", ToTab::StopRecording.encode()),
        ("focus.bin", ToShell::Focus(Some(-10)).encode()),
    ];
    for (name, bytes) in messages {
        write("ipc", name, &bytes);
    }

    for (i, a) in [
        "vy://exempel.se/",
        "vy://localhost:8080/hello?x=1",
        "vy://[::1]:8443/a/b",
        "vy://EXEMPEL.se:443/sida",
        "https://exempel.se/a/b\n../c?d",
    ]
    .iter()
    .enumerate()
    {
        write("address", &format!("{i}.txt"), a.as_bytes());
    }

    let res = root().join("modules/gallery/res");
    let mut pngs: Vec<PathBuf> = std::fs::read_dir(&res)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    pngs.sort_by_key(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(u64::MAX));
    for p in pngs.iter().take(2) {
        write(
            "png",
            p.file_name().unwrap().to_str().unwrap(),
            &std::fs::read(p).unwrap(),
        );
    }

    // DNS: an id, then an answer with a v=vy1 record and one with a CNAME.
    let q = vy_proto::dns::build_query(0x1234, "_vy.exempel.se").unwrap();
    let record = vy_manifest::dns_txt_record(&key.public());
    let mut a = q.clone();
    a[2..4].copy_from_slice(&0x8180u16.to_be_bytes());
    a[6..8].copy_from_slice(&1u16.to_be_bytes());
    a.extend_from_slice(&[0xC0, 12, 0, 16, 0, 1, 0, 0, 1, 44]);
    a.extend_from_slice(&((record.len() + 1) as u16).to_be_bytes());
    a.push(record.len() as u8);
    a.extend_from_slice(record.as_bytes());
    let mut seed = vec![0x12, 0x34];
    seed.extend_from_slice(&a);
    write("dns", "txt.bin", &seed);
    let mut nx = q.clone();
    nx[2..4].copy_from_slice(&0x8183u16.to_be_bytes());
    let mut seed = vec![0x12, 0x34];
    seed.extend_from_slice(&nx);
    write("dns", "nxdomain.bin", &seed);
}
