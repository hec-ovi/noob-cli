//! The rows the dock pins above the prompt: the plan as a checklist, the fleet
//! as a block, and a queued message as one line.
//!
//! Split from the dock's own loop: what is pinned and how it is capped to the
//! screen is one subject, and the loop that redraws it is another.

use super::style::RESET;
use super::table;
use crate::ui::{RegionTone, Ui};

/// How many steps of a plan the dock pins before it caps the rest into a
/// "more" row.
pub(crate) const PLAN_STEP_ROWS: usize = 6;



/// One pinned row per queued message, styled exactly like the `› message`
/// record it will become (green marker, plain text) with only the trailing
/// `[queued]` tag in the non-bold activity green. Clamped to one physical row
/// like every region row. It lives only in the pinned region while the
/// message waits; dispatch removes the row (and the tag with it) and echoes
/// the plain `› message` record into the transcript.
pub(crate) fn queued_region_row(ui: &Ui, message: &str, width: usize) -> PinnedRegionRow {
    const TAG: &str = "[queued]";
    let shown: String = message
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    // Marker (2 cells) + text + one space + tag must fit one physical row.
    // Budgeted in display cells like every region row (clamp_to_row), so a
    // wide CJK/emoji message cannot wrap the pinned row and desync the
    // frame-height bookkeeping every later erase relies on.
    let avail = width.max(1).saturating_sub(2 + 1 + TAG.len());
    let text = if table::cell_width(&shown) <= avail {
        shown
    } else {
        let budget = avail.saturating_sub(1);
        let mut used = 0usize;
        let mut clipped = String::new();
        for c in shown.chars() {
            let w = table::char_width(c);
            if used + w > budget {
                break;
            }
            clipped.push(c);
            used += w;
        }
        clipped.push('…');
        clipped
    };
    let marker = ui.box_color();
    let marker_reset = if marker.is_empty() { "" } else { RESET };
    let tag = if ui.regions_enabled() {
        ui.theme.activity.sgr(ui.depth)
    } else {
        String::new()
    };
    let tag_reset = if tag.is_empty() { "" } else { RESET };
    PinnedRegionRow {
        rendered: format!("{marker}› {marker_reset}{text} {tag}{TAG}{tag_reset}"),
        priority: RegionPriority::None,
    }
}
pub(crate) fn frame_label(input: &str) -> String {
    let mut shown = String::new();
    let mut chars = input.chars();
    for ch in chars.by_ref().take(80) {
        shown.push(if ch.is_control() { ' ' } else { ch });
    }
    if chars.next().is_some() {
        shown.push('…');
    }
    shown
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RegionPriority {
    None,
    Plan,
    Agents,
}
#[derive(Clone, Copy)]
pub(crate) enum RegionSource {
    Plan,
    Agents,
}
pub(crate) struct PinnedRegionRow {
    pub(crate) rendered: String,
    pub(crate) priority: RegionPriority,
}
/// The plan region with the plan's own cap: every non-step row (the header),
/// a contiguous window of at most PLAN_STEP_ROWS steps that contains the
/// active one and prefers what comes next, and one dim "… +N more" row
/// naming what is hidden. A plan at or under the cap renders whole. The
/// screen cap in `cap_region_rows` still applies afterwards, so the active
/// step keeps its reservation there via its row priority.
pub(crate) fn capped_plan_rows(ui: &Ui, text: &str, width: usize) -> Vec<PinnedRegionRow> {
    let rows = checklist_pinned_rows(ui, text, width, RegionSource::Plan);
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    let step_lines: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| is_plan_step(line))
        .map(|(index, _)| index)
        .collect();
    let steps: Vec<&str> = step_lines.iter().map(|&index| lines[index]).collect();
    let Some((range, hidden_done, hidden_queued)) = plan_cap_selection(&steps) else {
        return rows;
    };
    let window = &step_lines[range];

    let mut visible: Vec<PinnedRegionRow> = rows
        .into_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            (!is_plan_step(lines[index]) || window.contains(&index)).then_some(row)
        })
        .collect();
    visible.push(PinnedRegionRow::summary(
        ui,
        plan_cap_label(hidden_done, hidden_queued),
        width,
        RegionTone::Dim,
        RegionPriority::None,
    ));
    visible
}
pub(crate) fn is_plan_step(line: &str) -> bool {
    line.starts_with("[x]")
        || line.starts_with("[!]")
        || line.starts_with("[~]")
        || line.starts_with("[ ]")
}
/// The pinned rows for one plan checklist: the capped step window while it
/// runs, collapsing to a one-line "plan completed" summary once every step is
/// done (the summary stays pinned only until the turn ends; then
/// `retire_completed_plan` moves it into the transcript). Shared by the
/// in-turn regions and the idle prompt so the plan looks the same wherever it
/// is pinned.
pub(crate) fn plan_region_rows(ui: &Ui, text: &str, width: usize) -> Vec<PinnedRegionRow> {
    if let Some(label) = completed_plan_label(text) {
        vec![PinnedRegionRow::summary(
            ui,
            label,
            width,
            RegionTone::Activity,
            RegionPriority::Plan,
        )]
    } else {
        capped_plan_rows(ui, text, width)
    }
}
/// The one-line summary of a plan whose every step is done, shared by the
/// mid-turn pinned row and the turn-end transcript record so both read
/// identically. None while any step is still open.
pub(crate) fn completed_plan_label(text: &str) -> Option<String> {
    let counts = checklist_counts(text);
    if !counts.is_complete() {
        return None;
    }
    let mut label = format!("plan completed · {}/{}", counts.done, counts.total());
    if let Some(plan_elapsed) = plan_elapsed(text) {
        label.push_str(" · ");
        label.push_str(plan_elapsed);
    }
    Some(label)
}
/// Pure window math for the plan cap. Given the step glyph lines in order,
/// the contiguous PLAN_STEP_ROWS window to show and the hidden done/queued
/// counts; None when the plan already fits. The window anchors on the active
/// step (falling back to the first pending one) so it shows the active step
/// plus what comes next, shifting back only when the tail runs short.
pub(crate) fn plan_cap_selection(steps: &[&str]) -> Option<(std::ops::Range<usize>, usize, usize)> {
    if steps.len() <= PLAN_STEP_ROWS {
        return None;
    }
    let anchor = steps
        .iter()
        .position(|step| step.starts_with("[~]"))
        .or_else(|| steps.iter().position(|step| step.starts_with("[ ]")))
        .unwrap_or(0);
    let start = anchor.min(steps.len() - PLAN_STEP_ROWS);
    let range = start..start + PLAN_STEP_ROWS;
    let mut hidden_done = 0usize;
    let mut hidden_queued = 0usize;
    for (position, step) in steps.iter().enumerate() {
        if range.contains(&position) {
            continue;
        }
        if step.starts_with("[ ]") {
            hidden_queued += 1;
        } else {
            hidden_done += 1;
        }
    }
    Some((range, hidden_done, hidden_queued))
}
pub(crate) fn plan_cap_label(hidden_done: usize, hidden_queued: usize) -> String {
    let hidden = hidden_done + hidden_queued;
    let mut label = format!(
        "… +{hidden} more step{}",
        if hidden == 1 { "" } else { "s" }
    );
    if hidden_done > 0 {
        label.push_str(&format!(" · {hidden_done} done"));
    }
    if hidden_queued > 0 {
        label.push_str(&format!(" · {hidden_queued} queued"));
    }
    label
}
pub(crate) fn checklist_pinned_rows(
    ui: &Ui,
    text: &str,
    width: usize,
    source: RegionSource,
) -> Vec<PinnedRegionRow> {
    let source_lines: Vec<&str> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    ui.checklist_region_rows(text, width)
        .into_iter()
        .zip(source_lines)
        .enumerate()
        .map(|(index, (rendered, line))| {
            let priority = match source {
                RegionSource::Plan if line.starts_with("[~]") => RegionPriority::Plan,
                RegionSource::Agents if index == 0 => RegionPriority::Agents,
                _ => RegionPriority::None,
            };
            PinnedRegionRow { rendered, priority }
        })
        .collect()
}
pub(crate) fn agent_snapshot_block(snapshot: &crate::subagent::JobsSnapshot) -> String {
    let mut block = format!(
        "agents ({} active, {} ready): · {} queued · {} running · Tab to close",
        snapshot.active, snapshot.ready, snapshot.queued, snapshot.running,
    );
    for row in &snapshot.rows {
        let glyph = if row.contains(" · queued · ") {
            "[ ]"
        } else if row.contains(" · ready · ") {
            "[x]"
        } else {
            "[~]"
        };
        block.push('\n');
        block.push_str(glyph);
        block.push(' ');
        block.push_str(row);

        let id = row.split(" · ").next().unwrap_or_default();
        if let Some(progress) = snapshot
            .recent_progress
            .iter()
            .find(|progress| progress.id == id)
        {
            for line in &progress.lines {
                block.push('\n');
                block.push_str("    ");
                block.push_str(id);
                block.push_str(" │ ");
                block.push_str(line);
            }
        }
    }
    block
}
pub(crate) fn expanded_agent_snapshot_block(snapshot: &crate::subagent::JobsSnapshot) -> Option<String> {
    (!snapshot.rows.is_empty()).then(|| agent_snapshot_block(snapshot))
}
pub(crate) fn collapsed_agent_snapshot_block(snapshot: &crate::subagent::JobsSnapshot) -> Option<String> {
    if snapshot.active > 0 {
        // After an explicit stop-everything cancel the whole fleet is
        // winding down; "running" would misread as the cancel having been
        // ignored while the workers reap the children.
        if snapshot.stopping == snapshot.active {
            return Some(format!(
                "[{}] agents stopping (Tab to view)",
                snapshot.active
            ));
        }
        Some(format!(
            "[{}] agents running (Tab to view)",
            snapshot.active
        ))
    } else if snapshot.ready > 0 {
        Some(format!("[{}] agents ready (Tab to view)", snapshot.ready))
    } else {
        None
    }
}
/// New plan payloads append lifecycle time after the compatible header,
/// `plan (x/y done): · 1.2s`. Old payloads have no suffix and fall back to the
/// dock turn duration in final summaries.
pub(crate) fn plan_elapsed(text: &str) -> Option<&str> {
    let header = text.lines().next()?;
    let (_, elapsed) = header.split_once("): · ")?;
    let elapsed = elapsed.trim();
    (!elapsed.is_empty()).then_some(elapsed)
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct ChecklistCounts {
    pub(crate) done: usize,
    pub(crate) active: usize,
    pub(crate) pending: usize,
}
pub(crate) fn checklist_counts(text: &str) -> ChecklistCounts {
    let mut counts = ChecklistCounts::default();
    for line in text.lines() {
        if line.starts_with("[x]") || line.starts_with("[!]") {
            counts.done += 1;
        } else if line.starts_with("[~]") {
            counts.active += 1;
        } else if line.starts_with("[ ]") {
            counts.pending += 1;
        }
    }
    counts
}
pub(crate) fn animated_region_row(row: &str, tick: usize) -> String {
    const FRAMES: [&str; 4] = ["[|]", "[/]", "[-]", "[\\]"];
    row.replacen("[~]", FRAMES[tick % FRAMES.len()], 1)
}
pub(crate) fn styled_rule(label: &str, width: usize, open: &str) -> String {
    let reset = if open.is_empty() { "" } else { RESET };
    let max_label = width.saturating_sub(4);
    let mut shown: String = label.chars().take(max_label).collect();
    if label.chars().count() > max_label && max_label > 0 {
        shown.pop();
        shown.push('…');
    }
    let used = (3 + shown.chars().count() + 1).min(width);
    let fill = "─".repeat(width.saturating_sub(used));
    format!("{open}── {shown} {fill}{reset}")
}

impl PinnedRegionRow {
    fn summary(
        ui: &Ui,
        label: String,
        width: usize,
        tone: RegionTone,
        priority: RegionPriority,
    ) -> PinnedRegionRow {
        PinnedRegionRow {
            rendered: ui.region_summary_row(&label, width, tone),
            priority,
        }
    }
}

impl ChecklistCounts {
    pub(crate) fn total(self) -> usize {
        self.done + self.active + self.pending
    }

    pub(crate) fn is_complete(self) -> bool {
        self.total() > 0 && self.done == self.total()
    }

    pub(crate) fn plus(self, other: ChecklistCounts) -> ChecklistCounts {
        ChecklistCounts {
            done: self.done + other.done,
            active: self.active + other.active,
            pending: self.pending + other.pending,
        }
    }
}
