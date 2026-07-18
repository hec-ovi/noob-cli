# noob/src/ui

Terminal input and rendering for REPL, exec, JSONL, and child surfaces. The module has no TUI or line-editor dependency.

## Interactive input

`prompt.rs` provides the termios editor: insertion, multibyte backspace, cursor movement, Home, End, Delete, Ctrl-A/E/U/K/W, bracketed paste, interrupt, and EOF. `NOOB_RAW=0` selects cooked input.

`dock.rs` is enabled by default at an interactive terminal. `NOOB_DOCK=0` selects the classic per-prompt editor. The dock holds raw mode for the session and uses:

- One blocking stdin reader that emits decoded keys.
- One per-turn worker running the agent.
- One main-thread render loop as the sole terminal writer.
- One ordered channel for keys, semantic render events, questions, reader loss, and turn end.

The active frame always shows status, editable draft or question, and queue/cancel state; the idle frame shares the exact same layout (top rule, pinned rows inside the frame, input row, bottom rule), so plan and agents rows never read as loose transcript text between turns. Enter accepts a non-empty message as queued, leaving the running turn untouched: the message waits as a pinned row styled like the record it will become (green marker, plain text) with only the trailing `[queued]` tag in green, under the plan and agents rows, and dispatches in order once the turn ends, echoed into the transcript as a plain `› message` record at that moment (never at acceptance, so no stale `[queued]` marker survives an answer). Explicit cancellation hands queued and unsubmitted text back to the draft. Escape twice within five seconds cancels; Ctrl-C commits cancellation immediately.

Each erase-and-redraw cycle of the frame reaches the terminal as one batched write, so a repaint never renders as partial frames (the pinned plan does not blink while output streams). A terminal resize resets the viewport (clear screen, home, repaint the frame): after a reflow no cursor-relative erase can reliably find the old frame. Known pending issue: repeated resizes leave stale idle frames and blank gaps in scrollback history; resize handling remains under rework.

Typeahead received before a question cannot answer it. EOF and reader errors persist as closed-input state and deny questions without deadlock.

Tab completes a `/`-prefixed command in the input editor; a dim hint lists candidates for an ambiguous prefix and never enters the buffer. With an empty draft and detached jobs, Tab toggles a persistent view of their IDs, state, elapsed time, prompt slices, and bounded recent activity. The view survives parent-turn boundaries while the editor stays usable; when closed, a one-line live running/ready counter stays pinned above the idle input instead of vanishing. While a turn runs and the buffer is empty, the input row shows a queueing placeholder. Completion, agent details, and the placeholder are input-side only: the model receives a draft only on submission.

## Semantic rendering

`Ui::for_turn` emits `TurnEvent` values for text, reasoning, line ends, actual tool starts, tool finishes, notes, errors, and completion. The main renderer replays those semantics through the normal byte renderer. Only adjacent render events may be coalesced; questions, keys, reader loss, and end are ordering barriers.

Tool requests remain JSONL planning events. Interactive tool start lines are emitted only when the scheduler begins execution, and finish lines follow real completion order.

The `plan` checklist and sub-agent status are independent state machines that may coexist in the themed REPL's bounded region. The active plan glyph animates display-only, during turns and at the idle prompt alike (a pinned in-progress row never freezes between turns). Completed actions carry their own times. Long plans end in a counted summary and reserve the active step plus an agent status row; a fully completed plan collapses to one timed line, and a canceled turn leaves the plan pinned in its actual state. An unfinished plan survives turn boundaries on the session and stays pinned at the idle prompt; a plan whose every step completed is retired at turn end into one timed transcript record and unpinned. Turn end re-records neither an in-progress plan nor the hub-backed agents view into the transcript. The closed agents view derives its count from the live hub snapshot instead of a stale acknowledgment. Covered per-subagent activity lines are suppressed. On resume, `replay_transcript` redraws the prior conversation before the first prompt while filtering synthetic bookkeeping items; it never mutates the transcript or session.

## Markdown and tables

Interactive assistant text passes through `markdown.rs` and `table.rs`. Supported structure is headings, emphasis, inline code, lists, quotes, fenced code, JSON accents, and GFM-style tables. Wide tables render as wrapped grids; narrow tables use stacked records.

Bounds are 16 Ki characters per logical line, 32 KiB or 512 lines per fenced block, and 32 KiB, 128 rows, or 16 columns per table. Overflow degrades to literal streaming and never drops source output.

Control bytes in model text, reasoning, tool briefs, notes, and questions are made visible or converted to safe spacing before terminal output.

## Themes and modes

`Theme` supports `matrix`, `ocean`, `amber`, and `violet`, with documented aliases and `matrix` fallback. Color depth is truecolor, 256, 16, then none. `NO_COLOR` removes color without removing structure or status.

Rich rendering requires an interactive REPL terminal. Piped REPL, exec, JSONL, and child output remain plain and protocol-stable. Display code never mutates requests, transcript items, or session files.

Terminal restoration is guarded on normal return, panic, and SIGINT. Bracketed paste is disabled on every exit path.
