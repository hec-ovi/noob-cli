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

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    
    use crate::dock::{Dock, Space};
    
    

    /// Clicking an agent's row opens that child's own output as a tab in
    /// the top-left space: the strip reads the agent's number, the pane its
    /// lines, and the AGENTS list leads every row with the number a click
    /// means.
    #[test]
    fn a_chosen_agent_has_its_own_output_tab() {
        let mut state = busy_state();
        state.apply(noob_proto::Event::AgentSpawn {
            agent_id: "kid".into(),
            prompt: "look around".into(),
            tools: "read".into(),
        });
        state.apply(noob_proto::Event::AgentOutput {
            agent_id: "kid".into(),
            line: "reading src/main.rs".into(),
        });

        // The list names every agent by its number: busy_state's own child
        // is [1], the one spawned here is [2].
        let mut listing = Dock::new();
        listing.reveal(View::Agents);
        let text = text_of(&render(&state, 1400.0, 900.0, &listing, &[]).scene);
        assert!(text.contains("[1] Agent"), "{text}");
        assert!(text.contains("[2] Agent"), "{text}");

        // Chosen, the tab stands in the top-left space under the agent's
        // number and carries that child's own lines, nobody else's.
        assert!(state.show_agent(2));
        let mut dock = Dock::new();
        assert!(dock.unhide(View::Agent));
        assert_eq!(dock.space_of(View::Agent), Some(Space::TopLeft));
        let out = render(&state, 1400.0, 900.0, &dock, &[]);
        let text = text_of(&out.scene);
        assert!(text.contains("[2] AGENT - OUTPUT"), "{text}");
        assert!(text.contains("reading src/main.rs"), "{text}");
    }
}
