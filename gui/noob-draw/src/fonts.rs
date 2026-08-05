//! Which faces the window draws from, and which face a character comes from.
//!
//! The window is one colour scheme over a character grid, and both of those are
//! decisions about fonts. A character drawn from whatever face a machine
//! happens to have is drawn differently on the next machine: the same row of
//! emoji came out as outlines from DejaVu here and as full-colour bitmaps from
//! Noto Color Emoji there, in the same line of text. So the pool is built
//! rather than inherited.

use glyphon::cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping};
use std::collections::HashMap;

/// The family name of the embedded symbol font.
///
/// Symbols Nerd Font Mono, shipped in the binary rather than looked for on the
/// system. It carries the Codicon, Seti and Devicon sets, which is what a
/// window button and a file-type mark need.
pub const ICON_FAMILY: &str = "Symbols Nerd Font Mono";

/// The family name of the embedded emoji font.
pub const EMOJI_FAMILY: &str = "Noto Emoji";

const ICON_FONT: &[u8] = include_bytes!("../fonts/SymbolsNerdFontMono-Regular.ttf");

/// Noto Emoji, the monochrome one, under the SIL Open Font License (the text is
/// alongside it). Monochrome is the point: its glyphs take the colour of the
/// text around them like every other glyph, so a transcript stays one palette.
/// Every glyph in it is the same width, which is what lets an emoji be drawn as
/// the two columns the grid counts it as.
const EMOJI_FONT: &[u8] = include_bytes!("../fonts/NotoEmoji[wght].ttf");

/// How every buffer in this crate is shaped.
///
/// `Advanced` is the only one of the two strategies that looks in the pool for a
/// character the text face lacks. The cheap one resolves such a character to
/// `.notdef` and draws it as a blank box, which is what an emoji, an accent or a
/// CJK character in the transcript came out as.
///
/// It is also the faster of the two here, measured, because `shape-run-cache` is
/// on: a run that shaped on an earlier frame is looked up instead of shaped
/// again, and the buffers are rebuilt every frame.
pub const SHAPING: Shaping = Shaping::Advanced;

/// How a question about one face is asked.
///
/// The cheap strategy is the right one here for the reason it is the wrong one
/// for drawing: it never leaves the face it was given. Asking with fallback on
/// answers "some font in the pool has this", which is true of almost every
/// character and tells a caller nothing about the face it named.
pub const COVERAGE: Shaping = Shaping::Basic;

/// The system's fonts, plus the two embedded ones, minus every face that draws
/// in its own colours.
///
/// Dropping the colour faces is what makes one style: a colour font paints its
/// own bitmap or its own palette and ignores the colour the text asks for, so
/// one emoji from Noto Color Emoji sitting in a line of monochrome ones is a
/// yellow blot in a cyan transcript. With them out of the pool, every glyph the
/// window draws is a mask it tints, and the embedded emoji font is what covers
/// the characters they used to.
///
/// The rule is read off the font, not off a list of names: a face carrying
/// `CBDT`, `sbix` or `COLR` is a colour face whatever it is called.
pub fn pool() -> FontSystem {
    let mut system = FontSystem::new();
    let db = system.db_mut();
    db.load_font_data(ICON_FONT.to_vec());
    db.load_font_data(EMOJI_FONT.to_vec());
    let coloured: Vec<_> = db
        .faces()
        .filter(|face| db.with_face_data(face.id, draws_its_own_colours) == Some(true))
        .map(|face| face.id)
        .collect();
    for id in coloured {
        db.remove_face(id);
    }
    system
}

/// Whether a face carries colour glyphs, read from its table directory.
///
/// `CBDT` is a colour bitmap strike (Noto Color Emoji), `sbix` is Apple's, and
/// `COLR` is layered vector colour. A face with none of them has only outlines,
/// which take the colour they are drawn in.
fn draws_its_own_colours(data: &[u8], index: u32) -> bool {
    let be32 = |at: usize| -> Option<u32> {
        data.get(at..at + 4)
            .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    };
    // A collection holds one table directory per face, named in its header.
    let top = if be32(0) == Some(u32::from_be_bytes(*b"ttcf")) {
        be32(12 + 4 * index as usize).unwrap_or(0) as usize
    } else {
        0
    };
    let count = data
        .get(top + 4..top + 6)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .unwrap_or(0) as usize;
    (0..count).any(|i| {
        let at = top + 12 + 16 * i;
        matches!(data.get(at..at + 4), Some(b"CBDT" | b"sbix" | b"COLR"))
    })
}

/// Which characters the embedded emoji font covers, remembered.
///
/// The answer costs a shaping and never changes, so it is worked out once per
/// character and kept. It is also the whole rule for what an emoji is here:
/// nothing in this crate enumerates characters, it asks the font. That matters
/// because the font's coverage is exactly right, it has the pictographs and it
/// does not have the text symbols (a check mark and an arrow stay in the text
/// face, at one column, where they belong).
#[derive(Default)]
pub struct Emoji {
    covered: HashMap<char, bool>,
    em: Option<f32>,
}

impl Emoji {
    /// The size to draw an emoji span at so it fills exactly `columns` columns.
    ///
    /// The font's glyphs are all one width, and that width is not two columns of
    /// the text face: left alone, an emoji draws about a quarter of a column
    /// wider than the grid counts it as, and everything after it on the row
    /// slides. Scaling the span is what puts it back on the grid, and because
    /// every glyph in the font is the same width one measurement covers them
    /// all.
    pub fn size_for(&mut self, fonts: &mut FontSystem, columns: f32) -> f32 {
        let em = match self.em {
            Some(em) => em,
            None => {
                // At a large size, so the ratio is not rounded by hinting.
                const AT: f32 = 512.0;
                let mut buffer = Buffer::new(fonts, Metrics::new(AT, AT));
                buffer.set_size(Some(4096.0), Some(AT * 2.0));
                buffer.set_text(
                    "\u{1f600}",
                    &Attrs::new().family(Family::Name(EMOJI_FAMILY)),
                    COVERAGE,
                    None,
                );
                buffer.shape_until_scroll(fonts, false);
                let em = buffer
                    .layout_runs()
                    .next()
                    .map(|run| run.line_w / AT)
                    .filter(|w| *w > 0.0)
                    .unwrap_or(1.0);
                self.em = Some(em);
                em
            }
        };
        columns / em
    }

    /// Whether this character is drawn from the embedded emoji font.
    pub fn covers(&mut self, fonts: &mut FontSystem, ch: char) -> bool {
        // ASCII is in every text face and in no emoji font; skipping it keeps
        // the common line out of the map entirely.
        if ch.is_ascii() {
            return false;
        }
        if let Some(known) = self.covered.get(&ch) {
            return *known;
        }
        let mut buffer = Buffer::new(fonts, Metrics::new(16.0, 20.0));
        buffer.set_size(Some(64.0), Some(32.0));
        buffer.set_text(
            &ch.to_string(),
            &Attrs::new().family(Family::Name(EMOJI_FAMILY)),
            COVERAGE,
            None,
        );
        buffer.shape_until_scroll(fonts, false);
        let found = buffer
            .layout_runs()
            .flat_map(|run| run.glyphs.iter())
            .all(|glyph| glyph.glyph_id != 0);
        self.covered.insert(ch, found);
        found
    }

    /// One string split into the longest stretches that come from one face:
    /// `true` for the emoji font, `false` for the text face.
    ///
    /// Empty in, nothing out. A run of ordinary text comes back as one span, so
    /// the common case costs one scan and no allocation beyond the one span.
    pub fn spans<'t>(&mut self, fonts: &mut FontSystem, text: &'t str) -> Vec<(&'t str, bool)> {
        let mut spans: Vec<(&str, bool)> = Vec::new();
        let mut start = 0;
        let mut current: Option<bool> = None;
        let mut walk = text.char_indices().peekable();
        while let Some((at, ch)) = walk.next() {
            let next = walk.peek().map(|(_, next)| *next);
            // The routing rule is the grid's own: a character goes to the
            // emoji font exactly when the grid counts it two columns (or the
            // selector promotes it to two), so what is drawn two cells wide is
            // what was counted two cells wide. A one-column symbol the emoji
            // font happens to cover (a bare spade, a check mark) stays in the
            // text face at one column, which is what it is counted as. The
            // selector rides with whatever its symbol was routed to.
            let emoji = match ch {
                text_geometry::VS16 => current.unwrap_or(false),
                _ => {
                    self.covers(fonts, ch)
                        && (text_geometry::width_of(ch) == 2
                            || text_geometry::promoted(ch, next))
                }
            };
            match current {
                Some(same) if same == emoji => {}
                Some(other) => {
                    spans.push((&text[start..at], other));
                    start = at;
                }
                None => {}
            }
            current = Some(emoji);
        }
        if let Some(last) = current {
            spans.push((&text[start..], last));
        }
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One style means no face may paint itself. A colour font ignores the
    /// colour the text asks for, so one emoji out of it lands in a monochrome
    /// transcript as a coloured blot, which is what the window was doing.
    #[test]
    fn no_face_in_the_pool_draws_its_own_colours() {
        let mut system = pool();
        let db = system.db_mut();
        let painted: Vec<&str> = db
            .faces()
            .filter(|face| db.with_face_data(face.id, draws_its_own_colours) == Some(true))
            .map(|face| face.post_script_name.as_str())
            .collect();
        assert!(painted.is_empty(), "colour faces left in the pool: {painted:?}");
    }

    /// And the font that replaces them is in it, or every emoji is a blank box.
    #[test]
    fn the_emoji_font_we_ship_is_in_the_pool() {
        let mut system = pool();
        assert!(
            system
                .db_mut()
                .faces()
                .any(|face| face.families.iter().any(|(name, _)| name == EMOJI_FAMILY)),
            "the embedded emoji font is not in the pool under {EMOJI_FAMILY:?}"
        );
    }

    /// The routing rule is the font's own coverage, so this is the whole of it:
    /// pictographs come from the emoji font, and the symbols that have always
    /// been one column wide stay in the text face where they are one column
    /// wide. Nothing here is a list the code consults; the assertions are what
    /// the font answered.
    #[test]
    fn pictographs_route_to_the_emoji_font_and_text_symbols_do_not() {
        let mut system = pool();
        let mut emoji = Emoji::default();
        for ch in ['\u{2705}', '\u{274c}', '\u{1f600}', '\u{1f923}', '\u{1f604}'] {
            assert!(
                emoji.covers(&mut system, ch),
                "U+{:04X} must come from the font we ship, or it is drawn in whatever \
                 style the machine has",
                ch as u32
            );
        }
        for ch in ['a', '\u{2713}', '\u{2192}', '\u{e0a0}'] {
            assert!(
                !emoji.covers(&mut system, ch),
                "U+{:04X} belongs to the text face",
                ch as u32
            );
        }
    }

    /// The point of the whole exercise: an emoji drawn as exactly the two
    /// columns the grid counts it as. Left at its own advance it is about a
    /// quarter of a column wider, and eight of them on a row put the text after
    /// them two columns right of where the selection band is.
    #[test]
    fn an_emoji_is_drawn_as_exactly_two_columns() {
        let mut system = pool();
        let mut emoji = Emoji::default();
        let column = 8.4;
        let size = emoji.size_for(&mut system, 2.0 * column);

        let mut buffer = Buffer::new(&mut system, Metrics::new(size, size * 1.42));
        buffer.set_size(Some(4096.0), Some(size * 2.0));
        buffer.set_text(
            "\u{2705}\u{1f604}\u{274c}",
            &Attrs::new().family(Family::Name(EMOJI_FAMILY)),
            COVERAGE,
            None,
        );
        buffer.shape_until_scroll(&mut system, false);
        let drawn = buffer.layout_runs().next().map_or(0.0, |run| run.line_w);
        let want = 3.0 * 2.0 * column;
        assert!(
            (drawn - want).abs() < 0.5,
            "three emoji drew {drawn:.2} wide, the grid gives them {want:.2}"
        );
    }

    /// The routing rule is the grid's: two-column characters go to the emoji
    /// font, one-column symbols stay in the text face even when the emoji font
    /// covers them, and the selector promotes its symbol, riding with it.
    #[test]
    fn routing_follows_the_grid_and_the_selector_rides_along() {
        let mut system = pool();
        let mut emoji = Emoji::default();
        // Bare spade: one column, text face. With the selector: emoji font.
        assert_eq!(
            emoji.spans(&mut system, "a\u{2660}b"),
            vec![("a\u{2660}b", false)],
            "a bare suit is a one-column text symbol"
        );
        assert_eq!(
            emoji.spans(&mut system, "a\u{2660}\u{fe0f}b"),
            vec![("a", false), ("\u{2660}\u{fe0f}", true), ("b", false)],
            "the selector promotes it and rides with it"
        );
    }

    /// A line is split into the fewest stretches that come from one face, so a
    /// row of emoji is one span and ordinary prose is one span.
    #[test]
    fn a_line_is_split_into_one_span_per_face() {
        let mut system = pool();
        let mut emoji = Emoji::default();
        assert_eq!(emoji.spans(&mut system, ""), Vec::new());
        assert_eq!(emoji.spans(&mut system, "plain"), vec![("plain", false)]);
        assert_eq!(
            emoji.spans(&mut system, "ok \u{2705}\u{1f604} done"),
            vec![("ok ", false), ("\u{2705}\u{1f604}", true), (" done", false)],
            "consecutive emoji are one span, not one span each"
        );
    }
}
