use skia_safe::{Canvas, Color, Data, Font, FontMgr, Paint, Point, Rect};

use crate::core::glyph::{color_from_index, GlyphAttrs};
use crate::core::types::Term;

const FONT_DATA: &[u8] = include_bytes!("../../assets/font.ttf");

pub struct Renderer {
    pub font: Font,
    pub painter: Paint,
    pub cell_w: f32,
    pub cell_h: f32,
    pub descent: f32,
    palette: [u32; 16],
}

impl Renderer {
    pub fn new(font_size: f32, palette: [u32; 16]) -> Self {
        let font_mgr = FontMgr::new();

        let font_data = Data::new_copy(FONT_DATA);
        let typeface = font_mgr.new_from_data(&font_data, None).unwrap_or_else(|| {
            log::warn!("Failed to load embedded font, using system fallback");
            font_mgr
                .match_family_style("monospace", skia_safe::FontStyle::default())
                .or_else(|| font_mgr.match_family_style("", skia_safe::FontStyle::default()))
                .expect("No fonts available")
        });

        let font = Font::from_typeface(typeface, font_size);
        let (_, metrics) = font.metrics();
        let cell_w = font.measure_str("M", None).1.width().max(16.0);
        let cell_h = (metrics.descent - metrics.ascent + metrics.leading).max(20.0);
        let descent = metrics.descent;

        log::info!("Font loaded: cell={}x{}", cell_w, cell_h);

        Self {
            font,
            painter: Paint::default(),
            cell_w,
            cell_h,
            descent,
            palette,
        }
    }

    #[inline]
    fn draw_char(&self, canvas: &Canvas, c: char, x: f32, y: f32, paint: &Paint) {
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        canvas.draw_str(s, Point::new(x, y), &self.font, paint);
    }

    pub fn draw_cells(&mut self, term: &Term, canvas: &Canvas) {
        for y in 0..term.rows {
            let base_y = y as f32 * self.cell_h;
            let text_y = (y + 1) as f32 * self.cell_h - self.descent;

            for x in 0..term.cols {
                let g = term.get(x, y);
                let base_x = x as f32 * self.cell_w;
                let attrs = GlyphAttrs::from_bits_truncate(g.attrs);
                let (mut fg_idx, mut bg_idx) = (g.fg, g.bg);

                if attrs.contains(GlyphAttrs::REVERSE) {
                    (fg_idx, bg_idx) = (bg_idx, fg_idx);
                }
                if attrs.contains(GlyphAttrs::BOLD) && fg_idx < 8 {
                    fg_idx += 8;
                }
                if attrs.contains(GlyphAttrs::INVISIBLE) {
                    fg_idx = bg_idx;
                }

                self.painter
                    .set_color(color_from_index(&self.palette, bg_idx));
                let rect = Rect::from_xywh(base_x, base_y, self.cell_w, self.cell_h);
                canvas.draw_rect(rect, &self.painter);

                let c = g.char();
                if c != ' ' {
                    self.painter
                        .set_color(color_from_index(&self.palette, fg_idx));
                    self.draw_char(canvas, c, base_x, text_y, &self.painter);
                }
            }
        }
    }

    pub fn draw_cursor(&mut self, term: &Term, canvas: &Canvas) {
        let x = term.cursor.x as f32 * self.cell_w;
        let y = term.cursor.y as f32 * self.cell_h;

        self.painter.set_color(Color::WHITE);
        let rect = Rect::from_xywh(x, y, self.cell_w, self.cell_h);
        canvas.draw_rect(rect, &self.painter);

        let g = term.get(term.cursor.x, term.cursor.y);
        let c = g.char();
        if c != ' ' {
            self.painter.set_color(Color::BLACK);
            let text_y = (term.cursor.y + 1) as f32 * self.cell_h - self.descent;
            self.draw_char(canvas, c, x, text_y, &self.painter);
        }
    }

    pub fn render(&mut self, canvas: &Canvas, term: &Term, cursor_visible: bool) {
        canvas.clear(color_from_index(&self.palette, 0));
        self.draw_cells(term, canvas);
        if cursor_visible {
            self.draw_cursor(term, canvas);
        }
    }
}
