# CLIppy plan

Every item in `featuresandbugs.md`, graded against the code as it actually is.
Effort figures come from reading the files, not from estimating by feel.

Every load-bearing claim below was checked against the source: the missing
right-button path, the unread activation token, the free `extra` slots, the
`sandbox_label` that never reaches the wire, the absent tool schemas, and the
running-sum rates that make a median impossible. No claim failed verification.

Two numbers worth holding: `gui/` is 9,660 lines (about 100k tokens), and the
whole repo is near 700k because the CLI in `crates/` is 56k lines. Almost
everything below lands in `gui/`, and the two halves talk only through the event
enum in `noob-proto`, so this work never requires holding the CLI in your head.

## Phase 0: structural, do these first

Nothing here is a feature. Each one either makes several later items cheap, or
makes them get rebuilt if skipped.

### 0.1 Split `gui/` into contract-isolated layers

`gui/clippy/src/` is eight modules that all reach into each other. `view.rs`
alone is 2,006 lines and reads `state`, `dock`, `skin`, `markdown`, `syntax` and
`select` directly. That is why the rule for turning a line of text into rows on
screen is currently written out in eight places and disagrees with itself in
three of them.

Proposed layers, each a blackbox with `CONTRACT.md`, `schema/`, `tests/` and a
private `src/`:

| Layer | Owns today | What its contract fixes |
|---|---|---|
| `text` | `markdown.rs`, `syntax.rs`, `select.rs`, wrap math | one owner for wrap, which closes items 3 and 4 |
| `layout` | `dock.rs` + the `Layout` half of `view.rs` | the split tree, items 22, 10, 21, 23 |
| `render` | `noob-draw`, `noob-gpu` | primitives: chamfer, stroke, orb discs, icons, text caching |
| `session-state` | `state.rs`, `monitor.rs` | metrics, items 6, 7 |
| `session-link` | `link.rs` + the new socket | items 27, 28, 29 |
| `settings` | `config.rs`, `skin.rs` | items 11, 19 |
| `animation` | `avatar.rs` + the orb | items 2, 14 |

Plus `docs/INDEX.md` mapping "the thing you want to change" to the one folder to
open.

### 0.2 Panes count visual rows, not logical lines (item 4)

`state.rs`, `view.rs`, `main.rs`. About 120 lines in `state.rs`, 40 in
`view.rs`, 15 in `main.rs`, 80 of new tests.

Every pane hands the shaper exactly as many logical lines as rows fit. Any line
wider than the pane wraps to two or more visual rows, the buffer overflows its
clip box, and the overflow (the newest text) is discarded with no scroll
position that can reach it, because `scroll_forward` floors at 0. The scrollbar
lies about it for the same reason.

Wrap at draw time, not at push time: `Selection` anchors to absolute line
numbers that survive eviction, and `Pane::stream` appends to the tail line on
every token, so wrapping on push means rewrapping on the hot path and
invalidating live selections.

The visual-row walk this adds is also what item 3's hit testing and item 7's
clickable rows need.

### 0.3 Overlay layer, right-click, and `Dock::hide`/`unhide` (item 25)

`view.rs` ~120, `main.rs` ~80, `dock.rs` ~40 with tests.

There is no right-button path anywhere in the program, and no way to remove a
view from a slot at runtime. Items 21, 12 and 24 all call into these two pieces,
so building them once avoids three partial versions.

### 0.4 Chamfer and stroke in the rect SDF (item 20)

`noob-draw/src/lib.rs`, `view.rs`. About 50 lines in `noob-draw`, 20 across
four call sites, one test rewrite.

`Rect.extra` is a vec4 with only slot 0 in use (radius), so chamfer size, corner
mask and border width fit in `extra.y/z/w` with no struct growth. This is also a
performance win: a bordered pane drops from 5 rects to 2, because the current
border is four separate 1px rects. Landing it before items 12 and 16 means
panels and tabs get styled once instead of twice.

All four slots are spent after this, so the packing scheme has to be decided
now. The module doc records that a Rust/WGSL layout mismatch once silently
corrupted every instance after the first.

### 0.5 Embedded symbol font and a generic animation wakeup (item 13)

About 20 lines in `noob-draw` plus a subset font file; the animation wakeup is
~20 lines in `about_to_wait`.

The cheap shape is a cosmic-text `Fallback` impl behind a preloaded font db,
replacing the bare `FontSystem::new()` at `noob-draw/src/lib.rs:467`. That makes
icon codepoints resolve through the existing `Family::Monospace` runs, so not a
single call site changes.

No nerd font exists on this machine, which is exactly why the title bar marks
are drawn as rectangles today. Without the icon route, items 5, 12 and 16 will
each hand-draw throwaway rect marks. Embed the font rather than rely on the
system, or the same bug returns for the first user without it.

### 0.6 Say which isolation mode the agent is in (item 27)

`noob-proto`, `crates/noob/src/main.rs`, `state.rs`, `view.rs`. About 40 lines
across five files in two workspaces, plus round-trip and fold tests.

Concretely: add `sandbox: String` to `Event::SessionStart` and fill it from the
`sandbox_label` the CLI already computes at `crates/noob/src/main.rs:139`, which
today only reaches the model's system prompt and never the wire. Then fold it
into `State` and display it.

`~/.local/bin/noob` is a native ELF right now, no `noob:local` image exists, and
the build cache is empty, so the window is silently driving an unsandboxed host
agent. `install.sh` will not repair it by itself: it looks for the string
`noob-cli managed Docker launcher` in the destination, does not find it in an
ELF file, and exits refusing to replace. Run `./install.sh --force`.

Must land before item 28, because that socket hands the same authority to
anything that can write to it.

### 0.7 Split tree replacing the fixed three-slot dock (item 22)

`dock.rs`, `view.rs`, `main.rs`, plus a new layout persistence file. About 900
to 1,100 lines touched.

`dock.rs` is three hard-coded named spaces and its own module doc argues at
length that a splitter tree is the wrong design, so the file stops lying about
itself only when it is rewritten. `Dock` derives `PartialEq` and a test compares
whole docks, so ratios need to be integers or permille rather than f32.

Items 10 and 23 have nowhere to land without it. Gated on decision D1 before any
code gets written.

## Easy wins

- **1. Launch spinner and missing dock icon.** ~10 lines in `resumed()` and
  `window_attributes()`. winit 0.30.13 already ships `startup_notify` ungated
  and nothing reads the token, so GNOME spins until its own 15 second timeout,
  which is exactly what you see. The same pass fixes `Exec=clippy %f`, which
  passes a file where a directory is wanted.
- **9. Black block and green lines on double click.** ~15 lines. It is the
  shaded state painting a full-window backdrop, because `shade()` asks for a
  30px window against a 680x380 minimum. Fill only the title strip and drop the
  minimum inner size while shaded.
- **17. Hardware bars as four-dot columns.** ~60 to 90 lines inside `gauges()`.
  The renderer already draws a clean disc through `Panel::fill(..).radius(w/2)`,
  so there is no new primitive. The real cost is finding vertical room in a 9px
  track, and one test finds bars by filtering rects larger than 2px, so it
  reports "no bars were drawn" until rewritten against dot geometry.
- **18. Hardware shows only bars.** ~40 lines, same function as item 17, so land
  them in one commit. The GPU capability report loses its only display surface.
- **8. Remove the footer.** ~35 lines deleted plus two test edits, and every
  pane gets 24px back. Blocked only on decision D3.
- **24. Remove the fold arrow.** ~45 lines, or ~90 if the folded state goes too.
  Must follow 0.3: until closing exists, folding is the only way to get a pane
  out of the way.
- **31. Drop the CLIPPY animation tab.** 16 references across `view.rs`,
  `config.rs` and `main.rs` plus the enum in `dock.rs`. Consistent, since that
  animation becomes the corner orb.
- **19. Wider palette, named themes, config writer.** ~120 lines. `opacity` and
  eight colors already parse and `Skin::from` is the single color source, so
  this is the 14 tool colors, the 5 syntax colors, presets, and a
  comment-preserving writer that can be ported from
  `crates/noob/src/config/mod.rs:79`.

## Medium

- **3. Selection precision.** ~150 lines plus ~60 of test edits. Three confirmed
  defects, not one: `spot_at` hit-tests with `pane_font_size` while Talk is
  drawn at `body_size`, Files is offset by the tab strip plus a 4-character
  gutter, and `cell` assumes nothing wraps. Fixed by putting the drawn size and
  column on `Placed`. The band, the hit test and the copy have to move together
  or the highlight and the clipboard disagree. Needs 0.2 and decision D2.
- **26. Input select-all, click-to-caret, configurable height.** ~140 lines.
  Rows and click-to-caret are nearly free. Ctrl+A is the real work, because the
  prompt has no selection model at all and there is no focus concept: `App::key`
  is the only text sink in the program.
- **6. Split the LLM monitor.** ~350 lines, most of it a new `totals.rs`. The
  view split and the session readings are cheap; persistence is new machinery
  for this crate, and the median needs a stored sample ring that the current
  `Rates` design cannot produce retroactively.
- **7. Debug pane for failed tool calls.** ~155 lines for the GUI half.
  `ToolStart` carries the full argument object and `ToolEnd` carries structured
  error fields, both rendered to text and then dropped, so keeping them on a
  `failures` list and expanding the clicked row is contained. The expected
  schema is the expensive half: the string "schema" does not appear in
  `noob-proto` at all. Adding `Event::ToolSpecs` must go in at wire v1 without
  bumping `VERSION`, because a v2 agent blanks a v1 window.
- **12. Tab color, accent line, close X.** ~15 lines for the color, ~90 more for
  the X. The color half can ship immediately; the X needs 0.3's `Dock::hide` and
  a reopen surface, or a user permanently deletes Talk with one click.
- **2 and 14. Thinking orb.** New `orb.rs` ~400 lines plus ~90 across
  `view.rs`, `main.rs` and `skin.rs`. No shader work: CPU math emitting discs
  through the existing SDF, 204 rects for the idle state and 516 for working.
  It does contradict the deliberate no-free-running-render stance in
  `noob-gpu/src/lib.rs:19` unless the idle state freezes. Decision D9.
- **Idle animation from `docs/asciis`.** 382 frames at 128x37, 24fps, a 15.9
  second seamless loop of the face. `avatar.rs` already plays exactly this
  format, and the size gate has 29 MiB free of its 40, so the asset is not a
  constraint. Keep the ramp characters in the file as the tone index and draw
  every cell as one dot tinted by its level, which needs no new renderer because
  per-run color already exists. Watch the cost: text is reshaped from scratch
  every frame, so ~4,700 tinted cells at 24fps wants buffer caching first.
- **10. Drop zones with a landing preview.** ~180 lines, after 0.7. With a fixed
  three-slot dock an edge drop has nowhere to create a split, so building it
  first means it degrades to "move to the nearest slot" and gets thrown away.
- **23. Classic preset and pinned input.** ~140 lines: ~15 for the preset after
  0.7 (~55 before), ~50 for the single-line scrolling input, ~20 of tests. The
  classic nesting is the inverse of today's hardcoded split, so after 0.7 it is
  a literal.
- **21. Orb launcher panel.** ~300 lines plus title-bar geometry rework. Much
  cheaper after 0.3 and after item 9, whose shade path it shares.
- **15. Startup folder picker.** ~280 lines. `connect()` has to take a path and
  become re-entrant either way. This also fixes two live bugs: the dock launch
  silently hands the agent `$HOME`, and `Exec=clippy %f` passes a file where a
  directory is wanted, which `packaging.rs:41-51` asserts against.
- **28. The `noob` bridge over a unix socket.** ~250 lines, no new crates, std
  has `UnixListener`. A listener on `$XDG_RUNTIME_DIR/clippy/<workspace>.sock`
  reads `noob_proto::Command` frames into a second channel, funnelled through a
  split-out `submit_text` so a socket prompt and a typed prompt take the same
  path. `PromptSubmit` and `PromptQueue` already exist in the protocol and
  `noob serve` already handles them; what is missing is an address. Bare `noob`
  attaches to the active CLIppy session for the current workspace, an explicit
  resume flag targets a named one. Follows 0.6.
- **29. Session management in settings.** Sessions are plain files at
  `~/.config/noob/sessions/`. `SessionList` and `SessionOpen` are declared in
  the protocol but have no handler anywhere in `crates/noob/src/`, no event
  carries a list back, and there is no delete command. Three additive pieces
  plus the panel, and it shares its plumbing with item 28, so build them
  together.
- **30. File-type icons.** Symbols Nerd Font carries the Seti glyph set that VS
  Code's explorer uses, at 2 MiB against 29 MiB of headroom. Falls out of 0.5.

## Hard

- **5. Files as an explorer tree.** ~600 to 800 lines: a tree module ~300, a
  scan thread ~80, `view.rs` ~200, `main.rs` ~60, plus 0.5's icon route. A new
  subsystem, since nothing in the GUI walks a directory today, and nothing
  watches the filesystem, so the tree goes stale the moment the agent writes a
  file. Decision D4 sets its shape.
- **11. Config panel.** ~800 lines of GUI (including a focus and widget layer
  that does not exist), ~150 in `noob-proto`, ~120 in `noob serve`, plus four
  missing CLI features. Four of the seven rows you asked for have no
  implementation at any layer, `SkillList`, `McpList` and `McpState` are
  declared in the protocol with no producer anywhere in the CLI, and two rows
  contradict the documented secrets stance in `crates/noob/src/config/mod.rs:16`.
  This is a CLI feature set with a GUI on top, not a GUI task.

## Decisions

### Settled

- **D1 (item 22): capped 2x2 grid.** Max depth 2, at most 4 leaves, dividers
  drag freely. No arbitrary nesting, so no leaf can collapse below a drawable
  size. Roughly half the code of a general tree.
- **D9 (item 2): own 66px block at top-left, above the panes.** Idle plays the
  ASCII face loop, the orb animates only while the agent is working, and the
  wakeup stops at the end of a turn. Keeps the no-free-running-render rule in
  `noob-gpu/src/lib.rs:19-24` intact.
- **D22 (item 20): top-right corner only, 10px, on every panel** including tabs
  and the input box.

### Still gating

- **D8 (item 28).** Does a browser page reach the agent directly, which means
  CLIppy grows an HTTP listener with origin and token checks, or does your site
  talk to a broker you run that holds the socket? And does the caller need the
  answer streamed back in v1?
- **D24 (item 11).** Do API keys and NordVPN credentials get typed into CLIppy
  at all, given the CLI deliberately keeps secrets out of settable config and
  out of the process environment? And is enabling a skill per-workspace or
  global?

These can be answered as their item comes up:

- **D2 (item 3).** Does copying from Talk give the raw Markdown you could paste
  back into a file, or the rendered text you see on screen?
- **D3 (item 8).** The context gauge and the token budget live only in the
  footer: gone, moved to the title bar, or into the LLM monitor?
- **D4 (item 5).** Tree replaces the touched-files panel, sits beside it as a
  second column, or becomes a ninth view? Hide dotfiles and gitignored paths?
- **D5 (item 21).** Does the launcher list float over the panes (cheap) or push
  them aside as a real left rail (recomputes layout)?
- **D6 (item 7).** Are the failing call's arguments enough for v1, or is the
  expected schema required (a protocol change plus a CLI edit)?
- **D7 (item 15).** In-window picker drawn with rects and text, or a native
  portal dialog through rfd (dozens of new crates)? Does passing a folder on the
  command line skip the picker?
- **D10 (item 16).** Confirm the visual set is: orb top-left, four-dot gauges
  with hardware notes stripped, cut corner on every panel, tabs that read as
  blocks. If yes, item 16 dissolves into those four.
- **D11 (item 24).** Does `folded` disappear entirely, or does the arrow go
  while clicking the already-active tab still collapses its space?
- **D12 (item 25).** What rows are on the context menu? Minimum is Close;
  candidates are Copy, Select All, Split here, Move to.
- **D13 (item 10).** "Only allow splitting there" means a depth cap (never
  re-split the whole window), or only splitting inside the leaf under the
  pointer?
- **D14 (item 23).** In classic, where do activity, hardware, llm and files go:
  tabbed behind plan and agents, or hidden? Is classic a one-shot rearrange or a
  locked mode?
- **D15 (item 6).** Is "overall" all-time across every workspace or
  per-workspace? Backfill from `~/.config/noob/sessions/*.jsonl`? Both average
  and median?
- **D16 (item 17).** Confirm the dot grid: 10 columns of 4 dots covering 0 to
  100 percent, so 52.5 percent is 5 full columns plus 1 dot. The arithmetic in
  your note does not close as written: 10 columns at 10 percent each cannot make
  "10.5" into ten full columns plus one at two dots, that is 105 percent.
- **D17 (item 18).** Does "only bars" also drop the row labels and numeric
  readings? Should the GPU capability report move to the activity pane at
  startup, or vanish?
- **D18 (item 26).** With both a pane selection and a prompt selection live,
  which does Ctrl+C take? Should Ctrl+A over a pane select the pane?
- **D19 (item 19).** Is another block of keys in `clippy.conf` enough, or must
  the palette be editable in-app (which makes it wait on item 11)?
- **D20 (item 12).** Close X on every tab, only the active one, or only on
  hover? Always-on costs 2 columns per tab and will drop a tab off the 5-tab
  strip at narrow widths.
- **D21 (item 13).** Which icon font, and subset at build time or check in a
  pre-subset ttf?
- **D22 (item 20).** Which corner gets the cut (top-right only?), at what size,
  and does it apply to the input box too?
- **D23 (item 27).** Should CLIppy refuse to drive an agent reporting Workspace
  mode, warn and continue, or just display it? Is `--network host` intentional
  for your local model server?
