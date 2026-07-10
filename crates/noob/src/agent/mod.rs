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
use crate::tools::guard::fnv1a64;
use crate::tools::{self, ToolCtx, ToolOutcome};
use crate::ui::Ui;

/// Locked breaker thresholds (ARCHITECTURE.md, agent loop).
const TURN_CAP: u32 = 50;
const DOOM_WINDOW: usize = 12;
const DOOM_REPEATS: usize = 3;
const NUDGE_AT: u32 = 4;
const PAUSE_AT: u32 = 8;

pub struct Agent {
    pub client: Client,
    pub config_dir: PathBuf,
    pub ov: Overrides,
    /// Frozen at session start; every request sends exactly this head.
    pub system: String,
    /// Frozen at session start; byte-stable for the whole session.
    pub tools: Vec<ToolSpec>,
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
        Agent {
            client,
            config_dir,
            ov,
            system,
            tools,
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
                    if !self.compact(ui) {
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

            if let Some(u) = turn.usage {
                self.last_usage = Some(u);
                self.chars_since_usage = 0;
            }
            // Output is never capped, so Length means the context filled up
            // mid-turn: discard the partial turn, compact once, retry.
            if turn.finish == Finish::Length && !emergency_used {
                emergency_used = true;
                ui.end_line();
                ui.note("the model hit the end of the context mid-turn; compacting and retrying");
                if !self.compact(ui) {
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

            if turn.tool_calls.is_empty() {
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
                if ui.ask(&format!(
                    "{} tool calls in a row failed; keep going?",
                    self.consec_errors
                )) {
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
        // serde_json's default map is sorted, so this serialization is
        // canonical: key order cannot dodge the repeat detector.
        let canonical = format!("{}\u{0}{}", call.name, args);
        let hash = fnv1a64(canonical.as_bytes());
        let repeats = self.recent_calls.iter().filter(|&&h| h == hash).count();
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
