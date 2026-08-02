# widgets/agent

contractVersion: 1.0.0

## Purpose

The agent-output widget: one running sub-agent's own lines as a scrolled
pane, the body of the `[N] AGENT - OUTPUT` tab a click on the AGENTS pane
opens. With no agent chosen it says so and draws nothing else.

## Surface

One painter: props in, draw calls out. It reads `frame.state.agent_shown()`
(the chosen agent's bounded pane) and the layout's panel, pushes rects and
text into the scene, and owns nothing between frames.

## Invariants

1. Pure paint: no state outside the call, no clock, no filesystem.
2. Placement, paint, and hit regions use the same rectangles: the panel
   handed in is the one the layout hit-tests.
3. Which agent shows is the state box's `shown_agent`; this box never
   chooses.

## Dependencies

Contracts: the view box (Frame, the shared chrome, the one dispatch),
[`noob-draw`](../../../../noob-draw/CONTRACT.md) (Scene, Panel, Run), the
state box (the chosen agent and its pane), the style box (colors).

## Tests

Scene-level: this box renders its own window through `view::testkit` and reads what was drawn (1 tests in all here).
