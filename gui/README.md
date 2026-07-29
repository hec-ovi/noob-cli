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

## Installing

```
./dev.sh gui-install         # ~/.local/bin/clippy, the launcher and the icon
./dev.sh gui-package         # the release tarball, same contents
```

Everything lands under `~/.local`, so nothing needs root and nothing goes
outside your home directory. `install.sh --uninstall` takes it all back out.
Not a Flatpak or an AppImage: the binary is static apart from the system's own
GPU drivers, and those are exactly the thing a bundle cannot ship.

The icon is one closed path in one flat colour, drawn on a 128 grid with a
module of 8 so every edge is a whole pixel at 16, 32, 64 and 128, with a second
drawing at 16 rather than a scaled copy of the first. Its green is darker than
the interface accent on purpose: the accent measures 1.73:1 against white and
vanishes in a light dock, while the icon clears 3:1 on both grounds and so needs
no plate behind it. On Wayland a window cannot set its own icon at all, so the
launcher file and the icon file are the entire mechanism, and their names have
to match the name the window announces. A test asserts all three agree, because
when they stop agreeing the window still works perfectly and just wears a grey
square forever.

## What is where

Three spaces: a wide one on the left and two stacked on the right. Every view is
a tab in one of them, and **dragging its tab onto another space moves it there**.
A space you empty gives its room to its neighbour. Each view has a colour of its
own, drawn as a line along the top of the tab showing it, so which view a space
is holding is answerable without reading the labels. A tab strip has no surface
of its own; the tabs carry the pane's, the showing one at full strength and the
rest at a lower alpha.

| view | carries |
|---|---|
| ACTIVITY | every call, one colour and one tag per tool: bash, read, ls, glob, grep, write, edit, web, skill, mcp, agent, plan, plus a running command's output as it arrives |
| PLAN | the checklist, straight from the plan tool's own arguments |
| AGENTS | sub-agents: their tool set, their brief, the last thing each one said, and how it ended |
| HARDWARE | CPU and RAM, plus GPU, VRAM and GTT on an AMD card |
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

The GPU rows are AMD only, because they come from `gpu_busy_percent` and the
`mem_info_*` files, which the amdgpu driver exposes and other drivers do not.
On anything else those rows are simply absent and the pane shows CPU and memory
alone. Reading an Nvidia card means its own library, which is the dependency
this deliberately does not have.

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

The prompt grows as you type, up to eight lines or whatever `max_input_rows` in
the settings says. Click anywhere in it to put the caret there, drag across it to
select a span, Ctrl-A selects the whole line, and typing or backspace over a
selection replaces it. Ctrl-V pastes; a pasted newline becomes a space, because
the prompt is one wrapped line and Enter is what sends it.

Drag across the conversation, the activity list or a file to select text, and
Ctrl-C copies it. Ctrl-C with nothing selected still cancels the turn, which is
the thing that must never get hard to reach; Ctrl-Shift-C always copies, and
Escape drops the selection before it touches anything else. Selection is only
on the panes that are made of lines, because the plan, the agent list and the
two monitors are lists and readings rather than text, and pretending otherwise
would mean guessing at a layout that does not exist.

A selection holds line numbers rather than screen positions, so output arriving
underneath it does not slide it onto different text mid-drag.

Routing is by tool name, and by file extension for syntax coloring. The agent is
never told any of this exists: everything the window shows is derived from calls
the model was already making, including the plan.

Keys: Enter sends, Escape drops a selection then clears the line then cancels
the turn, Ctrl-A selects the prompt, Ctrl-C copies a selection or cancels and
takes the prompt's selection over a pane's since that is the one you were last
touching, Ctrl-Shift-C always copies, Tab walks every
view wherever it has been dragged, Shift-Tab stays in one space and walks its
own tabs, PageUp and PageDown scroll whatever the pointer is over, the wheel
does the same, Ctrl-C cancels, Ctrl-Q quits.

Mouse: drag the title bar to move, drag an edge to resize, click a tab to switch
to it, and click the tab already showing to fold that space away. **Drag a tab
into another space** to move it there, or **drag it off the window** to close that
widget. **Double-click the title bar** to shade the
window down to that one strip, Winamp style: it keeps showing THINKING, WORKING
or FINISHED with the plan count and how many files changed, so a collapsed
window is still a status light. Double-click again to bring it back.

Right click the prompt for Copy and Paste, or a pane or its tab for Settings,
Copy selection and Close this widget. A row with nothing to act on is greyed
rather than absent, so the menu is the same shape every time and the row you were
aiming for has not moved. Settings is greyed everywhere: there is no settings
panel behind it yet. Closing a widget is one way for now, and the way back is the
launcher that is still to come; a space left with no tabs gives its room to its
neighbour rather than leaving a hole.

## Settings

`~/.config/noob/clippy.conf`, written with the defaults on first run and
commented. Opacity, both font sizes, how tall the prompt may grow, which panes
exist, and the whole palette: the eight base colors, one per tool, one per view,
and the five the highlighter uses for code. A key it does not
recognise is reported in the ACTIVITY pane rather than ignored.

`theme = noob | amber | ice | plum` sets every color at once. The tool and view
colors name the thing rather than the window, so a theme leaves them alone. The
colors ship as commented defaults so the theme has something to set, so
uncomment one line to keep the theme and override that single color.
`clippy --set theme=amber` makes the same edit from a terminal without touching
the comments.

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
| `asciify` | GIF to a text animation, run at authoring time only |

`gui/data/` holds what the desktop needs: the icon, its small redrawn variant,
the launcher entry and the installer that places them.

Its own cargo workspace, deliberately. The CLI is capped at 8 MiB and 45 runtime
crates and those are published claims; a GPU stack is several hundred crates.
One lockfile for both would put a `workspace = true` between the two budgets.
They share exactly one thing, `crates/noob-proto`, by path.

CLIppy has its own ceiling, 40 MiB and 400 crates, enforced by
`./dev.sh gui-check` the same way the CLI's is. It currently uses 13.0 MiB and
147 crates. Most of the size is one asset: the symbol font is embedded rather
than looked for on the system, because a glyph a machine does not have draws as
nothing at all. Five of the crates are the clipboard: a copy has to reach the display
server, and Wayland and X11 do not agree on how. `asciify` is a fourth crate in that workspace and is never a
dependency of the window, so its GIF decoder is not in the binary.

## Transparency is probed, never assumed

`CompositeAlphaMode` support varies by compositor and driver. The mode is chosen
from what the surface reports, and when none of the modes composite, the palette
falls back to fully opaque, which looks deliberate rather than broken.

## What is not here yet

Grid tiling, voice, and real tree-sitter highlighting. The file pane uses a small scanner instead: comments,
strings, numbers and keywords, chosen by file extension. Real grammars are the
right answer for a full editor view and are also eight crates and several
megabytes, which is a trade worth making later and not now.

The window will not resize past 2200x1400. Unbounded is not useful: a
conversation four thousand pixels wide is one long line per paragraph and the
panes stop being panes.
