# no0b

contractVersion: 1.0.0

## Purpose

The GPU window. `src/main.rs` is the composition root: the winit event
loop, the App that owns every box's state, and the input routing that
turns hits and keys into box calls. Every capability lives in a box with
its own contract.

## The boxes behind it

Models: dock, prompt, select, scroll, state (the reducer), sessions,
config, agent (agent-files), menu, monitor, orb, design. Surfaces: view
(layout, hit, chrome), widgets/* (one per pane), settings, picker.
Machinery: link (the serve child), install, packaging, style, plus the
crates noob-gpu and noob-draw and the text-geometry layer. `docs/INDEX.md`
maps them all.

## The shell's own rules

1. The shell routes and owns lifecycle; it decides nothing a box could
   decide. Display state the reducer must not hold (scrolls, selection,
   the file-follow policy) lives here.
2. One frame is a function of what it is handed: the shell snapshots
   state into a Frame and the view builds the scene from that alone.
3. The agent is reached only through the link box, which speaks only the
   serve contract.

## Tests

The shell's routing tests live in `src/main.rs`; every box carries its
own. One command runs everything: `./dev.sh test-all`.
