# Where to change what

Open one folder, not the repo. Find the thing you want to change on the left and
open only what the right column names.

## NO0B, the GPU window (`gui/`)

The folder is `gui/clippy` because that was the product's first name. The
package, the binary and everything a user sees are `no0b`.

| You want to change | Open |
|---|---|
| How a line of text becomes rows on screen: wrapping, scroll windows, selection bands, the scrollbar's extent | `gui/layers/text-geometry/CONTRACT.md` |
| Which panel sits where, tabs, drag and drop, where the two dividers sit | `gui/clippy/src/dock/CONTRACT.md` and the `Layout` half of `gui/clippy/src/view.rs` |
| What a frame looks like: panels, tabs, gauges, the title bar | `gui/clippy/src/view.rs` |
| Colors, transparency, the palette | `gui/clippy/src/skin.rs`, keys in `gui/clippy/src/config.rs` |
| Settings file format and its defaults | `gui/clippy/src/config.rs` |
| What the settings panel lists, and what changing a row writes | `gui/clippy/src/settings.rs`, drawn by `view::settings_panel`, routed in `gui/clippy/src/main.rs` |
| Mouse, keyboard, selection gestures, the window lifecycle | `gui/clippy/src/main.rs` |
| What a right click offers and what a row does | `gui/clippy/src/menu.rs`, routed in `gui/clippy/src/main.rs` |
| Choosing the folder the agent works in, and the folders NO0B remembers | `gui/clippy/src/picker.rs`, drawn by `view::folder_picker`, routed in `gui/clippy/src/main.rs` |
| Which saved sessions the first screen offers, and which folder each one belongs to | `gui/clippy/src/sessions.rs`, listed by `picker::Picker::show_sessions`, routed in `gui/clippy/src/main.rs` |
| The conversation and metrics model: what an event does to state | `gui/clippy/src/state.rs` |
| Where a pane that is a list is scrolled to, and what clamps it | `gui/clippy/src/scroll/CONTRACT.md`, content measured by `view::scroll_extent` |
| Which readings the monitors show | `gui/clippy/src/monitor.rs` |
| Which skills and MCP servers the panel lists, and what turning one off or removing it does on disk | `gui/clippy/src/agent.rs`, listed by `gui/clippy/src/settings.rs` |
| What a tool call is remembered as, and what the popup over an activity row shows | `gui/clippy/src/state.rs`, drawn by `view::call_popup` |
| Talking to the agent process | `gui/clippy/src/link.rs` |
| Drawing primitives: rects, corners, text, anything the shader does | `gui/noob-draw/src/lib.rs` |
| The GPU device, surface, transparency probing | `gui/noob-gpu/src/lib.rs` |
| Markdown rendering in the transcript | `gui/clippy/src/markdown.rs` |
| Syntax colors in the file view | `gui/clippy/src/syntax.rs` |
| The thinking orb in the title strip: its maths, its two states | `gui/clippy/src/orb/CONTRACT.md`, drawn by `view::title_bar`, clocked in `gui/clippy/src/main.rs` |
| Desktop entry, icons, packaging | `gui/clippy/src/packaging.rs`, `gui/data/`, `dev.sh gui-package` |

## The agent (`crates/`)

| You want to change | Open |
|---|---|
| The wire protocol between agent and window | `crates/noob-proto/CONTRACT.md`, shapes in `crates/noob-proto/schema/` |
| What the agent can do: tools and their schemas | `crates/noob/src/tools/CONTRACT.md` |
| The system prompt and what goes into it | `crates/noob/src/agent/CONTRACT.md`, texts in `crates/noob/prompts/` |
| Sessions on disk, resume | `crates/noob/src/session/CONTRACT.md` |
| Endpoint, keys, sandbox detection | `crates/noob/src/config/CONTRACT.md` |
| Talking to a model server | `crates/noob-provider/CONTRACT.md` |
| Skills | `crates/noob/src/skills/CONTRACT.md` |
| Talking to MCP servers, mcp.json | `crates/noob/src/mcp/CONTRACT.md` |
| The `serve` subcommand the window drives | `crates/noob/src/serve/CONTRACT.md` |

## Boxes

A box is a blackbox. From outside it you read its `CONTRACT.md` and its
`schema/`, and nothing else: not its `src/`, not its tests. Cross-box data is
schema shaped and the boundary fails closed.

Boxes so far:

- [`gui/layers/text-geometry`](../gui/layers/text-geometry/CONTRACT.md) - logical
  lines to visual rows.
- [`crates/noob-proto`](../crates/noob-proto/CONTRACT.md) - the wire protocol:
  Event frames out, Command frames in, shapes in `schema/`.
- [`crates/noob-provider`](../crates/noob-provider/CONTRACT.md) - transcript in,
  model events out, over both OpenAI wire shapes.
- [`crates/noob-testkit`](../crates/noob-testkit/CONTRACT.md) - dev-only test
  rig: mock OpenAI and MCP servers with wire assertions, a pty driver, a
  terminal screen emulator.
- [`crates/noob/src/emit`](../crates/noob/src/emit/CONTRACT.md) - the
  `NOOB_EMIT` side channel: Event frames to a file beside the session, off by
  default.
- [`crates/noob/src/config`](../crates/noob/src/config/CONTRACT.md) - config
  dir, the one settings lookup rule, validated atomic `.env` writes, sandbox
  mode, endpoint autodetect.
- [`crates/noob/src/session`](../crates/noob/src/session/CONTRACT.md) -
  append-only JSONL transcripts, resume with repair, token totals, shapes in
  `schema/`.
- [`crates/noob/src/mcp`](../crates/noob/src/mcp/CONTRACT.md) - the lazy MCP
  client: connect caches a catalog, calls validate locally, timeouts kill
  wedged servers.
- [`crates/noob/src/subagent`](../crates/noob/src/subagent/CONTRACT.md) - the
  subagent tool and the background hub: detached children of the binary
  itself, one result line each.
- [`crates/noob/src/skills`](../crates/noob/src/skills/CONTRACT.md) - SKILL.md
  discovery, the L1 index, install and remove with staged atomic publish.
- [`crates/noob/src/exec`](../crates/noob/src/exec/CONTRACT.md) - the one
  process runner: merged bounded output, the child tree as one killable
  unit, no residue.
- [`crates/noob/src/term`](../crates/noob/src/term/CONTRACT.md) - the
  terminal backend: raw mode with guaranteed restore, bytes to keys, size,
  signals.
- [`crates/noob/src/tools`](../crates/noob/src/tools/CONTRACT.md) - the tool
  registry: specs, dispatch rails, per-capability context slices, shared
  write/truncation policy.
- [`crates/noob/src/agent`](../crates/noob/src/agent/CONTRACT.md) - the
  agentic loop: rounds, batches, plan mode, compaction, all reported through
  the ui turn surface.
- [`crates/noob/src/ui`](../crates/noob/src/ui/CONTRACT.md) - the four
  output surfaces behind one turn surface; the dock; headless bytes never
  change.
- [`crates/noob/src/serve`](../crates/noob/src/serve/CONTRACT.md) - Command
  frames in, Event frames out; the surface a front end drives.
- [`crates/noob`](../crates/noob/CONTRACT.md) - the binary itself: argv
  surface, exit codes, and the box map behind it.
- [`gui/clippy/src/dock`](../gui/clippy/src/dock/CONTRACT.md) - the 2x2
  grid model: views, cells, tabs, dividers.
- [`gui/clippy/src/prompt`](../gui/clippy/src/prompt/CONTRACT.md) - the
  input line: text, caret, selection, counted in characters.
- [`gui/clippy/src/select`](../gui/clippy/src/select/CONTRACT.md) - pointer
  selection over monospace panes, in absolute lines.
- [`gui/clippy/src/scroll`](../gui/clippy/src/scroll/CONTRACT.md) - one
  top-anchored offset per view, clamped to content.
- [`gui/clippy/src/orb`](../gui/clippy/src/orb/CONTRACT.md) - the thinking
  animation: dots, orbit, square rest plate, one morph.
- [`gui/clippy/src/design`](../gui/clippy/src/design/CONTRACT.md) - the
  space and type scales from one number, and the named icons.
- [`gui/noob-gpu`](../gui/noob-gpu/CONTRACT.md) - adapter, device, surface:
  acquire, present, resize, transparency probing.
- [`gui/noob-draw`](../gui/noob-draw/CONTRACT.md) - drawing vocabulary: panels,
  rects, glyph text, the embedded symbol font.

The rest of the tree becomes boxes round by round; the plan and its order are
Task 0 in [`NEXT.md`](NEXT.md).

### One note on how the boundary is enforced

For a layer on the per-frame path, validating a JSON envelope 24 times a second
would cost more than the work inside the layer. So the runtime call is a typed
Rust function whose types mirror the schema exactly, and the schema is enforced
at the test boundary instead: `tests/contract.py` validates real fixtures
against `schema/`, including fixtures that must be rejected. The contract is
still the only thing a caller reads, and it is still checked; what it is not is
re-parsed every frame.
