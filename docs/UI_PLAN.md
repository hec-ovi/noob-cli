# Interactive terminal status and backlog

This document records the current REPL behavior and the remaining interface work. Runtime mechanics are specified in [ARCHITECTURE.md](../ARCHITECTURE.md) and [the UI contract](../crates/noob/src/ui/contract.md).

## Current behavior

The persistent dock is enabled by default at an interactive terminal. `NOOB_DOCK=0` selects the classic per-prompt editor, and `NOOB_RAW=0` selects cooked input.

During a turn, the dock keeps three rows visible:

1. Animated status with elapsed time, plan mode, and active tools.
2. Editable draft or an active confirmation question.
3. Queue and cancellation state.

The user can type while the model streams or tools run. Enter records the submitted text with a `[queued]` marker and leaves the running turn untouched; queued messages dispatch in order once the turn finishes on its own. Escape or Ctrl-C cancellation hands queued and unsubmitted text back to the editor. Escape twice within five seconds cancels; Ctrl-C cancels immediately. A second Ctrl-C during cancellation restores the terminal and exits with status 130.

One main-thread render loop owns terminal output. A stdin reader and the agent worker send ordered events to it. Text, reasoning, tool start, tool finish, notes, errors, questions, keys, reader loss, and turn end retain channel order. Only adjacent render events can share a short repaint window.

The interactive REPL also provides:

- Tab completion for a `/`-prefixed command, with a dim hint that lists candidates for an ambiguous prefix. On an empty draft, Tab instead opens a persistent, bounded view of detached sub-agent state and recent activity.
- A dim `type a message; Enter queues it` placeholder on the input row while a turn runs and the buffer is empty.
- A `plan` checklist maintained by the model through the `plan` tool. It is pinned above the input during turns and at the idle prompt, updating in place across turn boundaries instead of re-printing into the transcript. The active glyph animates, completed actions show elapsed time, long lists are capped with counts, and a completed plan collapses to one timed line. It is agent-driven with no approval step and is separate from `/plan` mode.
- Detached `subagent` jobs with stable IDs, cancellation through `/agents`, and one final result injected into the parent context as soon as that job is ready. This includes `tools: "all"` jobs; unrelated slow jobs do not delay a completed result.
- Redisplay on resume: at an interactive terminal a resumed conversation is redrawn before the first prompt, with synthetic bookkeeping items filtered. Corrupt records are skipped with one bounded warning. Redisplay is display-only and never touches the request, transcript, or session log.
- Resize reflow through SIGWINCH while idle or during an active turn. The erase walks the frame's physical height after the terminal's own rewrap (drawn cells over the new width), so shrinking the window repaints cleanly instead of leaving rule fragments.

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
- Terminal and scheduler behavior is covered by unit tests and real pseudo-terminal tests.

## Backlog

- In-session history navigation.
- Optional folding for long displayed tool results without changing transcript content.
- Richer MCP JSON presentation after the untrusted-content wrapper.

These are interface enhancements, not release blockers for the current dock and Markdown implementation.
