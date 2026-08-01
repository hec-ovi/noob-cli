# subagent

contractVersion: 2.0.0

## Purpose

The `subagent` tool and the background job hub: spawn the binary itself as a
detached child agent, keep its progress bounded, and deliver exactly one
result string back into the parent transcript.

## The process boundary

The child is `current_exe()` run as `noob child`: the task goes to the
child's stdin as one JSON object (capped at 8 MiB, pre-checked by the parent
so an oversized payload gets a real error), exactly one JSON result line
comes back on stdout, progress flows on stderr. argv + stdin + stdout is the
whole IPC surface; only the result string enters the parent transcript.

## Public surface

```rust
pub fn spec() -> ToolSpec;   // prompt | status | cancel; tools mode
                             // read-only (default) | web | all; max_turns
pub struct SpawnEnv<'a>;     // everything the tool needs, as plain params:
                             // workspace, websearch, task, loaded_skills.
                             // The tools box builds it at dispatch
pub fn run(env: &SpawnEnv, args: &Value) -> ToolOutcome;

pub const MAX_DEPTH: u32 = 2;         // depth 0 and 1 spawn; at 2 the tool
                                      // is not registered at all
pub const DEFAULT_CONCURRENCY: usize = 4;
pub const DEFAULT_MAX_TURNS: u32 = 25;
pub const DEFAULT_WALL_CLOCK_S: u64 = 0;   // 0: no limit

pub struct TaskCfg;          // session-scoped settings, resolved at
                             // bootstrap: depth, concurrency, max_turns,
                             // wall_clock, verbose, overrides, yolo,
                             // ancestor_skills, optional BackgroundHub

pub struct BackgroundHub;    // session-scoped detached jobs, cap 64
impl BackgroundHub {
    pub fn new(concurrency: usize) -> BackgroundHub;
    pub fn with_emitter(concurrency: usize, emitter: Emitter) -> BackgroundHub;
    pub fn take_ready(&self) -> Vec<ReadyResult>;   // consuming
    pub fn settled_ready(&self) -> bool;
    pub fn revision(&self) -> u64;      // bumps on every visible change
    pub fn snapshot(&self) -> JobsSnapshot;
    pub fn cancel(&self, id: &str) -> bool;
    pub fn cancel_all(&self) -> usize;
    pub fn shutdown(&self) -> Vec<ReadyResult>;
    pub fn raise_next_id(&self, next: u64);   // resume: ids keep ascending
}
pub struct ReadyResult;      // one finished job's outcome, moved whole
pub struct JobsSnapshot;     // what a status call or a panel renders
```

## Errors

The tool never panics the parent: every failure is a `ToolOutcome` the model
reads (missing prompt, unknown mode, cap exceeded, child died, wall clock
fired). A child that vanishes mid-run surfaces as a failed outcome with the
bounded stderr tail attached when verbose.

## Invariants

1. The depth ceiling is structural: children at the cap simply do not get
   the tool, so a fleet cannot multiply behind the user's back.
2. Kill is by process group, and a child dies with its parent (parent-death
   signal), so no sub-agent outlives the session.
3. Child overrides travel over the private stdin protocol: a root `--model`
   or `--base-url` cannot silently send detached work elsewhere, and the
   root sandbox decision crosses the boundary intact.
4. Skills loaded by ancestors are excluded from children, the whole chain
   down, so a child cannot rediscover and re-run the skill that spawned it.
5. Parent memory is bounded: stderr keeps a 64 KiB head per child, the hub's
   progress window keeps 2 KiB and 12 display lines per job; watchers get
   the raw stream through the emit tap instead.
6. Workers own only child processes and hub state; the parent Agent stays
   the sole owner of transcript, session log, requests, and UI.
7. Results move whole: the completed report reaches the parent unchanged
   through `ReadyResult`, however much progress was clipped.

## Dependencies

Contracts: the tools box (`ToolOutcome` and the arg readers; the session
context itself never crosses this boundary),
[`crates/noob-provider/CONTRACT.md`](../../../noob-provider/CONTRACT.md)
(`Overrides`, `ToolSpec`, the interrupt flag),
[`crates/noob-proto/CONTRACT.md`](../../../noob-proto/CONTRACT.md) and
[`crates/noob/src/emit/CONTRACT.md`](../emit/CONTRACT.md) (agent frames for
anything watching). Callers pass the `DEFAULT_*` values through the config
box's task getters at bootstrap.

## Tests

Inline: hub lifecycle, caps, cancellation, progress bounding, child protocol
slices. Boundary: `crates/noob/tests/e2e_p6.rs` (the child protocol against
the real binary), `crates/noob/tests/ui_agents.rs` (the fleet through the
terminal).
