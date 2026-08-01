//! The plan pane: the agent's checklist as list rows.

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



pub(crate) fn plan(scene: &mut Scene, frame: &Frame, panel: Panel) {
    list_pane(scene, frame, panel, View::Plan, plan_rows(frame.state, frame.skin));
}

/// One row per todo, wrapped in whatever width the pane has.
pub(crate) fn plan_rows(state: &State, skin: &Skin) -> Vec<ListRow> {
    if state.plan.is_empty() {
        return vec![ListRow::new(vec![Run::tinted("no plan yet", skin.dim)])];
    }
    state
        .plan
        .iter()
        .map(|todo| {
            let (mark, color) = match todo.state {
                TodoState::Done => ("[x] ", skin.good),
                TodoState::Active => ("[>] ", skin.bright),
                TodoState::Pending => ("[ ] ", skin.dim),
            };
            ListRow::new(vec![
                Run::tinted(mark, color),
                Run::tinted(&todo.text, color),
            ])
        })
        .collect()
}
