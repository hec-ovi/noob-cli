//! G0: the device, the surface, and what this machine will actually do.
//!
//! Everything above this layer draws. This one decides what drawing is
//! possible: which adapter, which surface format, whether the window can be
//! transparent, and how presentation is paced. It knows nothing about
//! rectangles, glyphs, panes or the agent.
//!
//! ## Transparency is probed, never assumed
//!
//! `CompositeAlphaMode` support varies by platform, compositor and driver, and
//! is reported per surface. On this machine (RADV, Wayland) the surface offers
//! `Opaque` and `PreMultiplied`; another machine may offer neither useful one.
//! So the mode is chosen from what the surface reports and the result is
//! recorded in [`Capabilities::transparent`]. When transparency is unavailable
//! the caller draws an opaque themed backdrop, which looks deliberate. What it
//! must never do is assume alpha worked and render a window that reads as
//! broken.
//!
//! ## Presentation is paced by what is happening
//!
//! Idle means block until an event arrives. The spike this replaces rendered
//! static text at 3,500 fps and spent a third of the graphics pipe doing it.
//! Mailbox is preferred when available so a burst of agent output never queues
//! frames behind the compositor; Fifo is the fallback and is always present.

use std::sync::Arc;

use winit::window::Window;

/// What the adapter and surface turned out to support, and what was chosen.
///
/// Carried as strings for the parts that are only ever displayed, so a status
/// pane can render the report without depending on wgpu's types.
#[derive(Clone, Debug)]
pub struct Capabilities {
    pub adapter: String,
    pub backend: String,
    pub driver: String,
    pub format: String,
    pub alpha_modes: Vec<String>,
    pub present_modes: Vec<String>,
    pub chosen_alpha: String,
    pub chosen_present: String,
    /// Whether the chosen alpha mode actually composites against what is
    /// behind the window. False means the caller must draw its own backdrop.
    pub transparent: bool,
}

impl Capabilities {
    /// One line per fact, for the status pane and for a bug report.
    pub fn report(&self) -> Vec<String> {
        vec![
            format!("adapter  {} ({})", self.adapter, self.backend),
            format!("driver   {}", self.driver),
            format!("format   {}", self.format),
            format!(
                "alpha    {} of {}",
                self.chosen_alpha,
                self.alpha_modes.join(", ")
            ),
            format!(
                "present  {} of {}",
                self.chosen_present,
                self.present_modes.join(", ")
            ),
            format!(
                "window   {}",
                if self.transparent {
                    "transparent"
                } else {
                    "opaque backdrop (compositor refused alpha)"
                }
            ),
        ]
    }
}

/// The device, the surface and its configuration. One per window.
pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub config: wgpu::SurfaceConfiguration,
    pub caps: Capabilities,
}

/// A surface texture that has been acquired and not yet presented.
///
/// Returned rather than rendered into here so the draw layer owns the render
/// pass and this layer owns acquisition and presentation, which is the only
/// part that has to know about surface loss.
pub struct Frame {
    pub texture: wgpu::SurfaceTexture,
    pub view: wgpu::TextureView,
}

impl Gpu {
    /// Bring up a device for this window.
    ///
    /// Fails only when there is no adapter at all; every other unsupported
    /// capability degrades to a recorded fallback rather than an error, because
    /// a front end that refuses to start tells the user nothing.
    pub async fn new(window: Arc<Window>) -> Result<Gpu, String> {
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("cannot create a surface for the window: {e}"))?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|e| format!("no usable GPU adapter: {e}"))?;
        let info = adapter.get_info();
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("no0b"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| format!("the adapter refused a device: {e}"))?;

        let surface_caps = surface.get_capabilities(&adapter);
        let alpha_mode = pick_alpha(&surface_caps.alpha_modes);
        let present_mode = pick_present(&surface_caps.present_modes);
        let format = pick_format(&surface_caps.formats);

        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let caps = Capabilities {
            adapter: info.name.clone(),
            backend: format!("{:?}", info.backend),
            driver: format!("{} {}", info.driver, info.driver_info)
                .trim()
                .to_string(),
            format: format!("{format:?}"),
            alpha_modes: surface_caps
                .alpha_modes
                .iter()
                .map(|m| format!("{m:?}"))
                .collect(),
            present_modes: surface_caps
                .present_modes
                .iter()
                .map(|m| format!("{m:?}"))
                .collect(),
            chosen_alpha: format!("{alpha_mode:?}"),
            chosen_present: format!("{present_mode:?}"),
            transparent: composites(alpha_mode),
        };

        Ok(Gpu {
            device,
            queue,
            surface,
            config,
            caps,
        })
    }

    pub fn width(&self) -> f32 {
        self.config.width as f32
    }

    pub fn height(&self) -> f32 {
        self.config.height as f32
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return; // minimized; configuring a zero-sized surface is invalid
        }
        if width == self.config.width && height == self.config.height {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
    }

    /// Acquire the next surface texture, reconfiguring on the way if the
    /// surface went stale. `None` means skip this frame; the next event will
    /// bring another.
    pub fn acquire(&mut self) -> Option<Frame> {
        use wgpu::CurrentSurfaceTexture as Current;
        let texture = match self.surface.get_current_texture() {
            Current::Success(t) | Current::Suboptimal(t) => t,
            Current::Outdated | Current::Lost => {
                self.surface.configure(&self.device, &self.config);
                return None;
            }
            _ => return None,
        };
        let view = texture.texture.create_view(&Default::default());
        Some(Frame { texture, view })
    }

    pub fn present(&self, frame: Frame) {
        let Frame { texture, view } = frame;
        drop(view);
        self.queue.present(texture);
    }
}

/// Prefer a mode that respects alpha; fall back to whatever exists.
///
/// `PreMultiplied` first because that is what the rect shader and glyphon both
/// produce. `Inherit` means the compositor decides, which is usually right on
/// the platforms that offer it.
fn pick_alpha(modes: &[wgpu::CompositeAlphaMode]) -> wgpu::CompositeAlphaMode {
    for wanted in [
        wgpu::CompositeAlphaMode::PreMultiplied,
        wgpu::CompositeAlphaMode::PostMultiplied,
        wgpu::CompositeAlphaMode::Inherit,
    ] {
        if modes.contains(&wanted) {
            return wanted;
        }
    }
    modes.first().copied().unwrap_or(wgpu::CompositeAlphaMode::Auto)
}

/// An sRGB surface when the surface offers one, whatever it offers first
/// otherwise.
///
/// An sRGB format encodes what a shader writes on the way into the texture,
/// which is the correct place for that to happen and is what makes blending
/// physically right. It is not free, though: a colour that goes to the shader
/// unconverted comes out of that encode lighter than it was asked for, and the
/// window shipped that way. The rule and the conversion both live in
/// `noob-draw`, which asks this format whether it is an sRGB one.
fn pick_format(formats: &[wgpu::TextureFormat]) -> wgpu::TextureFormat {
    formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(formats[0])
}

/// Mailbox when it exists: a burst of agent output must not queue frames behind
/// the compositor. Fifo is guaranteed present and is the honest fallback.
fn pick_present(modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    if modes.contains(&wgpu::PresentMode::Mailbox) {
        wgpu::PresentMode::Mailbox
    } else {
        wgpu::PresentMode::Fifo
    }
}

fn composites(mode: wgpu::CompositeAlphaMode) -> bool {
    matches!(
        mode,
        wgpu::CompositeAlphaMode::PreMultiplied
            | wgpu::CompositeAlphaMode::PostMultiplied
            | wgpu::CompositeAlphaMode::Inherit
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::CompositeAlphaMode as Alpha;
    use wgpu::PresentMode as Present;

    /// The mode this machine reports, and the one the shader is written for.
    #[test]
    fn premultiplied_wins_when_the_surface_offers_it() {
        assert_eq!(
            pick_alpha(&[Alpha::Opaque, Alpha::PreMultiplied]),
            Alpha::PreMultiplied
        );
    }

    /// A surface that will not composite must not be reported as transparent,
    /// because the caller draws a different backdrop when it is not.
    #[test]
    fn an_opaque_only_surface_is_reported_as_opaque() {
        let chosen = pick_alpha(&[Alpha::Opaque]);
        assert_eq!(chosen, Alpha::Opaque);
        assert!(!composites(chosen));
    }

    #[test]
    fn every_alpha_mode_that_composites_is_reported_as_transparent() {
        for mode in [Alpha::PreMultiplied, Alpha::PostMultiplied, Alpha::Inherit] {
            assert!(composites(mode), "{mode:?}");
        }
        assert!(!composites(Alpha::Opaque));
    }

    /// A surface reporting nothing at all must still yield a usable value
    /// rather than panicking on an empty slice.
    #[test]
    fn an_empty_capability_list_does_not_panic() {
        assert_eq!(pick_alpha(&[]), Alpha::Auto);
        assert_eq!(pick_present(&[]), Present::Fifo);
    }

    /// Which format is chosen decides whether a colour is encoded on its way
    /// into the texture, and `noob-draw` converts or does not convert on the
    /// answer. A surface with both kinds must land on the sRGB one, because
    /// that is the case the draw layer's conversion is written against and the
    /// case every machine this has run on reports.
    #[test]
    fn an_srgb_surface_is_chosen_when_the_surface_offers_one() {
        use wgpu::TextureFormat as Format;
        let chosen = pick_format(&[Format::Bgra8Unorm, Format::Bgra8UnormSrgb]);
        assert_eq!(chosen, Format::Bgra8UnormSrgb);
        assert!(chosen.is_srgb());
        // And a surface with no sRGB format at all still gets a usable one,
        // which the draw layer then writes to without converting.
        let plain = pick_format(&[Format::Bgra8Unorm, Format::Rgba8Unorm]);
        assert_eq!(plain, Format::Bgra8Unorm);
        assert!(!plain.is_srgb());
    }

    #[test]
    fn mailbox_is_preferred_and_fifo_is_the_fallback() {
        assert_eq!(
            pick_present(&[Present::Fifo, Present::Mailbox]),
            Present::Mailbox
        );
        assert_eq!(pick_present(&[Present::Fifo]), Present::Fifo);
        assert_eq!(pick_present(&[Present::Immediate]), Present::Fifo);
    }

    /// The report is what a user pastes into a bug report, so it must say what
    /// happened rather than what was wanted.
    #[test]
    fn the_report_names_the_fallback_when_alpha_was_refused() {
        let caps = Capabilities {
            adapter: "Test".into(),
            backend: "Vulkan".into(),
            driver: "radv".into(),
            format: "Bgra8UnormSrgb".into(),
            alpha_modes: vec!["Opaque".into()],
            present_modes: vec!["Fifo".into()],
            chosen_alpha: "Opaque".into(),
            chosen_present: "Fifo".into(),
            transparent: false,
        };
        let report = caps.report().join("\n");
        assert!(report.contains("compositor refused alpha"), "{report}");
    }
}
