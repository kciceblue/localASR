/// Tracks the exact text we have typed into the focused app during a session.
/// On every update, computes the minimal diff: how many trailing chars to delete
/// (via backspaces) and what new tail to append (via paste).
#[derive(Debug, Default, Clone)]
pub struct TypedState {
    typed: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Diff {
    pub backspace_count: usize,
    pub append: String,
}

impl TypedState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> &str { &self.typed }

    /// Compute the diff needed to bring the typed text to `new_full`.
    /// Returns the diff and updates internal state.
    pub fn update(&mut self, new_full: &str) -> Diff {
        let common = common_prefix_len_chars(&self.typed, new_full);
        let typed_chars: usize = self.typed.chars().count();
        let backspace_count = typed_chars.saturating_sub(common);
        let append: String = new_full.chars().skip(common).collect();
        self.typed = new_full.to_string();
        Diff { backspace_count, append }
    }

    pub fn reset(&mut self) {
        self.typed.clear();
    }
}

fn common_prefix_len_chars(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_to_text() {
        let mut s = TypedState::new();
        let d = s.update("hello");
        assert_eq!(d, Diff { backspace_count: 0, append: "hello".into() });
        assert_eq!(s.current(), "hello");
    }

    #[test]
    fn append_only() {
        let mut s = TypedState::new();
        s.update("hello");
        let d = s.update("hello world");
        assert_eq!(d, Diff { backspace_count: 0, append: " world".into() });
    }

    #[test]
    fn replace_tail() {
        let mut s = TypedState::new();
        s.update("hello wrold");
        let d = s.update("hello world");
        // Common prefix is "hello w" (7 chars); diff replaces the trailing "rold" with "orld".
        assert_eq!(d, Diff { backspace_count: 4, append: "orld".into() });
    }

    #[test]
    fn full_replacement() {
        let mut s = TypedState::new();
        s.update("foo");
        let d = s.update("bar");
        assert_eq!(d, Diff { backspace_count: 3, append: "bar".into() });
    }

    #[test]
    fn truncation_only() {
        let mut s = TypedState::new();
        s.update("hello world");
        let d = s.update("hello");
        assert_eq!(d, Diff { backspace_count: 6, append: "".into() });
    }

    #[test]
    fn unchanged() {
        let mut s = TypedState::new();
        s.update("same");
        let d = s.update("same");
        assert_eq!(d, Diff { backspace_count: 0, append: "".into() });
    }

    #[test]
    fn multibyte_chars_counted_correctly() {
        let mut s = TypedState::new();
        s.update("café");
        let d = s.update("cafés");
        assert_eq!(d, Diff { backspace_count: 0, append: "s".into() });

        let d2 = s.update("café");
        assert_eq!(d2, Diff { backspace_count: 1, append: "".into() });
    }

    #[test]
    fn reset_clears_state() {
        let mut s = TypedState::new();
        s.update("hello");
        s.reset();
        let d = s.update("world");
        assert_eq!(d, Diff { backspace_count: 0, append: "world".into() });
    }
}
