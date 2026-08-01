# ui

contractVersion: 1.0.0

## Purpose

The CLI's four output surfaces behind one turn surface: interactive REPL
(with the dock), `exec`, `exec --json`, and `child`. The agent reports
semantic events; this box decides how each surface shows them, and headless
bytes never change because of anything visual.

## The turn surface

What a turn driver may call, and all it may call mid-turn:

```rust
impl Ui {
    pub fn note(&mut self, s: &str);            // dim advisory line
    pub fn error(&mut self, s: &str);
    pub fn ask(&mut self, q: &str) -> bool;     // blocking user question
    pub fn text_delta(&mut self, s: &str);
    pub fn reasoning_delta(&mut self, s: &str); // never enters transcripts
    pub fn end_line(&mut self);
    pub fn tool_requested(&mut self, name: &str, args: &Value);
    pub fn tool_start(&mut self, id: &str, name: &str, brief: &str,
                      read_only: bool);
    pub fn tool_done(&mut self, id: &str, summary: &str, is_error: bool);
    pub fn tool_error_detail(&mut self, id: &str, block: &str);
    pub fn checklist(&mut self, text: &str);    // the plan panel
    pub fn agents(&mut self, block: &str, ids: &[String]);
    pub fn usage(&mut self, u: Usage);
    pub fn done(&mut self, usage: Option<Usage>);   // turn end
}
pub enum Mode { Repl, Exec, ExecJson, Child }
```

`Mode` routes each call: Repl themes to stdout and drives the dock, Exec
prints text to stdout and activity to stderr, ExecJson emits the JSONL
event stream, Child reserves stdout for the one result line. The dock
consumes the same events over its ordered channel (`TurnEvent`, rendered by
`BufferedTurnRenderer`), which is this surface in wire form.

## Inside the box

`dock.rs` (single-writer render loop, pinned regions, the reader policy
over the term box), `prompt.rs` (the pure line editor and input windowing),
`commands.rs` (the slash-command table and completion), `markdown.rs`,
`table.rs`, `theme.rs`, `style.rs`, `scanner.rs` (render primitives).

## Invariants

1. Display-only: nothing here mutates agent state, sessions, or files; the
   one exception is the dock writing the terminal.
2. Headless byte identity: for Exec, ExecJson, and Child, output bytes are
   asserted verbatim by the e2e suites; themes and the dock can never leak
   into them.
3. The dock is the terminal's single writer while active; every event
   arrives through its ordered channel, and only adjacent renders coalesce.
4. Session token totals render through this box's wording (widest label
   that fits, or nothing); the counts come from the session box.

## Dependencies

Contracts: [`term`](../term/CONTRACT.md) (keys, raw mode, size, stdin),
[`session`](../session/CONTRACT.md) (`SessionTokens`),
[`noob-provider`](../../../noob-provider/CONTRACT.md) (`Usage`, interrupt),
[`subagent`](../subagent/CONTRACT.md) (fleet snapshots the dock panels
render).

## Tests

Inline: editor, decoder-driven flows, wording, tables, themes. Boundary:
the pty suites (`ui_editor.rs`, `ui_dock.rs`, `ui_screen.rs`,
`ui_regions.rs`, `ui_session.rs`, `ui_commands.rs`, `ui_agents.rs`) assert
exact screens and scrollback through the real binary.
