# widgets/agents

contractVersion: 1.0.0

## Purpose

The agents widget: the live fleet as list rows, each led by the bright
`[N] Agent` a click means. Finished children are not rows: the state box
removes them in the event that ends them.

## Surface

One painter plus one resolver: `agents` draws the list from
frame.state.agents; `agent_at(frame, panel, x, y)` answers which agent's
ordinal a point is over, through the same rows and scroll window the
painter draws with (a press on the head row or the news line under it both
name that agent). Extent questions (how many rows exist) go through
`view::scroll_extent`, which asks this box's row builders where they exist.

## Invariants

1. Pure paint: no state outside the call, no clock, no filesystem.
2. Placement, paint, and hit regions use the same rectangles: the panel
   handed in is the one the layout hit-tests.
3. Adding a widget is adding a folder here plus one dispatch arm in the
   view's build; nothing else changes.

## Dependencies

Contracts: the view box (Frame, the shared chrome and list vocabulary, the
one dispatch), [`noob-draw`](../../../../noob-draw/CONTRACT.md) (Scene,
Panel, Run), the state box (what it reads), the style box (colors).

## Tests

Scene-level: this box renders its own window through `view::testkit` and reads what was drawn (2 tests in all here).
