//! `vy-publish`: keygen, build and inspect packages.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use vy_manifest::parse_key_file;
use vy_publish::{
    build, inspect, keygen, load_resources, read_key, write_key, BuildError, SignatureStatus,
};

fn usage() -> ExitCode {
    eprintln!(
        "usage:\n  \
         vy-publish keygen --out <dev.key>\n  \
         vy-publish pubkey --key <dev.key> [--out <dev.key.pub>]\n  \
         vy-publish build --manifest <draft.json> --module <module.wasm> --key <dev.key> --out <page.vyp> [--resource <file>]...\n  \
         vy-publish inspect <page.vyp> [--public-key <dev.key.pub>]"
    );
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("keygen") => cmd_keygen(&args[1..]),
        Some("pubkey") => cmd_pubkey(&args[1..]),
        Some("build") => cmd_build(&args[1..]),
        Some("inspect") => cmd_inspect(&args[1..]),
        _ => return usage(),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Minimal `--flag value` parsing.
struct Args {
    positional: Vec<String>,
    flags: Vec<(String, String)>,
}

impl Args {
    fn parse(args: &[String], value_flags: &[&str]) -> Result<Self, String> {
        let mut positional = Vec::new();
        let mut flags = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if let Some(name) = a.strip_prefix("--") {
                if !value_flags.contains(&name) {
                    return Err(format!("unknown flag --{name}"));
                }
                let v = it.next().ok_or_else(|| format!("--{name} needs a value"))?;
                flags.push((name.to_string(), v.clone()));
            } else {
                positional.push(a.clone());
            }
        }
        Ok(Self { positional, flags })
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn values(&self, name: &str) -> Vec<PathBuf> {
        self.flags
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| PathBuf::from(v))
            .collect()
    }
}

fn cmd_keygen(args: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(args, &["out"])?;
    let Some(out) = a.value("out") else {
        return Ok(usage());
    };
    let key = keygen().map_err(|e| e.to_string())?;
    let pub_path = write_key(Path::new(out), &key).map_err(|e| e.to_string())?;
    println!("key_id: {}", key.public().key_id());
    println!("public: {}", key.public());
    println!("seed:   {out} (keep private, never commit)");
    println!("pub:    {}", pub_path.display());
    println!(
        "dns:    {} (TXT record at _vy.<host>, SPEC-7.4.5)",
        vy_manifest::dns_txt_record(&key.public())
    );
    Ok(ExitCode::SUCCESS)
}

fn cmd_pubkey(args: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(args, &["key", "out"])?;
    let Some(key_path) = a.value("key") else {
        return Ok(usage());
    };
    let key = read_key(Path::new(key_path)).map_err(|e| e.to_string())?;
    let text = vy_manifest::key_file(&[key.public()]);
    match a.value("out") {
        Some(out) => {
            std::fs::write(out, &text).map_err(|e| format!("{out}: {e}"))?;
            println!("key_id: {}", key.public().key_id());
            println!("pub:    {out}");
        }
        None => print!("{text}"),
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_build(args: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(args, &["manifest", "module", "key", "out", "resource"])?;
    let (Some(manifest), Some(module), Some(key), Some(out)) = (
        a.value("manifest"),
        a.value("module"),
        a.value("key"),
        a.value("out"),
    ) else {
        return Ok(usage());
    };
    let manifest_path = Path::new(manifest);
    let json = std::fs::read_to_string(manifest_path).map_err(|e| format!("{manifest}: {e}"))?;
    let draft = vy_manifest::Draft::from_json(&json).map_err(|e| format!("{manifest}: {e}"))?;
    let wasm = std::fs::read(module).map_err(|e| format!("{module}: {e}"))?;
    let resources =
        load_resources(manifest_path, &draft, &a.values("resource")).map_err(|e| e.to_string())?;
    let key = read_key(Path::new(key)).map_err(|e| e.to_string())?;
    let package = match build(&draft, wasm, resources, &key) {
        Ok(p) => p,
        Err(BuildError::Invalid(problems)) => {
            eprintln!("error: manifest does not describe the package:");
            for p in problems {
                eprintln!("  {p}");
            }
            return Ok(ExitCode::FAILURE);
        }
        Err(e) => return Err(e.to_string()),
    };
    let bytes = package.pack().map_err(|e| e.to_string())?;
    std::fs::write(out, &bytes).map_err(|e| format!("{out}: {e}"))?;
    println!("identity: {}", package.identity());
    println!("key_id:   {}", key.public().key_id());
    println!(
        "module:   {} bytes, {} resources, package {} bytes -> {out}",
        package.module.len(),
        package.resources.len(),
        bytes.len()
    );
    Ok(ExitCode::SUCCESS)
}

fn cmd_inspect(args: &[String]) -> Result<ExitCode, String> {
    let a = Args::parse(args, &["public-key"])?;
    let Some(path) = a.positional.first() else {
        return Ok(usage());
    };
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    let key = match a.value("public-key") {
        Some(p) => {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
            let keys = parse_key_file(&text).map_err(|e| format!("{p}: {e}"))?;
            keys.into_iter().next()
        }
        None => None,
    };
    let i = inspect(&bytes, key.as_ref()).map_err(|e| e.to_string())?;
    println!("{}", i.package.manifest.to_json());
    println!("identity:  {}", i.package.identity());
    println!(
        "package:   {} bytes compressed; module {} bytes; {} resources; hashes ok",
        i.compressed_bytes,
        i.package.module.len(),
        i.package.resources.len()
    );
    let key_id = i
        .package
        .manifest
        .sig
        .as_ref()
        .map(|s| s.key_id.clone())
        .unwrap_or_default();
    match i.signature {
        SignatureStatus::Valid => println!("signature: valid (key_id {key_id})"),
        SignatureStatus::Invalid => {
            println!("signature: INVALID for the key given (key_id {key_id})");
            return Ok(ExitCode::FAILURE);
        }
        SignatureStatus::NotChecked => {
            println!("signature: not checked, no --public-key (key_id {key_id})")
        }
    }
    Ok(ExitCode::SUCCESS)
}
