//! Headless grid ANSI parser for PTY output sanitization (blueprint §6.1).

pub const GRID_COLS: usize = 220;
pub const GRID_ROWS: usize = 50;

/// In-memory terminal grid that applies PTY bytes like a real emulator.
pub struct VirtualGrid {
    cells: Vec<Vec<char>>,
    cursor_row: usize,
    cursor_col: usize,
    vte_parser: vte::Parser,
}

impl VirtualGrid {
    pub fn new() -> Self {
        Self {
            cells: vec![vec![' '; GRID_COLS]; GRID_ROWS],
            cursor_row: 0,
            cursor_col: 0,
            vte_parser: vte::Parser::new(),
        }
    }

    /// Feed raw PTY bytes into the virtual terminal.
    pub fn feed(&mut self, bytes: &[u8]) {
        let mut parser = std::mem::replace(&mut self.vte_parser, vte::Parser::new());
        for &byte in bytes {
            parser.advance(self, byte);
        }
        self.vte_parser = parser;
    }

    /// Alias used by the sanitizer task.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.feed(bytes);
    }

    /// Return visible text from the grid (ANSI stripped, `\r` overwrites honored).
    pub fn extract_text(&self) -> String {
        let end_row = self.cursor_row.min(GRID_ROWS.saturating_sub(1));
        let lines: Vec<String> = (0..=end_row)
            .map(|row| {
                self.cells[row]
                    .iter()
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        let start = lines
            .iter()
            .position(|line| !line.is_empty())
            .unwrap_or(lines.len());
        let end = lines
            .iter()
            .rposition(|line| !line.is_empty())
            .map(|i| i + 1)
            .unwrap_or(start);
        if start >= end {
            return String::new();
        }
        lines[start..end].join("\n")
    }

    fn write_char(&mut self, c: char) {
        if self.cursor_row >= GRID_ROWS {
            self.scroll_up();
            self.cursor_row = GRID_ROWS.saturating_sub(1);
        }
        if self.cursor_col >= GRID_COLS {
            self.cursor_col = GRID_COLS.saturating_sub(1);
        }

        self.cells[self.cursor_row][self.cursor_col] = c;
        self.cursor_col += 1;
        if self.cursor_col >= GRID_COLS {
            self.cursor_col = 0;
            self.cursor_row += 1;
            if self.cursor_row >= GRID_ROWS {
                self.scroll_up();
                self.cursor_row = GRID_ROWS - 1;
            }
        }
    }

    fn scroll_up(&mut self) {
        self.cells.remove(0);
        self.cells.push(vec![' '; GRID_COLS]);
    }

    fn clamp_cursor(&mut self) {
        self.cursor_row = self.cursor_row.min(GRID_ROWS.saturating_sub(1));
        self.cursor_col = self.cursor_col.min(GRID_COLS.saturating_sub(1));
    }

    fn clear_entire_grid(&mut self) {
        for row in self.cells.iter_mut() {
            row.fill(' ');
        }
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    fn clear_line(&mut self, mode: u16) {
        if self.cursor_row >= GRID_ROWS {
            return;
        }
        let row = self.cursor_row;
        match mode {
            0 => {
                for col in self.cursor_col..GRID_COLS {
                    self.cells[row][col] = ' ';
                }
            }
            1 => {
                for col in 0..=self.cursor_col.min(GRID_COLS - 1) {
                    self.cells[row][col] = ' ';
                }
            }
            2 => self.cells[row].fill(' '),
            _ => {}
        }
    }
}

impl Default for VirtualGrid {
    fn default() -> Self {
        Self::new()
    }
}

impl vte::Perform for VirtualGrid {
    fn print(&mut self, c: char) {
        self.write_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\r' => self.cursor_col = 0,
            b'\n' => {
                self.cursor_row += 1;
                self.cursor_col = 0;
                if self.cursor_row >= GRID_ROWS {
                    self.scroll_up();
                    self.cursor_row = GRID_ROWS - 1;
                }
            }
            0x08 => self.cursor_col = self.cursor_col.saturating_sub(1),
            0x07 => {}
            _ => {}
        }
    }

    fn csi_dispatch(
        &mut self,
        params: &vte::Params,
        _intermediates: &[u8],
        ignore: bool,
        action: char,
    ) {
        if ignore {
            return;
        }

        match action {
            'A' => {
                let n = csi_param(params, 0, 1);
                self.cursor_row = self.cursor_row.saturating_sub(n as usize);
                self.clamp_cursor();
            }
            'B' => {
                let n = csi_param(params, 0, 1);
                self.cursor_row = (self.cursor_row + n as usize).min(GRID_ROWS - 1);
            }
            'C' => {
                let n = csi_param(params, 0, 1);
                self.cursor_col = (self.cursor_col + n as usize).min(GRID_COLS - 1);
            }
            'D' => {
                let n = csi_param(params, 0, 1);
                self.cursor_col = self.cursor_col.saturating_sub(n as usize);
            }
            'G' => {
                let col = csi_param(params, 0, 1);
                self.cursor_col = col.saturating_sub(1) as usize;
                self.clamp_cursor();
            }
            'H' | 'f' => {
                let row = csi_param(params, 0, 1);
                let col = csi_param(params, 1, 1);
                self.cursor_row = row.saturating_sub(1) as usize;
                self.cursor_col = col.saturating_sub(1) as usize;
                self.clamp_cursor();
            }
            'J' => {
                if csi_param(params, 0, 0) == 2 {
                    self.clear_entire_grid();
                }
            }
            'K' => self.clear_line(csi_param(params, 0, 0)),
            'm' => {}
            _ => {}
        }
    }
}

/// Last non-empty line of sanitized text (for prompt / auto-reply checks).
#[must_use]
pub fn last_non_empty_line(text: &str) -> Option<&str> {
    text.lines().rev().find_map(|line| {
        let trimmed = line.trim_end_matches('\r').trim();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

fn csi_param(params: &vte::Params, index: usize, default: u16) -> u16 {
    params
        .iter()
        .nth(index)
        .and_then(|p| p.first().copied())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::VirtualGrid;

    #[test]
    fn spinner_carriage_return_overwrites_one_line() {
        let mut grid = VirtualGrid::new();
        for frame in ["-", "\\", "|", "/"] {
            grid.feed(format!("\r{frame}").as_bytes());
        }
        grid.feed(b"\r");
        grid.feed(b"done");
        assert_eq!(grid.extract_text(), "done");
    }

    #[test]
    fn ansi_colors_ignored() {
        let mut grid = VirtualGrid::new();
        grid.feed(b"\x1b[1;31mhello\x1b[0m world");
        assert_eq!(grid.extract_text(), "hello world");
    }

    #[test]
    fn erase_line_k_clears_to_end() {
        let mut grid = VirtualGrid::new();
        grid.feed(b"hello world");
        grid.feed(b"\x1b[6D");
        grid.feed(b"\x1b[K");
        assert_eq!(grid.extract_text(), "hello");
    }

    #[test]
    fn erase_line_k_clears_entire_line() {
        let mut grid = VirtualGrid::new();
        grid.feed(b"hello world");
        grid.feed(b"\x1b[2K");
        assert_eq!(grid.extract_text(), "");
    }

    #[test]
    fn erase_display_j_param_2_clears_entire_grid() {
        let mut grid = VirtualGrid::new();
        grid.feed(b"hello");
        grid.feed(b"\nworld");
        grid.feed(b"\x1b[2J");
        grid.feed(b"after");
        assert_eq!(grid.extract_text(), "after");
    }

    #[test]
    fn cursor_column_g_is_1_based() {
        let mut grid = VirtualGrid::new();
        grid.feed(b"abcdef");
        grid.feed(b"\x1b[3G");
        grid.feed(b"Z");
        assert_eq!(grid.extract_text(), "abZdef");
    }

    #[test]
    fn cursor_position_h_moves_and_overwrites() {
        let mut grid = VirtualGrid::new();
        grid.feed(b"hello");
        grid.feed(b"\nworld");
        grid.feed(b"\x1b[1;1H");
        grid.feed(b"HEY");
        assert_eq!(grid.extract_text(), "HEYlo");
    }

    #[test]
    fn cursor_relative_moves_abcd_respected() {
        let mut grid = VirtualGrid::new();
        grid.feed(b"one\ntwo\nthree");
        grid.feed(b"\x1b[2A");
        grid.feed(b"\x1b[1G");
        grid.feed(b"ONE");
        grid.feed(b"\x1b[1B");
        grid.feed(b"\x1b[2C");
        grid.feed(b"X");
        // Note: cursor up/down preserves column. After writing "ONE" (col=3),
        // moving down and then forward 2 columns places 'X' at col=5.
        assert_eq!(grid.extract_text(), "ONE\ntwo  X");
    }

    #[test]
    fn newline_resets_column_so_prompt_starts_at_line_beginning() {
        let mut grid = VirtualGrid::new();
        grid.feed(b"Response body\n> \n");
        assert_eq!(grid.extract_text(), "Response body\n>");
    }

    #[test]
    fn extract_text_trims_trailing_blank_lines() {
        let mut grid = VirtualGrid::new();
        grid.feed(b"Hello World\n");
        assert_eq!(grid.extract_text(), "Hello World");
    }

    #[test]
    fn extract_text_stops_at_cursor_row() {
        let mut grid = VirtualGrid::new();
        grid.feed(b"line1\nline2\nline3");
        grid.feed(b"\x1b[1;1H");
        assert_eq!(grid.extract_text(), "line1");
    }
}
