use ratatui::crossterm::event::KeyCode;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const MAX_INPUT_BYTES: usize = 8_192;

#[derive(Clone, Debug, Default)]
pub struct Editor {
    pub text: String,
    pub cursor: usize,
    pub dirty: bool,
}

impl Editor {
    pub fn new(text: String) -> Self {
        let mut editor = Self::default();
        editor.insert(&text);
        editor.dirty = false;
        editor
    }

    pub fn insert(&mut self, text: &str) {
        let clean = text.replace('\t', "    ").replace('\r', "");
        for grapheme in clean.graphemes(true) {
            if grapheme.chars().any(|ch| ch.is_control() && ch != '\n') {
                continue;
            }
            if self.text.len() + grapheme.len() > MAX_INPUT_BYTES {
                break;
            }
            self.text.insert_str(self.cursor, grapheme);
            self.cursor += grapheme.len();
            self.dirty = true;
        }
    }

    fn previous(&self) -> usize {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index)
    }

    fn next(&self) -> usize {
        self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.text.len(), |ch| self.cursor + ch.len())
    }

    pub fn key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char(ch) => self.insert(&ch.to_string()),
            KeyCode::Enter => self.insert("\n"),
            KeyCode::Left => self.cursor = self.previous(),
            KeyCode::Right => self.cursor = self.next(),
            KeyCode::Home => {
                self.cursor = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
            }
            KeyCode::End => {
                self.cursor = self.text[self.cursor..]
                    .find('\n')
                    .map_or(self.text.len(), |i| self.cursor + i);
            }
            KeyCode::Up | KeyCode::Down => self.move_line(code == KeyCode::Down),
            KeyCode::Backspace if self.cursor > 0 => {
                let start = self.previous();
                self.text.drain(start..self.cursor);
                self.cursor = start;
                self.dirty = true;
            }
            KeyCode::Delete if self.cursor < self.text.len() => {
                self.text.drain(self.cursor..self.next());
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn move_line(&mut self, down: bool) {
        let start = self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1);
        let column = self.text[start..self.cursor].graphemes(true).count();
        let target = if down {
            self.text[self.cursor..]
                .find('\n')
                .map(|i| self.cursor + i + 1)
        } else if start > 0 {
            Some(self.text[..start - 1].rfind('\n').map_or(0, |i| i + 1))
        } else {
            None
        };
        if let Some(target) = target {
            let line = self.text[target..].split('\n').next().unwrap_or_default();
            self.cursor = target
                + line
                    .graphemes(true)
                    .take(column)
                    .map(str::len)
                    .sum::<usize>();
        }
    }

    /// Use the same cell-aware wrapping for drawing and cursor placement.
    pub fn wrapped(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(2);
        let mut lines = vec![String::new()];
        let mut col = 0;
        let mut cursor = (0, 0);
        for (index, grapheme) in self.text.grapheme_indices(true) {
            let cells = grapheme.width();
            if grapheme != "\n" && col + cells > width {
                lines.push(String::new());
                col = 0;
            }
            if index == self.cursor {
                cursor = (col, lines.len() - 1);
            }
            if grapheme == "\n" {
                lines.push(String::new());
                col = 0;
            } else {
                lines.last_mut().unwrap().push_str(grapheme);
                col += cells;
            }
        }
        if self.cursor == self.text.len() {
            if col >= width {
                lines.push(String::new());
                col = 0;
            }
            cursor = (col, lines.len() - 1);
        }
        (lines, cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_deletion_navigation_and_wrapping() {
        let mut editor = Editor::new("a界e\u{301}🙂".into());
        editor.key(KeyCode::Left);
        editor.key(KeyCode::Backspace);
        assert_eq!(editor.text, "a界🙂");
        editor.key(KeyCode::Home);
        editor.key(KeyCode::Delete);
        assert_eq!(editor.text, "界🙂");
        editor.key(KeyCode::End);
        assert_eq!(editor.wrapped(3), (vec!["界".into(), "🙂".into()], (2, 1)));
    }

    #[test]
    fn paste_is_bounded_and_navigation_is_not_text() {
        let mut editor = Editor::default();
        editor.insert(&"界".repeat(MAX_INPUT_BYTES));
        assert!(editor.text.len() <= MAX_INPUT_BYTES);
        assert!(editor.text.is_char_boundary(editor.cursor));
        let text = editor.text.clone();
        editor.key(KeyCode::F(1));
        assert_eq!(text, editor.text);
    }
}
