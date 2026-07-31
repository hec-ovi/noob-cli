# noob-gpu

contractVersion: 1.0.0

## Purpose

Bring up the GPU for one window: pick the adapter, configure the surface,
probe what this machine supports (transparency included), and own frame
acquisition and presentation.

This is a typed Rust surface, so the signatures below are the schema. The
crate's unit tests enforce the selection policy at the test boundary; callers
get the types checked by the compiler, not per call.

## Types

| Type | Fields | Notes |
|---|---|---|
| `Gpu` | `device: wgpu::Device`, `queue: wgpu::Queue`, `surface: wgpu::Surface<'static>`, `config: wgpu::SurfaceConfiguration`, `caps: Capabilities` | One per window. All fields public: the draw layer borrows `device` and `queue` through it. |
| `Frame` | `texture: wgpu::SurfaceTexture`, `view: wgpu::TextureView` | A surface texture acquired and not yet presented. The draw layer renders into `view`; this layer owns acquisition and presentation. |
| `Capabilities` | `adapter`, `backend`, `driver`, `format`, `chosen_alpha`, `chosen_present`: `String`; `alpha_modes`, `present_modes`: `Vec<String>`; `transparent: bool` | `Clone + Debug`. Strings on purpose: a status pane renders the report without depending on wgpu's types. |

## Operations

| Call | In | Out | Behavior |
|---|---|---|---|
| `Gpu::new(window).await` | `Arc<winit::window::Window>` | `Result<Gpu, String>` | Requests a high-performance adapter compatible with the window's surface, a device with empty features and default limits, then configures the surface: render-attachment usage, chosen format, alpha and present mode, frame latency 2, size clamped to at least 1x1. |
| `gpu.width()` / `gpu.height()` | none | `f32` | The configured surface size, as floats for layout math. |
| `gpu.resize(width, height)` | `u32, u32` | none | Reconfigures the surface to the new size. A zero dimension (minimized) or an unchanged size is a no-op. |
| `gpu.acquire()` | none | `Option<Frame>` | The next surface texture, suboptimal accepted. `None` means skip this frame: an outdated or lost surface is reconfigured on the way out, and the next event brings another frame. |
| `gpu.present(frame)` | `Frame` | none | Drops the view and presents the texture through the queue. Consumes the frame. |
| `caps.report()` | none | `Vec<String>` | One line per fact (adapter, driver, format, alpha, present, window), for the status pane and for a bug report. When alpha was refused the window line says "opaque backdrop (compositor refused alpha)". |

## Selection policy

Chosen at `Gpu::new`, recorded in `Capabilities`, fixed for the Gpu's lifetime:

- Alpha: `PreMultiplied`, then `PostMultiplied`, then `Inherit` (that order is
  what the rect shader and glyphon produce), else the first mode the surface
  offers, else `Auto` on an empty list.
- Format: the first sRGB format the surface offers, else its first format.
  `noob-draw` asks the chosen format whether it is sRGB and converts colours
  on the answer.
- Present: `Mailbox` when offered (a burst of agent output must not queue
  frames behind the compositor), else `Fifo`, which is always present.

## Events

None.

## Errors

All from `Gpu::new`, all `String`, closed set of three:

| Error | Meaning |
|---|---|
| `cannot create a surface for the window: ...` | The instance could not make a surface for this window. |
| `no usable GPU adapter: ...` | No adapter at all; the machine cannot run this front end. |
| `the adapter refused a device: ...` | Adapter found but device creation failed. |

Nothing else fails. Every other unsupported capability degrades to a recorded
fallback in `Capabilities`, because a front end that refuses to start tells
the user nothing. `resize` and `acquire` handle their bad cases (zero size,
lost surface) as no-op and `None`.

## Invariants

1. `Capabilities.transparent` is true exactly when the chosen alpha mode
   composites against what is behind the window (`PreMultiplied`,
   `PostMultiplied` or `Inherit`). False means the caller must draw its own
   opaque backdrop. Transparency is probed per surface, never assumed:
   the same binary reports differently per platform, compositor and driver.
2. The `Gpu` is the owner of record for the device, queue, surface and
   config, one per window, for the window's lifetime. Callers borrow through
   the public fields; nobody else creates wgpu devices or surfaces.
3. Resize is legal at any time no `Frame` is outstanding: call it on the
   window's resize event, before acquiring. Zero-sized and unchanged calls
   are safe no-ops, so the caller forwards every resize event unfiltered.
4. A frame lives exactly from `acquire` to `present`, within one event.
   `None` from `acquire` is not an error: skip the frame and wait for the
   next event.
5. This crate knows nothing about rectangles, glyphs, panes or the agent.
   Anything that draws lives above it.

## Dependencies

No contracts. Two crates, pinned by the gui workspace: `wgpu` 30 and
`winit` 0.30.

## How to modify this blackbox safely

The selection helpers (`pick_alpha`, `pick_format`, `pick_present`,
`composites`) are private, but their policy is the contract: the unit tests
in `src/lib.rs` name each rule and run without a GPU. Changing the alpha
preference order or the sRGB rule changes what `noob-draw` renders against,
so that is a breaking change even though no signature moves. Adding a
capability field or an operation is additive: minor `contractVersion` bump.
