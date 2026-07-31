# Where to change what

Open one folder, not the repo. Find the thing you want to change on the left and
open only what the right column names.

## NO0B, the GPU window (`gui/`)

The folder is `gui/clippy` because that was the product's first name. The
package, the binary and everything a user sees are `no0b`.

| You want to change | Open |
|---|---|
| How a line of text becomes rows on screen: wrapping, scroll windows, selection bands, the scrollbar's extent | `gui/layers/text-geometry/CONTRACT.md` |
| Which panel sits where, tabs, drag and drop, where the two dividers sit | `gui/clippy/src/dock.rs` and the `Layout` half of `gui/clippy/src/view.rs` |
| What a frame looks like: panels, tabs, gauges, the title bar | `gui/clippy/src/view.rs` |
| Colors, transparency, the palette | `gui/clippy/src/skin.rs`, keys in `gui/clippy/src/config.rs` |
| Settings file format and its defaults | `gui/clippy/src/config.rs` |
| What the settings panel lists, and what changing a row writes | `gui/clippy/src/settings.rs`, drawn by `view::settings_panel`, routed in `gui/clippy/src/main.rs` |
| Mouse, keyboard, selection gestures, the window lifecycle | `gui/clippy/src/main.rs` |
| What a right click offers and what a row does | `gui/clippy/src/menu.rs`, routed in `gui/clippy/src/main.rs` |
| Choosing the folder the agent works in, and the folders NO0B remembers | `gui/clippy/src/picker.rs`, drawn by `view::folder_picker`, routed in `gui/clippy/src/main.rs` |
| Which saved sessions the first screen offers, and which folder each one belongs to | `gui/clippy/src/sessions.rs`, listed by `picker::Picker::show_sessions`, routed in `gui/clippy/src/main.rs` |
| The conversation and metrics model: what an event does to state | `gui/clippy/src/state.rs` |
| Where a pane that is a list is scrolled to, and what clamps it | `gui/clippy/src/scroll.rs`, content measured by `view::scroll_extent` |
| Which readings the monitors show | `gui/clippy/src/monitor.rs` |
| Which skills and MCP servers the panel lists, and what turning one off or removing it does on disk | `gui/clippy/src/agent.rs`, listed by `gui/clippy/src/settings.rs` |
| What a tool call is remembered as, and what the popup over an activity row shows | `gui/clippy/src/state.rs`, drawn by `view::call_popup` |
| Talking to the agent process | `gui/clippy/src/link.rs` |
| Drawing primitives: rects, corners, text, anything the shader does | `gui/noob-draw/src/lib.rs` |
| The GPU device, surface, transparency probing | `gui/noob-gpu/src/lib.rs` |
| Markdown rendering in the transcript | `gui/clippy/src/markdown.rs` |
| Syntax colors in the file view | `gui/clippy/src/syntax.rs` |
| The thinking orb in the title strip: its maths, its two states | `gui/clippy/src/orb.rs`, drawn by `view::title_bar`, clocked in `gui/clippy/src/main.rs` |
| Desktop entry, icons, packaging | `gui/clippy/src/packaging.rs`, `gui/data/`, `dev.sh gui-package` |

## The agent (`crates/`)

| You want to change | Open |
|---|---|
| The wire protocol between agent and window | `crates/noob-proto/src/lib.rs` |
| What the agent can do: tools and their schemas | `crates/noob/src/tools/` |
| The system prompt and what goes into it | `crates/noob/src/agent/prompt.rs` |
| Sessions on disk, resume | `crates/noob/src/session/` |
| Endpoint, keys, sandbox detection | `crates/noob/src/config/` |
| Talking to a model server | `crates/noob-provider/src/lib.rs` |
| Skills | `crates/noob/src/skills/` |
| The `serve` subcommand the window drives | `crates/noob/src/main.rs` |

## Layers

A layer is a blackbox. From outside it you read its `CONTRACT.md` and its
`schema/`, and nothing else: not its `src/`, not its tests. Cross-layer data is
schema shaped and the boundary fails closed.

Layers so far:

- [`gui/layers/text-geometry`](../gui/layers/text-geometry/CONTRACT.md) - logical
  lines to visual rows.

The rest of `gui/clippy/src` is not yet split into layers. The next candidates,
in the order that makes the work after them cheaper, are the selection model in
`select.rs` and the palette in `skin.rs`: both are pure, both are read by
several modules, and both already have their rule written down in one place.

### One note on how the boundary is enforced

For a layer on the per-frame path, validating a JSON envelope 24 times a second
would cost more than the work inside the layer. So the runtime call is a typed
Rust function whose types mirror the schema exactly, and the schema is enforced
at the test boundary instead: `tests/contract.py` validates real fixtures
against `schema/`, including fixtures that must be rejected. The contract is
still the only thing a caller reads, and it is still checked; what it is not is
re-parsed every frame.
