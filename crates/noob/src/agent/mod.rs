//! The turn machine: build request -> stream events -> render -> execute
//! tool calls -> append results -> repeat until a turn ends with no tool
//! calls or a breaker trips. Owns the transcript, the doom-loop breakers,
//! interrupt semantics, and compaction triggers.

pub mod compact;
pub mod prompt;
pub mod sched;

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use noob_provider::http::{Client, INTERRUPTED};
use noob_provider::types::{
    Event, Finish, Item, Overrides, ProviderError, ToolCall, ToolSpec, TurnRequest, Usage,
};

use crate::session::Session;
use crate::tools::guard::{self, fnv1a64};
use crate::tools::{self, ToolCtx, ToolOutcome};
use crate::ui::Ui;

/// Locked breaker thresholds (ARCHITECTURE.md, agent loop).
const TURN_CAP: u32 = 50;
const DOOM_WINDOW: usize = 12;
const DOOM_REPEATS: usize = 3;
const NUDGE_AT: u32 = 4;
const PAUSE_AT: u32 = 8;

/// The plan-mode tool set (ARCHITECTURE.md, plan mode): read-only
/// exploration plus skills. Everything else is structurally absent from the
/// request, so it cannot tempt a small model into rejected round trips.
const PLAN_TOOLS: &[&str] = &["read", "grep", "glob", "ls", "skill"];

/// The injected user-role mode message (frozen phrasing; e2e-asserted).
pub const PLAN_ENTER_MSG: &str =
    "[plan mode] Explore read-only, then present a numbered implementation plan.";
/// What /go appends when the user approves (frozen phrasing).
pub const PLAN_APPROVED_MSG: &str = "Plan approved. Execute it.";

pub struct Agent {
    pub client: Client,
    pub config_dir: PathBuf,
    pub ov: Overrides,
    /// Frozen at session start; every request sends exactly this head.
    pub system: String,
    /// The active tool set. Byte-stable for the whole session except the
    /// two sanctioned plan-mode swaps (entry filters, /go restores).
    pub tools: Vec<ToolSpec>,
    /// The full registered set, kept for the /go restore.
    full_tools: Vec<ToolSpec>,
    /// Plan mode: read-only exploration until the user approves with /go.
    pub plan: bool,
    pub items: Vec<Item>,
    pub tool_ctx: ToolCtx,
    pub session: Option<Session>,
    /// NOOB_CTX: the context window compaction budgets against.
    pub ctx_tokens: u64,
    /// chars/4 stand-in for the fixed head when no usage has arrived yet.
    fixed_chars: usize,
    last_usage: Option<Usage>,
    chars_since_usage: usize,
    recent_calls: VecDeque<u64>,
    consec_errors: u32,
}

pub enum RunEnd {
    /// The model finished with plain text. The text has already streamed to
    /// the UI; the carried value is for surfaces that need it whole (the
    /// P6 child returns it as its single JSON result line).
    Completed(#[allow(dead_code)] String),
    /// Ctrl-C; the transcript is valid and the session can continue.
    Interrupted,
    /// A breaker or provider error stopped the run; message states why.
    Aborted(String),
}

impl Agent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: Client,
        config_dir: PathBuf,
        ov: Overrides,
        system: String,
        tools: Vec<ToolSpec>,
        items: Vec<Item>,
        tool_ctx: ToolCtx,
        session: Option<Session>,
        ctx_tokens: u64,
    ) -> Agent {
        let fixed_chars = system.len()
            + tools
                .iter()
                .map(|t| t.name.len() + t.description.len() + t.parameters.to_string().len())
                .sum::<usize>();
        let replayed_chars: usize = items.iter().map(compact::item_chars).sum();
        recover_loaded_skills(&tool_ctx, &items);
        Agent {
            client,
            config_dir,
            ov,
            system,
            full_tools: tools.clone(),
            tools,
            plan: false,
            items,
            tool_ctx,
            session,
            ctx_tokens,
            fixed_chars,
            last_usage: None,
            chars_since_usage: replayed_chars,
            recent_calls: VecDeque::new(),
            consec_errors: 0,
        }
    }

    pub fn last_usage(&self) -> Option<Usage> {
        self.last_usage
    }

    /// Enter plan mode: the tools array shrinks to the read-only set (one
    /// sanctioned cache bust) and a user-role mode message is appended (an
    /// ordinary append; the message prefix itself stays intact). False when
    /// already in plan mode.
    pub fn enter_plan(&mut self, ui: &mut Ui) -> bool {
        if self.plan {
            return false;
        }
        self.plan = true;
        self.tools = self
            .full_tools
            .iter()
            .filter(|t| PLAN_TOOLS.contains(&t.name.as_str()))
            .cloned()
            .collect();
        self.push_item(Item::User(PLAN_ENTER_MSG.to_string()));
        ui.note("cache prefix reset: plan mode (read-only tools; approve with /go)");
        true
    }

    /// Leave plan mode: the full tool set comes back (the second sanctioned
    /// bust). The caller follows up with `run_input(PLAN_APPROVED_MSG)`.
    /// False when not in plan mode.
    pub fn exit_plan(&mut self, ui: &mut Ui) -> bool {
        if !self.plan {
            return false;
        }
        self.plan = false;
        self.tools = self.full_tools.clone();
        ui.note("cache prefix reset: plan approved (full tools restored)");
        true
    }

    /// Estimated tokens currently in the context: the last server-reported
    /// usage plus chars/4 for everything appended since.
    pub fn context_estimate(&self) -> u64 {
        let base = match self.last_usage {
            Some(u) => u.prompt_tokens + u.completion_tokens,
            None => (self.fixed_chars / 4) as u64,
        };
        base + (self.chars_since_usage / 4) as u64
    }

    fn push_item(&mut self, item: Item) {
        self.chars_since_usage += compact::item_chars(&item);
        if let Some(s) = &mut self.session {
            s.log_item(&item);
        }
        self.items.push(item);
    }

    /// Process one user input to completion (or breaker / interrupt).
    pub fn run_input(&mut self, input: &str, ui: &mut Ui) -> RunEnd {
        self.push_item(Item::User(input.to_string()));
        self.consec_errors = 0;
        let mut emergency_used = false;

        for _round in 0..TURN_CAP {
            if self.context_estimate() >= self.ctx_tokens.saturating_mul(3) / 4 {
                self.compact(ui);
                // Ctrl-C during the summarization request aborts the input,
                // not just the compaction.
                if INTERRUPTED.load(Ordering::SeqCst) {
                    return self.finish_interrupt(ui, &[]);
                }
            }
            let req = TurnRequest {
                system: Some(self.system.clone()),
                items: self.items.clone(),
                tools: self.tools.clone(),
            };
            let result = noob_provider::run_turn(
                &self.client,
                &self.config_dir,
                &self.ov,
                &req,
                &mut |ev| match ev {
                    Event::Text(t) => ui.text_delta(&t),
                    Event::Reasoning(r) => ui.reasoning_delta(&r),
                    _ => {}
                },
            );

            let turn = match result {
                Err(ProviderError::Interrupted) => {
                    return self.finish_interrupt(ui, &[]);
                }
                Err(ProviderError::Http { status: 400, ref body })
                    if !emergency_used && looks_like_context_overflow(body) =>
                {
                    emergency_used = true;
                    ui.note("the endpoint reports a full context; compacting and retrying");
                    let compacted = self.compact(ui);
                    if INTERRUPTED.load(Ordering::SeqCst) {
                        return self.finish_interrupt(ui, &[]);
                    }
                    if !compacted {
                        return RunEnd::Aborted(
                            "the context window is full and nothing is left to compact; \
                             start a new session"
                                .to_string(),
                        );
                    }
                    continue;
                }
                Err(e) => {
                    ui.end_line();
                    return RunEnd::Aborted(e.to_string());
                }
                Ok(turn) => turn,
            };

            // Output is never capped, so Length means the context filled up
            // mid-turn: discard the partial turn, compact once, retry.
            if turn.finish == Finish::Length && !emergency_used {
                emergency_used = true;
                ui.end_line();
                ui.note("the model hit the end of the context mid-turn; compacting and retrying");
                let compacted = self.compact(ui);
                if INTERRUPTED.load(Ordering::SeqCst) {
                    return self.finish_interrupt(ui, &[]);
                }
                if !compacted {
                    return RunEnd::Aborted(
                        "the context window is full and nothing is left to compact; \
                         start a new session"
                            .to_string(),
                    );
                }
                continue;
            }
            ui.end_line();
            self.push_item(Item::Assistant {
                text: turn.text.clone(),
                tool_calls: turn.tool_calls.clone(),
                raw_items: turn.raw_items.clone(),
            });
            // Server-reported usage covers the prompt AND this turn's reply,
            // so it lands AFTER the assistant item is pushed: adding the
            // item's chars on top would double-count the reply and trigger
            // compaction below the 75% threshold.
            if let Some(u) = turn.usage {
                self.last_usage = Some(u);
                self.chars_since_usage = 0;
            }

            if turn.tool_calls.is_empty() {
                // A Ctrl-C that landed between the last token and here (the
                // stream tail and drain are a real window) has nothing left
                // to cancel; consume it or it phantom-cancels the next input
                // and a second press hard-exits the REPL.
                INTERRUPTED.store(false, Ordering::SeqCst);
                ui.done(self.last_usage);
                return RunEnd::Completed(turn.text);
            }
            // Ctrl-C landed between stream end and execution: every parsed
            // call gets a synthetic result so the transcript stays API-valid.
            if INTERRUPTED.load(Ordering::SeqCst) {
                return self.finish_interrupt(ui, &turn.tool_calls);
            }

            // Plan the batch: doom-loop intercepts and argument parsing
            // happen up front, in emission order.
            let mut batch = Vec::new();
            for call in &turn.tool_calls {
                let (planned, shown_args) = self.plan_call(call);
                let planned = self.gate_skills_write(planned, ui);
                ui.tool_start(&call.name, &shown_args, tools::is_read_only(&call.name));
                batch.push(planned);
            }
            let outcomes = sched::run_batch(&self.tool_ctx, batch);

            let mut nudge = false;
            for (call, outcome) in turn.tool_calls.iter().zip(&outcomes) {
                if let Some(w) = &outcome.warning {
                    ui.note(w);
                }
                ui.tool_done(&call.id, &outcome.summary, outcome.is_error);
                // A canceled call never executed: drop its doom-window
                // record so an immediate retry after the interrupt is not
                // intercepted as a repeat that "will not change". Keyed on
                // the scheduler's structural flag, not the content string,
                // so a tool cannot forge a cancellation by echoing it.
                if outcome.canceled {
                    self.forget_recent_call(call);
                }
                self.push_item(Item::ToolResult {
                    call_id: call.id.clone(),
                    content: outcome.content.clone(),
                });
                if outcome.is_error {
                    self.consec_errors += 1;
                    if self.consec_errors == NUDGE_AT {
                        nudge = true;
                    }
                } else {
                    self.consec_errors = 0;
                }
            }
            if INTERRUPTED.load(Ordering::SeqCst) {
                return self.finish_interrupt(ui, &[]);
            }
            if self.consec_errors >= PAUSE_AT {
                let last = outcomes
                    .iter()
                    .rev()
                    .find(|o| o.is_error)
                    .map(|o| clip(&o.content, 200))
                    .unwrap_or_default();
                let keep_going = ui.ask(&format!(
                    "{} tool calls in a row failed; keep going?",
                    self.consec_errors
                ));
                // A Ctrl-C pressed while the ask blocked on stdin has been
                // superseded by the explicit answer; a stale flag would
                // either kill the next request after a "y" or phantom-
                // cancel the next REPL input after a "n".
                INTERRUPTED.store(false, Ordering::SeqCst);
                if keep_going {
                    self.consec_errors = 0;
                } else {
                    return RunEnd::Aborted(format!(
                        "stopped after {} consecutive tool errors; last error: {last}",
                        self.consec_errors
                    ));
                }
            } else if nudge {
                self.push_item(Item::User(
                    "[note] the last four tool calls all failed; step back and reconsider: \
                     re-read the file or take a different approach"
                        .to_string(),
                ));
            }
        }
        RunEnd::Aborted(format!(
            "reached the {TURN_CAP}-round cap for one input; the task may be stuck; \
             give a narrower instruction to continue"
        ))
    }

    /// Common interrupt epilogue: synthetic results for any unexecuted
    /// calls, an `[interrupted]` user note, and a cleared flag so the next
    /// input starts fresh.
    fn finish_interrupt(&mut self, ui: &mut Ui, pending_calls: &[ToolCall]) -> RunEnd {
        INTERRUPTED.store(false, Ordering::SeqCst);
        ui.end_line();
        for call in pending_calls {
            self.push_item(Item::ToolResult {
                call_id: call.id.clone(),
                content: "canceled by user".to_string(),
            });
        }
        self.push_item(Item::User("[interrupted]".to_string()));
        ui.note("[interrupted]");
        RunEnd::Interrupted
    }

    /// Skills are loaded, never authored by the agent: a write/edit whose
    /// path lands inside any skills directory needs explicit confirmation
    /// in every mode, container included (agent-created skills are
    /// persistent injection vectors). Headless surfaces degrade to deny.
    fn gate_skills_write(&self, planned: sched::Planned, ui: &mut Ui) -> sched::Planned {
        let sched::Planned::Run { name, args } = &planned else {
            return planned;
        };
        if !matches!(name.as_str(), "write" | "edit") {
            return planned;
        }
        let Some(raw) = args.get("path").and_then(Value::as_str) else {
            return planned; // the tool itself reports the missing parameter
        };
        // Same key the write/edit tools re-check at execution time: the
        // filesystem-real target when it lands in a skills dir (catching a
        // symlinked route), else the lexical form.
        let Some(target) = guard::skill_write_target(&self.tool_ctx.workspace, raw) else {
            return planned;
        };
        if ui.ask(&format!("allow the agent to {name} inside a skills directory ({raw})?")) {
            // Record this exact target so the tool's execution-time re-check
            // passes; other paths (and a mid-batch symlink target) stay
            // unapproved and are refused there.
            self.tool_ctx
                .approved_skill_writes
                .lock()
                .unwrap()
                .insert(target);
            return planned;
        }
        sched::Planned::Canned(ToolOutcome::err(
            "refused: writing into a skills directory needs the user's confirmation \
             and it was not granted; leave skill files unchanged and continue \
             without them"
                .to_string(),
        ))
    }

    /// Remove the most recent doom-window record for `call`, mirroring
    /// plan_call's canonicalization (only object/null args ever got hashed).
    fn forget_recent_call(&mut self, call: &ToolCall) {
        let args = match serde_json::from_str::<Value>(&call.arguments) {
            Ok(v @ Value::Object(_)) => v,
            Ok(Value::Null) => json!({}),
            _ => return,
        };
        let canonical = format!("{}\u{0}{}", call.name, args);
        let hash = fnv1a64(canonical.as_bytes());
        if let Some(pos) = self.recent_calls.iter().rposition(|&h| h == hash) {
            self.recent_calls.remove(pos);
        }
    }

    /// Doom-loop guard + argument parsing. Returns what to execute plus the
    /// parsed args for display.
    fn plan_call(&mut self, call: &ToolCall) -> (sched::Planned, Value) {
        let args = match serde_json::from_str::<Value>(&call.arguments) {
            Ok(v @ Value::Object(_)) => v,
            Ok(Value::Null) => json!({}),
            Ok(other) => {
                return (
                    sched::Planned::Canned(ToolOutcome::err(format!(
                        "arguments must be a JSON object, got {other}; resend the call"
                    ))),
                    json!({}),
                );
            }
            Err(e) => {
                return (
                    sched::Planned::Canned(ToolOutcome::err(format!(
                        "arguments were not valid JSON ({e}); resend the call \
                         with a JSON object"
                    ))),
                    json!({}),
                );
            }
        };
        // Defense in depth behind the structural gate: even a hallucinated
        // call to an absent schema is refused while planning.
        if self.plan && !PLAN_TOOLS.contains(&call.name.as_str()) {
            return (
                sched::Planned::Canned(ToolOutcome::err(format!(
                    "plan mode is read-only: {} is unavailable; present your plan \
                     as text and wait for the user to approve it",
                    call.name
                ))),
                args,
            );
        }
        // serde_json's default map is sorted, so this serialization is
        // canonical: key order cannot dodge the repeat detector.
        let canonical = format!("{}\u{0}{}", call.name, args);
        let hash = fnv1a64(canonical.as_bytes());
        // "3 times within the last 12 calls": the window includes the
        // current call, so look at the 11 preceding ones.
        let repeats = self
            .recent_calls
            .iter()
            .rev()
            .take(DOOM_WINDOW - 1)
            .filter(|&&h| h == hash)
            .count();
        self.recent_calls.push_back(hash);
        if self.recent_calls.len() > DOOM_WINDOW {
            self.recent_calls.pop_front();
        }
        if repeats + 1 >= DOOM_REPEATS {
            return (
                sched::Planned::Canned(ToolOutcome::err(
                    "repeated identical call; the result will not change; \
                     take a different approach"
                        .to_string(),
                )),
                args,
            );
        }
        (
            sched::Planned::Run { name: call.name.clone(), args: args.clone() },
            args,
        )
    }
}

/// The post-compaction re-listing must survive resume. Two sources, both
/// needed: (a) successful skill tool results still in the transcript,
/// paired by call id (errored or canceled calls never delivered a body and
/// must not count); (b) `[loaded skills: ...]` lines carried by a spliced
/// compaction summary, because compaction removes the calls themselves.
/// Names missing from the current registry are dropped.
fn recover_loaded_skills(tool_ctx: &ToolCtx, items: &[Item]) {
    let mut pending: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut recovered: Vec<String> = Vec::new();
    for item in items {
        match item {
            Item::Assistant { tool_calls, .. } => {
                for call in tool_calls.iter().filter(|c| c.name == "skill") {
                    let name = serde_json::from_str::<Value>(&call.arguments)
                        .ok()
                        .and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_string));
                    if let Some(name) = name {
                        pending.insert(call.id.clone(), name);
                    }
                }
            }
            Item::ToolResult { call_id, content } => {
                if let Some(name) = pending.remove(call_id) {
                    // Only a delivered body counts; a success opens with the
                    // frozen "skill: {name}" header.
                    if content.starts_with(&format!("skill: {name}\n")) {
                        recovered.push(name);
                    }
                }
            }
            Item::User(text) => {
                for line in text.lines() {
                    if let Some(inner) = line
                        .strip_prefix("[loaded skills: ")
                        .and_then(|rest| rest.strip_suffix(']'))
                    {
                        recovered.extend(inner.split(", ").map(str::to_string));
                    }
                }
            }
        }
    }
    let mut loaded = tool_ctx.loaded_skills.lock().unwrap();
    for name in recovered {
        if tool_ctx.skills.iter().any(|s| s.name == name) && !loaded.iter().any(|n| *n == name) {
            loaded.push(name);
        }
    }
}

pub fn looks_like_context_overflow(body: &str) -> bool {
    let b = body.to_ascii_lowercase();
    b.contains("context")
        && ["exceed", "maximum", "too long", "overflow", "size", "length"]
            .iter()
            .any(|w| b.contains(w))
}

fn clip(s: &str, chars: usize) -> String {
    if s.chars().count() <= chars {
        return s.to_string();
    }
    let cut: String = s.chars().take(chars).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use noob_provider::http::Timeouts;

    #[test]
    fn doom_window_is_twelve_calls_inclusive_of_the_current_one() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        let mut agent = Agent::new(
            Client::new(Timeouts::default()),
            tmp.path().to_path_buf(),
            Overrides::default(),
            "sys".into(),
            vec![],
            vec![],
            ToolCtx::new(ws, crate::tools::guard::Sandbox::Container),
            None,
            131_072,
        );
        let call = |args: &str| ToolCall {
            id: "c".into(),
            name: "read".into(),
            arguments: args.into(),
        };
        let x = call(r#"{"path":"x"}"#);
        let ran = |p: &sched::Planned| matches!(p, sched::Planned::Run { .. });

        // X, X, then 10 distinct calls: the next X spans 13 calls, OUTSIDE
        // the 12-call window, so it must run.
        assert!(ran(&agent.plan_call(&x).0));
        assert!(ran(&agent.plan_call(&x).0));
        for i in 0..10 {
            let d = call(&format!(r#"{{"path":"d{i}"}}"#));
            assert!(ran(&agent.plan_call(&d).0));
        }
        assert!(ran(&agent.plan_call(&x).0), "a 13-call span must not intercept");
        // Now two X sit close together; a third within the window trips.
        assert!(ran(&agent.plan_call(&x).0));
        match agent.plan_call(&x).0 {
            sched::Planned::Canned(out) => {
                assert!(out.content.contains("repeated identical call"));
            }
            sched::Planned::Run { .. } => panic!("third X within 12 must intercept"),
        }
    }

    fn test_agent(items: Vec<Item>, tool_ctx: ToolCtx, config: &std::path::Path) -> Agent {
        Agent::new(
            Client::new(Timeouts::default()),
            config.to_path_buf(),
            Overrides::default(),
            "sys".into(),
            vec![],
            items,
            tool_ctx,
            None,
            131_072,
        )
    }

    fn skill_ctx(tmp: &std::path::Path, names: &[&str]) -> ToolCtx {
        let ws = tmp.canonicalize().unwrap();
        let mut tool_ctx = ToolCtx::new(ws, crate::tools::guard::Sandbox::Container);
        for name in names {
            tool_ctx.skills.push(crate::skills::Skill {
                name: name.to_string(),
                description: "d".into(),
                dir: tmp.join(name),
                file: tmp.join(name).join("SKILL.md"),
            });
        }
        tool_ctx
    }

    fn skill_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "skill".into(),
            arguments: format!(r#"{{"name":"{name}"}}"#),
        }
    }

    #[test]
    fn resume_scan_counts_only_successful_loads_of_existing_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let tool_ctx = skill_ctx(tmp.path(), &["alpha", "beta", "gamma"]);
        let items = vec![
            Item::User("hi".into()),
            Item::Assistant {
                text: String::new(),
                tool_calls: vec![
                    skill_call("1", "alpha"), // delivered
                    skill_call("2", "ghost"), // no longer exists
                    skill_call("3", "beta"),  // canceled mid-batch
                    skill_call("4", "gamma"), // errored (file unreadable)
                ],
                raw_items: vec![],
            },
            Item::ToolResult {
                call_id: "1".into(),
                content: "skill: alpha\ndir: alpha\n\nbody".into(),
            },
            Item::ToolResult { call_id: "2".into(), content: "unknown skill \"ghost\"".into() },
            Item::ToolResult { call_id: "3".into(), content: "canceled by user".into() },
            Item::ToolResult {
                call_id: "4".into(),
                content: "cannot read skill \"gamma\" at gamma/SKILL.md: gone".into(),
            },
        ];
        let agent = test_agent(items, tool_ctx, tmp.path());
        assert_eq!(
            *agent.tool_ctx.loaded_skills.lock().unwrap(),
            vec!["alpha".to_string()],
            "only the delivered body counts"
        );
    }

    #[test]
    fn resume_scan_recovers_names_from_a_spliced_compaction_summary() {
        // After compaction the skill CALLS are gone; the only trace is the
        // [loaded skills: ...] line inside the spliced summary message.
        let tmp = tempfile::tempdir().unwrap();
        let tool_ctx = skill_ctx(tmp.path(), &["alpha", "beta"]);
        let items = vec![
            Item::User(
                "[conversation summary]\nwork happened\n[loaded skills: alpha, beta, ghost]"
                    .into(),
            ),
            Item::User("continue".into()),
        ];
        let agent = test_agent(items, tool_ctx, tmp.path());
        assert_eq!(
            *agent.tool_ctx.loaded_skills.lock().unwrap(),
            vec!["alpha".to_string(), "beta".to_string()],
            "summary-line names recovered, unknown names dropped"
        );
    }

    #[test]
    fn skills_write_gate_denies_headless_and_ignores_other_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        let agent = test_agent(vec![], ToolCtx::new(ws, crate::tools::guard::Sandbox::Container), tmp.path());
        let mut ui = crate::ui::Ui::new(crate::ui::Mode::Exec); // headless: ask degrades to deny
        let run = |name: &str, path: &str| sched::Planned::Run {
            name: name.into(),
            args: json!({"path": path, "content": "x"}),
        };

        match agent.gate_skills_write(run("write", ".claude/skills/evil/SKILL.md"), &mut ui) {
            sched::Planned::Canned(out) => {
                assert!(out.is_error);
                assert!(out.content.contains("refused"), "{}", out.content);
                assert!(out.content.contains("confirmation"));
            }
            sched::Planned::Run { .. } => panic!("headless skills write must be denied"),
        }
        // Relative traversal into a skills dir is still gated.
        match agent.gate_skills_write(run("edit", "src/../.noob/skills/x/SKILL.md"), &mut ui) {
            sched::Planned::Canned(_) => {}
            sched::Planned::Run { .. } => panic!("traversal into skills must be gated"),
        }
        // Ordinary paths and read-only tools pass untouched.
        assert!(matches!(
            agent.gate_skills_write(run("write", "src/main.rs"), &mut ui),
            sched::Planned::Run { .. }
        ));
        assert!(matches!(
            agent.gate_skills_write(run("read", ".claude/skills/evil/SKILL.md"), &mut ui),
            sched::Planned::Run { .. }
        ));
    }

    #[test]
    fn skills_write_gate_lets_a_granted_confirmation_through() {
        // The spec is "requires confirmation", not "always refuses": a
        // granted ask must let the write run (driven via the test seam;
        // the real tty path is covered by the e2e under a pty).
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        let agent =
            test_agent(vec![], ToolCtx::new(ws, crate::tools::guard::Sandbox::Container), tmp.path());
        let mut ui = crate::ui::Ui::new(crate::ui::Mode::Repl);
        ui.forced_ask = Some(true);
        let planned = sched::Planned::Run {
            name: "write".into(),
            args: json!({"path": ".claude/skills/x/SKILL.md", "content": "ok"}),
        };
        assert!(matches!(
            agent.gate_skills_write(planned, &mut ui),
            sched::Planned::Run { .. }
        ));
        ui.forced_ask = Some(false);
        let planned = sched::Planned::Run {
            name: "write".into(),
            args: json!({"path": ".claude/skills/x/SKILL.md", "content": "no"}),
        };
        assert!(matches!(
            agent.gate_skills_write(planned, &mut ui),
            sched::Planned::Canned(_)
        ));
    }

    #[test]
    fn gate_catches_writes_routed_through_a_symlink_into_skills() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        let target = ws.join(".claude/skills/pdf");
        std::fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, ws.join("innocent")).unwrap();
        let agent = test_agent(
            vec![],
            ToolCtx::new(ws, crate::tools::guard::Sandbox::Container),
            tmp.path(),
        );
        let mut ui = crate::ui::Ui::new(crate::ui::Mode::Exec);
        let planned = sched::Planned::Run {
            name: "write".into(),
            args: json!({"path": "innocent/SKILL.md", "content": "x"}),
        };
        match agent.gate_skills_write(planned, &mut ui) {
            sched::Planned::Canned(out) => assert!(out.content.contains("refused")),
            sched::Planned::Run { .. } => panic!("symlink route into skills must be gated"),
        }
    }

    #[test]
    fn canceled_calls_are_forgotten_by_the_doom_window() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        let mut agent = test_agent(
            vec![],
            ToolCtx::new(ws, crate::tools::guard::Sandbox::Container),
            tmp.path(),
        );
        let call = ToolCall {
            id: "c".into(),
            name: "bash".into(),
            arguments: r#"{"cmd":"make"}"#.into(),
        };
        // Planned twice (one canceled), so only ONE record must remain:
        // the third plan is the second real attempt and must run.
        agent.plan_call(&call);
        agent.plan_call(&call);
        agent.forget_recent_call(&call);
        match agent.plan_call(&call).0 {
            sched::Planned::Run { .. } => {}
            sched::Planned::Canned(_) => panic!("retry after a canceled call was intercepted"),
        }
    }

    #[test]
    fn plan_mode_swaps_tool_sets_and_appends_the_mode_message() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        let spec = |name: &str| ToolSpec {
            name: name.into(),
            description: "d".into(),
            parameters: json!({"type": "object"}),
        };
        let full: Vec<ToolSpec> =
            ["read", "grep", "glob", "ls", "skill", "write", "edit", "bash", "mcp_call"]
                .iter()
                .map(|n| spec(n))
                .collect();
        let mut agent = Agent::new(
            Client::new(Timeouts::default()),
            tmp.path().to_path_buf(),
            Overrides::default(),
            "sys".into(),
            full.clone(),
            vec![],
            ToolCtx::new(ws, crate::tools::guard::Sandbox::Container),
            None,
            131_072,
        );
        let mut ui = crate::ui::Ui::new(crate::ui::Mode::Exec);

        assert!(agent.enter_plan(&mut ui));
        assert!(!agent.enter_plan(&mut ui), "double entry must be a no-op");
        let names: Vec<&str> = agent.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["read", "grep", "glob", "ls", "skill"]);
        assert!(matches!(&agent.items[..], [Item::User(m)] if m == PLAN_ENTER_MSG));

        // Defense in depth: a hallucinated mutating call is refused even
        // though its schema is absent.
        let call = ToolCall { id: "c".into(), name: "write".into(),
            arguments: r#"{"path":"x","content":"y"}"#.into() };
        match agent.plan_call(&call).0 {
            sched::Planned::Canned(out) => {
                assert!(out.is_error);
                assert!(out.content.contains("plan mode is read-only"), "{}", out.content);
            }
            sched::Planned::Run { .. } => panic!("mutating call ran in plan mode"),
        }
        // Read-only calls still run.
        let read = ToolCall { id: "r".into(), name: "read".into(),
            arguments: r#"{"path":"x"}"#.into() };
        assert!(matches!(agent.plan_call(&read).0, sched::Planned::Run { .. }));

        assert!(agent.exit_plan(&mut ui));
        assert!(!agent.exit_plan(&mut ui), "double exit must be a no-op");
        assert_eq!(agent.tools.len(), full.len(), "full set restored");
        match agent.plan_call(&call).0 {
            sched::Planned::Run { .. } => {}
            sched::Planned::Canned(_) => panic!("write must run again after /go"),
        }
    }

    #[test]
    fn overflow_detector_matches_real_server_phrasings() {
        // llama.cpp
        assert!(looks_like_context_overflow(
            "the request exceeds the available context size. try increasing the context size"
        ));
        // OpenAI
        assert!(looks_like_context_overflow(
            "This model's maximum context length is 128000 tokens. However, your messages \
             resulted in 131000 tokens."
        ));
        // Ordinary 400s must NOT trigger emergency compaction.
        assert!(!looks_like_context_overflow("Unknown field: stream_options"));
        assert!(!looks_like_context_overflow("invalid model name"));
    }
}
