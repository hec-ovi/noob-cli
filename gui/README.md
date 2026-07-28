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

Three spaces: a wide one on the left and two stacked on the right. Every view is
a tab in one of them, and **dragging its tab onto another space moves it there**.
A space you empty gives its room to its neighbour.

| view | carries |
|---|---|
| ACTIVITY | every call, one colour and one tag per tool: bash, read, ls, glob, grep, write, edit, web, skill, mcp, agent, plan, plus a running command's output as it arrives |
| PLAN | the checklist, straight from the plan tool's own arguments |
| AGENTS | sub-agents: their tool set, their brief, the last thing each one said, and how it ended |
| HARDWARE | GPU, VRAM, GTT, CPU and RAM |
| LLM | context, where compaction triggers, cache, total and last prefill and output, and the measured prefill and decode rates |
| FILES | one tab per file touched, with the diff, a line-number gutter and syntax coloring |

The conversation is rendered as Markdown, because the model writes Markdown
whether or not anything asked it to: headings, bold, bullets and fenced code
become formatting instead of showing their marks, and a fenced block is syntax
colored by the language the fence named.

HARDWARE reads `/sys/class/drm/card*/device` and `/proc` directly. No vendor
library, no dependency: a labelled bar against its maximum the way radeontop
lays it out, with its own history drawn behind it the way btop does. It only
samples while it is on screen, so an idle window still costs nothing.

LLM is the other question: not whether the machine is keeping up but whether the
budget is. Context comes from the agent's own estimate rather than from the last
request, so it moves while a turn is still running, and COMPACTS AT is the line
that actually runs out. The rates are measured rather than reported. Prefill is from the
request leaving to the first token arriving, which is what a long transcript
costs; decode is from the first token to the last, which is what the answer
costs. Both averaged over the session, because one request is noise.

A running command scrolls. `cargo build` used to be one row that said nothing
for two minutes and then said how it went; its output now arrives line by line
in the calling tool's own colour, so two commands at once stay apart. A command
that floods stops after five thousand lines and says so rather than going
quiet; the full output still reaches the model untouched.

A failure says what class it was, the exit status as a number, and what to do
next on its own line. `exit_status 127` above `available here: python3 node` is
the whole answer often enough to be worth the room.

Sub-agents are the children themselves, not the calls that asked for them. The
`subagent` call is an admission that returns in microseconds while the child
runs for minutes, and it is also how a cancel and a status poll are asked for,
so drawing rows from it showed every fan-out twice. Each row is one child, with
what it was given, what it is saying, and how it ended.

Calls are one list, not two. Splitting `bash` off looked right on paper and read
as arbitrary in use: `ls` is the `ls` tool and `rm -rf` is `bash`, so the split
put two neighbouring thoughts in two places. Colour separates them now, one per
tool. Grouping them by category was tried first and read as no colour at all,
because most of a session is `read`, `ls` and `grep`.

Files show a line-number gutter and a band behind each block header, so a write
reads as a mark between two stretches of file rather than as part of one.

The prompt grows as you type, up to eight lines.

Routing is by tool name, and by file extension for syntax coloring. The agent is
never told any of this exists: everything the window shows is derived from calls
the model was already making, including the plan.

Keys: Enter sends, Escape clears the line or cancels the turn, Tab walks every
view wherever it has been dragged, Shift-Tab stays in one space and walks its
own tabs, PageUp and PageDown scroll whatever the pointer is over, the wheel
does the same, Ctrl-C cancels, Ctrl-Q quits.

Mouse: drag the title bar to move, drag an edge to resize, click a tab to switch
to it, click the tab already showing to fold that space away, and the `▾` at the
right of a strip does the same. **Drag a tab into another space** to move it
there. **Double-click the title bar** to shade the
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
