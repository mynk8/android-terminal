use vte::{Params, Parser as VteParserInner};

use crate::core::glyph::Glyph;
use crate::core::types::{Cursor, Term, TermMode};

pub struct VteParser {
    parser: VteParserInner,
}

impl VteParser {
    pub fn new() -> Self {
        Self {
            parser: VteParserInner::new(),
        }
    }

    pub fn process(&mut self, term: &mut Term, c: u8) {
        let mut performer = Performer(term);
        self.parser.advance(&mut performer, &[c]);
    }
}

impl Default for VteParser {
    fn default() -> Self {
        Self::new()
    }
}

struct Performer<'a>(&'a mut Term);

impl<'a> vte::Perform for Performer<'a> {
    fn print(&mut self, c: char) {
        let term = &mut *self.0;
        clamp_cursor(term);
        let idx = term.cursor.y * term.cols + term.cursor.x;
        if idx < term.grid.len() {
            let attrs = term.cursor.attr.attrs;
            term.grid[idx] = Glyph::new(c, term.cursor.attr.fg, term.cursor.attr.bg);
            term.grid[idx].attrs = attrs;
            mark_dirty(term);
        }

        if term.cursor.x + 1 >= term.cols {
            term.cursor.x = 0;
            if term.cursor.y + 1 >= term.rows {
                term.cursor.y = term.rows - 1;
                scroll_up(term);
            } else {
                term.cursor.y += 1;
            }
            mark_dirty(term);
        } else {
            term.cursor.x += 1;
        }
    }

    fn execute(&mut self, c: u8) {
        let term = &mut *self.0;
        clamp_cursor(term);
        match c {
            0x00 => {}
            0x07 => {}
            0x08 => {
                if term.cursor.x > 0 {
                    term.cursor.x -= 1;
                } else if term.cursor.y > 0 {
                    term.cursor.y -= 1;
                    term.cursor.x = term.cols - 1;
                }
                mark_dirty(term);
            }
            0x09 => {
                let mut x = term.cursor.x;
                x = (x + 8) & !7;
                if x >= term.cols {
                    x = term.cols - 1;
                }
                term.cursor.x = x;
                mark_dirty(term);
            }
            0x0a | 0x0b | 0x0c => {
                term.cursor.y += 1;
                if term.cursor.y >= term.rows {
                    term.cursor.y = term.rows - 1;
                    scroll_up(term);
                }
                mark_dirty(term);
            }
            0x0d => {
                term.cursor.x = 0;
                mark_dirty(term);
            }
            0x84 => {
                term.cursor.y += 1;
                if term.cursor.y >= term.rows {
                    term.cursor.y = term.rows - 1;
                    scroll_up(term);
                }
                mark_dirty(term);
            }
            0x85 => {
                term.cursor.x = 0;
                term.cursor.y += 1;
                if term.cursor.y >= term.rows {
                    term.cursor.y = term.rows - 1;
                    scroll_up(term);
                }
                mark_dirty(term);
            }
            0x88 => {}
            0x8d => {
                if term.cursor.y == 0 {
                    scroll_down(term);
                } else {
                    term.cursor.y -= 1;
                    mark_dirty(term);
                }
            }
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, _intermediates: &[u8], _ignore: bool, c: char) {
        let term = &mut *self.0;
        clamp_cursor(term);

        macro_rules! get_param {
            ($i:expr, $default:expr) => {
                params
                    .iter()
                    .nth($i)
                    .and_then(|p| p.first().copied())
                    .unwrap_or($default) as usize
            };
        }

        match c as u8 {
            b'@' => {
                let n = get_param!(0, 1);
                insert_blank(term, n);
            }
            b'A' => {
                let n = get_param!(0, 1);
                term.cursor.y = term.cursor.y.saturating_sub(n);
                mark_dirty(term);
            }
            b'B' | b'e' => {
                let n = get_param!(0, 1);
                term.cursor.y = (term.cursor.y + n).min(term.rows - 1);
                mark_dirty(term);
            }
            b'C' | b'a' => {
                let n = get_param!(0, 1);
                term.cursor.x = (term.cursor.x + n).min(term.cols - 1);
                mark_dirty(term);
            }
            b'D' => {
                let n = get_param!(0, 1);
                term.cursor.x = term.cursor.x.saturating_sub(n);
                mark_dirty(term);
            }
            b'E' => {
                let n = get_param!(0, 1);
                term.cursor.x = 0;
                term.cursor.y = (term.cursor.y + n).min(term.rows - 1);
                mark_dirty(term);
            }
            b'F' => {
                let n = get_param!(0, 1);
                term.cursor.x = 0;
                term.cursor.y = term.cursor.y.saturating_sub(n);
                mark_dirty(term);
            }
            b'G' | b'`' => {
                let x = get_param!(0, 1).saturating_sub(1);
                term.cursor.x = x.min(term.cols - 1);
                mark_dirty(term);
            }
            b'H' | b'f' => {
                let y = get_param!(0, 1).saturating_sub(1);
                let x = get_param!(1, 1).saturating_sub(1);
                term.cursor.x = x.min(term.cols - 1);
                term.cursor.y = y.min(term.rows - 1);
                mark_dirty(term);
            }
            b'J' => {
                let mode = get_param!(0, 0);
                match mode {
                    0 => clear_region(
                        term,
                        term.cursor.x,
                        term.cursor.y,
                        term.cols - 1,
                        term.rows - 1,
                    ),
                    1 => clear_region(term, 0, 0, term.cursor.x, term.cursor.y),
                    2 | 3 => clear_region(term, 0, 0, term.cols - 1, term.rows - 1),
                    _ => {}
                }
            }
            b'K' => {
                let mode = get_param!(0, 0);
                match mode {
                    0 => clear_region(
                        term,
                        term.cursor.x,
                        term.cursor.y,
                        term.cols - 1,
                        term.cursor.y,
                    ),
                    1 => clear_region(term, 0, term.cursor.y, term.cursor.x, term.cursor.y),
                    2 => clear_region(term, 0, term.cursor.y, term.cols - 1, term.cursor.y),
                    _ => {}
                }
            }
            b'L' => {
                let n = get_param!(0, 1);
                insert_lines(term, n);
            }
            b'M' => {
                let n = get_param!(0, 1);
                delete_lines(term, n);
            }
            b'P' => {
                let n = get_param!(0, 1);
                delete_chars(term, n);
            }
            b'S' => {
                let n = get_param!(0, 1);
                for _ in 0..n {
                    scroll_up(term);
                }
            }
            b'T' => {
                let n = get_param!(0, 1);
                for _ in 0..n {
                    scroll_down(term);
                }
            }
            b'X' => {
                let n = get_param!(0, 1);
                let end_x = (term.cursor.x + n).min(term.cols - 1);
                clear_region(term, term.cursor.x, term.cursor.y, end_x, term.cursor.y);
            }
            b'd' => {
                let y = get_param!(0, 1).saturating_sub(1);
                term.cursor.y = y.min(term.rows - 1);
                mark_dirty(term);
            }
            b'h' => {
                set_mode(term, params, true);
            }
            b'l' => {
                set_mode(term, params, false);
            }
            b'm' => {
                sgr(term, params);
            }
            b'r' => {
                term.cursor.x = 0;
                term.cursor.y = 0;
                term.dirty.iter_mut().for_each(|d| *d = true);
            }
            b's' => {}
            b'u' => {}
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, c: u8) {
        let term = &mut *self.0;
        clamp_cursor(term);
        match c {
            b'D' => {
                term.cursor.y += 1;
                if term.cursor.y >= term.rows {
                    term.cursor.y = term.rows - 1;
                    scroll_up(term);
                }
                mark_dirty(term);
            }
            b'E' => {
                term.cursor.x = 0;
                term.cursor.y += 1;
                if term.cursor.y >= term.rows {
                    term.cursor.y = term.rows - 1;
                    scroll_up(term);
                }
                mark_dirty(term);
            }
            b'H' => {}
            b'M' => {
                if term.cursor.y == 0 {
                    scroll_down(term);
                } else {
                    term.cursor.y -= 1;
                    mark_dirty(term);
                }
            }
            b'7' => {}
            b'8' => {}
            b'c' => {
                term.reset();
            }
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _ignore: bool) {}
}

fn scroll_up(term: &mut Term) {
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

fn scroll_down(term: &mut Term) {
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

fn clear_region(term: &mut Term, x1: usize, y1: usize, x2: usize, y2: usize) {
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

fn insert_blank(term: &mut Term, n: usize) {
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

fn delete_chars(term: &mut Term, n: usize) {
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

fn insert_lines(term: &mut Term, n: usize) {
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

fn delete_lines(term: &mut Term, n: usize) {
    let y = term.cursor.y;
    let n = n.min(term.rows - y);

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

fn set_mode(term: &mut Term, params: &Params, set: bool) {
    for param in params.iter() {
        let val = param.first().copied().unwrap_or(0) as usize;
        match val {
            7 => {
                if set {
                    term.mode.insert(TermMode::WRAP);
                } else {
                    term.mode.remove(TermMode::WRAP);
                }
            }
            1049 => {
                if set {
                    term.mode.insert(TermMode::ALTSCREEN);
                } else {
                    term.mode.remove(TermMode::ALTSCREEN);
                }
            }
            _ => {}
        }
    }
}

fn sgr(term: &mut Term, params: &Params) {
    clamp_cursor(term);
    let mut iter = params.iter().peekable();

    while let Some(param) = iter.next() {
        let val = param.first().copied().unwrap_or(0) as u32;

        match val {
            0 => {
                term.cursor.attr = Cursor::default().attr;
            }
            1 => {
                term.cursor.attr.attrs |= 1 << 0;
            }
            2 => {
                term.cursor.attr.attrs |= 1 << 1;
            }
            3 => {
                term.cursor.attr.attrs |= 1 << 2;
            }
            4 => {
                term.cursor.attr.attrs |= 1 << 3;
            }
            5 | 6 => {
                term.cursor.attr.attrs |= 1 << 4;
            }
            7 => {
                term.cursor.attr.attrs |= 1 << 5;
            }
            8 => {
                term.cursor.attr.attrs |= 1 << 6;
            }
            9 => {
                term.cursor.attr.attrs |= 1 << 7;
            }
            22 => {
                term.cursor.attr.attrs &= !(1 << 0 | 1 << 1);
            }
            23 => {
                term.cursor.attr.attrs &= !(1 << 2);
            }
            24 => {
                term.cursor.attr.attrs &= !(1 << 3);
            }
            25 => {
                term.cursor.attr.attrs &= !(1 << 4);
            }
            27 => {
                term.cursor.attr.attrs &= !(1 << 5);
            }
            28 => {
                term.cursor.attr.attrs &= !(1 << 6);
            }
            29 => {
                term.cursor.attr.attrs &= !(1 << 7);
            }
            30..=37 => {
                term.cursor.attr.fg = (val - 30) as u8;
            }
            38 => {
                if let Some(next_param) = iter.next() {
                    let next_val = next_param.first().copied().unwrap_or(0) as u32;
                    if next_val == 5 {
                        if let Some(color_param) = iter.next() {
                            term.cursor.attr.fg = color_param.first().copied().unwrap_or(0) as u8;
                        }
                    } else if next_val == 2 {
                        let r = iter.next().and_then(|p| p.first().copied()).unwrap_or(0) as u8;
                        let g = iter.next().and_then(|p| p.first().copied()).unwrap_or(0) as u8;
                        let b = iter.next().and_then(|p| p.first().copied()).unwrap_or(0) as u8;
                        term.cursor.attr.fg = rgb_to_ansi256(r, g, b);
                    }
                }
            }
            39 => {
                term.cursor.attr.fg = 7;
            }
            40..=47 => {
                term.cursor.attr.bg = (val - 40) as u8;
            }
            48 => {
                if let Some(next_param) = iter.next() {
                    let next_val = next_param.first().copied().unwrap_or(0) as u32;
                    if next_val == 5 {
                        if let Some(color_param) = iter.next() {
                            term.cursor.attr.bg = color_param.first().copied().unwrap_or(0) as u8;
                        }
                    } else if next_val == 2 {
                        let r = iter.next().and_then(|p| p.first().copied()).unwrap_or(0) as u8;
                        let g = iter.next().and_then(|p| p.first().copied()).unwrap_or(0) as u8;
                        let b = iter.next().and_then(|p| p.first().copied()).unwrap_or(0) as u8;
                        term.cursor.attr.bg = rgb_to_ansi256(r, g, b);
                    }
                }
            }
            49 => {
                term.cursor.attr.bg = 0;
            }
            90..=97 => {
                term.cursor.attr.fg = (val - 90 + 8) as u8;
            }
            100..=107 => {
                term.cursor.attr.bg = (val - 100 + 8) as u8;
            }
            _ => {}
        }
    }
    mark_dirty(term);
}

pub type Parser = VteParser;

fn clamp_cursor(term: &mut Term) {
    if term.rows == 0 || term.cols == 0 {
        term.cursor.x = 0;
        term.cursor.y = 0;
        return;
    }

    term.cursor.x = term.cursor.x.min(term.cols - 1);
    term.cursor.y = term.cursor.y.min(term.rows - 1);
}

fn mark_dirty(term: &mut Term) {
    if term.dirty.is_empty() {
        return;
    }

    let row = term
        .cursor
        .y
        .min(term.rows.saturating_sub(1))
        .min(term.dirty.len() - 1);
    term.dirty[row] = true;
}

fn rgb_to_ansi256(r: u8, g: u8, b: u8) -> u8 {
    if r == g && g == b {
        if r < 8 {
            return 16;
        }
        if r > 248 {
            return 231;
        }
        return 232 + ((r as u16 - 8) / 10) as u8;
    }

    let r6 = ((r as u16 * 5 + 127) / 255) as u8;
    let g6 = ((g as u16 * 5 + 127) / 255) as u8;
    let b6 = ((b as u16 * 5 + 127) / 255) as u8;
    16 + 36 * r6 + 6 * g6 + b6
}
