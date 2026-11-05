//! Devtools (F12, SPEC-11.4.1): a panel the shell draws at the right of
//! the window, beside the page area (the page gets the narrower area; the
//! panel is chrome, SPEC-9.6.1). Five panes: "Display list" (the current
//! frame's ops with counts and bytes; selecting one outlines its rectangle
//! over the page), "Semantik" (the tree with roles, ids, text and the focus
//! mark), "Logg" (the page's log with level and frame number),
//! "Inspelning" (record the session of a tab process to a file and replay
//! one, with the `--record`/`--replay` machinery) and "Studio" (live reload
//! of localhost pages and a link to Studio).
//!
//! Everything here turns what the shell has into lines of text; the shell
//! does what a line's [`Command`] says, and `chrome::draw_devtools` draws
//! the lines.

use crate::chrome::Rect;
use std::path::PathBuf;
use vy_render::TextEngine;
use vy_runtime::SemNode;

/// Width of the panel in logical pixels (less in a narrow window).
pub const PANEL_W: f32 = 440.0;
/// Height of the pane tabs at the panel's top.
pub const HEADER_H: f32 = 30.0;
/// Height of a line.
pub const LINE_H: f32 = 16.0;
/// Most lines a pane lists (a frame may have 65 536 ops).
pub const MAX_LINES: usize = 4000;
/// Where "Öppna Studio" goes: the m4/dev daemon serves Studio there.
pub const STUDIO_ADDRESS: &str = "vy://localhost:8081/";
/// How often live reload asks a localhost page's server whether the page
/// changed (a conditional GET, SPEC-7.3.2).
pub const LIVE_RELOAD_MS: i64 = 500;

/// A pane of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    /// The current frame's ops.
    DisplayList,
    /// The semantic tree.
    Semantic,
    /// The page's log.
    Log,
    /// Record and replay.
    Recording,
    /// Live reload and Studio.
    Studio,
}

impl Pane {
    /// The panes in tab order.
    pub const ALL: [Pane; 5] = [
        Pane::DisplayList,
        Pane::Semantic,
        Pane::Log,
        Pane::Recording,
        Pane::Studio,
    ];

    /// The tab's label.
    pub fn label(self) -> &'static str {
        match self {
            Pane::DisplayList => "Display list",
            Pane::Semantic => "Semantik",
            Pane::Log => "Logg",
            Pane::Recording => "Inspelning",
            Pane::Studio => "Studio",
        }
    }
}

/// What a click on a line does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Select (or unselect) op `n` of the frame: its outline over the page.
    SelectOp(usize),
    /// Record the active tab's session from a fresh load.
    StartRecording,
    /// Stop recording; the file is complete.
    StopRecording,
    /// Replay a recording in a new tab.
    Replay(PathBuf),
    /// Live reload of localhost pages on or off.
    ToggleLiveReload,
    /// Open Studio in a new tab.
    OpenStudio,
}

/// One line of a pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// What it says.
    pub text: String,
    /// Drawn stronger (a heading, the selected op, the focused node).
    pub strong: bool,
    /// What a click does, if anything.
    pub command: Option<Command>,
}

impl Line {
    /// A line of plain text.
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            strong: false,
            command: None,
        }
    }

    /// A heading.
    pub fn strong(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            strong: true,
            command: None,
        }
    }

    /// A line a click acts on.
    pub fn button(text: impl Into<String>, command: Command) -> Self {
        Self {
            text: text.into(),
            strong: false,
            command: Some(command),
        }
    }
}

/// An op of the frame as the panel lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct OpInfo {
    /// The opcode's name (appendix C).
    pub name: &'static str,
    /// Its bytes in the list.
    pub bytes: usize,
    /// Its arguments, short.
    pub detail: String,
    /// The rectangle it covers, in page coordinates, when known without
    /// laying out text.
    pub rect: Option<Rect>,
}

/// Text cut to `max` characters with an ellipsis.
fn short(s: &str, max: usize) -> String {
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out.replace('\n', "⏎")
}

/// The ops of a display list (appendix C), in order; empty for a list
/// that does not decode.
pub fn ops(display_list: &[u8]) -> Vec<OpInfo> {
    let Ok(ops) = vy_dl::decode(display_list) else {
        return Vec::new();
    };
    ops.into_iter()
        .map(|op| match op {
            vy_dl::Op::Rect {
                x,
                y,
                w,
                h,
                radius,
                color,
            } => OpInfo {
                name: "RECT",
                bytes: 25,
                detail: format!("{x:.0},{y:.0} {w:.0}×{h:.0} r{radius:.0} #{color:08X}"),
                rect: Some(Rect::new(x, y, w, h)),
            },
            vy_dl::Op::Text {
                x,
                y,
                max_w,
                style_id,
                text,
            } => OpInfo {
                name: "TEXT",
                bytes: 17 + text.len(),
                detail: format!(
                    "{x:.0},{y:.0} w{max_w:.0} stil {style_id} «{}»",
                    short(&String::from_utf8_lossy(&text), 40)
                ),
                rect: None,
            },
            vy_dl::Op::Image {
                x,
                y,
                w,
                h,
                resource_id,
                fit,
            } => OpInfo {
                name: "IMAGE",
                bytes: 20,
                detail: format!("{x:.0},{y:.0} {w:.0}×{h:.0} resurs {resource_id} fit {fit}"),
                rect: Some(Rect::new(x, y, w, h)),
            },
            vy_dl::Op::Path {
                color,
                stroke_w,
                cmds,
            } => {
                let mut pts: Vec<(f32, f32)> = Vec::new();
                let mut words = 0usize;
                for c in &cmds {
                    words += 1;
                    match *c {
                        vy_dl::PathCmd::MoveTo(x, y) | vy_dl::PathCmd::LineTo(x, y) => {
                            pts.push((x, y))
                        }
                        vy_dl::PathCmd::QuadTo { ctrl, to } => {
                            // A quad is two commands on the wire (SPEC-C.1.2).
                            words += 1;
                            pts.push(ctrl);
                            pts.push(to);
                        }
                        vy_dl::PathCmd::Close => {}
                    }
                }
                let rect = bbox(&pts);
                OpInfo {
                    name: "PATH",
                    bytes: 11 + 9 * words,
                    detail: format!("{} kommandon, linje {stroke_w:.1} #{color:08X}", cmds.len()),
                    rect,
                }
            }
            vy_dl::Op::ClipPush { x, y, w, h, radius } => OpInfo {
                name: "CLIP_PUSH",
                bytes: 21,
                detail: format!("{x:.0},{y:.0} {w:.0}×{h:.0} r{radius:.0}"),
                rect: Some(Rect::new(x, y, w, h)),
            },
            vy_dl::Op::ClipPop => OpInfo {
                name: "CLIP_POP",
                bytes: 1,
                detail: String::new(),
                rect: None,
            },
        })
        .collect()
}

fn bbox(pts: &[(f32, f32)]) -> Option<Rect> {
    let finite: Vec<_> = pts
        .iter()
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .collect();
    let (first, rest) = finite.split_first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.0, first.1, first.0, first.1);
    for (x, y) in rest {
        x0 = x0.min(*x);
        y0 = y0.min(*y);
        x1 = x1.max(*x);
        y1 = y1.max(*y);
    }
    Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
}

/// The rectangle of op `n`, laying out a `TEXT` op with the page's own
/// styles when that is what it is.
pub fn op_rect(display_list: &[u8], n: usize, text: &mut TextEngine) -> Option<Rect> {
    let op = vy_dl::decode(display_list).ok()?.into_iter().nth(n)?;
    match op {
        vy_dl::Op::Text {
            x,
            y,
            max_w,
            style_id,
            text: bytes,
        } => {
            let size = text
                .measure(style_id, max_w, &String::from_utf8_lossy(&bytes))
                .ok()?;
            Some(Rect::new(x, y, size.w, size.h))
        }
        _ => ops(display_list).into_iter().nth(n).and_then(|o| o.rect),
    }
}

/// The "Display list" pane: totals, counts and bytes per opcode, then
/// every op (up to [`MAX_LINES`]), the selected one strong.
pub fn display_list_lines(display_list: &[u8], selected: Option<usize>) -> Vec<Line> {
    let ops = ops(display_list);
    let mut out = vec![Line::strong(format!(
        "{} opkoder, {} byte",
        ops.len(),
        display_list.len()
    ))];
    let mut kinds: Vec<(&str, usize, usize)> = Vec::new();
    for o in &ops {
        match kinds.iter_mut().find(|k| k.0 == o.name) {
            Some(k) => {
                k.1 += 1;
                k.2 += o.bytes;
            }
            None => kinds.push((o.name, 1, o.bytes)),
        }
    }
    for (name, n, bytes) in kinds {
        out.push(Line::plain(format!("  {name}: {n} st, {bytes} byte")));
    }
    out.push(Line::plain(""));
    for (i, o) in ops.iter().enumerate().take(MAX_LINES) {
        let mut line = Line::button(
            format!("#{i} {} {} ({} B)", o.name, o.detail, o.bytes),
            Command::SelectOp(i),
        );
        line.strong = Some(i) == selected;
        out.push(line);
    }
    if ops.len() > MAX_LINES {
        out.push(Line::plain(format!("… {} till", ops.len() - MAX_LINES)));
    }
    out
}

/// The "Semantik" pane: the tree indented by nesting (a node under its
/// parent when the parent comes earlier, as the accessibility tree has
/// it), with role, id, text and the focus mark.
pub fn tree_lines(sem: &[SemNode], focus: Option<i32>) -> Vec<Line> {
    if sem.is_empty() {
        return vec![Line::plain("Inget semantiskt träd (otillgänglig).")];
    }
    let mut depth: Vec<usize> = Vec::with_capacity(sem.len());
    let mut out = vec![Line::strong(format!("{} noder", sem.len()))];
    for (i, n) in sem.iter().enumerate().take(MAX_LINES) {
        let parent = sem[..i]
            .iter()
            .position(|p| n.parent_id != 0 && p.id == n.parent_id);
        let d = parent.map_or(0, |p| depth[p] + 1).min(12);
        depth.push(d);
        let focused = focus == Some(n.id);
        let mut line = Line::plain(format!(
            "{}{}{} #{}{}",
            "  ".repeat(d),
            if focused { "▶ " } else { "" },
            vy_abi::role::name(n.role).unwrap_or("?"),
            n.id,
            if n.text.is_empty() {
                String::new()
            } else {
                format!(" «{}»", short(&n.text, 48))
            }
        ));
        line.strong = focused;
        out.push(line);
    }
    out
}

/// The "Logg" pane: the page's log lines with the frame each came in.
pub fn log_lines(logs: &[(u64, String)]) -> Vec<Line> {
    if logs.is_empty() {
        return vec![Line::plain("Inga loggrader.")];
    }
    logs.iter()
        .rev()
        .take(MAX_LINES)
        .rev()
        .map(|(frame, l)| Line::plain(format!("frame {frame} {l}")))
        .collect()
}

/// The "Inspelning" pane.
pub fn recording_lines(
    recording: Option<&std::path::Path>,
    can_record: bool,
    recordings: &[(PathBuf, u64)],
    status: &str,
) -> Vec<Line> {
    let mut out = vec![Line::strong("Inspelning av flikens session (SPEC-11.4.1)")];
    match (recording, can_record) {
        (Some(p), _) => out.push(Line::button(
            format!("■ Stoppa inspelningen ({})", file_name(p)),
            Command::StopRecording,
        )),
        (None, true) => out.push(Line::button(
            "● Spela in från en ny laddning av sidan",
            Command::StartRecording,
        )),
        (None, false) => out.push(Line::plain(
            "Inspelning kräver en sida i en egen flikprocess.",
        )),
    }
    if !status.is_empty() {
        out.push(Line::plain(status));
    }
    out.push(Line::plain(""));
    out.push(Line::strong("Spela upp"));
    if recordings.is_empty() {
        out.push(Line::plain("Inga inspelningar ännu."));
    }
    for (p, size) in recordings {
        out.push(Line::button(
            format!("▶ {} ({size} byte)", file_name(p)),
            Command::Replay(p.clone()),
        ));
    }
    out
}

/// The "Studio" pane.
pub fn studio_lines(live_reload: bool) -> Vec<Line> {
    vec![
        Line::strong("Studio"),
        Line::button(
            format!(
                "[{}] Live-omladdning (localhost)",
                if live_reload { "x" } else { " " }
            ),
            Command::ToggleLiveReload,
        ),
        Line::plain(format!(
            "    frågar servern var {LIVE_RELOAD_MS} ms (villkorlig GET) och laddar om vid ändring"
        )),
        Line::button(
            format!("Öppna Studio ({STUDIO_ADDRESS})"),
            Command::OpenStudio,
        ),
    ]
}

fn file_name(p: &std::path::Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

/// The recordings in `dir`, newest first (at most ten), with sizes.
pub fn list_recordings(dir: &std::path::Path) -> Vec<(PathBuf, u64)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "vyrec"))
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((e.path(), m.len(), m.modified().ok()?))
        })
        .collect();
    files.sort_by(|a, b| b.2.cmp(&a.2).then(b.0.cmp(&a.0)));
    files.into_iter().take(10).map(|(p, s, _)| (p, s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> Vec<u8> {
        let mut dl = vy_dl::DisplayList::new();
        dl.begin().unwrap();
        dl.rect(1.0, 2.0, 30.0, 40.0, 0.0, 0xFF0000FF).unwrap();
        dl.clip_push(0.0, 0.0, 100.0, 100.0, 4.0).unwrap();
        dl.text(5.0, 6.0, 0.0, 1, "Hej".as_bytes()).unwrap();
        dl.clip_pop().unwrap();
        dl.rect(9.0, 9.0, 1.0, 1.0, 0.0, 0x00FF00FF).unwrap();
        dl.end().unwrap();
        dl.bytes().to_vec()
    }

    /// SPEC-11.4.1, SPEC-C.1.1: the "Display list" pane lists the frame's
    /// ops in order with their bytes per appendix C, counts per opcode and
    /// the total; a line selects its op, whose rectangle is known (a text
    /// op's from the page's own text engine).
    #[test]
    fn spec_11_4_1_display_list_pane() {
        let bytes = list();
        let o = ops(&bytes);
        let names: Vec<&str> = o.iter().map(|o| o.name).collect();
        assert_eq!(names, ["RECT", "CLIP_PUSH", "TEXT", "CLIP_POP", "RECT"]);
        assert_eq!(
            o.iter().map(|o| o.bytes).sum::<usize>() + 1,
            bytes.len(),
            "every byte but END"
        );
        let lines = display_list_lines(&bytes, Some(2));
        assert_eq!(lines[0].text, format!("5 opkoder, {} byte", bytes.len()));
        assert!(lines.iter().any(|l| l.text == "  RECT: 2 st, 50 byte"));
        let text_line = lines
            .iter()
            .find(|l| l.command == Some(Command::SelectOp(2)))
            .unwrap();
        assert!(text_line.strong && text_line.text.contains("«Hej»"));
        assert_eq!(o[0].rect, Some(Rect::new(1.0, 2.0, 30.0, 40.0)));
        let mut engine = TextEngine::new();
        engine
            .style(vy_abi::font::SANS, 16.0, vy_abi::weight::NORMAL, 0x000000FF)
            .unwrap();
        let r = op_rect(&bytes, 2, &mut engine).unwrap();
        assert_eq!((r.x, r.y), (5.0, 6.0));
        assert!(r.w > 10.0 && r.h > 10.0);
        assert_eq!(op_rect(&bytes, 99, &mut engine), None);
        assert!(display_list_lines(b"garbage", None)[0]
            .text
            .starts_with("0 opkoder"));
    }

    /// SPEC-11.4.1, SPEC-10.1.1: the "Semantik" pane shows the tree with
    /// nesting, roles, ids, text and the focus mark; "Logg" carries the
    /// frame number; "Inspelning" and "Studio" offer their commands.
    #[test]
    fn spec_11_4_1_tree_log_recording_studio_panes() {
        let n = |id, parent_id, role_id, text: &str| SemNode {
            id,
            parent_id,
            role: role_id,
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
            text: text.into(),
        };
        let sem = vec![
            n(1, 0, vy_abi::role::MAIN, ""),
            n(2, 1, vy_abi::role::HEADING, "Rubrik"),
            n(3, 1, vy_abi::role::TEXTBOX, "Anna"),
        ];
        let lines = tree_lines(&sem, Some(3));
        assert_eq!(lines[0].text, "3 noder");
        assert_eq!(lines[1].text, "main #1");
        assert_eq!(lines[2].text, "  heading #2 «Rubrik»");
        assert_eq!(lines[3].text, "  ▶ textbox #3 «Anna»");
        assert!(lines[3].strong);
        assert!(tree_lines(&[], None)[0].text.contains("otillgänglig"));
        let logs = log_lines(&[(4, "[module info] hej".into())]);
        assert_eq!(logs[0].text, "frame 4 [module info] hej");
        let rec = recording_lines(None, true, &[(PathBuf::from("/x/a.vyrec"), 10)], "");
        assert!(rec
            .iter()
            .any(|l| l.command == Some(Command::StartRecording)));
        assert!(rec
            .iter()
            .any(|l| l.command == Some(Command::Replay(PathBuf::from("/x/a.vyrec")))));
        let rec = recording_lines(Some(std::path::Path::new("/x/b.vyrec")), true, &[], "");
        assert!(rec
            .iter()
            .any(|l| l.command == Some(Command::StopRecording)));
        assert!(recording_lines(None, false, &[], "")
            .iter()
            .all(|l| l.command.is_none()));
        let studio = studio_lines(true);
        assert!(studio[1].text.starts_with("[x] Live-omladdning"));
        assert!(studio
            .iter()
            .any(|l| l.command == Some(Command::OpenStudio)));
    }
}
