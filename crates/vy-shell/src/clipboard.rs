//! The clipboard behind Ctrl+C, Ctrl+V and Ctrl+X (SPEC-5.2.12,
//! SPEC-8.2.4, SPEC-10.2.2): the operating system's in a window, a value
//! in memory headless and in tests. The shell is the only thing that
//! touches it; a page sees it through `clipboard_read` (with the
//! capability, in its tab process) and writes it through
//! `clipboard_write`, which the main process puts here.
//!
//! Text only: the clipboard carries the logical string, never pixels
//! (SPEC-10.2.2). Line endings are normalised to `\n` on the way into a
//! page, since a page sees text the way it would be typed.

use std::fmt;

/// Most bytes of a paste delivered to a page at once: one `TEXT_INPUT`
/// per chunk, cut at character boundaries. Host policy, not ABI: the
/// event carries `(ptr, len)` from `vy_alloc` (SPEC-5.1.6), and a page's
/// arena may be small (the SDK's is 64 KB).
pub const PASTE_CHUNK_BYTES: usize = 4096;
/// Most bytes of a paste in all; the rest is dropped. The same cap as a
/// page's own `clipboard_write` (SPEC-9.3.2).
pub const MAX_PASTE_BYTES: usize = vy_abi::limits::MAX_CLIPBOARD_BYTES as usize;

/// Most bytes of a paste into the address bar: four times the longest
/// address a page may navigate to (SPEC-9.3.2), so that pasting a whole
/// document there costs nothing to draw.
pub const MAX_ADDRESS_PASTE_BYTES: usize = 4 * vy_abi::limits::MAX_URL_BYTES as usize;

/// Where the clipboard lives.
pub trait ClipboardBackend: Send {
    /// The clipboard's text, or `None` when it is empty, not text, or
    /// cannot be read right now.
    fn read(&mut self) -> Option<String>;
    /// Replace the clipboard with `text`.
    fn write(&mut self, text: &str);
    /// What the backend is, for the log.
    fn name(&self) -> &'static str;
}

impl fmt::Debug for dyn ClipboardBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ClipboardBackend({})", self.name())
    }
}

/// A clipboard in memory: the headless shell's, the tests', and the
/// fallback when the system's cannot be opened (no display).
#[derive(Debug, Default)]
pub struct MemoryClipboard {
    text: Option<String>,
}

impl MemoryClipboard {
    /// An empty clipboard.
    pub fn new() -> Self {
        Self::default()
    }
}

impl ClipboardBackend for MemoryClipboard {
    fn read(&mut self) -> Option<String> {
        self.text.clone().filter(|t| !t.is_empty())
    }
    fn write(&mut self, text: &str) {
        self.text = Some(text.to_string());
    }
    fn name(&self) -> &'static str {
        "memory"
    }
}

/// The operating system's clipboard, through `arboard`.
#[cfg(feature = "os-clipboard")]
pub struct SystemClipboard(arboard::Clipboard);

#[cfg(feature = "os-clipboard")]
impl SystemClipboard {
    /// Open the system clipboard; `Err` where there is none (no display,
    /// no session), in which case the caller falls back to memory.
    pub fn open() -> Result<Self, String> {
        arboard::Clipboard::new()
            .map(Self)
            .map_err(|e| e.to_string())
    }
}

#[cfg(feature = "os-clipboard")]
impl ClipboardBackend for SystemClipboard {
    fn read(&mut self) -> Option<String> {
        self.0.get_text().ok().filter(|t| !t.is_empty())
    }
    fn write(&mut self, text: &str) {
        let _ = self.0.set_text(text.to_string());
    }
    fn name(&self) -> &'static str {
        "system"
    }
}

/// The system clipboard when it can be opened, else memory. Never panics:
/// a client without a clipboard still runs (Linux CI has no display).
pub fn system_or_memory() -> Box<dyn ClipboardBackend> {
    #[cfg(feature = "os-clipboard")]
    match SystemClipboard::open() {
        Ok(c) => return Box::new(c),
        Err(e) => eprintln!("vy-shell: no system clipboard ({e}); using an in-memory one"),
    }
    Box::new(MemoryClipboard::new())
}

/// Text as a page should receive it: `\r\n` and `\r` become `\n`;
/// nothing else is touched.
pub fn normalize_line_endings(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// `text` cut into pieces of at most `max` bytes at character
/// boundaries, in order; nothing for an empty text.
pub fn chunks(text: &str, max: usize) -> Vec<&str> {
    let max = max.max(4);
    let mut out = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + max).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        out.push(&text[start..end]);
        start = end;
    }
    out
}

/// `text` cut to at most `max` bytes at a character boundary.
pub fn truncate(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// A paste as the page receives it: line endings normalised, cut to
/// `MAX_PASTE_BYTES` at a character boundary, in chunks of
/// `PASTE_CHUNK_BYTES`.
pub fn paste_chunks(text: &str) -> Vec<String> {
    let text = normalize_line_endings(text);
    chunks(truncate(&text, MAX_PASTE_BYTES), PASTE_CHUNK_BYTES)
        .into_iter()
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-5.2.12, SPEC-10.2.2: the memory clipboard holds what was
    /// written and reads empty as none; a paste reaches a page with `\n`
    /// line endings only, in character-boundary chunks, and never more
    /// than a page may write itself.
    #[test]
    fn spec_5_2_12_memory_clipboard_and_paste_chunks() {
        let mut m = MemoryClipboard::new();
        assert_eq!(m.read(), None);
        m.write("hej");
        assert_eq!(m.read().as_deref(), Some("hej"));
        m.write("");
        assert_eq!(m.read(), None, "empty is none");
        assert_eq!(m.name(), "memory");
        assert_eq!(normalize_line_endings("a\r\nb\rc\nd"), "a\nb\nc\nd");
        assert_eq!(normalize_line_endings("plain"), "plain");
        assert_eq!(chunks("", 4), Vec::<&str>::new());
        assert_eq!(chunks("abcdefgh", 4), vec!["abcd", "efgh"]);
        assert_eq!(chunks("abcdefghi", 4), vec!["abcd", "efgh", "i"]);
        // Never inside a character: "åäö" is six bytes.
        assert_eq!(chunks("åäö", 4), vec!["åä", "ö"]);
        assert_eq!(
            chunks("åäö", 1),
            vec!["åä", "ö"],
            "the cut is at least 4 bytes"
        );
        let long = "x".repeat(PASTE_CHUNK_BYTES * 2 + 1);
        let parts = paste_chunks(&long);
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.len() <= PASTE_CHUNK_BYTES));
        assert_eq!(parts.concat(), long);
        let over = "ö".repeat(MAX_PASTE_BYTES / 2 + 10);
        let parts = paste_chunks(&over);
        let total: usize = parts.iter().map(String::len).sum();
        assert!((MAX_PASTE_BYTES - 1..=MAX_PASTE_BYTES).contains(&total));
        assert!(parts.iter().all(|p| p.chars().all(|c| c == 'ö')));
        assert_eq!(paste_chunks("a\r\nb"), vec!["a\nb"]);
        assert_eq!(truncate("åäö", 3), "å");
        assert_eq!(truncate("åäö", 99), "åäö");
        assert_eq!(truncate("", 0), "");
        // Without a display (CI) the system clipboard cannot be opened and
        // memory stands in; either way, nothing panics.
        let system = system_or_memory();
        assert!(["system", "memory"].contains(&system.name()));
    }
}
