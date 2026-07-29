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
- The dock icon appears as soon as the window opens. The desktop entry asks for
  a startup notification and nothing answered it, so the cursor spun until
  GNOME's own timeout, around fifteen seconds.
- The window buttons are no longer clipped. Their text box was sized to one
  estimated glyph advance, so maximize lost all but the left edge of its frame
  and close all but one arm of its cross.

### Added

- `gui/layers/text-geometry`, the first contract-isolated layer: it owns the
  rule for turning logical lines into rows on screen, which was previously
  written out at eight call sites and disagreed with itself at three. Ships with
  a contract, JSON schemas, fixtures that must be rejected, and its own tests.
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
- The whole palette is settings. All twenty seven colours are config keys, there
  are four named themes, and the writer preserves the comments in your file.
- Prompt editing: Ctrl+A selects the line, clicking places the caret, and how
  tall the input grows is a setting.
- One colour per view, with the showing tab carrying an accent line in it.
- `docs/INDEX.md`, which maps the thing you want to change to the one folder to
  open.

### Changed

- Gauges are dot grids rather than solid bars.
- The hardware pane shows only its readings. The notes and the GPU capability
  report underneath are gone, which also means a machine without an amdgpu no
  longer says why those rows are missing.
- The status bar along the bottom is gone and every pane gained its height. The
  phase word and the token budget moved into the title strip and the context
  gauge became a hairline beneath it.
- The fold arrow at the end of each tab strip is gone. Clicking the tab already
  showing still collapses its space.

### Reverted

- Markdown tables were box drawn and then given sideways scrolling. Both were
  rolled back and tables pass through as written.
