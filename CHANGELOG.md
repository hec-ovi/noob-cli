# Changelog

Notable changes, newest first. Releases before 0.6.0 are recorded in the git
tags rather than here; this file starts where it was added.

## Unreleased

All of this is CLIppy, the GPU window. The CLI is unchanged.

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
  right walks in, left goes out, typing narrows the list, Enter opens, and a
  folder named on the command line skips the picker as before. Folders chosen
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
- The rolling trend behind each gauge is gone with the bars. The samples are
  still recorded for the graph the hardware pane is getting.
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

### Reverted

- Markdown tables were box drawn and then given sideways scrolling. Both were
  rolled back and tables pass through as written.
