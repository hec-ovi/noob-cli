//! G1 and G2: rectangles and glyphs, which between them are the whole visual
//! vocabulary of a retro chrome.
//!
//! A caller builds a [`Scene`] out of [`Rect`]s and [`Text`]s in painter's
//! order and hands it over. There is no path rasterizer, no vector fills and no
//! general 2D API, on purpose: borders, panels, bars and text are what this
//! interface is made of, and a general renderer is where GPU UIs go to die.
//!
//! Two rules are enforced by the types rather than by care.
//!
//! **A pane's text cannot reach its neighbour.** [`Panel::inset`] produces one
//! content box, and the wrap width and the clip rectangle are both taken from
//! it. Deriving those from different numbers is what let a paragraph wrap at
//! the window width while being clipped at the pane width, so it crossed the
//! divider.
//!
//! **A floating thing goes on the overlay layer or it is not floating.** One
//! instanced pass draws a layer's rectangles and one text pass draws its glyphs
//! after them, so pushing a box last does not put it over text that was pushed
//! earlier. [`Scene::over_rect`] and [`Scene::over_text`] are the second layer,
//! painted after the whole base layer, and they are what a menu, a panel or a
//! drag preview has to use.
//!
//! **A struct shared with a shader is all `vec4`-sized members.** WGSL aligns a
//! `vec3` to 16 bytes and Rust does not. A `{[f32;4], [f32;4], f32, [f32;3]}`
//! is 48 bytes here and 64 there, which silently corrupted every rectangle
//! after the first. [`Rect`] is three `[f32; 4]`s and the shader agrees.
//!
//! **A colour in the settings file is the colour on the screen.** A palette is
//! written the way a colour picker writes one, in sRGB, and the surface this
//! draws into is usually an sRGB one, which encodes whatever a shader wrote on
//! its way into the texture. A colour handed straight to the shader therefore
//! arrives lighter than it was asked for: the bar, `#0e2e1e`, was read back off
//! the window as `#427660`. [`srgb_to_linear`] undoes that encode before the
//! colour reaches the shader, and [`text_color_mode`] tells glyphon to do the
//! same to a glyph's colour in its own shader. Both read one fact, whether the
//! surface format is an sRGB one, so rectangles and text of the same configured
//! colour cannot come out different shades. Glyphs were already right and
//! rectangles were not, which is exactly the drift the single flag prevents.

use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Wrap,
};
use std::ops::Range;

/// A rectangle in physical pixels. The only way geometry is expressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panel {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Panel {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Panel {
        Panel {
            x,
            y,
            w: w.max(0.0),
            h: h.max(0.0),
        }
    }

    /// The content box: text is wrapped to this width AND clipped to these
    /// edges, from the same numbers.
    pub fn inset(self, margin: f32) -> Panel {
        Panel {
            x: self.x + margin,
            y: self.y + margin,
            w: (self.w - 2.0 * margin).max(1.0),
            h: (self.h - 2.0 * margin).max(1.0),
        }
    }

    /// Split off `height` from the top, returning (taken, rest).
    pub fn split_top(self, height: f32) -> (Panel, Panel) {
        let height = height.clamp(0.0, self.h);
        (
            Panel {
                h: height,
                ..self
            },
            Panel {
                y: self.y + height,
                h: self.h - height,
                ..self
            },
        )
    }

    /// Split off `height` from the bottom, returning (rest, taken).
    pub fn split_bottom(self, height: f32) -> (Panel, Panel) {
        let height = height.clamp(0.0, self.h);
        (
            Panel {
                h: self.h - height,
                ..self
            },
            Panel {
                y: self.y + self.h - height,
                h: height,
                ..self
            },
        )
    }

    /// A single centred row of text inside a bar: `pad` off each side, exactly
    /// `line` tall, sitting in the middle of whatever height is left.
    ///
    /// This exists because insetting a 22-pixel bar by a 9-pixel margin leaves
    /// a 4-pixel box, and text taller than its box is clipped to nothing. The
    /// result looks like the text was never drawn, and the caret beside it
    /// looks misplaced because it is the only thing still visible. A bar wants
    /// one line centred, not a margin, so it says so.
    pub fn row(self, pad: f32, line: f32) -> Panel {
        let line = line.min(self.h).max(1.0);
        Panel {
            x: self.x + pad,
            y: self.y + ((self.h - line) * 0.5).max(0.0).floor(),
            w: (self.w - 2.0 * pad).max(1.0),
            h: line,
        }
    }

    /// Split off `width` from the left, returning (taken, rest).
    pub fn split_left(self, width: f32) -> (Panel, Panel) {
        let width = width.clamp(0.0, self.w);
        (
            Panel { w: width, ..self },
            Panel {
                x: self.x + width,
                w: self.w - width,
                ..self
            },
        )
    }

    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    pub fn fill(self, rgba: [f32; 4]) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h, rgba)
    }

    /// A hairline along an edge, which is how a pane reads as a pane without
    /// spending four rectangles on a border. There is no `top_edge`: a line
    /// above a pane sat under its tab strip and separated the tab from its own
    /// surface, so nothing draws one any more.
    pub fn bottom_edge(self, rgba: [f32; 4]) -> Rect {
        Rect::new(self.x, self.y + self.h - 1.0, self.w, 1.0, rgba)
    }

    pub fn left_edge(self, rgba: [f32; 4]) -> Rect {
        Rect::new(self.x, self.y, 1.0, self.h, rgba)
    }

    pub fn right_edge(self, rgba: [f32; 4]) -> Rect {
        Rect::new(self.x + self.w - 1.0, self.y, 1.0, self.h, rgba)
    }

    /// A hairline all the way round, inside the panel, as one rectangle.
    ///
    /// This was four 1px rectangles until the fragment stage learned to stroke.
    /// Four of them cannot follow a cut or a rounded corner, so a bordered panel
    /// had a square outline around a shaped fill.
    pub fn outline(self, rgba: [f32; 4], width: f32) -> Rect {
        self.fill(rgba).stroke(width)
    }

    fn bounds(self) -> TextBounds {
        TextBounds {
            left: self.x as i32,
            top: self.y as i32,
            right: (self.x + self.w) as i32,
            bottom: (self.y + self.h) as i32,
        }
    }
}

/// One instanced rectangle.
///
/// Square by construction, and shaped by `extra` rather than by more members:
/// the struct size is what the shader agrees with, so the shape parameters had
/// to fit in the space that was already there.
///
/// `extra` is now full. The scheme, which the WGSL fragment stage reads back in
/// this order:
///
/// | slot | meaning |
/// |---|---|
/// | `x` | corner radius in pixels, 0 for square corners |
/// | `y` | chamfer size in pixels: how far a 45 degree cut reaches along each edge, 0 for no cut |
/// | `z` | which corners that cut applies to, as the 4 bit mask below |
/// | `w` | stroke width in pixels, 0 to fill the shape instead of outlining it |
///
/// A fifth parameter has nowhere to go. Either share a slot (two small
/// integers packed into one float, decoded in the shader) or accept that
/// growing the struct means editing the WGSL `Rect` in the same commit, which
/// is the mismatch the module docs describe.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Rect {
    xywh: [f32; 4],
    rgba: [f32; 4],
    /// Radius, chamfer size, corner mask, stroke width. A `vec4` on both sides,
    /// never a bare trailing scalar. See the type and module docs.
    extra: [f32; 4],
}

impl Rect {
    /// Corner bits for [`Rect::chamfer`], clockwise from the top left. Held as
    /// a mask rather than four booleans because it travels to the shader as one
    /// float.
    ///
    /// Only `TOP_RIGHT` has a caller: the skin cuts one corner. The other three
    /// are not dead code waiting to be swept, they are the Rust half of a shader
    /// contract. The fragment stage loops over all four bits, so dropping the
    /// names would leave a capability nothing can ask for and an `EVERY_CORNER`
    /// mask covering corners with no name.
    pub const TOP_LEFT: u32 = 1;
    pub const TOP_RIGHT: u32 = 2;
    pub const BOTTOM_RIGHT: u32 = 4;
    pub const BOTTOM_LEFT: u32 = 8;
    pub const EVERY_CORNER: u32 = 15;

    pub fn new(x: f32, y: f32, w: f32, h: f32, rgba: [f32; 4]) -> Rect {
        Rect {
            xywh: [x, y, w, h],
            rgba,
            extra: [0.0; 4],
        }
    }

    /// Rounded corners, in pixels. Present because the primitive supports it,
    /// not because the skin uses it.
    pub fn radius(mut self, px: f32) -> Rect {
        self.extra[0] = px;
        self
    }

    /// A 45 degree cut across the named corners, reaching `px` along each edge.
    ///
    /// The shader caps the reach at half the shorter side, so a 10 pixel cut on
    /// a pane squeezed down to 12 pixels loses its corner instead of losing the
    /// whole rectangle.
    pub fn chamfer(mut self, px: f32, corners: u32) -> Rect {
        self.extra[1] = px.max(0.0);
        self.extra[2] = (corners & Rect::EVERY_CORNER) as f32;
        self
    }

    /// Draw the outline instead of the fill: a ring `px` wide lying inside the
    /// edge, which is where the four separate 1px rectangles used to sit.
    pub fn stroke(mut self, px: f32) -> Rect {
        self.extra[3] = px.max(0.0);
        self
    }

    /// Position and size, so a caller can assert a scene without a GPU.
    pub fn xywh(&self) -> [f32; 4] {
        self.xywh
    }

    /// The fill, for the same reason: telling one rectangle from another in a
    /// scene means knowing what colour it is.
    pub fn rgba(&self) -> [f32; 4] {
        self.rgba
    }

    /// The shape parameters, in the packing the type docs describe. Same
    /// reason again: a test asserts the shape of a rectangle without a GPU.
    pub fn extra(&self) -> [f32; 4] {
        self.extra
    }

    /// The same rectangle with its fill in the space the surface writes.
    ///
    /// An sRGB surface encodes what the shader gives it, so the fill goes in as
    /// the linear value that encodes back to the colour that was asked for. A
    /// surface that encodes nothing takes the colour as it is. Alpha is a
    /// coverage rather than a colour and is never converted, on either path.
    pub fn for_surface(mut self, srgb: bool) -> Rect {
        if srgb {
            for channel in &mut self.rgba[..3] {
                *channel = srgb_to_linear(*channel);
            }
        }
        self
    }
}

/// One channel of an sRGB colour as the linear value behind it.
///
/// The formula the sRGB standard writes, and character for character the one in
/// glyphon's own shader, so a glyph and a rectangle of the same colour are
/// converted by the same curve rather than by two that nearly agree.
pub fn srgb_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// The encode an sRGB surface applies on the way into the texture: the inverse
/// of [`srgb_to_linear`].
///
/// Nothing in the draw path calls this. It is here so a test can put a palette
/// colour through both halves and say what the screen ends up showing, which is
/// the only claim about colour worth making.
pub fn linear_to_srgb(channel: f32) -> f32 {
    if channel <= 0.003_130_8 {
        channel * 12.92
    } else {
        1.055 * channel.powf(1.0 / 2.4) - 0.055
    }
}

/// Which way glyphon converts a glyph's colour, from the same fact
/// [`Rect::for_surface`] reads.
///
/// `Accurate` is glyphon's sRGB to linear conversion, which is what an sRGB
/// surface needs and what its default already was. `Web` passes the colour
/// through untouched, which is what a surface that encodes nothing needs.
pub fn text_color_mode(srgb: bool) -> ColorMode {
    if srgb {
        ColorMode::Accurate
    } else {
        ColorMode::Web
    }
}

/// The family name of the embedded symbol font.
///
/// Symbols Nerd Font Mono, shipped in the binary rather than looked for on the
/// system. It carries the Codicon, Seti and Devicon sets, which is what a
/// window button and a file-type mark need.
const ICON_FAMILY: &str = "Symbols Nerd Font Mono";

/// The bytes of that font, embedded at build time.
const ICON_FONT: &[u8] = include_bytes!("../fonts/SymbolsNerdFontMono-Regular.ttf");

/// A font system holding the system fonts plus the embedded symbol font.
fn icon_fonts() -> FontSystem {
    let mut system = FontSystem::new();
    system.db_mut().load_font_data(ICON_FONT.to_vec());
    system
}

/// Whether the embedded symbol font has a real glyph for this character.
///
/// A font returns `.notdef` for a character it lacks, and `.notdef` draws as
/// nothing, so an icon that is simply absent looks exactly like an icon that
/// was never asked for. Callers naming codepoints should assert this in a test
/// rather than discover it on someone's screen.
///
/// Builds a whole `FontSystem` per call, which is why this is for tests and not
/// for a draw path.
pub fn has_glyph(ch: char) -> bool {
    let mut fonts = icon_fonts();
    let mut buffer = Buffer::new(&mut fonts, Metrics::new(14.0, 20.0));
    buffer.set_size(Some(64.0), Some(32.0));
    buffer.set_text(
        &ch.to_string(),
        &Attrs::new().family(Family::Name(ICON_FAMILY)),
        Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut fonts, false);
    buffer
        .layout_runs()
        .flat_map(|run| run.glyphs.iter())
        .all(|g| g.glyph_id != 0)
}

/// A stretch of characters that share a color. `None` means the run takes the
/// text's default, which is the common case and costs no per-run state.
#[derive(Clone, Debug)]
pub struct Run {
    pub text: String,
    pub color: Option<[u8; 4]>,
    /// Draw this run in the icon font rather than the monospace one.
    ///
    /// Named on the run rather than left to font fallback. Nerd Font glyphs
    /// live in the private use area, where fallback has no script to match on,
    /// and a glyph that silently resolves to nothing is the exact failure that
    /// made the window buttons rectangles in the first place.
    pub icon: bool,
}

impl Run {
    /// The same runs, broken into rows the way `text-geometry` says they break,
    /// so the shaper has nothing left to wrap.
    ///
    /// The rule itself is not here and must not be: the window counts the rows
    /// of a line to place a selection, a caret and a scrollbar, and a second
    /// version of the rule living in the renderer is what put the highlight on
    /// different characters from the ones on screen. This walks the runs,
    /// hands each logical line to [`text_geometry::rows_in`], and lays the
    /// answer out: a newline between the rows, and the one character a row
    /// broke at dropped, since it is drawn on neither row.
    ///
    /// The rows run across the runs, not within each one: a row of a
    /// syntax-colored file is a dozen runs and it is still one row. Counting
    /// per run would break after every token. Colors, icon flags and the run
    /// boundaries are untouched, so what comes back shapes into the same
    /// spans it went in as.
    pub fn wrapped(runs: &[Run], cols: usize, at: text_geometry::Break) -> Vec<Run> {
        Run::wrapped_under(runs, cols, at, 0)
    }

    /// The same, for a box that draws a fixed strip of chrome in front of every
    /// line: the line-number gutter of the file view.
    ///
    /// `indent` is how many columns that chrome takes, and the first `indent`
    /// characters of each logical line are it. They are never wrapped and never
    /// counted as text; what follows them is broken into rows of `cols` by the
    /// same [`text_geometry::rows_in`] call the window counts the line with, and
    /// every row after the first starts with `indent` blanks so it lands under
    /// the text rather than under the gutter.
    ///
    /// That is what makes every row of a line the same width. Wrapping the
    /// gutter along with the text instead gave the first row `cols` characters
    /// and every row under it `cols + indent`, so the band, the caret and the
    /// clipboard were all four columns out from the second row of a file line
    /// down.
    pub fn wrapped_under(
        runs: &[Run],
        cols: usize,
        at: text_geometry::Break,
        indent: usize,
    ) -> Vec<Run> {
        if cols == 0 {
            return runs.to_vec();
        }
        // One pass to lay the whole box out, because a row can start in one run
        // and end in another and a break opportunity can sit either side of a
        // boundary.
        let whole: String = runs.iter().map(|run| run.text.as_str()).collect();
        let count = whole.chars().count();
        // For every character of the box: whether it is drawn, and whether a
        // row starts in front of it.
        let mut kept = vec![true; count];
        let mut breaks = vec![false; count];
        let mut line = 0usize;
        let mut rows = Vec::new();
        for segment in whole.split('\n') {
            let length = segment.chars().count();
            // The chrome in front of the line, which is drawn as it is. A line
            // shorter than its own gutter is all chrome and has nothing to
            // wrap.
            let head = indent.min(length);
            let text = match segment.char_indices().nth(head) {
                Some((byte, _)) => &segment[byte..],
                None => "",
            };
            let mut covered = 0;
            text_geometry::rows_into(text, cols, at, &mut rows);
            for (index, row) in rows.iter().enumerate() {
                if index > 0 {
                    breaks[line + head + row.start] = true;
                }
                // Whatever the rows leave out is a character the break was
                // spent on, drawn on neither side of it.
                for gap in covered..row.start {
                    kept[line + head + gap] = false;
                }
                covered = row.end;
            }
            for gap in covered..length - head {
                kept[line + head + gap] = false;
            }
            // Past the segment sits the newline that ended it, which is a
            // character of the box and stays exactly as it is.
            line += length + 1;
        }
        let mut at_char = 0;
        runs.iter()
            .map(|run| {
                let mut text = String::with_capacity(run.text.len());
                for ch in run.text.chars() {
                    if breaks[at_char] {
                        text.push('\n');
                        for _ in 0..indent {
                            text.push(' ');
                        }
                    }
                    if kept[at_char] {
                        text.push(ch);
                    }
                    at_char += 1;
                }
                Run {
                    text,
                    ..run.clone()
                }
            })
            .collect()
    }

    pub fn plain(text: impl Into<String>) -> Run {
        Run {
            text: text.into(),
            color: None,
            icon: false,
        }
    }

    pub fn tinted(text: impl Into<String>, color: [u8; 4]) -> Run {
        Run {
            text: text.into(),
            color: Some(color),
            icon: false,
        }
    }

    /// A run of glyphs from the embedded symbol font.
    pub fn icon(text: impl Into<String>, color: [u8; 4]) -> Run {
        Run {
            text: text.into(),
            color: Some(color),
            icon: true,
        }
    }
}

/// Text wrapped and clipped to one content box.
///
/// Built from runs rather than one string so a transcript, a diff and a
/// syntax-colored file are all the same primitive. One shaped buffer per box,
/// not per colored fragment: colors are attributes on a single layout, so a
/// line of forty tokens still wraps as one line.
pub struct Text {
    pub runs: Vec<Run>,
    pub at: Panel,
    pub size: f32,
    pub line_height: f32,
    /// The color of any run that does not name its own.
    pub color: [u8; 4],
    /// Lines scrolled off the top. A pane showing the tail of a long stream
    /// sets this to `lines - visible` and pays for the visible rows only.
    pub scroll_lines: f32,
    /// Lay this box out in rows of this many columns, rather than letting the
    /// shaper decide where a row ends.
    ///
    /// What a monospace pane needs. Everything around one of these boxes counts
    /// characters: how many rows a line takes, which row a pointer is over, and
    /// which characters a selection on that row is holding. None of that is
    /// true unless the rows the shaper lays out are the rows the window
    /// counted, and no shaper wraps the way the window counts on its own: it
    /// swallows the blank at each break, so the row below starts one character
    /// further along than the arithmetic says. One swallowed blank per break is
    /// what put a space nobody could see into a copied selection, and it only
    /// showed up once the window was narrow enough to wrap. A box that names
    /// its column count is broken into rows before shaping instead, by the same
    /// `text-geometry` call the window counts with.
    pub wrap_cols: Option<usize>,
    /// Where those rows are allowed to end.
    ///
    /// Prose reads in words, so a pane breaks at a blank. The prompt places its
    /// caret as `row * cols + column`, so it breaks on the column and takes the
    /// mid-word break that comes with it. Ignored unless `wrap_cols` is set.
    pub wrap_break: text_geometry::Break,
    /// Columns of chrome drawn in front of every line, which the rows after the
    /// first are indented past.
    ///
    /// The file view's line-number gutter, and nothing else so far. The number
    /// is written once, on the first row of the line, and the rows it continues
    /// onto start under the text rather than under the number, which is what
    /// keeps every row of the line `wrap_cols` characters wide. Ignored unless
    /// `wrap_cols` is set.
    pub wrap_indent: usize,
}

impl Text {
    pub fn new(content: impl Into<String>, at: Panel, size: f32, color: [u8; 4]) -> Text {
        Text::rich(vec![Run::plain(content)], at, size, color)
    }

    pub fn rich(runs: Vec<Run>, at: Panel, size: f32, color: [u8; 4]) -> Text {
        Text {
            runs,
            at,
            size,
            line_height: Text::line_for(size),
            color,
            scroll_lines: 0.0,
            wrap_cols: None,
            wrap_break: text_geometry::Break::Word,
            wrap_indent: 0,
        }
    }

    /// Lay this box out in rows `cols` columns wide, breaking at a blank so
    /// words stay whole. Ignored when `cols` is zero, which is what a box too
    /// narrow for one column reports.
    pub fn wrap_at(mut self, cols: usize) -> Text {
        self.wrap_cols = (cols > 0).then_some(cols);
        self.wrap_break = text_geometry::Break::Word;
        self
    }

    /// Lay this box out in rows of exactly `cols` characters, wherever that
    /// falls. For a box whose caret and whose click both count
    /// `row * cols + column`, which is the prompt: a row that ended early would
    /// put the caret a word away from the character it is on.
    /// Keep `indent` columns in front of every line for chrome the box draws
    /// itself, and start the rows a line continues onto under its text rather
    /// than under that chrome. The file view's line-number gutter.
    pub fn hanging(mut self, indent: usize) -> Text {
        self.wrap_indent = indent;
        self
    }

    pub fn break_at(mut self, cols: usize) -> Text {
        self.wrap_cols = (cols > 0).then_some(cols);
        self.wrap_break = text_geometry::Break::Column;
        self
    }

    pub fn line_height(mut self, height: f32) -> Text {
        self.line_height = height;
        self
    }

    pub fn scrolled(mut self, lines: f32) -> Text {
        self.scroll_lines = lines.max(0.0);
        self
    }

    /// Rows of this text size that fit in a panel of this height.
    pub fn rows_for(size: f32, height: f32) -> usize {
        (height / Text::line_for(size)).floor().max(0.0) as usize
    }

    /// The height one line of this text size occupies. The single source of
    /// that number: a box sized from a different one clips its own text.
    pub fn line_for(size: f32) -> f32 {
        (size * 1.42).round().max(1.0)
    }
}

/// Everything to draw this frame, in painter's order, on two layers.
///
/// **A rectangle cannot cover a glyph on the same layer, so a floating thing
/// needs the other one.** [`Renderer::draw`] paints every rectangle of a layer
/// in one instanced pass and then every glyph of that layer in one text pass,
/// because that is what makes a window this size one draw call and one text
/// pass. The cost is that painter's order stops applying between a rectangle and
/// a glyph: a menu box pushed last still landed under the pane text it covered,
/// and the menu was unreadable over anything with writing in it.
///
/// So the scene has an overlay layer. [`Scene::over_rect`] and
/// [`Scene::over_text`] push to it, and it is painted after both base passes,
/// its rectangles first and its glyphs last. Anything that floats over the
/// window (the right click menu, and whatever else grows one) goes there and
/// nothing in the base layer can reach it. Within a layer it is still painter's
/// order, and a rectangle in the overlay still cannot cover a glyph in the
/// overlay: two things that overlap and both carry text want two layers, and
/// there are only two.
#[derive(Default)]
pub struct Scene {
    pub rects: Vec<Rect>,
    pub texts: Vec<Text>,
    /// The floating layer's rectangles, painted after all of the above.
    pub over_rects: Vec<Rect>,
    /// The floating layer's glyphs, painted after everything else there is.
    pub over_texts: Vec<Text>,
}

impl Scene {
    pub fn rect(&mut self, rect: Rect) -> &mut Scene {
        self.rects.push(rect);
        self
    }

    pub fn text(&mut self, text: Text) -> &mut Scene {
        self.texts.push(text);
        self
    }

    /// A rectangle on the floating layer, painted after the whole base layer.
    pub fn over_rect(&mut self, rect: Rect) -> &mut Scene {
        self.over_rects.push(rect);
        self
    }

    /// Text on the floating layer, painted after everything else in the frame.
    pub fn over_text(&mut self, text: Text) -> &mut Scene {
        self.over_texts.push(text);
        self
    }

    /// The two instance ranges [`Renderer::draw`] issues, in draw order.
    ///
    /// Both layers' rectangles live in one storage buffer, base first and
    /// overlay directly after it, so there is still one buffer and one bind
    /// group and the two passes differ only by which instances they name. The
    /// arithmetic is here, once, because an off by one at the call site silently
    /// drops the last rectangle of the base layer or draws one of them twice,
    /// and neither looks like a bug worth chasing.
    fn instances(&self) -> (Range<u32>, Range<u32>) {
        let base = self.rects.len() as u32;
        let total = base + self.over_rects.len() as u32;
        (0..base, base..total)
    }

    /// How many rectangles the storage buffer has to hold: both layers.
    fn rect_count(&self) -> usize {
        self.rects.len() + self.over_rects.len()
    }
}

const SHADER: &str = r#"
struct Rect {
    xywh: vec4<f32>,
    rgba: vec4<f32>,
    extra: vec4<f32>,
};
@group(0) @binding(0) var<uniform> screen: vec4<f32>;
@group(0) @binding(1) var<storage, read> rects: array<Rect>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) rgba: vec4<f32>,
    @location(1) local: vec2<f32>,
    @location(2) half_size: vec2<f32>,
    // radius, chamfer, corner mask, stroke width. Flat: the mask is a bitfield
    // read back with integer arithmetic, and an interpolated 2.0 that arrives
    // as 1.9999997 cuts the wrong corner.
    @location(3) @interpolate(flat) extra: vec4<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let r = rects[ii];
    let corner = corners[vi];
    let px = r.xywh.xy + corner * r.xywh.zw;
    let ndc = vec2<f32>(px.x / screen.x * 2.0 - 1.0, 1.0 - px.y / screen.y * 2.0);
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.rgba = r.rgba;
    out.half_size = r.xywh.zw * 0.5;
    out.local = (corner - vec2<f32>(0.5, 0.5)) * r.xywh.zw;
    out.extra = r.extra;
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let radius = in.extra.x;
    let cuts = u32(in.extra.z + 0.5);
    let stroke = in.extra.w;

    // Signed distance to the rectangle; radius 0 is an ordinary square corner.
    let q = abs(in.local) - (in.half_size - vec2<f32>(radius, radius));
    var d = length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - radius;

    // A 45 degree cut is a half-plane: in the corner's own quadrant, where both
    // coordinates are positive, everything past x + y = hx + hy - c is outside.
    // Intersecting (max) with the rectangle takes that wedge away. Capped at the
    // shorter half side so an oversized cut eats a corner, not the rectangle.
    let chamfer = min(in.extra.y, min(in.half_size.x, in.half_size.y));
    if chamfer > 0.0 {
        let reach = in.half_size.x + in.half_size.y - chamfer;
        let signs = array<vec2<f32>, 4>(
            vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0),
            vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
        );
        for (var i = 0u; i < 4u; i = i + 1u) {
            if (cuts & (1u << i)) != 0u {
                let p = in.local * signs[i];
                d = max(d, (p.x + p.y - reach) * 0.70710678);
            }
        }
    }

    // The outline: the ring between the shape and the shape shrunk by the
    // stroke width. Inside the edge, where the four 1px rectangles it replaces
    // were, so a stroked panel still ends exactly where the panel ends.
    if stroke > 0.0 {
        d = max(d, -(d + stroke));
    }

    let a = in.rgba.a * (1.0 - smoothstep(-1.0, 1.0, d));
    // Premultiplied: the surface composites against the desktop behind it.
    return vec4<f32>(in.rgba.rgb * a, a);
}
"#;

/// Which of the two layers a text pass belongs to. Private: a caller says which
/// layer by which push it calls, not by naming one of these.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Layer {
    Base,
    Over,
}

impl Layer {
    fn name(self) -> &'static str {
        match self {
            Layer::Base => "base",
            Layer::Over => "overlay",
        }
    }
}

/// The renderer. One per window, reused for every frame.
pub struct Renderer {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    storage: wgpu::Buffer,
    bind_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    capacity: usize,
    /// Whether the surface encodes what is written to it. The one fact both
    /// colour paths read: it picks the conversion for a rectangle's fill and
    /// the [`ColorMode`] the glyph atlas was built with.
    srgb: bool,
    /// Both layers' rectangles, in the space the surface writes, reused every
    /// frame. Held here rather than built fresh so converting a colour costs no
    /// allocation per frame.
    written: Vec<Rect>,
    font_system: FontSystem,
    swash: SwashCache,
    atlas: TextAtlas,
    viewport: Viewport,
    text: TextRenderer,
    /// The overlay layer's glyphs. A second text renderer over the same atlas
    /// and the same viewport, which glyphon supports: each renderer owns a
    /// vertex buffer and `render` only reads the atlas. It exists because the
    /// overlay's glyphs have to be drawn after the overlay's rectangles, and one
    /// renderer prepared once can only be drawn once.
    over_text: TextRenderer,
}

impl Renderer {
    pub fn new(gpu: &noob_gpu::Gpu) -> Renderer {
        let device = &gpu.device;
        let format = gpu.config.format;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rect"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rect"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rect"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("rect"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screen"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let capacity = 256;
        let storage = new_storage(device, capacity);
        let bind_group = bind(device, &bind_layout, &uniform, &storage);

        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let srgb = format.is_srgb();
        let mut atlas = TextAtlas::with_color_mode(
            device,
            &gpu.queue,
            &cache,
            format,
            text_color_mode(srgb),
        );
        let text = TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        let over_text =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);

        Renderer {
            pipeline,
            uniform,
            storage,
            bind_layout,
            bind_group,
            capacity,
            srgb,
            written: Vec::new(),
            font_system: icon_fonts(),
            swash: SwashCache::new(),
            atlas,
            viewport,
            text,
            over_text,
        }
    }

    /// Width of one monospace column at this text size.
    ///
    /// Measured by shaping, not guessed from the size: a guess is off by enough
    /// to misplace a caret by several characters across a line.
    pub fn column_width(&mut self, size: f32) -> f32 {
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(size, size * 1.42));
        buffer.set_size(Some(4096.0), Some(size * 2.0));
        buffer.set_text(
            "0000000000",
            &Attrs::new().family(Family::Monospace),
            Shaping::Basic,
            None,
        );
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer
            .layout_runs()
            .next()
            .map(|run| run.line_w / 10.0)
            .filter(|w| *w > 0.0)
            .unwrap_or(size * 0.6)
    }

    /// Draw one scene into one acquired frame, and present it.
    ///
    /// Four steps, in this order: base rectangles, base glyphs, overlay
    /// rectangles, overlay glyphs. Both rectangle steps read one storage buffer
    /// at two instance ranges and both glyph steps share one atlas, so a second
    /// layer costs one more draw call and one more text pass, not a second
    /// buffer or a second atlas.
    pub fn draw(&mut self, gpu: &mut noob_gpu::Gpu, scene: &Scene, frame: noob_gpu::Frame) {
        let (w, h) = (gpu.width(), gpu.height());
        self.ensure_capacity(&gpu.device, scene.rect_count());
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::cast_slice(&[w, h, 0.0f32, 0.0]));
        let (base, over) = scene.instances();
        // The overlay's rectangles go directly after the base layer's, which is
        // what makes the second instance range a range and not a second
        // binding, and every fill lands in the space the surface writes on the
        // way past.
        self.written.clear();
        self.written.extend(
            scene
                .rects
                .iter()
                .chain(&scene.over_rects)
                .map(|rect| rect.for_surface(self.srgb)),
        );
        if !self.written.is_empty() {
            gpu.queue
                .write_buffer(&self.storage, 0, bytemuck::cast_slice(&self.written));
        }

        // Buffers must outlive `prepare`, which borrows them through TextArea.
        let base_buffers = self.shape(&scene.texts);
        let over_buffers = self.shape(&scene.over_texts);

        self.viewport.update(
            &gpu.queue,
            Resolution {
                width: gpu.config.width,
                height: gpu.config.height,
            },
        );
        self.prepare_text(gpu, Layer::Base, &scene.texts, &base_buffers);
        self.prepare_text(gpu, Layer::Over, &scene.over_texts, &over_buffers);

        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("no0b"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Fully transparent: whatever is behind the window shows
                        // through anywhere nothing is drawn.
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !base.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..6, base);
            }
            let _ = self.text.render(&self.atlas, &self.viewport, &mut pass);
            // The pipeline and the bind group are set again because the text
            // pass in between bound its own, and then the overlay's glyphs go
            // last so nothing at all can be drawn over them.
            if !over.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..6, over);
            }
            let _ = self.over_text.render(&self.atlas, &self.viewport, &mut pass);
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.present(frame);
        self.atlas.trim();
    }

    /// Shape one layer's text into one buffer per box. Separate from `prepare`
    /// because the buffers have to outlive the borrow `prepare` takes of them.
    fn shape(&mut self, texts: &[Text]) -> Vec<Buffer> {
        let mono = Attrs::new().family(Family::Monospace);
        texts
            .iter()
            .map(|item| {
                let mut buffer = Buffer::new(
                    &mut self.font_system,
                    Metrics::new(item.size, item.line_height),
                );
                // Wrap width and clip rectangle from the same content box.
                buffer.set_size(Some(item.at.w), Some(item.at.h));
                // A box that names its column count is broken into rows here,
                // so there is nothing left for the shaper to wrap and no
                // chance of it putting a row boundary somewhere the arithmetic
                // around the box did not.
                let wrapped;
                let runs = match item.wrap_cols {
                    Some(cols) => {
                        buffer.set_wrap(Wrap::None);
                        wrapped =
                            Run::wrapped_under(&item.runs, cols, item.wrap_break, item.wrap_indent);
                        wrapped.as_slice()
                    }
                    None => item.runs.as_slice(),
                };
                match runs {
                    [only] if only.color.is_none() => {
                        buffer.set_text(&only.text, &mono, Shaping::Basic, None);
                    }
                    runs => {
                        let spans = runs.iter().map(|run| {
                            let attrs = Attrs::new().family(if run.icon {
                                Family::Name(ICON_FAMILY)
                            } else {
                                Family::Monospace
                            });
                            let attrs = match run.color {
                                Some([r, g, b, a]) => attrs.color(Color::rgba(r, g, b, a)),
                                None => attrs,
                            };
                            (run.text.as_str(), attrs)
                        });
                        buffer.set_rich_text(spans, &mono, Shaping::Basic, None);
                    }
                }
                if item.scroll_lines > 0.0 {
                    let mut scroll = buffer.scroll();
                    scroll.vertical = item.scroll_lines * item.line_height;
                    buffer.set_scroll(scroll);
                }
                buffer.shape_until_scroll(&mut self.font_system, false);
                buffer
            })
            .collect()
    }

    /// Prepare one layer's glyphs on that layer's own text renderer.
    ///
    /// Both layers fail the same way, on purpose: a frame with missing text is
    /// better than a dead window, and the atlas recovers on the next frame once
    /// it has been trimmed. An overlay that panicked the window when the atlas
    /// filled up would be worse than the bug this layer was added to fix.
    fn prepare_text(
        &mut self,
        gpu: &noob_gpu::Gpu,
        layer: Layer,
        texts: &[Text],
        buffers: &[Buffer],
    ) {
        let areas: Vec<TextArea> = texts
            .iter()
            .zip(buffers)
            .map(|(item, buffer)| TextArea {
                buffer,
                left: item.at.x,
                top: item.at.y,
                scale: 1.0,
                bounds: item.at.bounds(),
                default_color: Color::rgba(
                    item.color[0],
                    item.color[1],
                    item.color[2],
                    item.color[3],
                ),
                custom_glyphs: &[],
            })
            .collect();
        let renderer = match layer {
            Layer::Base => &mut self.text,
            Layer::Over => &mut self.over_text,
        };
        if let Err(e) = renderer.prepare(
            &gpu.device,
            &gpu.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash,
        ) {
            eprintln!("noob-draw: {} text prepare failed: {e:?}", layer.name());
        }
    }

    fn ensure_capacity(&mut self, device: &wgpu::Device, needed: usize) {
        if needed <= self.capacity {
            return;
        }
        self.capacity = needed.next_power_of_two();
        self.storage = new_storage(device, self.capacity);
        self.bind_group = bind(device, &self.bind_layout, &self.uniform, &self.storage);
    }
}

fn new_storage(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rects"),
        size: (capacity * std::mem::size_of::<Rect>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    storage: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rect"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: storage.as_entire_binding(),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape one string in a family and report the glyph ids it resolved to.
    /// Glyph 0 is `.notdef`, the empty box a font returns for a character it
    /// does not have.
    fn glyph_ids(text: &str, family: Family) -> Vec<u16> {
        let mut fonts = icon_fonts();
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(14.0, 20.0));
        buffer.set_size(Some(400.0), Some(40.0));
        buffer.set_text(text, &Attrs::new().family(family), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut fonts, false);
        buffer
            .layout_runs()
            .flat_map(|run| run.glyphs.iter().map(|g| g.glyph_id))
            .collect()
    }

    /// The window buttons were drawn as bare rectangles because the glyphs they
    /// wanted were not on this machine, and a missing glyph renders as nothing
    /// rather than as an error. The font ships in the binary now, so the test
    /// that matters is that these codepoints resolve to real glyphs.
    #[test]
    fn every_icon_the_window_uses_is_in_the_embedded_font() {
        // Codicon window controls, Seti file types, a folder and a gear.
        let icons = [
            ("close", '\u{eab8}'),
            ("maximize", '\u{eab9}'),
            ("minimize", '\u{eaba}'),
            ("markdown", '\u{e609}'),
            ("python", '\u{e606}'),
            ("javascript", '\u{e60c}'),
            ("rust", '\u{e7a8}'),
            ("folder", '\u{e5ff}'),
            ("terminal", '\u{ea85}'),
            ("gear", '\u{f013}'),
        ];
        for (name, ch) in icons {
            let ids = glyph_ids(&ch.to_string(), Family::Name(ICON_FAMILY));
            assert_eq!(ids.len(), 1, "{name} shaped to {} glyphs", ids.len());
            assert_ne!(
                ids[0], 0,
                "{name} (U+{:04X}) resolved to .notdef, so it would draw as nothing",
                ch as u32
            );
        }
    }

    /// And the embedded font must not displace the monospace one: ordinary
    /// prose still has to shape in the text face.
    #[test]
    fn loading_the_symbol_font_leaves_monospace_text_alone() {
        let ids = glyph_ids("hello", Family::Monospace);
        assert_eq!(ids.len(), 5, "five characters, five glyphs");
        assert!(
            ids.iter().all(|id| *id != 0),
            "monospace text resolved to .notdef: {ids:?}"
        );
    }

    /// The shader is a `&str` handed to a driver, so nothing else in the build
    /// notices a WGSL mistake. Parse and validate it the way wgpu does.
    #[test]
    fn the_shader_compiles() {
        let module = naga::front::wgsl::parse_str(SHADER).expect("the shader parses");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .expect("the shader validates");
    }

    /// The alignment bug, caught by the type system rather than by eye. Rust
    /// must agree with WGSL that this is three `vec4`s, or every instance after
    /// the first reads the previous one's tail as its own head.
    #[test]
    fn a_rect_is_exactly_three_vec4s() {
        assert_eq!(std::mem::size_of::<Rect>(), 48);
        assert_eq!(std::mem::align_of::<Rect>(), 4);
    }

    /// One content box, so wrap width and clip rectangle can never disagree.
    #[test]
    fn insetting_shrinks_from_every_side() {
        let panel = Panel::new(10.0, 20.0, 100.0, 60.0);
        let content = panel.inset(8.0);
        assert_eq!(content, Panel::new(18.0, 28.0, 84.0, 44.0));
        assert!(content.w < panel.w && content.h < panel.h);
    }

    /// An inset larger than the panel must not produce a negative box, which
    /// would wrap text at a nonsense width and clip it inside out.
    #[test]
    fn insetting_past_the_panel_stays_positive() {
        let tiny = Panel::new(0.0, 0.0, 4.0, 4.0).inset(20.0);
        assert!(tiny.w >= 1.0 && tiny.h >= 1.0, "{tiny:?}");
    }

    #[test]
    fn splitting_conserves_the_panel() {
        let panel = Panel::new(0.0, 0.0, 200.0, 100.0);
        let (top, rest) = panel.split_top(30.0);
        assert_eq!(top.h + rest.h, panel.h);
        assert_eq!(rest.y, 30.0);
        let (rest, bottom) = panel.split_bottom(25.0);
        assert_eq!(rest.h + bottom.h, panel.h);
        assert_eq!(bottom.y, 75.0);
        let (left, right) = panel.split_left(80.0);
        assert_eq!(left.w + right.w, panel.w);
        assert_eq!(right.x, 80.0);
    }

    /// A split bigger than the panel takes the whole panel rather than
    /// producing a negative remainder that would draw off-screen.
    #[test]
    fn an_oversized_split_is_clamped() {
        let panel = Panel::new(0.0, 0.0, 50.0, 40.0);
        let (top, rest) = panel.split_top(999.0);
        assert_eq!(top.h, 40.0);
        assert_eq!(rest.h, 0.0);
    }

    #[test]
    fn hit_testing_is_half_open_so_adjacent_panels_do_not_both_claim_a_pixel() {
        let left = Panel::new(0.0, 0.0, 10.0, 10.0);
        let right = Panel::new(10.0, 0.0, 10.0, 10.0);
        assert!(left.contains(9.9, 5.0));
        assert!(!left.contains(10.0, 5.0));
        assert!(right.contains(10.0, 5.0));
    }

    /// The bug this was written for: a 22-pixel bar inset by a 9-pixel margin
    /// leaves a 4-pixel box, and a 20-pixel line drawn into it is clipped to
    /// nothing. A row is one line, centred, whatever the margin would have done.
    #[test]
    fn a_row_is_exactly_one_line_tall_and_centred() {
        let bar = Panel::new(6.0, 700.0, 400.0, 22.0);
        let line = Text::line_for(14.0);
        let row = bar.row(9.0, line);
        assert_eq!(row.h, line);
        assert_eq!(row.x, 15.0);
        assert_eq!(row.w, 382.0);
        assert!(row.y >= bar.y && row.y + row.h <= bar.y + bar.h, "{row:?}");
        // For comparison, the mistake it replaces.
        assert!(bar.inset(9.0).h < line, "the margin version cannot hold a line");
    }

    /// A bar shorter than one line gives up its padding rather than producing a
    /// box that clips, and never escapes the bar.
    #[test]
    fn a_row_in_a_bar_too_short_for_it_is_clamped() {
        let bar = Panel::new(0.0, 0.0, 100.0, 8.0);
        let row = bar.row(9.0, 20.0);
        assert_eq!(row.h, 8.0);
        assert!(row.y + row.h <= bar.y + bar.h);
        assert!(row.w >= 1.0);
    }

    #[test]
    fn rows_for_a_height_never_overflow_the_panel() {
        // Ten lines of 14pt at 20px each fit in 200px, not 201.
        assert_eq!(Text::rows_for(14.0, 200.0), 10);
        assert_eq!(Text::rows_for(14.0, 199.0), 9);
        assert_eq!(Text::rows_for(14.0, 0.0), 0);
    }

    /// Edges are hairlines on the panel, not outside it, so a border never
    /// paints over a neighbour.
    #[test]
    fn edges_stay_inside_the_panel() {
        let panel = Panel::new(5.0, 5.0, 20.0, 20.0);
        let edges = [
            panel.bottom_edge([0.0; 4]),
            panel.left_edge([0.0; 4]),
            panel.right_edge([0.0; 4]),
        ];
        for rect in edges {
            let [x, y, w, h] = rect.xywh;
            assert!(x >= panel.x && y >= panel.y, "{rect:?}");
            assert!(x + w <= panel.x + panel.w, "{rect:?}");
            assert!(y + h <= panel.y + panel.h, "{rect:?}");
        }
    }

    /// An outline is the panel itself, not a ring drawn around it: the stroke
    /// lies inside the edge, the way the four hairlines it replaces did, so a
    /// bordered panel still ends where the panel ends.
    #[test]
    fn an_outline_covers_the_panel_and_asks_for_a_stroke() {
        let panel = Panel::new(5.0, 5.0, 20.0, 20.0);
        let rect = panel.outline([1.0; 4], 1.0);
        assert_eq!(rect.xywh(), [5.0, 5.0, 20.0, 20.0]);
        assert_eq!(rect.extra()[3], 1.0);
        // And a fill is not a stroke, or every panel would be hollow.
        assert_eq!(panel.fill([1.0; 4]).extra()[3], 0.0);
    }

    /// The packing the shader reads back. Each builder owns one slot and
    /// leaves the others alone, so a chamfered stroke is both and not either.
    #[test]
    fn the_shape_builders_write_the_slots_the_shader_reads() {
        let plain = Rect::new(0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        assert_eq!(plain.extra(), [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(plain.radius(3.0).extra(), [3.0, 0.0, 0.0, 0.0]);
        assert_eq!(
            plain.chamfer(10.0, Rect::TOP_RIGHT).extra(),
            [0.0, 10.0, 2.0, 0.0]
        );
        assert_eq!(plain.stroke(1.0).extra(), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(
            plain
                .radius(2.0)
                .chamfer(10.0, Rect::TOP_LEFT | Rect::BOTTOM_RIGHT)
                .stroke(1.5)
                .extra(),
            [2.0, 10.0, 5.0, 1.5]
        );
        assert_eq!(
            plain.chamfer(4.0, Rect::EVERY_CORNER).extra()[2],
            15.0,
            "all four corner bits"
        );
    }

    /// The corner bits are read in the shader as `1u << i` against a fixed
    /// table of quadrant signs, so their values are part of the contract and
    /// cannot be renumbered on this side alone.
    #[test]
    fn the_corner_bits_are_the_ones_the_shader_indexes() {
        assert_eq!(Rect::TOP_LEFT, 1);
        assert_eq!(Rect::TOP_RIGHT, 2);
        assert_eq!(Rect::BOTTOM_RIGHT, 4);
        assert_eq!(Rect::BOTTOM_LEFT, 8);
        assert_eq!(
            Rect::TOP_LEFT | Rect::TOP_RIGHT | Rect::BOTTOM_RIGHT | Rect::BOTTOM_LEFT,
            Rect::EVERY_CORNER
        );
    }

    /// The two instance ranges have to cover every rectangle exactly once, with
    /// no gap and no overlap. An off by one here drops the last rectangle of the
    /// base layer or draws one of them twice, and both look like a shading
    /// mistake rather than an arithmetic one.
    #[test]
    fn the_instance_ranges_cover_every_rect_exactly_once() {
        let box_ = |n: usize| (0..n).map(|_| Rect::new(0.0, 0.0, 1.0, 1.0, [1.0; 4]));
        for (base_n, over_n) in [(0, 0), (1, 0), (0, 1), (1, 1), (7, 3), (256, 1), (3, 300)] {
            let mut scene = Scene::default();
            scene.rects.extend(box_(base_n));
            scene.over_rects.extend(box_(over_n));
            let (base, over) = scene.instances();
            assert_eq!(base.start, 0, "the base layer starts at the buffer's head");
            assert_eq!(base.end, over.start, "a gap or an overlap between the two");
            assert_eq!(base.len(), base_n, "{base_n}+{over_n}: base range");
            assert_eq!(over.len(), over_n, "{base_n}+{over_n}: overlay range");
            assert_eq!(
                over.end as usize,
                scene.rect_count(),
                "the ranges stop short of the last rectangle"
            );
            // Every instance index is named once and only once, which is the
            // property both assertions above are really about.
            let mut seen = vec![0u8; scene.rect_count()];
            for i in base.chain(over) {
                seen[i as usize] += 1;
            }
            assert!(
                seen.iter().all(|count| *count == 1),
                "{base_n}+{over_n}: {seen:?}"
            );
        }
    }

    /// The capacity the storage buffer is sized to counts both layers. Sized to
    /// the base layer alone, the overlay's rectangles would be written past the
    /// end of the buffer and dropped.
    #[test]
    fn the_rect_count_is_both_layers() {
        let mut scene = Scene::default();
        scene.rect(Rect::new(0.0, 0.0, 1.0, 1.0, [1.0; 4]));
        scene.over_rect(Rect::new(0.0, 0.0, 1.0, 1.0, [1.0; 4]));
        scene.over_rect(Rect::new(0.0, 0.0, 1.0, 1.0, [1.0; 4]));
        assert_eq!(scene.rect_count(), 3);
        assert_eq!(scene.rects.len(), 1, "the base layer keeps its own list");
        assert_eq!(scene.over_rects.len(), 2);
    }

    /// The overlay pushes go to the overlay lists and nowhere else. A push that
    /// landed in the base layer would put the thing back under every glyph in
    /// the window, which is the bug this layer exists for.
    #[test]
    fn an_overlay_push_stays_out_of_the_base_layer() {
        let mut scene = Scene::default();
        let at = Panel::new(0.0, 0.0, 10.0, 10.0);
        scene.over_rect(at.fill([1.0; 4]));
        scene.over_text(Text::new("menu", at, 13.0, [255; 4]));
        assert!(scene.rects.is_empty() && scene.texts.is_empty());
        assert_eq!(scene.over_rects.len(), 1);
        assert_eq!(scene.over_texts.len(), 1);
    }

    /// What the surface stores for one channel of a rectangle's fill: the
    /// shader writes what [`Rect::for_surface`] handed it, and an sRGB surface
    /// encodes that on the way into the texture.
    fn rect_on_screen(channel: u8, srgb: bool) -> u8 {
        let fill = [channel as f32 / 255.0, 0.0, 0.0, 1.0];
        let written = Rect::new(0.0, 0.0, 1.0, 1.0, fill).for_surface(srgb).rgba()[0];
        let stored = if srgb { linear_to_srgb(written) } else { written };
        (stored * 255.0).round() as u8
    }

    /// The same for a glyph, standing in for glyphon's own vertex shader: it
    /// converts the colour in `Accurate` and passes it through in `Web`, and
    /// the surface then does whatever it does to both alike.
    fn glyph_on_screen(channel: u8, srgb: bool) -> u8 {
        let colour = channel as f32 / 255.0;
        let written = match text_color_mode(srgb) {
            ColorMode::Accurate => srgb_to_linear(colour),
            ColorMode::Web => colour,
        };
        let stored = if srgb { linear_to_srgb(written) } else { written };
        (stored * 255.0).round() as u8
    }

    /// The rule the whole palette rests on, said without a GPU: a colour in the
    /// settings file is the colour on the screen. Every channel value, both
    /// paths, and both kinds of surface.
    #[test]
    fn a_colour_in_the_settings_file_is_the_colour_on_the_screen() {
        for srgb in [true, false] {
            for channel in 0..=255u8 {
                assert_eq!(
                    rect_on_screen(channel, srgb),
                    channel,
                    "a rectangle of {channel} on an srgb={srgb} surface"
                );
                assert_eq!(
                    glyph_on_screen(channel, srgb),
                    channel,
                    "a glyph of {channel} on an srgb={srgb} surface"
                );
            }
        }
    }

    /// And the two paths land on the same shade, which is the part that was
    /// wrong: glyphs were converted and rectangles were not, so text and a fill
    /// of one configured colour were two colours on the screen.
    #[test]
    fn a_rectangle_and_a_glyph_of_one_colour_are_one_shade() {
        for srgb in [true, false] {
            for channel in 0..=255u8 {
                assert_eq!(
                    rect_on_screen(channel, srgb),
                    glyph_on_screen(channel, srgb),
                    "{channel} on an srgb={srgb} surface"
                );
            }
        }
        // Both take their instruction from the one fact about the surface, so
        // there is nowhere for a future format change to move one and not the
        // other.
        assert_eq!(text_color_mode(true), ColorMode::Accurate);
        assert_eq!(text_color_mode(false), ColorMode::Web);
        let fill = [0.5, 0.25, 0.75, 0.6];
        let plain = Rect::new(0.0, 0.0, 1.0, 1.0, fill);
        assert_eq!(plain.for_surface(false).rgba(), fill, "nothing to undo");
        assert_ne!(plain.for_surface(true).rgba(), fill, "an encode to undo");
        assert_eq!(
            plain.for_surface(true).rgba()[3],
            fill[3],
            "alpha is a coverage, not a colour"
        );
    }

    /// The measurement this was found by, kept as the test. The bar is
    /// `#0e2e1e` in the settings file and the window showed `#427660`: that
    /// colour put through the sRGB encode one more time than it should have
    /// been, which is why a shaded window read as bright green.
    #[test]
    fn the_bar_is_the_shade_the_settings_file_asks_for() {
        let asked = [0x0eu8, 0x2e, 0x1e];
        let showed = [0x42u8, 0x76, 0x60];
        for (channel, was) in asked.into_iter().zip(showed) {
            let unconverted = (linear_to_srgb(channel as f32 / 255.0) * 255.0).round() as u8;
            assert_eq!(unconverted, was, "the shade the window used to show");
            assert_eq!(rect_on_screen(channel, true), channel, "and shows now");
        }
    }

    /// The two curves are each other's inverse across the whole range,
    /// including the straight piece at the bottom where the two formulas meet.
    #[test]
    fn the_two_conversions_undo_each_other() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert_eq!(srgb_to_linear(1.0), 1.0);
        assert_eq!(linear_to_srgb(0.0), 0.0);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
        for step in 0..=1000 {
            let colour = step as f32 / 1000.0;
            let round = linear_to_srgb(srgb_to_linear(colour));
            assert!((round - colour).abs() < 1e-5, "{colour} came back {round}");
        }
        // Dark is where the mistake was loudest: near black, the encode lifts a
        // colour by a factor of four.
        assert!(srgb_to_linear(0.05) < 0.005);
    }

    /// Width of one monospace column at this size, measured by shaping the way
    /// [`Renderer::column_width`] does rather than guessed from the size.
    fn column_of(fonts: &mut FontSystem, size: f32) -> f32 {
        let mut ruler = Buffer::new(fonts, Metrics::new(size, Text::line_for(size)));
        ruler.set_size(Some(4096.0), Some(size * 2.0));
        ruler.set_text(
            "0000000000",
            &Attrs::new().family(Family::Monospace),
            Shaping::Basic,
            None,
        );
        ruler.shape_until_scroll(fonts, false);
        let column = ruler.layout_runs().next().map_or(0.0, |run| run.line_w) / 10.0;
        assert!(column > 0.0, "no monospace font to measure a column with");
        column
    }

    /// The characters each visual row really ends up holding, laid out the way
    /// [`Renderer::shape`] lays a box out: `wrap` is what a box with no column
    /// count gets, and `named` is the rule a box that names its columns is
    /// broken by before the shaper sees it.
    fn rows_on_screen(
        text: &str,
        cols: usize,
        named: Option<text_geometry::Break>,
        wrap: Wrap,
    ) -> Vec<String> {
        let size = 14.0;
        let mut fonts = icon_fonts();
        let column = column_of(&mut fonts, size);
        let laid = match named {
            Some(at) => Run::wrapped(&[Run::plain(text)], cols, at).swap_remove(0).text,
            None => text.to_string(),
        };
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(size, Text::line_for(size)));
        // Half a column of slack, so `cols` glyphs fit and `cols + 1` do not
        // whichever way the float rounds.
        buffer.set_size(Some(cols as f32 * column + column * 0.5), Some(4096.0));
        buffer.set_wrap(if named.is_some() { Wrap::None } else { wrap });
        buffer.set_text(
            &laid,
            &Attrs::new().family(Family::Monospace),
            Shaping::Basic,
            None,
        );
        buffer.shape_until_scroll(&mut fonts, false);
        buffer
            .layout_runs()
            .map(|run| {
                // Glyph offsets are into the buffer line the run belongs to,
                // which is one hard-wrapped row here and one whole paragraph
                // when the shaper is doing the wrapping.
                run.glyphs
                    .iter()
                    .flat_map(|g| run.text[g.start..g.end].chars())
                    .collect()
            })
            .collect()
    }

    /// A box that names its columns is drawn in the rows `text-geometry` says
    /// it has, character for character.
    ///
    /// This used to assert that a named box always broke on the column, blank
    /// or not, which is how the rows were made to match the arithmetic in the
    /// first place: it was exact, and it broke prose in the middle of words.
    /// Now the arithmetic breaks at the blank as well, so the assertion is that
    /// the rows on screen are the rows the layer named, whichever rule the box
    /// asked for.
    ///
    /// Neither wrap mode the shaper offers can be trusted with this on its own.
    /// Word wrap drops the blank at the break off the screen entirely, and
    /// character wrap lets a blank sitting on the boundary hang over the edge,
    /// so the row below starts one character further along than any count of it
    /// says. One swallowed blank per break is the space that turned up in a
    /// copied selection with nothing on screen to explain it.
    #[test]
    fn a_box_that_names_its_columns_is_drawn_in_the_rows_it_was_counted_in() {
        let cols = 20;
        // The blank at index 20 is the one both shaper modes swallow.
        let prose = "hello worldly people everywhere now";
        assert_eq!(prose.chars().count(), 35);
        assert_eq!(prose.chars().nth(20), Some(' '));
        let chars: Vec<char> = prose.chars().collect();

        for at in [text_geometry::Break::Word, text_geometry::Break::Column] {
            let on_screen = rows_on_screen(prose, cols, Some(at), Wrap::None);
            let counted = text_geometry::rows_in(prose, cols, at);
            assert_eq!(on_screen.len(), counted.len(), "{at:?} drew a different number of rows");
            for (row, span) in on_screen.iter().zip(counted) {
                assert_eq!(
                    row.chars().collect::<Vec<char>>(),
                    chars[span.start..span.end],
                    "{at:?}: {row:?} is not the characters {span:?} names"
                );
            }
        }
        assert_eq!(
            rows_on_screen(prose, cols, Some(text_geometry::Break::Word), Wrap::None),
            vec!["hello worldly people", "everywhere now"],
            "the blank at the break is on neither row"
        );
        assert_eq!(
            rows_on_screen(prose, cols, Some(text_geometry::Break::Column), Wrap::None),
            vec!["hello worldly people", " everywhere now"],
            "a box breaking on the column keeps every character it was given"
        );

        // What the two shaper modes do with the same box, left to themselves.
        let glyph = rows_on_screen(prose, cols, None, Wrap::Glyph);
        assert_eq!(
            glyph,
            vec!["hello worldly people ", "everywhere now"],
            "character wrap lets the blank on the boundary hang over"
        );
        let word = rows_on_screen(prose, cols, None, Wrap::WordOrGlyph);
        assert_eq!(word, vec!["hello worldly people", "everywhere now"]);
    }

    /// Blanks at the very start of a box keep their columns.
    ///
    /// The window's prompt spends its first two columns on a marker, and while a
    /// turn runs that marker is two blanks with three animated rectangles drawn
    /// over them. Everything else in that row (the caret, the selection band, the
    /// click inverse) adds those two columns as arithmetic, so if the shaper
    /// swallowed the blanks the text would sit two columns left of where the
    /// caret is put. It does not: leading blanks are shaped like any other glyph.
    #[test]
    fn blanks_at_the_start_of_a_box_hold_their_columns() {
        let size = 14.0;
        let mut fonts = icon_fonts();
        let column = column_of(&mut fonts, size);
        let start_of = |fonts: &mut FontSystem, text: &str| -> f32 {
            let mut buffer = Buffer::new(fonts, Metrics::new(size, Text::line_for(size)));
            buffer.set_size(Some(40.0 * column), Some(4096.0));
            buffer.set_wrap(Wrap::None);
            buffer.set_text(
                text,
                &Attrs::new().family(Family::Monospace),
                Shaping::Basic,
                None,
            );
            buffer.shape_until_scroll(fonts, false);
            let run = buffer.layout_runs().next().expect("a row");
            run.glyphs
                .iter()
                .find(|glyph| run.text[glyph.start..glyph.end].starts_with('h'))
                .expect("the text is in the row")
                .x
        };
        let bare = start_of(&mut fonts, "hello");
        let marked = start_of(&mut fonts, "  hello");
        assert!(
            (marked - bare - 2.0 * column).abs() < 0.5,
            "two blanks moved the text {} pixels, not {}",
            marked - bare,
            2.0 * column
        );
    }

    /// The break is put in front of a character that exists, so a line of
    /// exactly `cols` characters is one row and not one row plus an empty one,
    /// and a newline that is already there is not doubled. That is what keeps
    /// the drawn row count equal to the row count the box was measured by.
    #[test]
    fn a_hard_break_is_only_inserted_where_a_row_really_overflows() {
        let wrap = |text: &str, cols: usize| {
            Run::wrapped(&[Run::plain(text)], cols, text_geometry::Break::Column)
                .swap_remove(0)
                .text
        };
        assert_eq!(wrap("abcde", 5), "abcde", "exactly full is one row");
        assert_eq!(wrap("abcdef", 5), "abcde
f");
        assert_eq!(wrap("abcde
fg", 5), "abcde
fg", "no doubled break");
        assert_eq!(wrap("", 5), "");
        assert_eq!(wrap("

", 5), "

", "empty lines stay one row each");
        assert_eq!(wrap("abcdefghijkl", 5), "abcde
fghij
kl");
        assert_eq!(wrap("abcdef", 0), "abcdef", "a box with no columns is left alone");
    }

    /// A box with chrome in front of every line keeps that chrome out of the
    /// wrap and puts the rows a line continues onto under its text.
    ///
    /// The file view's gutter. Wrapped along with the text it gave the first
    /// row of a line four characters fewer than the rows under it, so the row
    /// count, the caret and the band all disagreed with the screen from the
    /// second row down.
    #[test]
    fn a_box_with_chrome_in_front_of_its_lines_indents_what_wraps_under_it() {
        let cols = 10;
        let gutter = 4;
        let line = "aaa bbb ccc ddd eee";
        let runs = vec![
            Run::tinted("012 ", [1, 2, 3, 4]),
            Run::plain(line),
            Run::plain("\n"),
            Run::tinted("013 ", [1, 2, 3, 4]),
            Run::plain("short"),
        ];
        let out = Run::wrapped_under(&runs, cols, text_geometry::Break::Word, gutter);
        let laid: String = out.iter().map(|r| r.text.as_str()).collect();
        let rows: Vec<&str> = laid.split('\n').collect();

        // The number is written once, and every row under it starts where the
        // text of the first row started.
        assert_eq!(
            rows,
            vec!["012 aaa bbb", "    ccc ddd", "    eee", "013 short"],
            "{laid:?}"
        );
        // Which is the same as saying: every row holds the characters the
        // window counted that line in, in the columns it counted them in.
        let counted = text_geometry::rows_in(line, cols, text_geometry::Break::Word);
        assert_eq!(counted.len(), 3);
        let chars: Vec<char> = line.chars().collect();
        for (row, span) in rows.iter().zip(&counted) {
            let text: String = row.chars().skip(gutter).collect();
            assert_eq!(text.chars().collect::<Vec<char>>(), chars[span.start..span.end]);
            assert!(text.chars().count() <= cols, "{text:?} is wider than the box");
        }
        assert_eq!(
            out.iter().map(|r| r.color).collect::<Vec<_>>(),
            vec![Some([1, 2, 3, 4]), None, None, Some([1, 2, 3, 4]), None],
            "the colors ride along unchanged"
        );
        // And with no chrome to keep, it is the plain wrap it always was.
        let plain: Vec<String> = Run::wrapped(&runs, cols, text_geometry::Break::Word)
            .iter()
            .map(|run| run.text.clone())
            .collect();
        let none: Vec<String> = Run::wrapped_under(&runs, cols, text_geometry::Break::Word, 0)
            .iter()
            .map(|run| run.text.clone())
            .collect();
        assert_eq!(none, plain);
    }

    /// The count runs across the runs, not within each one: a syntax-colored
    /// row is a dozen runs and it is still one row. Counting per run would put
    /// a break after every token.
    #[test]
    fn colored_runs_are_wrapped_as_one_line_not_one_each() {
        let runs = vec![
            Run::plain("abc"),
            Run::tinted("def", [1, 2, 3, 4]),
            Run::plain("ghi"),
        ];
        let out = Run::wrapped(&runs, 4, text_geometry::Break::Column);
        let text: String = out.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(text, "abcd
efgh
i");
        assert_eq!(
            out.iter().map(|r| r.color).collect::<Vec<_>>(),
            vec![None, Some([1, 2, 3, 4]), None],
            "the colors ride along unchanged"
        );
    }

    /// A negative size would flip the sign of the distance field and paint the
    /// whole quad, which looks like a solid block over the window.
    #[test]
    fn a_negative_shape_parameter_is_refused() {
        let plain = Rect::new(0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        assert_eq!(plain.chamfer(-4.0, Rect::TOP_RIGHT).extra()[1], 0.0);
        assert_eq!(plain.stroke(-1.0).extra()[3], 0.0);
    }
}
