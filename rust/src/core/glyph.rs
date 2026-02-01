use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct GlyphAttrs: u8 {
        const BOLD = 1 << 0;
        const FAINT = 1 << 1;
        const ITALIC = 1 << 2;
        const UNDERLINE = 1 << 3;
        const BLINK = 1 << 4;
        const REVERSE = 1 << 5;
        const INVISIBLE = 1 << 6;
        const STRUCK = 1 << 7;
    }
}

/// Layout: [rune: 4 bytes][fg: 1 byte][bg: 1 byte][attrs: 1 byte][pad: 1 byte]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Glyph {
    pub rune: u32, // char as u32 (4 bytes)
    pub fg: u8,    // foreground color index (1 byte)
    pub bg: u8,    // background color index (1 byte)
    pub attrs: u8, // GlyphAttrs bits (1 byte)
    _pad: u8,      // alignment padding (1 byte)
}

impl Glyph {
    #[inline]
    pub fn new(c: char, fg: u8, bg: u8) -> Self {
        Self {
            rune: c as u32,
            fg,
            bg,
            attrs: 0,
            _pad: 0,
        }
    }

    #[inline]
    pub fn char(&self) -> char {
        char::from_u32(self.rune).unwrap_or(' ')
    }
}

impl Default for Glyph {
    fn default() -> Self {
        Self {
            rune: ' ' as u32,
            fg: 7, // white
            bg: 0, // black
            attrs: 0,
            _pad: 0,
        }
    }
}

/// Base16 color palette
pub const COLORS: [u32; 16] = [
    0x1e1e1e, // 0: black (bg)
    0xf44747, // 1: red
    0x608b4e, // 2: green
    0xdcdcaa, // 3: yellow
    0x569cd6, // 4: blue
    0xc586c0, // 5: magenta
    0x4ec9b0, // 6: cyan
    0xd4d4d4, // 7: white (fg)
    0x808080, // 8: bright black
    0xf44747, // 9: bright red
    0x608b4e, // 10: bright green
    0xdcdcaa, // 11: bright yellow
    0x569cd6, // 12: bright blue
    0xc586c0, // 13: bright magenta
    0x4ec9b0, // 14: bright cyan
    0xffffff, // 15: bright white
];

#[inline]
pub fn color_from_index(idx: u8) -> skia_safe::Color {
    let rgb = COLORS[(idx & 0x0F) as usize];
    skia_safe::Color::from_rgb(
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >> 8) & 0xFF) as u8,
        (rgb & 0xFF) as u8,
    )
}
