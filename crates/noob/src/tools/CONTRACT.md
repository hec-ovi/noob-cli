# tools

contractVersion: 1.0.0

## Purpose

The tool registry: the specs the model sees, one dispatch with its safety
rails, the per-capability context slices, and the shared infrastructure
(write policy, truncation, path resolution, untrusted wrapping) every tool
builds on. What a tool can touch is its signature: dispatch hands each tool
only its slices.

## The context and its slices

```rust
pub struct ToolCtx {            // built once at bootstrap
    pub core: Core,             // workspace, sandbox, folder lock, caps,
                                // emitter: what nearly every tool needs
    pub fs: FsState,            // seen-file stamps, edit escalation, dedup
    pub grants: WriteGrants,    // the skills-dir write gate
    pub skills: SkillsState,    // discovered list + loaded names
    pub plan: PlanState,        // the checklist and its display timing
    pub gauge: ContextGauge,    // context accounting, lock-free reads
    pub evidence: Evidence,     // successful websearch call count
    pub bash_warned: AtomicBool,
    pub mcp: Option<Mcp>,
    pub websearch: bool,
    pub task: Option<TaskCfg>,
}
```

Each tool's `run` takes exactly its slices; the grain is deliberate: one
file per tool, one slice list per signature, no nested folders.

| Tool | Signature takes |
|---|---|
| read | `&Core, &FsState` |
| write, edit | `&Core, &FsState, &WriteGrants` |
| grep, glob, ls | `&Core` |
| bash | `&Core, &AtomicBool` |
| skill | `&Core, &SkillsState, can_delegate: bool` |
| plan | `&PlanState` |
| context | `&ContextGauge` |
| websearch | `&Core, &Evidence` |
| mcp_connect, mcp_call | `Option<&Mcp>, &Caps` |
| subagent | its own `SpawnEnv`, built here at dispatch |

## Dispatch and its rails

`dispatch(ctx, name, args) -> ToolOutcome`. Before any tool runs: write and
edit take the cross-process workspace lease (busy means a clear refusal,
never a corrupted tree); a mistyped `path` on read/edit/ls/grep is corrected
to the near miss with a note the model learns from, while write is never
redirected (a new file must not clobber a neighbor). `specs()` is byte-
stable for the session; `is_read_only`, `READ_ONLY_SET`, and
`WEB_RESEARCH_SET` define the restricted registrations.

## Results

`ToolOutcome { content, summary, warning, canceled, kind, code, remedy }`:
content enters the transcript verbatim, summary is the human line, warning
is UI-only. `canceled` is structural so a tool cannot forge it. `fail::*`
is the closed class set; every classified error also says what to do next.

## Shared infrastructure on this contract

- `guard`: the two-state sandbox (`Sandbox`, from the config box's mode),
  write/edit path admission, `FileStamp` staleness, the workspace write
  lease, `atomic_write` (temp + fsync + rename, writes THROUGH symlinks,
  preserves mode), and the skills-dir target rule behind `WriteGrants`.
- The folder lock: in workspace mode `Core.lockdown` holds the exec box's
  `Lockdown` when the kernel provides one, and bash hands it to every run,
  so a model-typed command cannot write outside the workspace and temp. A
  locked command's toolchain caches (cargo, go, npm, XDG) are redirected
  under the temp tree so builds keep working. bash's one-time UI notice
  states whichever is true: folder-locked, or no sandbox at all.
- `truncate`: `Caps` (every cap in one struct, `uncapped()` lifts all),
  head+tail truncation with markers where the middle went, line clipping, the frozen
  trailer phrasings.
- `paths`: near-miss resolution for the correction rail.
- `untrusted`: delimiter-wrapping for content that came from outside.

## Invariants

1. Registration is decided at bootstrap and never changes mid-session, so
   the request tools array stays byte-stable for the prompt cache.
2. A tool sees only its slices; nothing reaches the whole context but
   dispatch itself.
3. Unspent skills-write grants die at batch end (`grants.clear()` in the
   agent); a grant covers exactly one applied mutation of its exact target.
4. Truncation policy is resolved once at bootstrap; when lifted, no
   truncation marker ever renders.

## Dependencies

Contracts: [`exec`](../exec/CONTRACT.md) (bash and websearch run children
through it), [`emit`](../emit/CONTRACT.md) (the side channel in `Core`),
[`config`](../config/CONTRACT.md) (sandbox mode, caps switch, dedup),
[`skills`](../skills/CONTRACT.md) (the discovered list), [`mcp`](../mcp/CONTRACT.md)
(the manager behind the two MCP tools), [`subagent`](../subagent/CONTRACT.md)
(`SpawnEnv`, `TaskCfg`), [`noob-provider`](../../../noob-provider/CONTRACT.md)
(`ToolSpec`, the interrupt flag).

## Tests

Each tool file carries its own tests through its real `run` surface; the
registry's rails (lease, path correction, specs stability) are tested in
this file's test module and end to end by `crates/noob/tests/e2e_p5.rs` and
the ui suites.
