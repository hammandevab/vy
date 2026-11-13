//! A small Markdown parser: the source in, blocks of styled text out.
//!
//! Blocks: headings (`#`, `##`, `###`), paragraphs, block quotes (`>`),
//! unordered (`-`, `*`, `+`) and ordered (`1.`) list items in two levels,
//! fenced code blocks and horizontal rules. Inline: `**bold**`, `*italic*`
//! and `_italic_`, `` `code` ``, `[text](url)` and backslash escapes.
//! Nothing here touches the host, so the same code runs in the native
//! tests.

use alloc::string::String;
use alloc::vec::Vec;

/// Style bits of a [`Run`].
pub const BOLD: u8 = 1;
/// Emphasis.
pub const ITALIC: u8 = 2;
/// Inline code.
pub const CODE: u8 = 4;

/// What a block is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A heading of level 1 to 3.
    Heading(u8),
    /// A paragraph.
    Para,
    /// A block quote.
    Quote,
    /// An unordered list item; `nested` for the second level.
    Bullet {
        /// True for an item indented under another.
        nested: bool,
    },
    /// An ordered list item with the number as written.
    Number {
        /// True for an item indented under another.
        nested: bool,
        /// The number before the dot.
        n: u16,
    },
    /// A fenced code block; the text keeps its line breaks.
    Code,
    /// A horizontal rule.
    Rule,
}

impl Kind {
    /// True for a list item of either sort.
    pub fn is_item(self) -> bool {
        matches!(self, Kind::Bullet { .. } | Kind::Number { .. })
    }

    /// True when an item of this kind goes on the list that `first`
    /// began: any nested item, or an item of the same sort.
    pub fn continues(self, first: Kind) -> bool {
        matches!(
            (self, first),
            (Kind::Bullet { nested: true }, _)
                | (Kind::Number { nested: true, .. }, _)
                | (Kind::Bullet { .. }, Kind::Bullet { .. })
                | (Kind::Number { .. }, Kind::Number { .. })
        )
    }
}

/// A stretch of a block's text in one style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    /// Byte range into [`Block::text`].
    pub start: usize,
    /// End of the range.
    pub end: usize,
    /// [`BOLD`], [`ITALIC`], [`CODE`] bits.
    pub style: u8,
    /// Index into [`Doc::links`] when the run is a link's text.
    pub link: Option<u16>,
}

/// One block of the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    /// What it is.
    pub kind: Kind,
    /// The text without its markup.
    pub text: String,
    /// The text's styles, in order, covering all of it.
    pub runs: Vec<Run>,
    /// Byte range of the block in the source, for following the caret.
    pub src: (usize, usize),
}

/// A parsed document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Doc {
    /// The blocks, top to bottom.
    pub blocks: Vec<Block>,
    /// The target of every link, in order of appearance.
    pub links: Vec<String>,
}

impl Doc {
    /// The block the source offset `at` falls in, or the last one before it.
    pub fn block_at(&self, at: usize) -> Option<usize> {
        self.blocks.iter().rposition(|b| b.src.0 <= at)
    }
}

/// Words (runs of non-whitespace) and characters of `src`.
pub fn count(src: &str) -> (usize, usize) {
    (src.split_whitespace().count(), src.chars().count())
}

/// The list marker a line starts with: the item's kind and where its
/// text begins.
fn item(line: &str) -> Option<(Kind, usize)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let nested = indent >= 2;
    let rest = &line[indent..];
    let bytes = rest.as_bytes();
    if matches!(bytes.first(), Some(b'-' | b'*' | b'+')) && bytes.get(1) == Some(&b' ') {
        return Some((Kind::Bullet { nested }, indent + 2));
    }
    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if (1..=4).contains(&digits)
        && bytes.get(digits) == Some(&b'.')
        && bytes.get(digits + 1) == Some(&b' ')
    {
        let n = rest[..digits].parse().unwrap_or(0);
        return Some((Kind::Number { nested, n }, indent + digits + 2));
    }
    None
}

/// True for a line of three or more `-`, `*` or `_` and nothing else.
fn rule(line: &str) -> bool {
    let t = line.trim();
    let first = t.as_bytes().first().copied();
    t.len() >= 3 && matches!(first, Some(b'-' | b'*' | b'_')) && t.bytes().all(|b| Some(b) == first)
}

/// A block being collected: paragraphs, quotes and list items go on
/// over the following plain lines.
struct Open {
    kind: Kind,
    text: String,
    start: usize,
    end: usize,
}

/// Parse `src` into blocks.
pub fn parse(src: &str) -> Doc {
    let mut doc = Doc::default();
    let mut open: Option<Open> = None;
    let mut fence: Option<Open> = None;
    let mut at = 0;
    let close = |doc: &mut Doc, open: &mut Option<Open>| {
        if let Some(o) = open.take() {
            let (text, runs) = match o.kind {
                Kind::Code => {
                    let runs = Vec::from([Run {
                        start: 0,
                        end: o.text.len(),
                        style: CODE,
                        link: None,
                    }]);
                    (o.text, runs)
                }
                _ => inline(&o.text, &mut doc.links),
            };
            doc.blocks.push(Block {
                kind: o.kind,
                text,
                runs,
                src: (o.start, o.end),
            });
        }
    };
    for raw in src.split('\n') {
        let (start, end) = (at, at + raw.len());
        at = end + 1;
        let line = raw.trim_end();
        let body = line.trim_start();
        if let Some(code) = fence.as_mut() {
            if body.starts_with("```") {
                code.end = end;
                close(&mut doc, &mut fence);
            } else {
                // `end` is past `start` once the block has a line.
                if code.end > code.start {
                    code.text.push('\n');
                }
                code.text.push_str(line);
                code.end = end;
            }
            continue;
        }
        let begin = |kind: Kind, text: &str| Open {
            kind,
            text: String::from(text.trim()),
            start,
            end,
        };
        if body.starts_with("```") {
            close(&mut doc, &mut open);
            fence = Some(Open {
                kind: Kind::Code,
                text: String::new(),
                start,
                end: start,
            });
        } else if body.is_empty() {
            close(&mut doc, &mut open);
        } else if let Some(level) = (1..=3u8).rev().find(|n| {
            line.strip_prefix(&"###"[..usize::from(*n)])
                .is_some_and(|rest| rest.starts_with(' '))
        }) {
            close(&mut doc, &mut open);
            open = Some(begin(Kind::Heading(level), &line[usize::from(level)..]));
            close(&mut doc, &mut open);
        } else if rule(line) {
            close(&mut doc, &mut open);
            open = Some(begin(Kind::Rule, ""));
            close(&mut doc, &mut open);
        } else if let Some(rest) = body.strip_prefix('>') {
            match open.as_mut() {
                Some(o) if o.kind == Kind::Quote => {
                    o.text.push(' ');
                    o.text.push_str(rest.trim());
                    o.end = end;
                }
                _ => {
                    close(&mut doc, &mut open);
                    open = Some(begin(Kind::Quote, rest));
                }
            }
        } else if let Some((kind, text_at)) = item(line) {
            close(&mut doc, &mut open);
            open = Some(begin(kind, &line[text_at..]));
        } else {
            // Plain text: more of what is open, or a new paragraph.
            match open.as_mut() {
                Some(o) => {
                    o.text.push(' ');
                    o.text.push_str(body);
                    o.end = end;
                }
                None => open = Some(begin(Kind::Para, body)),
            }
        }
    }
    close(&mut doc, &mut open);
    // A fence left open runs to the end of the text.
    close(&mut doc, &mut fence);
    doc
}

/// Where the next occurrence of one byte is, remembered between look-ups.
///
/// A text with one opener after another and no closer (`[[[[…`, or
/// `[a](` over and over) would otherwise be searched to its end once for
/// every opener: quadratic, and enough to stop a page on the 6 000 bytes
/// it holds (granskning 0013:1). The look-ups of one pass over a text
/// start further and further on, so what one of them found, or that it
/// found nothing, answers the ones after it until they have passed that
/// place; the stretches searched never overlap, and the whole pass looks
/// at each byte once.
pub(crate) struct Next {
    byte: u8,
    /// Where the remembered search started; past any text before the first.
    from: usize,
    at: Option<usize>,
}

impl Next {
    pub(crate) fn new(byte: u8) -> Self {
        Self {
            byte,
            from: usize::MAX,
            at: None,
        }
    }

    /// The first `byte` of `b` at or after `from`.
    fn find(&mut self, b: &[u8], from: usize) -> Option<usize> {
        let known = self.from <= from && self.at.is_none_or(|at| at >= from);
        if !known {
            let rest = b.get(from..).unwrap_or(&[]);
            let found = rest.iter().position(|c| *c == self.byte);
            #[cfg(test)]
            SEARCHED.with(|n| n.set(n.get() + found.map_or(rest.len(), |n| n + 1)));
            self.from = from;
            self.at = found.map(|n| from + n);
        }
        self.at
    }
}

#[cfg(test)]
std::thread_local! {
    /// Bytes the look-ups of [`Next`] examined on this thread: what the
    /// tests bound for texts made to be searched over and over.
    pub static SEARCHED: core::cell::Cell<usize> = const { core::cell::Cell::new(0) };
}

/// The text of a link starting at `[` in `b[i]`: where the text ends and
/// where the target starts and ends. `close` and `paren` remember where
/// the next `]` and `)` are.
pub(crate) fn link_at(
    b: &[u8],
    i: usize,
    close: &mut Next,
    paren: &mut Next,
) -> Option<(usize, usize, usize)> {
    let text_end = close.find(b, i + 1)?;
    if b.get(text_end + 1) != Some(&b'(') {
        return None;
    }
    let end = paren.find(b, text_end + 2)?;
    (text_end > i + 1 && end > text_end + 2).then_some((text_end, text_end + 2, end))
}

fn alnum_before(s: &str, i: usize) -> bool {
    s[..i]
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
}

fn alnum_after(s: &str, i: usize) -> bool {
    s[i..].chars().next().is_some_and(char::is_alphanumeric)
}

/// The inline markup of a block's text taken out: the plain text and its
/// runs. Links add their targets to `links`.
fn inline(src: &str, links: &mut Vec<String>) -> (String, Vec<Run>) {
    let b = src.as_bytes();
    let mut out = String::new();
    let mut runs: Vec<Run> = Vec::new();
    let mut style = 0u8;
    // The marker that opened the emphasis, to close with the same.
    let mut emphasis = 0u8;
    let mut from = 0;
    let mut i = 0;
    let (mut close, mut paren) = (Next::new(b']'), Next::new(b')'));
    // End the run of plain or emphasised text collected so far.
    fn cut(out: &str, runs: &mut Vec<Run>, from: &mut usize, style: u8, link: Option<u16>) {
        if out.len() > *from {
            runs.push(Run {
                start: *from,
                end: out.len(),
                style,
                link,
            });
            *from = out.len();
        }
    }
    while i < b.len() {
        match b[i] {
            b'\\' if b.get(i + 1).is_some_and(u8::is_ascii_punctuation) => {
                out.push(b[i + 1] as char);
                i += 2;
            }
            b'`' => match b[i + 1..].iter().position(|c| *c == b'`') {
                Some(n) if n > 0 => {
                    cut(&out, &mut runs, &mut from, style, None);
                    out.push_str(&src[i + 1..i + 1 + n]);
                    cut(&out, &mut runs, &mut from, style | CODE, None);
                    i += n + 2;
                }
                _ => {
                    out.push('`');
                    i += 1;
                }
            },
            b'*' if b.get(i + 1) == Some(&b'*') => {
                if style & BOLD != 0 || src[i + 2..].contains("**") {
                    cut(&out, &mut runs, &mut from, style, None);
                    style ^= BOLD;
                } else {
                    out.push_str("**");
                }
                i += 2;
            }
            c @ (b'*' | b'_') => {
                let word = c == b'_';
                let toggles = if style & ITALIC != 0 {
                    c == emphasis && !(word && alnum_after(src, i + 1))
                } else {
                    b[i + 1..].contains(&c)
                        && b.get(i + 1).is_some_and(|n| *n != b' ')
                        && !(word && alnum_before(src, i))
                };
                if toggles {
                    cut(&out, &mut runs, &mut from, style, None);
                    style ^= ITALIC;
                    emphasis = c;
                } else {
                    out.push(c as char);
                }
                i += 1;
            }
            b'[' => match link_at(b, i, &mut close, &mut paren) {
                Some((text_end, url, url_end)) => {
                    cut(&out, &mut runs, &mut from, style, None);
                    let link = links.len() as u16;
                    links.push(String::from(src[url..url_end].trim()));
                    out.push_str(&src[i + 1..text_end]);
                    cut(&out, &mut runs, &mut from, style, Some(link));
                    i = url_end + 1;
                }
                None => {
                    out.push('[');
                    i += 1;
                }
            },
            _ => {
                let ch = src[i..].chars().next().unwrap_or(' ');
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    cut(&out, &mut runs, &mut from, style, None);
    (out, runs)
}
