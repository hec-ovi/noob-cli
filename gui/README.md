# NO0B

A window for watching an agent work. It starts `noob serve` in a workspace,
sends what you type, and renders the frames that come back.

No terminal, no web stack, no system window chrome. One GPU surface, drawn with
wgpu and winit, composited against your desktop.

```
./dev.sh gui                 # opens the folder picker
./dev.sh gui ~/some/project  # straight into that folder
```

Named a folder, it starts there. Without one the window opens on a picker
instead of guessing: launched from the dock, the current directory is your home
directory, and handing the agent that silently was the old behaviour. Arrows
move, right walks into a folder, left goes back out, Backspace with nothing typed
goes up, Enter opens what the cursor is on, and Esc quits. What you type goes in
the search field above the list, a bordered box with a magnifier in it and the
same cut corner as everything else here. Typing dims every folder that does not
carry what you typed rather than taking it off the list, and the arrows then walk
the ones that do. The mouse does the same: click a row, dim
or not, double-click to walk in, and the Open button at the foot confirms. The
list is a tree: the plus in front of a folder puts what is inside it under that
folder without leaving the one you are in, and the minus takes it back out. Both
are a small green box with nothing filled in, drawn out of rectangles rather than
out of a glyph, so the mark is a control instead of a block. A
folder nobody has permission to read says so on a row of its own instead of
looking empty. The row the cursor is on is a green band with the theme's darkest
colour written over it. The box is one size, so walking into a folder with far
more or far fewer entries does not move it. Folders you have opened before sit at the top of
the list, so the second launch is Enter. They live in
`~/.config/noob/no0b.recent`, one path per line, newest first; delete a line
to forget it and delete the file to forget them all.

The Sessions button beside Open, or Ctrl-R, lists the conversations the agent
has already saved, in the same box with the same keys. An arrow at the left of
the heading goes back to the folders, and so do the button at the foot, which
then says Folders, and Esc. The list is a table under a row that names its
columns: when it was, the folder it belongs to, how big the transcript is, how
full its context window got, and the start of the first thing you asked in it.
Up and down move the cursor, Enter carries the session on, which is
`noob serve --resume <id>` in that folder, and a right click on a row offers the
same Open plus Delete, which removes the transcript and its line from the note
below. The ones belonging to the folder you are looking at come first.
Transcripts do not record which folder they happened in, so the window keeps
that note itself in `~/.config/noob/no0b.sessions`, one `<id> <folder>` per
line, with `ctx=<used>/<total>` in front of the folder once a window has watched
that session run: a session started from a terminal has no note, opens in the
folder written above the list, and shows `-` in the context column, and so does
every session saved before the note started carrying the reading. Nothing here
guesses that number. One whose folder has been deleted cannot be opened at all
and says so instead of starting the agent somewhere you did not choose. A
session file that was cut short (killed mid-write) is skipped and counted beside
the heading rather than taking the list down with it.

`NOOB_BIN` names the agent binary when it is not `noob` on PATH.

## Installing

```
./dev.sh gui-install         # ~/.local/bin/no0b, the launcher and the icon
./dev.sh gui-package         # the release tarball, same contents
```

Everything lands under `~/.local`, so nothing needs root and nothing goes
outside your home directory. `install.sh --uninstall` takes it all back out.
Installing also removes what the old CLIppy name left behind, the launcher, the
entry and both icons, so the menu holds one entry rather than two with the older
one starting nothing.
Not a Flatpak or an AppImage. The binary links the system's C library and loads
the machine's own GPU and display libraries at runtime, and a driver is exactly
the thing a bundle cannot ship.

The icon is a console drawn as a hollow wire with `>_` inside it: one path of
four subpaths in one flat colour, on a 128 grid with a module of 8 so every edge
is a whole pixel at 16, 32, 64 and 128. The body is open rather than filled, so
the dock shows through it, and the top right corner takes the same 45 degree cut
every panel in the window takes. The glow is one blur merged under the mark and
nothing more, so a rasterizer with filters off draws the same four subpaths
sharp. Its radius reaches further than the 8 of margin, so the canvas edge trims
the halo down both sides where it has fallen to 7% alpha. That is the chosen
price of a brighter glow rather than something to fix. The filter region is
written out instead of left to the default, so the only thing that ever clips
the halo is the canvas. The small
variant is drawn again on a 16 grid rather than scaled onto it, flat and with no
halo, because at that size a blur is mud and the diagonals land between pixels.
Its green is darker than the interface accent on purpose: the accent measures
1.73:1 against white and vanishes in a light dock, while the icon is 3.48:1 on
white and 6.03:1 on black and so needs no plate behind it. On Wayland a window
cannot set its own icon at all, so the launcher file and the icon file are the
entire mechanism, and their names have to match the name the window announces. A
test asserts all three agree, because when they stop agreeing the window still
works perfectly and just wears a grey square forever.

## What is where

A 2x2 grid: four cells, two dividers, nothing nested deeper. Every view is a tab
in one cell, and **dragging its tab into a cell moves it there**; **dropping it on
the line between two cells** merges the pair and gives its pane both, which is how
a pane comes to span a whole column or a whole row. A cell you empty gives its
room to its neighbour, so the window opens as one conversation down the left
(both cells of that column, because the one under it is empty) with the monitors
above right and the files below them. The tab that is showing carries a green
line along its top, one colour for every view, so which tab you are on is
answerable without reading the labels. A tab strip has no surface of its own; the
tabs carry the pane's, the showing one at full strength and the rest at a lower
alpha.

The title strip reads the orb, then NO0B, then the version and commit this build
was cut from. The orb is the one animated thing in the window, and it is two
objects rather than one: while a turn is running it is twelve tilted rings of
dots around one centre with three runners chasing each ring, 516 discs a frame,
and at rest it is a dotted globe, 112 square dots on a lattice of latitude and
longitude. Both are ported from `thinking-orbs` and neither needs a shader,
because a dot is one rectangle through the same rounded-rect distance field
every panel is drawn with: corner radius at half the width for a disc, none at
all for a square. Its clock is a 30 frames a second deadline that exists only
while the agent is working; the resting globe reads no clock at all, so a window
with nothing happening in it goes back to blocking until you touch it.

While a turn runs, the prompt's marker is three dots taking turns rising, one up
and two down, on the same deadline. At rest it is the chevron again and nothing
moves.

| view | carries |
|---|---|
| OUTPUT | the conversation: what you asked and what the model said, prose and reasoning, streamed as it arrives |
| ACTIVITY | every call, one colour and one tag per tool: bash, read, ls, glob, grep, context, write, edit, web, skill, mcp, agent, plan, and one for anything else, plus a running command's output as it arrives |
| PLAN | the checklist, straight from the plan tool's own arguments |
| AGENTS | sub-agents: their tool set, their brief, the last thing each one said, and how it ended |
| HARDWARE | CPU and RAM, plus GPU, VRAM and GTT on an AMD card |
| CONTEXT | how full this run is: which phase, model and workspace, the context fill, total requests and tool calls, and what the last request prefilled and generated |
| SESSION | what this run has spent: tokens prefilled, generated and served from cache, and the measured prefill and decode speed |
| DEBUG | tool calls that failed, and the arguments that were sent to the one you click |
| FILES | every file touched, listed down the left, with the open one's diff, a line-number gutter and syntax coloring |

The conversation is rendered as Markdown, because the model writes Markdown
whether or not anything asked it to: headings, bold, bullets and fenced code
become formatting instead of showing their marks, and a fenced block is syntax
colored by the language the fence named.

A reading with a maximum is drawn as a block of dots, twenty across and four
down, in the metric's own colour: one row is 25 percent and one dot is 1.25, and
the block fills from the bottom. The number sits beside it at the pane's own
size, in the same colour, which is what says it is the thing being read. A
reading with no maximum has nothing to be a proportion of, so it is the number
alone, with no empty track under it. Each metric keeps its colour wherever it
appears, so prefill is the same blue in CONTEXT's LAST PREFILL as in SESSION's
PREFILLED.

Twenty dots to a row is a lot of width to ask a pane for, so the number is
served first and the block takes what is left. A pane dragged narrow enough that
a dot would be under four pixels across drops the block and keeps the number,
rather than drawing twenty dots two pixels wide in the room the number needed.
Every row of a monitor is the same height, block row or not, because that is what
lets the pane say how much of itself is on screen; the block shrinks to fit the
readings the pane has, and past four pixels the pane scrolls instead.

HARDWARE reads `/sys/class/drm/card*/device` and `/proc` directly. No vendor
library, no dependency. It only samples while it is on screen, so an idle window
still costs nothing.

The GPU rows are AMD only, because they come from `gpu_busy_percent` and the
`mem_info_*` files, which the amdgpu driver exposes and other drivers do not.
On anything else those rows are simply absent and the pane shows CPU and memory
alone. Reading an Nvidia card means its own library, which is the dependency
this deliberately does not have.

CONTEXT is the other question: not whether the machine is keeping up but whether
the budget is. The fill comes from the agent's own estimate rather than from the
last request, so it moves while a turn is still running. Under it are the totals
for this run and then the last request on its own, which is the pair that says
what one more turn is going to cost.

SESSION is what the run has spent, out of the same events. The rates are measured
rather than reported: prefill is from the request leaving to the first token
arriving, which is what a long transcript costs, and decode is from the first
token to the last, which is what the answer costs.

Neither pane reads the all-time totals. Those are still kept in
`~/.config/noob/no0b.totals` beside the settings, written by rename at the end of
every turn and when the window closes, with a mean and a median per phase: the
mean is every request ever, the median is the middle one, which is the reading
that survives a cold start with a full transcript. That needs the samples
themselves, so the file keeps the last 512 per-request rates. A missing file is a
first run and an unreadable one reads as zero; neither stops the window opening.
They had a pane and it showed those counts with nothing on it to say they were
not this session, which is why it went. They are a section of the settings panel
now, called ALL TIME, with this session already added in.

DEBUG counts the calls that failed and shows what was sent to them. Click a row
and the arguments of that call open under it. Both halves are already on the
wire, they were being written into the activity log and then dropped, so this
needed no protocol change. What it does not show is the schema the tool expected:
that is on no event at all.

Every pane scrolls inside its own box. The plan, the agent list, the failed calls
and the three monitors used to draw what fitted and lose the rest, so a long plan
ran off the bottom edge with nothing on screen saying it was there. All of them
take their window, their clamp and their bar from the same place the conversation
does, so the wheel and PageUp/PageDown mean the same thing in every pane and a bar
appears only when there is more than the box holds. CONTEXT keeps its three header
rows in place while its readings scroll, because a monitor whose first rows
scrolled away is a monitor of an unnamed session.

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

The file list is a column down the left of the pane, one row per file with its
type icon, the way an editor's explorer reads. It was a strip of tabs across the
top, which ran out of room at about six files and dropped the rest; the column
scrolls instead, and the wheel over it moves the list rather than the file. The
row of whatever the agent just touched is marked and scrolled to. It is a flat
set, not a filesystem: these are the files the agent has opened, so there are no
directory rows and nothing to expand. The column is narrow, so a name that does
not fit loses its parent directory first and its own tail second, and the list
never grows past the width that leaves the file beside it readable.

The prompt grows as you type, up to eight lines or whatever `max_input_rows` in
the settings says. Click anywhere in it to put the caret there, drag across it to
select a span, Ctrl-A selects the whole line, and typing or backspace over a
selection replaces it. Ctrl-V pastes; a pasted newline becomes a space, because
the prompt is one wrapped line and Enter is what sends it.

Drag across the conversation, the activity list or a file to select text, and
Ctrl-C copies it. Ctrl-C with nothing selected still cancels the turn, which is
the thing that must never get hard to reach; Ctrl-Shift-C always copies, and
Escape drops the selection before it touches anything else. Selection is only
on the panes that are made of lines, because the plan, the agent list, the three
monitors and the debug list are lists and readings rather than text, and
pretending otherwise would mean guessing at a layout that does not exist. A click
in DEBUG opens a failed call instead of starting a selection.

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
into a cell** to move it there, **onto the line between two cells** to give its pane
both of them, onto a tab strip to put it at that place in the strip, or **off the
window** to close that widget. A green box says which room the drop would take
before you let go, one cell or two. **Drag the gap between two panes** to move the
divider: the pointer takes a resize shape over it, and where you leave it is
written back to the settings file. The grid is capped at 2x2, so four cells is
the most there is and no space can be dragged smaller than a tab strip with
enough pane under it to read. **Double-click the title bar** to maximize the
window, and again to put it back, the same toggle as the maximize button and as
every other window on the desktop.

Right click the prompt for Copy and Paste, or a pane or its tab for Settings,
Copy selection, Close this widget and Widgets. A row with nothing to act on is
greyed rather than absent, so the menu is the same shape every time and the row
you were aiming for has not moved. Closing a widget takes it out of the window
and a space left with no tabs gives its room to its neighbour rather than
leaving a hole. Every row carries its own icon in front of its label: two sheets
for a copy, a clipboard for a paste, a gear for Settings, a cross for Close this
widget and a grid of frames for Widgets.

**Widgets** is the way back, and the way out. It is the last row, marked in front
with that grid and with a `>` at its end, and it opens a list of all nine in a box beside itself, right by
default and left when the menu is near the right edge of the window. Every row
of the list is a switch: a ticked box means the widget is in the window and
picking it takes the widget out, an empty box means it is closed and picking it
puts it back in the space it opens in by default. The menu stays open over the
list so you can switch a second widget without opening it again, unless what you
switched off is the widget the menu was opened over, which takes the rest of its
rows with it. The list is clamped into the window like the menu itself, and in a
window too short for all nine the wheel scrolls it.

## Settings

**Settings opens the panel**, from a right click on any pane or tab. It takes the
whole window under the title strip and it is eight sections, named down a rail on
the left with the chosen one beside it. Up and down on the rail walk the sections,
right goes into one, left comes back out, and inside a section up and down walk
its rows while left and right change the row the cursor is on. Enter flips a
switch and starts an edit on the endpoint, Tab crosses between the rail and the
rows, the wheel scrolls, and Esc puts the panel away. The panes are exactly where
they were when it closes, and the agent keeps working behind it.

Four of the sections are the agent's own files rather than the window's. AGENT
reads `~/.config/noob/.env`, the file the CLI re-reads on every request: it names
the file, shows the endpoint and every other key that is set, and the endpoint can
be typed over. That write goes through a port of the CLI's own `.env` writer, so
every other line and every comment survives it, and the file is read back
afterwards. No credential is ever shown or written from here: a key, token, secret
or password reads as `set, and not shown here`, which is the same line the CLI
takes by keeping secrets out of settable config. SESSIONS lists the saved
conversations the folder picker offers, through the same reader, with when each
one was, which folder it belongs to and the opening of what was first said in it.
SKILLS lists the directories under `~/.config/noob/skills`, named and described
from each `SKILL.md`'s front matter and by the directory name when it has none.
MCP names both files the CLI merges, `<config>/mcp.json` and
`<workspace>/.noob/mcp.json`, and says `none configured` when neither is there
rather than showing an empty list that reads as broken; a malformed one is a line
saying so, and the servers from the file that did parse are still listed.

The other four are the window's own settings file, and every key in it is on one
of them: APPEARANCE, PANES, COLOURS and ALL TIME. Each row is the key as the file
spells it and the value as the file spells it, so the panel doubles as the
reference for editing the file by hand. A change is written straight away through
the same writer `no0b --set` uses, so the comments stay, and then the whole file
is read back and the window is restyled from it: the palette, both font sizes and
the two panes that can be turned off all move without a restart. That read-back is
also why a row cannot show a value the file will not carry, since what you see is
what the next launch reads.

A setting with a range is a slider: opacity, both font sizes, how tall the prompt
may grow and both dividers. Press the track and drag it, with the value beside it
and the arrow keys still nudging it one step. The file is written when the button
comes up rather than on every motion event, which would be hundreds of writes for
one decision, and the panel carries the value it is being dragged to until then.

The palette is its own section, as swatches to read rather than fields to edit,
with the path of the file beside them. Changing a colour means typing a hex value,
and the one field this window has is the endpoint. `theme` is one section away and
repaints the window in a preset; a file carrying a palette that is not one of the
four exactly reads as `custom`, which is what one hand-tuned colour over a preset
makes it.

ALL TIME is the all-time totals: tokens prefilled, generated and served from
cache, and a mean and a median prefill and decode speed across every request this
machine has ever run.

`~/.config/noob/no0b.conf`, written with the defaults on first run and
commented. Opacity, both font sizes, how tall the prompt may grow, where the two
dividers sit, which panes exist, and the whole palette: the eight base colors, one per tool,
ten gauge slots a monitor reading picks from, and the five the highlighter uses
for code. A key it does not recognise is reported in the ACTIVITY pane rather
than ignored, and a key an older build wrote and this one dropped, such as any of
the nine `view_*` colours, is read off the floor without a word.

`theme = noob | amber | ice | plum` sets the eight base tones, the five code
colors and the two tool colors that are prose rather than a tool. The rest of the
tool and gauge colors name the thing rather than the window, so a theme leaves
them alone. The
colors ship as commented defaults so the theme has something to set, so
uncomment one line to keep the theme and override that single color.
`no0b --set theme=amber` makes the same edit from a terminal without touching
the comments.

The window shipped as CLIppy up to 0.6.0 and wrote `clippy.conf`,
`clippy.recent` and `clippy.totals`. The first run under the new name moves each
of those to `no0b.*`, so a tuned palette, the folders you have opened and the
all-time totals survive the rename. A file that already exists under the new
name wins and the old one is left alone.

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
| `noob-draw` | instanced rectangles and shaped glyphs, and nothing else, on a base layer and a floating one |
| `no0b` | the window shell, the layout, the panes, the settings panel, the agent link (in `gui/clippy/`, which still carries the old folder name) |
| `text-geometry` | which visual rows a run of logical lines occupies, and every question that follows from it. A contract-isolated layer under `gui/layers/`, with no dependencies at all, so it tests without a GPU or a font |

A frame has two layers because one instanced pass draws all of a layer's
rectangles and one text pass draws all of its glyphs after them, so a box pushed
last still lands under text pushed earlier. The right click menu was drawn under
the writing in the panes it covered for exactly that reason. `Scene::over_rect`
and `Scene::over_text` push to the floating layer, painted after the whole base
layer, and anything that floats over the window uses them.

`gui/data/` holds what the desktop needs: the icon, its small redrawn variant,
the launcher entry and the installer that places them.

Its own cargo workspace, deliberately. The CLI is capped at 8 MiB and 45 runtime
crates and those are published claims; a GPU stack is several hundred crates.
One lockfile for both would put a `workspace = true` between the two budgets.
They share exactly one thing, `crates/noob-proto`, by path.

NO0B has its own ceiling, 40 MiB and 400 crates, enforced by
`./dev.sh gui-check` the same way the CLI's is. It currently uses 13.2 MiB and
147 crates. The largest single thing in the binary is the symbol font, 2.4 MiB,
embedded rather than looked for on the system because a glyph a machine does not
have draws as nothing at all. Five of the crates are the clipboard: a copy has
to reach the display server, and Wayland and X11 do not agree on how.

## Transparency is probed, never assumed

`CompositeAlphaMode` support varies by compositor and driver. The mode is chosen
from what the surface reports, and when none of the modes composite, the palette
falls back to fully opaque, which looks deliberate rather than broken.

## What is not here yet

Nesting a pane inside a pane. The grid is capped at 2x2: four cells is the most
there is, and what a drop moves is which cells a pane covers. Voice is not here
either.
Nor is real tree-sitter highlighting: the file pane uses a small scanner
instead, comments, strings, numbers and keywords, chosen by file extension.
Real grammars are the right answer for a full editor view and are also eight
crates and several megabytes, which is a trade worth making later and not now.

The window will not resize past 2200x1400. Unbounded is not useful: a
conversation four thousand pixels wide is one long line per paragraph and the
panes stop being panes.
