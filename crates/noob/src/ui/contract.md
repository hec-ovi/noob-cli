# noob/src/ui

Rendering for the four surfaces (P2), now themeable. No TUI framework and no
line-editor crate: the terminal provides line editing; escapes are hand-rolled
over `libc` (zero new crates).

An interactive REPL at a color terminal gets a display-only themed surface: a
green `No0B-CL1` banner, role-colored activity and notes, a red error accent,
a colored prompt marker, and a light tint on streamed assistant text so a human
can tell the model's words from their own echoed input. The tint opens once per
message and keeps the text contiguous (a streamed marker is never split by an
escape); it is always reset before the next prompt.

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
