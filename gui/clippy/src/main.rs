//! NO0B: a window for watching an agent work.
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
//! Usage: `no0b [workspace]`, with `NOOB_BIN` naming the agent binary when it
//! is not `noob` on PATH. With no folder named the window opens on the picker
//! (see `picker`) rather than on whatever directory the process was started in,
//! which under a desktop launcher is the home directory. Settings live beside
//! noob's own; see `config`. `no0b --set <key>=<value>` changes one of them
//! and exits.

/// The clip player, with nothing drawing it at the moment. Kept compiled and
/// tested because the format it reads is about to carry an idle animation in
/// the corner of the window; deleting the parser with the view would mean
/// writing it again.
#[allow(dead_code)]
mod avatar;
mod config;
mod dock;
mod icons;
mod link;
mod markdown;
mod menu;
mod monitor;
mod orb;
mod packaging;
mod picker;
mod prompt;
mod select;
mod skin;
mod state;
mod syntax;
mod totals;
mod view;

use std::path::PathBuf;
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
use menu::{Item, Menu, Target};
use monitor::Monitor;
use picker::Picker;
use prompt::Prompt;
use skin::Skin;
use state::{State, Tone};
use totals::Totals;
use view::{Drag, Hit, Landing, Layout, Shape};

/// The only user event: something arrived, come and look. Deliberately carries
/// no payload; the channel holds the frames and the loop drains it.
struct Wake;

/// A second click inside this long, on the same thing, is a double click.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// How often the monitor reads the kernel, and only while it is on screen. A
/// monitor is inherently periodic, which is the opposite of the
/// redraw-on-change rule; the rule is kept by not sampling when nobody looks.
const SAMPLE_EVERY: Duration = Duration::from_millis(500);

/// The views this clock exists for. Every one of the three reads its numbers out
/// of [`Monitor::sample`], the two token panes included, so a pane left off this
/// list would show whatever the counters said when it was opened.
const SAMPLED: [View; 3] = [View::Hardware, View::Context, View::Session];

/// Whether the sampling clock should be running: one of [`SAMPLED`] is the
/// showing view of an unfolded space, and nothing is covering the window.
fn sampling(shaded: bool, picking: bool, dock: &Dock) -> bool {
    !shaded
        && !picking
        && Space::ALL.into_iter().any(|space| {
            let slot = dock.slot(space);
            !slot.folded && slot.active().is_some_and(|view| SAMPLED.contains(&view))
        })
}

/// How often the orb gets a new frame while a turn is running. Thirty a second
/// is enough for an orbit to read as motion and it costs 516 rectangles a frame,
/// which is one draw call.
const ORB_EVERY: Duration = Duration::from_millis(33);

/// How far the pointer has to move with a tab held before it counts as a drag
/// rather than a click that wobbled.
const DRAG_SLOP: f64 = 5.0;

/// The window will not grow past this. Unbounded is not useful: a conversation
/// at four thousand pixels wide is one long line per paragraph, and the panes
/// stop being panes.
const MAX_SIZE: LogicalSize<f64> = LogicalSize::new(2200.0, 1400.0);
const MIN_SIZE: LogicalSize<f64> = LogicalSize::new(680.0, 380.0);

/// What shading asks the window for: the minimum inner size to hold it to, and
/// the size to become. Split out from [`App::shade`] so the rule can be tested
/// without a compositor.
///
/// Shaded there is no minimum at all. `MIN_SIZE` is taller than the strip, and
/// a window that keeps its minimum while shaded simply does not shrink.
fn shade_request(
    shaded: bool,
    remembered: Option<PhysicalSize<u32>>,
) -> (Option<LogicalSize<f64>>, Option<PhysicalSize<u32>>) {
    match (shaded, remembered) {
        (true, Some(was)) => (
            None,
            Some(PhysicalSize::new(was.width, view::TITLE_H as u32)),
        ),
        (true, None) => (None, None),
        (false, was) => (Some(MIN_SIZE), was),
    }
}

/// When the orb wants its next frame, given the deadline it is already holding.
///
/// `None` is the point of this function: the clock exists only while there is a
/// turn to animate and disappears the moment the turn ends. An earlier version of
/// this window free-ran at 3,500 frames a second drawing text that was not
/// changing and spent a third of the graphics pipe on it, which is what
/// `noob-gpu` warns about and why nothing here ever asks for `ControlFlow::Poll`.
///
/// Pure so the rule can be tested without a window: an animation deadline is not
/// something to find out about by watching a fan.
fn orb_deadline(now: Instant, busy: bool, pending: Option<Instant>) -> Option<Instant> {
    if !busy {
        return None;
    }
    match pending {
        // Still waiting on the frame that was asked for.
        Some(at) if now < at => Some(at),
        _ => Some(now + ORB_EVERY),
    }
}

/// The one moment the event loop waits until, out of every clock the window
/// holds.
///
/// Composed rather than assigned. Two clocks that each set the control flow
/// leave whichever ran last in charge, and the other one either wakes late or
/// never wakes at all, which is a monitor that stops sampling as soon as
/// something animates.
fn soonest(deadlines: [Option<Instant>; 2]) -> Option<Instant> {
    deadlines.into_iter().flatten().min()
}

/// The folder named on the command line, if one was.
///
/// The first argument that is not a flag. Without one the window opens on the
/// picker: `current_dir()` under a desktop launcher is `$HOME`, and handing the
/// agent the home directory because nobody said otherwise is what this replaces.
fn workspace_arg(args: &[String]) -> Option<PathBuf> {
    args.iter()
        .find(|arg| !arg.starts_with('-'))
        .map(PathBuf::from)
}

/// What a right click opens, for what it landed on, or nothing when it landed
/// on something no menu belongs to: the title strip, a window button, the
/// margin between panes.
///
/// A free function taking everything it reads, so the routing from a hit to a
/// menu can be tested without a window or a GPU. The greying of the copy rows
/// is decided here too, because whether there is anything to copy is something
/// only the window knows.
fn menu_for(
    hit: Option<Hit>,
    at: (f32, f32),
    dock: &Dock,
    prompt_selection: bool,
    pane_selection: Option<View>,
) -> Option<Menu> {
    let widget = |view: View, space: Space| {
        Some(Menu::for_widget(
            at,
            view,
            space,
            pane_selection == Some(view),
        ))
    };
    match hit? {
        Hit::Input => Some(Menu::for_input(at, prompt_selection)),
        Hit::Tab(view, space) => widget(view, space),
        // A pane and the rows of its own file list are the same widget: the menu
        // acts on whatever that space is showing.
        Hit::Body(space) | Hit::File(_, space) => widget(dock.slot(space).active()?, space),
        Hit::TitleBar | Hit::Close | Hit::Maximize | Hit::Minimize => None,
        // The menu already open. Its own right click is handled before this is
        // reached, and a row is picked with the left button.
        Hit::Menu | Hit::MenuRow(_) => None,
        // The picker is not a widget: there is no pane to close, no settings
        // behind it, and nothing in it to select.
        Hit::Picker | Hit::PickerRow(_) | Hit::PickerOpen => None,
    }
}

/// What a released tab does to the arrangement.
///
/// Off the window closes the widget. Pure so the rule can be tested without a
/// compositor, and so the one place a drop changes the dock is the one place a
/// test drives.
fn land(dock: &mut Dock, view: View, landing: Landing) -> bool {
    match landing {
        Landing::In(space) => dock.move_view(view, space),
        Landing::Out => dock.hide(view),
        Landing::Nowhere => false,
    }
}

/// What a paste actually puts in the prompt.
///
/// The prompt is one line that wraps, not a text area, and Enter submits. A
/// newline pasted straight in has no glyph in any font, so it would draw as
/// nothing while still counting as a character; tabs and the rest of the
/// control characters are the same. They become spaces so a copied block of
/// code arrives as one readable line.
fn pasted(raw: &str) -> String {
    raw.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

struct App {
    window: Option<Arc<Window>>,
    gpu: Option<noob_gpu::Gpu>,
    renderer: Option<noob_draw::Renderer>,
    proxy: EventLoopProxy<Wake>,

    config: Config,
    state: State,
    monitor: Monitor,
    next_sample: Option<Instant>,
    /// When the orb wants its next frame, and `None` whenever it is still.
    ///
    /// The one animation clock in the window. It exists only while a turn is
    /// running, which is what keeps a window nobody is talking to at zero frames
    /// a second. See [`orb_deadline`].
    next_orb: Option<Instant>,
    /// The sessions that came before this one, as loaded. Never the live one:
    /// the running session is added on top whenever a reading or a write needs
    /// it, so writing the file twice cannot count this session twice.
    totals: Totals,
    totals_path: Option<std::path::PathBuf>,
    /// Whether a failed write has already been reported. A totals file that
    /// cannot be written is worth saying once and not once per turn.
    totals_trouble: bool,
    skin: Skin,
    link: Option<Link>,
    trouble: Option<String>,

    /// The folder named on the command line, until it has been connected to.
    /// Taken in `resumed`, because a folder given up front skips the picker.
    workspace: Option<PathBuf>,
    /// The folder picker, while it is up. Nothing else in the window is live
    /// while it is: there is no agent until it closes.
    picker: Option<Picker>,

    prompt: Prompt,
    dock: Dock,
    /// The open right click menu, or nothing. Held here rather than in the
    /// layout because it outlives a frame: it stays up until a row is picked or
    /// something puts it away.
    menu: Option<Menu>,
    /// A tab that has been pressed, and whether the pointer has moved far
    /// enough since to call it a drag rather than a click.
    holding: Option<(View, Space, PhysicalPosition<f64>)>,
    /// True while the button is down inside a pane, so pointer motion extends
    /// the selection instead of merely moving the cursor.
    selecting: bool,
    /// The same for the prompt. Separate from `selecting` because the two
    /// selections are different models over different text, and a drag that
    /// began in the prompt must not start extending a pane's band when the
    /// pointer wanders over one.
    prompt_selecting: bool,
    /// Opened once, and only when something is first copied: the clipboard
    /// connects to the display server, which is work an idle window has no
    /// reason to do.
    clipboard: Option<copypasta::ClipboardContext>,
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
    fn new(proxy: EventLoopProxy<Wake>, config: Config, workspace: Option<PathBuf>) -> App {
        let skin = Skin::from(&config);
        let totals_path = totals::path();
        // A view the settings turned off has no tab at all. Folding it away
        // would leave a strip saying the name of something you asked not to
        // see, which is not the same thing.
        let mut hidden = Vec::new();
        if !config.show_activity {
            hidden.push(View::Activity);
        }
        if !config.show_files {
            hidden.push(View::Files);
        }
        App {
            dock: Dock::hiding(&hidden),
            window: None,
            gpu: None,
            renderer: None,
            proxy,
            config,
            state: State::new(),
            monitor: Monitor::new(),
            next_sample: None,
            next_orb: None,
            totals: totals_path.as_deref().map(Totals::load).unwrap_or_default(),
            totals_path,
            totals_trouble: false,
            skin,
            link: None,
            trouble: None,
            workspace,
            picker: None,
            prompt: Prompt::default(),
            menu: None,
            holding: None,
            selecting: false,
            prompt_selecting: false,
            clipboard: None,
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
            menu: self.menu.as_ref(),
            picker: self.picker.as_ref(),
            file_labels: self
                .state
                .files
                .iter()
                .map(|file| view::short_name(&file.path))
                .collect(),
            file_first: self.state.file_scroll,
            column: self.column,
            pane_size: self.config.pane_font_size,
            pane_column: self.pane_column,
            input_h: self.input_height(width),
        }
    }

    /// How tall the prompt strip is for what has been typed so far.
    fn input_height(&self, width: f32) -> f32 {
        view::input_height(
            width,
            self.column,
            self.prompt.len(),
            noob_draw::Text::line_for(self.config.font_size),
            self.config.max_input_rows,
        )
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
        // The box the text is in, not the whole pane: the file view spends its
        // left column on the explorer, and scrolling it by the wider box moves
        // more rows than the file shows.
        Some((view, layout.content(space)))
    }

    /// Start the agent in `workspace`. A failure is shown in the window rather
    /// than printed to a terminal nobody is watching.
    ///
    /// Re-entrant: the picker calls it after the window is already up, so an
    /// agent from an earlier call has to be let go first. Nothing clears the
    /// transcript, because the only way here twice is through the picker before
    /// a turn has been taken and what is in it is the picker's own messages.
    fn connect(&mut self, workspace: PathBuf) {
        if let Some(mut link) = self.link.take() {
            link.shutdown();
        }
        self.trouble = None;
        let program = std::env::var("NOOB_BIN").unwrap_or_else(|_| String::from("noob"));
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
                self.state.output.say(message, Tone::Bad);
            }
        }
        self.dirty = true;
    }

    /// Open the picker, on the folder the process happens to be in.
    ///
    /// That folder is `$HOME` when the launcher started us, which is exactly why
    /// the picker exists: it is a place to start walking from, not a workspace.
    fn open_picker(&mut self) {
        let start = std::env::current_dir()
            .ok()
            .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        let recents = picker::recents_path()
            .map(|path| picker::load_recents(&path))
            .unwrap_or_default();
        self.picker = Some(Picker::open(Box::new(picker::Disk), start, recents));
        self.dirty = true;
    }

    /// A folder was chosen: remember it and start the agent there.
    fn choose(&mut self, workspace: PathBuf) {
        self.picker = None;
        if let Some(file) = picker::recents_path() {
            // Read again immediately before writing, rather than reusing what
            // the picker opened with, so a second window that chose a folder in
            // the meantime does not have its entry erased.
            let list = picker::remember(&picker::load_recents(&file), &workspace);
            // A recents file that cannot be written costs the next launch one
            // keystroke. Not worth a line in a conversation that is about to
            // start.
            let _ = picker::save_recents(&file, &list);
        }
        self.connect(workspace);
    }

    /// Keys while the picker is up. Nothing else in the window is live, so this
    /// is the whole keyboard rather than a branch inside [`App::key`].
    fn key_in_picker(&mut self, event: &winit::event::KeyEvent, event_loop: &ActiveEventLoop) {
        let rows = self.picker_rows();
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        let mut chosen = None;
        self.dirty |= match event.logical_key.as_ref() {
            Key::Named(NamedKey::ArrowUp) => picker.step(false),
            Key::Named(NamedKey::ArrowDown) => picker.step(true),
            Key::Named(NamedKey::PageUp) => picker.page(rows, false),
            Key::Named(NamedKey::PageDown) => picker.page(rows, true),
            Key::Named(NamedKey::Home) => picker.jump(false),
            Key::Named(NamedKey::End) => picker.jump(true),
            // Walking in has its own key so Enter can mean "this one": a picker
            // where Enter walks into folders needs a second gesture to choose,
            // and choosing is what it is for.
            Key::Named(NamedKey::ArrowRight) | Key::Named(NamedKey::Tab) => picker.walk_in(),
            Key::Named(NamedKey::ArrowLeft) => picker.walk_out(),
            Key::Named(NamedKey::Backspace) => picker.backspace(),
            Key::Named(NamedKey::Enter) => {
                chosen = picker.confirm();
                true
            }
            // Nothing has started yet, so there is nothing to cancel and no
            // pane to fall back to: Escape drops what has been typed, and with
            // nothing typed it closes the window.
            Key::Named(NamedKey::Escape) => {
                if !picker.clear_filter() {
                    event_loop.exit();
                }
                true
            }
            Key::Character("q") if self.modifiers.control_key() => {
                event_loop.exit();
                true
            }
            _ => match event.text.as_ref() {
                Some(text) if !self.modifiers.control_key() && !self.modifiers.super_key() => {
                    picker.type_text(text)
                }
                _ => false,
            },
        };
        if let Some(workspace) = chosen {
            self.choose(workspace);
            return;
        }
        self.reveal_picker_cursor();
    }

    /// How many rows the picker's list can show right now.
    fn picker_rows(&self) -> usize {
        self.layout().picker_capacity(self.config.pane_font_size)
    }

    /// Keep the cursor on screen after it has moved. Here rather than in the
    /// picker, because how many rows the list can show is a question about the
    /// window and the layout is the only thing that knows.
    fn reveal_picker_cursor(&mut self) {
        let rows = self.picker_rows();
        if let Some(picker) = self.picker.as_mut() {
            self.dirty |= picker.reveal(rows);
        }
    }

    /// A press inside the picker: a row, or the button that confirms.
    fn click_in_picker(&mut self, hit: Hit, double: bool) {
        let mut chosen = None;
        if let Some(picker) = self.picker.as_mut() {
            self.dirty |= match hit {
                Hit::PickerRow(index) if double => {
                    chosen = picker.double(index);
                    true
                }
                Hit::PickerRow(index) => picker.point_at(index),
                Hit::PickerOpen => {
                    chosen = picker.confirm();
                    true
                }
                _ => false,
            };
        }
        match chosen {
            Some(workspace) => self.choose(workspace),
            None => self.reveal_picker_cursor(),
        }
    }

    fn drain(&mut self) {
        let Some(link) = self.link.as_mut() else {
            return;
        };
        let mut turn_ended = false;
        for item in link.drain() {
            match item {
                Incoming::Frame(event) => {
                    let at = self.epoch.elapsed().as_secs_f64();
                    turn_ended |= matches!(event, noob_proto::Event::TurnEnd { .. });
                    self.dirty |= self.state.apply_at(event, Some(at));
                }
                Incoming::Diagnostic(line) => {
                    self.state.output.say(format!("noob: {line}"), Tone::Bad);
                    self.dirty = true;
                }
                Incoming::Ended(reason) => {
                    self.state.phase = state::Phase::Gone;
                    self.trouble = Some(reason);
                    self.dirty = true;
                }
            }
        }
        // The end of a turn is the natural boundary to record at: the numbers
        // have stopped moving and there are a handful of turns a minute, not a
        // handful of writes a second. A window killed mid-turn loses that turn,
        // which is the price of not rewriting the file on every token.
        if turn_ended {
            self.remember();
        }
        self.follow_open_file();
    }

    /// Keep the file the agent just touched on screen in the explorer.
    ///
    /// Here rather than in `State`, because how many rows the list can show is a
    /// question about the window, and the layout is the only thing that knows.
    fn follow_open_file(&mut self) {
        let layout = self.layout();
        if layout.file_list.h < 1.0 {
            return;
        }
        let rows = layout.rows(layout.file_list, self.config.pane_font_size);
        self.dirty |= self.state.reveal_open_file(rows);
    }

    /// Write the running totals: what was carried in, plus this session.
    fn remember(&mut self) {
        let Some(path) = self.totals_path.clone() else {
            return;
        };
        if let Err(error) = self.totals.plus(&self.state).save(&path) {
            // Once. A path that cannot be written will not start working, and a
            // line per turn about it would bury the conversation it interrupts.
            if !self.totals_trouble {
                self.totals_trouble = true;
                self.state
                    .output
                    .say(format!("cannot keep the running totals: {error}"), Tone::Bad);
                self.dirty = true;
            }
        }
    }

    fn submit(&mut self) {
        let text = self.prompt.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.prompt.clear();
        self.state.submitted(&text);
        match self.link.as_mut() {
            Some(link) if link.is_alive() => link.send(Cmd::PromptSubmit { text }),
            _ => self.state.output.say("no agent is running", Tone::Bad),
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
        }
        let (min, size) = shade_request(self.shaded, self.unshaded);
        // The minimum goes first. A resize request is clamped to it, and
        // asking a 680x380 minimum for a 30 pixel strip left the surface at
        // full height with the strip painted across the top of it, which is
        // the black bar the window showed when it was shaded.
        window.set_min_inner_size(min);
        if let Some(size) = size {
            let _ = window.request_inner_size(size);
        }
        if !self.shaded {
            self.unshaded = None;
        }
        self.dirty = true;
    }

    fn click(&mut self, hit: Hit, window: &Window, event_loop: &ActiveEventLoop) {
        // While a menu is open it decides what a press means, because it is
        // above the window: a row acts, its margin is swallowed, and anywhere
        // else only puts the menu away. Dismissing and acting on the same press
        // would fold a pane that was only being clicked past to close a menu.
        // Before the double click bookkeeping, so a dismiss is not remembered
        // as the first of a pair.
        if self.menu.is_some() {
            match hit {
                Hit::MenuRow(index) => self.pick(index),
                Hit::Menu => {}
                _ => {
                    self.menu = None;
                    self.dirty = true;
                }
            }
            return;
        }

        let now = Instant::now();
        let double = matches!(self.last_click, Some((last, at))
            if last == hit && now.duration_since(at) < DOUBLE_CLICK);
        self.last_click = Some((hit, now));

        if self.picker.is_some() {
            // The title bar is still the title bar while the picker is up: the
            // window can be moved, shaded and closed before a folder is chosen.
            if !matches!(hit, Hit::TitleBar | Hit::Close | Hit::Maximize | Hit::Minimize) {
                self.click_in_picker(hit, double);
                return;
            }
        }

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
            Hit::File(index, _) => {
                self.state.show_file(index);
                self.dirty = true;
            }
            // The debug pane is a list you click rather than text you select:
            // its rows open to show what was sent to the call that failed, and
            // there is nothing selectable in it (`State::pane_of` has no
            // scrollback for it), so a press here would otherwise do nothing.
            Hit::Body(space) if self.dock.slot(space).active() == Some(View::Debug) => {
                self.open_failure_under_pointer(space);
            }
            Hit::Body(space) => self.begin_selection(space),
            // All three are handled above, while the picker is up, which is the
            // only time any of them can be hit at all.
            Hit::PickerRow(_) | Hit::PickerOpen | Hit::Picker => {}
            Hit::Input => {
                // A press, not a placement: the anchor stays here so motion
                // with the button still down selects the span between the two.
                // An anchor sitting on the caret is not a selection, so a click
                // that never moves still reads as one that selected nothing.
                self.prompt.press(self.caret_under_pointer());
                self.prompt_selecting = true;
                self.dirty = true;
            }
            // Both handled above, while a menu is open, which is the only time
            // either can be hit at all.
            Hit::MenuRow(_) | Hit::Menu => {}
        }
    }

    /// Where in the typed text the pointer is, for a press or a drag in the
    /// prompt. The layout owns the arithmetic; this only feeds it the metrics
    /// the prompt is drawn with.
    fn caret_under_pointer(&self) -> usize {
        self.layout().input_caret(
            self.cursor.x as f32,
            self.cursor.y as f32,
            self.config.font_size,
            self.column,
            self.prompt.len(),
        )
    }

    /// The right button came down: open the menu for whatever is under it.
    fn right_click(&mut self) {
        let at = (self.cursor.x as f32, self.cursor.y as f32);
        // Over a menu that is already open, a second right click only puts it
        // away. Opening a menu for what is behind the one on screen would be a
        // menu for something the pointer cannot see.
        let over_menu = matches!(
            self.layout().hit(at.0, at.1),
            Some(Hit::MenuRow(_) | Hit::Menu)
        );
        let had = self.menu.take().is_some();
        if over_menu {
            self.dirty = true;
            return;
        }
        // Recomputed now the menu is down, so the target is resolved against
        // the window rather than against the menu that was floating over it.
        let layout = self.layout();
        self.menu = menu_for(
            layout.hit(at.0, at.1),
            at,
            &self.dock,
            self.prompt.selection().is_some(),
            self.pane_selection(),
        );
        self.dirty |= had || self.menu.is_some();
    }

    /// Which pane holds a selection worth copying, if any. A click that never
    /// moved leaves an empty selection behind, and a Copy row that lights up
    /// for one would copy nothing.
    fn pane_selection(&self) -> Option<View> {
        self.state
            .selection
            .filter(|selection| !selection.is_empty())
            .map(|selection| selection.view)
    }

    /// Do what the row at `index` says, and put the menu away either way.
    ///
    /// A greyed row still closes it: leaving a menu open under a pointer that
    /// has already committed to a row reads as a click that missed the window.
    fn pick(&mut self, index: usize) {
        let Some(menu) = self.menu.take() else {
            return;
        };
        self.dirty = true;
        let Some(item) = menu.pick(index) else {
            return;
        };
        match (item, menu.target) {
            (Item::Copy, _) => {
                self.copy_prompt();
            }
            (Item::Paste, _) => self.paste(),
            // There is no settings panel to open yet, so the row ships disabled
            // and `pick` never returns it. The arm is here so the day the panel
            // exists is one line rather than a hunt.
            (Item::Settings, _) => {}
            (Item::CopySelection, _) => {
                self.copy_selection();
            }
            (Item::Close, Target::Widget(view, _)) => self.close_view(view),
            // The prompt's menu has no Close row, so this cannot happen; it is
            // matched rather than caught by a wildcard so adding one is a
            // compile error here instead of a click that silently does nothing.
            (Item::Close, Target::Input) => {}
        }
    }

    /// Take a widget out of the window.
    ///
    /// One way for now: nothing inside the window puts it back, and the way
    /// back is the orb launcher, which does not exist yet. The arrangement
    /// survives it because a space with no tabs gives its room to its
    /// neighbour rather than leaving a hole; see `Layout::compute`.
    fn close_view(&mut self, view: View) {
        if self.dock.hide(view) {
            self.forget_selection_in(view);
            self.dirty = true;
        }
    }

    /// Drop a selection that belonged to a pane which is no longer on screen.
    /// Left behind it would still be what Ctrl-C copied, with nothing drawn
    /// anywhere saying so.
    fn forget_selection_in(&mut self, view: View) {
        if self
            .state
            .selection
            .is_some_and(|selection| selection.view == view)
        {
            self.state.selection = None;
        }
    }

    /// Put the clipboard into the prompt at the caret.
    fn paste(&mut self) {
        use copypasta::ClipboardProvider;
        if self.clipboard.is_none() {
            self.clipboard = copypasta::ClipboardContext::new().ok();
        }
        let got = match self.clipboard.as_mut() {
            Some(clipboard) => clipboard.get_contents(),
            None => {
                self.state.activity.say(
                    "could not reach the clipboard on this display",
                    state::Tone::Bad,
                );
                self.dirty = true;
                return;
            }
        };
        match got {
            Ok(text) => {
                let text = pasted(&text);
                if !text.is_empty() {
                    self.prompt.insert(&text);
                    self.dirty = true;
                }
            }
            // Worth saying out loud: the alternative is a paste that appeared
            // to do nothing.
            Err(e) => {
                self.state
                    .activity
                    .say(format!("nothing to paste: {e}"), state::Tone::Bad);
                self.dirty = true;
            }
        }
    }

    /// Open or close the failed call the pointer is on, in the debug pane.
    ///
    /// The row comes from the same `Layout::cell` arithmetic the panes are drawn
    /// with, and which failure that row belongs to comes from
    /// [`State::debug_rows`], which is the list that was drawn. Neither end
    /// guesses.
    fn open_failure_under_pointer(&mut self, space: Space) {
        let layout = self.layout();
        let Some((at, row, _)) = layout.cell(
            self.cursor.x as f32,
            self.cursor.y as f32,
            self.config.pane_font_size,
            self.pane_column,
        ) else {
            return;
        };
        if at != space {
            return;
        }
        self.dirty |= self.state.toggle_failure(row);
    }

    /// Press inside a pane: put the anchor where the pointer is, or clear the
    /// selection when the press is somewhere with no text to select.
    fn begin_selection(&mut self, space: Space) {
        let previous = self.state.selection.take();
        let Some(view) = self.dock.slot(space).active() else {
            self.dirty = previous.is_some();
            return;
        };
        let layout = self.layout();
        let spot = self.spot_at(&layout, space, view);
        self.state.selection = spot.map(|spot| select::Selection::new(view, spot));
        self.selecting = self.state.selection.is_some();
        self.dirty = true;
    }

    /// The character under the pointer, as a line and a column the pane can
    /// still resolve after it has scrolled.
    fn spot_at(&self, layout: &view::Layout, space: Space, view: View) -> Option<select::Spot> {
        let pane = self.state.pane_of(view)?;
        // The output pane is drawn at the transcript size, not the pane size.
        // Hit testing it with the smaller one put every click a growing number
        // of rows away from the character under the pointer.
        let (size, column) = self.metrics_of(view);
        let (over, row, at) =
            layout.cell(self.cursor.x as f32, self.cursor.y as f32, size, column)?;
        if over != space {
            return None;
        }
        // The box the glyphs are in, which is not the whole pane in the file
        // view: its left column is the explorer.
        let body = layout.content(space);
        let rows = layout.rows(body, size);
        // A file row is drawn with its line number in front of the text, so the
        // column under the pointer is that many columns further along than the
        // character it is over.
        let (cols, chrome) = view::text_columns(view, body, column);
        let at = at.saturating_sub(chrome);
        let Some((line, offset)) = pane.spot_in(rows, cols, row) else {
            // Below the last line, the selection runs to the end of the text
            // rather than to a line that does not exist.
            let last = pane.last().saturating_sub(1);
            let end = pane.line(last).map_or(0, |l| l.text.chars().count());
            return Some(select::Spot::new(last, end));
        };
        // `offset` is where this visual row starts inside its logical line, so
        // a click on the second row of a wrapped line lands past the wrap.
        Some(select::Spot::new(line, offset + at))
    }

    /// The font size and column width a view is drawn with. The window-side
    /// twin of `view::Frame::metrics_of`, for the paths that run before a
    /// frame exists.
    fn metrics_of(&self, view: View) -> (f32, f32) {
        match view {
            View::Output => (self.config.font_size, self.column),
            _ => (self.config.pane_font_size, self.pane_column),
        }
    }

    /// Extend the selection to wherever the pointer is now.
    fn extend_selection(&mut self) {
        let Some(mut selection) = self.state.selection else {
            return;
        };
        let layout = self.layout();
        let Some(space) = self.dock_space_of(selection.view) else {
            return;
        };
        if let Some(spot) = self.spot_at(&layout, space, selection.view) {
            selection.extend(spot);
            self.state.selection = Some(selection);
            self.dirty = true;
        }
    }

    fn dock_space_of(&self, view: View) -> Option<Space> {
        Space::ALL
            .into_iter()
            .find(|space| self.dock.slot(*space).active() == Some(view))
    }

    /// Put the pane selection on the system clipboard. Returns whether there
    /// was anything to copy, so the caller can fall back to what the key
    /// otherwise does.
    fn copy_selection(&mut self) -> bool {
        let Some(selection) = self.state.selection else {
            return false;
        };
        let Some(pane) = self.state.pane_of(selection.view) else {
            return false;
        };
        let text = selection.text(pane);
        if text.is_empty() {
            return false;
        }
        self.put_on_clipboard(text);
        true
    }

    /// The same, for what is selected in the prompt.
    fn copy_prompt(&mut self) -> bool {
        let Some(text) = self.prompt.selected() else {
            return false;
        };
        self.put_on_clipboard(text);
        true
    }

    /// A copy the user asked for, from wherever it came from. Prefers the
    /// prompt over a pane, because the prompt is what they were last typing
    /// into and a band left behind in the transcript must not swallow it.
    fn copy(&mut self) -> bool {
        self.copy_prompt() || self.copy_selection()
    }

    fn put_on_clipboard(&mut self, text: String) {
        use copypasta::ClipboardProvider;
        if self.clipboard.is_none() {
            self.clipboard = copypasta::ClipboardContext::new().ok();
        }
        match self.clipboard.as_mut() {
            Some(clipboard) => {
                // A clipboard that will not take it is worth saying out loud:
                // the alternative is a copy that silently did nothing.
                if let Err(e) = clipboard.set_contents(text) {
                    self.state.activity.say(
                        format!("could not reach the clipboard: {e}"),
                        state::Tone::Bad,
                    );
                }
            }
            None => self.state.activity.say(
                "could not reach the clipboard on this display",
                state::Tone::Bad,
            ),
        }
        self.dirty = true;
    }

    /// The pointer came up. A tab that never moved is a click; one that did is
    /// a drop.
    fn release(&mut self) {
        self.selecting = false;
        self.prompt_selecting = false;
        let layout = self.layout();
        if let Some(drag) = self.drag.take() {
            let landing = layout.landing(self.cursor.x as f32, self.cursor.y as f32);
            land(&mut self.dock, drag.view, landing);
            if self.dock.is_hidden(drag.view) {
                self.forget_selection_in(drag.view);
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
        // A menu was opened for a pointer, and a keystroke means the pointer
        // has been left behind. Leaving it floating over text being typed under
        // it is worse than losing it. Escape stops here rather than falling
        // through, so putting a menu away does not also drop a selection or a
        // half typed line.
        if self.menu.take().is_some() {
            self.dirty = true;
            if matches!(event.logical_key.as_ref(), Key::Named(NamedKey::Escape)) {
                return;
            }
        }
        if self.picker.is_some() {
            self.key_in_picker(&event, event_loop);
            return;
        }
        let ctrl = self.modifiers.control_key();
        match event.logical_key.as_ref() {
            Key::Named(NamedKey::Enter) => self.submit(),
            Key::Named(NamedKey::Backspace) => self.dirty |= self.prompt.backspace(),
            Key::Named(NamedKey::Delete) => self.dirty |= self.prompt.delete(),
            Key::Named(NamedKey::ArrowLeft) => {
                self.prompt.left();
                self.dirty = true;
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.prompt.right();
                self.dirty = true;
            }
            Key::Named(NamedKey::Home) => {
                self.prompt.home();
                self.dirty = true;
            }
            Key::Named(NamedKey::End) => {
                self.prompt.end();
                self.dirty = true;
            }
            // Escape drops a selection first, then a half-typed line, and only
            // cancels the turn when there is neither. Losing a typed prompt to
            // a key meant for the agent is the worse mistake of the two, and
            // losing a turn to one meant for a selection is worse still.
            Key::Named(NamedKey::Escape) => {
                if self.state.selection.take().is_some() {
                    self.dirty = true;
                } else if self.prompt.is_empty() {
                    self.cancel();
                } else {
                    self.prompt.clear();
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
                    .unwrap_or(View::Output);
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
            // Copy when there is a selection, cancel when there is not. The
            // same key does both because cancelling a turn is the thing this
            // window must never make hard to reach, and a selection you just
            // made is the thing you obviously meant.
            Key::Character("c") if ctrl => {
                if !self.copy() {
                    self.cancel();
                }
            }
            // And an unambiguous copy, for the muscle memory a terminal built.
            Key::Character("C") if ctrl && self.modifiers.shift_key() => {
                self.copy();
            }
            // Nothing in this window can take the keyboard focus, and the
            // prompt is its only text field, so select-all means the prompt
            // wherever the pointer happens to be. This is the line that has to
            // learn which pane has the focus on the day one can have it.
            Key::Character("a") if ctrl => {
                self.prompt.select_all();
                self.dirty = true;
            }
            // The other half of the prompt's menu, for the muscle memory every
            // other text field built.
            Key::Character("v") if ctrl => self.paste(),
            Key::Character("u") if ctrl => {
                self.prompt.clear();
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
                self.prompt.insert(&typed);
                self.dirty = true;
            }
        }
    }

    /// Scroll whatever the pointer is over, in pages. Positive is back into
    /// history.
    fn scroll_hovered(&mut self, pages: f32) {
        let layout = self.layout();
        // The picker is the only thing on screen while it is up, so the wheel is
        // its list wherever the pointer happens to be. The cursor stays where it
        // was: the wheel moves what you are looking at, not what you have picked.
        if self.picker.is_some() {
            let rows = layout.picker_capacity(self.config.pane_font_size);
            let by = ((rows as f32 * pages.abs()).round() as usize).max(1);
            if let Some(picker) = self.picker.as_mut() {
                self.dirty |= picker.scroll(by, pages < 0.0, rows);
            }
            return;
        }
        // Over the explorer, the wheel moves the list rather than the file. The
        // pointer is on the thing being scrolled, which is what every file tree
        // does, and the list is the only way to reach a file that is off it.
        if layout
            .file_list
            .contains(self.cursor.x as f32, self.cursor.y as f32)
        {
            let rows = layout.rows(layout.file_list, self.config.pane_font_size);
            let by = ((rows as f32 * pages.abs()).round() as usize).max(1);
            self.dirty |= self.state.scroll_files(by, pages < 0.0, rows);
            return;
        }
        let Some((view, panel)) = self.under_pointer(&layout) else {
            return;
        };
        let (size, column) = self.metrics_of(view);
        let rows = layout.rows(panel, size).saturating_sub(1).max(1);
        // The file view spends four columns per row on its gutter, so its text
        // wraps in a narrower box than the panel is wide.
        let (cols, _) = view::text_columns(view, panel, column);
        let by = ((rows as f32 * pages.abs()).round() as usize).max(1);
        let open_file = self.state.open_file;
        let pane = match view {
            View::Output => Some(&mut self.state.output),
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
            pane.scroll_back(by, rows, cols)
        } else {
            pane.scroll_forward(by)
        };
    }

    fn render(&mut self) {
        // Measured before the surface is borrowed, because the prompt's height
        // is read off the whole app and the renderer holds it mutably.
        let Some(input_h) = self.gpu.as_ref().map(|gpu| self.input_height(gpu.width())) else {
            return;
        };
        let (Some(gpu), Some(renderer)) = (self.gpu.as_mut(), self.renderer.as_mut()) else {
            return;
        };
        let Some(frame) = gpu.acquire() else {
            return;
        };
        let shape = Shape {
            shaded: self.shaded,
            dock: &self.dock,
            menu: self.menu.as_ref(),
            picker: self.picker.as_ref(),
            file_labels: self
                .state
                .files
                .iter()
                .map(|file| view::short_name(&file.path))
                .collect(),
            file_first: self.state.file_scroll,
            column: self.column,
            pane_size: self.config.pane_font_size,
            pane_column: self.pane_column,
            input_h,
        };
        let layout = Layout::compute(gpu.width(), gpu.height(), &shape);
        let scene = view::build(&view::Frame {
            state: &self.state,
            monitor: &self.monitor,
            dock: &self.dock,
            skin: &self.skin,
            layout: &layout,
            prompt: &self.prompt,
            column: self.column,
            pane_column: self.pane_column,
            body_size: self.config.font_size,
            pane_size: self.config.pane_font_size,
            drag: self.drag,
            hot: self.hot,
            trouble: self.trouble.as_deref(),
            selection: self.state.selection,
            // The same menu the layout above was computed from, or the rows
            // would be drawn somewhere other than where they are hit tested.
            menu: self.menu.as_ref(),
            picker: self.picker.as_ref(),
            // The orb's clock. Read here rather than inside the scene, so a
            // frame stays a function of what it is handed.
            clock: self.epoch.elapsed().as_secs_f32(),
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

/// The name the desktop knows this window by.
///
/// It has to be exactly the basename of the installed `.desktop` file or the
/// desktop cannot match the two, and an unmatched window gets a generic icon in
/// the dock and in the switcher no matter what the code does. On Wayland the
/// code cannot set an icon at all, so this string IS the icon.
pub const APP_ID: &str = "io.github.hec_ovi.NO0B";

/// What this program is called on a command line, which is what it calls itself
/// when it has to print something. From cargo rather than typed out, so it
/// cannot say one name while the shell says another.
pub const BINARY: &str = env!("CARGO_BIN_NAME");

fn window_attributes() -> winit::window::WindowAttributes {
    let attributes = Window::default_attributes().with_title("NO0B");
    // Set on both, because the same binary runs under either and the two
    // display servers spell the same idea differently.
    #[cfg(all(unix, not(target_os = "macos")))]
    let attributes = {
        use winit::platform::wayland::WindowAttributesExtWayland;
        use winit::platform::x11::WindowAttributesExtX11;
        WindowAttributesExtX11::with_name(
            WindowAttributesExtWayland::with_name(attributes, APP_ID, BINARY),
            APP_ID,
            BINARY,
        )
    };
    attributes
}

impl ApplicationHandler<Wake> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = window_attributes()
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(true)
            .with_min_inner_size(MIN_SIZE)
            .with_max_inner_size(MAX_SIZE)
            .with_inner_size(LogicalSize::new(1180.0, 760.0));
        // The window carries the answer to the launch, so the token has to be
        // on it before it exists; there is no saying it afterwards. Taken here
        // rather than in `main` so it is out of the environment before
        // `connect()` at the bottom of this function spawns the agent.
        #[cfg(all(unix, not(target_os = "macos")))]
        let attributes = match packaging::take_activation_token(event_loop) {
            Some(token) => {
                use winit::platform::startup_notify::WindowAttributesExtStartupNotify;
                attributes.with_activation_token(token)
            }
            None => attributes,
        };
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(e) => {
                eprintln!("{BINARY}: cannot open a window: {e}");
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
                eprintln!("{BINARY}: {e}");
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        // A folder on the command line means the window was opened for it and
        // there is nothing to ask. Without one, the picker is the first thing on
        // screen and it calls `connect` itself.
        match self.workspace.take() {
            Some(workspace) => self.connect(workspace),
            None => self.open_picker(),
        }
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
                if self.selecting {
                    self.extend_selection();
                }
                if self.prompt_selecting {
                    self.prompt.drag_to(self.caret_under_pointer());
                    self.dirty = true;
                }
                let (x, y) = (position.x as f32, position.y as f32);
                let layout = self.layout();
                let hot = match layout.hit(x, y) {
                    Some(
                        hit @ (Hit::Close
                        | Hit::Maximize
                        | Hit::Minimize
                        | Hit::MenuRow(_)
                        | Hit::PickerOpen),
                    ) => Some(hit),
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
                let hit = layout.hit(x, y);
                // A resize edge wins against whatever pane is under it, or the
                // border becomes six pixels of unusable pane. An open menu wins
                // against the edge in turn: it is drawn over the border, and a
                // menu clamped against the left of the window would otherwise
                // have its rows resize the window instead of acting.
                if !self.shaded
                    && !matches!(hit, Some(Hit::MenuRow(_) | Hit::Menu))
                    && let Some(dir) = view::edge(x, y, layout.width, layout.height)
                {
                    let _ = window.drag_resize_window(dir);
                    return;
                }
                if let Some(hit) = hit {
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
            // The right button only ever opens or closes a menu, so it has one
            // arm rather than a path through `click`, and nothing to do on the
            // way back up.
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                self.right_click();
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
        // Two things change without an event, and each one holds a clock only
        // while it is the case: the monitor while a monitor is on screen, and the
        // orb while a turn is running. Everything else redraws because something
        // happened, which is what keeps an idle window free.
        if sampling(self.shaded, self.picker.is_some(), &self.dock) {
            let now = Instant::now();
            if self.next_sample.is_none_or(|at| now >= at) {
                // The state and nothing else. The totals file used to be merged
                // in here for the pane that read it; it is still written at the
                // end of every turn, it just has no reader on screen.
                self.monitor.sample(&self.state);
                self.next_sample = Some(now + SAMPLE_EVERY);
                self.dirty = true;
            }
        } else {
            self.next_sample = None;
        }
        // The orb. A deadline that was not there before is a frame that is due
        // now, so it is also what marks the window dirty; the same one coming
        // back means the frame is still waiting and nothing is redrawn.
        let next = orb_deadline(Instant::now(), self.state.phase.busy(), self.next_orb);
        self.dirty |= next.is_some() && next != self.next_orb;
        self.next_orb = next;
        self.redraw();
        // Both clocks, never one of them: the earlier deadline wins and the other
        // is still there when it comes round.
        event_loop.set_control_flow(match soonest([self.next_sample, self.next_orb]) {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        });
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        // Last chance to keep what this session did: everything since the last
        // turn ended is only in memory until here.
        self.remember();
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
    // `no0b --set theme=amber` edits the settings file and exits. Not an
    // editor: it is a terminal one-liner for the same file, and the only
    // caller of the writer.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(request) = config::set_request(&args) {
        let written = request.and_then(|(key, value)| {
            let path = config::path().ok_or_else(|| "no settings file to write".to_string())?;
            // Write the commented default first when there is no file yet, or
            // the first `--set` on a fresh install leaves a two-line file and
            // the documentation never gets written at all.
            let _ = Config::load();
            config::write_setting(&path, key, Some(value))
                .map(|()| format!("{key} = {value} in {}", path.display()))
        });
        match written {
            Ok(line) => println!("{BINARY}: {line}"),
            Err(error) => {
                eprintln!("{BINARY}: {error}");
                std::process::exit(1);
            }
        }
        return;
    }

    let config = Config::load();
    let event_loop = match EventLoop::<Wake>::with_user_event().build() {
        Ok(loop_) => loop_,
        Err(e) => {
            eprintln!("{BINARY}: no display: {e}");
            std::process::exit(1);
        }
    };
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(event_loop.create_proxy(), config, workspace_arg(&args));
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("{BINARY}: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shading has to drop the minimum inner size. It did not, so the
    /// compositor clamped the 30 pixel request back up to 380 and the window
    /// stayed tall behind a title strip.
    #[test]
    fn shading_drops_the_minimum_and_unshading_puts_it_back() {
        let open = PhysicalSize::new(1180, 760);

        let (min, size) = shade_request(true, Some(open));
        assert_eq!(min, None, "a minimum taller than the strip refuses it");
        assert_eq!(size, Some(PhysicalSize::new(1180, view::TITLE_H as u32)));

        let (min, size) = shade_request(false, Some(open));
        assert_eq!(min, Some(MIN_SIZE));
        assert_eq!(size, Some(open), "and it goes back to the size it was");
    }

    /// The animation clock exists while a turn is running and at no other time.
    /// A deadline that outlives the turn is a window animating with nothing to
    /// animate, which is the 3,500 frames a second `noob-gpu` warns about.
    #[test]
    fn the_orb_clock_exists_only_while_a_turn_is_running() {
        let now = Instant::now();
        assert_eq!(orb_deadline(now, false, None), None, "nothing running, no clock");
        assert_eq!(
            orb_deadline(now, false, Some(now + ORB_EVERY)),
            None,
            "the turn ending drops the deadline it was holding"
        );

        let first = orb_deadline(now, true, None).expect("a running turn animates");
        assert_eq!(first, now + ORB_EVERY);
        // Asked for and not due yet: the same deadline, so nothing is redrawn in
        // between however many events arrive.
        assert_eq!(orb_deadline(now, true, Some(first)), Some(first));
        // Due: a new one, and a new one is what marks the window dirty.
        let past = first + Duration::from_millis(1);
        assert_eq!(orb_deadline(past, true, Some(first)), Some(past + ORB_EVERY));
    }

    /// Every pane that reads the monitor holds the sampling clock, the two token
    /// ones included: they are sampled out of the state rather than read from it
    /// at draw time, so a pane missing from [`SAMPLED`] would sit on the numbers
    /// it opened with.
    #[test]
    fn the_sampling_clock_runs_for_every_pane_that_reads_the_monitor() {
        for view in [View::Hardware, View::Context, View::Session] {
            let mut dock = Dock::new();
            dock.reveal(view);
            assert!(sampling(false, false, &dock), "{view:?} is not sampled");
            // Covered is not on screen: a shaded window is a title strip, and
            // the picker is a full takeover.
            assert!(!sampling(true, false, &dock), "{view:?} while shaded");
            assert!(!sampling(false, true, &dock), "{view:?} behind the picker");
            // Folded away is not on screen either.
            let space = Space::ALL
                .into_iter()
                .find(|space| dock.slot(*space).active() == Some(view))
                .expect("the revealed view is showing somewhere");
            for other in Space::ALL {
                dock.slot_mut(other).folded = true;
            }
            assert!(!sampling(false, false, &dock), "{view:?} folded away");
            dock.slot_mut(space).folded = false;
            assert!(sampling(false, false, &dock), "{view:?} unfolded again");
        }
        // And a window showing none of them costs nothing.
        let mut dock = Dock::new();
        for space in Space::ALL {
            let slot = dock.slot_mut(space);
            slot.views = vec![View::Output];
            slot.show(View::Output);
            slot.folded = false;
        }
        assert!(!sampling(false, false, &dock), "no monitor is on screen");
    }

    /// Two clocks, one control flow. Whichever is due first wins and the other
    /// keeps its deadline: assigning instead of composing is how the monitor
    /// stops sampling as soon as the orb starts turning.
    #[test]
    fn the_monitor_and_the_orb_compose_into_one_deadline() {
        let now = Instant::now();
        let (sample, orb) = (now + SAMPLE_EVERY, now + ORB_EVERY);
        assert!(ORB_EVERY < SAMPLE_EVERY, "the orb is the faster clock");
        assert_eq!(soonest([Some(sample), Some(orb)]), Some(orb));
        assert_eq!(soonest([Some(orb), Some(sample)]), Some(orb));
        assert_eq!(soonest([None, Some(sample)]), Some(sample));
        assert_eq!(soonest([Some(orb), None]), Some(orb));
        assert_eq!(soonest([None, None]), None, "an idle window blocks");
    }

    const W: f32 = 1400.0;
    const H: f32 = 900.0;
    const COLUMN: f32 = 8.0;
    const SIZE: f32 = 14.0;

    fn laid_out<'a>(dock: &'a Dock, menu: Option<&'a Menu>, chars: usize) -> Layout {
        let shape = Shape {
            shaded: false,
            dock,
            menu,
            picker: None,
            file_labels: Vec::new(),
            file_first: 0,
            column: COLUMN,
            pane_size: Config::default().pane_font_size,
            pane_column: COLUMN,
            input_h: view::input_height(
                W,
                COLUMN,
                chars,
                noob_draw::Text::line_for(SIZE),
                Config::default().max_input_rows,
            ),
        };
        Layout::compute(W, H, &shape)
    }

    fn middle(panel: noob_draw::Panel) -> (f32, f32) {
        (panel.x + panel.w * 0.5, panel.y + panel.h * 0.5)
    }

    fn opened(layout: &Layout, dock: &Dock, at: (f32, f32)) -> Option<Menu> {
        menu_for(layout.hit(at.0, at.1), at, dock, false, None)
    }

    /// A right click has to land on the menu for the thing under it, and on no
    /// menu at all where there is nothing a menu could act on.
    #[test]
    fn a_right_click_opens_the_menu_for_what_is_under_it() {
        let dock = Dock::new();
        let layout = laid_out(&dock, None, 0);

        let menu = opened(&layout, &dock, middle(layout.input)).expect("the prompt has a menu");
        assert_eq!(menu.target, Target::Input);
        assert_eq!(menu.rows.len(), 2);
        assert_eq!(menu.pick(1), Some(Item::Paste));

        // A tab, and the pane it names, are the same widget.
        let (view, tab) = layout.placed(Space::TopRight).tabs[1];
        let menu = opened(&layout, &dock, middle(tab)).expect("a tab has a menu");
        assert_eq!(menu.target, Target::Widget(view, Space::TopRight));
        assert_eq!(menu.pick(2), Some(Item::Close));

        let showing = dock.slot(Space::Left).active().unwrap();
        let menu = opened(&layout, &dock, middle(layout.placed(Space::Left).body))
            .expect("a pane has a menu");
        assert_eq!(menu.target, Target::Widget(showing, Space::Left));

        // Nothing a menu could act on.
        for at in [middle(layout.close), (400.0, 8.0)] {
            assert!(opened(&layout, &dock, at).is_none(), "at {at:?}");
        }
        // And the open menu itself: the second right click puts it away rather
        // than opening a menu for what it covers.
        let menu = Menu::for_widget((500.0, 400.0), View::Plan, Space::TopRight, false);
        let over = laid_out(&dock, Some(&menu), 0);
        let at = middle(over.menu_rows[0]);
        assert!(opened(&over, &dock, at).is_none());
    }

    /// The copy row belongs to the pane the menu opened over. A selection in
    /// some other pane must not light it up, because copying would then hand
    /// over text from a pane nobody pointed at.
    #[test]
    fn the_copy_row_reads_the_selection_of_its_own_pane() {
        let dock = Dock::new();
        let layout = laid_out(&dock, None, 0);
        let (view, tab) = layout.placed(Space::TopRight).tabs[0];
        let at = middle(tab);
        let hit = layout.hit(at.0, at.1);

        let mine = menu_for(hit, at, &dock, false, Some(view)).unwrap();
        assert_eq!(mine.pick(1), Some(Item::CopySelection));
        let elsewhere = menu_for(hit, at, &dock, false, Some(View::Output)).unwrap();
        assert_eq!(elsewhere.pick(1), None);
        assert_eq!(
            mine.rows.len(),
            elsewhere.rows.len(),
            "the menu is the same shape either way"
        );

        // The prompt's own copy row reads the prompt's selection.
        let at = middle(layout.input);
        let hit = layout.hit(at.0, at.1);
        assert_eq!(
            menu_for(hit, at, &dock, true, None).unwrap().pick(0),
            Some(Item::Copy)
        );
        assert_eq!(menu_for(hit, at, &dock, false, None).unwrap().pick(0), None);
    }

    /// Dropped on a space a tab moves; dropped off the window it is closed, the
    /// same as picking Close; dropped on neither it stays where it was.
    #[test]
    fn a_tab_dropped_off_the_window_is_closed_rather_than_moved() {
        let mut dock = Dock::new();
        assert!(land(&mut dock, View::Files, Landing::In(Space::Left)));
        assert_eq!(dock.space_of(View::Files), Some(Space::Left));

        let before = dock.clone();
        assert!(!land(&mut dock, View::Files, Landing::Nowhere));
        assert_eq!(dock, before, "a release on nothing changes nothing");

        assert!(land(&mut dock, View::Files, Landing::Out));
        assert!(dock.is_hidden(View::Files));
        assert_eq!(dock.space_of(View::Files), None);
        assert!(!dock.walk().contains(&View::Files));
        assert!(
            !land(&mut dock, View::Files, Landing::Out),
            "and throwing it out twice is not two hidden entries"
        );
        // A view that is out stays out until something unhides it.
        assert!(!land(&mut dock, View::Files, Landing::In(Space::Left)));
    }

    /// The whole pointer path for a selection in the prompt: two pixel
    /// positions become two caret offsets, and what is between them is what a
    /// copy would take.
    #[test]
    fn dragging_in_the_prompt_selects_the_span_the_pointer_crossed() {
        let dock = Dock::new();
        let mut prompt = Prompt::default();
        prompt.insert("select me please");
        let layout = laid_out(&dock, None, prompt.len());
        let y = layout.input.y + layout.input.h * 0.5;
        let chars = prompt.len();
        let caret = |x: f32| layout.input_caret(x, y, SIZE, COLUMN, chars);
        // The pixel that resolves to a given offset, found by asking the layout
        // rather than by working out where the prompt marker ends.
        let x_of = |want: usize| {
            (0..W as usize)
                .map(|x| x as f32)
                .find(|x| caret(*x) == want)
                .unwrap_or_else(|| panic!("no pixel resolves to {want}"))
        };

        prompt.press(caret(x_of(3)));
        assert_eq!(prompt.selection(), None, "a press alone selects nothing");
        prompt.drag_to(caret(x_of(9)));
        assert_eq!(prompt.selected().as_deref(), Some("ect me"));

        // Back the other way, from the same press.
        prompt.drag_to(caret(x_of(0)));
        assert_eq!(prompt.selected().as_deref(), Some("sel"));
        assert_eq!(prompt.caret(), 0);

        // Off the right hand end stops at the end of the text.
        prompt.drag_to(caret(W - 1.0));
        assert_eq!(prompt.selected().as_deref(), Some("ect me please"));
    }

    /// The whole pointer path for the debug pane: a pixel inside the pane
    /// becomes a row, and that row becomes the failure whose arguments open.
    ///
    /// Both halves are the ones `App::open_failure_under_pointer` calls, driven
    /// here without a window: `Layout::cell` for the row, `State::debug_rows` for
    /// which failure that row belongs to.
    #[test]
    fn clicking_a_failed_call_opens_the_arguments_that_were_sent() {
        let mut state = State::new();
        for (id, name) in [("a", "bash"), ("b", "write")] {
            state.apply(noob_proto::Event::ToolStart {
                call_id: id.into(),
                name: name.into(),
                brief: format!("{name} something"),
                args: noob_proto::Value::Object(
                    [(String::from("which"), noob_proto::Value::String(id.into()))]
                        .into_iter()
                        .collect(),
                ),
            });
            state.apply(noob_proto::Event::ToolEnd {
                call_id: id.into(),
                summary: "no".into(),
                elapsed_ms: 1,
                error: Some(noob_proto::ToolError {
                    kind: "denied".into(),
                    code: None,
                    message: format!("{name} was refused"),
                    detail: None,
                    remedy: None,
                }),
            });
        }

        let mut dock = Dock::new();
        assert!(dock.reveal(View::Debug));
        let layout = laid_out(&dock, None, 0);
        let body = layout.placed(Space::BottomRight).body;
        let pane_size = Config::default().pane_font_size;
        let line = noob_draw::Text::line_for(pane_size);
        // The pane draws its rows from the top of its content box, so the second
        // failure is the third row down.
        let (x, y) = (body.x + 20.0, body.y + 9.0 + 2.5 * line);
        let (space, row, _) = layout
            .cell(x, y, pane_size, COLUMN)
            .expect("the pointer is over a pane");
        assert_eq!(space, Space::BottomRight);
        assert_eq!(row, 2);

        assert!(state.toggle_failure(row));
        assert_eq!(state.open_failure, Some(1));
        let rows: Vec<String> = state.debug_rows().into_iter().map(|r| r.text).collect();
        assert!(
            rows.iter().any(|text| text.contains("which = b")),
            "the arguments of the second failure are not shown: {rows:?}"
        );
        // A press on the count at the top is not a failure and opens nothing.
        let (_, row, _) = layout
            .cell(x, body.y + 9.0, pane_size, COLUMN)
            .expect("the first row");
        assert_eq!(row, 0);
        assert!(!state.toggle_failure(row));
        assert_eq!(state.open_failure, Some(1), "and it left the open one alone");
    }

    /// A folder on the command line is the workspace and skips the picker.
    /// Without one there is no workspace to fall back to: `current_dir()` under
    /// a desktop launcher is `$HOME`, which is the folder this stopped handing
    /// the agent by default.
    #[test]
    fn a_folder_on_the_command_line_is_the_one_that_opens() {
        let args = |list: &[&str]| -> Vec<String> {
            list.iter().map(|s| s.to_string()).collect()
        };
        assert_eq!(
            workspace_arg(&args(&["/home/hec/workspace/noob-cli"])),
            Some(PathBuf::from("/home/hec/workspace/noob-cli"))
        );
        assert_eq!(workspace_arg(&args(&[])), None, "the picker opens");
        assert_eq!(
            workspace_arg(&args(&["--anything"])),
            None,
            "a flag is not a folder"
        );
        // A flag before the folder still finds it, so the order arguments were
        // typed in does not decide whether the picker opens.
        assert_eq!(
            workspace_arg(&args(&["--flag", "code"])),
            Some(PathBuf::from("code"))
        );
    }

    /// The prompt is one line and Enter submits it, so a pasted newline cannot
    /// stay a newline. It has no glyph in any font, which would draw as nothing
    /// while still counting as a character.
    #[test]
    fn a_paste_arrives_as_one_line() {
        assert_eq!(pasted("cargo test\n"), "cargo test ");
        assert_eq!(pasted("one\r\n\ttwo"), "one   two");
        assert_eq!(pasted(""), "");
        assert_eq!(pasted("nothing to do"), "nothing to do");
    }
}
