//! Conformance suite runner (SPEC-12.1.2, SPEC-12.2.2).
//!
//! `conformance/cases.toml` lists cases; each renders a module at a size and
//! scale and compares the pixels with `conformance/golden/<name>.png`. A case
//! may also name a semantic tree under `conformance/semantic/`, grant
//! capabilities to the headless host and replay input events before the
//! final frame, so focus, hover and scroll states are testable.

use crate::{render, RenderRequest, ScriptedEvent};
use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use vy_render::{diff_ratio, RgbaImage};
use vy_runtime::{Capabilities, SemNode};

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
    /// Events replayed after the first frame.
    pub events: Vec<ScriptedEvent>,
}

impl Case {
    fn golden(&self, root: &Path) -> PathBuf {
        root.join("conformance/golden")
            .join(format!("{}.png", self.name))
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

/// One event from its inline table: `kind` plus the arguments of that kind.
fn parse_event(table: &str, line: usize) -> Result<ScriptedEvent> {
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
        "pointer_move" => &["kind", "x", "y", "buttons", "modifiers"],
        "pointer_down" | "pointer_up" => &["kind", "x", "y", "button", "modifiers"],
        "scroll" => &["kind", "dx", "dy", "lines", "modifiers"],
        "key_down" => &["kind", "code", "modifiers", "repeat"],
        "key_up" => &["kind", "code", "modifiers"],
        "text_input" => &["kind", "text"],
        other => bail!("line {line}: unknown event kind {other:?}"),
    };
    for (k, _) in &pairs {
        if !known.contains(&k.as_str()) {
            bail!("line {line}: event {kind} has no key {k:?}");
        }
    }
    Ok(match kind {
        "pointer_move" => ScriptedEvent::PointerMove {
            x: need("x")?.num("x", line)? as f32,
            y: need("y")?.num("y", line)? as f32,
            buttons: int("buttons", 0)?,
            modifiers: int("modifiers", 0)?,
        },
        "pointer_down" => ScriptedEvent::PointerDown {
            x: need("x")?.num("x", line)? as f32,
            y: need("y")?.num("y", line)? as f32,
            button: int("button", 1)?,
            modifiers: int("modifiers", 0)?,
        },
        "pointer_up" => ScriptedEvent::PointerUp {
            x: need("x")?.num("x", line)? as f32,
            y: need("y")?.num("y", line)? as f32,
            button: int("button", 1)?,
            modifiers: int("modifiers", 0)?,
        },
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
        _ => ScriptedEvent::TextInput(need("text")?.str("text", line)?.to_string()),
    })
}

/// Parse the restricted TOML used by `cases.toml`: `[[case]]` tables holding
/// `key = value` lines with quoted strings, numbers, booleans, arrays of
/// strings (`capabilities`) and arrays of inline tables (`events`, which
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
        events: Vec<ScriptedEvent>,
    }
    fn finish(d: Draft, line: usize) -> Result<Case> {
        let name = d
            .name
            .ok_or_else(|| anyhow!("case before line {line}: missing name"))?;
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

/// SPEC-10.4.2: a page with input fields (`textbox` nodes) that was not
/// granted `ime` is rejected. Returns the problem, if any.
pub fn ime_requirement(semantic: &[SemNode], caps: Capabilities) -> Option<String> {
    let textboxes = semantic
        .iter()
        .filter(|n| n.role == vy_abi::role::TEXTBOX)
        .count();
    if textboxes > 0 && !caps.ime {
        return Some(format!(
            "{textboxes} textbox node(s) but capability `ime` is not granted (SPEC-10.4.2)"
        ));
    }
    None
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

fn run_case_inner(root: &Path, case: &Case, update: bool, result: &mut CaseResult) -> Result<()> {
    let module_path = root
        .join("target/wasm")
        .join(format!("{}.wasm", case.module));
    let wasm = fs::read(&module_path).with_context(|| {
        format!(
            "reading {} (run tools/build_modules.sh)",
            module_path.display()
        )
    })?;
    let req = RenderRequest {
        wasm: &wasm,
        width: case.width,
        height: case.height,
        scale: case.scale,
        frames: case.frames,
        memory_max_pages: vy_runtime::DEFAULT_MEMORY_MAX_PAGES,
        capabilities: case.capabilities,
        resources: &[],
        events: &case.events,
    };
    let out = render(&req)?;
    if let Some(problem) = ime_requirement(&out.semantic, case.capabilities) {
        bail!("{problem}");
    }
    let golden_path = case.golden(root);

    if update {
        fs::create_dir_all(golden_path.parent().unwrap())?;
        fs::write(&golden_path, out.image.encode_png()?)
            .with_context(|| format!("writing {}", golden_path.display()))?;
        result.diff = Some(0.0);
        if let Some(path) = case.semantic_path(root) {
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(&path, &out.semantic_json)
                .with_context(|| format!("writing {}", path.display()))?;
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
    let diff = diff_ratio(&golden, &out.image);
    result.diff = Some(diff);
    let mut problems = Vec::new();
    if diff > MAX_DIFF_RATIO {
        let new_path = golden_path.with_extension("png.new");
        fs::write(&new_path, out.image.encode_png()?)?;
        problems.push(format!(
            "{:.3} % of pixels differ (limit {:.1} %); actual written to {}",
            diff * 100.0,
            MAX_DIFF_RATIO * 100.0,
            new_path.display()
        ));
    }
    if let Some(path) = case.semantic_path(root) {
        let expected = fs::read_to_string(&path)
            .with_context(|| format!("no semantic tree {}", path.display()))?;
        let same = expected == out.semantic_json;
        result.semantic_ok = Some(same);
        if !same {
            let new_path = path.with_extension("json.new");
            fs::write(&new_path, &out.semantic_json)?;
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
  {kind = "scroll", dy = -120, lines = false},
]
"#;
        let cases = parse_cases(text).unwrap();
        assert_eq!(cases.len(), 1);
        let c = &cases[0];
        assert!(c.capabilities.ime && c.capabilities.storage && !c.capabilities.net);
        assert_eq!(c.frames, 8, "1 + 7 events");
        assert_eq!(
            c.events,
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
                ScriptedEvent::Scroll {
                    dx: 0.0,
                    dy: -120.0,
                    lines: false,
                    modifiers: 0
                },
            ]
        );
        let base = "[[case]]\nname = \"x\"\nmodule = \"m\"\nwidth = 1\nheight = 1\n";
        for bad in [
            "capabilities = [\"file\"]",
            "capabilities = \"ime\"",
            "events = [{kind = \"tap\", x = 1, y = 1}]",
            "events = [{kind = \"pointer_down\", x = 1}]",
            "events = [{kind = \"pointer_down\", x = 1, y = 1, z = 1}]",
            "events = [{kind = \"text_input\"}]",
            "events = [{kind = \"key_down\", code = \"a\"}]",
            "events = [{kind = \"scroll\", dy = 1, lines = 1}]",
            "events = [{kind = \"scroll\", dy = 1}",
        ] {
            assert!(parse_cases(&format!("{base}{bad}\n")).is_err(), "{bad}");
        }
        let explicit = parse_cases(&format!(
            "{base}frames = 1\nevents = [{{kind = \"key_down\", code = 1}}]\n"
        ))
        .unwrap();
        assert_eq!(explicit[0].frames, 1, "explicit frames wins");
    }

    /// SPEC-10.4.2: a page with input fields but no `ime` capability fails
    /// conformance; with `ime` granted, or without input fields, it passes.
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
        let ime = Capabilities::from_names(["ime"]).unwrap();
        assert!(ime_requirement(&textbox, Capabilities::default())
            .unwrap()
            .contains("SPEC-10.4.2"));
        assert_eq!(ime_requirement(&textbox, ime), None);
        assert_eq!(ime_requirement(&button, Capabilities::default()), None);
        assert_eq!(ime_requirement(&[], Capabilities::default()), None);

        // The real page: every form case grants ime, and the suite rejects
        // the same page when a case does not. With ime granted the host
        // accepts `ime_begin` on focus (SPEC-5.2.6); without it the call is
        // denied and logged (SPEC-5.3.1), and the page runs on regardless.
        let cases = load_cases(&root()).unwrap();
        let form: Vec<_> = cases.iter().filter(|c| c.module == "form").collect();
        assert!(!form.is_empty(), "the form page is in the suite");
        assert!(form.iter().all(|c| c.capabilities.ime));
        let filled = form
            .iter()
            .find(|c| c.name == "form-800x600@1-filled")
            .unwrap();
        let mut no_ime = (*filled).clone();
        no_ime.name = "form-no-ime".into();
        no_ime.capabilities = Capabilities::default();
        let r = run_case(&root(), &no_ime, false);
        assert!(
            r.problem.as_deref().unwrap_or("").contains("SPEC-10.4.2"),
            "{r:?}"
        );
        let wasm = fs::read(root().join("target/wasm/form.wasm")).unwrap();
        let logs = |caps: Capabilities| {
            let req = RenderRequest {
                wasm: &wasm,
                width: 800,
                height: 600,
                scale: 1.0,
                frames: no_ime.frames,
                memory_max_pages: vy_runtime::DEFAULT_MEMORY_MAX_PAGES,
                capabilities: caps,
                resources: &[],
                events: &no_ime.events,
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
