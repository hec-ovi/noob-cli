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

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    use crate::config::Config;
    use crate::dock::Dock;
    use crate::monitor::Monitor;
    use noob_draw::Text;
    

    /// A child's brief is a paragraph and its output lines run long; the
    /// list clips both to one row each, so the pane stays a list however
    /// long the fleet's prompts run.
    #[test]
    fn an_agent_entry_is_one_row_however_long_its_text() {
        let mut state = busy_state();
        state.apply(noob_proto::Event::AgentSpawn {
            agent_id: "kid".into(),
            prompt: "Research Arthur Schopenhauer. Focus on his main ideas. ".repeat(8),
            tools: "all".into(),
        });
        state.apply(noob_proto::Event::AgentOutput {
            agent_id: "kid".into(),
            line: "* websearch search over a very long query string ".repeat(8),
        });
        let skin = Skin::from(&Config::default());
        let cols = 48;
        let rows = crate::widgets::agents::agent_rows(&state, &skin, cols);
        for row in &rows {
            assert_eq!(row.rows(cols), 1, "an entry wrapped");
        }
    }
    /// A press on an agent's rows resolves to that agent, through the same
    /// geometry the list is drawn with, and a press past the list resolves
    /// to nothing.
    #[test]
    fn a_press_on_the_agents_list_names_the_agent_under_it() {
        let mut state = busy_state();
        for n in 1..=2 {
            state.apply(noob_proto::Event::AgentSpawn {
                agent_id: format!("kid-{n}"),
                prompt: format!("task {n}"),
                tools: "read".into(),
            });
            state.apply(noob_proto::Event::AgentOutput {
                agent_id: format!("kid-{n}"),
                line: format!("news {n}"),
            });
        }
        let mut dock = Dock::new();
        dock.reveal(View::Agents);
        let shape = shape(&dock, &[]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let skin = Skin::from(&Config::default());
        let frame = Frame {
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
        };
        let space = dock.space_of(View::Agents).expect("the pane is somewhere");
        let panel = layout.placed(space).body;
        let inset = panel.inset(PAD);
        let line = Text::line_for(13.0);
        // One row per agent: busy_state's own child, then the two spawned.
        let rows: Vec<Option<usize>> = (0..4)
            .map(|row| {
                crate::widgets::agents::agent_at(
                    &frame,
                    panel,
                    inset.x + 4.0,
                    inset.y + row as f32 * line + line * 0.5,
                )
            })
            .collect();
        assert_eq!(rows, vec![Some(1), Some(2), Some(3), None]);
        assert_eq!(
            crate::widgets::agents::agent_at(
                &frame,
                panel,
                inset.x + 4.0,
                inset.y + 20.0 * line,
            ),
            None,
            "past the list is nobody"
        );
    }
}
