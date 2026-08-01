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



/// The fleet: one running child per row, and under each the last thing it
/// said. A finished child is not here: the state box removes it in the event
/// that ends it, and its report reaches the conversation on its own.
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
/// The head row leads with `[N] Agent` in the bright tone: it is the press
/// that opens the child's own output tab, and bright over a pane of dim
/// detail is what says these rows are for clicking. While a child runs the
/// second row is its latest output line.
pub(crate) fn agent_rows(state: &State, skin: &Skin) -> Vec<ListRow> {
    agent_list(state, skin).0
}

/// Which agent each list row belongs to, by ordinal, aligned with
/// [`agent_rows`]: the head row and the news row under it both name their
/// agent, so a press on either opens the same tab.
pub(crate) fn agent_at(frame: &Frame, panel: Panel, x: f32, y: f32) -> Option<usize> {
    let (rows, owners) = agent_list(frame.state, frame.skin);
    let at = list_row_at(frame, panel, View::Agents, &rows, x, y)?;
    owners.get(at).copied().flatten()
}

/// The rows and their owners in one pass, so the two cannot drift.
fn agent_list(state: &State, skin: &Skin) -> (Vec<ListRow>, Vec<Option<usize>>) {
    if state.agents.is_empty() {
        return (
            vec![ListRow::new(vec![Run::tinted(
                "no sub-agents running",
                skin.dim,
            )])],
            vec![None],
        );
    }
    let mut rows = Vec::new();
    let mut owners = Vec::new();
    for agent in &state.agents {
        let mut runs = vec![
            Run::tinted(format!("{:<10}", format!("[{}] Agent", agent.ordinal)), skin.bright),
            Run::tinted(format!("{:<10}", agent.state), skin.tone(agent.tone)),
        ];
        // The tool set says whether this child can change anything, which is
        // the one thing about a detached child worth knowing at a glance.
        if !agent.tools.is_empty() {
            runs.push(Run::tinted(format!("{:<10}", agent.tools), skin.dim));
        }
        runs.push(Run::tinted(clip(&agent.brief, 300), skin.body));
        rows.push(ListRow::new(runs));
        owners.push(Some(agent.ordinal));
        if !agent.last.is_empty() {
            rows.push(ListRow::new(vec![Run::tinted(
                format!("           {}", clip(&agent.last, 300)),
                skin.dim,
            )]));
            owners.push(Some(agent.ordinal));
        }
    }
    (rows, owners)
}
