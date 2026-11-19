//! Colors, text attributes and styles.

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub enum Color {
    /// The terminal's default color.
    #[default]
    Reset,
    /// xterm 256-color palette index (0..16 are the ANSI colors).
    Idx(u8),
    Rgb(u8, u8, u8),
}

/// `0xRRGGBB` → [`Color::Rgb`].
pub const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub mod attr {
    pub const BOLD: u8 = 1;
    pub const DIM: u8 = 2;
    pub const ITALIC: u8 = 4;
    pub const UNDERLINE: u8 = 8;
    pub const REVERSE: u8 = 16;
    pub const STRIKE: u8 = 32;
    pub const HIDDEN: u8 = 64;
    pub const BLINK: u8 = 128;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub attrs: u8,
}

impl Style {
    pub const DEFAULT: Style = Style { fg: Color::Reset, bg: Color::Reset, attrs: 0 };

    pub const fn new(fg: Color, bg: Color) -> Style {
        Style { fg, bg, attrs: 0 }
    }
    pub const fn fg(mut self, c: Color) -> Style {
        self.fg = c;
        self
    }
    pub const fn bg(mut self, c: Color) -> Style {
        self.bg = c;
        self
    }
    pub const fn with(mut self, a: u8) -> Style {
        self.attrs |= a;
        self
    }
    pub const fn without(mut self, a: u8) -> Style {
        self.attrs &= !a;
        self
    }
    pub const fn bold(self) -> Style {
        self.with(attr::BOLD)
    }
    pub const fn italic(self) -> Style {
        self.with(attr::ITALIC)
    }
    pub const fn underline(self) -> Style {
        self.with(attr::UNDERLINE)
    }
    pub const fn has(&self, a: u8) -> bool {
        self.attrs & a != 0
    }
}

/// The 16 ANSI colors as RGB (xterm defaults), used for 256→RGB conversions.
pub const ANSI16: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0xcd, 0x31, 0x31),
    (0x0d, 0xbc, 0x79),
    (0xe5, 0xe5, 0x10),
    (0x24, 0x72, 0xc8),
    (0xbc, 0x3f, 0xbc),
    (0x11, 0xa8, 0xcd),
    (0xe5, 0xe5, 0xe5),
    (0x66, 0x66, 0x66),
    (0xf1, 0x4c, 0x4c),
    (0x23, 0xd1, 0x8b),
    (0xf5, 0xf5, 0x43),
    (0x3b, 0x8e, 0xea),
    (0xd6, 0x70, 0xd6),
    (0x29, 0xb8, 0xdb),
    (0xff, 0xff, 0xff),
];

const CUBE: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// Nearest xterm-256 palette index for an RGB color (cube or gray ramp).
pub fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    fn cube_idx(v: u8) -> usize {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            ((v as usize - 35) / 40).min(5)
        }
    }
    let (ri, gi, bi) = (cube_idx(r), cube_idx(g), cube_idx(b));
    let (cr, cg, cb) = (CUBE[ri], CUBE[gi], CUBE[bi]);
    let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2);
    let cube_d = d(r, cr) + d(g, cg) + d(b, cb);
    let avg = (r as u32 + g as u32 + b as u32) / 3;
    let gray_i = if avg > 238 { 23 } else { (avg.saturating_sub(3) / 10).min(23) };
    let gv = (8 + gray_i * 10) as u8;
    let gray_d = d(r, gv) + d(g, gv) + d(b, gv);
    if gray_d < cube_d { 232 + gray_i as u8 } else { (16 + 36 * ri + 6 * gi + bi) as u8 }
}

/// RGB value of a palette index.
pub fn idx_to_rgb(i: u8) -> (u8, u8, u8) {
    match i {
        0..=15 => ANSI16[i as usize],
        16..=231 => {
            let v = i - 16;
            (CUBE[(v / 36) as usize], CUBE[((v / 6) % 6) as usize], CUBE[(v % 6) as usize])
        }
        _ => {
            let g = 8 + (i - 232) * 10;
            (g, g, g)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conversions() {
        assert_eq!(rgb_to_256(0, 0, 0), 16);
        assert_eq!(rgb_to_256(255, 255, 255), 231);
        assert_eq!(rgb_to_256(0x80, 0x80, 0x80), 244);
        assert_eq!(idx_to_rgb(196), (255, 0, 0));
    }
}
