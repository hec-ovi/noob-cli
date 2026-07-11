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
    /// Tool activity lines: the base color for the marker and the brief.
    pub activity: Style,
    /// Per-label accents for activity lines. Each tool/skill kind tints its own
    /// leading word a distinct hue so a scan of the transcript reads at a
    /// glance; muted so they sit with the matrix green, and red stays reserved
    /// for errors (which are never drawn from this palette). Retune freely; no
    /// test keys on a value, only on distinctness and stability.
    pub activity_palette: [Rgb; 10],
    /// Lifecycle notes.
    pub note: Style,
    /// Errors: the one non-green accent.
    pub error: Style,
    /// Dark -> bright green gradient for the banner wordmark and its rule.
    pub ramp: [Rgb; 6],
    /// The thinking scanner's comet: a vivid green head fading to a faded, soft
    /// green tail (index 5 is the head, the tail steps down toward index 0).
    /// Kept distinct from `ramp` so the loader reads green-to-faded-green rather
    /// than dark-to-bright. Retune freely; no test keys on a value.
    pub scanner: [Rgb; 6],
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
            // read, grep, glob, ls, bash, edit, write, task, skill, mcp (in the
            // slot order `label_style` assigns). Cool greens and teals for the
            // read-only lookups, warmer amber/gold for the tools that run or
            // mutate, cooler blue-violets for the delegating ones (task/skill/
            // mcp), so kind reads by temperature before you even parse the word.
            activity_palette: [
                rgb(90, 180, 175),  // read   - soft cyan
                rgb(95, 165, 205),  // grep   - azure
                rgb(80, 180, 150),  // glob   - teal
                rgb(110, 185, 160), // ls     - seafoam
                rgb(205, 165, 90),  // bash   - amber
                rgb(155, 195, 100), // edit   - lime
                rgb(205, 150, 90),  // write  - gold
                rgb(150, 155, 215), // task   - periwinkle
                rgb(190, 135, 195), // skill  - orchid
                rgb(120, 160, 205), // mcp    - steel blue
            ],
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
            // Faded green (tail) -> vivid green (head). The comet head is index
            // 5; each trailing square steps toward index 0, so the sweep reads as
            // green fading to a soft, faded green.
            scanner: [
                rgb(88, 128, 104),
                rgb(94, 148, 112),
                rgb(102, 170, 120),
                rgb(110, 192, 130),
                rgb(118, 214, 140),
                rgb(126, 234, 148),
            ],
        }
    }
}

impl Theme {
    /// The accent for an activity line's leading word. The core tools take a
    /// hand-placed slot (stable, collision-free, so `bash` and `read` never
    /// share a hue); the summary's past-tense wording is folded back onto its
    /// tool so a done line matches its start line. Anything else (a future
    /// tool, an mcp result word) hashes into the palette, so it is still
    /// distinct and stable rather than falling back to one flat color.
    pub fn label_style(&self, label: &str) -> Style {
        let normalized = match label {
            "edited" => "edit",
            "wrote" => "write",
            other => other,
        };
        let idx = match normalized {
            "read" => 0,
            "grep" => 1,
            "glob" => 2,
            "ls" => 3,
            "bash" => 4,
            "edit" => 5,
            "write" => 6,
            "task" => 7,
            "skill" => 8,
            "mcp" | "mcp_call" | "mcp_connect" => 9,
            other => (fnv1a(other) % self.activity_palette.len() as u64) as usize,
        };
        Style::new(self.activity_palette[idx], false)
    }
}

impl Default for Theme {
    fn default() -> Theme {
        Theme::matrix()
    }
}

/// A tiny FNV-1a over the label bytes: a stable palette slot for any leading
/// word we did not hand-place. Pure and allocation-free.
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
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
    fn label_style_is_stable_distinct_and_normalizes_past_tense() {
        let t = Theme::matrix();
        // The contract is not "which green": it is that each core tool reads as
        // its own color, so distinct tools must not collide on one hue.
        let core = ["read", "grep", "glob", "ls", "bash", "edit", "write", "task", "skill", "mcp"];
        for (i, a) in core.iter().enumerate() {
            for b in &core[i + 1..] {
                assert_ne!(
                    t.label_style(a).fg,
                    t.label_style(b).fg,
                    "{a} and {b} share a hue; each tool must be distinguishable"
                );
            }
        }
        // A done line's past-tense wording folds onto its tool, so start and
        // done lines of the same tool carry the same accent.
        assert_eq!(t.label_style("edited").fg, t.label_style("edit").fg);
        assert_eq!(t.label_style("wrote").fg, t.label_style("write").fg);
        // An unplaced word still resolves to a real, stable palette slot.
        let unknown = t.label_style("frobnicate").fg;
        assert_eq!(unknown, t.label_style("frobnicate").fg, "hash slot must be stable");
        assert!(t.activity_palette.contains(&unknown), "hash must land inside the palette");
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
