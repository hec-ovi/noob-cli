//! The agents pane: the detached fleet as list rows.

use noob_draw::{Panel, Run, Scene};

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



/// The fleet: one child per row, and under each the last thing it said.
pub(crate) fn agents(scene: &mut Scene, frame: &Frame, panel: Panel) {
    list_pane(
        scene,
        frame,
        panel,
        View::Agents,
        agent_rows(frame.state, frame.skin),
    );
}

/// Two rows per child, and the second is where the news is.
///
/// A row alone is a name and a word, which for eight children at once tells you
/// nothing about any of them: while a child runs the second row is that child's
/// own output, and once it ends it is the reason it ended. Two rows each is also
/// why this pane needs a scroll more than any other, a fleet of eight being
/// sixteen rows.
pub(crate) fn agent_rows(state: &State, skin: &Skin) -> Vec<ListRow> {
    if state.agents.is_empty() {
        return vec![ListRow::new(vec![Run::tinted(
            "no sub-agents this session",
            skin.dim,
        )])];
    }
    let mut rows = Vec::new();
    for agent in &state.agents {
        let mut runs = vec![
            Run::tinted(format!("{:<9}", agent.label), skin.dim),
            Run::tinted(format!("{:<10}", agent.state), skin.tone(agent.tone)),
        ];
        // The tool set says whether this child can change anything, which is
        // the one thing about a detached child worth knowing at a glance.
        if !agent.tools.is_empty() {
            runs.push(Run::tinted(format!("{:<10}", agent.tools), skin.dim));
        }
        runs.push(Run::tinted(clip(&agent.brief, 300), skin.body));
        rows.push(ListRow::new(runs));
        if !agent.last.is_empty() {
            rows.push(ListRow::new(vec![Run::tinted(
                format!("           {}", clip(&agent.last, 300)),
                skin.dim,
            )]));
        }
    }
    rows
}
