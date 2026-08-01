//! The agent-output pane: one running sub-agent's own lines.

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

/// The `[N] AGENT - OUTPUT` pane: everything the chosen agent has said,
/// scrolled like a transcript. The tab only exists while an agent is chosen
/// and running, but a frame can still land between the agent finishing and
/// the shell hiding the tab, so the empty state is drawn rather than assumed
/// unreachable.
pub(crate) fn agent(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let rows = frame.layout.rows(panel, frame.pane_size);
    let cols = cols_of(panel, frame.pane_column);
    let Some(chosen) = state.agent_shown() else {
        scene.text(Text::rich(
            vec![Run::tinted(
                "no agent chosen: click one on the AGENTS pane",
                skin.dim,
            )],
            panel.inset(PAD),
            frame.pane_size,
            skin.dim,
        ));
        return;
    };
    let mut runs = Vec::new();
    for line in chosen.pane.visible(rows, cols) {
        runs.push(Run::tinted(&line.text, skin.tone(line.tone)));
        runs.push(Run::plain("\n"));
    }
    scene.text(
        Text::rich(runs, panel.inset(PAD), frame.pane_size, skin.body)
            .scrolled(chosen.pane.window(rows, cols).skip as f32)
            .wrap_at(cols),
    );
    scrollbar(scene, skin, panel, chosen.pane.thumb(rows, cols));
}
