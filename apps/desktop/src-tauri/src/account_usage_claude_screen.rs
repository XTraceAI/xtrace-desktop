//! A small, bounded terminal screen for the Claude `/usage` probe. It replays
//! the cursor-addressed output Claude Code's terminal UI writes so the probe
//! can see what is on screen (trust dialog, usage panel) the way a person
//! would. Colors and other attributes are ignored; nothing is stored beyond
//! the fixed grid.

pub(super) struct Screen {
    rows: usize,
    cols: usize,
    cells: Vec<Vec<char>>,
    row: usize,
    col: usize,
    wrap_pending: bool,
    saved: (usize, usize),
    state: Parse,
    params: String,
    utf8: Vec<u8>,
    replies: Vec<u8>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Parse {
    Ground,
    Escape,
    /// ESC followed by an intermediate such as `(`: one more byte ends it.
    EscapeIntermediate,
    Csi,
    /// OSC, DCS, APC or PM string: ends with BEL or ESC `\`.
    String,
    StringEscape,
}

/// Parameters longer than this are not a real control sequence.
const MAX_PARAMS: usize = 64;

impl Screen {
    pub(super) fn new(rows: usize, cols: usize) -> Self {
        Self {
            rows,
            cols,
            cells: vec![vec![' '; cols]; rows],
            row: 0,
            col: 0,
            wrap_pending: false,
            saved: (0, 0),
            state: Parse::Ground,
            params: String::new(),
            utf8: Vec::new(),
            replies: Vec::new(),
        }
    }

    /// Bytes the terminal must answer with (cursor position reports), drained.
    pub(super) fn take_replies(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.replies)
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.byte(byte);
        }
    }

    /// The visible screen, one line per row, trailing spaces trimmed.
    pub(super) fn text(&self) -> String {
        let mut out = String::new();
        for (index, row) in self.cells.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            let line: String = row.iter().filter(|c| **c != WIDE_TAIL).collect();
            out.push_str(line.trim_end());
        }
        out
    }

    fn byte(&mut self, byte: u8) {
        match self.state {
            Parse::Ground => self.ground(byte),
            Parse::Escape => self.escape(byte),
            Parse::EscapeIntermediate => self.state = Parse::Ground,
            Parse::Csi => self.csi(byte),
            Parse::String => match byte {
                0x07 => self.state = Parse::Ground,
                0x1b => self.state = Parse::StringEscape,
                _ => {}
            },
            Parse::StringEscape => {
                self.state = if byte == b'\\' {
                    Parse::Ground
                } else {
                    Parse::String
                }
            }
        }
    }

    fn ground(&mut self, byte: u8) {
        if !self.utf8.is_empty() || byte >= 0x80 {
            self.utf8.push(byte);
            match std::str::from_utf8(&self.utf8) {
                Ok(text) => {
                    let chars: Vec<char> = text.chars().collect();
                    self.utf8.clear();
                    for c in chars {
                        self.print(c);
                    }
                }
                Err(error) if error.error_len().is_none() && self.utf8.len() < 4 => {}
                Err(_) => {
                    self.utf8.clear();
                    self.print('\u{fffd}');
                }
            }
            return;
        }
        match byte {
            0x1b => {
                self.state = Parse::Escape;
                self.params.clear();
            }
            b'\r' => {
                self.col = 0;
                self.wrap_pending = false;
            }
            b'\n' | 0x0b | 0x0c => {
                self.line_feed();
            }
            0x08 => {
                self.col = self.col.saturating_sub(1);
                self.wrap_pending = false;
            }
            b'\t' => {
                self.col = ((self.col / 8) + 1) * 8;
                if self.col >= self.cols {
                    self.col = self.cols - 1;
                }
            }
            0x20..=0x7e => self.print(byte as char),
            _ => {}
        }
    }

    fn escape(&mut self, byte: u8) {
        self.state = Parse::Ground;
        match byte {
            b'[' => self.state = Parse::Csi,
            b']' | b'P' | b'_' | b'^' | b'X' => self.state = Parse::String,
            b'(' | b')' | b'*' | b'+' | b'#' | b'%' => self.state = Parse::EscapeIntermediate,
            b'7' => self.saved = (self.row, self.col),
            b'8' => {
                (self.row, self.col) = self.saved;
                self.wrap_pending = false;
            }
            b'D' => self.line_feed(),
            b'E' => {
                self.col = 0;
                self.line_feed();
            }
            b'M' => {
                if self.row == 0 {
                    self.cells.pop();
                    self.cells.insert(0, vec![' '; self.cols]);
                } else {
                    self.row -= 1;
                }
            }
            b'c' => self.clear_all(),
            _ => {}
        }
    }

    fn csi(&mut self, byte: u8) {
        if (0x40..=0x7e).contains(&byte) {
            self.state = Parse::Ground;
            let params = std::mem::take(&mut self.params);
            self.dispatch(&params, byte);
            return;
        }
        if (0x20..=0x3f).contains(&byte) {
            if self.params.len() < MAX_PARAMS {
                self.params.push(byte as char);
            }
            return;
        }
        // A stray control byte inside a sequence ends it.
        self.state = Parse::Ground;
        self.ground(byte);
    }

    fn dispatch(&mut self, params: &str, final_byte: u8) {
        let private = params.starts_with(['?', '>', '<', '=']);
        let numbers: Vec<usize> = params
            .trim_start_matches(['?', '>', '<', '='])
            .split([';', ':'])
            .map(|part| part.parse::<usize>().unwrap_or(0))
            .collect();
        let first = numbers.first().copied().unwrap_or(0);
        let count = first.max(1);
        self.wrap_pending = false;
        match final_byte {
            b'A' => self.row = self.row.saturating_sub(count),
            b'B' | b'e' => self.row = (self.row + count).min(self.rows - 1),
            b'C' | b'a' => self.col = (self.col + count).min(self.cols - 1),
            b'D' => self.col = self.col.saturating_sub(count),
            b'E' => {
                self.row = (self.row + count).min(self.rows - 1);
                self.col = 0;
            }
            b'F' => {
                self.row = self.row.saturating_sub(count);
                self.col = 0;
            }
            b'G' | b'`' => self.col = (count - 1).min(self.cols - 1),
            b'd' => self.row = (count - 1).min(self.rows - 1),
            b'H' | b'f' => {
                let row = numbers.first().copied().unwrap_or(1).max(1);
                let col = numbers.get(1).copied().unwrap_or(1).max(1);
                self.row = (row - 1).min(self.rows - 1);
                self.col = (col - 1).min(self.cols - 1);
            }
            b'J' => match first {
                0 => {
                    self.erase_line_from(self.col);
                    for row in self.row + 1..self.rows {
                        self.cells[row].fill(' ');
                    }
                }
                1 => {
                    for row in 0..self.row {
                        self.cells[row].fill(' ');
                    }
                    self.erase_line_to(self.col);
                }
                _ => {
                    for row in &mut self.cells {
                        row.fill(' ');
                    }
                }
            },
            b'K' => match first {
                0 => self.erase_line_from(self.col),
                1 => self.erase_line_to(self.col),
                _ => self.cells[self.row].fill(' '),
            },
            b'X' => {
                let end = (self.col + count).min(self.cols);
                self.cells[self.row][self.col..end].fill(' ');
            }
            b'P' => {
                let line = &mut self.cells[self.row];
                for _ in 0..count.min(self.cols - self.col) {
                    line.remove(self.col);
                    line.push(' ');
                }
            }
            b'@' => {
                let line = &mut self.cells[self.row];
                for _ in 0..count.min(self.cols - self.col) {
                    line.pop();
                    line.insert(self.col, ' ');
                }
            }
            b'L' => {
                for _ in 0..count.min(self.rows - self.row) {
                    self.cells.pop();
                    self.cells.insert(self.row, vec![' '; self.cols]);
                }
            }
            b'M' => {
                for _ in 0..count.min(self.rows - self.row) {
                    self.cells.remove(self.row);
                    self.cells.push(vec![' '; self.cols]);
                }
            }
            b'S' => {
                for _ in 0..count.min(self.rows) {
                    self.cells.remove(0);
                    self.cells.push(vec![' '; self.cols]);
                }
            }
            b's' if !private => self.saved = (self.row, self.col),
            b'u' if !private => (self.row, self.col) = self.saved,
            b'n' if !private && first == 6 => {
                let reply = format!("\x1b[{};{}R", self.row + 1, self.col + 1);
                self.replies.extend_from_slice(reply.as_bytes());
            }
            b'h' | b'l' if private => {
                // Entering or leaving the alternate screen starts a blank page.
                if numbers.iter().any(|n| matches!(n, 47 | 1047 | 1049)) {
                    self.clear_all();
                }
            }
            _ => {}
        }
    }

    fn clear_all(&mut self) {
        for row in &mut self.cells {
            row.fill(' ');
        }
        self.row = 0;
        self.col = 0;
    }

    fn erase_line_from(&mut self, col: usize) {
        let col = col.min(self.cols);
        self.cells[self.row][col..].fill(' ');
    }

    fn erase_line_to(&mut self, col: usize) {
        let end = (col + 1).min(self.cols);
        self.cells[self.row][..end].fill(' ');
    }

    fn line_feed(&mut self) {
        self.wrap_pending = false;
        if self.row + 1 >= self.rows {
            self.cells.remove(0);
            self.cells.push(vec![' '; self.cols]);
        } else {
            self.row += 1;
        }
    }

    fn print(&mut self, c: char) {
        let width = char_width(c);
        if width == 0 {
            return;
        }
        if self.wrap_pending || self.col + width > self.cols {
            self.col = 0;
            self.line_feed();
        }
        self.cells[self.row][self.col] = c;
        if width == 2 && self.col + 1 < self.cols {
            self.cells[self.row][self.col + 1] = WIDE_TAIL;
        }
        self.col += width;
        if self.col >= self.cols {
            self.col = self.cols - 1;
            self.wrap_pending = true;
        }
    }
}

/// Marks the second cell of a double-width character; never printed.
const WIDE_TAIL: char = '\u{0}';

fn char_width(c: char) -> usize {
    let code = c as u32;
    if code < 0x20 || (0x7f..0xa0).contains(&code) {
        return 0;
    }
    // Combining marks and zero-width joiners/variation selectors.
    if (0x300..=0x36f).contains(&code)
        || (0x200b..=0x200f).contains(&code)
        || (0xfe00..=0xfe0f).contains(&code)
    {
        return 0;
    }
    let wide = (0x1100..=0x115f).contains(&code)
        || (0x2e80..=0xa4cf).contains(&code)
        || (0xac00..=0xd7a3).contains(&code)
        || (0xf900..=0xfaff).contains(&code)
        || (0xfe30..=0xfe4f).contains(&code)
        || (0xff00..=0xff60).contains(&code)
        || (0xffe0..=0xffe6).contains(&code)
        || (0x1f300..=0x1faff).contains(&code)
        || (0x20000..=0x3fffd).contains(&code);
    if wide { 2 } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replays_cursor_moves_erases_and_wrapping() {
        let mut screen = Screen::new(4, 10);
        screen.feed(b"hello\r\nworld");
        screen.feed(b"\x1b[1A\x1b[2K\x1b[Gbye");
        assert_eq!(screen.text(), "bye\nworld\n\n");
        screen.feed(b"\x1b[4;1H0123456789AB");
        // The last row wrapped, scrolling the screen up by one.
        assert_eq!(screen.text(), "world\n\n0123456789\nAB");
        screen.feed(b"\x1b[2J\x1b[H\x1b[31mred\x1b[0m \xe2\x9d\xaf ok");
        assert_eq!(screen.text().lines().next(), Some("red ❯ ok"));
    }

    #[test]
    fn answers_cursor_position_queries_and_skips_osc_strings() {
        let mut screen = Screen::new(5, 20);
        screen.feed(b"\x1b]0;title\x07ab\x1b[6n");
        assert_eq!(screen.take_replies(), b"\x1b[1;3R");
        assert!(screen.take_replies().is_empty());
        screen.feed(b"\x1b]11;?\x1b\\c");
        assert_eq!(screen.text().lines().next(), Some("abc"));
    }

    #[test]
    fn split_utf8_sequences_are_joined() {
        let mut screen = Screen::new(2, 10);
        let bytes = "❯ Yes".as_bytes();
        screen.feed(&bytes[..1]);
        screen.feed(&bytes[1..]);
        assert_eq!(screen.text().lines().next(), Some("❯ Yes"));
    }
}
