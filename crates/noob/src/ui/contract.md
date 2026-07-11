# noob/src/ui

Rendering for the four surfaces (P2), themeable, plus a line editor for the
interactive prompt (on by default at an interactive terminal; `NOOB_RAW=0` falls
back to cooked). No TUI framework and no line-editor crate: escapes and the
termios editor are hand-rolled over `libc` (zero new crates).

Input (prompt.rs): at an interactive terminal the REPL runs a small raw-mode
line editor. An idle prompt is just the bare green `› ` marker; the first
keystroke expands it into a green input line framed by a top and a bottom rule
(no rounded corners and no side borders, only the two horizontal lines, so the
frame stays minimal). It offers real editing (insert, backspace across a
multibyte char, Ctrl-A/E/U/K/W, left/right/Home/End, Delete, bracketed paste
that holds newlines until a real Enter). On submit the frame collapses to a
compact `› message` line, so history reads as a list of sent messages, not a
stack of frames. The live input is shown as a one-row horizontal window of the
buffer (a long line scrolls instead of wrapping), so the frame stays exactly
three rows and every in-place redraw stays exact. Widths are counted one column
per character (no unicode-width table, by design), so runs of double-width CJK or
emoji are the one case that can still spill a row; the buffer and submitted line
are always correct. The frame is first drawn on the keystroke that expands it, by
which point a freshly spawned pty has reported its real width (so there is no
narrow first box to snap), and it re-fits to the terminal width on each later
keystroke (a bare ioctl on the read path already taken): a live resize is tracked
the same way. No timer, no idle loop, and no SIGWINCH handler (signals are left
exactly as they were, so nothing new can touch the request path). It is off the
inference path: raw mode is entered only while typing and restored to cooked
before the agent runs, so keystrokes never reach the model (it sees the message
once, on Enter). Three hooks restore the terminal so a crash never leaves the
shell raw: the RAII guard on the normal return, a panic hook (release is
`panic = "abort"`, so `Drop` does not run on a panic), and the SIGINT handler
before its `_exit(130)`. EOF (Ctrl-D) exits; Ctrl-C at the prompt cancels the
line and reprompts, kept distinct. `NOOB_RAW=0` forces the cooked reader. Piped
or headless input falls back to the exact cooked `read_line`, so those surfaces
stay plain: no box, no bracketed-paste toggles, no
escapes.

The REPL persists its session (a fresh id, or `--session <id>` to resume a
closed one) and, on exit at a terminal, prints how to reopen it
(`session <id> saved · resume with --session <id>`).

Dock (dock.rs): an opt-in interactive mode (`NOOB_DOCK=1`) that keeps the input
frame live while the model streams, output scrolling above it. It holds raw mode
for the whole session (not just while typing) and inverts the turn: one thread
(the render loop on the main thread) is the sole terminal writer, a stdin reader
thread decodes keys, and the turn runs on a worker thread that renders through a
`Ui::for_turn` whose sinks ship every styled byte over one `std::sync::mpsc`
channel, so the rendering and the agent loop are byte-identical to the direct
path. Typing during a turn edits a draft (no keystroke reaches the model until
Enter); Enter queues the draft to dispatch as the next turn; a double-tap ESC
(first arms a red "press ESC again" window, second commits) or a single Ctrl-C
cancels via the existing `INTERRUPTED` watchdog; an interrupt drains the queue
back into the editor. A mid-turn confirmation is answered through the channel
(never a second stdin reader). When `NOOB_DOCK` is unset the REPL uses the
per-prompt editor above, unchanged, and every non-interactive surface is
untouched either way. This is the two-writers-one-terminal design in `fable.md`.

Skills on the fly is a REPL command family (`/skills list|reload|add|remove`),
not a `ui/` concern: the command lives in `main.rs` and the reconciliation
(re-discovery, mid-session `skill`-tool registration, the in-band `[skills
updated]` announcement) lives in `agent/` and `skills/`. The dock only renders
its notes.

An interactive REPL at a color terminal gets a display-only themed surface: a
green `No0B-CL1` banner, role-colored activity and notes, a red error accent,
a colored prompt marker, and a light tint on streamed assistant text so a human
can tell the model's words from their own echoed input. The tint opens once per
message and keeps the text contiguous (a streamed marker is never split by an
escape); it is always reset before the next prompt.

On that surface each tool activity line tints its leading word its own per-tool
accent and pads it to a fixed column, so a scan of the transcript reads by color
and by column before the eye parses the word: `bash`, `read`, `edit`, `grep`, a
loaded `skill`, and the rest each hold a distinct muted hue. The color is keyed
off the line's leading word (the tool name every start line and summary already
begins with), so the render stays inside `ui/` with no change to what the agent
passes; a summary's past-tense wording (`edited`, `wrote`) folds back onto its
tool so a done line matches its start line, and any unplaced word hashes to a
stable palette slot rather than one flat color. A failed line is the exception:
it is drawn the error accent end to end, no label split, so red stays the single
reserved failure color. The palette is muted to sit with the matrix green and is
a theme token like every other; no test keys on a color value, only on the label
being isolated, padded, and reset. Every non-themed surface (a piped or NO_COLOR
REPL, `exec`, `--json`, `child`) emits the exact plain bytes: `* line` plain, or
`{DIM}* line{RESET}` at a plain tty, with no padding and no palette.

Thinking scanner (scanner.rs): on that same themed surface a small square comet
(a vivid green head trailing a faded-green tail) sweeps on its own line while a
turn is in flight, from dispatch until the first output byte, so the
request-to-first-token gap is not dead air. It is off
the inference path by construction: the comet animates on a side thread that only
writes to stdout while the main thread is blocked on the request, and it is torn
down (the thread joined, its line cleared) before the first reply byte is
written, so the two never interleave and throughput is untouched. It never reads
or mutates any request, transcript, or model state. Themed REPL only: a piped
REPL, `exec`, `--json`, and `child` never start it, so their bytes are unchanged.

Every other surface stays plain: a piped REPL, `exec`,
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
