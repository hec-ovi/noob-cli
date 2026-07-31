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
or not, double-click to walk in, and the Open selected button at the right of
the head confirms. The
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

The head is one title, OPEN FOLDER OR CONTINUE SESSION, and three buttons:
Folders and Sessions at the left, and Open selected at the right. The pair
chooses which list is in the box, and the one whose list is showing wears the
same green band the chosen row wears, so the buttons say where you are. Sessions,
or Ctrl-R, lists the conversations the agent has already saved, in the same box
with the same keys; Folders, Ctrl-R again, or Esc goes back to the tree. The
list is a table under a row that names its
columns: when it was, the folder it belongs to, how big the transcript is, how
full its context window got, and the start of the first thing you asked in it.
Up and down move the cursor, Enter carries the session on, which is
`noob serve --resume <id>` in that folder, and a right click on a row offers the
same Open plus Delete, which removes the transcript and its line from the note
below. Delete is pressed twice: the first press turns the row into `sure?` and
the second one does it, the same question the settings panel's trash asks.
Moving off the row, or closing the menu with a key or a press anywhere else,
puts the question back. The ones belonging to the folder you are looking at come first.
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

The orb is the one animated thing in the window, and it is two formations rather
than one: while a turn is running it is twelve tilted rings of dots around one
centre with three runners chasing each ring, 516 discs a frame, and at rest it is
a square, a filled 11 by 11 plate of 121 dots. The rings are ported from
`thinking-orbs`; the square is this window's own. Neither needs a shader, because
a dot is one rectangle through the same rounded-rect distance field every panel
is drawn with: corner radius at half the width for a disc, none at all for a
square.

Starting a turn is not a swap between the two. Each dot of the plate is paired
with a dot of the rings and travels to it over 300ms, rounding off from a square
to a disc on the way, while the rings' other dots come up out of nothing behind
them; ending a turn runs the same move backwards. Its clock is a 30 frames a
second deadline that exists only while the agent is working or the orb is still
on its way back, and the resting square reads no clock at all, so a window with
nothing happening in it goes back to blocking until you touch it.

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

Every row says when it happened, as a time of day in front of the tag and in the
dim tone, so the clock column does not compete with the subject for the row. The
window asks the system what time it is once at startup and adds the monotonic
second each frame arrived at, which is the arrival the call record already kept
for the box below, so the row and the box cannot disagree about one call. Rows
the window writes for itself, a clipboard it could not open, are stamped the same
way. If it cannot get an answer about local time, the rows carry no reading
rather than a column of times an hour out.

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
reads as a mark between two stretches of file rather than as part of one. A line
too long for the pane continues under its own text, indented past that gutter:
the number is written once, on the first row, and every row of the line holds
the same columns, which is what keeps the band, the caret and the clipboard on
the characters you can see.

The file list is a column down the left of the pane, one row per file with its
type icon, the way an editor's explorer reads. It was a strip of tabs across the
top, which ran out of room at about six files and dropped the rest; the column
scrolls instead, and the wheel over it moves the list rather than the file. The
row of whatever the agent just touched is marked and scrolled to. It is a flat
set, not a filesystem: these are the files the agent has opened, so there are no
directory rows and nothing to expand. The column is narrow, so a name that does
not fit loses its parent directory first and its own tail second, and the list
never grows past the width that leaves the file beside it readable.

The prompt is as tall as `prompt_rows` in the settings says and stays there,
typed into or not (one by default, so it is a line until you ask for more). It
used to climb to that number a row at a time as you typed, which moved the
conversation every time a line wrapped. The key was called `max_input_rows` and
the window never read it, so that name is retired: a file that still carries it
opens at one row and says nothing. A message longer than the box scrolls inside
it: the row the caret is on is the row you see, and typing off the bottom of it
brings the text up rather than hiding it. The panes keep a floor under them, so a
row count the window is too short for takes what is left over instead of the last
of the conversation, and goes back to the number you set as soon as there is room
for it.
Click anywhere in it to put the caret there, drag across it to
select a span, Ctrl-A selects the whole line, and typing or backspace over a
selection replaces it. Ctrl-V pastes; a pasted newline becomes a space, because
the prompt is one wrapped line and Enter is what sends it.

Drag across the conversation, the activity list, a file or the document beside
the settings panel's entry list to select text, and Ctrl-C copies it. Ctrl-C with nothing selected still cancels the turn, which is
the thing that must never get hard to reach; Ctrl-Shift-C always copies, and
Escape drops the selection before it touches anything else. Selection is only
on the panes that are made of lines, because the plan, the agent list and the
three monitors are lists and readings rather than text, and pretending otherwise
would mean guessing at a layout that does not exist.

A selection holds line numbers rather than screen positions, so output arriving
underneath it does not slide it onto different text mid-drag.

The panes wrap at blanks, so words stay whole, and a word wider than the pane
breaks on the column because there is nowhere else to break it. The rule is
written once, in `text-geometry`, and both the renderer and the selection ask it
the same question: the characters on the row you point at are the characters
that end up on the clipboard. The blank a row broke at is on neither row, and it
comes back when you copy a run that crosses the break. The prompt is the one box
that still breaks on the column, because its caret is placed by counting them.

The conversation is counted in the Markdown it draws. A bullet is measured,
banded and copied as `• read a file`, not as the `- **read** a file` behind it:
the marks are not on the screen, so there is nothing to point at and nothing to
select. That is also why a copy off the transcript comes back without them.

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
to it. No click collapses a pane; the divider is how a pane gets smaller. **Drag a tab
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

Right click the prompt for Copy and Paste, a pane or its tab for Settings,
Copy selection, Close this widget and Widgets, or the settings panel's document
column for the one row that fits there, Copy selection. A row with nothing to act on is
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
the left with the chosen one beside it. **Click a section to choose it, or Tab to
it.** The arrow keys never touch the rail: up and down walk the rows of whatever
is showing, and left and right change the row the cursor is on. Tab moves on to
the next section and Shift-Tab back to the one before, wrapping at both ends, and
the footer legend says so on every row. Enter flips a switch, turns a skill or a
server on and off, starts an edit on the endpoint and on the field a skill is
installed from, and ends that edit by installing what was typed, Delete arms the
two presses a saved conversation and the restore under the palette take, Shift
with left or right crosses a card, the wheel scrolls, Ctrl-C copies what is
selected in the document column, and Esc puts the panel
away. The rail never hides a name: when the window is too short to list the
sections down one column, they wrap into a second one rather than falling off the
bottom, so a font size raised far enough to fill the screen still leaves
APPEARANCE, where it is lowered again, one click or one Tab away. The panes are exactly where
they were when it closes, and the agent keeps working behind it. **Drag the line
between the rail and the settings** to give one of them more room: the pointer
takes a resize shape over it, neither side goes narrower than the longest section
name, and where you leave it is written to `settings_rail` in the settings file,
so the panel opens that way next time.

The panel is laid out on one scale. Four gaps and five text sizes, all derived
from the pane text size, live in `gui/clippy/src/design.rs`, and `gui/DESIGN.md`
says in words what each of them is for. A group of settings is a card: a
bordered box with its title in a header, one divider under it, its fields inside
with room on all four sides, and its buttons in a footer at the bottom of the
box whatever the body's height. A field is its label above a value, never beside
it, and a value that can be typed into carries the box that says so while a
reading does not. There is no hairline under a row any more: space and the
card's own border say where one group ends, which is what a line between every
two things on screen never did. Cards are full width and stack, and it is a
card's contents that answer a narrow window: two fields go side by side while
both keep their columns and stack when they cannot. Every section is built this
way: AGENT, SESSIONS, SKILLS, MCP and APPEARANCE.

Four of the sections are the agent's own files rather than the window's. AGENT
is five cards, in the order somebody meeting the window needs them: where the
model is (the endpoint and the key it is called with), which model it asks for
(`NOOB_MODEL`, `NOOB_API_STYLE`, `NOOB_REASONING`), what the agent gets, the file
all of it is written in, and whatever else that file carries. Every field is a
plain-words name over its value with one sentence under it naming the key and
saying what it decides, because `NOOB_TASK_CONCURRENCY 4` on its own is not
something anybody can act on. Shift with left or right crosses between the two
fields of a card, and the plain arrow keys go on nudging whatever the cursor is
on. The file is `~/.config/noob/.env`, which the CLI
re-reads on every request; the endpoint is the one line the window types into,
and every other key that is set is read out on a card of its own. The two numbers
are `NOOB_CTX`, the context
window the agent budgets against before it compacts, and `NOOB_TASK_CONCURRENCY`,
how many sub-agent tasks it runs at once. Both are tracks held to the CLI's own bounds, so the context
window starts at the 4096 below which the CLI stops reading it and the right end
of the concurrency track is the 16 the CLI caps itself at: the maximum is a place
to drop the pointer, not a number to guess. Until either is in the file the field
reads what the CLI falls back to and the card says so. That write goes through
a port of the CLI's own `.env` writer, so
every other line and every comment survives it, and the file is read back
afterwards. Nothing is passed to the agent on its command line, because `serve`
rejects a flag it does not know; instead the launch clears those two names out of
the child's environment, since the CLI prefers the environment over its file and
an exported value would outrank every line the panel writes. No credential is ever
shown or written from here: a key, token, secret
or password reads as `set, and not shown here`, which is the same line the CLI
takes by keeping secrets out of settable config.

Under those cards are two blocks of text, cards themselves: the title in the
header, where the text came from under it, and the text scrolling inside the
body. The first is the agent's global
instructions, `<config dir>/AGENTS.md`: the CLI reads that file and puts it at the
top of every prompt, before the project's own `AGENTS.md`, and the block names the
path and shows what is in it. With no file there it says so and Enter writes a
starter one. An edit lands on the next session rather than the next message,
because the prompt is assembled once when `noob serve` starts, and the block says
that too. The second block is the whole prompt: the window runs `noob debug
prompt` in the folder the session is running in, off the interface thread and with
the same two names cleared out of the environment the agent itself is started
with, so what it prints is the prompt the session sends and not a different one.
`AGENTS.md` is one capped layer of it, so it is never labelled as the prompt. A
command that failed puts the reason on the block. Both blocks are a fixed twelve
lines tall and read with Page Up and Page Down, or the wheel over them.

SESSIONS is one card holding a table of the saved conversations the folder
picker offers, read through the same reader. The header says how many there are
and how many are chosen; the body is the table, headed by the names of its
columns on a filled band: a mark, when each conversation was, which folder it
belongs to, how big the transcript is, how full its context window was, and the
opening of what was first said in it. The two number columns are written against
their right edge so they can be read down. The heading at the top of the panel
says SAVED CONVERSATIONS rather than repeating the rail's word, which is the only
place the section is titled. Up and down pick a row and it carries a band across
the whole of it; twelve rows are on screen at once and the rest scroll inside the
card, with the wheel or Page Up and Page Down, so the header and the buttons stay
where they are.

Several conversations go in one press. Space or Enter marks the row the keys are
on, a press on the box in the first column marks that row without moving the
keys, and Ctrl-A or the `select all` button marks every conversation on the list
rather than the twelve on screen. The three buttons are centred in the card's
footer: `select all`, `select none`, and the delete, which says how many it would
take. Press it, or the Delete key, once and it says `sure?` while the line under
the panel says how many conversations would go; press it again and each
transcript and the line about it in `no0b.sessions` both go. With nothing marked
it takes the row the keys are on and names that one instead. When one of several
refuses, the rest still go and the panel says which one refused.

SKILLS and MCP are two columns: a list on the left and, beside it, whatever the
row under the cursor is. In a window too narrow to hold both, the list keeps the
width and the second column is not drawn. Each row of the list is a card, and its
three strings are three different things to look at: the name in the header at
the card title size, what it is for in the body at the ordinary size, wrapped
onto as many lines as it takes, and the repository or the file it came out of
under that, smaller and dim. The card the keys are on wears the focus colour on
its border with a mark down its edge. The column beside the list is a card too,
with the name of whatever it belongs to in its header and the text in its body,
wrapped at whatever width the column has, by the same rule the panes wrap prose
at. Both sides scroll, each in its own box: the wheel moves whichever one the
pointer is over.

The text in that column selects. Drag across it and the characters under the
pointer are banded, Ctrl-C puts them on the clipboard, and the right button
offers the same copy. It is the transcript's selection, over the same kind of
pane: the document is measured in the Markdown it draws, so a bullet copies as
`• read a file` with the marks gone the way they are gone from the screen, and a
run that crosses a wrap comes back with the blank the row broke at. Scrolling the
column moves the band with the text, since a selection holds line numbers. Moving
the cursor onto another entry drops it, because the document under it is then a
different document.

SKILLS opens on the card that installs one. Type a git address, an `owner/name`
shorthand or a folder on this machine into the field and press Enter, or press
the install button in the card's footer. A repository is cloned shallow with a
two minute limit and with git's terminal prompt turned off, so a private URL
fails with a message instead of hanging on a password nobody can see the prompt
for; the skill inside it is the root when that holds a `SKILL.md` and the one
directory under it that does otherwise. It is read and refused before anything is
written: no front matter, no `name`, no `description`, a name that is not lower
case letters, digits and hyphens, or a name already installed (turned off
counts), and the block over the list says which, in git's own words when git is
what refused. It runs on a thread of its own and the window stays live while it
does. What lands is a directory under `~/.config/noob/skills`, and the list is
read back off the disk afterwards rather than from what the install said.

The window does that itself rather than calling the CLI, because there is nothing
there to call: `noob` installs skills only as the REPL command `/skills add`, and
into `<workspace>/.noob/skills`, which is the project's directory and not the one
this section lists. The rules are the CLI's, though, down to the limits on the
two required keys, so a skill this accepts is a skill the agent loads.

SKILLS lists the directories under `~/.config/noob/skills`, named and described
from each `SKILL.md`'s front matter and by the directory name when it has none.
Under the description is the repository the skill records, or, since nothing the
CLI writes down records where an installed skill came from, the directory it was
found in. The column beside the list is that skill's own `SKILL.md`, rendered
the way the transcript renders what the model writes. Each skill is a card: the
name in the header, the description in the body, the repository under it, and a
toggle and an uninstall in the footer.
The toggle really turns the skill off: there is no enabled flag anywhere in the
CLI, so off means the directory is moved to `~/.config/noob/skills.off`, which is
none of the four places the agent looks, and on moves it back. Nothing is
remembered in a settings file; what the row says is where the directory is.
Uninstall deletes the directory, and it takes two presses: the first arms it and
the footer names what is about to go, the second one does it, and anything else
at all puts it back. It will only ever delete a directory sitting directly inside
those two, never a link and never a path that walks out of them.

MCP opens on a card naming both files the CLI merges, `<config>/mcp.json` and
`<workspace>/.noob/mcp.json`, side by side while there is room for both and
stacked when there is not, and says `none configured` under them when neither is
there rather than showing an empty list that reads as broken; a malformed one is
a line saying so, and the servers from the file that did parse are still listed.
Each server is a card of its own with its URL or command line in the body, the
file it came from under that, its entry out of that file beside it, and the same
toggle and uninstall a skill has. Off moves the
entry into a `disabled` object in the same file, which the CLI's loader does not
read, and on moves it back. Uninstall takes the entry out of the file for good,
out of whichever of the two objects it is in, and takes the same two presses:
the first arms it and the footer names the server and the file, the second one
does it. Both go through the same rewrite: the file is parsed whole, one key
changes, and the whole value is written to a temporary file beside it and
renamed, so every other server, every `timeout_s` and anything else in there
survives, and a rewrite that cannot be finished leaves the file that was there.
A file that is not JSON is a file to fix by hand: the panel refuses and says so
rather than overwriting it with what it could parse. There is no field here for
adding one: a server is a name and either a URL or a command with its arguments
and its environment, which is more than the two fields a card can be changed
through and more than one line of text. Add one with `/mcp add <name> <url>` in
the CLI, or write it into either file, and it is on this list next time the
panel opens.

The fifth is the window's own settings file: APPEARANCE. Three cards of
settings, then the palette: how big the text is (the conversation and the
panes), how solid the window is (the panels, and the empty space around them),
and how tall the prompt is. Two things the
file carries are deliberately not rows: which panes are open, which is the right
click menu's list, and where the four dividers and the settings rail sit, which
is set by dragging the lines. Both are still written and read the same way, so a
closed pane and a dragged layout come back at the next launch; there is just
nothing on the panel to type them into. Every field is a plain-words name over
its value with the key of the file in the sentence under it, so the panel says
what a setting does and still answers "which line do I edit", and anything that
can be changed is drawn as a box with an outline round it: if it has an edge, it
takes a press or a keystroke. The box lights up under the pointer and keeps its cut
corner while it is lit, the way every other surface in the window does. A change is written straight away through
the same writer `no0b --set` uses, so the comments stay, and then the whole file
is read back and the window is restyled from it: the palette and both font sizes
move without a restart. That read-back is
also why a row cannot show a value the file will not carry, since what you see is
what the next launch reads.

A setting with a range is a slider: the two opacities, both font sizes and how
tall the prompt is. Press the track and drag it, with the value beside it
and the arrow keys still nudging it one step. The window takes the value while
you drag: the opacity you are dragging to is the only thing that tells you where
to stop. The file is written when the button comes up rather than on every motion
event, which would be hundreds of writes for one decision, and the live value
goes through the same clamps the file is read with, so what you drag to is what
the next launch reads.

**Two transparencies, not one.** `opacity` is the panes, the bars and the menus:
everything with words on it. `window_opacity` is the window itself, the empty
space around and between the panes, which was pinned at 55% of the other one
until this release. So the gaps can go to glass while the text you are reading
stays solid. Both are on the same card, both say which surface they move, and
they open at 90% and 50%, which is what that ratio came to, so the window opens
looking as it always did.

The palette is the rest of APPEARANCE and it opens with the control that writes
it: `THE PALETTE` is a card whose field is the theme, drawn as all three
names side by side with the one the window is wearing filled in, so picking one
is one press and there is nothing to discover by pressing twice. The sentence
under the card names the theme those colours came from. **Picking a theme
applies it**: the pick comments out any colour line in your file that would
override the preset, then writes `theme = <name>`, because an explicit colour
beats the theme it belongs to and a file carrying eight of them answered every
theme change with the same window under a new name. Nothing else in the file is
touched. Then the groups, one card each: `THE WINDOW'S OWN TONES`, `THE CODE
COLOURS`, `THE TOOL MARKS` and `THE METERS`, with the colours inside reflowing
to as many across as the card is wide enough for, each one a block of the colour
with what it paints written beside it in words. The meters say which readings
wear them, so `gauge_7` reads as `ram and prefilled`. Pressing a swatch says
which key in the file writes it, on the line at the foot of the panel; they are
swatches to read rather than fields to edit, since changing one means typing a
hex value and the one field this window has is the endpoint. A file carrying a
palette that is not one of the three exactly reads as `custom`, which is what one
hand-tuned colour over a preset makes it.

**Back to the defaults** is the last card of the section, in the colour this
window keeps for anything that loses work, and it asks once before it acts: the
first press says on the button and on the footer what is about to go, the second
one does it. The keys reach it like any other card, and Delete is the press. What it does is comment out every size, transparency and colour line
in the settings file, so all of them fall back to what the window ships with. The
lines stay in the file as comments, and the dividers, the pane flags, your own
comments and any line this build has never heard of are left exactly as they
were.

`~/.config/noob/no0b.conf`, written with the defaults on first run and
commented. Both opacities, both font sizes, how tall the prompt is, where the
dividers sit, how wide the settings rail is, which panes exist, and the whole palette: the eight base colors, one per tool,
ten gauge slots a monitor reading picks from, and the five the highlighter uses
for code. A key it does not recognise is reported in the ACTIVITY pane rather
than ignored, and a key an older build wrote and this one dropped, such as any of
the `view_*` colours, is read off the floor without a word.

`theme = noob-matrix | noob-cool | noob-red` sets the eight base tones, the five
code colors, the ten gauge slots and the two tool colors that are prose rather
than a tool. noob-matrix is the green the window has always been, noob-cool is
cyan over a blue-black panel, noob-red is warm over a deep maroon bar. The yes
marks stay green in all three: the drop target, the picked row and the showing
tab's line are one colour between them, and a red window whose green went would
have lost that. The gauges come with the palette too, ten cold hues in noob-cool
and ten warm ones in noob-red, still ten you can tell apart as a column. The
twelve tool colors are the exception: they name the tool rather than the window,
so a theme leaves them where they are. The colors ship as commented defaults so
the theme has something to set, so uncomment one line to keep the theme and
override that single color, and know that picking a theme on the panel comments
that line back out. `no0b --set theme=noob-red` writes the theme line from a
terminal without touching the comments; it does not clear an overriding colour,
which the panel does. The names were `noob`, `amber`, `ice`
and `plum` up to 0.7.1; a file still carrying one of those opens on the closest
of the three rather than being told it is a typo.

The window shipped as CLIppy up to 0.6.0 and wrote `clippy.conf` and
`clippy.recent`. The first run under the new name moves each of those to
`no0b.*`, so a tuned palette and the folders you have opened survive the rename.
A file that already exists under the new name wins and the old one is left
alone.

Opacity defaults to 90% and window opacity to 50%. Lower the first to see more
of your desktop through the reading surface; below about 60% a busy wallpaper
starts competing with the text, which is a taste call rather than a bug. Lower
the second, which is the empty space around the panes, as far as you like: there
is nothing written on it.

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
