use crate::core::glyph::Glyph;
use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy)]
    pub struct TermMode: u32 {
        const WRAP      = 1 << 0;
        const INSERT    = 1 << 1;
        const ALTSCREEN = 1 << 2;
        const CRLF      = 1 << 3;
        const ECHO      = 1 << 4;
        const PRINT     = 1 << 5;
        const UTF8      = 1 << 6;
    }
}

bitflags! {
    #[derive(Clone, Copy)]
    pub struct EscapeState: u32 {
        const START      = 1 << 0;
        const CSI        = 1 << 1;
        const STR        = 1 << 2;
        const ALTCHARSET = 1 << 3;
        const STR_END    = 1 << 4;
        const TEST       = 1 << 5;
        const UTF8       = 1 << 6;
    }
}

#[derive(Clone, Copy)]
pub enum CursorState {
    Default,
    WrapNext,
    Origin,
}

#[derive(Clone, Copy)]
pub enum Charset {
    Graphic0,
    Graphic1,
    UK,
    USA,
    Multi,
    Ger,
    Fin,
}

#[derive(Clone, Copy)]
pub struct Cursor {
    pub attr: Glyph,
    pub x: usize,
    pub y: usize,
    pub state: CursorState,
}

impl Default for Cursor {
    fn default() -> Self {
        Self {
            attr: Glyph::default(),
            x: 0,
            y: 0,
            state: CursorState::Default,
        }
    }
}

pub struct Term {
    pub rows: usize,
    pub cols: usize,
    pub grid: Vec<Glyph>,
    pub alt_grid: Vec<Vec<Glyph>>,
    pub dirty: Vec<bool>,
    pub cursor: Cursor,
    pub mode: TermMode,
    pub esc: EscapeState,
    pub charset: Charset,
    pub lastc: char,
}

impl Term {
    pub fn new(cols: usize, rows: usize) -> Self {
        let grid = vec![Glyph::default(); cols * rows];
        let dirty = vec![true; rows];

        Self {
            rows,
            cols,
            grid,
            alt_grid: Vec::new(),
            dirty,
            cursor: Cursor::default(),
            mode: TermMode::WRAP | TermMode::UTF8,
            esc: EscapeState::empty(),
            charset: Charset::USA,
            lastc: '\0',
        }
    }

    #[inline]
    pub fn idx(&self, x: usize, y: usize) -> usize {
        y * self.cols + x
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> &Glyph {
        &self.grid[self.idx(x, y)]
    }

    pub fn put_char(&mut self, c: char) {
        let idx = self.idx(self.cursor.x, self.cursor.y);
        self.grid[idx] = Glyph::new(c, 7, 0); // white on black
        self.dirty[self.cursor.y] = true;
        self.lastc = c;

        self.cursor.x += 1;
        if self.cursor.x >= self.cols {
            self.cursor.x = 0;
            self.cursor.y += 1;
            if self.cursor.y >= self.rows {
                self.cursor.y = self.rows - 1;
                self.scroll_up();
            }
        }
    }

    pub fn backspace(&mut self) {
        if self.cursor.x > 0 {
            self.cursor.x -= 1;
        } else if self.cursor.y > 0 {
            self.cursor.y -= 1;
            self.cursor.x = self.cols - 1;
        }

        let idx = self.idx(self.cursor.x, self.cursor.y);
        self.grid[idx] = Glyph::default();
        self.dirty[self.cursor.y] = true;
    }

    pub fn newline(&mut self) {
        self.cursor.x = 0;
        self.cursor.y += 1;
        if self.cursor.y >= self.rows {
            self.cursor.y = self.rows - 1;
            self.scroll_up();
        }
        self.dirty[self.cursor.y] = true;
    }

    fn scroll_up(&mut self) {
        for y in 1..self.rows {
            let src_start = y * self.cols;
            let dst_start = (y - 1) * self.cols;
            for x in 0..self.cols {
                self.grid[dst_start + x] = self.grid[src_start + x];
            }
            self.dirty[y - 1] = true;
        }

        let bottom_start = (self.rows - 1) * self.cols;
        for x in 0..self.cols {
            self.grid[bottom_start + x] = Glyph::default();
        }
        self.dirty[self.rows - 1] = true;
    }

    pub fn mark_dirty(&mut self) {
        for dirty in self.dirty.iter_mut() {
            *dirty = true;
        }
    }

    pub fn reset(&mut self) {
        for g in self.grid.iter_mut() {
            *g = Glyph::default();
        }
        self.cursor = Cursor::default();
        self.mode = TermMode::WRAP | TermMode::UTF8;
        self.esc = EscapeState::empty();
        self.charset = Charset::USA;
        self.lastc = '\0';
        self.mark_dirty();
    }
}
