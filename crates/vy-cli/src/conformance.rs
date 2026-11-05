//! Conformance suite runner (SPEC-12.1.2, SPEC-12.2.2).
//!
//! `conformance/cases.toml` lists cases; each renders a module at a size and
//! scale and compares the pixels with `conformance/golden/<name>.png`. A case
//! may also name a semantic tree under `conformance/semantic/`, grant
//! capabilities to the headless host, declare canned responses for
//! `fetch_begin`, and replay input and host events before the final frame,
//! so focus, hover, scroll, paused, reflowed, persisted and delivered
//! states are testable.

use crate::{render, CannedResponse, RenderRequest, ScriptedEvent, Step};
use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use vy_manifest::{mime_for_path, CapabilityRequests, Draft, Manifest, ResourceInput, SigningKey};
use vy_render::{diff_ratio, Resource, RgbaImage};
use vy_runtime::{Capabilities, SemNode};
use vy_shell::page::{LoadedPage, Source};

/// Largest accepted fraction of differing pixels (0.1 %, SPEC-12.2.2).
pub const MAX_DIFF_RATIO: f64 = 0.001;
/// Path of the case list relative to the repository root.
pub const CASES_FILE: &str = "conformance/cases.toml";

/// One conformance case.
#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    /// Golden image name, `<module>-<WxH>@<scale>[-<state>]`.
    pub name: String,
    /// Module name under `target/wasm/`.
    pub module: String,
    /// Logical width.
    pub width: u32,
    /// Logical height.
    pub height: u32,
    /// DPI scale.
    pub scale: f32,
    /// Most frames to run before rasterizing (see `RenderRequest::frames`).
    pub frames: u32,
    /// Name under `conformance/semantic/` to compare the tree with, if any.
    pub semantic: Option<String>,
    /// Capabilities the headless host grants (SPEC-8.2.1).
    pub capabilities: Capabilities,
    /// Steps replayed after the first frame.
    pub events: Vec<Step>,
    /// Responses the headless host answers `fetch_begin` with (SPEC-5.2.7).
    pub fetch: Vec<CannedResponse>,
    /// Package resources as paths relative to the repository root;
    /// `resources[i]` becomes resource id `i + 1` (SPEC-4.5.1). The list
    /// must name the same files, in the same order, as the module's
    /// manifest draft; a size the case states must be the one the manifest
    /// declares (SPEC-8.1.2).
    pub resources: Vec<CaseResource>,
    /// True for a case of the whole client (SPEC-9.6.1, SPEC-11.3.2): the
    /// module runs in a tab process of the headless shell, with its
    /// manifest draft, and the golden is the composited frame, chrome
    /// included; `width` × `height` is the window. Such a case has no
    /// events, no semantic tree and grants nothing itself.
    pub shell: bool,
    /// For a shell case: the `vy://` address the page is taken to have
    /// been fetched from, signed with the suite's key and verified
    /// (SPEC-7.4.2), so the default policy decides its grants
    /// (SPEC-8.2.3) and the chrome shows the badges and the question.
    /// Without it a shell case is a local file in restricted mode
    /// (SPEC-7.4.3).
    pub address: Option<String>,
}

/// Seed of the key the suite signs a shell case's package with when the
/// case has an `address`; the headless shell trusts the page because the
/// suite verified the signature, as the client does for a fetched page
/// under a pinned key (SPEC-7.4.2). Only ever used here.
const SHELL_TRUST_SEED: [u8; 32] = *b"vy-conformance-shell-trust-key!!";

/// One entry of a case's `resources`: a bare path, or a table with the
/// declared image size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseResource {
    /// Path relative to the repository root.
    pub path: String,
    /// `(w, h)` as the manifest declares it, when stated.
    pub pixels: Option<(u32, u32)>,
}

impl Case {
    fn golden(&self, root: &Path) -> PathBuf {
        root.join("conformance/golden")
            .join(format!("{}.png", self.name))
    }
    /// The module's manifest draft, `conformance/manifests/<module>.json`
    /// (the publisher's input, SPEC-8.1.1; `vy-publish build` reads the
    /// same file).
    fn manifest_path(&self, root: &Path) -> PathBuf {
        root.join("conformance/manifests")
            .join(format!("{}.json", self.module))
    }
    fn semantic_path(&self, root: &Path) -> Option<PathBuf> {
        self.semantic
            .as_ref()
            .map(|s| root.join("conformance/semantic").join(format!("{s}.json")))
    }
}

/// A value in the restricted TOML of `cases.toml`.
#[derive(Clone, Debug, PartialEq)]
enum Scalar {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Scalar {
    fn parse(v: &str, line: usize) -> Result<Self> {
        let v = v.trim();
        if let Some(inner) = v.strip_prefix('"') {
            let inner = inner
                .strip_suffix('"')
                .ok_or_else(|| anyhow!("line {line}: unterminated string"))?;
            if inner.contains('"') || inner.contains('\\') {
                bail!("line {line}: escapes are not supported");
            }
            return Ok(Self::Str(inner.to_string()));
        }
        match v {
            "true" => return Ok(Self::Bool(true)),
            "false" => return Ok(Self::Bool(false)),
            _ => {}
        }
        if let Ok(i) = v.parse::<i64>() {
            return Ok(Self::Int(i));
        }
        if let Ok(f) = v.parse::<f64>() {
            return Ok(Self::Float(f));
        }
        bail!("line {line}: cannot parse value {v:?}")
    }

    fn str(&self, key: &str, line: usize) -> Result<&str> {
        match self {
            Self::Str(s) => Ok(s),
            _ => bail!("line {line}: {key} must be a quoted string"),
        }
    }

    fn num(&self, key: &str, line: usize) -> Result<f64> {
        match self {
            Self::Int(i) => Ok(*i as f64),
            Self::Float(f) => Ok(*f),
            _ => bail!("line {line}: {key} must be a number"),
        }
    }

    fn int(&self, key: &str, line: usize) -> Result<i64> {
        match self {
            Self::Int(i) => Ok(*i),
            _ => bail!("line {line}: {key} must be an integer"),
        }
    }

    fn bool(&self, key: &str, line: usize) -> Result<bool> {
        match self {
            Self::Bool(b) => Ok(*b),
            _ => bail!("line {line}: {key} must be true or false"),
        }
    }
}

/// Split `s` on `sep` at bracket depth 0 and outside quotes.
fn split_top(s: &str, sep: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut quoted, mut start) = (0i32, false, 0usize);
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '[' | '{' if !quoted => depth += 1,
            ']' | '}' if !quoted => depth -= 1,
            c if c == sep && !quoted && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// `s` without a trailing `#` comment; a `#` inside quotes is text.
fn strip_comment(s: &str) -> &str {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &s[..i],
            _ => {}
        }
    }
    s
}

/// Net bracket depth of `s` outside quotes.
fn depth(s: &str) -> i32 {
    let (mut depth, mut quoted) = (0i32, false);
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            '[' | '{' if !quoted => depth += 1,
            ']' | '}' if !quoted => depth -= 1,
            _ => {}
        }
    }
    depth
}

/// Parse `[ a, b, ... ]` into its top-level elements.
fn parse_array(v: &str, line: usize) -> Result<Vec<&str>> {
    let inner = v
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| anyhow!("line {line}: expected an array [...]"))?;
    Ok(split_top(inner, ',')
        .into_iter()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .collect())
}

/// Parse `{ key = value, ... }` into pairs.
fn parse_table(v: &str, line: usize) -> Result<Vec<(String, Scalar)>> {
    let inner = v
        .trim()
        .strip_prefix('{')
        .and_then(|s| s.strip_suffix('}'))
        .ok_or_else(|| anyhow!("line {line}: expected an inline table {{...}}"))?;
    split_top(inner, ',')
        .into_iter()
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .map(|pair| {
            let (k, v) = pair
                .split_once('=')
                .ok_or_else(|| anyhow!("line {line}: expected key = value in table"))?;
            Ok((k.trim().to_string(), Scalar::parse(v, line)?))
        })
        .collect()
}

/// One resource from a bare string or a `{path, w, h}` table.
fn parse_resource(entry: &str, line: usize) -> Result<CaseResource> {
    if entry.trim_start().starts_with('{') {
        let pairs = parse_table(entry, line)?;
        let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v);
        for (k, _) in &pairs {
            if !["path", "w", "h"].contains(&k.as_str()) {
                bail!("line {line}: resource has no key {k:?}");
            }
        }
        let path = get("path")
            .ok_or_else(|| anyhow!("line {line}: resource needs path"))?
            .str("path", line)?
            .to_string();
        let side = |key: &str| -> Result<u32> {
            let v = get(key)
                .ok_or_else(|| anyhow!("line {line}: resource needs {key}"))?
                .int(key, line)?;
            u32::try_from(v)
                .ok()
                .filter(|v| *v > 0)
                .ok_or_else(|| anyhow!("line {line}: {key} must be a positive integer"))
        };
        let pixels = match (get("w"), get("h")) {
            (None, None) => None,
            _ => Some((side("w")?, side("h")?)),
        };
        return Ok(CaseResource { path, pixels });
    }
    Ok(CaseResource {
        path: Scalar::parse(entry, line)?
            .str("resources", line)?
            .to_string(),
        pixels: None,
    })
}

/// One canned response from `{url, status, body}`.
fn parse_fetch(table: &str, line: usize) -> Result<CannedResponse> {
    let pairs = parse_table(table, line)?;
    let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v);
    for (k, _) in &pairs {
        if !["url", "status", "body"].contains(&k.as_str()) {
            bail!("line {line}: fetch has no key {k:?}");
        }
    }
    let url = get("url")
        .ok_or_else(|| anyhow!("line {line}: fetch needs url"))?
        .str("url", line)?
        .to_string();
    let status = get("status").map_or(Ok(200), |v| v.int("status", line))?;
    let status =
        i32::try_from(status).with_context(|| format!("line {line}: status out of range"))?;
    let body = get("body")
        .map_or(Ok(""), |v| v.str("body", line))?
        .as_bytes()
        .to_vec();
    Ok(CannedResponse { url, status, body })
}

/// One step from its inline table: `kind` plus the arguments of that kind,
/// and `after_frames` for any kind.
fn parse_event(table: &str, line: usize) -> Result<Step> {
    let pairs = parse_table(table, line)?;
    let get = |key: &str| pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v);
    let num = |key: &str, default: f64| -> Result<f32> {
        Ok(get(key).map_or(Ok(default), |v| v.num(key, line))? as f32)
    };
    let int = |key: &str, default: i64| -> Result<i32> {
        let v = get(key).map_or(Ok(default), |v| v.int(key, line))?;
        i32::try_from(v).with_context(|| format!("line {line}: {key} out of range"))
    };
    let flag = |key: &str| -> Result<bool> { get(key).map_or(Ok(false), |v| v.bool(key, line)) };
    let need = |key: &str| -> Result<&Scalar> {
        get(key).ok_or_else(|| anyhow!("line {line}: event needs {key}"))
    };
    let kind = need("kind")?.str("kind", line)?;
    let known: &[&str] = match kind {
        "pointer_move" => &["x", "y", "buttons", "modifiers"],
        "pointer_down" | "pointer_up" => &["x", "y", "button", "modifiers"],
        "pointer_leave" | "budget" | "suspend" | "reinit" | "fetch_done" => &[],
        "scroll" => &["dx", "dy", "lines", "modifiers"],
        "key_down" => &["code", "modifiers", "repeat"],
        "key_up" => &["code", "modifiers"],
        "text_input" => &["text"],
        "focus" => &["node", "gained", "keyboard"],
        "visibility" => &["visible"],
        "resize" => &["width", "height", "scale"],
        other => bail!("line {line}: unknown event kind {other:?}"),
    };
    for (k, _) in &pairs {
        if !(k == "kind" || k == "after_frames" || known.contains(&k.as_str())) {
            bail!("line {line}: event {kind} has no key {k:?}");
        }
    }
    let after_frames = match get("after_frames") {
        None => None,
        Some(v) => Some(
            u32::try_from(v.int("after_frames", line)?).with_context(|| {
                format!("line {line}: after_frames must be a non-negative integer")
            })?,
        ),
    };
    let side = |key: &str| -> Result<u32> {
        let v = need(key)?.int(key, line)?;
        u32::try_from(v)
            .ok()
            .filter(|v| *v > 0)
            .ok_or_else(|| anyhow!("line {line}: {key} must be a positive integer"))
    };
    let event = match kind {
        "pointer_move" => ScriptedEvent::PointerMove {
            x: need("x")?.num("x", line)? as f32,
            y: need("y")?.num("y", line)? as f32,
            buttons: int("buttons", 0)?,
            modifiers: int("modifiers", 0)?,
        },
        "pointer_down" => ScriptedEvent::PointerDown {
            x: need("x")?.num("x", line)? as f32,
            y: need("y")?.num("y", line)? as f32,
            button: int("button", i64::from(vy_abi::button::PRIMARY))?,
            modifiers: int("modifiers", 0)?,
        },
        "pointer_up" => ScriptedEvent::PointerUp {
            x: need("x")?.num("x", line)? as f32,
            y: need("y")?.num("y", line)? as f32,
            button: int("button", i64::from(vy_abi::button::PRIMARY))?,
            modifiers: int("modifiers", 0)?,
        },
        "pointer_leave" => ScriptedEvent::PointerLeave,
        "scroll" => ScriptedEvent::Scroll {
            dx: num("dx", 0.0)?,
            dy: need("dy")?.num("dy", line)? as f32,
            lines: flag("lines")?,
            modifiers: int("modifiers", 0)?,
        },
        "key_down" => ScriptedEvent::KeyDown {
            code: need("code")?.int("code", line)? as i32,
            modifiers: int("modifiers", 0)?,
            repeat: flag("repeat")?,
        },
        "key_up" => ScriptedEvent::KeyUp {
            code: need("code")?.int("code", line)? as i32,
            modifiers: int("modifiers", 0)?,
        },
        "text_input" => ScriptedEvent::TextInput(need("text")?.str("text", line)?.to_string()),
        "focus" => ScriptedEvent::Focus {
            node: need("node")?.int("node", line)? as i32,
            gained: get("gained").map_or(Ok(true), |v| v.bool("gained", line))?,
            keyboard: get("keyboard").map_or(Ok(true), |v| v.bool("keyboard", line))?,
        },
        "visibility" => ScriptedEvent::Visibility {
            visible: need("visible")?.bool("visible", line)?,
        },
        "budget" => ScriptedEvent::Budget,
        "resize" => ScriptedEvent::Resize {
            width: side("width")?,
            height: side("height")?,
            scale: num("scale", 1.0)?,
        },
        "suspend" => ScriptedEvent::Suspend,
        "reinit" => ScriptedEvent::Reinit,
        _ => ScriptedEvent::FetchDone,
    };
    Ok(Step {
        event,
        after_frames,
    })
}

/// Parse the restricted TOML used by `cases.toml`: `[[case]]` tables holding
/// `key = value` lines with quoted strings, numbers, booleans, arrays of
/// strings (`capabilities`), arrays of strings or `{path, w, h}` tables
/// (`resources`) and arrays of inline tables (`events` and `fetch`, which
/// may span lines); `#` comments.
///
/// This is deliberately not a TOML parser (CLAUDE.md: no dependency for what
/// fits in 200 lines); anything outside the subset is an error.
pub fn parse_cases(text: &str) -> Result<Vec<Case>> {
    #[derive(Default)]
    struct Draft {
        name: Option<String>,
        module: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
        scale: Option<f32>,
        frames: Option<u32>,
        semantic: Option<String>,
        capabilities: Capabilities,
        events: Vec<Step>,
        resources: Vec<CaseResource>,
        fetch: Vec<CannedResponse>,
        shell: bool,
        address: Option<String>,
    }
    fn finish(d: Draft, line: usize) -> Result<Case> {
        let name = d
            .name
            .ok_or_else(|| anyhow!("case before line {line}: missing name"))?;
        if d.shell
            && (!d.events.is_empty()
                || d.semantic.is_some()
                || !d.fetch.is_empty()
                || d.capabilities != Capabilities::default())
        {
            bail!("case {name}: a shell case has no events, fetch, semantic tree or capabilities (the policy grants, SPEC-8.2.3)");
        }
        if d.address.is_some() && !d.shell {
            bail!("case {name}: address is for shell cases");
        }
        Ok(Case {
            module: d
                .module
                .ok_or_else(|| anyhow!("case {name}: missing module"))?,
            width: d
                .width
                .ok_or_else(|| anyhow!("case {name}: missing width"))?,
            height: d
                .height
                .ok_or_else(|| anyhow!("case {name}: missing height"))?,
            scale: d.scale.unwrap_or(1.0),
            // One frame after init, then one per event that asks for it.
            frames: d.frames.unwrap_or(1 + d.events.len() as u32),
            semantic: d.semantic,
            capabilities: d.capabilities,
            events: d.events,
            resources: d.resources,
            fetch: d.fetch,
            shell: d.shell,
            address: d.address,
            name,
        })
    }

    let mut cases = Vec::new();
    let mut current: Option<Draft> = None;
    let mut lines = text.lines().enumerate().peekable();
    while let Some((i, raw)) = lines.next() {
        let line = i + 1;
        let content = strip_comment(raw).trim();
        if content.is_empty() {
            continue;
        }
        if content == "[[case]]" {
            if let Some(d) = current.take() {
                cases.push(finish(d, line)?);
            }
            current = Some(Draft::default());
            continue;
        }
        let Some(d) = current.as_mut() else {
            bail!("line {line}: key outside a [[case]] table");
        };
        let (key, value) = content
            .split_once('=')
            .ok_or_else(|| anyhow!("line {line}: expected key = value"))?;
        let key = key.trim();
        // An array may continue on following lines until its brackets balance.
        let mut value = value.trim().to_string();
        while depth(&value) > 0 {
            let Some((_, more)) = lines.next() else {
                bail!("line {line}: unterminated array");
            };
            value.push(' ');
            value.push_str(strip_comment(more).trim());
        }
        let value = value.as_str();
        match key {
            "name" => d.name = Some(Scalar::parse(value, line)?.str(key, line)?.to_string()),
            "module" => d.module = Some(Scalar::parse(value, line)?.str(key, line)?.to_string()),
            "semantic" => {
                d.semantic = Some(Scalar::parse(value, line)?.str(key, line)?.to_string())
            }
            "width" | "height" | "frames" => {
                let n = Scalar::parse(value, line)?.int(key, line)?;
                let n = u32::try_from(n).with_context(|| {
                    format!("line {line}: {key} must be a non-negative integer")
                })?;
                match key {
                    "width" => d.width = Some(n),
                    "height" => d.height = Some(n),
                    _ => d.frames = Some(n),
                }
            }
            "scale" => d.scale = Some(Scalar::parse(value, line)?.num(key, line)? as f32),
            "shell" => d.shell = Scalar::parse(value, line)?.bool(key, line)?,
            "address" => {
                let address = Scalar::parse(value, line)?.str(key, line)?.to_string();
                if !address.starts_with("vy://") {
                    bail!("line {line}: address must be a vy:// address");
                }
                d.address = Some(address);
            }
            "capabilities" => {
                let mut caps = Capabilities::default();
                for e in parse_array(value, line)? {
                    let name = Scalar::parse(e, line)?;
                    let name = name.str(key, line)?;
                    if !caps.grant(name) {
                        bail!("line {line}: unknown capability {name:?} (SPEC-8.2.1)");
                    }
                }
                d.capabilities = caps;
            }
            "events" => {
                d.events = parse_array(value, line)?
                    .into_iter()
                    .map(|e| parse_event(e, line))
                    .collect::<Result<_>>()?;
            }
            "resources" => {
                d.resources = parse_array(value, line)?
                    .into_iter()
                    .map(|e| parse_resource(e, line))
                    .collect::<Result<_>>()?;
            }
            "fetch" => {
                d.fetch = parse_array(value, line)?
                    .into_iter()
                    .map(|e| parse_fetch(e, line))
                    .collect::<Result<_>>()?;
            }
            other => bail!("line {line}: unknown key {other:?}"),
        }
    }
    if let Some(d) = current.take() {
        cases.push(finish(d, text.lines().count() + 1)?);
    }
    for (i, a) in cases.iter().enumerate() {
        if cases[i + 1..].iter().any(|b| b.name == a.name) {
            bail!("duplicate case name {:?}", a.name);
        }
    }
    Ok(cases)
}

/// Load `conformance/cases.toml` under `root`.
pub fn load_cases(root: &Path) -> Result<Vec<Case>> {
    let path = root.join(CASES_FILE);
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    parse_cases(&text)
}

/// SPEC-10.4.2: a page with input fields (`textbox` nodes) whose manifest
/// does not request `ime` is rejected. Returns the problem, if any.
pub fn ime_requirement(semantic: &[SemNode], requested: &CapabilityRequests) -> Option<String> {
    let textboxes = semantic
        .iter()
        .filter(|n| n.role == vy_abi::role::TEXTBOX)
        .count();
    if textboxes > 0 && !requested.requests("ime") {
        return Some(format!(
            "{textboxes} textbox node(s) but the manifest does not request `ime` (SPEC-10.4.2)"
        ));
    }
    None
}

/// SPEC-8.2.2: a case may grant only what the manifest requests. Returns
/// the first capability granted without being requested.
pub fn grants_within_manifest(
    caps: Capabilities,
    requested: &CapabilityRequests,
) -> Option<&'static str> {
    vy_abi::cap::NAMES
        .into_iter()
        .find(|name| caps.has(vy_abi::cap::id(name).unwrap_or(0)) && !requested.requests(name))
}

/// What a case runs: the module, the manifest finished from the module's
/// draft with the case's resources, and the resources as the host holds
/// them, each with the size the manifest declares (SPEC-8.1.2).
#[derive(Clone, Debug, PartialEq)]
pub struct CasePackage {
    /// The module bytes.
    pub wasm: Vec<u8>,
    /// The finished, unsigned manifest.
    pub manifest: Manifest,
    /// `resources[i]` is resource id `i + 1` (SPEC-4.5.1).
    pub resources: Vec<Resource>,
}

/// Read the module and the draft, finish the draft with the case's
/// resources and check that the result describes the package
/// (SPEC-4.1.1): the case names the draft's files in the draft's order,
/// a size the case states is the one the manifest declares, and
/// `Manifest::validate` finds nothing wrong (hashes, image headers,
/// `memory_max` against the module's initial memory, SPEC-8.1.3, and
/// every limit of §4 and §8).
pub fn load_case_package(root: &Path, case: &Case) -> Result<CasePackage> {
    let module_path = root
        .join("target/wasm")
        .join(format!("{}.wasm", case.module));
    let wasm = fs::read(&module_path).with_context(|| {
        format!(
            "reading {} (run tools/build_modules.sh)",
            module_path.display()
        )
    })?;
    let manifest_path = case.manifest_path(root);
    let manifest_text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("reading {} (SPEC-4.1.1)", manifest_path.display()))?;
    let draft =
        Draft::from_json(&manifest_text).with_context(|| format!("{}", manifest_path.display()))?;
    let manifest_dir = manifest_path.parent().unwrap_or(root);
    if draft.resources.len() != case.resources.len() {
        bail!(
            "case lists {} resources but {} declares {} (SPEC-4.1.1)",
            case.resources.len(),
            manifest_path.display(),
            draft.resources.len()
        );
    }
    let mut inputs = Vec::with_capacity(case.resources.len());
    for (i, (declared, given)) in draft.resources.iter().zip(&case.resources).enumerate() {
        let id = i + 1;
        let case_path = root.join(&given.path);
        let draft_path = manifest_dir.join(&declared.path);
        let same_file = match (fs::canonicalize(&case_path), fs::canonicalize(&draft_path)) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        if !same_file {
            bail!(
                "resource {id}: the case names {} but the manifest names {} (SPEC-4.5.1)",
                case_path.display(),
                draft_path.display()
            );
        }
        if let (Some(stated), Some(w), Some(h)) = (given.pixels, declared.w, declared.h) {
            if stated != (w, h) {
                bail!(
                    "resource {id}: the case states {}x{} but the manifest declares {w}x{h} (SPEC-8.1.2)",
                    stated.0,
                    stated.1
                );
            }
        }
        let bytes = fs::read(&case_path)
            .with_context(|| format!("reading resource {}", case_path.display()))?;
        inputs.push(ResourceInput {
            mime: declared
                .mime
                .clone()
                .unwrap_or_else(|| mime_for_path(&declared.path).to_string()),
            bytes,
            size_hint: declared.w.zip(declared.h),
        });
    }
    let manifest = draft.finish(&wasm, &inputs);
    let problems = manifest.validate(&wasm, &inputs.iter().map(|r| &r.bytes).collect::<Vec<_>>());
    if !problems.is_empty() {
        let lines: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
        bail!(
            "{} does not describe the package:\n  {}",
            manifest_path.display(),
            lines.join("\n  ")
        );
    }
    let resources = inputs
        .into_iter()
        .zip(&manifest.resources)
        .map(|(input, r)| Resource {
            bytes: input.bytes,
            pixels: r.w.zip(r.h),
        })
        .collect();
    Ok(CasePackage {
        wasm,
        manifest,
        resources,
    })
}

/// Outcome of one case.
#[derive(Clone, Debug, PartialEq)]
pub struct CaseResult {
    /// Case name.
    pub name: String,
    /// Fraction of differing pixels, or `None` when no golden exists.
    pub diff: Option<f64>,
    /// Semantic tree matched, or `None` when the case has none.
    pub semantic_ok: Option<bool>,
    /// What went wrong, if anything.
    pub problem: Option<String>,
}

impl CaseResult {
    /// True when the case passed the gate.
    pub fn ok(&self) -> bool {
        self.problem.is_none()
    }
}

/// Run every case. With `update`, goldens and semantic trees are rewritten
/// instead of compared (a golden change is its own `golden:` commit).
pub fn run(root: &Path, update: bool) -> Result<Vec<CaseResult>> {
    let cases = load_cases(root)?;
    if cases.is_empty() {
        bail!("{CASES_FILE} lists no cases");
    }
    let mut results = Vec::new();
    for case in &cases {
        results.push(run_case(root, case, update));
    }
    Ok(results)
}

fn run_case(root: &Path, case: &Case, update: bool) -> CaseResult {
    let mut result = CaseResult {
        name: case.name.clone(),
        diff: None,
        semantic_ok: None,
        problem: None,
    };
    match run_case_inner(root, case, update, &mut result) {
        Ok(()) => {}
        Err(e) => result.problem = Some(format!("{e:#}")),
    }
    result
}

/// The executable started as a tab process for shell cases: this one
/// (`vy-cli --tab`); from a test binary under `deps/`, the `vy-cli` next
/// to it, which cargo builds for the integration tests.
pub fn tab_executable() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if let Some(deps) = exe.parent().filter(|p| p.ends_with("deps")) {
        if let Some(profile) = deps.parent() {
            let candidate = profile.join(format!("vy-cli{}", std::env::consts::EXE_SUFFIX));
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    // The running image, even if the file has been replaced meanwhile.
    Ok(vy_shell::process::self_exe()?)
}

/// A shell case: the module and its draft through the headless shell,
/// this executable serving as the tab process (`vy-cli --tab`). With an
/// `address`, the page is handed to the shell as fetched and trusted
/// (see [`load_shell_page`]); without one it is a local file.
fn run_shell_case(root: &Path, case: &Case) -> Result<RgbaImage> {
    let options = vy_shell::headless::Options {
        width: case.width as f32,
        height: case.height as f32,
        scale: case.scale,
        max_ticks: case.frames.max(1),
        exe: Some(tab_executable()?),
        timeout: std::time::Duration::from_secs(120),
    };
    if let Some(address) = &case.address {
        let page = load_shell_page(root, case, address)?;
        return vy_shell::headless::screenshot_loaded(page, &options)
            .map_err(|e| anyhow!("shell: {e}"));
    }
    let module_path = root
        .join("target/wasm")
        .join(format!("{}.wasm", case.module));
    if !module_path.is_file() {
        bail!(
            "reading {} (run tools/build_modules.sh)",
            module_path.display()
        );
    }
    let source = vy_shell::page::Source::File {
        path: module_path,
        draft: Some(case.manifest_path(root)),
    };
    vy_shell::headless::screenshot(source, &options).map_err(|e| anyhow!("shell: {e}"))
}

/// The page of a shell case with an `address`: the case's package, signed
/// with the suite's key and verified, as if fetched from the address
/// under a pinned key (SPEC-7.4.2). The default policy then decides the
/// grants (SPEC-8.2.3) with the address's authority as the page's own
/// origin (SPEC-9.5.1): `net` to another origin, `clipboard` and
/// `persist_frame` are questions, which the chrome shows over the page.
fn load_shell_page(root: &Path, case: &Case, address: &str) -> Result<LoadedPage> {
    let CasePackage {
        wasm,
        mut manifest,
        resources,
    } = load_case_package(root, case)?;
    let Source::Address(address) =
        Source::parse(address).map_err(|e| anyhow!("case {}: {e}", case.name))?
    else {
        bail!("case {}: address must be a vy:// address", case.name);
    };
    let key = SigningKey::from_seed(&SHELL_TRUST_SEED);
    manifest.sign(&key);
    let trust = manifest
        .verify(&key.public())
        .ok_or_else(|| anyhow!("case {}: the package did not verify", case.name))?;
    let origin = address.origin();
    let authority = origin.trim_start_matches("vy://");
    let own = [origin.clone(), format!("https://{authority}")];
    let verdicts = manifest
        .capabilities
        .default_grants(Some(&trust), |n| n.origins.iter().all(|o| own.contains(o)));
    Ok(LoadedPage {
        display: address.to_string(),
        origin: Some(origin),
        sha256: manifest.module.hash.0,
        manifest: Some(manifest),
        wasm,
        resources: resources.into_iter().map(|r| (r.bytes, r.pixels)).collect(),
        trusted: true,
        verdicts,
        is_localhost: address.is_localhost(),
    })
}

fn run_case_inner(root: &Path, case: &Case, update: bool, result: &mut CaseResult) -> Result<()> {
    if case.shell {
        let image = run_shell_case(root, case)?;
        return compare_golden(root, case, update, result, &image, None);
    }
    let CasePackage {
        wasm,
        manifest,
        resources,
    } = load_case_package(root, case)?;
    if let Some(name) = grants_within_manifest(case.capabilities, &manifest.capabilities) {
        bail!("case grants `{name}` but the manifest does not request it (SPEC-8.2.2)");
    }
    // What the manifest requests shapes the host (SPEC-5.2.8, SPEC-9.3.1).
    let net_origins = manifest
        .capabilities
        .net
        .as_ref()
        .map(|n| n.origins.clone())
        .unwrap_or_default();
    let storage_quota_bytes = manifest
        .capabilities
        .storage
        .as_ref()
        .map(|s| u64::from(s.quota_kb) * 1024)
        .unwrap_or(vy_runtime::DEFAULT_STORAGE_QUOTA_BYTES);
    let req = RenderRequest {
        wasm: &wasm,
        width: case.width,
        height: case.height,
        scale: case.scale,
        frames: case.frames,
        memory_max_pages: manifest.memory_max,
        capabilities: case.capabilities,
        resources: &resources,
        events: &case.events,
        fetch: &case.fetch,
        net_origins: &net_origins,
        storage_quota_bytes,
    };
    let out = render(&req)?;
    if let Some(problem) = ime_requirement(&out.semantic, &manifest.capabilities) {
        bail!("{problem}");
    }
    compare_golden(
        root,
        case,
        update,
        result,
        &out.image,
        Some(&out.semantic_json),
    )
}

/// Compare (or, with `update`, write) the golden image and, when the case
/// has one, the semantic tree.
fn compare_golden(
    root: &Path,
    case: &Case,
    update: bool,
    result: &mut CaseResult,
    image: &RgbaImage,
    semantic_json: Option<&str>,
) -> Result<()> {
    let golden_path = case.golden(root);

    if update {
        fs::create_dir_all(golden_path.parent().unwrap())?;
        fs::write(&golden_path, image.encode_png()?)
            .with_context(|| format!("writing {}", golden_path.display()))?;
        result.diff = Some(0.0);
        if let (Some(path), Some(json)) = (case.semantic_path(root), semantic_json) {
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
            result.semantic_ok = Some(true);
        }
        return Ok(());
    }

    let golden_bytes = fs::read(&golden_path).with_context(|| {
        format!(
            "no golden {} (run conformance --update)",
            golden_path.display()
        )
    })?;
    let golden = RgbaImage::decode_png(&golden_bytes)?;
    let diff = diff_ratio(&golden, image);
    result.diff = Some(diff);
    let mut problems = Vec::new();
    if diff > MAX_DIFF_RATIO {
        let new_path = golden_path.with_extension("png.new");
        fs::write(&new_path, image.encode_png()?)?;
        problems.push(format!(
            "{:.3} % of pixels differ (limit {:.1} %); actual written to {}",
            diff * 100.0,
            MAX_DIFF_RATIO * 100.0,
            new_path.display()
        ));
    }
    if let (Some(path), Some(json)) = (case.semantic_path(root), semantic_json) {
        let expected = fs::read_to_string(&path)
            .with_context(|| format!("no semantic tree {}", path.display()))?;
        let same = expected == json;
        result.semantic_ok = Some(same);
        if !same {
            let new_path = path.with_extension("json.new");
            fs::write(&new_path, json)?;
            problems.push(format!(
                "semantic tree differs from {}; actual written to {}",
                path.display(),
                new_path.display()
            ));
        }
    }
    if !problems.is_empty() {
        bail!("{}", problems.join("; "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap()
    }

    /// SPEC-12.2.2: every case renders within 0.1 % of its golden image.
    #[test]
    fn spec_12_2_2_pixel_diff_gate() {
        let results = run(&root(), false).unwrap();
        assert!(results.len() >= 4, "{results:?}");
        for r in &results {
            assert!(
                r.ok(),
                "{}: {}",
                r.name,
                r.problem.clone().unwrap_or_default()
            );
            assert!(r.diff.unwrap() <= MAX_DIFF_RATIO);
        }
        // The gate itself: 0.1 % passes, more fails.
        let a = RgbaImage::new(100, 10);
        let mut b = a.clone();
        b.data[0] = 1;
        assert!(diff_ratio(&a, &b) <= MAX_DIFF_RATIO);
        b.data[4] = 1;
        assert!(diff_ratio(&a, &b) > MAX_DIFF_RATIO);
    }

    /// SPEC-12.1.2, SPEC-12.1.1: the suite is what defines a client, so it
    /// must cover what exists: every reference module under `modules/` has
    /// at least two cases (sizes or states) and a semantic tree, every
    /// case has its golden on disk, and no golden or tree is an orphan.
    #[test]
    fn spec_12_1_2_conformance_defines_client() {
        let root = root();
        let cases = load_cases(&root).unwrap();
        let modules: Vec<String> = fs::read_dir(root.join("modules"))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().join("Cargo.toml").is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(modules.len() >= 3, "{modules:?}");
        for m in &modules {
            let mine: Vec<_> = cases.iter().filter(|c| &c.module == m).collect();
            assert!(mine.len() >= 2, "module {m} has {} case(s)", mine.len());
            assert!(
                mine.iter().any(|c| c.semantic.is_some()),
                "module {m} has no semantic tree"
            );
        }
        for c in &cases {
            assert!(
                modules.contains(&c.module),
                "case {} names no module",
                c.name
            );
            assert!(c.golden(&root).is_file(), "case {} has no golden", c.name);
            if let Some(p) = c.semantic_path(&root) {
                assert!(p.is_file(), "case {} has no semantic tree file", c.name);
            }
        }
        let listed = |dir: &str, ext: &str| -> Vec<String> {
            fs::read_dir(root.join(dir))
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(ext))
                .collect()
        };
        for g in listed("conformance/golden", ".png") {
            let name = g.trim_end_matches(".png");
            assert!(cases.iter().any(|c| c.name == name), "orphan golden {g}");
        }
        for t in listed("conformance/semantic", ".json") {
            let name = t.trim_end_matches(".json");
            assert!(
                cases.iter().any(|c| c.semantic.as_deref() == Some(name)),
                "orphan semantic tree {t}"
            );
        }
    }

    /// SPEC-12.2.2: the case list parses and names the four hello cases.
    #[test]
    fn spec_12_2_2_cases_parse() {
        let cases = load_cases(&root()).unwrap();
        let names: Vec<&str> = cases.iter().map(|c| c.name.as_str()).collect();
        for expected in [
            "hello-800x600@1",
            "hello-1280x800@1",
            "hello-400x700@1",
            "hello-800x600@2",
        ] {
            assert!(names.contains(&expected), "{names:?}");
        }
        let c = cases.iter().find(|c| c.name == "hello-800x600@2").unwrap();
        assert_eq!((c.width, c.height, c.scale), (800, 600, 2.0));
        assert_eq!(c.module, "hello");
        assert_eq!(c.capabilities, Capabilities::default());
        assert!(c.events.is_empty());

        let parsed = parse_cases(
            "# comment\n[[case]]\nname = \"a\"\nmodule = \"m\"\nwidth = 1\nheight = 2\n\n[[case]]\nname = \"b\" # x\nmodule = \"m\"\nwidth = 3\nheight = 4\nscale = 1.5\nframes = 2\nsemantic = \"m\"\n",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1].scale, 1.5);
        assert_eq!(parsed[1].frames, 2);
        assert_eq!(parsed[1].semantic.as_deref(), Some("m"));
        assert!(parse_cases("name = \"x\"").is_err(), "key outside table");
        assert!(parse_cases(
            "[[case]]\nname = \"x\"\nmodule = \"m\"\nwidth = 1\nheight = 1\ncolour = 3"
        )
        .is_err());
        assert!(parse_cases("[[case]]\nname = \"x\"\nmodule = \"m\"\nwidth = 1\nheight = 1\n[[case]]\nname = \"x\"\nmodule = \"m\"\nwidth = 1\nheight = 1").is_err());
    }

    /// SPEC-5.1.3, SPEC-A.2.1, SPEC-8.2.1: a case may grant capabilities by
    /// name and script events of the supported kinds, across lines, with
    /// `#` and `,` inside strings left alone; `frames` defaults to one plus
    /// one per event; unknown kinds, keys and capabilities are errors.
    #[test]
    fn spec_5_1_3_cases_with_events_and_capabilities() {
        let text = r#"
[[case]]
name = "form-800x600@1-filled"
module = "form"
width = 800
height = 600
capabilities = ["ime", "storage"]
events = [                                      # comment after the bracket
  {kind = "pointer_down", x = 100, y = 200.5},   # click
  {kind = "pointer_up", x = 100, y = 200.5, button = 1, modifiers = 2},
  {kind = "text_input", text = "Anna, #1"},
  {kind = "key_down", code = 43, repeat = true}, {kind = "key_up", code = 43},
  {kind = "pointer_move", x = 1, y = 2, buttons = 1},
  {kind = "pointer_leave"},
  {kind = "scroll", dy = -120, lines = false},
  {kind = "focus", node = 12, gained = false, keyboard = false},
  {kind = "focus", node = 13},
  {kind = "visibility", visible = false, after_frames = 20},
  {kind = "visibility", visible = true},
  {kind = "budget", after_frames = 0},
  {kind = "resize", width = 400, height = 700, scale = 1.5},
  {kind = "suspend"}, {kind = "reinit"}, {kind = "fetch_done"},
]
resources = ["modules/shop/assets/01.png", {path = "modules/shop/assets/02.png", w = 240, h = 240}]
fetch = [{url = "https://chat.example/api/messages", status = 201, body = "ok"}, {url = "https://chat.example/x"}]
"#;
        let cases = parse_cases(text).unwrap();
        assert_eq!(cases.len(), 1);
        let c = &cases[0];
        assert!(c.capabilities.ime && c.capabilities.storage && !c.capabilities.net);
        assert_eq!(c.frames, 18, "1 + 17 events");
        assert_eq!(
            c.fetch,
            vec![
                CannedResponse {
                    url: "https://chat.example/api/messages".into(),
                    status: 201,
                    body: b"ok".to_vec()
                },
                CannedResponse {
                    url: "https://chat.example/x".into(),
                    status: 200,
                    body: Vec::new()
                }
            ]
        );
        assert_eq!(
            c.resources,
            vec![
                CaseResource {
                    path: "modules/shop/assets/01.png".into(),
                    pixels: None
                },
                CaseResource {
                    path: "modules/shop/assets/02.png".into(),
                    pixels: Some((240, 240))
                }
            ]
        );
        let after: Vec<Option<u32>> = c.events.iter().map(|s| s.after_frames).collect();
        assert_eq!(after[..10], [None; 10]);
        assert_eq!(
            after[10..],
            [Some(20), None, Some(0), None, None, None, None]
        );
        let events: Vec<ScriptedEvent> = c.events.iter().map(|s| s.event.clone()).collect();
        assert_eq!(
            events,
            vec![
                ScriptedEvent::PointerDown {
                    x: 100.0,
                    y: 200.5,
                    button: 1,
                    modifiers: 0
                },
                ScriptedEvent::PointerUp {
                    x: 100.0,
                    y: 200.5,
                    button: 1,
                    modifiers: 2
                },
                ScriptedEvent::TextInput("Anna, #1".into()),
                ScriptedEvent::KeyDown {
                    code: 43,
                    modifiers: 0,
                    repeat: true
                },
                ScriptedEvent::KeyUp {
                    code: 43,
                    modifiers: 0
                },
                ScriptedEvent::PointerMove {
                    x: 1.0,
                    y: 2.0,
                    buttons: 1,
                    modifiers: 0
                },
                ScriptedEvent::PointerLeave,
                ScriptedEvent::Scroll {
                    dx: 0.0,
                    dy: -120.0,
                    lines: false,
                    modifiers: 0
                },
                ScriptedEvent::Focus {
                    node: 12,
                    gained: false,
                    keyboard: false
                },
                ScriptedEvent::Focus {
                    node: 13,
                    gained: true,
                    keyboard: true
                },
                ScriptedEvent::Visibility { visible: false },
                ScriptedEvent::Visibility { visible: true },
                ScriptedEvent::Budget,
                ScriptedEvent::Resize {
                    width: 400,
                    height: 700,
                    scale: 1.5
                },
                ScriptedEvent::Suspend,
                ScriptedEvent::Reinit,
                ScriptedEvent::FetchDone,
            ]
        );
        let base = "[[case]]\nname = \"x\"\nmodule = \"m\"\nwidth = 1\nheight = 1\n";
        for bad in [
            "events = [{kind = \"visibility\"}]",
            "events = [{kind = \"visibility\", visible = 1}]",
            "events = [{kind = \"budget\", x = 1}]",
            "events = [{kind = \"resize\", width = 0, height = 1}]",
            "events = [{kind = \"resize\", width = 1}]",
            "events = [{kind = \"suspend\", after_frames = -1}]",
            "events = [{kind = \"reinit\", after_frames = \"x\"}]",
            "fetch = [{status = 200}]",
            "fetch = [{url = \"x\", foo = 1}]",
            "fetch = [{url = \"x\", status = \"ok\"}]",
            "fetch = \"x\"",
            "capabilities = [\"file\"]",
            "capabilities = \"ime\"",
            "events = [{kind = \"tap\", x = 1, y = 1}]",
            "events = [{kind = \"pointer_down\", x = 1}]",
            "events = [{kind = \"pointer_down\", x = 1, y = 1, z = 1}]",
            "events = [{kind = \"pointer_leave\", x = 1}]",
            "events = [{kind = \"text_input\"}]",
            "events = [{kind = \"key_down\", code = \"a\"}]",
            "events = [{kind = \"scroll\", dy = 1, lines = 1}]",
            "events = [{kind = \"scroll\", dy = 1}",
            "events = [{kind = \"focus\"}]",
            "events = [{kind = \"focus\", node = 1, gained = 1}]",
            "resources = \"a.png\"",
            "resources = [1]",
            "resources = [{path = \"a.png\", w = 1}]",
            "resources = [{path = \"a.png\", w = 0, h = 1}]",
            "resources = [{w = 1, h = 1}]",
            "resources = [{path = \"a.png\", w = 1, h = 1, hash = \"x\"}]",
        ] {
            assert!(parse_cases(&format!("{base}{bad}\n")).is_err(), "{bad}");
        }
        let explicit = parse_cases(&format!(
            "{base}frames = 1\nevents = [{{kind = \"key_down\", code = 1}}]\n"
        ))
        .unwrap();
        assert_eq!(explicit[0].frames, 1, "explicit frames wins");
        assert_eq!(explicit[0].events[0].after_frames, None);
        // A shell case takes an address and nothing the headless host
        // would do itself; the address is for shell cases only.
        let shell = parse_cases(&format!(
            "{base}shell = true\naddress = \"vy://pages.example/x\"\n"
        ))
        .unwrap();
        assert!(shell[0].shell);
        assert_eq!(shell[0].address.as_deref(), Some("vy://pages.example/x"));
        assert_eq!(
            parse_cases(&format!("{base}shell = true\n")).unwrap()[0].address,
            None
        );
        for bad in [
            "address = \"vy://pages.example/x\"",
            "shell = true\naddress = \"https://pages.example/x\"",
            "shell = true\naddress = 1",
            "shell = true\ncapabilities = [\"ime\"]",
            "shell = true\nsemantic = \"m\"",
            "shell = true\nevents = [{kind = \"pointer_leave\"}]",
            "shell = true\nfetch = [{url = \"x\"}]",
        ] {
            assert!(parse_cases(&format!("{base}{bad}\n")).is_err(), "{bad}");
        }
    }

    /// SPEC-10.4.2, SPEC-8.2.2: a page with input fields whose manifest does
    /// not request `ime` fails conformance; with `ime` requested, or without
    /// input fields, it passes. A case may grant only what the manifest
    /// requests, and every module in the suite has a manifest that
    /// describes its module and resources: `memory_max` leaves the module
    /// room to grow (SPEC-8.1.3) and the images are the size it declares
    /// (SPEC-8.1.2).
    #[test]
    fn spec_10_4_2_ime_required_for_input() {
        let node = |role: i32| SemNode {
            id: 1,
            parent_id: 0,
            role,
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            text: String::new(),
        };
        let textbox = [node(vy_abi::role::TEXTBOX)];
        let button = [node(vy_abi::role::BUTTON)];
        let read = |name: &str| {
            Draft::from_json(
                &fs::read_to_string(root().join(format!("conformance/manifests/{name}.json")))
                    .unwrap(),
            )
            .unwrap()
            .capabilities
        };
        let (form_requests, hello_requests) = (read("form"), read("hello"));
        assert!(form_requests.requests("ime") && !hello_requests.requests("ime"));
        assert!(ime_requirement(&textbox, &hello_requests)
            .unwrap()
            .contains("SPEC-10.4.2"));
        assert_eq!(ime_requirement(&textbox, &form_requests), None);
        assert_eq!(ime_requirement(&button, &hello_requests), None);
        assert_eq!(ime_requirement(&[], &hello_requests), None);
        let ime = Capabilities::from_names(["ime"]).unwrap();
        assert_eq!(grants_within_manifest(ime, &hello_requests), Some("ime"));
        assert_eq!(grants_within_manifest(ime, &form_requests), None);

        // The real suite: every form case grants ime (its manifest requests
        // it), a case granting what the manifest does not request is
        // refused, and every module's manifest fits its module.
        let cases = load_cases(&root()).unwrap();
        let form: Vec<_> = cases
            .iter()
            .filter(|c| c.module == "form" && !c.shell)
            .collect();
        assert!(!form.is_empty(), "the form page is in the suite");
        assert!(form.iter().all(|c| c.capabilities.ime));
        let filled = form
            .iter()
            .find(|c| c.name == "form-800x600@1-filled")
            .unwrap();
        let mut too_much = (*filled).clone();
        too_much.name = "form-net".into();
        too_much.capabilities = Capabilities::from_names(["ime", "net"]).unwrap();
        let r = run_case(&root(), &too_much, false);
        assert!(
            r.problem.as_deref().unwrap_or("").contains("SPEC-8.2.2"),
            "{r:?}"
        );
        let mut modules: Vec<String> = cases.iter().map(|c| c.module.clone()).collect();
        modules.sort();
        modules.dedup();
        assert_eq!(modules.len(), 11);
        for m in &modules {
            let first = cases.iter().find(|c| &c.module == m).unwrap();
            let package =
                load_case_package(&root(), first).unwrap_or_else(|e| panic!("{m}: {e:#}"));
            assert_eq!(package.manifest.resources.len(), first.resources.len());
            assert!(package
                .resources
                .iter()
                .zip(&first.resources)
                .all(|(held, stated)| stated.pixels.is_none() || held.pixels == stated.pixels));
            for c in cases.iter().filter(|c| &c.module == m) {
                assert_eq!(
                    grants_within_manifest(c.capabilities, &package.manifest.capabilities),
                    None,
                    "{}",
                    c.name
                );
            }
        }
        // A case must name the draft's files, in order, with its sizes;
        // the manifest must fit the module (SPEC-8.1.3).
        let gallery = cases
            .iter()
            .find(|c| c.name == "gallery-800x600@1")
            .unwrap();
        let mut swapped = gallery.clone();
        swapped.resources.swap(0, 1);
        let e = load_case_package(&root(), &swapped).unwrap_err();
        assert!(format!("{e:#}").contains("SPEC-4.5.1"), "{e:#}");
        let mut fewer = gallery.clone();
        fewer.resources.pop();
        let e = load_case_package(&root(), &fewer).unwrap_err();
        assert!(format!("{e:#}").contains("SPEC-4.1.1"), "{e:#}");
        let mut wrong_size = gallery.clone();
        wrong_size.resources[0].pixels = Some((200, 320));
        let e = load_case_package(&root(), &wrong_size).unwrap_err();
        assert!(format!("{e:#}").contains("SPEC-8.1.2"), "{e:#}");
        // A draft whose memory_max cannot hold the module, in a scratch
        // repository root with the real hello module.
        let scratch = std::env::temp_dir().join(format!("vy-cli-cramped-{}", std::process::id()));
        fs::create_dir_all(scratch.join("conformance/manifests")).unwrap();
        fs::create_dir_all(scratch.join("target/wasm")).unwrap();
        fs::copy(
            root().join("target/wasm/hello.wasm"),
            scratch.join("target/wasm/hello.wasm"),
        )
        .unwrap();
        let draft = fs::read_to_string(root().join("conformance/manifests/hello.json")).unwrap();
        assert!(draft.contains("\"memory_max\": 32"));
        fs::write(
            scratch.join("conformance/manifests/hello.json"),
            draft.replace("\"memory_max\": 32", "\"memory_max\": 1"),
        )
        .unwrap();
        let hello = cases.iter().find(|c| c.module == "hello").unwrap();
        let e = load_case_package(&scratch, hello).unwrap_err();
        assert!(format!("{e:#}").contains("SPEC-8.1.3"), "{e:#}");
        let _ = fs::remove_dir_all(&scratch);

        // Without ime granted the host denies `ime_begin` on focus and logs
        // it (SPEC-5.3.1); the page runs on regardless.
        let mut no_ime = (*filled).clone();
        no_ime.capabilities = Capabilities::default();
        let wasm = fs::read(root().join("target/wasm/form.wasm")).unwrap();
        let logs = |caps: Capabilities| {
            let req = RenderRequest {
                capabilities: caps,
                frames: no_ime.frames,
                events: &no_ime.events,
                ..RenderRequest::new(&wasm, 800, 600)
            };
            let out = render(&req).unwrap();
            assert_eq!(out.semantic[3].text, "Anna Lind", "the page still ran");
            out.logs
                .into_iter()
                .filter(|l| l.message.contains("E_DENIED"))
                .map(|l| l.message)
                .collect::<Vec<_>>()
        };
        assert!(logs(filled.capabilities).is_empty());
        let denied = logs(Capabilities::default());
        assert!(
            denied.iter().any(|m| m.starts_with("ime_begin: E_DENIED")),
            "{denied:?}"
        );
        assert!(denied.iter().any(|m| m.starts_with("ime_end: E_DENIED")));
    }
}
