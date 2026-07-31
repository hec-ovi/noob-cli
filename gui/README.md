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

A 2x2 grid: four cells, nothing nested deeper. One line cuts the grid in half
and each half is then cut by a line of its own, so the left column can break at
70/30 while the right one breaks at 40/60. Every view is a tab in one cell, and
**dragging its tab into a cell moves it there**; **dropping it on the line
between two cells** merges the pair and gives its pane both, which is how a pane
comes to span a whole column or a whole row. A cell you empty gives its
room to its neighbour, so the window opens as one conversation down the left
(both cells of that column, because the one under it is empty) with the monitors
above right and the files below them. The tab that is showing carries a green
line along its top, one colour for every view, so which tab you are on is
answerable without reading the labels. A tab strip has no surface of its own; the
tabs carry the pane's, the showing one at full strength and the rest at a lower
alpha.

The title strip reads the orb, then NO0B, then the version, then the folder the
agent is working in, each pair separated by the same marker. It used to end on
the commit the build was cut from; seven characters of hex is not something
anyone reads off a title, and the room they took says where you are instead.

The orb is the one animated thing in the window, and it is two objects rather
than one: while a turn is running it is twelve tilted rings of dots around one
centre with three runners chasing each ring, 516 discs a frame, and at rest it is
a dotted globe, 112 square dots on a lattice of latitude and longitude. Both are
ported from `thinking-orbs` and neither needs a shader, because a dot is one
rectangle through the same rounded-rect distance field every panel is drawn
with: corner radius at half the width for a disc, none at all for a square. Its
clock is a 30 frames a second deadline that exists only while the agent is
working; the resting globe reads no clock at all, so a window with nothing
happening in it goes back to blocking until you touch it.

While a turn runs, the prompt's marker is three dots taking turns rising, one up
and two down, on the same deadline. At rest it is the chevron again and nothing
moves.

| view | carries |
|---|---|
| OUTPUT | the conversation: what you asked and what the model said, prose and reasoning, streamed as it arrives |
| ACTIVITY | every call, one colour and one tag per tool: bash, read, ls, glob, grep, context, write, edit, web, skill, mcp, agent, plan, and one for anything else, plus a running command's output as it arrives. A bar in the gutter marks the calls that were in flight together, and clicking a row opens it out |
| PLAN | the checklist, straight from the plan tool's own arguments |
| AGENTS | sub-agents: their tool set, their brief, the last thing each one said, and how it ended |
| HARDWARE | CPU and RAM, plus GPU, VRAM and GTT on an AMD card |
| CONTEXT | how full this run is: the phase, total requests, total tool calls (with the failed ones beside them) and the last prefill as labelled rows, then the context fill and the last response as readings |
| SESSION | what this run has spent: tokens prefilled, generated and served from cache, and the measured prefill and decode speed |
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
the budget is. Its first four rows are labelled the way the phase always was: the
phase, the requests this run has made, the calls it has made (and how many of
those came back with an error), and what the last request prefilled. Under them
the fill itself, which comes from the agent's own estimate rather than from the
last request and so moves while a turn is still running, and the last response.
The model and the workspace were rows up there too. The strip says the folder now
and the settings panel says the model, so neither is worth a second answer.

SESSION is what the run has spent, out of the same events. The rates are measured
rather than reported: prefill is from the request leaving to the first token
arriving, which is what a long transcript costs, and decode is from the first
token to the last, which is what the answer costs.

Neither pane reads anything that outlives the window. There were all-time counts
across every session this machine had ever run, first as a pane called OVERALL
and then as a settings section called ALL TIME, kept in a file beside the
settings. A column of numbers from sessions nobody remembers reads as this
session's however it is labelled, so both are gone and so is the file.

There was a DEBUG pane that listed the calls that failed and opened the arguments
of the one you clicked. It is gone: a pane of its own for something that is one
number most of the time, and the failure itself is already written into ACTIVITY
where it happened, with its class, its message and what to do about it. The
number survives, beside the total tool calls in CONTEXT.

## The activity list

Read-only tools run up to eight at a time, so a turn that fans out writes four
rows that look like four unrelated calls. Every call still in flight when the
next one starts gets a bar in the gutter between its tag and its subject, so a
fan-out reads as one down the column. The bar replaces a space rather than
widening the row: every row count and selection column in that pane is character
arithmetic, and a mark that made a row one character longer would put the rows
that are drawn and the rows that are measured out of step.

Nothing on the wire says "these ran in parallel". There is no batch id, no
per-call ordinal and no agent-side timestamp on a tool frame, so the mark is read
off the calls that were open when a start frame arrived, which is exact, and the
turn takes care of itself because a turn that ends closes everything it left
open.

Clicking a row opens that call out into a box over the window: what was invoked
(the skill by name, the MCP server and the tool on it, or just bash), which turn
it was in and how long it took, the arguments the model generated, what came
back, and the detail. It closes the way the right click menu does, on Escape or
on a press anywhere else. A press that turns into a drag is still a selection:
the box goes away as soon as the drag has selected anything.

Two of those cells cannot be filled for a call that worked, and they say so
rather than sitting blank. `ToolEnd` carries a display summary and no result
body; the tool's own output reaches the window only as `ToolError.detail`, and
only when the call failed. Where a tool taps its own stdout the streamed lines
are kept and shown as the return value. The skill's file path is the same story:
only the skill's name is sent, and the box says so where the path would be. A
blank cell would read as "the tool returned nothing", which is a different claim
and usually a false one.

Every pane scrolls inside its own box. The plan, the agent list and the three
monitors used to draw what fitted and lose the rest, so a long plan ran off the
bottom edge with nothing on screen saying it was there. All of them take their
window, their clamp and their bar from the same place the conversation does, so
the wheel and PageUp/PageDown mean the same thing in every pane and a bar appears
only when there is more than the box holds. CONTEXT keeps its four header
rows in place while its readings scroll, because a monitor whose first rows
scrolled away is a monitor with no summary.

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

The prompt is one line and grows as you type, up to whatever `max_input_rows` in
the settings says (one by default, so it stays a line until you ask for more).
Past that it scrolls inside itself: the row the caret is on is the row you see,
and typing off the bottom of it brings the text up rather than hiding it.
Click anywhere in it to put the caret there, drag across it to
select a span, Ctrl-A selects the whole line, and typing or backspace over a
selection replaces it. Ctrl-V pastes; a pasted newline becomes a space, because
the prompt is one wrapped line and Enter is what sends it.

Drag across the conversation, the activity list or a file to select text, and
Ctrl-C copies it. Ctrl-C with nothing selected still cancels the turn, which is
the thing that must never get hard to reach; Ctrl-Shift-C always copies, and
Escape drops the selection before it touches anything else. Selection is only
on the panes that are made of lines, because the plan, the agent list and the
three monitors are lists and readings rather than text, and pretending otherwise
would mean guessing at a layout that does not exist.

A selection holds line numbers rather than screen positions, so output arriving
underneath it does not slide it onto different text mid-drag.

Routing is by tool name, and by file extension for syntax coloring. The agent is
never told any of this exists: everything the window shows is derived from calls
the model was already making, including the plan.

Keys: Enter sends, Escape closes an open menu or an open activity call first,
then drops a selection, then clears the line, then cancels the turn, Ctrl-A
selects the prompt, Ctrl-C copies a selection or cancels and
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
written back to the settings file. The two halves move apart, so dragging the
line over the right column leaves the left column where it was. The grid is
capped at 2x2, so four cells is the most there is and no space can be dragged
smaller than a tab strip with enough pane under it to read. **Double-click the title bar** to maximize the
window, and again to put it back, the same toggle as the maximize button and as
every other window on the desktop.

**Click a row of the activity list** to open that call out over the window, and
press anywhere else or Escape to put it away.

Right click the prompt for Copy and Paste, or a pane or its tab for Settings,
Copy selection, Close this widget and Widgets. A row with nothing to act on is
greyed rather than absent, so the menu is the same shape every time and the row
you were aiming for has not moved. Closing a widget takes it out of the window
and a space left with no tabs gives its room to its neighbour rather than
leaving a hole. Every row carries its own icon in front of its label: two sheets
for a copy, a clipboard for a paste, a gear for Settings, a cross for Close this
widget and a grid of frames for Widgets.

**Widgets** is the way back, and the way out. It is the last row, marked in front
with that grid and with a `>` at its end, and it opens a list of all eight in a
box beside itself, right by default and left when the menu is near the right edge
of the window. Every row
of the list is a switch: a ticked box means the widget is in the window and
picking it takes the widget out, an empty box means it is closed and picking it
puts it back in the space it opens in by default. The menu stays open over the
list so you can switch a second widget without opening it again, unless what you
switched off is the widget the menu was opened over, which takes the rest of its
rows with it. The list is clamped into the window like the menu itself, and in a
window too short for all eight the wheel scrolls it.

## Settings

**Settings opens the panel**, from a right click on any pane or tab. It takes the
whole window under the title strip and it is five sections, named down a rail on
the left with the chosen one beside it. Up and down on the rail walk the sections,
right goes into one, left comes back out, and inside a section up and down walk
its rows while left and right change the row the cursor is on. Enter flips a
switch, turns a skill or a server on and off, and starts an edit on the
endpoint, Tab crosses between the rail and the
rows, the wheel scrolls, and Esc puts the panel away. The panes are exactly where
they were when it closes, and the agent keeps working behind it. **Drag the line
between the rail and the settings** to give one of them more room: the pointer
takes a resize shape over it, neither side goes narrower than the longest section
name, and where you leave it is written to `settings_rail` in the settings file,
so the panel opens that way next time.

Four of the sections are the agent's own files rather than the window's. AGENT
reads `~/.config/noob/.env`, the file the CLI re-reads on every request: it names
the file, shows the endpoint and every other key that is set, and the endpoint can
be typed over. Two of the keys in it are controls rather than lines to read, under
`HOW MUCH THE AGENT GETS`: `NOOB_CTX`, the context window the agent budgets
against before it compacts, and `NOOB_TASK_CONCURRENCY`, how many sub-agent tasks
it runs at once. Both are tracks held to the CLI's own bounds, so the context
window starts at the 4096 below which the CLI stops reading it and the right end
of the concurrency track is the 16 the CLI caps itself at: the maximum is a place
to drop the pointer, not a number to guess. Until either is in the file the row
reads what the CLI falls back to and the section says so. That write goes through
a port of the CLI's own `.env` writer, so
every other line and every comment survives it, and the file is read back
afterwards. Nothing is passed to the agent on its command line, because `serve`
rejects a flag it does not know; instead the launch clears those two names out of
the child's environment, since the CLI prefers the environment over its file and
an exported value would outrank every line the panel writes. No credential is ever
shown or written from here: a key, token, secret
or password reads as `set, and not shown here`, which is the same line the CLI
takes by keeping secrets out of settable config. SESSIONS lists the saved
conversations the folder picker offers, through the same reader, with when each
one was, which folder it belongs to and the opening of what was first said in it.
SKILLS and MCP are two columns: a list on the left and, beside it, whatever the
row under the cursor is. In a window too narrow to hold both, the list keeps the
width and the second column is not drawn.

SKILLS lists the directories under `~/.config/noob/skills`, named and described
from each `SKILL.md`'s front matter and by the directory name when it has none.
Under each name is the repository the skill records, or, since nothing the CLI
writes down records where an installed skill came from, the directory it was
found in. The column beside the list is that skill's own `SKILL.md`, rendered
the way the transcript renders what the model writes; the wheel over it scrolls
the document rather than the list. Every row carries a toggle and an uninstall.
The toggle really turns the skill off: there is no enabled flag anywhere in the
CLI, so off means the directory is moved to `~/.config/noob/skills.off`, which is
none of the four places the agent looks, and on moves it back. Nothing is
remembered in a settings file; what the row says is where the directory is.
Uninstall deletes the directory, and it takes two presses: the first arms it and
the footer names what is about to go, the second one does it, and anything else
at all puts it back. It will only ever delete a directory sitting directly inside
those two, never a link and never a path that walks out of them.

MCP names both files the CLI merges, `<config>/mcp.json` and
`<workspace>/.noob/mcp.json`, and says `none configured` when neither is there
rather than showing an empty list that reads as broken; a malformed one is a line
saying so, and the servers from the file that did parse are still listed. Each
server is a row with its URL or command line under it and its entry out of the
file beside it, and a toggle of its own: off moves the entry into a `disabled`
object in the same file, which the CLI's loader does not read, and on moves it
back. The file is rewritten whole, so every other server, every `timeout_s` and
anything else in there survives it. There is no uninstall here, because a server
is a few lines somebody wrote by hand and turning it off already leaves them
where they are.

The fifth is the window's own settings file, and every key in it is on that one:
APPEARANCE. It is the sizes and the theme, then `WHICH PANES OPEN` and `WHERE THE
DIVIDERS SIT`, then the palette, each group under a heading of its own drawn
larger than the rows under it, with a hairline between every row. Each row is the
key as the file spells it and the value as the file spells it, so the panel
doubles as the reference for editing the file by hand, and anything that can be
changed is drawn as a box with an outline round it: if it has an edge, it takes a
press or a keystroke. A change is written straight away through
the same writer `no0b --set` uses, so the comments stay, and then the whole file
is read back and the window is restyled from it: the palette, both font sizes and
the two panes that can be turned off all move without a restart. That read-back is
also why a row cannot show a value the file will not carry, since what you see is
what the next launch reads.

A setting with a range is a slider: opacity, both font sizes, how tall the prompt
may grow, all four divider positions and the settings rail. Press the track and drag it, with the value beside it
and the arrow keys still nudging it one step. The window takes the value while
you drag: the opacity you are dragging to is the only thing that tells you where
to stop. The file is written when the button comes up rather than on every motion
event, which would be hundreds of writes for one decision, and the live value
goes through the same clamps the file is read with, so what you drag to is what
the next launch reads.

The palette is the last block of APPEARANCE, laid out as a grid: three colours to
a row under `THE WINDOW`, `THE HIGHLIGHTER`, `ONE PER TOOL` and `ONE PER GAUGE`,
each one a block of the colour with what it colours written beside it in words.
Pressing a swatch says which key in the file writes it, on the line at the foot of
the panel; they are swatches to read rather than fields to edit, since changing
one means typing a hex value and the one field this window has is the endpoint. `theme` is a few rows up and
repaints the window in a preset; a file carrying a palette that is not one of the
four exactly reads as `custom`, which is what one hand-tuned colour over a preset
makes it.

`~/.config/noob/no0b.conf`, written with the defaults on first run and
commented. Opacity, both font sizes, how tall the prompt may grow, where the
dividers sit, how wide the settings rail is, which panes exist, and the whole palette: the eight base colors, one per tool,
ten gauge slots a monitor reading picks from, and the five the highlighter uses
for code. A key it does not recognise is reported in the ACTIVITY pane rather
than ignored, and a key an older build wrote and this one dropped, such as any of
the `view_*` colours, is read off the floor without a word.

`theme = noob | amber | ice | plum` sets the eight base tones, the five code
colors and the two tool colors that are prose rather than a tool. The rest of the
tool and gauge colors name the thing rather than the window, so a theme leaves
them alone. The
colors ship as commented defaults so the theme has something to set, so
uncomment one line to keep the theme and override that single color.
`no0b --set theme=amber` makes the same edit from a terminal without touching
the comments.

The window shipped as CLIppy up to 0.6.0 and wrote `clippy.conf` and
`clippy.recent`. The first run under the new name moves each of those to
`no0b.*`, so a tuned palette and the folders you have opened survive the rename.
A file that already exists under the new name wins and the old one is left
alone.

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
