# Interactive terminal status and backlog

This document records the current REPL behavior and the remaining interface work. Runtime mechanics are specified in [ARCHITECTURE.md](../ARCHITECTURE.md) and [the UI contract](../crates/noob/src/ui/contract.md).

## Current behavior

The persistent dock is enabled by default at an interactive terminal. `NOOB_DOCK=0` selects the classic per-prompt editor, and `NOOB_RAW=0` selects cooked input.

During a turn, the dock keeps three rows visible:

1. Animated status with elapsed time, plan mode, and active tools.
2. Editable draft or an active confirmation question.
3. Queue count and cancellation state.

The user can type while the model streams or tools run. Enter queues one message for the next turn and immediately records it as queued in the transcript view. Cancellation returns queued text to the editor. Escape twice within five seconds cancels; Ctrl-C cancels immediately. A second Ctrl-C during cancellation restores the terminal and exits with status 130.

One main-thread render loop owns terminal output. A stdin reader and the agent worker send ordered events to it. Text, reasoning, tool start, tool finish, notes, errors, questions, keys, reader loss, and turn end retain channel order. Only adjacent render events can share a short repaint window.

The interactive REPL also provides:

- Tab completion for a `/`-prefixed command, with a dim hint that lists candidates for an ambiguous prefix. Completion is input-side editing between keystrokes, never during a token stream.
- A dim `type to queue a message` placeholder on the input row while a turn runs and the buffer is empty.
- A `todo` checklist: the model maintains a `[x]`/`[~]`/`[ ]` list through the `todo` tool and the themed REPL recolors the glyph lines. It is agent-driven with no approval step and is separate from `/plan`.
- An agents panel for `task` fan-out: two or more consecutive `task` calls render as one checklist with per-agent running or done status, a one-line result and turn count each, and the concurrency cap in the header. A lone `task` keeps its single activity line.
- Redisplay on resume: at an interactive terminal a resumed conversation is redrawn before the first prompt (past human turns, assistant Markdown, and one-line tool digests), with synthetic bookkeeping items filtered. It is display-only and never touches the request, transcript, or session log.

## Rendering

Assistant output uses a zero-dependency streaming Markdown renderer in the interactive REPL:

- Headings, bold, italic, inline code, lists, and block quotes.
- Fenced code with an optional language label and gutter.
- JSON key, string, number, boolean, and null accents in JSON fences.
- GFM-style table detection with alignment.
- Wrapped box-drawing grids on wide terminals, with cells sized by terminal display width so wide glyphs keep the borders aligned.
- Stacked key/value records on narrow terminals.

Line, fence, and table buffers are bounded. Crossing a bound switches to literal streaming without dropping source text.

Untrusted terminal control bytes are rendered visibly instead of executed. `NO_COLOR` removes color while retaining Markdown structure, reasoning, errors, and liveness.

Piped REPL, `exec`, `exec --json`, and child output do not use the rich renderer.

## Themes

`NOOB_THEME` accepts:

- `matrix`
- `ocean` or `blue`
- `amber` or `gold`
- `violet` or `purple`

Unknown names fall back to `matrix`. Color depth degrades from truecolor to 256 colors, 16 colors, then no color.

## Verified constraints

- No new runtime crate.
- Piped and headless protocol bytes remain unchanged.
- The model receives a draft only after submission.
- The request path does not parse Markdown; semantic text is parsed only by the interactive renderer.
- The terminal and scheduler behaviors have unit tests plus 41 pty tests.

## Backlog

- In-session history navigation.
- Optional folding for long displayed tool results without changing transcript content.
- Richer MCP JSON presentation after the untrusted-content wrapper.
- Terminal resize handling while no key or output event is arriving.

These are interface enhancements, not release blockers for the current dock and Markdown implementation.
