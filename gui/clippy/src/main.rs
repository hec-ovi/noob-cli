//! CLIppy: a window for watching an agent work.
//!
//! It starts `noob serve` in a workspace, sends what you type as a
//! `prompt.submit` and renders the frames that come back. There is no terminal
//! involved and no web anything: one GPU surface, no system window chrome, and
//! panes that separate the model's prose from its calls, its plan, its
//! sub-agents and the files it is changing.
//!
//! ## Redraw only on change
//!
//! The event loop blocks until something happens. Frames arriving from the
//! agent wake it through the event loop's proxy, so a window showing static
//! text costs nothing. The spike this grew out of rendered the same pixels at
//! 3,500 fps and spent a third of a GPU doing it, which is how a UI silently
//! eats a machine.
//!
//! Usage: `clippy [workspace]`, with `NOOB_BIN` naming the agent binary when it
//! is not `noob` on PATH. Settings live beside noob's own; see `config`.

mod config;
mod dock;
mod link;
mod markdown;
mod monitor;
mod skin;
mod state;
mod syntax;
mod view;

use std::sync::Arc;
use std::time::{Duration, Instant};

use noob_proto::Command as Cmd;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

use config::Config;
use dock::{Dock, Space, View};
use link::{Incoming, Link};
use monitor::Monitor;
use skin::Skin;
use state::{State, Tone};
use view::{Drag, Hit, Layout, Shape};

/// The only user event: something arrived, come and look. Deliberately carries
/// no payload; the channel holds the frames and the loop drains it.
struct Wake;

/// A second click inside this long, on the same thing, is a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// How often the monitor reads the kernel, and only while it is on screen. A
/// monitor is inherently periodic, which is the opposite of the
/// redraw-on-change rule; the rule is kept by not sampling when nobody looks.
const SAMPLE_EVERY: Duration = Duration::from_millis(500);

/// How far the pointer has to move with a tab held before it counts as a drag
/// rather than a click that wobbled.
const DRAG_SLOP: f64 = 5.0;

/// The window will not grow past this. Unbounded is not useful: a conversation
/// at four thousand pixels wide is one long line per paragraph, and the panes
/// stop being panes.
const MAX_SIZE: LogicalSize<f64> = LogicalSize::new(2200.0, 1400.0);
const MIN_SIZE: LogicalSize<f64> = LogicalSize::new(680.0, 380.0);

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<noob_gpu::Gpu>,
    renderer: Option<noob_draw::Renderer>,
    proxy: EventLoopProxy<Wake>,

    config: Config,
    state: State,
    monitor: Monitor,
    /// Facts about this machine, shown under the readings.
    reports: Vec<String>,
    next_sample: Option<Instant>,
    skin: Skin,
    link: Option<Link>,
    trouble: Option<String>,

    input: String,
    caret: usize,
    dock: Dock,
    /// A tab that has been pressed, and whether the pointer has moved far
    /// enough since to call it a drag rather than a click.
    holding: Option<(View, Space, PhysicalPosition<f64>)>,
    drag: Option<Drag>,
    shaded: bool,
    /// The size to go back to when the window is unshaded.
    unshaded: Option<PhysicalSize<u32>>,
    column: f32,
    pane_column: f32,

    cursor: PhysicalPosition<f64>,
    hot: Option<Hit>,
    last_click: Option<(Hit, Instant)>,
    modifiers: ModifiersState,
    /// Where time is measured from. The rates need a monotonic clock and
    /// `state` is deliberately pure, so it is kept here and passed in.
    epoch: Instant,
    dirty: bool,
}

impl App {
    fn new(proxy: EventLoopProxy<Wake>, config: Config) -> App {
        let skin = Skin::from(&config);
        App {
            window: None,
            gpu: None,
            renderer: None,
            proxy,
            config,
            state: State::new(),
            monitor: Monitor::new(),
            reports: Vec::new(),
            next_sample: None,
            skin,
            link: None,
            trouble: None,
            input: String::new(),
            caret: 0,
            dock: Dock::new(),
            holding: None,
            drag: None,
            shaded: false,
            unshaded: None,
            column: 8.0,
            pane_column: 8.0,
            cursor: PhysicalPosition::new(0.0, 0.0),
            hot: None,
            last_click: None,
            modifiers: ModifiersState::empty(),
            epoch: Instant::now(),
            dirty: true,
        }
    }

    fn shape(&self) -> Shape<'_> {
        let width = self.gpu.as_ref().map_or(1.0, |gpu| gpu.width());
        Shape {
            shaded: self.shaded,
            dock: &self.dock,
            file_labels: self
                .state
                .files
                .iter()
                .map(|file| view::short_name(&file.path))
                .collect(),
            column: self.column,
            input_h: view::input_height(
                width,
                self.column,
                self.input.chars().count(),
                noob_draw::Text::line_for(self.config.font_size),
            ),
        }
    }

    fn layout(&self) -> Layout {
        let (w, h) = match &self.gpu {
            Some(gpu) => (gpu.width(), gpu.height()),
            None => (1.0, 1.0),
        };
        Layout::compute(w, h, &self.shape())
    }

    /// The view the pointer is over, for routing the wheel and page keys.
    fn under_pointer(&self, layout: &Layout) -> Option<(View, noob_draw::Panel)> {
        let space = layout
            .hit(self.cursor.x as f32, self.cursor.y as f32)?
            .space()?;
        let view = self.dock.slot(space).active()?;
        Some((view, layout.placed(space).body))
    }

    /// Start the agent. A failure is shown in the window rather than printed to
    /// a terminal nobody is watching.
    fn connect(&mut self) {
        let program = std::env::var("NOOB_BIN").unwrap_or_else(|_| String::from("noob"));
        let workspace = std::env::args()
            .nth(1)
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let proxy = self.proxy.clone();
        match Link::spawn(&program, &workspace, None, move || {
            let _ = proxy.send_event(Wake);
        }) {
            Ok(link) => {
                self.link = Some(link);
                self.state.workspace = workspace.display().to_string();
            }
            Err(message) => {
                self.trouble = Some(message.clone());
                self.state.talk.say(message, Tone::Bad);
            }
        }
        self.dirty = true;
    }

    fn drain(&mut self) {
        let Some(link) = self.link.as_mut() else {
            return;
        };
        for item in link.drain() {
            match item {
                Incoming::Frame(event) => {
                    let at = self.epoch.elapsed().as_secs_f64();
                    self.dirty |= self.state.apply_at(event, Some(at));
                }
                Incoming::Diagnostic(line) => {
                    self.state.talk.say(format!("noob: {line}"), Tone::Bad);
                    self.dirty = true;
                }
                Incoming::Ended(reason) => {
                    self.state.phase = state::Phase::Gone;
                    self.trouble = Some(reason);
                    self.dirty = true;
                }
            }
        }
    }

    fn submit(&mut self) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.input.clear();
        self.caret = 0;
        self.state.submitted(&text);
        match self.link.as_mut() {
            Some(link) if link.is_alive() => link.send(Cmd::PromptSubmit { text }),
            _ => self.state.talk.say("no agent is running", Tone::Bad),
        }
        self.dirty = true;
    }

    fn cancel(&mut self) {
        if let Some(link) = self.link.as_mut() {
            link.send(Cmd::TurnCancel);
        }
        self.state.status = String::from("cancelling");
        self.dirty = true;
    }

    /// Collapse the window to its title bar, or restore it. The bar keeps
    /// showing what the agent is doing, so a shaded window is still a status
    /// light rather than a hidden one.
    fn shade(&mut self, window: &Window) {
        self.shaded = !self.shaded;
        if self.shaded {
            self.unshaded = Some(window.inner_size());
            let width = window.inner_size().width;
            let _ = window.request_inner_size(PhysicalSize::new(width, view::TITLE_H as u32));
        } else if let Some(size) = self.unshaded.take() {
            let _ = window.request_inner_size(size);
        }
        self.dirty = true;
    }

    fn click(&mut self, hit: Hit, window: &Window, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        let double = matches!(self.last_click, Some((last, at))
            if last == hit && now.duration_since(at) < DOUBLE_CLICK);
        self.last_click = Some((hit, now));

        match hit {
            Hit::Close => event_loop.exit(),
            Hit::Minimize => window.set_minimized(true),
            Hit::Maximize => window.set_maximized(!window.is_maximized()),
            Hit::TitleBar => {
                if double {
                    self.shade(window);
                } else {
                    let _ = window.drag_window();
                }
            }
            Hit::Tab(view, space) => {
                // Pressed, not yet clicked: a tab is also a drag handle, so
                // what this was is only decided when the pointer moves or is
                // released.
                self.holding = Some((view, space, self.cursor));
            }
            Hit::Fold(space) => {
                let slot = self.dock.slot_mut(space);
                slot.folded = !slot.folded;
                self.dirty = true;
            }
            Hit::File(index, _) => {
                self.state.open_file = index;
                self.dirty = true;
            }
            Hit::Body(_) | Hit::Input => {}
        }
    }

    /// The pointer came up. A tab that never moved is a click; one that did is
    /// a drop.
    fn release(&mut self) {
        let layout = self.layout();
        if let Some(drag) = self.drag.take() {
            if let Some(space) = layout
                .hit(self.cursor.x as f32, self.cursor.y as f32)
                .and_then(Hit::space)
            {
                self.dock.move_view(drag.view, space);
            }
            self.holding = None;
            self.dirty = true;
            return;
        }
        if let Some((view, space, _)) = self.holding.take() {
            let slot = self.dock.slot_mut(space);
            // Clicking the tab already showing folds its space away, which is
            // how a pane gets out of the way without a second control.
            if slot.active() == Some(view) {
                slot.folded = !slot.folded;
            } else {
                slot.show(view);
                slot.folded = false;
            }
            self.dirty = true;
        }
    }

    /// Promote a held tab to a drag once the pointer has moved far enough that
    /// it cannot have been a click.
    fn maybe_drag(&mut self) {
        let Some((view, _, from)) = self.holding else {
            return;
        };
        let moved = (self.cursor.x - from.x).abs() + (self.cursor.y - from.y).abs();
        if self.drag.is_none() && moved < DRAG_SLOP {
            return;
        }
        let layout = self.layout();
        self.drag = Some(Drag {
            view,
            at: (self.cursor.x as f32, self.cursor.y as f32),
            onto: layout
                .hit(self.cursor.x as f32, self.cursor.y as f32)
                .and_then(Hit::space),
        });
        self.dirty = true;
    }

    fn key(&mut self, event: winit::event::KeyEvent, event_loop: &ActiveEventLoop) {
        if event.state != ElementState::Pressed {
            return;
        }
        let ctrl = self.modifiers.control_key();
        match event.logical_key.as_ref() {
            Key::Named(NamedKey::Enter) => self.submit(),
            Key::Named(NamedKey::Backspace) => {
                if self.caret > 0 {
                    // By character, not by byte: a backspace after an emoji
                    // must remove the emoji, not half of it.
                    let mut chars: Vec<char> = self.input.chars().collect();
                    chars.remove(self.caret - 1);
                    self.input = chars.into_iter().collect();
                    self.caret -= 1;
                    self.dirty = true;
                }
            }
            Key::Named(NamedKey::Delete) => {
                let mut chars: Vec<char> = self.input.chars().collect();
                if self.caret < chars.len() {
                    chars.remove(self.caret);
                    self.input = chars.into_iter().collect();
                    self.dirty = true;
                }
            }
            Key::Named(NamedKey::ArrowLeft) => {
                self.caret = self.caret.saturating_sub(1);
                self.dirty = true;
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.caret = (self.caret + 1).min(self.input.chars().count());
                self.dirty = true;
            }
            Key::Named(NamedKey::Home) => {
                self.caret = 0;
                self.dirty = true;
            }
            Key::Named(NamedKey::End) => {
                self.caret = self.input.chars().count();
                self.dirty = true;
            }
            // Escape clears a half-typed line first, and only cancels the turn
            // when there is nothing to clear. Losing a typed prompt to a key
            // meant for the agent is the worse mistake of the two.
            Key::Named(NamedKey::Escape) => {
                if self.input.is_empty() {
                    self.cancel();
                } else {
                    self.input.clear();
                    self.caret = 0;
                    self.dirty = true;
                }
            }
            // Shift-Tab stays in one space and walks its own tabs; plain Tab
            // walks the whole window.
            Key::Named(NamedKey::Tab) if self.modifiers.shift_key() => {
                let layout = self.layout();
                let space = layout
                    .hit(self.cursor.x as f32, self.cursor.y as f32)
                    .and_then(Hit::space)
                    .unwrap_or(Space::TopRight);
                self.dirty |= self.dock.slot_mut(space).cycle();
            }
            Key::Named(NamedKey::Tab) => {
                let showing = Space::ALL
                    .into_iter()
                    .find_map(|space| self.dock.slot(space).active())
                    .unwrap_or(View::Talk);
                let at = self
                    .dock
                    .slot(Space::TopRight)
                    .active()
                    .unwrap_or(showing);
                if let Some(next) = self.dock.after(at) {
                    self.dock.reveal(next);
                }
                self.dirty = true;
            }
            Key::Named(NamedKey::PageUp) => self.scroll_hovered(1.0),
            Key::Named(NamedKey::PageDown) => self.scroll_hovered(-1.0),
            Key::Character("q") if ctrl => event_loop.exit(),
            Key::Character("c") if ctrl => self.cancel(),
            Key::Character("u") if ctrl => {
                self.input.clear();
                self.caret = 0;
                self.dirty = true;
            }
            _ => {
                // Printable text, from the platform's own composition, so dead
                // keys and IME produce the character they were meant to.
                let Some(text) = event.text.as_ref() else {
                    return;
                };
                if ctrl || self.modifiers.super_key() {
                    return;
                }
                let typed: String = text.chars().filter(|c| !c.is_control()).collect();
                if typed.is_empty() {
                    return;
                }
                let mut chars: Vec<char> = self.input.chars().collect();
                for (i, c) in typed.chars().enumerate() {
                    chars.insert(self.caret + i, c);
                }
                self.caret += typed.chars().count();
                self.input = chars.into_iter().collect();
                self.dirty = true;
            }
        }
    }

    /// Scroll whatever the pointer is over, in pages. Positive is back into
    /// history.
    fn scroll_hovered(&mut self, pages: f32) {
        let layout = self.layout();
        let Some((view, panel)) = self.under_pointer(&layout) else {
            return;
        };
        let size = match view {
            View::Talk => self.config.font_size,
            _ => self.config.pane_font_size,
        };
        let rows = layout.rows(panel, size).saturating_sub(1).max(1);
        let by = ((rows as f32 * pages.abs()).round() as usize).max(1);
        let open_file = self.state.open_file;
        let pane = match view {
            View::Talk => Some(&mut self.state.talk),
            View::Activity => Some(&mut self.state.activity),
            View::Files => self
                .state
                .files
                .get_mut(open_file)
                .map(|file| &mut file.pane),
            _ => None,
        };
        let Some(pane) = pane else {
            return;
        };
        self.dirty |= if pages > 0.0 {
            pane.scroll_back(by, rows)
        } else {
            pane.scroll_forward(by)
        };
    }

    fn render(&mut self) {
        let (Some(gpu), Some(renderer)) = (self.gpu.as_mut(), self.renderer.as_mut()) else {
            return;
        };
        let Some(frame) = gpu.acquire() else {
            return;
        };
        let shape = Shape {
            shaded: self.shaded,
            dock: &self.dock,
            file_labels: self
                .state
                .files
                .iter()
                .map(|file| view::short_name(&file.path))
                .collect(),
            column: self.column,
            input_h: view::input_height(
                gpu.width(),
                self.column,
                self.input.chars().count(),
                noob_draw::Text::line_for(self.config.font_size),
            ),
        };
        let layout = Layout::compute(gpu.width(), gpu.height(), &shape);
        let scene = view::build(&view::Frame {
            state: &self.state,
            monitor: &self.monitor,
            dock: &self.dock,
            skin: &self.skin,
            layout: &layout,
            input: &self.input,
            caret: self.caret,
            column: self.column,
            pane_column: self.pane_column,
            body_size: self.config.font_size,
            pane_size: self.config.pane_font_size,
            reports: &self.reports,
            drag: self.drag,
            hot: self.hot,
            trouble: self.trouble.as_deref(),
        });
        renderer.draw(gpu, &scene, frame);
        self.dirty = false;
    }

    fn redraw(&self) {
        if self.dirty && let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl ApplicationHandler<Wake> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("CLIppy")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(true)
            .with_min_inner_size(MIN_SIZE)
            .with_max_inner_size(MAX_SIZE)
            .with_inner_size(LogicalSize::new(1180.0, 760.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(e) => {
                eprintln!("clippy: cannot open a window: {e}");
                event_loop.exit();
                return;
            }
        };
        match pollster::block_on(noob_gpu::Gpu::new(window.clone())) {
            Ok(gpu) => {
                // A surface that refused alpha gets an opaque palette, which
                // looks deliberate rather than looking broken.
                if !gpu.caps.transparent {
                    self.skin = self.skin.opaque();
                }
                // Facts about the machine belong beside the readings, not in
                // the log of what the agent did.
                self.reports = gpu.caps.report();
                self.reports.push(config::describe());
                for key in &self.config.unknown {
                    self.state
                        .activity
                        .say(format!("settings: {key:?} is not a setting"), Tone::Bad);
                }
                let mut renderer = noob_draw::Renderer::new(&gpu);
                self.column = renderer.column_width(self.config.font_size);
                self.pane_column = renderer.column_width(self.config.pane_font_size);
                self.renderer = Some(renderer);
                self.gpu = Some(gpu);
            }
            Err(e) => {
                eprintln!("clippy: {e}");
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        self.connect();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _wake: Wake) {
        self.drain();
        self.redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
                self.dirty = true;
                self.redraw();
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                self.key(event, event_loop);
                self.redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                self.maybe_drag();
                let (x, y) = (position.x as f32, position.y as f32);
                let layout = self.layout();
                let hot = match layout.hit(x, y) {
                    Some(hit @ (Hit::Close | Hit::Maximize | Hit::Minimize)) => Some(hit),
                    _ => None,
                };
                if hot != self.hot {
                    self.hot = hot;
                    self.dirty = true;
                }
                // The pointer shape is the only thing telling a user that an
                // undecorated window can be resized at all.
                if let Some(window) = &self.window {
                    let icon = match view::edge(x, y, layout.width, layout.height) {
                        Some(dir) => resize_cursor(dir),
                        None => CursorIcon::Default,
                    };
                    window.set_cursor(icon);
                }
                self.redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                if self.hot.take().is_some() {
                    self.dirty = true;
                    self.redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 40.0) as f32,
                };
                self.scroll_hovered(lines * 0.34);
                self.redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let Some(window) = self.window.clone() else {
                    return;
                };
                let (x, y) = (self.cursor.x as f32, self.cursor.y as f32);
                let layout = self.layout();
                // A resize edge wins against whatever pane is under it, or the
                // border becomes six pixels of unusable pane.
                if !self.shaded
                    && let Some(dir) = view::edge(x, y, layout.width, layout.height)
                {
                    let _ = window.drag_resize_window(dir);
                    return;
                }
                if let Some(hit) = layout.hit(x, y) {
                    self.click(hit, &window, event_loop);
                }
                self.redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.release();
                self.redraw();
            }
            WindowEvent::RedrawRequested => self.render(),
            _ => {}
        }
    }

    /// Called when the loop is about to block. Idle means block indefinitely:
    /// an interface showing static text should cost nothing, and polling is how
    /// a UI silently eats a GPU.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // The monitor is the one thing that changes without an event, so it is
        // the one thing that gets a clock, and only while it is on screen.
        let watching = !self.shaded
            && Space::ALL.into_iter().any(|space| {
                let slot = self.dock.slot(space);
                !slot.folded && matches!(slot.active(), Some(View::Hardware) | Some(View::Llm))
            });
        if watching {
            let now = Instant::now();
            if self.next_sample.is_none_or(|at| now >= at) {
                self.monitor.sample(&self.state);
                self.next_sample = Some(now + SAMPLE_EVERY);
                self.dirty = true;
            }
        } else {
            self.next_sample = None;
        }
        self.redraw();
        event_loop.set_control_flow(match self.next_sample {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(link) = self.link.as_mut() {
            link.shutdown();
        }
    }
}

fn resize_cursor(dir: winit::window::ResizeDirection) -> CursorIcon {
    use winit::window::ResizeDirection as Dir;
    match dir {
        Dir::North => CursorIcon::NResize,
        Dir::South => CursorIcon::SResize,
        Dir::East => CursorIcon::EResize,
        Dir::West => CursorIcon::WResize,
        Dir::NorthEast => CursorIcon::NeResize,
        Dir::NorthWest => CursorIcon::NwResize,
        Dir::SouthEast => CursorIcon::SeResize,
        Dir::SouthWest => CursorIcon::SwResize,
    }
}

fn main() {
    let config = Config::load();
    let event_loop = match EventLoop::<Wake>::with_user_event().build() {
        Ok(loop_) => loop_,
        Err(e) => {
            eprintln!("clippy: no display: {e}");
            std::process::exit(1);
        }
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(event_loop.create_proxy(), config);
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("clippy: {e}");
        std::process::exit(1);
    }
}
