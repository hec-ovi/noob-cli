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
//! **A struct shared with a shader is all `vec4`-sized members.** WGSL aligns a
//! `vec3` to 16 bytes and Rust does not. A `{[f32;4], [f32;4], f32, [f32;3]}`
//! is 48 bytes here and 64 there, which silently corrupted every rectangle
//! after the first. [`Rect`] is three `[f32; 4]`s and the shader agrees.

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};

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

    /// A hairline along the top edge, which is how a pane reads as a pane
    /// without spending four rectangles on a border.
    pub fn top_edge(self, rgba: [f32; 4]) -> Rect {
        Rect::new(self.x, self.y, self.w, 1.0, rgba)
    }

    pub fn bottom_edge(self, rgba: [f32; 4]) -> Rect {
        Rect::new(self.x, self.y + self.h - 1.0, self.w, 1.0, rgba)
    }

    pub fn left_edge(self, rgba: [f32; 4]) -> Rect {
        Rect::new(self.x, self.y, 1.0, self.h, rgba)
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
/// Square by construction. Corners stay in `extra` so a rounded variant can be
/// added later without the struct changing size, which would mean the shader
/// changing with it.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Rect {
    xywh: [f32; 4],
    rgba: [f32; 4],
    /// x is the corner radius in pixels; the rest is reserved. A `vec4` on both
    /// sides, never a bare trailing scalar. See the module docs.
    extra: [f32; 4],
}

impl Rect {
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

    /// Position and size, so a caller can assert a scene without a GPU.
    pub fn xywh(&self) -> [f32; 4] {
        self.xywh
    }
}

/// A stretch of characters that share a color. `None` means the run takes the
/// text's default, which is the common case and costs no per-run state.
#[derive(Clone, Debug)]
pub struct Run {
    pub text: String,
    pub color: Option<[u8; 4]>,
}

impl Run {
    pub fn plain(text: impl Into<String>) -> Run {
        Run {
            text: text.into(),
            color: None,
        }
    }

    pub fn tinted(text: impl Into<String>, color: [u8; 4]) -> Run {
        Run {
            text: text.into(),
            color: Some(color),
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
        }
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

/// Everything to draw this frame, in painter's order.
#[derive(Default)]
pub struct Scene {
    pub rects: Vec<Rect>,
    pub texts: Vec<Text>,
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
    @location(3) radius: f32,
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
    out.radius = r.extra.x;
    return out;
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    // Signed distance to the rectangle; radius 0 is an ordinary square corner.
    let q = abs(in.local) - (in.half_size - vec2<f32>(in.radius, in.radius));
    let d = length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - in.radius;
    let a = in.rgba.a * (1.0 - smoothstep(-1.0, 1.0, d));
    // Premultiplied: the surface composites against the desktop behind it.
    return vec4<f32>(in.rgba.rgb * a, a);
}
"#;

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

        Renderer {
            pipeline,
            uniform,
            storage,
            bind_layout,
            bind_group,
            capacity,
            font_system: FontSystem::new(),
            swash: SwashCache::new(),
            atlas,
            viewport,
            text,
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
    pub fn draw(&mut self, gpu: &mut noob_gpu::Gpu, scene: &Scene, frame: noob_gpu::Frame) {
        let (w, h) = (gpu.width(), gpu.height());
        self.ensure_capacity(&gpu.device, scene.rects.len());
        gpu.queue
            .write_buffer(&self.uniform, 0, bytemuck::cast_slice(&[w, h, 0.0f32, 0.0]));
        if !scene.rects.is_empty() {
            gpu.queue
                .write_buffer(&self.storage, 0, bytemuck::cast_slice(&scene.rects));
        }

        // Buffers must outlive `prepare`, which borrows them through TextArea.
        let mono = Attrs::new().family(Family::Monospace);
        let buffers: Vec<Buffer> = scene
            .texts
            .iter()
            .map(|item| {
                let mut buffer = Buffer::new(
                    &mut self.font_system,
                    Metrics::new(item.size, item.line_height),
                );
                // Wrap width and clip rectangle from the same content box.
                buffer.set_size(Some(item.at.w), Some(item.at.h));
                match item.runs.as_slice() {
                    [only] if only.color.is_none() => {
                        buffer.set_text(&only.text, &mono, Shaping::Basic, None);
                    }
                    runs => {
                        let spans = runs.iter().map(|run| {
                            let attrs = Attrs::new().family(Family::Monospace);
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
            .collect();

        self.viewport.update(
            &gpu.queue,
            Resolution {
                width: gpu.config.width,
                height: gpu.config.height,
            },
        );
        let areas: Vec<TextArea> = scene
            .texts
            .iter()
            .zip(&buffers)
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
        if let Err(e) = self.text.prepare(
            &gpu.device,
            &gpu.queue,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            areas,
            &mut self.swash,
        ) {
            // A frame with missing text is better than a dead window; the atlas
            // recovers on the next frame once it has been trimmed.
            eprintln!("clippy: text prepare failed: {e:?}");
        }

        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clippy"),
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
            if !scene.rects.is_empty() {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.draw(0..6, 0..scene.rects.len() as u32);
            }
            let _ = self.text.render(&self.atlas, &self.viewport, &mut pass);
        }
        gpu.queue.submit([encoder.finish()]);
        gpu.present(frame);
        self.atlas.trim();
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
        for rect in [
            panel.top_edge([0.0; 4]),
            panel.bottom_edge([0.0; 4]),
            panel.left_edge([0.0; 4]),
        ] {
            let [x, y, w, h] = rect.xywh;
            assert!(x >= panel.x && y >= panel.y, "{rect:?}");
            assert!(x + w <= panel.x + panel.w, "{rect:?}");
            assert!(y + h <= panel.y + panel.h, "{rect:?}");
        }
    }
}
