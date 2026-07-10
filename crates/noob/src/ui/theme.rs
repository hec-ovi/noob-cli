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
            prompt: Style::new(rgb(46, 224, 96), true),
            assistant: Style::new(rgb(150, 225, 170), false),
            activity: Style::new(rgb(0, 160, 70), false),
            note: Style::new(rgb(96, 140, 104), false),
            error: Style::new(rgb(224, 72, 72), true),
            ramp: [
                rgb(0, 70, 20),
                rgb(0, 110, 35),
                rgb(0, 150, 55),
                rgb(20, 190, 80),
                rgb(70, 224, 120),
                rgb(150, 245, 175),
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
