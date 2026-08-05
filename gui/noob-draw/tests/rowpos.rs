//! Where the shaper puts each visual row. The selection bands, the caret and
//! the scrollbar all assume row k of a box sits exactly k line-heights from
//! its top; this pins the assumption at the layout itself.
//!
//! It exists because cosmic-text's own scroll broke it: a buffer that knows
//! its height clamps the scroll to keep its box full, and the clamp slid
//! every row off the grid by the box's sub-row remainder whenever the content
//! below the scroll ran short (a streaming tail, a reflowed table, a scrolled
//! transcript at certain heights). The selection bands were painted on the
//! grid and the glyphs were not, which is the misaligned-selection defect.
//! The renderer now lays the buffer out whole, unsized and unscrolled, and
//! translates the drawn area instead, so the grid claim below holds for every
//! skip and every content height.
use glyphon::cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Wrap};

#[test]
fn every_row_sits_exactly_its_row_number_down_the_buffer() {
    let mut fonts = FontSystem::new();
    fonts
        .db_mut()
        .load_font_data(std::fs::read("fonts/SymbolsNerdFontMono-Regular.ttf").unwrap());
    fonts
        .db_mut()
        .load_font_data(std::fs::read("fonts/NotoEmoji[wght].ttf").unwrap());

    let size = 14.0f32;
    let line_h = 20.0f32;
    // Rows the way the painter pre-breaks them: one visual row per '\n', the
    // emoji row as its own span at a scaled size the way the renderer draws
    // one, and a trailing empty row from the painter's last newline.
    let mono = Attrs::new().family(Family::Monospace);
    let emoji = Attrs::new()
        .family(Family::Name("Noto Emoji"))
        .metrics(Metrics::new(2.0 * 8.4 / 1.27, line_h));
    let mut buffer = Buffer::new(&mut fonts, Metrics::new(size, line_h));
    buffer.set_size(Some(520.0), None);
    buffer.set_wrap(Wrap::None);
    buffer.set_rich_text(
        [
            ("plain prose row one\nprose with wrap already done\n", mono.clone()),
            ("\u{1f600} \u{1f603} \u{1f604} \u{1f601}", emoji),
            ("\nafter the emoji row\nlast row\n", mono.clone()),
        ],
        &mono,
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut fonts, false);
    let placed: Vec<(usize, f32)> = buffer
        .layout_runs()
        .map(|run| (run.line_i, run.line_top))
        .collect();
    assert_eq!(placed.len(), 5, "five rows with glyphs on them: {placed:?}");
    for (line_i, line_top) in placed {
        assert!(
            (line_top - line_i as f32 * line_h).abs() < 0.01,
            "row {line_i} sits at {line_top}, not on the grid; the bands are \
             painted on the grid, so this is the misaligned-selection defect"
        );
    }
}
