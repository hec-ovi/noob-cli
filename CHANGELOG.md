# Changelog

Notable changes, newest first. Releases before 0.6.0 are recorded in the git
tags rather than here; this file starts where it was added.

## Unreleased

All of this is NO0B, the GPU window, which shipped as CLIppy up to 0.6.0. The
CLI's behaviour is unchanged; only its version moved.

### Changed, and it renames things

- The window is NO0B. The package, the binary, the desktop entry, the icons, the
  window title and the three files under `~/.config/noob` all carry the product
  name: `no0b` on PATH, `io.github.hec_ovi.NO0B.*` on the desktop, and
  `no0b.conf`, `no0b.recent` and `no0b.totals` beside noob's own settings. A
  file written under the old name is moved to the new one on the first read, so
  a tuned palette, the folders you have opened and the all-time totals survive
  the rename; a file already there under the new name wins and the old one is
  left alone, and a rename that cannot happen falls back to the defaults rather
  than refusing to open a window. Installing removes the old launcher, entry and
  both icons, on install as well as on uninstall, so the menu holds one entry
  instead of two with the older one starting nothing. The folder is still
  `gui/clippy` on disk, which moves no path in `dev.sh` or the docs and changes
  nothing a user sees.
- Both cargo workspaces are 0.7.0, and the title strip reads its version from
  the crate. It drew the commit stamp alone, so nothing on screen or in either
  manifest said which release a build came from.
- TALK is OUTPUT, the pane that said SESSION says CONTEXT, and the one that said
  OVERALL says SESSION. Each variant keeps the slot it already had, so no view's
  accent colour shifted along by one, and `view_talk` and `view_overall` in an
  older settings file still apply their colour under the current names.
- A setting a past build wrote and this one dropped is no longer reported as a
  typo. `show_avatar` and `avatar` were two red lines at startup blaming the
  user for a change the window made. The retired names are listed by hand, the
  writer refuses them too, and an unknown key that is not on that list is still
  reported, so a real typo stays visible.

### Fixed

- The end of a long message is reachable again. Panes handed the text shaper as
  many logical lines as rows fit, so any line that wrapped overflowed its clip
  box and the newest rows were discarded with no scroll position that could
  reach them. Panes count visual rows now.
- Text selection lands on the characters it highlights. Three separate defects:
  the transcript was hit tested at the pane font size while it is drawn at the
  larger one, the files pane was offset by its tab strip and its four column
  gutter, and nothing accounted for wrapping.
- The scrollbar reports how far a pane can actually scroll. Counting lines made
  a pane of wrapped text look shorter than it is.
- Every pane scrolls inside its own box. PLAN and AGENTS drew every row they had
  into one text box, so a long plan or a fleet of eight children ran off the
  bottom edge; DEBUG stopped at the last row that fitted; a monitor stopped at the
  last reading that fitted. None of the four had a scrollbar, so nothing said
  there was more, and nothing could reach it. All of them now take their window,
  their clamp and their thumb from `text-geometry`, the same as the transcript,
  with one offset per view held in one place and clamped against the content every
  frame, so a pane whose list shrank under a scroll is not left blank. A click in
  DEBUG adds the window's own offset back, or a scrolled pane would open a
  different call than the one under the pointer. Every row of a monitor pane is now
  the same height whether it draws a block or not, which is what lets the pane say
  how much of itself is on screen.
- A tab strip that cannot hold all of its tabs can be walked. Tabs were laid out
  left to right until one did not fit and the rest were dropped, with nothing
  saying they were there: the top right space opens with six tabs, and at the
  window's minimum width of 680 most of them were gone. A strip that overflows
  now keeps room for a `<` and a `>` at its right end before it decides which
  tabs fit, and each one steps the strip and the tab it is showing along by one.
  A strip that fits shows neither and loses no room to them. The offset is
  clamped against the room the strip actually has on every frame, so a resize, a
  closed tab or a tab dragged elsewhere cannot leave a space scrolled past its
  last tab, and the pane on screen always has its own tab in the strip.
- Dragging a tab in front of another one reorders the tabs. A move always pushed
  the tab onto the end of the target space and did nothing at all when that space
  was the one it was already in, so there was no way to change the order of a
  strip in either direction. A drop on a strip now names a place among its tabs,
  in front of the tab under the pointer or behind it depending on which half of it
  the pointer is in, and the tab lands there whether it came from that space or
  another. A drop in the body of a pane still names the space alone and lands at
  the end of it, so a drag that ends where it started changes nothing, including
  which tab was showing.
- A drag says where it would land. The only feedback was a hairline along the
  target pane's edges, and once the pane lost its top edge that outline no longer
  closed around anything. The space a drop would land in now takes a translucent
  green box over its tab strip and its pane, with a caret standing in the gap
  between the two tabs the tab would go between. Both are painted on the floating
  layer, along with the tab following the pointer, so they read over the pane
  instead of under its text.
- Dragging a tab outside the window says that letting go closes it. The drop
  already closed the widget and nothing announced it. The pointer becomes a
  crosshair while a drag is outside the surface, and the tab in the air is drawn
  in the bad colour with no target boxed anywhere, because there is nowhere out
  there to land.
- The folder picker keeps one shape and keeps every row. Its box was as tall as
  the folder had entries, so walking from a folder with three subfolders into one
  with forty resized the dialog, recentred it and moved every row out from under
  the pointer. The height now comes from the room the window has, between six and
  twenty-four rows, and is held whatever is being listed; a short folder leaves
  the bottom of its list empty, which is the price of a dialog that stays put.
  Typing no longer takes rows away either: every folder stays in the list and the
  ones that do not carry what was typed are drawn dim. The arrows walk the
  matches and step over the dim rows, a click still lands on one and Enter still
  opens it, and one function in the model answers for the drawing, the keyboard
  and the click alike, so a row cannot be dim on screen and live to the keyboard.
  The folder being listed and the way out of it are never dim, because they are
  how the list is walked rather than entries in it. A name starting with a dot is
  still out of the list until what has been typed starts with one.
- The folder picker's list is a tree, and the row the cursor is on can be seen.
  The list was one directory and walking into a folder replaced the whole of it,
  so looking inside two folders meant walking in and back out twice. A folder
  now carries a plus that puts what is inside it into the list under it, one
  step further in, and a minus that takes it back out; the keyboard still walks
  in and out, which is the fast way down a deep tree. A folder that cannot be
  read says why on a row of its own under it rather than opening to nothing,
  which on screen is a folder that is empty. Shutting a folder that held the
  cursor somewhere inside it leaves the cursor on that folder, and a folder
  opening above the cursor carries the cursor down with the row it was on. The
  plus is a target of its own inside the row, so pressing it opens the folder
  and pressing the row selects it. The row the cursor is on used to be the same
  quiet band the file explorer marks its open row with, which on a list of forty
  folders said almost nothing: it is now filled solid in the theme's green with
  the theme's darkest colour written over it.
- The picker's Open button reads as a button. It was drawn in the quietest fill
  in the palette with a hairline around it, and it spelled out the folder it
  would open, which made it as wide as a path and a different width every time
  the cursor moved. It says "Open", sits on a surface of its own with a brighter
  one under the pointer, carries the same 45 degree corner cut every panel in the
  window has, and is sized to the one word it says. The folder it would open is
  already written above the list.
- The dock icon appears as soon as the window opens. The desktop entry asks for
  a startup notification and nothing answered it, so the cursor spun until
  GNOME's own timeout, around fifteen seconds.
- Nothing is drawn in the corner a panel's cut takes away. The tab strip's fill
  and the hairline along its foot were square rectangles that ran past the cut
  and left a stray stroke there, and a pane's scrollbar started inside the same
  triangle, hanging outside the pane.
- The window buttons are no longer clipped. Their text box was sized to one
  estimated glyph advance, so maximize lost all but the left edge of its frame
  and close all but one arm of its cross.
- Double clicking the title bar shades the window to the strip and nothing else.
  It painted a black bar with stray lines across it, because the shade asked the
  window to become thirty pixels tall while a minimum size of 680 by 380 was
  still in force: the compositor refused the height, the surface stayed tall, and
  a full window backdrop was painted over it. The minimum is dropped while
  shaded and restored when it is not. Dropping the minimum is a request too, and
  a compositor can still keep the surface at the height it had, which happened
  unless the window was maximized: whatever surface comes back is now filled in
  the bar's own colour, so a shaded window reads as a green bar at any height and
  carries the window's transparency setting either way.

### Added

- The thinking orb, in the square at the left end of the title strip, so the
  strip reads orb, name, version. A port of the `orbits` mode of `thinking-orbs`
  against its source rather than its look: twelve tilted circles laid out from a
  deterministic hash, one shared spin and tilt, projected orthographically,
  sorted far to near and drawn as discs through the rounded rect field the window
  already has. No shader and no second pipeline, because a disc is a rectangle
  with its corner radius set to half its width. Two states and no third: while a
  turn is running it turns, 516 discs a frame; at rest it is the same globe
  frozen and fainter without its runners, 480, so the corner is never empty and
  never moves for no reason. Its clock is a 30 frames a second deadline that
  exists only while the agent is working and is composed with the monitor's
  sampling deadline rather than replacing it, so a window with nothing happening
  in it goes back to blocking until you touch it. It is the only animation in
  the window.
- A floating layer in the scene. Every rectangle in a frame was drawn in one
  instanced pass and every glyph in one pass after it, so a rectangle could never
  cover a glyph: the right click menu's box is a rectangle and the panes behind it
  are text, and the text won, which made the rows illegible over any pane with
  writing in it. `Scene::over_rect` and `Scene::over_text` push to a second layer
  painted after the whole base layer, and everything that floats over the window
  uses them.
- A redrawn icon: a console in hollow wire with `>_` inside it, one path of four
  subpaths whose winding directions are what keep the body open, so whatever the
  icon sits on shows through it and the top right corner takes the same 45 degree
  cut every panel takes. The halo is one bounded blur merged under the mark, so a
  rasterizer with filters off draws the same four subpaths sharp and loses only
  the light. Its radius reaches further than the mark's margin, so the canvas
  trims the tails where they are at 7% alpha, which is the price of the brighter
  glow rather than a bug: the filter region is declared rather than inherited, so
  the canvas is the only thing that ever clips it. The 16 pixel variant is drawn
  again on its own grid rather than divided down, because the prompt does not
  survive the division and renders as a grey wedge.
- A closed widget can be opened again from the menu that closed it. Closing one
  took it out of the window with nothing anywhere putting it back, so the only
  way home was a restart. The widget menu now ends in a Widgets row that opens a
  list of all nine under itself, each marked with whether it is in the window or
  closed, so the list is also the answer to where a pane went. Picking a closed
  one puts it back in the space it opens in by default, since where it used to be
  is not remembered and an arrangement dragged around since would have nowhere to
  put it; picking one that is already in the window shows it where it is and
  unfolds its space, which reaches a tab behind five others without hunting for
  its strip. The list is collapsible because nine permanent rows in front of a
  Close row is a wall, and it is last so opening it moves no row above it, which
  is the one place the menu is allowed to change height. It is clamped into the
  window like the menu itself, and where the window is too short for all of it
  the four rows above are kept and the wheel scrolls the rest.

- The first thing the window shows can now open a session that already exists.
  The picker opened on folders alone, so every launch started a fresh
  conversation and the only way back into an old one was the CLI. A Sessions
  button beside Open swaps the list for the sessions the agent has written,
  read out of `<config>/sessions` on the press rather than at launch, and the
  same box, cursor, keys, filter and scrolling drive both lists. A row says how
  long ago the session was, which folder it belongs to and the opening of the
  first thing that was said in it, and choosing one starts the agent with
  `--resume <id>` in that folder. Sessions for the folder being looked at come
  first. Only the head of each file is read (64 KiB), so a transcript that has
  been running all afternoon costs the same as a short one, and a file that is
  truncated mid-line or is not a session at all is one row missing with the
  count said above the list rather than a list that refuses to draw. The
  transcript format does not record which folder a session happened in, so the
  window keeps its own note in `no0b.sessions` beside the settings, written when
  the agent reports a session has started; a session written by the CLI on its
  own has no note and resumes in the folder above the list, and one whose folder
  has since been deleted is drawn as unopenable and says so when it is pressed
  rather than starting an agent in a directory that is not there.
- The dividers between the panes can be dragged. The left column took 0.54 of
  the width and the top right space 0.46 of the right column's height, both
  written into the layout as literals, so the only thing that could be
  rearranged was which space a tab lived in. The grid stays capped at 2x2 rather
  than becoming a split tree: the three spaces are the same three spaces, and
  what moves is where the two dividers sit. Neither can be dragged past what a
  space needs to be read, a tab strip plus the padding and either four rows of
  gauge dots or twenty-four columns of text, and a window with no room for two
  of those floors splits down the middle instead of collapsing one of them. The
  band the pointer grabs is fourteen pixels across a six pixel gap and the
  pointer takes a resize shape over it, which is the only thing that can say a
  divider is there, since a divider is nothing but the gap between two panes.
  Where they were left goes into the settings file as `left_width` and
  `top_height` when the drag ends, not on every motion event, and both are rows
  on the settings panel as well.
- The settings panel, opened from the Settings row of any pane's right click
  menu. It takes the whole window under the title strip: arrows move, left and
  right change the row the cursor is on, Enter flips a switch, the wheel scrolls,
  and Esc closes it. That row had been greyed since it was added, with no panel
  behind it, which reads as a broken window rather than an unfinished one.
  Every key the file understands is on the panel, as the key and the value the
  file spells, so it doubles as the reference for editing that file by hand. A
  change goes through the same comment-preserving writer `no0b --set` uses and
  the whole file is then read back, so the palette, both font sizes and the two
  panes that can be turned off all change without a restart, and a row cannot
  show a value the next launch will not read. The palette is swatches to read
  rather than fields to edit, since nothing in the window can take the keyboard
  focus yet, and the `theme` row repaints the window in a preset. The all-time totals sit
  above the settings, with this session added in.
- A folder picker, drawn with the same rectangles as the rest of the window.
  Launched from the dock with no argument, CLIppy called `current_dir()`, which
  under a desktop launcher is your home directory, and handed the agent that
  without saying so. Now the window opens on a list of folders: arrows move,
  right walks in, left goes out, typing dims everything it did not match, Enter
  opens, and a folder named on the command line skips the picker as before. Folders chosen
  before sit at the top of the list, remembered in `~/.config/noob/no0b.recent`
  beside the settings, so the second launch is one keystroke. No native dialog:
  a toolkit file chooser is dozens of crates and a portal at runtime.
- Three monitors where there was one. CONTEXT is how full this run is: which
  phase, model and workspace, the context fill, the requests and tool calls it
  took to get there, and what the last request alone prefilled and generated.
  SESSION is what the run has spent: tokens prefilled, generated and served from
  cache, and the measured prefill and decode rates. DEBUG is the calls that
  failed, and clicking one shows the arguments that were sent to it. What DEBUG
  cannot show is the schema the tool expected: no event carries one.
- Running totals that outlive the window, in `~/.config/noob/no0b.totals`
  beside the settings. Tokens prefilled, generated and served from cache, plus a
  mean and a median decode and prefill speed. The median needs the samples
  themselves, so the file keeps the last 512 per-request rates; a mean has
  already forgotten which requests it was made of. Written by rename at the end
  of every turn and when the window closes. A missing file is a first run and an
  unreadable one reads as zero. No pane shows them: a column of counts from
  sessions nobody remembers reads as this session's, so they are on the settings
  panel instead, under a heading that says ALL TIME.
- A gauge palette: ten colour slots, and every monitor reading names the one it
  wears. A metric keeps its colour across panes, so prefill is the same blue in
  CONTEXT's LAST PREFILL as in SESSION's PREFILLED.
- Right click menus. The prompt offers Copy and Paste; a pane or the tab that
  names it offers Settings, Copy selection and Close this widget. A row with
  nothing to act on is greyed rather than dropped, so the menu is the same shape
  every time it opens and the row you were aiming for has not moved. A menu is a floating layer,
  drawn after everything else and hit tested before it, so a click that lands on
  one cannot reach what it covers.
- Closing a widget, from that menu or by dragging its tab off the window. A space
  left with no tabs gives its room to its neighbour. There is no way back inside
  the window yet; reopening comes with the launcher.
- Selecting in the prompt with the pointer, and Ctrl+V. A pasted newline arrives
  as a space, since the prompt is one wrapped line and Enter is what sends it.
- `gui/layers/text-geometry`, the first contract-isolated layer: it owns the
  rule for turning logical lines into rows on screen, which was previously
  written out at eight call sites and disagreed with itself at three. Ships with
  a contract, JSON schemas, fixtures that must be rejected, and its own tests.
  At 1.1.0 it also converts a top-anchored position, which is what a list wants,
  into the scrollback everything else there speaks.
- A build stamp in the title bar. The crate version cannot tell two test builds
  apart, so the commit is stamped in at build time, with a trailing plus when
  the tree has uncommitted changes.
- An embedded symbol font. The window marks and the file type marks are real
  glyphs instead of hand drawn rectangles, and every codepoint the window names
  is asserted to exist in the font, because a missing glyph draws as nothing at
  all rather than failing.
- Cut corners and strokes in the rect shader. Panels carry a ten pixel forty
  five degree cut on their top right corner, and a bordered panel is one
  stroked rectangle instead of four hairlines.
- The whole palette is settings. Every colour is a config key: the base tones,
  one per tool, one per view, ten gauge slots and the five the highlighter uses.
  There are four named themes, and the writer preserves the comments in your
  file.
- Prompt editing: Ctrl+A selects the line, clicking places the caret, and how
  tall the input grows is a setting.
- One colour per view, with the showing tab carrying an accent line in it.
- `docs/INDEX.md`, which maps the thing you want to change to the one folder to
  open.

### Changed

- The file list is a column down the left of the FILES pane, one row per file
  with its type icon, and the diff beside it. It was a strip of tabs across the
  top, which ran out of room at about six files and dropped the rest. The column
  scrolls, the wheel over it moves the list rather than the file, and the row of
  whatever the agent just touched is marked and scrolled to. It is a flat set,
  not a filesystem, so there are no directory rows and nothing to expand.
- A gauge is a block of dots: twenty across and four down, in the metric's own
  colour, filling from the bottom, with the number beside it at the pane's own
  size. It was ten columns of four small dots in one shared colour, which read as
  a smear, then eight across and five down, which stood the panes on end. One row
  is 25 percent and one dot is 1.25. A reading with no maximum draws no block at
  all now: it used to draw an empty track, so most of a pane was empty rectangles
  and the two rows that were filled read as noise. The block is as chunky as its
  pane can afford and shrinks before it pushes a reading off the bottom; a pane
  too narrow for a legible dot drops the block and keeps the number, since the
  number is the reading and the block only describes it.
- Token counts in the monitors are grouped in thousands. Seven figures ungrouped
  has to be counted rather than read.
- The rolling trend behind each gauge is gone with the bars, and so is the ring
  of samples that fed it. Nothing drew them.
- The hardware pane shows only its readings. The notes and the GPU capability
  report underneath are gone, which also means a machine without an amdgpu no
  longer says why those rows are missing.
- The status bar along the bottom is gone and every pane gained its height. The
  context gauge became a hairline under the title strip.
- The title strip carries the window name and the build stamp and nothing else.
  The phase word, the model, the workspace and the token budget were readings
  crammed onto one unlabelled line up there; they are monitor readings and the
  monitors have room to label them. The stamp reads at the text tint now, not
  the faintest one the palette has, which is what a build stamp is for.
- A tab is not a button. A tab strip has no surface of its own and is the same
  ground as the window behind it. A tab carries the pane's surface, at full
  strength with its view's accent line when it is showing and at a lower alpha
  with a dimmer label when it is not, so the difference is weight rather than a
  filled block. Every tab takes the same ten pixel corner cut the panes take.
- There is no line under a tab strip. It was the top edge of the pane's own
  border, and with the showing tab in the pane's colour and flush against it, the
  two are one surface. A pane is bordered on its left, right and bottom, as three
  hairlines; the right one starts where the corner cut ends. Boxes that are their
  own thing (the prompt, the picker, a menu) still have all four sides.
- The fold arrow at the end of each tab strip is gone. Clicking the tab already
  showing still collapses its space.

### Removed

- The ASCII clip player and the converter that fed it. The orb took the idle
  animation and lives in the title strip, where a 128 by 37 character face has
  nowhere to go, so `avatar.rs`, the `asciify` crate, the clip it produced and
  the `./dev.sh avatar` route are all gone. The clip never appeared in a shipped
  build. The `show_avatar` and `avatar` settings stay retired rather than
  becoming unknown keys, because those lines are still sitting in people's
  files.

### Reverted

- Markdown tables were box drawn and then given sideways scrolling. Both were
  rolled back and tables pass through as written.
