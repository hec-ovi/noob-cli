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

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Wrap,
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
    /// Wrap at whatever character reaches the edge rather than at a word
    /// boundary. What a text field needs: a caret placed by counting columns
    /// only lands where the glyph is if the wrap counts columns too.
    pub wrap_anywhere: bool,
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
            wrap_anywhere: false,
        }
    }

    pub fn wrap_anywhere(mut self) -> Text {
        self.wrap_anywhere = true;
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
        let mut atlas = TextAtlas::new(device, &gpu.queue, &cache, format);
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
        if !scene.rects.is_empty() {
            gpu.queue
                .write_buffer(&self.storage, 0, bytemuck::cast_slice(&scene.rects));
        }
        if !scene.over_rects.is_empty() {
            // Directly after the base layer in the same buffer, which is what
            // makes the second instance range a range and not a second binding.
            let after = (scene.rects.len() * std::mem::size_of::<Rect>()) as u64;
            gpu.queue
                .write_buffer(&self.storage, after, bytemuck::cast_slice(&scene.over_rects));
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
                if item.wrap_anywhere {
                    buffer.set_wrap(Wrap::Glyph);
                }
                match item.runs.as_slice() {
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

    /// A negative size would flip the sign of the distance field and paint the
    /// whole quad, which looks like a solid block over the window.
    #[test]
    fn a_negative_shape_parameter_is_refused() {
        let plain = Rect::new(0.0, 0.0, 10.0, 10.0, [1.0; 4]);
        assert_eq!(plain.chamfer(-4.0, Rect::TOP_RIGHT).extra()[1], 0.0);
        assert_eq!(plain.stroke(-1.0).extra()[3], 0.0);
    }
}
