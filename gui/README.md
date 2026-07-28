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

## What is where

The conversation on the left. On the right, two tabbed groups:

| tab | carries |
|---|---|
| ACTIVITY | every call, colored by kind: shell, look, edit, web, skill, mcp, agent, plan |
| PLAN | the checklist, straight from the plan tool's own arguments |
| AGENTS | sub-agents, their brief and how they ended |
| MONITOR | GPU, VRAM, GTT, CPU, RAM and the session's token economy |
| files | one tab per file touched, with the diff and syntax coloring |

The conversation is rendered as Markdown, because the model writes Markdown
whether or not anything asked it to: headings, bold, bullets and fenced code
become formatting instead of showing their marks, and a fenced block is syntax
colored by the language the fence named.

MONITOR reads `/sys/class/drm/card*/device` and `/proc` directly. No vendor
library, no dependency: a labelled bar against its maximum the way radeontop
lays it out, with its own history drawn behind it the way btop does. It only
samples while it is on screen, so an idle window still costs nothing.

Calls are one list, not two. Splitting `bash` off looked right on paper and read
as arbitrary in use: `ls` is the `ls` tool and `rm -rf` is `bash`, so the split
put two neighbouring thoughts in two places. Color is what separates them now.

Routing is by tool name, and by file extension for syntax coloring. The agent is
never told any of this exists: everything the window shows is derived from calls
the model was already making, including the plan.

Keys: Enter sends, Escape clears the line or cancels the turn, Tab walks the
whole window (every tab, then every file, then back), PageUp and PageDown scroll
whatever the pointer is over, the wheel does the same, Ctrl-C cancels, Ctrl-Q
quits.

Mouse: drag the title bar to move, drag an edge to resize, click a tab to switch
to it, click the tab already showing to fold that group away, and the `▾` at the
right of a strip does the same. **Double-click the title bar** to shade the
window down to that one strip, Winamp style: it keeps showing THINKING, WORKING
or FINISHED with the plan count and how many files changed, so a collapsed
window is still a status light. Double-click again to bring it back.

## Settings

`~/.config/noob/clippy.conf`, written with the defaults on first run and
commented. Opacity, the seven palette colors, and both font sizes. A key it does
not recognise is reported in the ACTIVITY pane rather than ignored.

Opacity defaults to 88%. Lower it to see more of your desktop through the
reading surface; below about 60% a busy wallpaper starts competing with the
text, which is a taste call rather than a bug.

The panels are drawn dark under green text, not green under green: two greens
fight, and turning the opacity down to see your desktop made the text worse
rather than better. Black backs the text and the desktop shows through the
black.

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

Grid tiling, the ASCII avatar, voice, and real tree-sitter highlighting. The file pane uses a small scanner instead: comments,
strings, numbers and keywords, chosen by file extension. Real grammars are the
right answer for a full editor view and are also eight crates and several
megabytes, which is a trade worth making later and not now.

The window will not resize past 2200x1400. Unbounded is not useful: a
conversation four thousand pixels wide is one long line per paragraph and the
panes stop being panes.
