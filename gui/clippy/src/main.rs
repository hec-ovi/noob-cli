//! CLIppy: a window for watching an agent work.
//!
//! It starts `noob serve` in a workspace, sends what you type as a
//! `prompt.submit` and renders the frames that come back. There is no terminal
//! involved and no web anything: one GPU surface, no system window chrome, and
//! four panes that separate the model's prose from its shell, its tools and the
//! files it is changing.
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
//! is not `noob` on PATH.

mod link;
mod skin;
mod state;
mod syntax;
mod view;

use std::sync::Arc;

use noob_proto::Command as Cmd;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

use link::{Incoming, Link};
use skin::Skin;
use state::{State, Stream};
use view::Layout;

/// The only user event: something arrived, come and look. Deliberately carries
/// no payload; the channel holds the frames and the loop drains it.
struct Wake;

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<noob_gpu::Gpu>,
    renderer: Option<noob_draw::Renderer>,
    proxy: EventLoopProxy<Wake>,

    state: State,
    skin: Skin,
    link: Option<Link>,
    trouble: Option<String>,

    input: String,
    caret: usize,
    focus: Stream,
    column: f32,

    cursor: PhysicalPosition<f64>,
    modifiers: ModifiersState,
    dirty: bool,
}

impl App {
    fn new(proxy: EventLoopProxy<Wake>) -> App {
        App {
            window: None,
            gpu: None,
            renderer: None,
            proxy,
            state: State::new(),
            skin: Skin::matrix(),
            link: None,
            trouble: None,
            input: String::new(),
            caret: 0,
            focus: Stream::Talk,
            column: 8.0,
            cursor: PhysicalPosition::new(0.0, 0.0),
            modifiers: ModifiersState::empty(),
            dirty: true,
        }
    }

    fn layout(&self) -> Layout {
        match &self.gpu {
            Some(gpu) => Layout::compute(gpu.width(), gpu.height()),
            None => Layout::compute(1.0, 1.0),
        }
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
                self.state.status = String::from("no agent");
                self.trouble = Some(message.clone());
                self.state.talk.say(message, state::Tone::Bad);
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
                Incoming::Frame(event) => self.dirty |= self.state.apply(event),
                Incoming::Diagnostic(line) => {
                    self.state.talk.say(format!("noob: {line}"), state::Tone::Bad);
                    self.dirty = true;
                }
                Incoming::Ended(reason) => {
                    self.state.busy = false;
                    self.state.status = reason.clone();
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
        self.focus = Stream::Talk;
        match self.link.as_mut() {
            Some(link) if link.is_alive() => link.send(Cmd::PromptSubmit { text }),
            _ => self
                .state
                .talk
                .say("no agent is running", state::Tone::Bad),
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
            Key::Named(NamedKey::Tab) => {
                self.focus = match self.focus {
                    Stream::Talk => Stream::Shell,
                    Stream::Shell => Stream::Tools,
                    Stream::Tools => Stream::Code,
                    Stream::Code => Stream::Talk,
                };
                self.dirty = true;
            }
            Key::Named(NamedKey::PageUp) => self.scroll(self.focus, 1.0),
            Key::Named(NamedKey::PageDown) => self.scroll(self.focus, -1.0),
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

    /// Scroll a pane by whole pages. Positive is back into history.
    fn scroll(&mut self, stream: Stream, pages: f32) {
        let layout = self.layout();
        let rows = layout.rows(stream);
        let by = ((rows as f32 * pages.abs()).round() as usize).max(1);
        let moved = if pages > 0.0 {
            self.state.pane_mut(stream).scroll_back(by, rows)
        } else {
            self.state.pane_mut(stream).scroll_forward(by)
        };
        self.dirty |= moved;
    }

    fn render(&mut self) {
        let (Some(gpu), Some(renderer)) = (self.gpu.as_mut(), self.renderer.as_mut()) else {
            return;
        };
        let Some(frame) = gpu.acquire() else {
            return;
        };
        let layout = Layout::compute(gpu.width(), gpu.height());
        let scene = view::build(&view::Frame {
            state: &self.state,
            skin: &self.skin,
            layout: &layout,
            focus: self.focus,
            input: &self.input,
            caret: self.caret,
            column: self.column,
            trouble: self.trouble.as_deref(),
        });
        renderer.draw(gpu, &scene, frame);
        self.dirty = false;
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
                for line in gpu.caps.report() {
                    self.state.tools.say(line, state::Tone::Dim);
                }
                let mut renderer = noob_draw::Renderer::new(&gpu);
                self.column = renderer.column_width(14.0);
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
        if self.dirty && let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = self.gpu.as_mut() {
                    gpu.resize(size.width, size.height);
                }
                self.dirty = true;
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. } => {
                self.key(event, event_loop);
                if self.dirty && let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                // The pointer shape is the only thing telling a user that an
                // undecorated window can be resized at all.
                if let Some(window) = &self.window {
                    let (w, h) = (
                        window.inner_size().width as f32,
                        window.inner_size().height as f32,
                    );
                    let icon = match view::edge(position.x as f32, position.y as f32, w, h) {
                        Some(dir) => resize_cursor(dir),
                        None => CursorIcon::Default,
                    };
                    window.set_cursor(icon);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 40.0) as f32,
                };
                let layout = self.layout();
                let over = layout
                    .pane_at(self.cursor.x as f32, self.cursor.y as f32)
                    .unwrap_or(self.focus);
                self.scroll(over, lines * 0.34);
                if self.dirty && let Some(window) = &self.window {
                    window.request_redraw();
                }
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
                let size = window.inner_size();
                let layout = self.layout();
                if layout.close.contains(x, y) {
                    event_loop.exit();
                } else if let Some(dir) =
                    view::edge(x, y, size.width as f32, size.height as f32)
                {
                    let _ = window.drag_resize_window(dir);
                } else if layout.title.contains(x, y) {
                    let _ = window.drag_window();
                } else if let Some(stream) = layout.pane_at(x, y) {
                    self.focus = stream;
                    self.dirty = true;
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => self.render(),
            _ => {}
        }
    }

    /// Called when the loop is about to block. Idle means block indefinitely:
    /// an interface showing static text should cost nothing, and polling is how
    /// a UI silently eats a GPU.
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.dirty && let Some(window) = &self.window {
            window.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::Wait);
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
    let event_loop = match EventLoop::<Wake>::with_user_event().build() {
        Ok(loop_) => loop_,
        Err(e) => {
            eprintln!("clippy: no display: {e}");
            std::process::exit(1);
        }
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(event_loop.create_proxy());
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("clippy: {e}");
        std::process::exit(1);
    }
}
