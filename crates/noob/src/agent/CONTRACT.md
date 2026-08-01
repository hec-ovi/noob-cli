# agent

contractVersion: 1.0.0

## Purpose

The agentic loop: take one user input, drive inference rounds and tool
batches until the turn ends, and report everything through the ui box's turn
surface. Plan mode, compaction, the batch scheduler, background integration,
and prompt assembly live here.

## Public surface

```rust
pub struct Agent;                 // built by the composition root
impl Agent {
    pub fn new(...) -> Agent;     // system prompt, tool ctx, session,
                                  // provider client, budgets
    pub fn run_input(&mut self, input: &str, ui: &mut Ui) -> RunEnd;
    pub fn continue_after_background(&mut self, ui: &mut Ui) -> RunEnd;
    pub fn enter_plan(&mut self, ui: &mut Ui) -> bool;
    pub fn exit_plan(&mut self, ui: &mut Ui) -> bool;
    pub fn clear_plan_history(&mut self, ui: &mut Ui) -> usize;
    pub fn reload_skills(&mut self, ui: &mut Ui) -> (Vec<String>, Vec<String>);
    pub fn reload_mcp(&mut self, ui: &mut Ui) -> (Vec<String>, Vec<String>);
    pub fn enable_background_agents(...);
    pub fn background_hub(&self) -> Option<BackgroundHub>;
    pub fn background_snapshot(&self) -> JobsSnapshot;
    pub fn cancel_background(&self, id: &str) -> bool;
    pub fn cancel_all_background(&self) -> usize;
    pub fn shutdown_background_agents(&mut self, ui: &mut Ui);
    pub fn context_estimate(&self) -> u64;
    pub fn last_usage(&self) -> Option<Usage>;
}
pub enum RunEnd;                  // how the turn ended
```

`prompt::assemble` builds the fixed system prompt from `PromptInputs`, in
order: the config directory's `AGENTS.md` (the main prompt), its `TOOLS.md`
(tool guidance, merged after it), then the runtime layers (environment
block, project `AGENTS.md`, skills index, MCP line). A present file
replaces its embedded default wholesale; the defaults live in
`crates/noob/prompts/agents-default.md` and `tools-default.md`, with
`compact.md` for compaction, all loaded at compile time. User prompt files
are capped at 16 KiB each.

## The seam

The loop holds `&mut Ui` and speaks only the ui contract's turn surface
(note, error, ask, the deltas, the tool lifecycle, done, usage, agents,
checklist). The ui box multiplexes the four output surfaces behind those
methods, so this loop never knows which surface is attached. In the other
direction the loop owns the turn policy and its ToolCtx slices; the ui box
renders and never decides.

## Invariants

1. Inference rounds per input are capped (`TURN_CAP` 50 for the user's
   agent; children clamp their own budget against it).
2. The request prefix is byte-stable across a session: system prompt, tool
   array, and replayed transcript extend byte-exactly, so the provider's
   prompt cache keeps working. Anything that would change the prefix
   (skills reload, MCP reload) says so and starts a fresh cache lineage.
3. Compaction rewrites the transcript only through the session box's reset
   record, and never the bill; a failed or empty compaction backs off and
   raises the effective threshold it reports.
4. Every batch ends with unspent skills-write grants cleared; a canceled
   batch heals its dangling tool calls before the next request, so the
   transcript is always API-valid.
5. Background results are integrated at round boundaries, mid-turn when a
   turn is running, at the prompt otherwise; the parent never blocks on a
   child inside a round.

## Dependencies

Contracts: [`noob-provider`](../../../noob-provider/CONTRACT.md) (requests,
streaming), [`tools`](../tools/CONTRACT.md) (dispatch and the context
slices), [`session`](../session/CONTRACT.md) (log, resume, reset),
[`subagent`](../subagent/CONTRACT.md) (the hub), [`skills`](../skills/CONTRACT.md)
and [`mcp`](../mcp/CONTRACT.md) (reloads), [`emit`](../emit/CONTRACT.md)
(frames), [`config`](../config/CONTRACT.md) (budgets), and the ui box's turn
surface.

## Tests

Inline: plan-mode policy, doom-window, compaction shapes, scheduler
ordering (sched.rs), prompt assembly. Boundary: `crates/noob/tests/e2e_p2.rs`,
`e2e_p6.rs`, and the ui suites drive the loop through the real binary; the
prompt budget is gated by `crates/noob/tests/budget.rs`.
