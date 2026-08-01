//! The agents pane: the detached fleet as a list, one row per thing said.

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
    let cols = cols_of(panel, frame.pane_column);
    list_pane(
        scene,
        frame,
        panel,
        View::Agents,
        agent_rows(frame.state, frame.skin, cols),
    );
}

/// One row per child, clipped to one physical row like the file explorer
/// clips its names: a child's brief is a paragraph, and a list whose
/// entries wrap into paragraphs is not a list. The rest of the child's
/// story lives on its `[N] AGENT - OUTPUT` tab.
///
/// The row leads with `[N] Agent` in the bright tone: it is the press that
/// opens that tab, and bright over a pane of dim detail is what says these
/// rows are for clicking.
pub(crate) fn agent_rows(state: &State, skin: &Skin, cols: usize) -> Vec<ListRow> {
    agent_list(state, skin, cols).0
}

/// Which agent each list row belongs to, by ordinal, aligned with
/// [`agent_rows`].
pub(crate) fn agent_at(frame: &Frame, panel: Panel, x: f32, y: f32) -> Option<usize> {
    let cols = cols_of(panel, frame.pane_column);
    let (rows, owners) = agent_list(frame.state, frame.skin, cols);
    let at = list_row_at(frame, panel, View::Agents, &rows, x, y)?;
    owners.get(at).copied().flatten()
}

/// The rows and their owners in one pass, so the two cannot drift.
fn agent_list(state: &State, skin: &Skin, cols: usize) -> (Vec<ListRow>, Vec<Option<usize>>) {
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
            Run::tinted(format!("{:<9}", agent.state), skin.tone(agent.tone)),
        ];
        // The tool set says whether this child can change anything, which is
        // the one thing about a detached child worth knowing at a glance.
        if !agent.tools.is_empty() {
            runs.push(Run::tinted(format!("{:<5}", agent.tools), skin.dim));
        }
        // One line per agent, whole: the brief's head, and everything else
        // on its own `[N] AGENT - OUTPUT` tab.
        runs.push(Run::tinted(&agent.brief, skin.body));
        rows.push(one_row(runs, cols));
        owners.push(Some(agent.ordinal));
    }
    (rows, owners)
}

/// Styled runs cut down to one physical row of the pane: the run that
/// crosses the column budget is clipped with an ellipsis and the rest are
/// dropped, so no entry of the list can wrap however long its text runs.
fn one_row(runs: Vec<Run>, cols: usize) -> ListRow {
    let budget = cols.max(8);
    let mut used = 0usize;
    let mut kept = Vec::new();
    for mut run in runs {
        let wide = run.text.chars().count();
        if used + wide <= budget {
            used += wide;
            kept.push(run);
            continue;
        }
        // `clip` spends one column on the ellipsis it appends.
        let room = (budget - used).saturating_sub(1);
        if room > 0 {
            run.text = clip(&run.text, room);
            kept.push(run);
        }
        break;
    }
    ListRow::new(kept)
}
