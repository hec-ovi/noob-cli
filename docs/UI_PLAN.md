# noob-CLI interface and visuals plan (0.2.x series)

Engineering tracker for the REPL visual/interface uplift. This is the pending
list: when a version ships and Hector verifies it, its entry is WIPED from the
"Pending" section (checked off in the README instead) so pending stays clean.

## How we run this (per-version protocol)

At each version checkpoint:
1. Delivery is perfect and self-contained: rebuilt (`./dev.sh docker`) and
   `./dev.sh test` green, with all state written to durable stores (this file,
   agent memory, commits), because Hector compacts the agent at each checkpoint.
2. Hector tests in the real REPL and gives feedback; fixes applied if needed.
3. On approval: mark the version DONE in agent memory (survives compaction),
   wipe it from this file's Pending section, check it off in the README.
4. Commits and push flow mid-version too, even untested; README kept current.
5. Each shipped version is a git tag `v0.2.N`.

## Locked decisions

- Scope: aesthetics/interface only. The LLM system prompt and the agent's
  generalistic identity are NOT touched (that is mechanics, deferred).
- Deferred, not planned here: on-the-fly skill/plugin install (both the
  user-command form and agent self-install).
- Brand and theme: wordmark `No0B-CL1`; matrix green ramp (truecolor with
  256-color then 16-color fallback, off when piped); red is the only non-green
  accent (errors), faint amber optional (warnings). Square comet thinking
  scanner (`the middle small square`, green-ramp tail). One swappable `Theme`
  struct, all colors/glyphs/wordmark as named tokens, default `matrix`, future
  `NOOB_THEME` swaps the whole look. Ship only `matrix` now.
- Versioning: one `0.2.N` per version, tagged. The 0.1.0 release and the live
  gauntlet stay reserved for Hector.
- Order: foundation (v0.2.0) first, then the boxed raw-mode input (v0.2.1), then
  the rest.

## The zero-overhead law (hard requirement)

No performance or throughput cost, in any direction, proven per version:
- Keystrokes never reach the LLM. The model sees the message once, on Enter.
- Slash autocomplete is a local string-match over a ~7-item static command list:
  no model, no network, no tokens.
- Raw-mode input is restored to cooked BEFORE the agent runs, so it is off the
  inference path. Prefill/decode tokens-per-second are GPU/llama.cpp bound and
  untouched.
- All visuals are display-only string formatting on text noob already has, and
  are gated to an interactive TTY.

## Global invariants (every version inherits these)

- Zero new runtime crates. Hand-rolled ANSI/editor/markdown over the `libc`
  already linked. Budget: runtime crate graph <= 45 (currently 40), stripped
  binary <= 8 MB (currently ~3.39 MB). `./dev.sh size-check` gated on the
  code-heavy versions.
- Piped / `exec` / `--json` / child output stays byte-identical to today. All
  richness is gated on `Mode::Repl && color` (a `styled()` helper), never the
  bare terminal flag. The `--json` JSONL protocol and the child single-line
  stdout result are never colored, formatted, or animated.
- Display-only: rendering never rewrites request bodies, session JSONL, the raw
  transcript, or the append-only cache-prefix.
- Confined to `crates/noob/src/ui/` plus minimal `main.rs` wiring, with exactly
  one flagged additive `agent/mod.rs` borrow param (v0.2.5, JSON result bodies).
  No refactor of `agent/` or `noob-provider/`.

## Hardening notes carried from the adversarial review (do not lose)

- v0.2.0 must add a real test seam FIRST: an injectable writer sink plus a way
  to force mode+ansi independent of `is_terminal()`, and every renderer a pure
  `string -> string` function, or the styled path gets zero offline coverage
  (the suite runs piped, so `ansi=false`).
- Keep terminal-detection separate from a color flag: `NO_COLOR` disables color
  but must NOT make reasoning text or the scanner vanish, and must not change
  exec-mode stderr. Honor `NO_COLOR` only when present and non-empty.
- Assistant tint must reset unconditionally when tinted (a message ending in
  `\n` currently no-ops `end_line`, so color would bleed into the next prompt).
- The color-open, RESET, and the scanner erase live in a `text_delta` arm shared
  by Repl and Exec: gate them strictly on `styled()` and on the spinner handle
  existing, or they leak into `noob exec` at a TTY. Add a byte-identical test for
  Exec-at-a-TTY.
- Never wire a spinner call into `event()` (the JSONL emitter is sacred).
- v0.2.1 raw editor: restore the terminal at three sites (RAII guard, panic hook,
  SIGINT before `_exit(130)`) because release is `panic="abort"`; enable and
  disable bracketed-paste (`ESC[?2004h/l`) at those same sites; handle `0x03`.
- The prompt reader must distinguish EOF (exit) from Ctrl-C-at-prompt (reprompt);
  do not collapse them into one `None`.
- v0.2.5 JSON pretty-print: gate to `mcp_*` result bodies (not every JSON), and
  sanitize embedded ANSI/control bytes from untrusted MCP output.

## Pending versions

### v0.2.0 - Theme foundation + role colors + test seam  (risk: low)
Status: BUILT. `./dev.sh test` green (215 unit + all e2e, incl. the two PTY
styled-path tests); 4-lens adversarial review passed with 0 findings. Awaiting
Hector's REPL test; on approval this entry is wiped and the README box checked.
Tests are error/regression-oriented (byte-identity, bleed, marker-split, panics),
never asserting a theme color, so live color retuning does not break the suite.
Goal: one hand-rolled `Theme`/style layer (SGR + green ramp with fallback),
role colors on the discrete surfaces (prompt marker, tool activity, notes,
errors) and a light assistant tint, the `No0B-CL1` banner, and the test seam.
Files: `ui/style.rs` (new), `ui/theme.rs` (new), `ui/mod.rs`, `main.rs`.
Arch: amends the "dim single lines" / "raw" wording (consolidated amendment).
Tests: pure `paint()`/theme unit tests; `styled()` false for Exec/ExecJson/Child;
`NO_COLOR` emits no SGR but keeps content; prompt marker byte-identical when not
styled; existing ui tests stay green.
Manual: TTY shows banner + distinct role colors + assistant tint; `exec`,
`--json | cat`, and piped REPL are byte-identical to the old binary; `NO_COLOR=1`
shows no color but nothing disappears.

### v0.2.1 - Boxed raw-mode input (the strong two-line prompt)  (risk: high)
Goal: replace cooked input with an opt-in termios editor that draws a two-line
framed green input box, real line editing, restored on every exit; cooked
fallback when piped/headless. Includes extracting the prompt reader into its own
`ui/prompt.rs` component first.
Files: `ui/prompt.rs` (new), `ui/mod.rs`, `main.rs`.
Arch: amends row 5 ("cooked-mode; no line editor"); pulls the v0.2 termios editor
forward (consolidated amendment).
Zero-overhead: editor active only while typing; cooked restored before
`run_input`; measure per-keystroke render cost and assert inference throughput is
unchanged with editor on vs off.
Crash scope (owner): losing an unsubmitted in-progress line on a crash is fine;
do not gold-plate. A submitted line is logged to the session JSONL on Enter
(push_item, before the model replies), so it is never lost and a crash after
submit is resumable. Keep the three terminal-restore hooks only so the shell is
not left raw; nothing more.
Tests: editor buffer as a pure function over scripted keys (insert, backspace
across a multibyte char, Ctrl-A/E/U/K/W, cursor moves); `0x03` returns interrupt,
EOF returns None distinctly; no-TTY falls back to cooked and is byte-identical;
panic hook installed and calls the restore fn.
Manual: type/edit/submit; multi-line paste inserts literally; Ctrl-C reprompts
with a sane terminal, second exits sane; forced panic leaves the terminal cooked;
piped REPL byte-identical.

### v0.2.2 - Slash-command Tab-completion + ghost-text + history  (risk: medium)
Goal: Tab completes the current `/`-token against a single command-table source
of truth (also feeds the banner and the unknown-command list); dim ghost-text of
the completion; optional in-session Up/Down history. Interactive TTY only.
Files: `ui/prompt.rs`, `main.rs`, `ui/mod.rs`.
Zero-overhead: string-match over a small in-memory command registry, no
model/network/tokens.
Dynamic (design for it now): the command table is a RUNTIME registry (single
source of truth), not a compile-time const, so the future dynamic tool/command
install can register a `/newtool` and have it auto-appear in completion, the
banner, and the unknown-command list. Forward-compatible with the deferred
dynamic-install feature.
Tests: matcher as a pure function (`/pl`->`/plan `, `/`->lists all, unknown->none);
one table drives dispatch + banner + unknown-command; no completion on the cooked
path; Up then Enter re-submits.
Manual: `/pl`+Tab, `/`+Tab lists; Tab inside a paste does nothing; history cycles;
piped shows no completion and is byte-identical.

### v0.2.3 - Thinking scanner (square green comet)  (risk: low)
Goal: cover the request-sent-to-first-token wait with a background-thread scanner
(small square head, green-ramp tail) on a bracketed track, erased cleanly before
the first byte; strict no-op in every non-interactive mode; never wired to
`event()`.
Files: `ui/scanner.rs` (new), `ui/mod.rs`, `main.rs`.
Tests: begin+stop emit zero bytes for Exec/ExecJson/Child and ansi=false; stop is
idempotent and prompt; erase precedes the first delta byte.
Manual: scanner sweeps then vanishes with no residue; clears before a tool line;
Ctrl-C mid-wait clears cleanly; `exec`/`--json` byte-identical.

### v0.2.4 - Per-tool colored activity lines  (risk: low)
Goal: stable per-tool color/glyph on the `* tool ...` line, error lines flagged;
same single-line shape; byte-identical when not styled.
Files: `ui/mod.rs`, `ui/style.rs`.

### v0.2.5 - Pretty colored JSON for tool/MCP result bodies  (risk: medium)
Goal: show mcp_* result bodies as pretty colored JSON in the REPL (net-new
surface), via one additive borrow param into `tool_done`; display-only,
sanitized, gated to mcp_* and styled.
Files: `ui/json.rs` (new), `ui/mod.rs`, `agent/mod.rs` (one additive param).
Arch: the single flagged `agent/mod.rs` file-boundary exception + line 297
carve-out (consolidated amendment).

### v0.2.6 - Inline markdown-lite on assistant text  (risk: medium)
Goal: style headings/bold/italic/inline-code/lists per completed line, live;
raw passthrough for every non-interactive mode; no full-message re-render.
Files: `ui/markdown.rs` (new), `ui/mod.rs`.
Arch: reverses "no markdown rendering" for interactive TTY (consolidated
amendment).

### v0.2.7 - Fenced code blocks  (risk: medium)
Goal: buffer a fenced block, render each line verbatim behind a dim gutter with
an optional lang label; incomplete fence flushes raw; coarse dim tint only.
Files: `ui/markdown.rs`, `ui/mod.rs`. Gate `./dev.sh size-check`.

### v0.2.8 - Markdown tables  (risk: high)
Goal: zero-dep box-drawing table renderer (column widths on visible text, SGR-safe
math, ioctl width, greedy wrap, ASCII fallback); buffer the whole table before
layout. Gate `./dev.sh size-check`.
Files: `ui/table.rs` (new), `ui/markdown.rs`, `ui/mod.rs`.

## Consolidated ARCHITECTURE.md amendment (one sign-off, applied as each version lands)

Reframe the UI section: an interactive TTY gets a themed, colored, display-only
lightly-rendered surface plus an opt-in raw-mode line editor; piped / `exec` /
`--json` / child stays byte-identical raw. Specifically amends: row 5 "Cooked-mode
plain REPL; no ratatui, no rustyline" (to allow an opt-in zero-dep termios reader),
line 297 "streams model text raw ... no markdown rendering" (raw when piped;
display-only rendering at an interactive TTY), line 358 Cut list (markdown and the
line editor move to landed), `ui/contract.md`, and the `ui/mod.rs` header comment.

## Shipped

(none yet)
