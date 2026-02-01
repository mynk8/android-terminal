use crate::core::glyph::Glyph;
use crate::core::types::Term;

const ESC_BUF_SIZ: usize = 512;
const ESC_ARG_SIZ: usize = 16;
const STR_BUF_SIZ: usize = 512;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParserState {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    OscString,
    DcsEntry,
    DcsPassthrough,
    SosPmApcString,
}

pub struct CsiEscape {
    pub buf: [u8; ESC_BUF_SIZ],
    pub len: usize,
    pub priv_mode: bool,
    pub args: [i32; ESC_ARG_SIZ],
    pub nargs: usize,
    pub mode: [u8; 2],
}

impl Default for CsiEscape {
    fn default() -> Self {
        Self {
            buf: [0; ESC_BUF_SIZ],
            len: 0,
            priv_mode: false,
            args: [0; ESC_ARG_SIZ],
            nargs: 0,
            mode: [0; 2],
        }
    }
}

impl CsiEscape {
    pub fn reset(&mut self) {
        self.len = 0;
        self.priv_mode = false;
        self.nargs = 0;
        self.mode = [0; 2];
        for arg in &mut self.args {
            *arg = 0;
        }
    }

    pub fn parse(&mut self) {
        let mut i = 0;
        self.nargs = 0;

        if i < self.len && self.buf[i] == b'?' {
            self.priv_mode = true;
            i += 1;
        }

        while i < self.len && self.nargs < ESC_ARG_SIZ {
            if self.buf[i].is_ascii_digit() {
                let mut val: i32 = 0;
                while i < self.len && self.buf[i].is_ascii_digit() {
                    val = val * 10 + (self.buf[i] - b'0') as i32;
                    i += 1;
                }
                self.args[self.nargs] = val;
                self.nargs += 1;
            } else if self.buf[i] == b';' {
                if self.nargs == 0 || (i > 0 && self.buf[i - 1] == b';') {
                    self.args[self.nargs] = 0;
                    self.nargs += 1;
                }
                i += 1;
            } else {
                break;
            }
        }

        if i < self.len {
            self.mode[0] = self.buf[i];
            if i + 1 < self.len {
                self.mode[1] = self.buf[i + 1];
            }
        }
    }

    #[inline]
    pub fn arg(&self, idx: usize, default: i32) -> i32 {
        if idx < self.nargs && self.args[idx] != 0 {
            self.args[idx]
        } else {
            default
        }
    }
}

pub struct Parser {
    state: ParserState,
    csi: CsiEscape,
    osc_buf: [u8; STR_BUF_SIZ],
    osc_len: usize,
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

impl Parser {
    pub fn new() -> Self {
        Self {
            state: ParserState::Ground,
            csi: CsiEscape::default(),
            osc_buf: [0; STR_BUF_SIZ],
            osc_len: 0,
        }
    }

    pub fn process(&mut self, term: &mut Term, c: u8) {
        match self.state {
            ParserState::Ground => self.ground(term, c),
            ParserState::Escape => self.escape(term, c),
            ParserState::CsiEntry => self.csi_entry(term, c),
            ParserState::CsiParam => self.csi_param(term, c),
            ParserState::OscString => self.osc_string(term, c),
            _ => self.ground(term, c),
        }
    }

    fn ground(&mut self, term: &mut Term, c: u8) {
        match c {
            0x00 => {} // NUL - ignore
            0x07 => {} // BEL - bell (ignore for now)
            0x08 => {
                // BS - backspace
                if term.cursor.x > 0 {
                    term.cursor.x -= 1;
                    term.dirty[term.cursor.y] = true;
                }
            }
            0x09 => {
                // HT - horizontal tab
                let mut x = term.cursor.x;
                x = (x + 8) & !7; // Next tab stop (every 8 columns)
                if x >= term.cols {
                    x = term.cols - 1;
                }
                term.cursor.x = x;
                term.dirty[term.cursor.y] = true;
            }
            0x0a | 0x0b | 0x0c => {
                // LF, VT, FF - line feed
                self.newline(term);
            }
            0x0d => {
                // CR - carriage return
                term.cursor.x = 0;
                term.dirty[term.cursor.y] = true;
            }
            0x1b => {
                // ESC - escape
                self.state = ParserState::Escape;
            }
            // Printable ASCII
            0x20..=0x7e => {
                self.put_char(term, c as char);
            }
            // DEL - ignore
            0x7f => {}
            // C1 control characters (8-bit) - handle before general 0x80..=0xbf
            0x90 => {
                // DCS
                self.state = ParserState::DcsEntry;
            }
            0x9b => {
                // CSI (8-bit)
                self.csi.reset();
                self.state = ParserState::CsiEntry;
            }
            0x9d => {
                // OSC (8-bit)
                self.osc_len = 0;
                self.state = ParserState::OscString;
            }
            // UTF-8 continuation bytes and other C1 - ignore for now
            0x80..=0xbf => {}
            // UTF-8 start bytes - treat as printable for now
            0xc0..=0xff => {
                // TODO: proper UTF-8 decoding
            }
            _ => {}
        }
    }

    fn escape(&mut self, term: &mut Term, c: u8) {
        match c {
            b'[' => {
                self.csi.reset();
                self.state = ParserState::CsiEntry;
            }
            b']' => {
                self.osc_len = 0;
                self.state = ParserState::OscString;
            }
            b'(' | b')' | b'*' | b'+' => {
                // Charset designation - ignore
                self.state = ParserState::EscapeIntermediate;
            }
            b'D' => {
                // IND - Index (line feed)
                self.newline(term);
                self.state = ParserState::Ground;
            }
            b'E' => {
                // NEL - Next line
                term.cursor.x = 0;
                self.newline(term);
                self.state = ParserState::Ground;
            }
            b'H' => {
                // HTS - Horizontal tab set
                // TODO: set tab stop
                self.state = ParserState::Ground;
            }
            b'M' => {
                // RI - Reverse index
                if term.cursor.y == 0 {
                    self.scroll_down(term);
                } else {
                    term.cursor.y -= 1;
                    term.dirty[term.cursor.y] = true;
                }
                self.state = ParserState::Ground;
            }
            b'7' => {
                // DECSC - Save cursor
                // TODO: save cursor position
                self.state = ParserState::Ground;
            }
            b'8' => {
                // DECRC - Restore cursor
                // TODO: restore cursor position
                self.state = ParserState::Ground;
            }
            b'c' => {
                // RIS - Reset to initial state
                term.reset();
                self.state = ParserState::Ground;
            }
            b'\\' => {
                // ST - String terminator
                self.state = ParserState::Ground;
            }
            _ => {
                // Unknown escape sequence
                self.state = ParserState::Ground;
            }
        }
    }

    /// CSI entry state
    fn csi_entry(&mut self, term: &mut Term, c: u8) {
        match c {
            b'0'..=b'9' | b';' | b'?' | b':' => {
                if self.csi.len < ESC_BUF_SIZ {
                    self.csi.buf[self.csi.len] = c;
                    self.csi.len += 1;
                }
                self.state = ParserState::CsiParam;
            }
            0x40..=0x7e => {
                // Final character - execute CSI
                if self.csi.len < ESC_BUF_SIZ {
                    self.csi.buf[self.csi.len] = c;
                    self.csi.len += 1;
                }
                self.csi.parse();
                self.csi_dispatch(term, c);
                self.state = ParserState::Ground;
            }
            _ => {
                self.state = ParserState::Ground;
            }
        }
    }

    /// CSI param state
    fn csi_param(&mut self, term: &mut Term, c: u8) {
        match c {
            b'0'..=b'9' | b';' | b'?' | b':' => {
                if self.csi.len < ESC_BUF_SIZ {
                    self.csi.buf[self.csi.len] = c;
                    self.csi.len += 1;
                }
            }
            0x40..=0x7e => {
                // Final character
                if self.csi.len < ESC_BUF_SIZ {
                    self.csi.buf[self.csi.len] = c;
                    self.csi.len += 1;
                }
                self.csi.parse();
                self.csi_dispatch(term, c);
                self.state = ParserState::Ground;
            }
            _ => {
                self.state = ParserState::Ground;
            }
        }
    }

    /// OSC string state
    fn osc_string(&mut self, _term: &mut Term, c: u8) {
        match c {
            0x07 | 0x9c => {
                // BEL or ST terminates OSC
                // TODO: handle OSC parameters
                self.state = ParserState::Ground;
            }
            0x1b => {
                // ESC might start ST (\x1b\\)
                self.state = ParserState::Escape;
            }
            _ => {
                if self.osc_len < STR_BUF_SIZ {
                    self.osc_buf[self.osc_len] = c;
                    self.osc_len += 1;
                }
            }
        }
    }

    /// Dispatch CSI sequence
    fn csi_dispatch(&mut self, term: &mut Term, c: u8) {
        match c {
            b'@' => {
                // ICH - Insert characters
                let n = self.csi.arg(0, 1) as usize;
                self.insert_blank(term, n);
            }
            b'A' => {
                // CUU - Cursor up
                let n = self.csi.arg(0, 1) as usize;
                self.move_cursor(term, 0, -(n as isize));
            }
            b'B' | b'e' => {
                // CUD - Cursor down
                let n = self.csi.arg(0, 1) as usize;
                self.move_cursor(term, 0, n as isize);
            }
            b'C' | b'a' => {
                // CUF - Cursor forward
                let n = self.csi.arg(0, 1) as usize;
                self.move_cursor(term, n as isize, 0);
            }
            b'D' => {
                // CUB - Cursor back
                let n = self.csi.arg(0, 1) as usize;
                self.move_cursor(term, -(n as isize), 0);
            }
            b'E' => {
                // CNL - Cursor next line
                let n = self.csi.arg(0, 1) as usize;
                term.cursor.x = 0;
                self.move_cursor(term, 0, n as isize);
            }
            b'F' => {
                // CPL - Cursor previous line
                let n = self.csi.arg(0, 1) as usize;
                term.cursor.x = 0;
                self.move_cursor(term, 0, -(n as isize));
            }
            b'G' | b'`' => {
                // CHA - Cursor horizontal absolute
                let x = self.csi.arg(0, 1) as usize;
                self.move_to(term, x.saturating_sub(1), term.cursor.y);
            }
            b'H' | b'f' => {
                // CUP - Cursor position
                let y = self.csi.arg(0, 1) as usize;
                let x = self.csi.arg(1, 1) as usize;
                self.move_to(term, x.saturating_sub(1), y.saturating_sub(1));
            }
            b'J' => {
                // ED - Erase in display
                match self.csi.arg(0, 0) {
                    0 => self.clear_region(
                        term,
                        term.cursor.x,
                        term.cursor.y,
                        term.cols - 1,
                        term.rows - 1,
                    ),
                    1 => self.clear_region(term, 0, 0, term.cursor.x, term.cursor.y),
                    2 | 3 => self.clear_region(term, 0, 0, term.cols - 1, term.rows - 1),
                    _ => {}
                }
            }
            b'K' => {
                // EL - Erase in line
                match self.csi.arg(0, 0) {
                    0 => self.clear_region(
                        term,
                        term.cursor.x,
                        term.cursor.y,
                        term.cols - 1,
                        term.cursor.y,
                    ),
                    1 => self.clear_region(term, 0, term.cursor.y, term.cursor.x, term.cursor.y),
                    2 => self.clear_region(term, 0, term.cursor.y, term.cols - 1, term.cursor.y),
                    _ => {}
                }
            }
            b'L' => {
                // IL - Insert lines
                let n = self.csi.arg(0, 1) as usize;
                self.insert_lines(term, n);
            }
            b'M' => {
                // DL - Delete lines
                let n = self.csi.arg(0, 1) as usize;
                self.delete_lines(term, n);
            }
            b'P' => {
                // DCH - Delete characters
                let n = self.csi.arg(0, 1) as usize;
                self.delete_chars(term, n);
            }
            b'S' => {
                // SU - Scroll up
                let n = self.csi.arg(0, 1) as usize;
                for _ in 0..n {
                    self.scroll_up(term);
                }
            }
            b'T' => {
                // SD - Scroll down
                let n = self.csi.arg(0, 1) as usize;
                for _ in 0..n {
                    self.scroll_down(term);
                }
            }
            b'X' => {
                // ECH - Erase characters
                let n = self.csi.arg(0, 1) as usize;
                let end_x = (term.cursor.x + n).min(term.cols - 1);
                self.clear_region(term, term.cursor.x, term.cursor.y, end_x, term.cursor.y);
            }
            b'd' => {
                // VPA - Vertical line position absolute
                let y = self.csi.arg(0, 1) as usize;
                self.move_to(term, term.cursor.x, y.saturating_sub(1));
            }
            b'h' => { // SM - Set mode
                // TODO: mode handling
            }
            b'l' => { // RM - Reset mode
                // TODO: mode handling
            }
            b'm' => {
                // SGR - Select graphic rendition
                self.set_attr(term);
            }
            b'n' => { // DSR - Device status report
                // TODO: respond to status queries
            }
            b'r' => { // DECSTBM - Set scrolling region
                // TODO: scrolling region
            }
            b's' => { // SCOSC - Save cursor position
                // TODO: save cursor
            }
            b'u' => { // SCORC - Restore cursor position
                // TODO: restore cursor
            }
            _ => {
                // Unknown CSI sequence
            }
        }
    }

    fn put_char(&mut self, term: &mut Term, c: char) {
        let idx = term.cursor.y * term.cols + term.cursor.x;
        if idx < term.grid.len() {
            term.grid[idx] = Glyph::new(c, 7, 0);
            term.dirty[term.cursor.y] = true;
        }

        term.cursor.x += 1;
        if term.cursor.x >= term.cols {
            term.cursor.x = 0;
            self.newline(term);
        }
    }

    fn newline(&mut self, term: &mut Term) {
        term.cursor.y += 1;
        if term.cursor.y >= term.rows {
            term.cursor.y = term.rows - 1;
            self.scroll_up(term);
        }
        term.dirty[term.cursor.y] = true;
    }

    fn scroll_up(&mut self, term: &mut Term) {
        for y in 1..term.rows {
            let src_start = y * term.cols;
            let dst_start = (y - 1) * term.cols;
            for x in 0..term.cols {
                term.grid[dst_start + x] = term.grid[src_start + x];
            }
            term.dirty[y - 1] = true;
        }
        let bottom_start = (term.rows - 1) * term.cols;
        for x in 0..term.cols {
            term.grid[bottom_start + x] = Glyph::default();
        }
        term.dirty[term.rows - 1] = true;
    }

    fn scroll_down(&mut self, term: &mut Term) {
        for y in (1..term.rows).rev() {
            let src_start = (y - 1) * term.cols;
            let dst_start = y * term.cols;
            for x in 0..term.cols {
                term.grid[dst_start + x] = term.grid[src_start + x];
            }
            term.dirty[y] = true;
        }
        for x in 0..term.cols {
            term.grid[x] = Glyph::default();
        }
        term.dirty[0] = true;
    }

    fn move_cursor(&mut self, term: &mut Term, dx: isize, dy: isize) {
        let old_y = term.cursor.y;

        let new_x = (term.cursor.x as isize + dx).clamp(0, term.cols as isize - 1) as usize;
        let new_y = (term.cursor.y as isize + dy).clamp(0, term.rows as isize - 1) as usize;

        term.cursor.x = new_x;
        term.cursor.y = new_y;
        term.dirty[old_y] = true;
        term.dirty[new_y] = true;
    }

    fn move_to(&mut self, term: &mut Term, x: usize, y: usize) {
        let old_y = term.cursor.y;
        term.cursor.x = x.min(term.cols - 1);
        term.cursor.y = y.min(term.rows - 1);
        term.dirty[old_y] = true;
        term.dirty[term.cursor.y] = true;
    }

    fn clear_region(&mut self, term: &mut Term, x1: usize, y1: usize, x2: usize, y2: usize) {
        let x1 = x1.min(term.cols - 1);
        let x2 = x2.min(term.cols - 1);
        let y1 = y1.min(term.rows - 1);
        let y2 = y2.min(term.rows - 1);

        for y in y1..=y2 {
            let start_x = if y == y1 { x1 } else { 0 };
            let end_x = if y == y2 { x2 } else { term.cols - 1 };

            for x in start_x..=end_x {
                let idx = y * term.cols + x;
                term.grid[idx] = Glyph::default();
            }
            term.dirty[y] = true;
        }
    }

    fn insert_blank(&mut self, term: &mut Term, n: usize) {
        let y = term.cursor.y;
        let x = term.cursor.x;
        let n = n.min(term.cols - x);

        for i in (x + n..term.cols).rev() {
            let src = y * term.cols + i - n;
            let dst = y * term.cols + i;
            term.grid[dst] = term.grid[src];
        }

        for i in x..x + n {
            term.grid[y * term.cols + i] = Glyph::default();
        }
        term.dirty[y] = true;
    }

    fn delete_chars(&mut self, term: &mut Term, n: usize) {
        let y = term.cursor.y;
        let x = term.cursor.x;
        let n = n.min(term.cols - x);

        for i in x..term.cols - n {
            let src = y * term.cols + i + n;
            let dst = y * term.cols + i;
            term.grid[dst] = term.grid[src];
        }

        for i in (term.cols - n)..term.cols {
            term.grid[y * term.cols + i] = Glyph::default();
        }
        term.dirty[y] = true;
    }

    fn insert_lines(&mut self, term: &mut Term, n: usize) {
        let y = term.cursor.y;
        let n = n.min(term.rows - y);

        for i in ((y + n)..term.rows).rev() {
            let src_start = (i - n) * term.cols;
            let dst_start = i * term.cols;
            for x in 0..term.cols {
                term.grid[dst_start + x] = term.grid[src_start + x];
            }
            term.dirty[i] = true;
        }

        for i in y..y + n {
            for x in 0..term.cols {
                term.grid[i * term.cols + x] = Glyph::default();
            }
            term.dirty[i] = true;
        }
    }

    fn delete_lines(&mut self, term: &mut Term, n: usize) {
        let y = term.cursor.y;
        let n = n.min(term.rows - y);

        // Shift lines up
        for i in y..(term.rows - n) {
            let src_start = (i + n) * term.cols;
            let dst_start = i * term.cols;
            for x in 0..term.cols {
                term.grid[dst_start + x] = term.grid[src_start + x];
            }
            term.dirty[i] = true;
        }

        for i in (term.rows - n)..term.rows {
            for x in 0..term.cols {
                term.grid[i * term.cols + x] = Glyph::default();
            }
            term.dirty[i] = true;
        }
    }

    fn set_attr(&mut self, term: &mut Term) {
        term.dirty[term.cursor.y] = true;
    }
}
