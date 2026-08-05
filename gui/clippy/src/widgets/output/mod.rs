//! The transcript pane: the conversation itself, wrapped and scrolled.

use noob_draw::{Panel, Run, Scene, Text};

#[allow(unused_imports)]
use crate::dock::View;
#[allow(unused_imports)]
use crate::monitor::Gauge;
#[allow(unused_imports)]
use crate::state::{State, Tone, TodoState};
#[allow(unused_imports)]
use crate::style::skin::Skin;
#[allow(clippy::wildcard_imports)]
use crate::view::*;



/// The OUTPUT pane: what the model said, as Markdown, with any messages
/// waiting behind the running turn pinned to its bottom rows.
pub(crate) fn output(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    // The queued messages take the bottom of the panel before the transcript
    // is measured, so the pinned rows stand outside the scrollback: they are
    // there wherever the conversation is scrolled to, which is the point of
    // pinning them.
    let fit = frame.layout.rows(panel, frame.body_size);
    let reserved = state.output_reserved(fit);
    let rows = fit - reserved;
    let cols = cols_of(panel, frame.column);
    let mut runs = Vec::new();
    // A window that starts inside a fenced block has to know it is looking at
    // code, so the state is carried in from the lines above it.
    let mut fence = state.output.fence_before(rows, cols);
    for line in state.output.visible(rows, cols) {
        match line.tone {
            // A table is laid out across its block for this panel's width, so
            // it arrives here as the columns it is drawn as: tinted, not
            // marked up.
            Tone::Body if line.table().is_some() => table(line, skin, &mut runs),
            // Only the model's prose is Markdown. What the human typed and
            // what the harness noted are shown as written.
            Tone::Body => crate::markdown::line(&line.text, &mut fence, skin, &mut runs),
            tone => runs.push(Run::tinted(&line.text, skin.tone(tone))),
        }
        runs.push(Run::plain("\n"));
    }
    // The window may start partway down a wrapped line rather than dropping
    // it, so the shaped buffer is scrolled by the rows that sit above.
    //
    // The box names its column count, so the renderer breaks the rows with the
    // same `text-geometry` call the pane was measured with rather than wrapping
    // them itself. Left to the shaper the columns drift by one per blank it
    // swallows at a break, and the selection lands on the wrong glyphs.
    scene.text(
        Text::rich(runs, panel.inset(PAD), frame.body_size, frame.skin.body)
            .scrolled(state.output.window(rows, cols).skip as f32)
            .wrap_at(cols),
    );
    // The pinned rows themselves: one dim line per waiting message, styled
    // like the `› message` record it will become with the `[queued]` tag on
    // the end, clipped to one physical row so a long message cannot wrap into
    // the transcript's room. A queue deeper than the panel says how much of
    // it is out of sight rather than hiding it.
    if reserved > 0 {
        let inset = panel.inset(PAD);
        let line = Text::line_for(frame.body_size);
        let mut pinned = Vec::new();
        for (step, message) in state.queued.iter().take(reserved).enumerate() {
            let text = if step + 1 == reserved && state.queued.len() > reserved {
                format!("… {} more queued", state.queued.len() - reserved + 1)
            } else {
                clip(&format!("› {message} [queued]"), cols)
            };
            pinned.push(Run::tinted(text, skin.dim));
            pinned.push(Run::plain("\n"));
        }
        let band = Panel::new(
            inset.x,
            inset.y + rows as f32 * line,
            inset.w,
            reserved as f32 * line,
        );
        scene.text(Text::rich(pinned, band, frame.body_size, skin.dim));
    }
    scrollbar(scene, skin, panel, state.output.thumb(rows, cols));
}

/// One line of a laid-out table: the columns it stands in are structure and
/// wear the markup tint, the cells are text, and the row naming the columns is
/// the bright one.
pub(crate) fn table(line: &crate::state::Line, skin: &Skin, runs: &mut Vec<Run>) {
    let ink = match line.table() {
        Some(crate::table::Part::Head) => skin.bright,
        _ => skin.body,
    };
    let mut cell = String::new();
    for ch in line.shown().chars() {
        // The rule row is all structure, and so is every separator on a body
        // row. Everything between them is what the model wrote.
        if matches!(ch, '│' | '─' | '┼') {
            if !cell.is_empty() {
                runs.push(Run::tinted(std::mem::take(&mut cell), ink));
            }
            runs.push(Run::tinted(ch.to_string(), skin.markup));
            continue;
        }
        cell.push(ch);
    }
    if !cell.is_empty() {
        runs.push(Run::tinted(cell, ink));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    use crate::config::Config;
    use crate::dock::{Dock, Space};
    use crate::monitor::Monitor;
    use crate::state::Tone;

    /// A message waiting behind the running turn is pinned into the OUTPUT
    /// pane as a tagged row; dispatching it takes the tag away.
    #[test]
    fn a_queued_message_pins_a_tagged_row_into_the_output_pane() {
        let mut state = busy_state();
        state.enqueue("try the other file");
        let out = render_at(&state, 0.0);
        assert!(text_of(&out.scene).contains("› try the other file [queued]"));
        state.apply(noob_proto::Event::TurnEnd {
            turn: 1,
            interrupted: None,
        });
        state.apply(noob_proto::Event::TurnStart { turn: 2 });
        let text = text_of(&render_at(&state, 0.0).scene);
        assert!(text.contains("› try the other file"));
        assert!(!text.contains("[queued]"));
    }
    /// The transcript is drawn as Markdown, and it is counted in the Markdown
    /// it drew.
    ///
    /// `markdown::line` eats the stars, the backticks and the hashes and turns
    /// `- ` into `• `, so the drawn line is shorter than the line the model
    /// wrote. While the pane measured the source, the tail of every marked-up
    /// line had no glyph to point at, and a wrapped one was out by however many
    /// marks were consumed above the pointer. The pane renders the line as it
    /// pushes it now, and its rows, its bands and its clipboard are all counted
    /// in that, so this asserts the whole of it at once: what the renderer lays
    /// out on a row is what a drag across that row would copy.
    /// The same, over the lines that broke it in the field: a table whose rows
    /// hold pipes, a link with its URL in it, an arrow and a ballot box (both
    /// one column and several bytes), and a numbered list. Every one of them
    /// wraps, and a single character of disagreement between what is measured
    /// and what is drawn slides every band below it.
    #[test]
    fn the_transcript_is_counted_in_the_markdown_it_draws_for_wide_characters() {
        let mut state = busy_state();
        for text in [
            "1. Websearch tool: \u{2610} Online and working. SearXNG 2026.8.1 with 83 \
             engines, plus DuckDuckGo and Google as keyless providers.",
            "2. Search result for \"llama.cpp grammar tool calls\": The top hit is a \
             GitHub discussion asking whether custom grammars can compose with tool \
             calls ([link](https://github.com/ggml-org/llama.cpp/discussions/22408)).",
            "| websearch | Search the web, fetch pages as Markdown, find papers/repos \
             via SearXNG (init \u{2192} search/fetch/arxiv/github) |",
            "| skill | Load a specialized skill (e.g., cloudflare, \
             workers-best-practices) to guide your actions |",
        ] {
            state.output.say(text, Tone::Body);
        }
        assert_rows_hold_what_a_drag_would_copy(state);
    }

    #[test]
    fn the_transcript_is_counted_in_the_markdown_it_draws() {
        let mut state = busy_state();
        for text in [
            "## Notable Features",
            "- **read** a file, then `write` it back out again with __every__ \
             mark it came with, which is a good few marks and a good few blanks \
             and more than one row of any pane",
            "plain prose with no marks in it at all, long enough to wrap over a \
             row boundary and land some of itself on a second row",
            "",
            "1. a numbered item with `code` in it and a **bold** run near the end",
        ] {
            state.output.say(text, Tone::Body);
        }

        assert_rows_hold_what_a_drag_would_copy(state);
    }

    /// A table is drawn as the columns it describes, and everything the pane
    /// reports about it is counted in those columns: this is the same rule as
    /// the cases above, over the one kind of line that is laid out across a
    /// whole block rather than on its own.
    #[test]
    fn a_table_is_drawn_as_columns_and_counted_in_them() {
        let mut state = busy_state();
        for text in [
            "## Tools",
            "| Tool | What it does |",
            "|---|---|",
            "| websearch | Search the web, fetch pages as Markdown, find papers and repos \
             via SearXNG, then follow the links it comes back with |",
            "| skill | Load a specialized skill to guide your actions |",
            "",
            "back to prose",
        ] {
            state.output.say(text, Tone::Body);
        }
        let drawn = assert_rows_hold_what_a_drag_would_copy(state);
        let screen = drawn.join("\n");
        assert!(screen.contains('│'), "the columns are drawn: {screen}");
        assert!(!screen.contains("| Tool |"), "the source pipes are gone: {screen}");
        assert!(screen.contains("back to prose"), "prose after it is prose");
        // The separator stands in one column all the way down the block.
        let at: Vec<usize> = drawn
            .iter()
            .filter_map(|row| row.find('│'))
            .collect();
        assert!(at.len() > 3, "every row of the table has one: {screen}");
        assert!(at.windows(2).all(|two| two[0] == two[1]), "{screen}");
    }

    /// Every visual row holds exactly the characters a drag across it would
    /// copy, and a wrapped line's rows add back up to the line. Shared by the
    /// cases below so one rule is proven once over several kinds of text.
    ///
    /// Returns the rows as they were laid out, for a case with something of its
    /// own to say about them.
    fn assert_rows_hold_what_a_drag_would_copy(mut state: crate::state::State) -> Vec<String> {
        let dock = Dock::new();
        let shape = shape(&dock, &["a.rs"]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let panel = layout.placed(Space::TopLeft).body;
        let cols = cols_of(panel, 8.0);
        // What the shell does before it draws, and for the same reason: a table
        // is laid out for the panel it is in.
        state.output.reflow(cols);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot: None,
            trouble: None,
            esc_armed: false,
            popup_scroll: [0, 0],
            cursor: (-100.0, -100.0),
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        });

        let text = scene
            .texts
            .iter()
            .find(|text| text.at == panel.inset(PAD))
            .expect("the output pane draws its text");
        assert_eq!(text.wrap_cols, Some(cols));
        let laid: String = noob_draw::Run::wrapped(&text.runs, cols, text.wrap_break)
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        let drawn: Vec<Vec<char>> = laid.split('\n').map(|row| row.chars().collect()).collect();

        let rows = layout.rows(panel, 14.0);
        let skip = state.output.window(rows, cols).skip;
        let mut checked = 0;
        for row in 0..rows {
            let Some((line, start)) = state.output.spot_in(rows, cols, row, 0) else {
                break;
            };
            let (same, end) = state
                .output
                .spot_in(rows, cols, row, cols + 9)
                .expect("the row a moment ago is still a row");
            assert_eq!(same, line, "row {row} lands on two different lines");
            let held = state.output.line(line).expect("a row of a line still held");
            let source: Vec<char> = held.shown().chars().take(end).skip(start).collect();
            assert_eq!(
                drawn[row + skip], source,
                "screen row {row} holds something other than what a selection there would copy"
            );
            checked += 1;
        }
        assert!(checked > 6, "only {checked} rows were on screen");

        // And a line that takes several rows is drawn as the rows it is
        // counted as, in order, which is what keeps every row below it in step.
        let tall = (0..rows)
            .filter_map(|row| state.output.spot_in(rows, cols, row, 0))
            .map(|(line, _)| line)
            .max_by_key(|line| state.output.rows_of_line(*line, cols).len())
            .expect("something is on screen");
        let held = state.output.line(tall).expect("the tallest line is held");
        let counted = state.output.rows_of_line(tall, cols);
        assert!(counted.len() > 1, "nothing on screen takes more than a row");
        let top = (0..rows)
            .find(|row| state.output.spot_in(rows, cols, *row, 0).map(|(l, _)| l) == Some(tall))
            .expect("the line is on screen")
            + skip;
        for (step, span) in counted.iter().enumerate() {
            let held: Vec<char> = held.shown().chars().take(span.end).skip(span.start).collect();
            assert_eq!(drawn[top + step], held, "row {step} of the tallest line");
        }

        drawn.iter().map(|row| row.iter().collect()).collect()
    }
}
