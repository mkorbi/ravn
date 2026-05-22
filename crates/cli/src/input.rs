//! Tiny readline-style input buffer for the TUI's input pane.
//!
//! Wraps `text: String` + `cursor: usize` (byte offset, always on a
//! UTF-8 char boundary) with the minimum primitives needed by the TUI:
//! insert, backspace, delete-forward, and the four cursor moves.

#[derive(Debug, Default, Clone)]
pub struct InputBuffer {
    pub text: String,
    pub cursor: usize,
}

impl InputBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }

    pub fn insert_char(&mut self, ch: char) {
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    /// Insert a whole string at the cursor (used to drop in a voice
    /// transcript). `cursor` stays on a char boundary since `s` is valid UTF-8.
    pub fn insert_str(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.text[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.text.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    pub fn delete_forward(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let next_len = self.text[self.cursor..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(0);
        self.text
            .replace_range(self.cursor..self.cursor + next_len, "");
    }

    pub fn cursor_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = self.text[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }

    pub fn cursor_right(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let step = self.text[self.cursor..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(0);
        self.cursor += step;
    }

    pub fn cursor_home(&mut self) {
        self.cursor = 0;
    }

    pub fn cursor_end(&mut self) {
        self.cursor = self.text.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_left_right_ascii() {
        let mut b = InputBuffer {
            text: "hello".into(),
            cursor: 5,
        };
        b.cursor_home();
        assert_eq!(b.cursor, 0);
        b.cursor_right();
        assert_eq!(b.cursor, 1);
        b.cursor_end();
        assert_eq!(b.cursor, 5);
        b.cursor_left();
        assert_eq!(b.cursor, 4);
    }

    #[test]
    fn cursor_left_right_utf8() {
        // 'ö' is 2 bytes, '🚀' is 4 bytes.
        let mut b = InputBuffer {
            text: "möö🚀".into(),
            cursor: 0,
        };
        b.cursor_right(); // past 'm'
        assert_eq!(b.cursor, 1);
        b.cursor_right(); // past first 'ö'
        assert_eq!(b.cursor, 3);
        b.cursor_right(); // past second 'ö'
        assert_eq!(b.cursor, 5);
        b.cursor_right(); // past 🚀
        assert_eq!(b.cursor, 9);
        b.cursor_right(); // at end — noop
        assert_eq!(b.cursor, 9);
        b.cursor_left(); // back over 🚀
        assert_eq!(b.cursor, 5);
    }

    #[test]
    fn insert_then_backspace() {
        let mut b = InputBuffer {
            text: "hello".into(),
            cursor: 1,
        };
        b.insert_char('X');
        assert_eq!(b.text, "hXello");
        assert_eq!(b.cursor, 2);
        b.backspace();
        assert_eq!(b.text, "hello");
        assert_eq!(b.cursor, 1);
    }

    #[test]
    fn delete_forward_handles_multibyte() {
        let mut b = InputBuffer {
            text: "öx".into(),
            cursor: 0,
        };
        b.delete_forward();
        assert_eq!(b.text, "x");
        assert_eq!(b.cursor, 0);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut b = InputBuffer {
            text: "abc".into(),
            cursor: 0,
        };
        b.backspace();
        assert_eq!(b.text, "abc");
        assert_eq!(b.cursor, 0);
    }

    #[test]
    fn insert_str_advances_cursor() {
        let mut b = InputBuffer {
            text: "ac".into(),
            cursor: 1,
        };
        b.insert_str("XYZ");
        assert_eq!(b.text, "aXYZc");
        assert_eq!(b.cursor, 4);
    }

    #[test]
    fn take_clears_and_returns_text() {
        let mut b = InputBuffer {
            text: "hi".into(),
            cursor: 2,
        };
        assert_eq!(b.take(), "hi");
        assert_eq!(b.text, "");
        assert_eq!(b.cursor, 0);
    }
}
