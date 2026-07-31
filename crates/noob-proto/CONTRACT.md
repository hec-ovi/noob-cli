# noob-proto

contractVersion: 1.0.0

## Purpose

The wire contract between the agent and anything watching it: one
newline-delimited JSON frame per line, `Event` frames out, `Command` frames in,
and neither side ever links against the other.

## The wire

A frame is one line: `{"v":1,"t":"tool.start", ...fields}`. `v` and `t` lead
every line in that order, so a reader can classify a frame before parsing the
rest and a human can tail the stream. The agent writes `Event` frames; a front
end writes `Command` frames. Nothing else crosses.

## Public surface

```rust
pub const VERSION: u16 = 1;

pub struct Frame<T> { pub v: u16, pub body: T }
impl<T> Frame<T> {
    pub fn new(body: T) -> Frame<T>;   // stamps v = VERSION
    pub fn readable(&self) -> bool;    // v <= VERSION
}

pub trait Body: Sized {                // implemented by Event and Command
    fn tag(&self) -> &'static str;
    fn write_fields(&self, out: &mut String);
    fn from_value(value: &Value) -> Self;   // never fails; invariants 4 and 5
}

pub fn encode<T: Body>(body: &T) -> String;             // one line, newline included
pub fn decode<T: Body>(line: &str) -> Option<Frame<T>>; // None only per the error set

pub use serde_json::Value;   // Event::ToolStart carries one; consumers stay self-contained
```

The bodies are `pub enum Event` (24 variants) and `pub enum Command` (14
variants); the shapes they carry are `pub struct ToolError`, `Usage`, `Span`,
`Sample` and `pub enum AgentState`. Every field mirrors its schema file below,
field for field, and the schema is enforced at the test boundary
(`tests/contract.py`), not per call. Two helper methods beyond plain data:

```rust
impl Usage      { pub fn prefilled(&self) -> u64; }        // prompt - cached_prompt, saturating
impl AgentState { pub fn as_str(&self) -> &'static str; }  // the exact wire string
```

## Frames out: Event

Shapes and optionality per tag: [`schema/event.json`](schema/event.json).
A `?` field is written only when present, never as null.

| `t` | Variant | Carries |
|---|---|---|
| `session.start` | `SessionStart` | `id`, `workspace`, `model`, `resumed` |
| `session.end` | `SessionEnd` | `id` |
| `turn.start` | `TurnStart` | `turn` |
| `turn.end` | `TurnEnd` | `turn`, `interrupted?` |
| `text.delta` | `TextDelta` | `d` |
| `reasoning.delta` | `ReasoningDelta` | `d`; ephemeral, never enters a transcript, a consumer may drop it |
| `tool.start` | `ToolStart` | `call_id`, `name`, `brief` (the label every consumer renders), `args` (any JSON) |
| `tool.progress` | `ToolProgress` | `call_id`, `line` |
| `tool.end` | `ToolEnd` | `call_id`, `summary`, `elapsed_ms`, `error?` ([`schema/tool-error.json`](schema/tool-error.json)) |
| `file.open` | `FileOpen` | `path`, `lines`, `call_id?` |
| `file.span` | `FileSpan` | `path`, `span` ([`schema/span.json`](schema/span.json)), `call_id?` |
| `file.edit` | `FileEdit` | `path`, `span`, `before`, `after`, `call_id?` |
| `file.close` | `FileClose` | `path`, `call_id?` |
| `agent.spawn` | `AgentSpawn` | `agent_id`, `prompt`, `tools` |
| `agent.state` | `AgentStateChanged` | `agent_id`, `state` (queued, running, done, failed, canceled, unknown), `detail?` |
| `agent.output` | `AgentOutput` | `agent_id`, `line` |
| `skill.list` | `SkillList` | `names` |
| `mcp.list` | `McpList` | `names` |
| `mcp.state` | `McpState` | `name`, `connected`, `tools?` |
| `usage` | `UsageReport` | `usage` ([`schema/usage.json`](schema/usage.json)) |
| `metrics` | `Metrics` | `group`, `at_ms`, `samples` ([`schema/sample.json`](schema/sample.json)); a new readout is a new `group`, not a new frame type |
| `note` | `Note` | `line` |
| `error` | `Error` | `line` |
| `unknown` | `Unknown` | nothing; the envelope alone |

## Frames in: Command

Shapes per tag: [`schema/command.json`](schema/command.json).

| `t` | Variant | Carries |
|---|---|---|
| `prompt.submit` | `PromptSubmit` | `text` |
| `prompt.queue` | `PromptQueue` | `text`; queues behind the running turn instead of interrupting it |
| `turn.cancel` | `TurnCancel` | nothing |
| `agent.cancel` | `AgentCancel` | `agent_id` |
| `skill.add` | `SkillAdd` | `source` |
| `skill.remove` | `SkillRemove` | `name` |
| `mcp.add` | `McpAdd` | `name`, `spec` |
| `mcp.remove` | `McpRemove` | `name` |
| `mcp.connect` | `McpConnect` | `name` |
| `config.set` | `ConfigSet` | `key`, `value` |
| `config.unset` | `ConfigUnset` | `key` |
| `session.list` | `SessionList` | nothing |
| `session.open` | `SessionOpen` | `id` |
| `unknown` | `Unknown` | nothing |

## Errors

`encode` cannot fail. `decode` returns `None` for exactly four inputs: a blank
line, a line that is not JSON, a frame whose `v` is not a non-negative
integer, and a `v` above the reader's `VERSION`. That is the whole closed set, and it is silent by
design: a bad line is skipped so one frame cannot end a session. Everything
else degrades instead of failing, per invariants 4 and 5.

## Invariants

1. One frame per line. `encode` returns exactly one line with its newline;
   `decode` takes one line and trims surrounding whitespace.
2. `v` then `t` lead every line, in that order.
3. The writer stamps `VERSION`; a reader accepts `v` at or below its own and
   refuses anything above rather than half-understanding it. Additive changes
   bump `VERSION`; a breaking change ships a new `t` beside the old, callers
   migrate, the old one retires. A frame is never redefined in place.
4. An unknown `t` decodes to the `Unknown` variant, never `None`: a feature
   this reader lacks, not a stream it cannot read. One level down, an
   unrecognised `state` string reads as `AgentState::Unknown`, costing one
   field rather than the frame.
5. A missing or mistyped field reads as its zero value (empty string, 0,
   `None`, empty list), so a frame is usable in part. A `tool.end` `error` is
   read only when it is an object.
6. Absent options are omitted from the wire, never written as null.
7. A non-finite float is written as `null`: JSON has no NaN or infinity, and a
   frame no parser accepts is worse than a reading that is missing.
8. Strings are escaped per RFC 8259 (the quote, the backslash, everything
   below 0x20), so a value cannot forge a field or a frame.
9. Correlation is by id, never by arrival order. Every `tool.*` frame carries
   its `call_id`; `file.*` frames carry the call that caused them when one
   exists (compaction produces call-less file frames). Concurrent calls
   interleave, so consumers must not assume starts and ends nest.

## Dependencies

Contracts: none. Crates: `serde_json` alone (workspace pin). `serde` with
`derive` is deliberately absent: it pulls `serde_derive`, `proc-macro2`,
`quote`, `syn` and `unicode-ident`, and the binary's published gate of 45
runtime crates refuses that, so the serialization is written out by hand.

## How to modify this blackbox safely

The writing half is a match the compiler checks exhaustively; the reading half
is proven by nothing but the round-trip tests, which is why they cover every
variant of both enums rather than a sample. Keep that property: a new variant
gets a round-trip case, a branch in its frame schema, and a `fixtures/valid`
line produced by the real encoder, and it bumps `VERSION` and the minor
contractVersion. Breaking changes follow invariant 3.

Verify both halves of the boundary: `cargo test -p noob-proto` for behaviour,
`python3 tests/contract.py` (needs `jsonschema`) for the wire shapes. The
fixture runner mirrors `gui/layers/text-geometry/tests/contract.py`: fixtures
are named after their schema, `invalid/` must be rejected, and every schema
needs a fixture.
