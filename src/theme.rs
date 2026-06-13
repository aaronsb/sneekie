//! Color themes — the four phosphors the original 1988 monitor could be set to,
//! plus the CGA colorizer that assigns a hue per character class.

/// A color theme. Monochrome themes pick `dim`/`bright` by the attribute byte;
/// the CGA theme ignores them and colorizes per glyph via [`cga_color`].
pub struct Theme {
    pub name: &'static str,
    pub cga: bool,
    pub dim: (u8, u8, u8),
    pub bright: (u8, u8, u8),
}

pub const THEMES: [Theme; 4] = [
    Theme { name: "hercules", cga: false, dim: (46, 168, 46), bright: (125, 255, 125) },
    Theme { name: "amber", cga: false, dim: (191, 121, 0), bright: (255, 196, 56) },
    Theme { name: "white", cga: false, dim: (168, 176, 180), bright: (255, 255, 255) },
    Theme { name: "cga", cga: true, dim: (170, 170, 170), bright: (255, 255, 255) },
];

/// Resolve a theme name to its index, defaulting to CGA.
pub fn theme_index(name: &str) -> usize {
    THEMES.iter().position(|t| t.name == name).unwrap_or(3)
}

/// CGA 16-color palette entries used by the colorized theme.
pub fn cga_rgb(idx: u8) -> (u8, u8, u8) {
    match idx {
        2 => (0, 170, 0),
        3 => (0, 170, 170),
        6 => (170, 85, 0),
        7 => (170, 170, 170),
        10 => (85, 255, 85),
        12 => (255, 85, 85),
        13 => (255, 85, 255),
        14 => (255, 255, 85),
        15 => (255, 255, 255),
        _ => (170, 170, 170),
    }
}

/// What the 1988 game COULD have poked into the attribute bytes: a color per
/// character class, with brightness still following the real attribute byte.
pub fn cga_color(ch: u8, at: u8) -> u8 {
    let hi = at & 8 != 0;
    match ch {
        3 => 12,            // heart: light red
        5 => 10,            // club: light green
        1 => 14,            // smiley: yellow
        2 => 12,            // hunter (Sneekie+): light red — only ever in plus mode
        10 => 6,            // stone: brown
        24 | 26 | 27 => 13, // arrows: light magenta
        219 | 186 | 205 | 187 | 188 | 200 | 201 => {
            if hi { 10 } else { 2 } // snake: light green / green
        }
        179..=218 => 3, // walls: cyan
        _ => {
            if hi { 15 } else { 7 } // text: white / light gray
        }
    }
}
