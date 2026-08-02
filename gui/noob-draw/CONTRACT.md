# noob-draw

contractVersion: 1.0.0

## Purpose

Turn a scene of rectangles and text runs into one presented frame: panel
layout algebra, an instanced rectangle pipeline with shaped corners, and glyph
text whose wrapping is delegated to the text-geometry layer.

There is no path rasterizer and no general 2D API. Borders, panels, bars and
text are the whole vocabulary, on purpose.

## Surface

This box is a typed Rust crate, so the signatures below are the schema.
The text-geometry precedent applies: the typed surface mirrors the contract
and is enforced at the test boundary (this crate's `#[cfg(test)]` suite),
not per call.

### Panel: layout algebra

A rectangle in physical pixels, public fields `x, y, w, h: f32`. Every method
returns a new value; nothing mutates.

| Signature | Rule |
|---|---|
| `new(x, y, w, h) -> Panel` | `w` and `h` clamp at 0 |
| `inset(margin: f32) -> Panel` | The content box: text is wrapped to this width and clipped to these edges, from the same numbers. `w` and `h` floor at 1 |
| `split_top(height) -> (Panel, Panel)` | (taken, rest). `height` clamps into the panel; the halves tile it exactly |
| `split_bottom(height) -> (Panel, Panel)` | (rest, taken), same clamp |
| `split_left(width) -> (Panel, Panel)` | (taken, rest), same clamp |
| `row(pad: f32, line: f32) -> Panel` | One centred text row in a bar: exactly `line` tall (clamped into the bar, floor 1), `pad` off each side. For bars too short to survive `inset` |
| `contains(x, y) -> bool` | Half open on the right and bottom edges, so adjacent panels never both claim a pixel |
| `fill(rgba: [f32; 4]) -> Rect` | The panel as one filled rectangle |
| `outline(rgba, width) -> Rect` | The panel filled shape stroked `width` px inside its edge |
| `bottom_edge / left_edge / right_edge(rgba) -> Rect` | 1 px hairlines inside the panel. There is no `top_edge` |

### Rect: one instanced rectangle

`#[repr(C)]`, `bytemuck::Pod`, three `[f32; 4]` members, 48 bytes; the WGSL
struct agrees and a test pins the size. Fields are private; `xywh()`, `rgba()`
and `extra()` read them back so a test can assert a scene without a GPU.

| Signature | Rule |
|---|---|
| `new(x, y, w, h, rgba) -> Rect` | Square, filled, `extra` all zero |
| `radius(px) -> Rect` | Rounded corners, slot `extra[0]` |
| `chamfer(px, corners: u32) -> Rect` | 45 degree cut reaching `px` along each edge of the named corners, slots `extra[1..=2]`. Negative `px` clamps to 0; the shader caps the reach at half the shorter side |
| `stroke(px) -> Rect` | Outline instead of fill, a ring `px` wide inside the edge, slot `extra[3]`. Negative clamps to 0 |
| `for_surface(srgb: bool) -> Rect` | The fill moved into the space the surface writes (sRGB decode on the rgb channels, alpha untouched). `Renderer::draw` applies it; callers do not |

Corner mask constants, clockwise from the top left: `TOP_LEFT` 1, `TOP_RIGHT`
2, `BOTTOM_RIGHT` 4, `BOTTOM_LEFT` 8, `EVERY_CORNER` 15. The shader indexes
these bits, so they cannot be renumbered on the Rust side alone.

### Colour

| Signature | Rule |
|---|---|
| `srgb_to_linear(channel: f32) -> f32` | The sRGB standard's decode, character for character the formula in glyphon's shader |
| `linear_to_srgb(channel: f32) -> f32` | Its inverse, for tests that say what the screen ends up showing; nothing in the draw path calls it |
| `text_color_mode(srgb: bool) -> glyphon::ColorMode` | `Accurate` for an sRGB surface, `Web` otherwise: the glyph half of the same rule `for_surface` is |

### Run and Text: glyph text

`Run { text: String, color: Option<[u8; 4]>, icon: bool }`. `None` color takes
the text's default. Constructors: `plain(text)`, `tinted(text, color)`,
`icon(text, color)` (the embedded symbol font).

| Signature | Rule |
|---|---|
| `Run::wrapped(runs: &[Run], cols: usize, at: text_geometry::Break) -> Vec<Run>` | The same runs broken into rows exactly as text-geometry counts them: a newline between rows, the character each break was spent on dropped. Rows run across the runs, not within each one; colors and icon flags ride along unchanged. `cols` 0 returns the runs as they are |
| `Run::wrapped_under(runs, cols, at, indent: usize) -> Vec<Run>` | The same, with the first `indent` characters of every logical line treated as chrome (a line-number gutter): never wrapped, never counted, and every continuation row starts with `indent` blanks so it lands under the text |

`Text` is text wrapped and clipped to one content box. Public fields: `runs`,
`at: Panel`, `size`, `line_height`, `color: [u8; 4]` (default for runs without
their own), `scroll_lines`, `wrap_cols: Option<usize>`,
`wrap_break: text_geometry::Break`, `wrap_indent: usize`.

| Signature | Rule |
|---|---|
| `new(content, at: Panel, size: f32, color: [u8; 4]) -> Text` | One plain run |
| `rich(runs: Vec<Run>, at, size, color) -> Text` | One shaped buffer per box, not per colored fragment, so forty tokens still wrap as one line |
| `wrap_at(cols) -> Text` | Rows of `cols` columns breaking at a blank (`Break::Word`). `cols` 0 leaves wrapping to the shaper |
| `break_at(cols) -> Text` | Rows of exactly `cols` characters (`Break::Column`), for a box whose caret counts `row * cols + column`. Same 0 rule |
| `hanging(indent) -> Text` | `indent` columns of per-line chrome, continuation rows indented past it. Ignored unless `wrap_cols` is set |
| `line_height(height) -> Text` | Override the default line height |
| `scrolled(lines: f32) -> Text` | Lines scrolled off the top, clamped at 0 |
| `rows_for(size, height) -> usize` | Rows of this text size that fit in this height |
| `line_for(size) -> f32` | `(size * 1.42).round()`, floor 1. The single source of line height: a box sized from a different number clips its own text |

### Scene and Renderer

`Scene` (Default) holds the frame in painter's order on two layers: public
`rects`, `texts`, `over_rects`, `over_texts`, with push helpers `rect`,
`text`, `over_rect`, `over_text`, each returning `&mut Scene`.

| Signature | Rule |
|---|---|
| `Renderer::new(gpu: &noob_gpu::Gpu) -> Renderer` | One per window, reused every frame. Reads the surface format once: whether it is sRGB drives both colour paths |
| `column_width(&mut self, size: f32) -> f32` | Width of one monospace column at this size, measured by shaping ten zeros; `size * 0.6` when no monospace font measures |
| `draw(&mut self, gpu: &mut noob_gpu::Gpu, scene: &Scene, frame: noob_gpu::Frame)` | Draws the scene into the frame and presents it. Order: base rectangles, base glyphs, overlay rectangles, overlay glyphs. Clears to transparent and blends premultiplied, so the desktop shows through wherever nothing is drawn |

### The embedded symbol font

Symbols Nerd Font Mono ships in the binary
(`fonts/SymbolsNerdFontMono-Regular.ttf`, license alongside) and carries the
Codicon, Seti and Devicon sets. `Run::icon` reaches it; font fallback cannot,
because Nerd Font glyphs live in the private use area where fallback has no
script to match on.

`has_glyph(ch: char) -> bool` reports whether that font has a real glyph
(not `.notdef`) for a character. It builds a whole `FontSystem` per call, so
it is for callers' tests, not for a draw path: a missing icon draws as
nothing, and a test is where that surfaces instead of on someone's screen.

## Errors

The closed set has one member. A glyphon text prepare failure is written to
stderr (`noob-draw: base text prepare failed: ...` or the overlay twin) and
the frame is presented without that layer's glyphs; the atlas is trimmed
every frame, so the next frame recovers. A frame with missing text beats a
dead window.

Nothing else returns or throws: no function on this surface returns a
`Result`, and degenerate geometry is clamped by the rules in the tables above.

## Dependencies

| Contract | What is consumed |
|---|---|
| [`gui/noob-gpu/CONTRACT.md`](../noob-gpu/CONTRACT.md) | `Gpu { device, queue, config }`, `width()`, `height()`, `present(frame)`, and `Frame { view }`. `Renderer::new` borrows the `Gpu`; `draw` takes a `Frame` acquired from it and presents through it |
| [`gui/layers/text-geometry/CONTRACT.md`](../layers/text-geometry/CONTRACT.md) | `Break`, `rows_in`, `rows_into`: the one wrap rule. `Break` appears in this surface (`Text::wrap_break`, `Run::wrapped`), so callers share the type |

Crate pins from the workspace `Cargo.toml`: `wgpu` 30, `glyphon` 0.12
(`ColorMode` appears in this surface), `bytemuck` 1. Dev only, pinned in this
crate's own `Cargo.toml`: `naga` 30 with `wgsl-in`, wgpu's WGSL front end at
the same version, which the test `the_shader_compiles` validates the shader
string with, so a WGSL typo is a failing test rather than a black window.

## Invariants

1. A pane's text cannot reach its neighbour. A `Text` wraps to and clips
   against one `Panel` (`Text.at`), so the wrap width and the clip rectangle
   cannot come from different numbers.
2. A floating thing goes on the overlay layer or it is not floating. Each
   layer draws its rectangles in one instanced pass and its glyphs after
   them, so painter's order does not apply between a rectangle and a glyph on
   one layer. The overlay is painted after the whole base layer, its glyphs
   last of all. Two overlapping things that both carry text want two layers,
   and there are only two.
3. A struct shared with a shader is all `vec4` sized members. `Rect` is three
   `[f32; 4]`s, 48 bytes, and the WGSL `Rect` agrees; growing it means
   editing both in the same commit.
4. A colour in the settings file is the colour on the screen. `for_surface`
   and `text_color_mode` read the one fact (is the surface sRGB), so a
   rectangle and a glyph of one configured colour are one shade. Alpha is a
   coverage, never converted.
5. The wrap rule lives in text-geometry and nothing here re-derives it. A
   `Text` that names `wrap_cols` is broken into rows by `Run::wrapped_under`
   before shaping, with the shaper's own wrapping off, so the drawn rows are
   the counted rows character for character. Under a word break the character
   a row broke at is dropped, since it is drawn on neither row; under a
   column break every character is kept.
6. Degenerate input clamps rather than fails: negative panel sizes, oversized
   splits and insets, negative chamfer and stroke, out of range corner bits,
   negative scroll, and a chamfer wider than the shape all land on a defined
   shape.
7. Both layers' rectangles share one storage buffer (base first, overlay
   directly after) and two instance ranges that cover every rectangle exactly
   once. Capacity starts at 256 and grows to the next power of two; the
   conversion buffer is reused, so a steady frame allocates no rectangle
   storage.

## How to modify this blackbox safely

The shader is a string handed to a driver, so the build cannot catch a
mistake in it: `the_shader_compiles` is the gate, and any change to `Rect`'s
shape has to keep `a_rect_is_exactly_three_vec4s` passing on the Rust side and
edit the WGSL struct in the same commit. The `extra` vector is full; a fifth
shape parameter means packing two values into one slot or growing both
structs together.

A caller naming a new icon codepoint asserts `has_glyph` for it in its own
tests, the way `every_icon_the_window_uses_is_in_the_embedded_font` does
here, because a glyph the font lacks draws as nothing.

Callers must not wrap text themselves. If a layout number is missing, the fix
is a new operation in text-geometry's contract, consumed here, not arithmetic
at a call site.

Adding a builder or a `Panel` operation is additive: new method, new row in
the tables, minor `contractVersion` bump. Changing the draw order of the four
passes, the meaning of an `extra` slot, or the wrap delegation is breaking:
add the new shape alongside and migrate callers.
