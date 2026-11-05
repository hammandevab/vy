//! Text selection (SPEC-10.2.2): owned by the host, over the page's
//! semantic text nodes, and copied as their logical strings, never as
//! pixels. The page's display list is not touched: the selection is drawn
//! by the main process as an overlay on the page area (see
//! `Shell::compose`), so a page's frames stay what the module made them
//! (SPEC-2.1.5).
//!
//! An end of the selection is a [`Caret`]: a node id and a byte offset in
//! that node's text. The selection is every selectable node in document
//! order (the order of the tree, SPEC-10.1.1) from one end to the other.
//! Inside the end nodes it has glyph granularity when the node's text is
//! drawn by one `TEXT` op of the frame (the usual case: a heading, a
//! paragraph): the offset under the pointer and the highlighted spans come
//! from the same text engine and styles that rasterized the frame, through
//! its `measure` and `caret` (SPEC-5.2.5). A node whose text is not one op
//! (made of several runs, or not drawn at all) is taken whole.

use crate::chrome::Rect;
use vy_abi::role;
use vy_render::TextEngine;
use vy_runtime::SemNode;

/// One end of a selection: a node and a byte offset in its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caret {
    /// The semantic node's id.
    pub id: i32,
    /// Byte offset into its text, at a character boundary.
    pub offset: usize,
}

/// The host's selection in one page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    /// Where it started.
    pub anchor: Option<Caret>,
    /// Where it ends now (follows the pointer while dragging).
    pub focus: Option<Caret>,
    /// True while the primary button that started it is down.
    pub dragging: bool,
}

impl Selection {
    /// True when nothing is selected.
    pub fn is_empty(&self) -> bool {
        match (self.anchor, self.focus) {
            (Some(a), Some(f)) => a == f,
            _ => true,
        }
    }

    /// Forget the selection.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// True for the roles whose text a selection takes: text, headings,
/// links, list items and cells. Controls (buttons, fields, boxes) are
/// left to the page.
pub fn selectable(role_id: i32) -> bool {
    matches!(
        role_id,
        role::TEXT | role::HEADING | role::LINK | role::LISTITEM | role::CELL
    )
}

/// Indices into `sem` of the selectable nodes with text, in document order.
pub fn order(sem: &[SemNode]) -> Vec<usize> {
    sem.iter()
        .enumerate()
        .filter(|(_, n)| selectable(n.role) && !n.text.is_empty())
        .map(|(i, _)| i)
        .collect()
}

/// A `TEXT` op of the frame: where it is drawn, wrapped how, in which style.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    /// Left, logical pixels.
    pub x: f32,
    /// Top.
    pub y: f32,
    /// Wrap width (`≤ 0`: no wrapping, SPEC-6.4.3).
    pub max_w: f32,
    /// The style id.
    pub style: u16,
    /// The text.
    pub text: String,
}

/// The `TEXT` ops of a display list, in order.
pub fn runs(display_list: &[u8]) -> Vec<Run> {
    vy_dl::decode(display_list)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|op| match op {
            vy_dl::Op::Text {
                x,
                y,
                max_w,
                style_id,
                text,
            } => Some(Run {
                x,
                y,
                max_w,
                style: style_id,
                text: String::from_utf8_lossy(&text).into_owned(),
            }),
            _ => None,
        })
        .collect()
}

/// The run that draws `node`'s text: the same text, starting inside the
/// node's rectangle (two pixels of slack).
pub fn run_for<'a>(node: &SemNode, runs: &'a [Run]) -> Option<&'a Run> {
    runs.iter().find(|r| {
        r.text == node.text
            && r.x >= node.x - 2.0
            && r.y >= node.y - 2.0
            && r.x <= node.x + node.w + 2.0
            && r.y <= node.y + node.h + 2.0
    })
}

/// One laid-out line of a run: its bytes and its row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Line {
    start: usize,
    end: usize,
    row: usize,
}

/// The lines of `run` as the text engine wraps it: paragraphs split at
/// `\n`, each wrapped at `max_w`. A word's line is the line count of the
/// paragraph's prefix through that word (the wrap is greedy, so what comes
/// after a word never moves it), found by binary search with `measure`.
/// A word longer than the width, which the engine breaks inside, stays on
/// one line here: the highlight is then approximate for that word.
fn lines(text: &mut TextEngine, run: &Run, line_h: f32) -> Vec<Line> {
    let mut out = Vec::new();
    let mut row = 0usize;
    let mut para_start = 0usize;
    for para in run.text.split('\n') {
        let base = para_start;
        para_start += para.len() + 1;
        let words: Vec<(usize, usize)> = word_spans(para);
        if run.max_w <= 0.0 || words.len() < 2 {
            out.push(Line {
                start: base,
                end: base + para.len(),
                row,
            });
            row += 1;
            continue;
        }
        let mut count = |k: usize| -> usize {
            let h = text
                .measure(run.style, run.max_w, &para[..words[k].1])
                .map_or(line_h, |s| s.h);
            ((h / line_h).round() as usize).max(1)
        };
        let total = count(words.len() - 1);
        let mut starts = vec![0usize];
        let mut from = 0usize;
        for line in 2..=total {
            let (mut lo, mut hi) = (from, words.len() - 1);
            while lo < hi {
                let mid = (lo + hi) / 2;
                if count(mid) >= line {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            if words[lo].0 > *starts.last().expect("one start") {
                starts.push(words[lo].0);
            }
            from = lo;
        }
        for (i, s) in starts.iter().enumerate() {
            let e = starts.get(i + 1).copied().unwrap_or(para.len());
            out.push(Line {
                start: base + s,
                end: base + e,
                row,
            });
            row += 1;
        }
    }
    out
}

/// Byte spans of the words (runs of non-whitespace) of `s`.
fn word_spans(s: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(st)) => {
                out.push((st, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(st) = start {
        out.push((st, s.len()));
    }
    out
}

fn line_height(text: &TextEngine, style: u16) -> f32 {
    text.get_style(style)
        .map_or(16.0, |s| s.size_px * vy_render::text::LINE_HEIGHT_FACTOR)
}

/// The x of the caret before byte `at` of `line` (unwrapped, as the
/// engine lays out one line).
fn caret_x(text: &mut TextEngine, run: &Run, line: Line, at: usize) -> f32 {
    let s = &run.text[line.start..line.end];
    let at = at.clamp(line.start, line.end) - line.start;
    text.caret(run.style, s, at).map_or(0.0, |c| c.0)
}

/// The byte offset in `node`'s text nearest the page point `(px, py)`:
/// glyph granularity through `run` when there is one, else the node's
/// start or end, whichever side of its middle the point is on.
pub fn offset_at(
    text: &mut TextEngine,
    node: &SemNode,
    run: Option<&Run>,
    px: f32,
    py: f32,
) -> usize {
    let Some(run) = run else {
        let before = py < node.y || (py < node.y + node.h && px < node.x + node.w / 2.0);
        return if before { 0 } else { node.text.len() };
    };
    let line_h = line_height(text, run.style);
    let ls = lines(text, run, line_h);
    let Some(last) = ls.last().copied() else {
        return 0;
    };
    if py < run.y {
        return 0;
    }
    let row = ((py - run.y) / line_h).floor().max(0.0) as usize;
    if row > last.row {
        return run.text.len();
    }
    let line = ls.iter().copied().find(|l| l.row == row).unwrap_or(last);
    // The nearest character boundary to the point, by binary search on
    // the caret's x (left to right).
    let bounds: Vec<usize> = run.text[line.start..line.end]
        .char_indices()
        .map(|(i, _)| line.start + i)
        .chain(std::iter::once(line.end))
        .collect();
    let target = px - run.x;
    let (mut lo, mut hi) = (0usize, bounds.len() - 1);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if caret_x(text, run, line, bounds[mid]) < target {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo > 0 {
        let a = caret_x(text, run, line, bounds[lo - 1]);
        let b = caret_x(text, run, line, bounds[lo]);
        if (target - a).abs() <= (b - target).abs() {
            return bounds[lo - 1];
        }
    }
    bounds[lo]
}

/// The rectangles that highlight bytes `a..b` of `node`'s text, in page
/// coordinates: one per line through `run`, or the node's own rectangle.
pub fn span_rects(
    text: &mut TextEngine,
    node: &SemNode,
    run: Option<&Run>,
    a: usize,
    b: usize,
) -> Vec<Rect> {
    if a >= b {
        return Vec::new();
    }
    let Some(run) = run else {
        return vec![Rect::new(node.x, node.y, node.w, node.h)];
    };
    let line_h = line_height(text, run.style);
    let mut out = Vec::new();
    for line in lines(text, run, line_h) {
        let (s, e) = (a.max(line.start), b.min(line.end));
        if s >= e {
            continue;
        }
        let x0 = caret_x(text, run, line, s);
        let x1 = caret_x(text, run, line, e);
        if x1 > x0 {
            out.push(Rect::new(
                run.x + x0,
                run.y + line.row as f32 * line_h,
                x1 - x0,
                line_h,
            ));
        }
    }
    out
}

/// The selectable node at page point `(px, py)`: the last in document
/// order whose rectangle holds it.
pub fn node_at(sem: &[SemNode], order: &[usize], px: f32, py: f32) -> Option<usize> {
    order.iter().rev().copied().find(|&i| {
        let n = &sem[i];
        px >= n.x && py >= n.y && px < n.x + n.w && py < n.y + n.h
    })
}

/// The selectable node nearest page point `(px, py)`, for a drag that
/// left every node: the smallest distance to its rectangle, the later on
/// a tie.
pub fn nearest(sem: &[SemNode], order: &[usize], px: f32, py: f32) -> Option<usize> {
    let dist = |n: &SemNode| {
        let dx = (n.x - px).max(px - (n.x + n.w)).max(0.0);
        let dy = (n.y - py).max(py - (n.y + n.h)).max(0.0);
        dx * dx + dy * dy
    };
    let mut best: Option<(usize, f32)> = None;
    for &i in order {
        let d = dist(&sem[i]);
        if best.is_none_or(|(_, bd)| d <= bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

/// The selected byte range of each node, in document order: `(index into
/// sem, start, end)`, empty ranges left out. Nothing when an end's node
/// is not in the tree.
pub fn ranges(sem: &[SemNode], sel: &Selection) -> Vec<(usize, usize, usize)> {
    let (Some(a), Some(f)) = (sel.anchor, sel.focus) else {
        return Vec::new();
    };
    let order = order(sem);
    let pos = |c: Caret| order.iter().position(|&i| sem[i].id == c.id);
    let (Some(pa), Some(pf)) = (pos(a), pos(f)) else {
        return Vec::new();
    };
    let ((p0, c0), (p1, c1)) = if (pa, a.offset) <= (pf, f.offset) {
        ((pa, a), (pf, f))
    } else {
        ((pf, f), (pa, a))
    };
    let mut out = Vec::new();
    for (p, &i) in order.iter().enumerate().take(p1 + 1).skip(p0) {
        let len = sem[i].text.len();
        let start = if p == p0 { c0.offset.min(len) } else { 0 };
        let end = if p == p1 { c1.offset.min(len) } else { len };
        if start < end {
            out.push((i, start, end));
        }
    }
    out
}

/// What a copy of the selection puts on the clipboard (SPEC-10.2.2): the
/// selected part of each node's logical string, in document order, with
/// `\n` between nodes. Never pixels.
pub fn selected_text(sem: &[SemNode], sel: &Selection) -> String {
    ranges(sem, sel)
        .into_iter()
        .filter_map(|(i, s, e)| sem[i].text.get(s..e))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every text node of the page (Ctrl+A).
pub fn all(sem: &[SemNode]) -> Selection {
    let order = order(sem);
    match (order.first(), order.last()) {
        (Some(&first), Some(&last)) => Selection {
            anchor: Some(Caret {
                id: sem[first].id,
                offset: 0,
            }),
            focus: Some(Caret {
                id: sem[last].id,
                offset: sem[last].text.len(),
            }),
            dragging: false,
        },
        _ => Selection::default(),
    }
}

/// The highlight of the selection, in page coordinates.
pub fn rects(text: &mut TextEngine, sem: &[SemNode], runs: &[Run], sel: &Selection) -> Vec<Rect> {
    let mut out = Vec::new();
    for (i, s, e) in ranges(sem, sel) {
        let node = &sem[i];
        let run = run_for(node, runs);
        out.extend(span_rects(text, node, run, s, e));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i32, role_id: i32, y: f32, text: &str) -> SemNode {
        SemNode {
            id,
            parent_id: 0,
            role: role_id,
            x: 10.0,
            y,
            w: 300.0,
            h: 20.0,
            text: text.into(),
        }
    }

    /// SPEC-10.2.2: the selected text is the nodes' logical strings in
    /// document order, cut at the ends' offsets, joined with `\n`; the
    /// ends may come in either order; controls are not selected; Ctrl+A
    /// takes every text node.
    #[test]
    fn spec_10_2_2_selected_text_is_logical() {
        let sem = vec![
            node(1, role::HEADING, 0.0, "Rubrik"),
            node(2, role::BUTTON, 30.0, "Skicka"),
            node(3, role::TEXT, 60.0, "Första stycket."),
            node(4, role::TEXT, 90.0, "Andra stycket."),
        ];
        let sel = |a: (i32, usize), f: (i32, usize)| Selection {
            anchor: Some(Caret {
                id: a.0,
                offset: a.1,
            }),
            focus: Some(Caret {
                id: f.0,
                offset: f.1,
            }),
            dragging: false,
        };
        assert_eq!(
            selected_text(&sem, &sel((1, 2), (4, 5))),
            "brik\nFörsta stycket.\nAndra"
        );
        assert_eq!(
            selected_text(&sem, &sel((4, 5), (1, 2))),
            "brik\nFörsta stycket.\nAndra",
            "backwards"
        );
        assert_eq!(selected_text(&sem, &sel((3, 7), (3, 0))), "Första");
        assert_eq!(selected_text(&sem, &sel((3, 3), (3, 3))), "");
        assert!(sel((3, 3), (3, 3)).is_empty());
        assert_eq!(
            selected_text(&sem, &sel((1, 6), (3, 0))),
            "",
            "an end at a node's edge adds no empty line"
        );
        assert_eq!(
            selected_text(&sem, &all(&sem)),
            "Rubrik\nFörsta stycket.\nAndra stycket."
        );
        assert_eq!(selected_text(&sem, &sel((9, 0), (3, 3))), "", "gone node");
        assert_eq!(node_at(&sem, &order(&sem), 20.0, 65.0), Some(2));
        assert_eq!(node_at(&sem, &order(&sem), 20.0, 35.0), None, "a button");
        assert_eq!(nearest(&sem, &order(&sem), 20.0, 500.0), Some(3));
        assert_eq!(word_spans("  ab c  d"), vec![(2, 4), (5, 6), (8, 9)]);
    }

    /// SPEC-10.2.2, SPEC-6.4.3: glyph granularity follows the engine's own
    /// layout: a point on the second line of a wrapped run lands on that
    /// line, the offset nearest the point is a character boundary, and
    /// the highlight of a span over a wrap is one rectangle per line.
    #[test]
    fn spec_10_2_2_glyph_granularity_in_wrapped_runs() {
        let mut text = TextEngine::new();
        let style = text
            .style(vy_abi::font::SANS, 16.0, vy_abi::weight::NORMAL, 0x000000FF)
            .unwrap();
        let line_h = 16.0 * vy_render::text::LINE_HEIGHT_FACTOR;
        let s = "alpha beta gamma delta epsilon zeta eta theta";
        let size = text.measure(style, 120.0, s).unwrap();
        let rows = (size.h / line_h).round() as usize;
        assert!(rows >= 3, "{rows} rows");
        let run = Run {
            x: 10.0,
            y: 100.0,
            max_w: 120.0,
            style,
            text: s.into(),
        };
        let ls = lines(&mut text, &run, line_h);
        assert_eq!(ls.len(), rows, "{ls:?}");
        assert_eq!(ls[0].start, 0);
        for w in ls.windows(2) {
            assert_eq!(w[0].end, w[1].start);
            assert!(s.as_bytes()[w[1].start] != b' ', "a line starts at a word");
        }
        let n = SemNode {
            id: 1,
            parent_id: 0,
            role: role::TEXT,
            x: 10.0,
            y: 100.0,
            w: 120.0,
            h: size.h,
            text: s.into(),
        };
        // The start of the second line.
        let at = offset_at(&mut text, &n, Some(&run), 10.5, 100.0 + line_h * 1.5);
        assert_eq!(at, ls[1].start);
        // Far right on the first line: its end.
        let at = offset_at(&mut text, &n, Some(&run), 500.0, 101.0);
        assert_eq!(at, ls[0].end);
        // Above and below.
        assert_eq!(offset_at(&mut text, &n, Some(&run), 50.0, 0.0), 0);
        assert_eq!(offset_at(&mut text, &n, Some(&run), 50.0, 900.0), s.len());
        // Inside the first word: a boundary strictly inside it.
        let x2 = text.caret(style, s, 2).unwrap().0;
        assert_eq!(offset_at(&mut text, &n, Some(&run), 10.0 + x2, 101.0), 2);
        // A span over the first wrap: two rectangles, on rows 0 and 1.
        let r = span_rects(&mut text, &n, Some(&run), 2, ls[1].start + 2);
        assert_eq!(r.len(), 2, "{r:?}");
        assert_eq!(r[0].y, 100.0);
        assert_eq!(r[1].y, 100.0 + line_h);
        assert!(r[0].x > 10.0 && r[1].x == 10.0);
        // Without a run the node is taken whole.
        assert_eq!(
            span_rects(&mut text, &n, None, 1, 2),
            vec![Rect::new(10.0, 100.0, 120.0, size.h)]
        );
        assert_eq!(offset_at(&mut text, &n, None, 11.0, 101.0), 0);
        assert_eq!(offset_at(&mut text, &n, None, 129.0, 101.0), s.len());
        let runs = vec![run.clone()];
        assert_eq!(run_for(&n, &runs), Some(&run));
        let mut other = n.clone();
        other.text = "annan".into();
        assert_eq!(run_for(&other, &runs), None);
    }
}
