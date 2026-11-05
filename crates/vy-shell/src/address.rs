//! The address field's editing model (SPEC-9.6.1): the text being
//! edited, a caret and a selection, and the committed address it goes
//! back to on Escape. Pure state; the chrome draws it and the window feeds
//! it keys and text.

/// What a key does to the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Caret left (with `select`: extend the selection).
    Left {
        /// Extend the selection instead of moving the caret alone.
        select: bool,
        /// By a word instead of a character.
        word: bool,
    },
    /// Caret right.
    Right {
        /// Extend the selection.
        select: bool,
        /// By a word.
        word: bool,
    },
    /// Caret to the start.
    Home {
        /// Extend the selection.
        select: bool,
    },
    /// Caret to the end.
    End {
        /// Extend the selection.
        select: bool,
    },
    /// Delete before the caret, or the selection.
    Backspace {
        /// A whole word.
        word: bool,
    },
    /// Delete after the caret, or the selection.
    Delete {
        /// A whole word.
        word: bool,
    },
    /// Select everything.
    SelectAll,
}

/// The field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddressField {
    /// The address shown when the field is not being edited.
    committed: String,
    /// The text being edited.
    text: String,
    /// Caret as a byte offset into `text`, always on a char boundary.
    caret: usize,
    /// The other end of the selection, if any.
    anchor: Option<usize>,
    /// True while the field has keyboard focus.
    focused: bool,
}

impl Default for AddressField {
    fn default() -> Self {
        Self::new()
    }
}

impl AddressField {
    /// An empty, unfocused field.
    pub fn new() -> Self {
        Self {
            committed: String::new(),
            text: String::new(),
            caret: 0,
            anchor: None,
            focused: false,
        }
    }

    /// The text to draw: the edit in progress, or the committed address.
    pub fn text(&self) -> &str {
        if self.focused {
            &self.text
        } else {
            &self.committed
        }
    }

    /// The committed address.
    pub fn committed(&self) -> &str {
        &self.committed
    }

    /// True while the field has keyboard focus.
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// Caret byte offset into [`AddressField::text`].
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected byte range, if any and not empty.
    pub fn selection(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        if a == self.caret {
            return None;
        }
        Some((a.min(self.caret), a.max(self.caret)))
    }

    /// The page navigated (or started loading) `address`: shown when the
    /// field is not focused, and what Escape goes back to.
    pub fn set_committed(&mut self, address: &str) {
        self.committed = address.to_string();
        if !self.focused {
            self.text = self.committed.clone();
            self.caret = self.text.len();
            self.anchor = None;
        }
    }

    /// Ctrl+L or a click: take focus with the whole address selected, so
    /// typing replaces it.
    pub fn focus_all(&mut self) {
        self.focused = true;
        self.text = self.committed.clone();
        self.caret = self.text.len();
        self.anchor = Some(0);
        if self.text.is_empty() {
            self.anchor = None;
        }
    }

    /// A click at byte offset `at` of the shown text: focus with the caret
    /// there and no selection.
    pub fn focus_at(&mut self, at: usize) {
        if !self.focused {
            self.text = self.committed.clone();
        }
        self.focused = true;
        self.caret = self.boundary(at.min(self.text.len()));
        self.anchor = None;
    }

    /// Escape: give up the edit and show the committed address again.
    pub fn cancel(&mut self) {
        self.focused = false;
        self.text = self.committed.clone();
        self.caret = self.text.len();
        self.anchor = None;
    }

    /// Enter: the text to navigate to (trimmed), if there is any; the
    /// field keeps it as committed and loses focus. `None` for an empty
    /// field, which cancels instead.
    pub fn commit(&mut self) -> Option<String> {
        let text = self.text.trim().to_string();
        self.focused = false;
        self.anchor = None;
        if text.is_empty() {
            self.text = self.committed.clone();
            self.caret = self.text.len();
            return None;
        }
        self.committed = text.clone();
        self.text = text.clone();
        self.caret = self.text.len();
        Some(text)
    }

    /// Focus lost to the page or another part of the chrome: the edit is
    /// kept, like a browser does, but the field is no longer focused.
    pub fn blur(&mut self) {
        self.focused = false;
        self.anchor = None;
    }

    /// Typed or pasted text, replacing the selection. Line breaks are
    /// dropped: an address is one line.
    pub fn insert(&mut self, s: &str) {
        if !self.focused {
            return;
        }
        let clean: String = s.chars().filter(|c| !c.is_control()).collect();
        self.delete_selection();
        self.text.insert_str(self.caret, &clean);
        self.caret += clean.len();
    }

    /// The selected text, for copying.
    pub fn selected_text(&self) -> &str {
        match self.selection() {
            Some((a, b)) => &self.text[a..b],
            None => "",
        }
    }

    /// Apply an editing key.
    pub fn edit(&mut self, edit: Edit) {
        if !self.focused {
            return;
        }
        match edit {
            Edit::Left { select, word } => {
                if !select && self.selection().is_some() {
                    self.caret = self.selection().map_or(self.caret, |(a, _)| a);
                    self.anchor = None;
                    return;
                }
                self.keep_or_drop_anchor(select);
                self.caret = if word {
                    self.word_left(self.caret)
                } else {
                    self.char_left(self.caret)
                };
            }
            Edit::Right { select, word } => {
                if !select && self.selection().is_some() {
                    self.caret = self.selection().map_or(self.caret, |(_, b)| b);
                    self.anchor = None;
                    return;
                }
                self.keep_or_drop_anchor(select);
                self.caret = if word {
                    self.word_right(self.caret)
                } else {
                    self.char_right(self.caret)
                };
            }
            Edit::Home { select } => {
                self.keep_or_drop_anchor(select);
                self.caret = 0;
            }
            Edit::End { select } => {
                self.keep_or_drop_anchor(select);
                self.caret = self.text.len();
            }
            Edit::Backspace { word } => {
                if self.delete_selection() {
                    return;
                }
                let to = if word {
                    self.word_left(self.caret)
                } else {
                    self.char_left(self.caret)
                };
                self.text.replace_range(to..self.caret, "");
                self.caret = to;
            }
            Edit::Delete { word } => {
                if self.delete_selection() {
                    return;
                }
                let to = if word {
                    self.word_right(self.caret)
                } else {
                    self.char_right(self.caret)
                };
                self.text.replace_range(self.caret..to, "");
            }
            Edit::SelectAll => {
                self.anchor = if self.text.is_empty() { None } else { Some(0) };
                self.caret = self.text.len();
            }
        }
    }

    fn keep_or_drop_anchor(&mut self, select: bool) {
        if select {
            if self.anchor.is_none() {
                self.anchor = Some(self.caret);
            }
        } else {
            self.anchor = None;
        }
    }

    /// Remove the selection; true when there was one.
    fn delete_selection(&mut self) -> bool {
        let Some((a, b)) = self.selection() else {
            self.anchor = None;
            return false;
        };
        self.text.replace_range(a..b, "");
        self.caret = a;
        self.anchor = None;
        true
    }

    /// The nearest char boundary at or before `at`.
    fn boundary(&self, mut at: usize) -> usize {
        while at > 0 && !self.text.is_char_boundary(at) {
            at -= 1;
        }
        at
    }

    fn char_left(&self, at: usize) -> usize {
        self.text[..at]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn char_right(&self, at: usize) -> usize {
        self.text[at..]
            .chars()
            .next()
            .map_or(at, |c| at + c.len_utf8())
    }

    /// Word boundaries for an address: runs of alphanumerics; `/`, `.`,
    /// `:` and the rest are separators.
    fn is_word(c: char) -> bool {
        c.is_alphanumeric() || c == '-' || c == '_'
    }

    fn word_left(&self, at: usize) -> usize {
        let mut i = at;
        let chars: Vec<(usize, char)> = self.text[..at].char_indices().collect();
        let mut it = chars.iter().rev().peekable();
        while let Some((idx, c)) = it.peek() {
            if Self::is_word(*c) {
                break;
            }
            i = *idx;
            it.next();
        }
        while let Some((idx, c)) = it.peek() {
            if !Self::is_word(*c) {
                break;
            }
            i = *idx;
            it.next();
        }
        i
    }

    fn word_right(&self, at: usize) -> usize {
        let mut i = at;
        let mut it = self.text[at..].char_indices().peekable();
        while let Some((_, c)) = it.peek() {
            if Self::is_word(*c) {
                break;
            }
            i = at + it.next().map_or(0, |(idx, c)| idx + c.len_utf8());
        }
        while let Some((_, c)) = it.peek() {
            if !Self::is_word(*c) {
                break;
            }
            i = at + it.next().map_or(0, |(idx, c)| idx + c.len_utf8());
        }
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-9.6.1: the address field is the client's own: Ctrl+L selects
    /// the whole address so typing replaces it, Enter commits the trimmed
    /// text, Escape restores the committed address, and editing keys move
    /// the caret by characters and words through UTF-8 safely.
    #[test]
    fn spec_9_6_1_address_field_editing() {
        let mut f = AddressField::new();
        assert_eq!(f.text(), "");
        f.set_committed("vy://docs.example/rapport");
        assert_eq!(f.text(), "vy://docs.example/rapport");
        assert!(!f.focused());
        f.insert("x");
        assert_eq!(f.text(), "vy://docs.example/rapport", "unfocused: nothing");

        f.focus_all();
        assert!(f.focused());
        assert_eq!(f.selection(), Some((0, 25)));
        assert_eq!(f.selected_text(), "vy://docs.example/rapport");
        f.insert("vy://");
        assert_eq!(f.text(), "vy://");
        assert_eq!(f.caret(), 5);
        f.insert("shop.ex\nample");
        assert_eq!(f.text(), "vy://shop.example", "no line breaks");
        f.edit(Edit::Left {
            select: false,
            word: true,
        });
        assert_eq!(f.caret(), "vy://shop.".len());
        f.edit(Edit::Left {
            select: true,
            word: true,
        });
        assert_eq!(f.selected_text(), "shop.");
        f.edit(Edit::Backspace { word: false });
        assert_eq!(f.text(), "vy://example");
        f.edit(Edit::Home { select: false });
        f.edit(Edit::Delete { word: true });
        assert_eq!(f.text(), "://example");
        f.edit(Edit::Delete { word: true });
        assert_eq!(f.text(), "");
        f.insert("  vy://åäö.example/ö  ");
        f.edit(Edit::End { select: false });
        f.edit(Edit::Left {
            select: false,
            word: false,
        });
        f.edit(Edit::Left {
            select: false,
            word: false,
        });
        f.edit(Edit::Backspace { word: false });
        assert_eq!(
            f.text(),
            "  vy://åäö.example/  ",
            "one multi-byte char removed"
        );
        f.edit(Edit::SelectAll);
        assert_eq!(f.selection(), Some((0, f.text().len())));
        f.edit(Edit::Right {
            select: false,
            word: false,
        });
        assert_eq!(f.selection(), None);
        assert_eq!(f.caret(), f.text().len());
        assert_eq!(f.commit().as_deref(), Some("vy://åäö.example/"));
        assert!(!f.focused());
        assert_eq!(f.committed(), "vy://åäö.example/");
        assert_eq!(f.text(), "vy://åäö.example/");

        f.focus_at(2);
        assert_eq!(f.caret(), 2);
        f.insert("X");
        assert_eq!(f.text(), "vyX://åäö.example/");
        f.cancel();
        assert_eq!(f.text(), "vy://åäö.example/", "escape restores");
        assert!(!f.focused());
        f.focus_all();
        f.edit(Edit::Backspace { word: false });
        assert_eq!(f.text(), "");
        assert_eq!(f.commit(), None, "an empty commit cancels");
        assert_eq!(f.text(), "vy://åäö.example/");
        f.focus_at(100);
        assert_eq!(f.caret(), f.text().len(), "clamped");
        f.focus_at("vy://å".len() - 1);
        assert_eq!(f.caret(), "vy://".len(), "never inside a char");
        f.blur();
        assert!(!f.focused());
        f.set_committed("vy://other.example");
        assert_eq!(f.text(), "vy://other.example");
    }
}
