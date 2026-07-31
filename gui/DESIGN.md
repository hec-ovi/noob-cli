# How the window is laid out

The rules every panel follows, so two people building two sections build the
same thing. This describes the settings panel first, because that is where the
rules were written, but the tokens and the type scale are the window's.

The short version of what went wrong before it existed: everything was one text
size, every row was full width, and the only thing separating two rows was a
hairline. A hairline between every row is not structure, it is noise, and a
reader cannot tell a group title from a field label from a value when all three
are the same size. Structure comes from grouping and space. Lines are the last
resort, not the first.

The arithmetic is in `clippy/src/design.rs`; this file is what it means.

## Space

Four tokens, and nothing in between them. Each one is a fraction of one line of
pane text, so raising the pane size scales the panel instead of leaving the text
to overrun its boxes. The pixel column is what they come to at the size the
window opens at.

| Token | Of a line | Pixels | Used for |
|---|---|---|---|
| `TIGHT` | 0.2 | 4 | Between a label and the thing it labels |
| `STEP` | 0.4 | 8 | Between fields inside a card |
| `ROOM` | 0.7 | 14 | A card's inner padding, on all four sides |
| `APART` | 1.0 | 20 | Between one card and the next |

A card's body never touches its border: `ROOM` on every side, including under the
header and above the footer. Two cards never touch: `APART` between them.

## Type

Five roles, five sizes, derived from the pane size so the whole panel scales with
one setting. Nothing on a panel is drawn at a size that is not on this list.

| Role | Size | Tone | Used for |
|---|---|---|---|
| Panel title | 1.6x | bright | The one heading across the top of the panel |
| Card title | 1.15x | good | The title bar of a card, upper case |
| Label | 1.0x | dim | The name of a field, above the field |
| Value | 1.0x | bright | What the field holds, and any content |
| Hint | 0.85x | dim | The sentence under a field that explains it |

Two things follow, and both have already cost a day when they were ignored:

- A row must claim its height in whole line units before it draws at a size
  other than the pane size, because the model measures rows and the layout draws
  them, and a row measured at one height and drawn at another puts every click
  below it on the wrong row.
- Anything that lines text up with a rectangle uses the column width of that
  text's own size. Two sizes, two column widths. Only the pane size's advance is
  really measured, so the other roles get theirs scaled from it
  (`design::column_for`): good enough to clip a line with, and never to be
  measured against something that is pressed.

## A card

Every group of related settings is a card. A card is the only grouping device on
a panel; there are no bare rows outside one.

**A card is one row of the panel.** The scroll window counts rows and places them
by a running height, so a box that spanned several rows could not be counted. It
follows that a list whose rows carry actions is a stack of cards, one card per
row, rather than one card with a list inside it: that is what a skill and a
server are.

```
+-- CARD TITLE ----------------------------+   <- header: title, then a divider
|                                          |
|  Label                                   |   <- body, ROOM inside the border
|  [ value                               ] |
|                                          |
|  Label                                   |
|  [ value                               ] |
|  the sentence that explains this field     <- hint
|                                          |
|                        [ Primary ] [ x ] |   <- footer: buttons, bottom right
+------------------------------------------+
```

- **Border**: one hairline in `skin.edge`, with the window's 10px cut on the top
  right corner like every other surface. Stroked, not filled with a second rect.
- **Header**: the title in the card title role, then a divider the full width of
  the card. This is the one divider a card gets.
- **Body**: fields, stacked. `STEP` between them, `TIGHT` between a label and its
  input, hints directly under the input they explain.
- **Footer**: only when the card has actions. The buttons sit at the bottom of
  the card, always, whatever the body's height. A card with no actions has no
  footer and no divider above where one would be.
- **No hairline between fields.** Space separates them. The card's border is what
  says where the group ends.
- **Focus**: the card the keys are on carries `skin.edge_focus` on its own border
  and the mark down its left edge. Not a band: a filled block nine lines tall is
  a highlight nobody can read through.

## A field

Label above, input below, hint under that. Never label and value side by side on
one line: that is what made every value look like part of a sentence.

- The input is a bordered box, `INPUT_PAD` inside, filled with `skin.input`.
- A read-only reading uses the same shape with no border and no fill, so what can
  be typed into is obvious at a glance.
- A slider is the input box with the track inside it and the number at the right
  end of the same box.
- A choice with a small number of options is drawn as all of the options, with
  the current one banded. A choice the user cannot see the options of is a choice
  they will not know they have.

## A button

Three kinds, and no others.

| Kind | Fill | Border | Ink | For |
|---|---|---|---|---|
| Primary | `skin.button` | none | bright | The action the card exists for |
| Secondary | `skin.input` | `skin.edge` | text | Cancel, and anything reversible |
| Danger | none | `skin.close_hot` | `skin.bad` | Delete, uninstall, anything that loses data |

Buttons carry the same corner cut as everything else, and they light on hover:
a primary in `skin.button_hot`, the other two in `skin.hot`. A destructive button
asks before it acts: the first press arms it and it says so, the second press
does the thing.

A toggle is drawn as a primary while it is on and a secondary while it is off.
The fill is what says the thing is live, and the two words it holds are the same
width, so pressing it does not resize the thing that was just pressed.

## Text roles inside a list

A row in a list (a skill, a server, a saved conversation) carries several strings
and they must not look alike. The three roles:

- **Name**: the card's own title, in the card title role. What you are looking
  for when you scan.
- **Description**: value size, text tone, wrapped, never cut with an ellipsis.
- **Origin** (a path, a repository, an address): hint size, dim.

Where the list is a stack of cards, the row the cursor is on is the card wearing
the focus border. A table that is really a table (the saved conversations) keeps
the full row band in `skin.picked` with its ink in `skin.picked_ink`: a band, not
a tint on the text, because a tint is invisible next to fourteen other tints.

## Scrolling

- One scrollbar per scrollable region, drawn inside that region's right padding,
  never over its content and never over another region's bar.
- A region that can scroll always shows its bar, so "can this scroll" is
  answerable without trying it. A region that cannot scroll draws no bar.
- The wheel scrolls the region under the pointer, not the panel behind it.
- The bar reports the real extent, so the thumb's size says how much is off
  screen.

## Resize

The panel is used at every size from the window's minimum to a full screen, so
nothing is laid out for one width.

- Cards are full width and stack down the panel.
- A card's own contents reflow: fields go two across when the card is wide enough
  for both to keep their minimum, and one across when it is not.
- Nothing is ever clipped to unreadability. A string that does not fit wraps; a
  control that does not fit moves to its own line. The last resort is clipping,
  and only for a single line whose tail is not load bearing.
- Every control keeps a minimum in columns, and the layout gives up a column
  count rather than drawing a control nobody can hit.

## What this replaces

The panel was a flat list of full width rows at one text size, separated by a
hairline under every row, with buttons wherever the row happened to put them and
values on the same line as their labels. If you are changing a section and it
still looks like that, the section has not been done yet.

SKILLS and MCP are built this way. AGENT, SESSIONS and APPEARANCE are not yet:
their rows are the old flat ones, without the hairline, and they are what the
next passes convert.
