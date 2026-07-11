# noob/src/ui

Rendering for the four surfaces (P2), now themeable, plus an opt-in line editor
for the interactive prompt. No TUI framework and no line-editor crate: escapes
and the termios editor are hand-rolled over `libc` (zero new crates).

Input (prompt.rs): at an interactive terminal the REPL runs a small raw-mode
line editor. It draws a full-width green top rule over a `│ › ` input line (no
wordmark; the banner already carries it), with real editing (insert, backspace
across a multibyte char, Ctrl-A/E/U/K/W, left/right/Home/End, Delete, bracketed
paste that holds newlines until a real Enter). On submit the box collapses to a
compact `› message` line, so history reads as a list of sent messages, not a
stack of frames. The live input is shown as a one-row horizontal window of the
buffer (a long line scrolls instead of wrapping), so the box stays two rows and
every in-place redraw stays exact. Widths are counted one column per character
(no unicode-width table, by design), so runs of double-width CJK or emoji are
the one case that can still spill a row; the buffer and submitted line are always
correct. The
top rule re-fits to the terminal width on each keystroke (a bare ioctl on the
read path already taken): a freshly spawned pty often reports width 0 for the
first draw and its real size lands a moment later, so the box snaps to full width
on the first keypress, and a later resize is tracked the same way. No timer, no
idle loop, and no SIGWINCH handler (signals are left exactly as they were, so
nothing new can touch the request path). It is off the inference path: raw mode
is entered only while
typing and restored to cooked before the agent runs, so keystrokes never reach
the model (it sees the message once, on Enter). Three hooks restore the terminal
so a crash never leaves the shell raw: the RAII guard on the normal return, a
panic hook (release is `panic = "abort"`, so `Drop` does not run on a panic),
and the SIGINT handler before its `_exit(130)`. EOF (Ctrl-D) exits; Ctrl-C at
the prompt cancels the line and reprompts, kept distinct. `NOOB_RAW=0` forces
the cooked reader. Piped or headless input falls back to the exact cooked
`read_line`, so those surfaces stay byte-for-byte what they were: no box, no
bracketed-paste toggles, no escapes.

The REPL persists its session (a fresh id, or `--session <id>` to resume a
closed one) and, on exit at a terminal, prints how to reopen it
(`session <id> saved · resume with --session <id>`).

An interactive REPL at a color terminal gets a display-only themed surface: a
green `No0B-CL1` banner, role-colored activity and notes, a red error accent,
a colored prompt marker, and a light tint on streamed assistant text so a human
can tell the model's words from their own echoed input. The tint opens once per
message and keeps the text contiguous (a streamed marker is never split by an
escape); it is always reset before the next prompt.

Thinking scanner (scanner.rs): on that same themed surface a small green square
comet sweeps on its own line while a turn is in flight, from dispatch until the
first output byte, so the request-to-first-token gap is not dead air. It is off
the inference path by construction: the comet animates on a side thread that only
writes to stdout while the main thread is blocked on the request, and it is torn
down (the thread joined, its line cleared) before the first reply byte is
written, so the two never interleave and throughput is untouched. It never reads
or mutates any request, transcript, or model state. Themed REPL only: a piped
REPL, `exec`, `--json`, and `child` never start it, so their bytes are unchanged.

Every other surface stays byte-for-byte what it was: a piped REPL, `exec`,
`exec --json`, and `child` stream model text raw, render tool activity as a
single dim line, and emit no color. Color is gated on `Mode::Repl` plus a color
flag (interactive tty, `NO_COLOR` unset, a real color depth), never the bare
terminal flag, so `exec` at a tty does not leak escapes. `NO_COLOR` drops color
without hiding reasoning or changing exec-mode stderr. One swappable `Theme`
holds every color/glyph/wordmark token; `matrix` ships now, `NOOB_THEME` later.

The color depth ladder is truecolor, then 256-color, then 16-color, then none.
Rendering is display-only: it never rewrites request bodies, the session log,
the raw transcript, or the JSONL protocol.

Slash commands, complete v0.1 set: /plan, /go, /status, /compact, /quit.

`exec --json` emits one JSONL event per loop step; that stream plus exit codes
is the whole integration surface for wrappers (Telegram bridge, other agents).
The JSONL protocol line and the child single-line JSON result are never colored,
formatted, or animated.

Child mode (`noob child`, P6): stdout belongs to the single JSON result line, so
assistant text AND activity stream to stderr as parent-relayable progress. There
is never a TTY, so confirmations always deny.
