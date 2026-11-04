//! Conformance suite runner (SPEC-12.1.2, SPEC-12.2.2).
//!
//! `conformance/cases.toml` lists cases; each renders a module at a size and
//! scale and compares the pixels with `conformance/golden/<name>.png`. A case
//! may also name a semantic tree under `conformance/semantic/`.

use crate::{render, RenderRequest};
use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use vy_render::{diff_ratio, RgbaImage};

/// Largest accepted fraction of differing pixels (0.1 %, SPEC-12.2.2).
pub const MAX_DIFF_RATIO: f64 = 0.001;
/// Path of the case list relative to the repository root.
pub const CASES_FILE: &str = "conformance/cases.toml";

/// One conformance case.
#[derive(Clone, Debug, PartialEq)]
pub struct Case {
    /// Golden image name, `<module>-<WxH>@<scale>`.
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

/// Parse the restricted TOML used by `cases.toml`: `[[case]]` tables holding
/// `key = value` lines with quoted strings, integers or floats; `#` comments.
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
            frames: d.frames.unwrap_or(1),
            semantic: d.semantic,
            name,
        })
    }
    fn unquote(v: &str, line: usize) -> Result<String> {
        let inner = v
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or_else(|| anyhow!("line {line}: expected a quoted string"))?;
        if inner.contains('"') || inner.contains('\\') {
            bail!("line {line}: escapes are not supported");
        }
        Ok(inner.to_string())
    }

    let mut cases = Vec::new();
    let mut current: Option<Draft> = None;
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let content = raw.split('#').next().unwrap_or("").trim();
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
        let (key, value) = (key.trim(), value.trim());
        match key {
            "name" => d.name = Some(unquote(value, line)?),
            "module" => d.module = Some(unquote(value, line)?),
            "semantic" => d.semantic = Some(unquote(value, line)?),
            "width" | "height" | "frames" => {
                let n: u32 = value
                    .parse()
                    .with_context(|| format!("line {line}: {key} must be an integer"))?;
                match key {
                    "width" => d.width = Some(n),
                    "height" => d.height = Some(n),
                    _ => d.frames = Some(n),
                }
            }
            "scale" => {
                d.scale = Some(
                    value
                        .parse()
                        .with_context(|| format!("line {line}: scale must be a number"))?,
                )
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
        capabilities: vy_runtime::Capabilities::default(),
        resources: &[],
    };
    let out = render(&req)?;
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
}
