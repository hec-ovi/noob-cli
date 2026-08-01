# state

contractVersion: 1.0.0

## Purpose

The pure reducer: one noob-proto Event in, the conversation and metrics
model out. Everything the panes render (output, activity, plan, agents,
files, context, phase) is a fold over the stream, and nothing here touches
the window, the clock, or the filesystem.

## Public surface

```rust
pub struct State;
impl State {
    pub fn new() -> State;
    pub fn apply(&mut self, event: Event) -> bool;          // dirty?
    pub fn apply_at(&mut self, event: Event, at: Option<f64>) -> bool;
    pub fn show_file(&mut self, index: usize) -> bool;      // model only
    pub fn submitted(&mut self, text: &str);   // echo now, turn starting
    pub fn enqueue(&mut self, text: &str);     // wait in `queued`; echoes
                                               // once at the turn.start
                                               // that takes it, front first
    pub fn output_reserved(&self, rows: usize) -> usize;  // rows the queue
                                               // pins at the OUTPUT bottom;
                                               // one row always stays text
    pub fn show_agent(&mut self, ordinal: usize) -> bool;  // point the
                                               // output tab at one agent
    pub fn agent_shown(&self) -> Option<&AgentRow>;  // and read it back
    // read surface: output/activity/queued/plan/agents/files/context/
    // phase/status/usage/turn and the wrapped panes.
    // agents holds the live fleet only: each row carries a stable 1-based
    // ordinal (spawn order) and its own bounded output pane; a child that
    // finishes leaves the list in the event that ends it, and shown_agent
    // clears with it
}
pub struct Pane;   // wrapped scrollback: lines in, visual rows out, ring;
                   // .clipped() lists one row per line (activity), and
                   // anchor_first/spot_row lend the row arithmetic to a
                   // surface with an offset of its own (the call popup)
pub struct Line;   // one logical line with its Tone and Kind
pub enum Tone;  pub enum Kind;   // what a tool call renders as
pub struct Call;   // one remembered tool call, and its popup cells
// plus Todo, AgentRow, ContextFill, FileView, Phase, Rates
```

## Invariants

1. Pure fold: same events in the same order, same state; time enters only
   as the `at` stamp the caller passes.
2. Bounded: panes are rings, the file list caps at MAX_FILES, activity
   keeps a bounded window; a long session cannot grow memory unbounded.
3. Display ownership ends here: scrolling, selection, and the open-file
   follow policy live in the shell; this box only says what exists.
4. Unknown events are ignored (the proto degradation rule), never fatal.

## Dependencies

Contracts: [`noob-proto`](../../../../crates/noob-proto/CONTRACT.md) (the
events), [`text-geometry`](../../../layers/text-geometry/CONTRACT.md)
(wrapping), the style box (tone classification of markdown fences), the
dock box (`View` in pane addressing).

## Tests

Inline: the reducer's event shapes, ring bounds, wrap behavior, call
popups, phase transitions (headless, ~60 tests).
