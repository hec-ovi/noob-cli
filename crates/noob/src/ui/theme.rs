//! The one swappable look. A `Theme` is just named style tokens plus the
//! wordmark and the green ramp the banner (and, later, the scanner) step
//! through. Ship `matrix` only; a future `NOOB_THEME` will select others
//! without touching any call site. Display-only, like everything under `ui/`.

use super::style::{ColorDepth, Rgb, Style, paint_fg, rgb};

#[derive(Clone, Copy)]
pub struct Theme {
    pub wordmark: &'static str,
    /// The input prompt marker (`> ` / `plan> `).
    pub prompt: Style,
    /// Streamed assistant text: a light tint so a human can tell the model's
    /// words from their own echoed input. Tuned for a dark terminal.
    pub assistant: Style,
    /// Tool activity lines.
    pub activity: Style,
    /// Lifecycle notes.
    pub note: Style,
    /// Errors: the one non-green accent.
    pub error: Style,
    /// Dark -> bright green gradient for the banner wordmark and its rule.
    pub ramp: [Rgb; 6],
}

impl Theme {
    /// The default matrix-green theme.
    pub const fn matrix() -> Theme {
        Theme {
            wordmark: "No0B-CL1",
            // A muted mid-green band: nothing so dark it vanishes on black,
            // nothing so bright it shouts. Retune these freely; no test keys
            // on a color value.
            prompt: Style::new(rgb(90, 185, 120), true),
            assistant: Style::new(rgb(130, 175, 145), false),
            activity: Style::new(rgb(85, 145, 105), false),
            note: Style::new(rgb(115, 145, 128), false),
            error: Style::new(rgb(205, 95, 90), true),
            ramp: [
                rgb(60, 125, 85),
                rgb(78, 145, 100),
                rgb(96, 165, 118),
                rgb(116, 185, 135),
                rgb(138, 202, 152),
                rgb(160, 215, 175),
            ],
        }
    }
}

impl Default for Theme {
    fn default() -> Theme {
        Theme::matrix()
    }
}

/// The startup banner: the wordmark with each glyph stepped through the green
/// ramp, over a short ramped rule. Returned as one string; the caller places
/// it. At a depthless terminal it degrades to plain, readable text.
pub fn banner(theme: &Theme, depth: ColorDepth) -> String {
    let glyphs: Vec<char> = theme.wordmark.chars().collect();
    let mut s = String::from("\n  ");
    for (i, ch) in glyphs.iter().enumerate() {
        // Spread the ramp across the whole wordmark rather than repeating it.
        let idx = if glyphs.len() > 1 {
            i * (theme.ramp.len() - 1) / (glyphs.len() - 1)
        } else {
            0
        };
        s.push_str(&paint_fg(depth, theme.ramp[idx], true, &ch.to_string()));
    }
    s.push_str("\n  ");
    for c in theme.ramp {
        s.push_str(&paint_fg(depth, c, false, "──"));
    }
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_banner_shows_the_wordmark() {
        // At None depth the glyphs are unwrapped, so the wordmark is contiguous.
        let b = banner(&Theme::matrix(), ColorDepth::None);
        assert!(b.contains("No0B-CL1"), "plain banner missing wordmark: {b:?}");
    }

    #[test]
    fn styled_banner_resets_and_ends_clean() {
        // The concern is bleed, not the specific colors: the banner must reset
        // its escapes and end on a newline so nothing after it inherits color.
        let b = banner(&Theme::matrix(), ColorDepth::Truecolor);
        assert!(b.contains("\x1b[0m"), "styled banner never resets");
        assert!(b.ends_with('\n'), "banner must end on a newline");
    }
}
