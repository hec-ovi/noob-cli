# noob/src/ui

Cooked-mode REPL and the headless event surface (P2). No TUI framework, no
line-editor crate: the terminal provides line editing; model text streams raw
to stdout; tool activity renders as single dim ANSI lines; ANSI switches off
when piped.

Slash commands, complete v0.1 set: /plan, /go, /status, /compact, /quit.

`exec --json` emits one JSONL event per loop step; that stream plus exit
codes is the whole integration surface for wrappers (Telegram bridge, other
agents).

Child mode (`noob child`, P6): stdout belongs to the single JSON result
line, so assistant text AND activity stream to stderr as parent-relayable
progress. There is never a TTY, so confirmations always deny.
