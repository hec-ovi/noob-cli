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
