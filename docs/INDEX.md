# Where to change what

Open one folder, not the repo. Find the thing you want to change on the left and
open only what the right column names.

## CLIppy, the GPU window (`gui/`)

| You want to change | Open |
|---|---|
| How a line of text becomes rows on screen: wrapping, scroll windows, selection bands, the scrollbar's extent | `gui/layers/text-geometry/CONTRACT.md` |
| Which panel sits where, tabs, drag and drop, splits | `gui/clippy/src/dock.rs` and the `Layout` half of `gui/clippy/src/view.rs` |
| What a frame looks like: panels, tabs, gauges, the title bar | `gui/clippy/src/view.rs` |
| Colors, transparency, the palette | `gui/clippy/src/skin.rs`, keys in `gui/clippy/src/config.rs` |
| Settings file format and its defaults | `gui/clippy/src/config.rs` |
| Mouse, keyboard, selection gestures, the window lifecycle | `gui/clippy/src/main.rs` |
| The conversation and metrics model: what an event does to state | `gui/clippy/src/state.rs` |
| Which readings the monitors show | `gui/clippy/src/monitor.rs` |
| Talking to the agent process | `gui/clippy/src/link.rs` |
| Drawing primitives: rects, corners, text, anything the shader does | `gui/noob-draw/src/lib.rs` |
| The GPU device, surface, transparency probing | `gui/noob-gpu/src/lib.rs` |
| Markdown rendering in the transcript | `gui/clippy/src/markdown.rs` |
| Syntax colors in the file view | `gui/clippy/src/syntax.rs` |
| The ASCII animation and its file format, which nothing draws yet | `gui/clippy/src/avatar.rs`, authored by `gui/asciify/` |
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

The rest of `gui/clippy/src` is not yet split into layers. The plan in
[`CLIPPY-PLAN.md`](CLIPPY-PLAN.md) lists which module becomes which layer, in
the order that makes the work after it cheaper.

### One note on how the boundary is enforced

For a layer on the per-frame path, validating a JSON envelope 24 times a second
would cost more than the work inside the layer. So the runtime call is a typed
Rust function whose types mirror the schema exactly, and the schema is enforced
at the test boundary instead: `tests/contract.py` validates real fixtures
against `schema/`, including fixtures that must be rejected. The contract is
still the only thing a caller reads, and it is still checked; what it is not is
re-parsed every frame.
