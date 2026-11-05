//! One tab's history: the addresses visited, back and forward.

/// A history stack with a cursor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    entries: Vec<String>,
    /// Index of the current entry; meaningless while `entries` is empty.
    at: usize,
}

impl History {
    /// Empty.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current address, if any.
    pub fn current(&self) -> Option<&str> {
        self.entries.get(self.at).map(String::as_str)
    }

    /// Visit `address`: it becomes current and everything after the old
    /// current entry is forgotten. Visiting the current address again
    /// adds nothing.
    pub fn push(&mut self, address: &str) {
        if self.current() == Some(address) {
            return;
        }
        if !self.entries.is_empty() {
            self.entries.truncate(self.at + 1);
        }
        self.entries.push(address.to_string());
        self.at = self.entries.len() - 1;
    }

    /// Replace the current entry (`navigate` with `replace`, a redirect);
    /// a push when there is none.
    pub fn replace(&mut self, address: &str) {
        match self.entries.get_mut(self.at) {
            Some(e) => *e = address.to_string(),
            None => self.push(address),
        }
    }

    /// True when there is somewhere to go back to.
    pub fn can_back(&self) -> bool {
        self.at > 0 && !self.entries.is_empty()
    }

    /// True when there is somewhere to go forward to.
    pub fn can_forward(&self) -> bool {
        self.at + 1 < self.entries.len()
    }

    /// Step back; the address to load, or `None` at the start.
    pub fn back(&mut self) -> Option<&str> {
        if !self.can_back() {
            return None;
        }
        self.at -= 1;
        self.current()
    }

    /// Step forward; the address to load, or `None` at the end.
    pub fn forward(&mut self) -> Option<&str> {
        if !self.can_forward() {
            return None;
        }
        self.at += 1;
        self.current()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True with no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Back and forward walk the stack; a new visit after going back
    /// drops the forward entries; replace rewrites the current one.
    #[test]
    fn history_stack() {
        let mut h = History::new();
        assert_eq!(h.current(), None);
        assert!(!h.can_back() && !h.can_forward());
        assert_eq!(h.back(), None);
        h.push("vy://a.example");
        h.push("vy://a.example");
        assert_eq!(h.len(), 1, "the same address twice is one entry");
        h.push("vy://b.example");
        h.push("vy://c.example");
        assert_eq!(h.current(), Some("vy://c.example"));
        assert!(h.can_back() && !h.can_forward());
        assert_eq!(h.back(), Some("vy://b.example"));
        assert_eq!(h.back(), Some("vy://a.example"));
        assert_eq!(h.back(), None);
        assert!(h.can_forward());
        assert_eq!(h.forward(), Some("vy://b.example"));
        h.push("vy://d.example");
        assert!(!h.can_forward(), "forward entries dropped");
        assert_eq!(h.len(), 3);
        assert_eq!(h.forward(), None);
        h.replace("vy://d.example/2");
        assert_eq!(h.current(), Some("vy://d.example/2"));
        assert_eq!(h.len(), 3);
        let mut empty = History::new();
        empty.replace("vy://x.example");
        assert_eq!(empty.current(), Some("vy://x.example"));
        assert!(!empty.is_empty());
    }
}
