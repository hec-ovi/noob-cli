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
- Slash autocomplete is a local string-match over a ~5-item static command list:
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

## Field notes from real-REPL testing (2026-07-10, Hector)

Standing priority Hector reaffirmed: stability, speed, and efficiency of the
harness itself come first; visuals never cost throughput (the zero-overhead law).

Observations from a live session, mapped to where each is fixed:
- The input box "disappears" while the agent is thinking/writing. This is the
  collapse design (no frame during processing), but it reads as empty. Hector has
  since asked for a persistent input dock that stays visible during a turn so the
  next message can be QUEUED while the agent runs, with double-ESC to cancel; that
  is the two-writers-one-terminal work tracked in the repl-architecture memory and
  `.research/rust-cli-concurrent-io-2026/`.
- Already-planned phases the session confirmed we need:
  - v0.2.3 thinking scanner (green squares): "no thinking animation".
  - v0.2.4 per-tool colors + padding: "each skill and command its own style and
    color, like padding"; the `* bash ...` activity lines are uniform dim now.
  - v0.2.6 inline markdown: "MD style has no colors, improve list colors"; the
    `**bold**` headers and `-`/`1.` lists render raw.
  - v0.2.7 fenced code blocks.
  - v0.2.8 tables: "check the tables how awful it looks"; a markdown table asked
    of the model renders as raw pipes and wraps badly.

## v0.3.x mechanics series (dock: queue, cancel, skills-on-the-fly)

Separate from the 0.2.x cosmetic series: this is the two-writers-one-terminal
work (architecture in `fable.md`, research in
`.research/rust-cli-concurrent-io-2026/`). One terminal owner (the render loop
on main), a per-turn worker rendering through a channel-backed `Ui`, a stdin
reader thread, all over one `std::sync::mpsc`. Zero new crates. Built behind
`NOOB_DOCK=1` (opt-in) until M7 flips the default, so the shipped REPL never
changes while the driver is proven. Mini-milestones so each is a green,
self-contained deliverable (Hector is credit-constrained and compacts often).

- M1-M6, M8 BUILT + Docker-green + LIVE-validated (behind `NOOB_DOCK=1`, NOT
  committed). `ui/dock.rs`: the Ev channel, the `OutTracker` column tracker, and
  `DockSession` = a session-lifetime `RawGuard` + a stdin reader thread + an
  event-driven `read_prompt` + a `run_turn` scoped worker + a coalesced render
  loop that owns the terminal. `Ui::for_turn` renders a turn through channel
  sinks (byte-identical to the direct `Ui`), so `agent/` is untouched; `ask`
  reroutes over the channel; the scanner thread is retired (the dock row shows
  the comet).
  - M5 double-ESC cancel: a first ESC arms a 5 s window (the input row turns a
    red "press ESC again to cancel"); a second ESC commits via `INTERRUPTED`;
    Ctrl-C commits immediately; any other key or the window lapsing disarms.
  - M6 message queue: Enter during a turn queues the draft (a dim "N queued"
    row) and it dispatches as the next turn; an interrupt drains the queue back
    into the editor instead of firing it.
  - M8 skills on the fly: `/skills list|reload|add <path|git-url>|remove <name>`.
    A reload re-discovers, swaps the live set, registers the `skill` tool on the
    zero-skills-to-some transition (one accepted cache break), and appends an
    in-band `[skills updated]` message that supersedes the frozen head index
    (naming added skills with descriptions and removed ones); a removed skill is
    also rejected structurally by the `skill` tool. Compaction pins the current
    set when it drifted, so the correction outlives the summarized note.
  - An adversarial review of the foundation found and fixed three real bugs
    (uninterruptible `/compact`, an ask-modal EOF deadlock, and an exact-fill
    deferred-wrap line erase). A live drive against qwen3.6 at :8090 exercised
    queue, ESC-cancel, and the full skill add/use/remove/staleness loop; the
    reconstructed terminal screen is clean.
- M7 REMAINING (Hector's call): flipping `NOOB_DOCK` to default-on stays for
  Hector's own real-REPL drive (the per-version sign-off protocol, his stability
  zone). On his OK: flip the default and rewrite the prose the dock makes false
  in that mode (raw mode spans the turn, `ui/` is on the render path, a per-turn
  worker thread exists). Until then the dock is opt-in and the default per-prompt
  editor is unchanged.

## Pending versions

### v0.2.1 - Boxed raw-mode input (the strong two-line prompt)  (risk: high)
Goal: replace cooked input with a default-on termios editor (`NOOB_RAW=0` opts
out) that draws a framed green input box, real line editing, restored on every
exit; cooked fallback when piped/headless. Includes extracting the prompt reader into its own
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
(small square head, faded-green tail) on an indented track, erased cleanly before
the first byte; strict no-op in every non-interactive mode; never wired to
`event()`.
Files: `ui/scanner.rs` (new), `ui/mod.rs`, `main.rs`.
Tests: begin+stop emit zero bytes for Exec/ExecJson/Child and ansi=false; stop is
idempotent and prompt; erase precedes the first delta byte.
Manual: scanner sweeps then vanishes with no residue; clears before a tool line;
Ctrl-C mid-wait clears cleanly; `exec`/`--json` byte-identical.

### v0.2.4 - Per-tool colored activity lines  (BUILT, awaiting Hector's REPL test)
Goal: stable per-tool color on the `* tool ...` line, error lines flagged; same
single-line shape; byte-identical when not styled.
Built: on the themed surface each activity line tints its leading word (the tool
name) a distinct muted accent from a 10-color palette and pads it to a 7-column
field so the briefs line up down the transcript. Color is keyed off the line's
leading word, so the render stays inside `ui/` with no change to what the agent
passes (no `agent/`, no `main.rs`); a summary's past-tense word (`edited`,
`wrote`) folds back onto its tool so a done line matches its start line, and an
unplaced word hashes to a stable palette slot. A failed line stays the error
accent end to end (red reserved for failures, no label split). Palette is a
theme token; no test keys on a value.
Files: `ui/theme.rs` (palette + `label_style`), `ui/mod.rs` (`activity_line`),
`ui/contract.md`.
Tests: label mapping is stable, distinct across the core tools, and normalizes
past tense; styled line isolates + pads + resets the label; error line stays
whole-line red (no split); piped and NO_COLOR/dim and exec bytes byte-identical
to pre-v0.2.4 (no padding, no palette off the themed surface). Full offline suite
green, zero warnings, no new crate.
Manual for Hector: run a task that touches several tools (a `read`, a `bash`, an
`edit`, a `grep`, load a `skill`); each `* tool` word should read in its own hue
with the briefs aligned in a column; a failing tool line should read red end to
end; `noob exec ... 2>&1 | cat` and a piped REPL should look exactly as before.
Open sub-item to weigh at the checkpoint: whether slash commands (`/plan`, `/go`,
...) should also get a colored, aligned echo line, or stay as note-colored
feedback (they already read distinct from tool activity today).

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

- `v0.2.0` Theme foundation + role colors + test seam. `ui/style.rs` (SGR +
  color-depth ladder), `ui/theme.rs` (matrix Theme + banner), `ui/mod.rs`
  rewritten with a `Box<dyn Write>` seam and a tint-open-once state machine,
  `main.rs` wired. Interactive-REPL-only; all other surfaces byte-identical.
  4-lens adversarial review = 0 findings; Hector verified in the real REPL
  (colors retuned to a muted mid-green band). Commits `51c0a30`, `0c7cf0d`.
  Crate version left at `0.1.0`; not yet tagged (Hector's call).
