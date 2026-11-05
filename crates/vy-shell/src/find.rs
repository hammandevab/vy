//! Find on page (Ctrl+F): a search of the page's semantic tree
//! (SPEC-10.1.1), never of its pixels. Case-insensitive with simple
//! Unicode folding (each character's lowercase form, so `Å` finds `å` and
//! `SPEC` finds `spec`); matches are byte ranges of node texts in document
//! order, highlighted by the shell's overlay like the selection.
//!
//! Scrolling belongs to the module and the ABI has no way to ask for it,
//! so a match outside the viewport is brought towards it with synthetic
//! `SCROLL` events (see [`scroll_delta`]): a heuristic that works for a
//! page that scrolls its main area by `SCROLL`'s `dy`, as the reference
//! pages do, and does nothing harmful for one that does not.

use vy_runtime::SemNode;

/// One match: a node (by index into the tree and by id) and a byte range
/// of its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    /// Index into the tree.
    pub index: usize,
    /// The node's id.
    pub id: i32,
    /// First byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
}

/// Most matches kept; a search for a single letter in a long page stops
/// here.
pub const MAX_MATCHES: usize = 10_000;
/// Most `SCROLL` events sent to bring one match into view.
pub const MAX_SCROLL_STEPS: u32 = 8;

/// `s` folded for comparison, and for each byte of the folded text the
/// byte offset of the character it came from in `s` (plus one entry for
/// the end).
fn fold(s: &str) -> (String, Vec<usize>) {
    let mut out = String::with_capacity(s.len());
    let mut from = Vec::with_capacity(s.len() + 1);
    for (i, c) in s.char_indices() {
        for l in c.to_lowercase() {
            let before = out.len();
            out.push(l);
            from.extend(std::iter::repeat_n(i, out.len() - before));
        }
    }
    from.push(s.len());
    (out, from)
}

/// The query as it is compared: folded, surrounding whitespace dropped.
fn folded_query(query: &str) -> String {
    fold(query.trim()).0
}

/// Every match of `query` in the texts of `sem`, in document order,
/// non-overlapping within a node; none for an empty query.
pub fn search(sem: &[SemNode], query: &str) -> Vec<Match> {
    let q = folded_query(query);
    if q.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (index, node) in sem.iter().enumerate() {
        if node.text.is_empty() {
            continue;
        }
        let (hay, from) = fold(&node.text);
        let mut at = 0;
        while let Some(pos) = hay[at..].find(&q) {
            let (fs, fe) = (at + pos, at + pos + q.len());
            // Back to the original text: the characters the folded range
            // came from, whole.
            let start = from[fs];
            let end = from[fe..]
                .iter()
                .copied()
                .find(|&o| o > start)
                .unwrap_or(node.text.len());
            out.push(Match {
                index,
                id: node.id,
                start,
                end,
            });
            if out.len() == MAX_MATCHES {
                return out;
            }
            at = fe;
        }
    }
    out
}

/// The match to show first for a fresh search: the first one whose node
/// is at or below the top of the viewport, else the first.
pub fn first_visible(sem: &[SemNode], matches: &[Match]) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(
        matches
            .iter()
            .position(|m| sem.get(m.index).is_some_and(|n| n.y + n.h > 0.0))
            .unwrap_or(0),
    )
}

/// The next (or previous) match after `current`, wrapping around.
pub fn step(current: Option<usize>, total: usize, forward: bool) -> Option<usize> {
    if total == 0 {
        return None;
    }
    Some(match (current, forward) {
        (None, true) => 0,
        (None, false) => total - 1,
        (Some(i), true) => (i + 1) % total,
        (Some(i), false) => (i + total - 1) % total,
    })
}

/// Where `old` is among `fresh` matches of a new tree: the same node and
/// start, else the first at or after its place in document order.
pub fn carry(old: Option<Match>, fresh: &[Match]) -> Option<usize> {
    let old = old?;
    if fresh.is_empty() {
        return None;
    }
    fresh
        .iter()
        .position(|m| m.id == old.id && m.start == old.start)
        .or_else(|| {
            fresh
                .iter()
                .position(|m| (m.index, m.start) >= (old.index, old.start))
        })
        .or(Some(0))
}

/// "3 av 7" for the bar; "Inga träffar" for a query without matches;
/// nothing for an empty query.
pub fn label(query: &str, current: Option<usize>, total: usize) -> String {
    if query.trim().is_empty() {
        String::new()
    } else if total == 0 {
        "Inga träffar".into()
    } else {
        format!("{} av {}", current.map_or(0, |c| c + 1), total)
    }
}

/// The `SCROLL` distance (logical pixels, positive = the view moves down)
/// that would bring a node to the middle of a viewport `height` high, or
/// `None` when the node is already wholly inside it.
pub fn scroll_delta(node: &SemNode, height: f32) -> Option<f32> {
    let inside = node.y >= 0.0 && node.y + node.h.min(height) <= height;
    if inside {
        return None;
    }
    Some(node.y + node.h.min(height) / 2.0 - height / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: i32, y: f32, text: &str) -> SemNode {
        SemNode {
            id,
            parent_id: 0,
            role: vy_abi::role::TEXT,
            x: 0.0,
            y,
            w: 100.0,
            h: 20.0,
            text: text.into(),
        }
    }

    /// SPEC-10.1.1, SPEC-10.2.2: find searches the tree's logical text,
    /// case-insensitively with simple folding, in document order; the
    /// ranges are the original text's, whole characters; Enter and
    /// Shift+Enter wrap around; the label counts from one.
    #[test]
    fn spec_10_1_1_find_in_the_tree() {
        let sem = vec![
            node(1, -200.0, "Ångström och ÅNGA"),
            node(2, 10.0, "Ingen träff här"),
            node(3, 50.0, "ånga, ånga, ÅNGA"),
            node(4, 900.0, "SPEC-10.2.2 och spec"),
        ];
        let m = search(&sem, "ÅNG");
        assert_eq!(m.len(), 5, "{m:?}");
        assert_eq!(
            (m[0].index, m[0].start, m[0].end),
            (0, 0, "Ång".len()),
            "bytes of the original text"
        );
        assert_eq!(&sem[0].text[m[1].start..m[1].end], "ÅNG");
        assert_eq!(&sem[2].text[m[3].start..m[3].end], "ång");
        assert_eq!(search(&sem, "spec").len(), 2);
        assert_eq!(search(&sem, "  ").len(), 0, "empty query");
        assert_eq!(search(&sem, "xyz").len(), 0);
        // Fresh search: the first match at or below the viewport's top.
        assert_eq!(first_visible(&sem, &m), Some(2));
        assert_eq!(first_visible(&sem, &[]), None);
        // Cycling wraps.
        assert_eq!(step(Some(4), 5, true), Some(0));
        assert_eq!(step(Some(0), 5, false), Some(4));
        assert_eq!(step(None, 5, false), Some(4));
        assert_eq!(step(None, 0, true), None);
        // A new tree keeps the current match where it can.
        let moved = search(&sem[1..], "ång");
        assert_eq!(carry(Some(m[3]), &moved), Some(1));
        assert_eq!(carry(None, &moved), None);
        assert_eq!(label("ång", Some(2), 7), "3 av 7");
        assert_eq!(label("ång", None, 0), "Inga träffar");
        assert_eq!(label("", None, 0), "");
        // Scrolling towards a match: none when it is in view.
        assert_eq!(scroll_delta(&sem[1], 400.0), None);
        assert_eq!(scroll_delta(&sem[3], 400.0), Some(900.0 + 10.0 - 200.0));
        assert_eq!(scroll_delta(&sem[0], 400.0), Some(-200.0 + 10.0 - 200.0));
        // A folding that changes length (`İ` lowercases to two chars).
        let odd = vec![node(9, 0.0, "aİb")];
        let m = search(&odd, "b");
        assert_eq!(&odd[0].text[m[0].start..m[0].end], "b");
    }
}
