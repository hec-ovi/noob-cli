//! Zero-dependency SGR (ANSI) styling primitives: a color-depth ladder
//! (truecolor -> 256 -> 16), a pure `Rgb -> escape` mapping, and the handful
//! of attribute constants. Everything here is a pure function of its inputs,
//! so the styled render path is unit-testable without owning a terminal.
//! Display-only: nothing here touches request bodies, the session log, or the
//! wire protocol.

/// Reset every SGR attribute.
pub const RESET: &str = "\x1b[0m";

/// Wrap `text` in a style's SGR sequence at the given depth; plain text when
/// the depth renders no color. The one shared paint primitive for the
/// styled renderers (tables, markdown).
pub(super) fn paint(style: Style, depth: ColorDepth, text: &str) -> String {
    let open = style.sgr(depth);
    if open.is_empty() {
        text.to_string()
    } else {
        format!("{open}{text}{RESET}")
    }
}
/// Faint intensity, used by the legacy dim activity lines (kept byte-identical
/// on the non-REPL surfaces).
pub const DIM: &str = "\x1b[2m";

/// How much color the current terminal can render. Decided once at startup and
/// then only read while emitting. `None` means "emit no color at all".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorDepth {
    Truecolor,
    Ansi256,
    Ansi16,
    None,
}

/// A 24-bit color. Themes are authored in truecolor and down-sampled to the
/// terminal's real depth at emit time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
    Rgb { r, g, b }
}

/// A foreground color plus an optional bold weight: the whole vocabulary a
/// theme token needs.
#[derive(Clone, Copy)]
pub struct Style {
    pub fg: Rgb,
    pub bold: bool,
}

impl Style {
    pub const fn new(fg: Rgb, bold: bool) -> Style {
        Style { fg, bold }
    }

    /// The SGR opener for this style at the given depth, or "" at `None`.
    /// Callers pair it with `RESET`; an empty opener signals the caller to skip
    /// the `RESET` too, so a depthless terminal sees no stray escapes.
    pub fn sgr(&self, depth: ColorDepth) -> String {
        fg_sgr(depth, self.fg, self.bold)
    }
}

/// Foreground SGR opener for a color at a depth. Empty string at `None`.
pub fn fg_sgr(depth: ColorDepth, c: Rgb, bold: bool) -> String {
    let color = match depth {
        ColorDepth::Truecolor => format!("38;2;{};{};{}", c.r, c.g, c.b),
        ColorDepth::Ansi256 => format!("38;5;{}", to_256(c)),
        ColorDepth::Ansi16 => to_16(c).to_string(),
        ColorDepth::None => return String::new(),
    };
    if bold {
        format!("\x1b[1;{color}m")
    } else {
        format!("\x1b[{color}m")
    }
}

/// Wrap `s` in a foreground color and a trailing reset. At `None`, returns `s`
/// unchanged, with no stray reset.
pub fn paint_fg(depth: ColorDepth, c: Rgb, bold: bool, s: &str) -> String {
    let open = fg_sgr(depth, c, bold);
    if open.is_empty() {
        s.to_string()
    } else {
        format!("{open}{s}{RESET}")
    }
}

/// Map a 24-bit color to the xterm 256-color cube (indices 16..=231). The grey
/// ramp is skipped: this palette is chromatic and the 6x6x6 cube reproduces it
/// closely enough.
fn to_256(c: Rgb) -> u8 {
    let q = |v: u8| -> u16 { ((v as u16) * 5 + 127) / 255 }; // round to 0..=5
    (16 + 36 * q(c.r) + 6 * q(c.g) + q(c.b)) as u8
}

/// The 16 base ANSI colors as RGB, paired with their foreground SGR code.
const BASE16: [(Rgb, u8); 16] = [
    (rgb(0, 0, 0), 30),
    (rgb(170, 0, 0), 31),
    (rgb(0, 170, 0), 32),
    (rgb(170, 85, 0), 33),
    (rgb(0, 0, 170), 34),
    (rgb(170, 0, 170), 35),
    (rgb(0, 170, 170), 36),
    (rgb(170, 170, 170), 37),
    (rgb(85, 85, 85), 90),
    (rgb(255, 85, 85), 91),
    (rgb(85, 255, 85), 92),
    (rgb(255, 255, 85), 93),
    (rgb(85, 85, 255), 94),
    (rgb(255, 85, 255), 95),
    (rgb(85, 255, 255), 96),
    (rgb(255, 255, 255), 97),
];

/// Nearest of the 16 base colors by squared euclidean distance; returns its
/// foreground SGR code (30..=37, 90..=97).
fn to_16(c: Rgb) -> u8 {
    let mut best = BASE16[0].1;
    let mut best_d = u32::MAX;
    for (p, code) in BASE16 {
        let dr = c.r as i32 - p.r as i32;
        let dg = c.g as i32 - p.g as i32;
        let db = c.b as i32 - p.b as i32;
        let d = (dr * dr + dg * dg + db * db) as u32;
        if d < best_d {
            best_d = d;
            best = code;
        }
    }
    best
}

/// Depth from the two environment signals terminals actually set. Pure so it is
/// testable; `detect_depth` supplies the live values.
pub fn depth_from_env(colorterm: Option<&str>, term: Option<&str>) -> ColorDepth {
    if let Some(ct) = colorterm {
        let ct = ct.to_ascii_lowercase();
        if ct.contains("truecolor") || ct.contains("24bit") {
            return ColorDepth::Truecolor;
        }
    }
    match term {
        Some("dumb") | Some("") => ColorDepth::None,
        Some(t) if t.contains("256color") => ColorDepth::Ansi256,
        Some(_) => ColorDepth::Ansi16,
        // No TERM at all: assume the base 16 colors are safe.
        None => ColorDepth::Ansi16,
    }
}

pub fn detect_depth() -> ColorDepth {
    depth_from_env(
        std::env::var("COLORTERM").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truecolor_opener_is_exact() {
        assert_eq!(
            fg_sgr(ColorDepth::Truecolor, rgb(1, 2, 3), false),
            "\x1b[38;2;1;2;3m"
        );
        assert_eq!(
            fg_sgr(ColorDepth::Truecolor, rgb(1, 2, 3), true),
            "\x1b[1;38;2;1;2;3m"
        );
    }

    #[test]
    fn none_depth_emits_nothing() {
        assert_eq!(fg_sgr(ColorDepth::None, rgb(9, 9, 9), true), "");
        // paint_fg must not leave a stray reset behind.
        assert_eq!(paint_fg(ColorDepth::None, rgb(9, 9, 9), false, "hi"), "hi");
    }

    #[test]
    fn paint_wraps_and_resets() {
        assert_eq!(
            paint_fg(ColorDepth::Truecolor, rgb(0, 255, 0), false, "x"),
            "\x1b[38;2;0;255;0mx\x1b[0m"
        );
    }

    #[test]
    fn cube_maps_extremes() {
        assert_eq!(to_256(rgb(0, 0, 0)), 16);
        assert_eq!(to_256(rgb(255, 255, 255)), 231);
    }

    #[test]
    fn nearest_16_picks_bright_green() {
        assert_eq!(to_16(rgb(85, 255, 85)), 92);
        assert_eq!(to_16(rgb(0, 0, 0)), 30);
        assert_eq!(to_16(rgb(255, 255, 255)), 97);
    }

    #[test]
    fn depth_ladder_reads_env() {
        assert_eq!(
            depth_from_env(Some("truecolor"), Some("xterm")),
            ColorDepth::Truecolor
        );
        assert_eq!(depth_from_env(Some("24bit"), None), ColorDepth::Truecolor);
        assert_eq!(
            depth_from_env(None, Some("xterm-256color")),
            ColorDepth::Ansi256
        );
        assert_eq!(depth_from_env(None, Some("dumb")), ColorDepth::None);
        assert_eq!(depth_from_env(None, Some("xterm")), ColorDepth::Ansi16);
        assert_eq!(depth_from_env(None, None), ColorDepth::Ansi16);
    }
}
