//! The multi-agent fan-out panel (P2): a legible, live checklist of the
//! sub-agents a `task` batch spawned. It exists because a fan-out used to
//! render N identical truncated `* task ...` lines and then only `* task done`,
//! so a human could not tell which agent was doing what or read any result.
//!
//! This module is pure state + text: it groups the batch's consecutive `task`
//! calls (a run of two or more is a panel), gives each a stable ordinal and a
//! distinguishing prompt slice, and folds the scheduler's Started/Finished
//! transitions into a re-rendered checklist block. The block reuses the exact
//! `[~]`/`[x]` glyph vocabulary the `plan` tool renders, so the themed REPL
//! paints it with the same code. It never touches the transcript: the panel is
//! a display artifact, and the sub-agent result string still enters the model
//! context exactly as before.

use noob_provider::types::ToolCall;
use std::time::Duration;

/// One re-render of a panel: the plain checklist text (header + one glyph line
/// per agent) and the call ids the panel covers. The ids let the themed REPL
/// suppress the now-redundant per-task `* task` activity lines in favor of the
/// block; byte-identity surfaces ignore both.
pub(crate) struct AgentsRender {
    pub block: String,
    pub ids: Vec<String>,
}

/// One sub-agent row.
struct Member {
    /// 1-based position within its fan-out group (`agent 1`, `agent 2`, ...).
    ordinal: usize,
    /// A distinguishing slice of the prompt (head + ellipsis + tail), so a
    /// shared prefix like a common URL base does not collapse the rows.
    label: String,
    done: bool,
    failed: bool,
    canceled: bool,
    /// `(M turns)` parsed from the ok summary; None on error/cancel.
    turns: Option<String>,
    elapsed: Option<Duration>,
    /// First non-empty line of the sub-agent's result, clipped to one row.
    digest: Option<String>,
}

/// One fan-out group: a maximal run of two or more consecutive `task` calls.
struct Group {
    members: Vec<Member>,
    ids: Vec<String>,
    /// The initial all-running block has been emitted at least once.
    opened: bool,
    /// Detached jobs outlive this provider turn. Show only the compact
    /// running-count row; details live behind Tab and `/agents`.
    collapsed: bool,
}

/// All fan-out panels in one tool batch, plus the per-call routing to them.
pub(crate) struct Panels {
    /// call index in the batch -> (group, member) when it is a panel agent.
    slot: Vec<Option<(usize, usize)>>,
    groups: Vec<Group>,
    /// The concurrency cap surfaced in the header (`up to C at once`).
    concurrency: usize,
    /// Latest accepted detached-job observation in this tool batch. Scheduler
    /// completion events can arrive out of order even though hub job ordinals
    /// are assigned under one lock; keep an older acknowledgment from making
    /// the displayed global active count move backward.
    background_status: Option<(u64, usize)>,
}

/// Slice budget for a prompt label: keep a head and a tail so a shared prefix
/// never collapses two rows into the same text.
const HEAD: usize = 30;
const TAIL: usize = 30;
/// One-row clip for a result digest.
const DIGEST: usize = 72;

impl Panels {
    /// Group the batch's consecutive `task` calls; a run of two or more becomes
    /// a panel (a lone task keeps its classic single activity line).
    pub(crate) fn build(calls: &[ToolCall], concurrency: usize) -> Panels {
        Self::build_with(calls, concurrency, false)
    }

    pub(crate) fn build_background(calls: &[ToolCall], concurrency: usize) -> Panels {
        Self::build_with(calls, concurrency, true)
    }

    fn build_with(calls: &[ToolCall], concurrency: usize, collapsed: bool) -> Panels {
        let mut slot = vec![None; calls.len()];
        let mut groups: Vec<Group> = Vec::new();
        let mut i = 0;
        while i < calls.len() {
            if calls[i].name != "subagent" {
                i += 1;
                continue;
            }
            let start = i;
            while i < calls.len() && calls[i].name == "subagent" {
                i += 1;
            }
            if i - start < 2 && !collapsed {
                continue; // a lone task is not a fan-out; leave it as-is
            }
            let g = groups.len();
            let mut members = Vec::new();
            let mut ids = Vec::new();
            for (ord, idx) in (start..i).enumerate() {
                slot[idx] = Some((g, ord));
                members.push(Member {
                    ordinal: ord + 1,
                    label: prompt_slice(&prompt_of(&calls[idx])),
                    done: false,
                    failed: false,
                    canceled: false,
                    turns: None,
                    elapsed: None,
                    digest: None,
                });
                ids.push(calls[idx].id.clone());
            }
            groups.push(Group {
                members,
                ids,
                opened: false,
                collapsed,
            });
        }
        Panels {
            slot,
            groups,
            concurrency: concurrency.max(1),
            background_status: None,
        }
    }

    /// A task in this group is starting. Inline fan-outs open immediately.
    /// Detached calls wait for their successful hub acknowledgment: registering
    /// the id here would suppress the only useful diagnostic when validation,
    /// queue admission, or cancellation fails before a job exists.
    pub(crate) fn on_started(&mut self, index: usize) -> Option<AgentsRender> {
        let (g, _) = self.slot.get(index).copied().flatten()?;
        if self.groups[g].collapsed {
            return None;
        }
        if self.groups[g].opened {
            return None;
        }
        self.groups[g].opened = true;
        Some(self.render(g))
    }

    /// A task in this group finished: flip its row to done with the turn count
    /// and a one-line result digest, then re-render the whole block. Opens the
    /// group if a canned outcome finished it before any real start.
    pub(crate) fn on_finished(
        &mut self,
        index: usize,
        summary: &str,
        content: &str,
        is_error: bool,
        canceled: bool,
        elapsed: Option<Duration>,
    ) -> Option<AgentsRender> {
        let (g, m) = self.slot.get(index).copied().flatten()?;
        if self.groups[g].collapsed {
            // Only a real hub acknowledgment may cover this call's ordinary
            // completion line. Errors and cancellations return None, leaving
            // their diagnostic visible instead of replacing it with a phantom
            // "agents running" row. The acknowledgment summary carries the
            // hub-wide active count without changing the transcript JSON.
            let (ordinal, observed_active) =
                background_ack_active(summary, content, is_error, canceled)?;
            let active = match self.background_status {
                Some((seen, active)) if seen >= ordinal => active,
                _ => {
                    self.background_status = Some((ordinal, observed_active));
                    observed_active
                }
            };
            self.groups[g].opened = true;
            return Some(AgentsRender {
                block: format!("[{active}] agents running (Tab to view)"),
                ids: vec![self.groups[g].ids[m].clone()],
            });
        }
        let member = &mut self.groups[g].members[m];
        member.done = true;
        member.failed = is_error && !canceled;
        member.canceled = canceled;
        member.turns = if is_error { None } else { turns_of(summary) };
        member.elapsed = elapsed;
        member.digest = digest_of(content);
        self.groups[g].opened = true;
        Some(self.render(g))
    }

    fn render(&self, g: usize) -> AgentsRender {
        let group = &self.groups[g];
        let total = group.members.len();
        let done = group.members.iter().filter(|m| m.done).count();
        debug_assert!(
            !group.collapsed,
            "collapsed panels render only after an acknowledgment"
        );
        let mut block = format!(
            "agents ({done}/{total} done, up to {} at once):",
            self.concurrency
        );
        for m in &group.members {
            block.push('\n');
            if m.done {
                let glyph = if m.failed {
                    "[!]"
                } else if m.canceled {
                    "[-]"
                } else {
                    "[x]"
                };
                block.push_str(&format!("{glyph} agent {}: ", m.ordinal));
                block.push_str(m.digest.as_deref().unwrap_or(&m.label));
                if let Some(turns) = &m.turns {
                    block.push_str(&format!("  {turns}"));
                }
                if let Some(elapsed) = m.elapsed {
                    block.push_str(&format!(" · {}", crate::ui::elapsed_label(elapsed)));
                }
            } else {
                block.push_str(&format!("[~] agent {}: {}", m.ordinal, m.label));
            }
        }
        AgentsRender {
            block,
            ids: group.ids.clone(),
        }
    }
}

/// Validate the exact detached-job acknowledgment and recover the hub-wide
/// active count from its display-only summary. The JSON content deliberately
/// stays the stable `{job_id,status}` tool result consumed by the model and by
/// saved sessions.
fn background_ack_active(
    summary: &str,
    content: &str,
    is_error: bool,
    canceled: bool,
) -> Option<(u64, usize)> {
    if is_error || canceled {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(content).ok()?;
    if value.get("status").and_then(serde_json::Value::as_str) != Some("running") {
        return None;
    }
    let id = value.get("job_id").and_then(serde_json::Value::as_str)?;
    let ordinal = id.strip_prefix("agent-")?.parse::<u64>().ok()?;
    let (started, count) = summary.rsplit_once(" · ")?;
    if started.strip_suffix(" started")? != id {
        return None;
    }
    let active = count
        .strip_suffix(" active")?
        .parse::<usize>()
        .ok()
        .filter(|count| *count > 0)?;
    Some((ordinal, active))
}

/// The `prompt` argument of a `task` call, empty when absent or unparseable
/// (the tool itself reports a missing prompt; the panel just shows a blank).
fn prompt_of(call: &ToolCall) -> String {
    serde_json::from_str::<serde_json::Value>(&call.arguments)
        .ok()
        .and_then(|v| v.get("prompt").and_then(|p| p.as_str()).map(str::to_string))
        .unwrap_or_default()
}

/// A distinguishing one-line slice of a prompt: whitespace flattened, then
/// head + ellipsis + tail when it is long, so agents that share a prefix (a
/// common instruction or URL base) stay visibly distinct by their tails.
fn prompt_slice(prompt: &str) -> String {
    let flat = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let count = flat.chars().count();
    if count <= HEAD + TAIL + 1 {
        return flat;
    }
    let head: String = flat.chars().take(HEAD).collect();
    let tail: String = flat.chars().skip(count - TAIL).collect();
    format!("{head}…{tail}")
}

/// `(M turns)` lifted out of a `done (M turns)` summary, if present.
fn turns_of(summary: &str) -> Option<String> {
    let open = summary.find('(')?;
    let close = summary[open..].find(')')? + open;
    Some(summary[open..=close].to_string())
}

/// The first non-empty line of a sub-agent's result, flattened and clipped to
/// one row. None when the result is empty (skip the digest entirely).
fn digest_of(content: &str) -> Option<String> {
    let line = content.lines().map(str::trim).find(|l| !l.is_empty())?;
    let flat = line.replace('\r', " ");
    Some(super::clip(&flat, DIGEST))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn task(id: &str, prompt: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "subagent".into(),
            arguments: json!({ "prompt": prompt }).to_string(),
        }
    }

    fn other(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: "{}".into(),
        }
    }

    #[test]
    fn a_fanout_group_renders_distinct_rows_a_transition_and_the_cap() {
        let calls = vec![
            task(
                "f1",
                "Fetch and read the article at https://example.com/a/alpha-1111",
            ),
            task(
                "f2",
                "Fetch and read the article at https://example.com/a/beta-2222",
            ),
            task(
                "f3",
                "Fetch and read the article at https://example.com/a/gamma-3333",
            ),
        ];
        let mut panels = Panels::build(&calls, 4);

        // Group start: one all-running block, header carries the concurrency cap.
        let open = panels.on_started(0).expect("first start opens the panel");
        assert_eq!(open.ids, vec!["f1", "f2", "f3"]);
        assert!(
            open.block
                .starts_with("agents (0/3 done, up to 4 at once):"),
            "{}",
            open.block
        );
        assert_eq!(
            open.block.lines().filter(|l| l.starts_with("[~]")).count(),
            3
        );
        // Later starts do not re-render (nothing changed).
        assert!(panels.on_started(1).is_none());
        assert!(panels.on_started(2).is_none());

        // Distinct rows: the shared prefix does not collapse them; the tails differ.
        for tail in ["alpha-1111", "beta-2222", "gamma-3333"] {
            assert!(
                open.block.contains(tail),
                "row for {tail} not distinct:\n{}",
                open.block
            );
        }

        // Running -> done transition with turns and a one-line digest.
        let done = panels
            .on_finished(
                1,
                "task done (7 turns)",
                "Beta finished.\nmore detail",
                false,
                false,
                Some(Duration::from_millis(3200)),
            )
            .expect("finish re-renders");
        assert!(
            done.block
                .starts_with("agents (1/3 done, up to 4 at once):"),
            "{}",
            done.block
        );
        assert!(
            done.block.contains("[x] agent 2:"),
            "row 2 not marked done:\n{}",
            done.block
        );
        assert!(
            done.block.contains("(7 turns)"),
            "turns missing:\n{}",
            done.block
        );
        assert!(
            done.block.contains("3.2s"),
            "elapsed time missing:\n{}",
            done.block
        );
        assert!(
            done.block.contains("agent 2: Beta finished."),
            "digest missing:\n{}",
            done.block
        );
        // The other two stay running.
        assert_eq!(
            done.block.lines().filter(|l| l.starts_with("[~]")).count(),
            2
        );
    }

    #[test]
    fn identical_prompts_stay_distinguishable_by_ordinal() {
        let calls = vec![task("a", "do the thing"), task("b", "do the thing")];
        let mut panels = Panels::build(&calls, 2);
        let open = panels.on_started(0).unwrap();
        assert!(open.block.contains("agent 1: do the thing"));
        assert!(open.block.contains("agent 2: do the thing"));
    }

    #[test]
    fn detached_fanout_waits_for_ack_and_uses_the_global_active_count() {
        let calls = vec![task("a", "alpha"), task("b", "beta"), task("c", "gamma")];
        let mut panels = Panels::build_background(&calls, 2);
        assert!(
            panels.on_started(0).is_none(),
            "a start is not proof that the hub admitted a background job"
        );
        let done = panels
            .on_finished(
                0,
                "agent-1 started · 7 active",
                r#"{"job_id":"agent-1","status":"running"}"#,
                false,
                false,
                None,
            )
            .unwrap();
        assert_eq!(done.block.lines().count(), 1);
        assert_eq!(done.block, "[7] agents running (Tab to view)");
        assert_eq!(
            done.ids,
            vec!["a".to_string()],
            "only an admitted call may be covered"
        );
    }

    #[test]
    fn detached_failures_and_cancellations_do_not_open_a_phantom_panel() {
        let calls = vec![task("bad", "invalid"), task("stop", "canceled")];
        let mut panels = Panels::build_background(&calls, 2);
        assert!(panels.on_started(0).is_none());
        assert!(
            panels
                .on_finished(
                    0,
                    "parameter error",
                    "missing required parameter",
                    true,
                    false,
                    None,
                )
                .is_none(),
            "a failed submission must leave its normal diagnostic visible"
        );
        assert!(
            panels
                .on_finished(1, "canceled", "canceled by user", true, true, None)
                .is_none(),
            "a canceled submission never became a running job"
        );
    }

    #[test]
    fn detached_success_requires_a_well_formed_matching_acknowledgment() {
        let calls = vec![task("a", "alpha")];
        let mut panels = Panels::build_background(&calls, 2);
        for (summary, content) in [
            ("agent-1 started · 1 active", "not json"),
            (
                "agent-1 started · 1 active",
                r#"{"job_id":"agent-1","status":"done"}"#,
            ),
            (
                "agent-2 started · 1 active",
                r#"{"job_id":"agent-1","status":"running"}"#,
            ),
            (
                "agent-1 started · 0 active",
                r#"{"job_id":"agent-1","status":"running"}"#,
            ),
        ] {
            assert!(
                panels
                    .on_finished(0, summary, content, false, false, None)
                    .is_none(),
                "invalid acknowledgment {summary:?} / {content:?} opened a panel"
            );
        }
    }

    #[test]
    fn out_of_order_detached_acknowledgments_do_not_regress_the_global_count() {
        let calls = vec![task("a", "alpha"), task("b", "beta")];
        let mut panels = Panels::build_background(&calls, 2);
        let newer = panels
            .on_finished(
                1,
                "agent-2 started · 5 active",
                r#"{"job_id":"agent-2","status":"running"}"#,
                false,
                false,
                None,
            )
            .unwrap();
        assert_eq!(newer.block, "[5] agents running (Tab to view)");

        let older = panels
            .on_finished(
                0,
                "agent-1 started · 4 active",
                r#"{"job_id":"agent-1","status":"running"}"#,
                false,
                false,
                None,
            )
            .unwrap();
        assert_eq!(older.block, "[5] agents running (Tab to view)");
        assert_eq!(older.ids, vec!["a".to_string()]);
    }

    #[test]
    fn a_lone_task_is_not_a_panel() {
        let calls = vec![other("r", "read"), task("t", "solo"), other("b", "bash")];
        let mut panels = Panels::build(&calls, 4);
        assert!(
            panels.on_started(1).is_none(),
            "a single task must not open a panel"
        );
        assert!(
            panels
                .on_finished(1, "task done (1 turns)", "x", false, false, None)
                .is_none()
        );
    }

    #[test]
    fn only_consecutive_task_runs_group() {
        // Two tasks, a barrier, then two more: two separate panels.
        let calls = vec![
            task("f1", "one"),
            task("f2", "two"),
            other("m", "bash"),
            task("f3", "three"),
            task("f4", "four"),
        ];
        let mut panels = Panels::build(&calls, 3);
        let first = panels.on_started(0).unwrap();
        assert_eq!(first.ids, vec!["f1", "f2"]);
        let second = panels.on_started(3).unwrap();
        assert_eq!(second.ids, vec!["f3", "f4"]);
    }

    #[test]
    fn a_canned_finish_before_any_start_still_opens_the_panel() {
        let calls = vec![task("a", "alpha"), task("b", "beta")];
        let mut panels = Panels::build(&calls, 2);
        // A doom-loop / parse intercept finishes without ever starting.
        let r = panels
            .on_finished(0, "error", "repeated identical call", true, false, None)
            .expect("a finish opens the panel even with no prior start");
        assert!(r.block.contains("[!] agent 1:"));
        assert!(r.block.contains("agent 1: repeated identical call"));
        // An errored agent shows no turn count.
        assert!(
            !r.block.contains("turns)"),
            "error row must not claim a turn count:\n{}",
            r.block
        );
    }

    #[test]
    fn an_empty_result_skips_the_digest() {
        let calls = vec![task("a", "alpha"), task("b", "beta")];
        let mut panels = Panels::build(&calls, 2);
        let r = panels
            .on_finished(0, "task done (2 turns)", "   \n  ", false, false, None)
            .unwrap();
        assert!(r.block.contains("[x] agent 1: alpha  (2 turns)"));
        assert!(
            !r.block.contains(" · "),
            "an empty result must not add a digest:\n{}",
            r.block
        );
    }

    #[test]
    fn cancellation_is_terminal_without_a_failure_glyph() {
        let calls = vec![task("a", "alpha"), task("b", "beta")];
        let mut panels = Panels::build(&calls, 2);
        let rendered = panels
            .on_finished(0, "canceled", "canceled by user", true, true, None)
            .unwrap();
        assert!(rendered.block.contains("[-] agent 1: canceled by user"));
        assert!(!rendered.block.contains("[!] agent 1"));
    }

    #[test]
    fn a_shared_prefix_is_sliced_head_and_tail() {
        let long = format!("{}TAILMARK", "x".repeat(80));
        assert!(prompt_slice(&long).contains('…'));
        assert!(
            prompt_slice(&long).ends_with("TAILMARK"),
            "the distinguishing tail must survive"
        );
    }
}
