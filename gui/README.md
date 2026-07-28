# CLIppy

A window for watching an agent work. It starts `noob serve` in a workspace,
sends what you type, and renders the frames that come back.

No terminal, no web stack, no system window chrome. One GPU surface, drawn with
wgpu and winit, composited against your desktop.

```
./dev.sh gui                 # in the current directory
./dev.sh gui ~/some/project  # somewhere else
```

`NOOB_BIN` names the agent binary when it is not `noob` on PATH.

## Four panes, because four things are happening

The CLI puts a shell command, a file being rewritten and the model's prose in
one column, and reading it means sorting them out by eye every time. Here they
are sorted once, by where each frame came from:

| pane | carries |
|---|---|
| talk | the model's prose and reasoning, streamed |
| shell | `bash`, its command and its result |
| tools | search, skills, MCP, sub-agents |
| code | files opened and changed, with the diff |

Routing is by tool name, and by file extension for syntax coloring. The agent is
never told any of this exists: everything the window shows is derived from calls
the model was already making.

Keys: Enter sends, Escape clears the line or cancels the turn, Tab cycles the
focused pane, PageUp and PageDown scroll it, the wheel scrolls whatever the
pointer is over, Ctrl-C cancels, Ctrl-Q quits. Drag the title bar to move, drag
an edge to resize.

## Layers

| crate | owns |
|---|---|
| `noob-gpu` | adapter, device, surface, what this machine will actually do |
| `noob-draw` | instanced rectangles and shaped glyphs, and nothing else |
| `clippy` | the window shell, the layout, the panes, the agent link |

Its own cargo workspace, deliberately. The CLI is capped at 8 MiB and 45 runtime
crates and those are published claims; a GPU stack is several hundred crates.
One lockfile for both would put a `workspace = true` between the two budgets.
They share exactly one thing, `crates/noob-proto`, by path.

CLIppy has its own ceiling, 40 MiB and 400 crates, enforced by
`./dev.sh gui-check` the same way the CLI's is. It currently uses 10.2 MiB and
141 crates.

## Transparency is probed, never assumed

`CompositeAlphaMode` support varies by compositor and driver. The mode is chosen
from what the surface reports, and when none of the modes composite, the palette
falls back to fully opaque, which looks deliberate rather than broken. The tools
pane prints the capability report at startup, so what happened is on screen
rather than in a log.

## What is not here yet

Tabs and grid tiling, the ASCII avatar, the GPU and VRAM readouts, voice, and
real tree-sitter highlighting. The code pane uses a small scanner instead:
comments, strings, numbers and keywords, chosen by file extension. Real grammars
are the right answer for a full editor view and are also eight crates and
several megabytes, which is a trade worth making later and not now.
