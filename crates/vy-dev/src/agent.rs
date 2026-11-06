//! Claude Code as a subprocess: the command and how it is found (on
//! Windows through `PATH` and `PATHEXT`), how it is started headless with
//! the mission on stdin, how its `stream-json` output becomes log entries,
//! and how it and what it started are stopped.
//!
//! There is no Anthropic API client here and there will not be one: the
//! CLI is the connection, with the user's own login.

use crate::log::Kind;
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// The program started without `--agent-cmd` and `VY_AGENT_CMD`.
pub const DEFAULT_PROGRAM: &str = "claude";
/// Environment variable that replaces the default command.
pub const ENV_COMMAND: &str = "VY_AGENT_CMD";
/// Tools the agent may use without asking: the file tools (Claude Code
/// confines them to the working directory), cargo, the Vy tools and `ls`.
/// With `--permission-mode acceptEdits` and no one to answer a prompt in
/// `-p` mode, anything outside the list is refused.
pub const ALLOWED_TOOLS: &str =
    "Read,Edit,Write,Glob,Grep,Bash(cargo *),Bash(vy-cli *),Bash(vy-dev build*),Bash(ls *)";
/// How long a stopped agent gets to exit on `SIGTERM` before `SIGKILL`.
pub const STOP_GRACE: Duration = Duration::from_millis(1500);

/// The flags every run gets after the command's own arguments: headless
/// (`-p`), one JSON event per line (`stream-json` needs `--verbose` in
/// `-p` mode), edits accepted, the tool list. The mission is not among
/// them: it goes to stdin, which `claude -p` reads as the prompt when no
/// prompt argument is given. Stdin carries newlines, any length and any
/// character, where an argument to `claude.cmd` on Windows passes through
/// `cmd.exe` (no newlines, about 8 000 characters).
pub fn fixed_args() -> Vec<String> {
    [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--permission-mode",
        "acceptEdits",
        "--allowedTools",
        ALLOWED_TOOLS,
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

/// The agent command: a program and the arguments that come before
/// [`fixed_args`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCommand {
    /// The program, as given (`claude`, a path, a script).
    pub program: String,
    /// Arguments that precede [`fixed_args`].
    pub args: Vec<String>,
}

impl AgentCommand {
    /// Parse a command line: words split on whitespace, `"…"` keeps a word
    /// with spaces together (`"C:\Program Files\x\claude.cmd" --model
    /// opus`). The first word is the program.
    pub fn parse(text: &str) -> Result<Self> {
        let mut words = Vec::new();
        let mut word = String::new();
        let mut quoted = false;
        let mut started = false;
        for c in text.chars() {
            match c {
                '"' => {
                    quoted = !quoted;
                    started = true;
                }
                c if c.is_whitespace() && !quoted => {
                    if started {
                        words.push(std::mem::take(&mut word));
                        started = false;
                    }
                }
                c => {
                    word.push(c);
                    started = true;
                }
            }
        }
        if quoted {
            bail!("unbalanced quote in the agent command {text:?}");
        }
        if started {
            words.push(word);
        }
        let mut words = words.into_iter();
        let program = words
            .next()
            .filter(|p| !p.is_empty())
            .ok_or_else(|| anyhow!("the agent command is empty"))?;
        Ok(Self {
            program,
            args: words.collect(),
        })
    }

    /// `explicit` (`--agent-cmd`), else `VY_AGENT_CMD`, else `claude`.
    pub fn from_env_or(explicit: Option<&str>) -> Result<Self> {
        match explicit {
            Some(text) => Self::parse(text),
            None => match std::env::var(ENV_COMMAND) {
                Ok(text) if !text.trim().is_empty() => Self::parse(&text),
                _ => Self::parse(DEFAULT_PROGRAM),
            },
        }
    }

    /// The program as the OS will start it. On Windows `CreateProcess` does
    /// not try `PATHEXT`, so `claude` (installed by npm as `claude.cmd`)
    /// would not be found: a name without an extension is looked up with
    /// each extension of `PATHEXT`, on `PATH` for a bare name and next to
    /// itself for a path. Elsewhere the name is left to the OS.
    pub fn resolved_program(&self) -> PathBuf {
        if cfg!(windows) {
            let path = std::env::var_os("PATH").unwrap_or_default();
            let pathext = pathext(std::env::var("PATHEXT").ok().as_deref());
            if let Some(found) = find_program(&self.program, &path, &pathext) {
                return found;
            }
        }
        PathBuf::from(&self.program)
    }
}

/// The extensions of a `PATHEXT` value, or the Windows default.
pub fn pathext(value: Option<&str>) -> Vec<String> {
    let list: Vec<String> = value
        .unwrap_or("")
        .split(';')
        .map(str::trim)
        .filter(|e| e.starts_with('.') && e.len() > 1)
        .map(String::from)
        .collect();
    if list.is_empty() {
        [".COM", ".EXE", ".BAT", ".CMD"].map(String::from).to_vec()
    } else {
        list
    }
}

/// Find `name` the way `cmd.exe` would: a name with a directory part is
/// tried where it points, a bare name in each directory of `path`; each
/// candidate as given when it has an extension, then with each extension of
/// `pathext`. The first file found wins.
pub fn find_program(name: &str, path: &OsStr, pathext: &[String]) -> Option<PathBuf> {
    let has_dir = name.contains(['/', '\\']);
    let dirs: Vec<PathBuf> = if has_dir {
        vec![PathBuf::new()]
    } else {
        std::env::split_paths(path)
            .filter(|d| !d.as_os_str().is_empty())
            .collect()
    };
    for dir in dirs {
        let bare = dir.join(name);
        if Path::new(name).extension().is_some() && bare.is_file() {
            return Some(bare);
        }
        for ext in pathext {
            for ext in [ext.clone(), ext.to_ascii_lowercase()] {
                let mut candidate = bare.clone().into_os_string();
                candidate.push(&ext);
                let candidate = PathBuf::from(candidate);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

/// How to start one run.
#[derive(Clone, Debug)]
pub struct Launch<'a> {
    /// The command.
    pub cmd: &'a AgentCommand,
    /// The project directory: the agent's working directory.
    pub dir: &'a Path,
    /// The mission, written to stdin.
    pub mission: &'a str,
    /// Put first on the child's `PATH`: the directory of this binary,
    /// where `vy-cli` sits in a release, so the loop in the project's
    /// `CLAUDE.md` works without the user arranging `PATH`.
    pub prepend_path: Option<&'a Path>,
    /// More environment for the child (`VY_DEV_KEY`, `CARGO_TARGET_DIR`).
    pub env: &'a [(OsString, OsString)],
}

/// Start the agent, its stdout and stderr piped, the mission written to its
/// stdin from a thread of its own (a long mission must not block here until
/// the agent reads it) and stdin then closed.
pub fn spawn(launch: &Launch<'_>) -> Result<Child> {
    if launch.mission.trim().is_empty() {
        bail!("the mission is empty");
    }
    let program = launch.cmd.resolved_program();
    let mut command = Command::new(&program);
    command
        .args(&launch.cmd.args)
        .args(fixed_args())
        .current_dir(launch.dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(first) = launch.prepend_path {
        let mut paths = vec![first.to_path_buf()];
        if let Some(existing) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        if let Ok(joined) = std::env::join_paths(paths) {
            command.env("PATH", joined);
        }
    }
    for (k, v) in launch.env {
        command.env(k, v);
    }
    // No process group of its own: Ctrl+C on `vy-dev serve` reaches the
    // agent and what it started too, so nothing outlives the daemon in the
    // terminal. A stop walks the process tree instead ([`kill_tree`]).
    let mut child = command.spawn().with_context(|| {
        format!(
            "starting {} (is Claude Code installed and on PATH? `--agent-cmd` or {ENV_COMMAND} names another command)",
            program.display()
        )
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        let text = launch.mission.to_string();
        std::thread::spawn(move || {
            use std::io::Write;
            let _ = stdin.write_all(text.as_bytes());
            // Dropping `stdin` closes it: the end of the prompt.
        });
    }
    Ok(child)
}

/// Stop the process `pid` and everything it started. Windows: `taskkill
/// /T /F`. Elsewhere: the tree from `ps` gets `SIGTERM`, and whatever of it
/// is still there after [`STOP_GRACE`] gets `SIGKILL`. Blocks for the
/// grace period; call it from a thread of its own.
pub fn kill_tree(pid: u32) {
    if cfg!(windows) {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return;
    }
    let first = process_tree(pid);
    signal("TERM", &first);
    std::thread::sleep(STOP_GRACE);
    let mut rest = process_tree(pid);
    for p in first {
        if !rest.contains(&p) {
            rest.push(p);
        }
    }
    signal("KILL", &rest);
}

/// `pid` and its descendants, from `ps -A -o pid= -o ppid=` (POSIX, the
/// same on Linux and macOS). Just `pid` when `ps` is not there.
pub fn process_tree(pid: u32) -> Vec<u32> {
    let pairs = Command::new("ps")
        .args(["-A", "-o", "pid=", "-o", "ppid="])
        .stderr(Stdio::null())
        .output()
        .map(|o| parse_ps(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default();
    descendants(pid, &pairs)
}

/// `(pid, ppid)` pairs of `ps -o pid= -o ppid=` output.
pub fn parse_ps(text: &str) -> Vec<(u32, u32)> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
        })
        .collect()
}

/// `root` and every process below it in `pairs`, root first.
pub fn descendants(root: u32, pairs: &[(u32, u32)]) -> Vec<u32> {
    let mut out = vec![root];
    let mut i = 0;
    while i < out.len() {
        let parent = out[i];
        for &(pid, ppid) in pairs {
            if ppid == parent && pid != root && !out.contains(&pid) {
                out.push(pid);
            }
        }
        i += 1;
    }
    out
}

fn signal(name: &str, pids: &[u32]) {
    if pids.is_empty() {
        return;
    }
    let _ = Command::new("kill")
        .arg(format!("-{name}"))
        .args(pids.iter().map(u32::to_string))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Turn one line of Claude Code's `stream-json` output into log entries:
/// the `init` message, each text block and tool use of an `assistant`
/// message, and the final `result`. Anything else, and a line that is not
/// JSON, gives nothing (the raw line is logged on its own).
pub fn parse_stream_line(line: &str) -> Vec<(Kind, String)> {
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    match v["type"].as_str() {
        Some("system") if v["subtype"] == "init" => {
            let model = v["model"].as_str().unwrap_or("okänd modell");
            vec![(Kind::Agent, format!("Claude Code startade ({model})"))]
        }
        Some("assistant") => {
            let mut out = Vec::new();
            for block in v["message"]["content"].as_array().into_iter().flatten() {
                match block["type"].as_str() {
                    Some("text") => {
                        let text = block["text"].as_str().unwrap_or("").trim();
                        if !text.is_empty() {
                            out.push((Kind::Assistant, text.to_string()));
                        }
                    }
                    Some("tool_use") => {
                        let name = block["name"].as_str().unwrap_or("verktyg");
                        out.push((Kind::Tool, tool_summary(name, &block["input"])));
                    }
                    _ => {}
                }
            }
            out
        }
        Some("result") => {
            let text = v["result"].as_str().unwrap_or("").trim();
            let text = match (v["is_error"] == true, text.is_empty()) {
                (true, true) => "fel".to_string(),
                (true, false) => format!("fel: {text}"),
                (false, true) => "klart".to_string(),
                (false, false) => text.to_string(),
            };
            vec![(Kind::Result, text)]
        }
        _ => Vec::new(),
    }
}

/// `Edit src/lib.rs`, `Bash cargo build …`: the tool's name and the input
/// that says what it touched, cut to one line of 120 characters.
fn tool_summary(name: &str, input: &Value) -> String {
    let detail = ["file_path", "command", "pattern", "path", "description"]
        .into_iter()
        .find_map(|k| input[k].as_str())
        .unwrap_or("");
    let detail: String = detail
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(120)
        .collect();
    if detail.is_empty() {
        name.to_string()
    } else {
        format!("{name} {detail}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_parse() {
        let c = AgentCommand::parse("  claude --model sonnet ").unwrap();
        assert_eq!(c.program, "claude");
        assert_eq!(c.args, ["--model", "sonnet"]);
        let q = AgentCommand::parse(r#""C:\Program Files\nodejs\claude.cmd" --x "a b""#).unwrap();
        assert_eq!(q.program, r"C:\Program Files\nodejs\claude.cmd");
        assert_eq!(q.args, ["--x", "a b"]);
        assert!(AgentCommand::parse("   ").is_err());
        assert!(AgentCommand::parse("\"\"").is_err());
        assert!(AgentCommand::parse("\"open").is_err());
        assert_eq!(
            AgentCommand::from_env_or(Some("tools/fake_agent.sh")).unwrap(),
            AgentCommand::parse("tools/fake_agent.sh").unwrap()
        );
        let args = fixed_args();
        assert_eq!(args[0], "-p");
        assert!(args
            .windows(2)
            .any(|w| w == ["--output-format", "stream-json"]));
        assert!(args.contains(&"--verbose".to_string()));
        assert!(args
            .windows(2)
            .any(|w| w == ["--permission-mode", "acceptEdits"]));
        assert_eq!(args.last().map(String::as_str), Some(ALLOWED_TOOLS));
    }

    /// The `PATH` + `PATHEXT` lookup Windows needs for `claude.cmd`,
    /// exercised on every platform with directories of our own.
    #[test]
    fn finds_scripts_with_pathext() {
        let dir = std::env::temp_dir().join(format!("vy-dev-pathext-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("claude.CMD"), "@echo off\n").unwrap();
        std::fs::write(bin.join("tool.exe"), "").unwrap();
        std::fs::write(bin.join("other.cmd"), "").unwrap();
        let path = std::env::join_paths([dir.join("empty"), bin.clone()]).unwrap();
        let ext = pathext(Some(".COM;.EXE;.BAT;.CMD;;junk"));
        assert_eq!(ext, [".COM", ".EXE", ".BAT", ".CMD"]);
        assert_eq!(pathext(None), ext);
        // Windows and macOS file systems ignore case, so the first
        // extension in PATHEXT order that names the file wins there
        // (`tool.EXE` for `tool.exe`); the file is the same either way.
        let same = |found: Option<PathBuf>, want: PathBuf, what: &str| {
            let found = found.unwrap_or_else(|| panic!("{what}: not found"));
            assert!(
                found == want
                    || found
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&want.to_string_lossy()),
                "{what}: found {}, want {}",
                found.display(),
                want.display()
            );
        };
        same(
            find_program("claude", &path, &ext),
            bin.join("claude.CMD"),
            "claude",
        );
        same(
            find_program("tool", &path, &ext),
            bin.join("tool.exe"),
            "tool",
        );
        same(
            find_program("other", &path, &ext),
            bin.join("other.cmd"),
            "lower-case extensions too",
        );
        same(
            find_program("tool.exe", &path, &ext),
            bin.join("tool.exe"),
            "a name with an extension is tried as given",
        );
        let with_dir = format!("{}/claude", bin.display());
        same(
            find_program(&with_dir, OsStr::new(""), &ext),
            PathBuf::from(format!("{with_dir}.CMD")),
            "a path is completed where it points, not on PATH",
        );
        assert_eq!(find_program("nothing", &path, &ext), None);
        let _ = std::fs::remove_dir_all(&dir);
        if !cfg!(windows) {
            let c = AgentCommand::parse("claude").unwrap();
            assert_eq!(c.resolved_program(), PathBuf::from("claude"));
        }
    }

    #[test]
    fn process_trees() {
        let pairs = parse_ps("  1     0\n 10     1\n 11    10\n 12    11\n 20     1\n bad line\n");
        assert_eq!(pairs.len(), 5);
        assert_eq!(descendants(10, &pairs), [10, 11, 12]);
        assert_eq!(descendants(20, &pairs), [20]);
        assert_eq!(descendants(99, &pairs), [99]);
        if !cfg!(windows) {
            assert!(process_tree(std::process::id()).contains(&std::process::id()));
        }
    }

    #[test]
    fn stream_json_lines_become_entries() {
        let init = r#"{"type":"system","subtype":"init","cwd":"/p","model":"claude-opus-5-5","tools":["Read"]}"#;
        assert_eq!(
            parse_stream_line(init),
            vec![(Kind::Agent, "Claude Code startade (claude-opus-5-5)".into())]
        );
        let assistant = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":" Jag ändrar rubriken. "},{"type":"tool_use","id":"t1","name":"Edit","input":{"file_path":"src/lib.rs","old_string":"a","new_string":"b"}},{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"cargo build --release\n--target wasm32"}},{"type":"tool_use","id":"t3","name":"Glob","input":{}}]}}"#;
        assert_eq!(
            parse_stream_line(assistant),
            vec![
                (Kind::Assistant, "Jag ändrar rubriken.".into()),
                (Kind::Tool, "Edit src/lib.rs".into()),
                (Kind::Tool, "Bash cargo build --release".into()),
                (Kind::Tool, "Glob".into()),
            ]
        );
        let result = r#"{"type":"result","subtype":"success","is_error":false,"result":"Rubriken är ändrad.","total_cost_usd":0.1}"#;
        assert_eq!(
            parse_stream_line(result),
            vec![(Kind::Result, "Rubriken är ändrad.".into())]
        );
        // What Claude Code 2.1 prints when the API cannot be reached.
        let failed = r#"{"type":"result","subtype":"success","is_error":true,"result":"API Error: Connection refused"}"#;
        assert_eq!(
            parse_stream_line(failed),
            vec![(Kind::Result, "fel: API Error: Connection refused".into())]
        );
        assert!(parse_stream_line(r#"{"type":"user","message":{}}"#).is_empty());
        assert!(parse_stream_line(r#"{"type":"system","subtype":"status"}"#).is_empty());
        assert!(parse_stream_line("not json").is_empty());
        assert!(parse_stream_line("").is_empty());
    }
}
