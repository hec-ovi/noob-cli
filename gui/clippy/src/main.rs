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

mod agent;
mod commands;
mod config;
mod design;
mod dock;
mod install;
mod link;
mod menu;
mod monitor;
mod orb;
mod packaging;
mod picker;
mod prompt;
mod scroll;
mod select;
mod sessions;
mod settings;
mod state;
mod style;
#[allow(unused_imports)]
use style::{markdown, skin, syntax, table};
mod view;
mod widgets;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use noob_proto::Command as Cmd;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowId};

use config::Config;
use dock::{Dock, Space, View};
use link::{Incoming, Link};
use menu::{Item, Menu, Target};
use monitor::Monitor;
use picker::{Chosen, Picker};
use settings::Settings;
use prompt::Prompt;
use skin::Skin;
use state::{State, Tone};
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


/// How often the orb gets a new frame while a turn is running. Thirty a second
/// is enough for an orbit to read as motion and it costs 516 rectangles a frame,
/// which is one draw call.
const ORB_EVERY: Duration = Duration::from_millis(33);

/// How far the pointer has to move with a tab held before it counts as a drag
/// rather than a click that wobbled.
const DRAG_SLOP: f64 = 5.0;

/// How long a first ESC keeps the cancel armed. The same window the CLI's
/// dock gives its double-ESC, so the gesture is one habit across both.
const ESC_CANCEL_WINDOW: Duration = Duration::from_secs(5);

/// How long a sent prompt has to produce a turn before the window says it did
/// not.
///
/// `turn.start` leaves the agent the moment it takes the prompt, before the
/// endpoint is called at all, so this is a pipe's round trip and not a model's:
/// ten seconds is a long way past generous. What it protects against is the
/// prompt that never arrived, which otherwise leaves the orb turning over a
/// conversation that has stopped, with nothing anywhere saying so.
const ANSWER_WAIT: Duration = Duration::from_secs(10);


/// The window will not grow past this. Unbounded is not useful: a conversation
/// at four thousand pixels wide is one long line per paragraph, and the panes
/// stop being panes.
const MAX_SIZE: LogicalSize<f64> = LogicalSize::new(2200.0, 1400.0);
const MIN_SIZE: LogicalSize<f64> = LogicalSize::new(680.0, 380.0);











/// How long the orb takes to travel between its resting square and its turning
/// circles.
///
/// Nine frames at [`ORB_EVERY`]: long enough to see the dots move into place and
/// short enough that the first thing the agent says does not arrive halfway
/// through it.
const ORB_MORPH: Duration = Duration::from_millis(300);


impl Morph {
    fn new(now: Instant) -> Self {
        Morph {
            at: 0.0,
            from: 0.0,
            since: now,
            busy: false,
        }
    }

    /// Where the orb is after `since` has passed, travelling towards the end
    /// `busy` names. Clamped at both ends, which is what makes the clock this
    /// holds open finite.
    fn travelled(from: f32, busy: bool, since: Duration) -> f32 {
        let gone = since.as_secs_f32() / ORB_MORPH.as_secs_f32();
        match busy {
            true => (from + gone).min(1.0),
            false => (from - gone).max(0.0),
        }
    }

    fn step(&mut self, busy: bool, now: Instant) {
        if busy != self.busy {
            self.from = self.at;
            self.since = now;
            self.busy = busy;
        }
        self.at = Self::travelled(self.from, busy, now.saturating_duration_since(self.since));
    }

    /// What the scene is handed: the progress while the orb is moving, and
    /// `None` once it has arrived, which is every frame outside a transition and
    /// so nearly every frame the window draws.
    fn showing(&self) -> Option<f32> {
        let settled = match self.busy {
            true => 1.0,
            false => 0.0,
        };
        (self.at != settled).then_some(self.at)
    }

    /// Whether the orb still wants frames with no turn to animate. Only true on
    /// the way back from one, and only until it gets there.
    fn moving(&self) -> bool {
        self.at > 0.0
    }
}


















/// What a key held with control does while the settings panel is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Quit,
    /// Put what is highlighted in the document on the clipboard.
    Copy,
    /// Mark every conversation on the table, which is what Ctrl-A means in
    /// every list anybody has ever used.
    MarkAll,
    /// Write the system prompt's document from its editor, which is what
    /// Ctrl-S means in every editor anybody has ever used. Enter cannot be
    /// the save there, because Enter is how a line breaks.
    Save,
    Nothing,
}


/// What a key that moves the settings panel itself, rather than the row under
/// the cursor, is asking for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Walk {
    /// On to the next section, or back to the one before it.
    Section(bool),
    /// Across a form row, to the half the arrow points at.
    Cross(settings::Side),
}





struct App {
    window: Option<Arc<Window>>,
    gpu: Option<noob_gpu::Gpu>,
    renderer: Option<noob_draw::Renderer>,
    proxy: EventLoopProxy<Wake>,

    config: Config,
    state: State,
    /// Where each list pane is scrolled to. Shell-owned: scrolling is a
    /// window question, and the reducer stays a function of the stream.
    scrolls: scroll::Scrolls,
    /// A drag over one of the text panes, if there is one.
    selection: Option<select::Selection>,
    /// The explorer's first visible row, top-anchored.
    file_scroll: usize,
    /// The open file at the last look, so a change can drop that pane's
    /// selection and reveal the row wherever the change came from.
    last_open_file: usize,
    monitor: Monitor,
    next_sample: Option<Instant>,
    /// When the orb wants its next frame, and `None` whenever it is still.
    ///
    /// The one animation clock in the window. It exists only while a turn is
    /// running or the orb is still travelling back from one, which is what keeps
    /// a window nobody is talking to at zero frames a second. See
    /// [`orb_deadline`].
    next_orb: Option<Instant>,
    /// How far the orb is between its resting square and its turning circles.
    orb: Morph,
    skin: Skin,
    link: Option<Link>,
    trouble: Option<String>,
    /// The call popup's first visible content row. Reset when a call opens;
    /// clamped against the popup's own extent on every move.
    popup_scroll: usize,
    /// Armed until this instant by a first ESC that had nothing left to drop.
    /// A second ESC inside the window cancels the turn; any other key, or the
    /// window lapsing, disarms. One tap was too easy to spend by accident: a
    /// key meant for a menu or a selection that had already gone cost a turn.
    esc_armed: Option<Instant>,
    /// When the prompt that was just sent stops being given the benefit of the
    /// doubt. Cleared by the turn it starts. See [`ANSWER_WAIT`].
    answer_by: Option<Instant>,

    /// The folder named on the command line, until it has been connected to.
    /// Taken in `resumed`, because a folder given up front skips the picker.
    workspace: Option<PathBuf>,
    /// The session the agent says it is running, and the folder it is running
    /// in, from the one frame that names both. Kept so the note written for it
    /// can be updated later in the session with how full its context window
    /// got: the frames that carry that figure do not say which session they
    /// belong to.
    session: Option<(String, String)>,
    /// The folder picker, while it is up. Nothing else in the window is live
    /// while it is: there is no agent until it closes.
    picker: Option<Picker>,
    /// The settings panel, while it is up. A takeover, so while it is here the
    /// keyboard and the pointer belong to it and the panes are not drawn; the
    /// agent behind it keeps running, and what it says arrives when the panel
    /// closes.
    settings: Option<Settings>,

    prompt: Prompt,
    dock: Dock,
    /// The open right click menu, or nothing. Held here rather than in the
    /// layout because it outlives a frame: it stays up until a row is picked or
    /// something puts it away.
    menu: Option<Menu>,
    /// A tab that has been pressed, and whether the pointer has moved far
    /// enough since to call it a drag rather than a click.
    holding: Option<(View, Space, PhysicalPosition<f64>)>,
    /// The divider the button came down on, while it is being dragged. The same
    /// press, motion, release cycle the tabs use, and only one of the two can be
    /// running at a time because a press lands on exactly one thing.
    ///
    /// Only ever a [`Hit::ColumnDivider`] or a [`Hit::RowDivider`], and the
    /// half of the grid it carries says which of the two lines on that axis is
    /// moving.
    sizing: Option<Hit>,
    /// The scrollbar the button came down on, while it is being dragged.
    /// The same press, motion, release cycle a divider runs.
    thumbing: Option<Thumb>,
    /// The settings row and half whose slider the button came down on, while it
    /// is being dragged. The same cycle again, on the panel: the value follows
    /// the pointer and the file is written once, when the button comes up.
    sliding: Option<(usize, settings::Side)>,
    /// The thread reading the prompt's environment tail out of the CLI, while
    /// it is still reading it. Dropped as soon as it answers, so a panel
    /// opened twice does not take the first run's answer for the second one's.
    asking: Option<std::sync::mpsc::Receiver<link::Asked>>,
    /// The thread asking the CLI whether the endpoint answers, while it is
    /// still asking. Dropped the moment it does, for the same reason.
    checking: Option<std::sync::mpsc::Receiver<(bool, Option<String>)>>,
    /// The thread installing a skill, while it is still installing it: the
    /// source it was given, and the name it landed under or why it did not.
    ///
    /// Its own thread because a clone is given two minutes and the interface
    /// is one thread. Some, so a second press while one is running is refused
    /// rather than starting a race between two installs for one directory.
    installing: Option<std::sync::mpsc::Receiver<(String, Result<String, String>)>>,
    /// Whether the running install was asked for by a /command, so its
    /// answer goes to the transcript rather than only to a panel that may
    /// not even be open.
    install_from_command: bool,
    /// Where the dividers are, as fractions: how much of the width the left
    /// column takes in each row, and how much of the height the top space takes
    /// in each column. Read out of the settings file at launch and written back
    /// when a drag ends.
    ///
    /// A pair each because each half of the grid breaks where it was dragged to,
    /// and the half whose line is not on screen keeps its number so turning the
    /// grid round finds it again.
    left_width: [f32; 2],
    top_height: [f32; 2],
    /// The same for the settings panel's rail: how much of the panel the column
    /// of section names takes, dragged while the panel is up and written back
    /// when the drag ends.
    settings_rail: f32,
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
    /// Whether the window was maximized when it was shaded, so opening the strip
    /// can put it back. Shading a maximized window has to un-maximize it first,
    /// because a maximized window ignores a resize request.
    was_maximized: bool,
    /// True while a shade is waiting on a window to leave maximized. See
    /// [`shade_of`]: what a window says about its size across that round trip is
    /// our own request being worked through rather than evidence about what the
    /// window is.
    settling: bool,
    /// Where the button went down on the title bar, while it is still down.
    /// The compositor is only handed an interactive move once the pointer has
    /// moved away from here; see [`began_move`].
    moving: Option<PhysicalPosition<f64>>,
    column: f32,
    pane_column: f32,
    /// One column at the size a menu's rows are written at, which is not the
    /// size anything else in the window is written at. See [`menu::paint::MENU_SIZE`].
    menu_column: f32,

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
        // Every activity row says what time it happened, which takes the
        // reader's clock as well as the monotonic one the rates are measured
        // on. Asked once here and never again: the frames carry their own
        // monotonic second, and a time of day is that second added to this.
        let mut state = State::new();
        state.day_zero = local_day_second();
        let epoch = Instant::now();
        // The saved arrangement wins over the two switches when it parses;
        // a hand-broken line falls back whole rather than half-building.
        let dock = config
            .dock
            .as_deref()
            .and_then(Dock::from_arrangement)
            .unwrap_or_else(|| Dock::hiding(&hidden));
        App {
            dock,
            left_width: [config.left_width, config.left_width_bottom],
            top_height: [config.top_height, config.top_height_right],
            settings_rail: config.settings_rail,
            window: None,
            gpu: None,
            renderer: None,
            proxy,
            config,
            state,
            scrolls: scroll::Scrolls::default(),
            selection: None,
            file_scroll: 0,
            last_open_file: 0,
            monitor: Monitor::new(),
            next_sample: None,
            next_orb: None,
            orb: Morph::new(epoch),
            skin,
            link: None,
            trouble: None,
            esc_armed: None,
            answer_by: None,
            popup_scroll: 0,
            workspace,
            session: None,
            picker: None,
            settings: None,
            prompt: Prompt::default(),
            menu: None,
            holding: None,
            sizing: None,
            thumbing: None,
            sliding: None,
            asking: None,
            checking: None,
            installing: None,
            install_from_command: false,
            selecting: false,
            prompt_selecting: false,
            clipboard: None,
            drag: None,
            shaded: false,
            unshaded: None,
            was_maximized: false,
            settling: false,
            moving: None,
            column: 8.0,
            pane_column: 8.0,
            menu_column: 7.0,
            cursor: PhysicalPosition::new(0.0, 0.0),
            hot: None,
            last_click: None,
            modifiers: ModifiersState::empty(),
            epoch,
            dirty: true,
        }
    }

    /// How far into this window's life it is, in the monotonic seconds every
    /// frame is folded in with. What a row of the activity list is stamped
    /// with, whether an agent frame or the window's own note caused it.
    fn now(&self) -> Option<f64> {
        Some(self.epoch.elapsed().as_secs_f64())
    }

    fn shape(&self) -> Shape<'_> {
        Shape {
            shaded: self.shaded,
            dock: &self.dock,
            menu: self.menu.as_ref(),
            picker: self.picker.as_ref(),
            settings: self.settings.as_ref(),
            file_labels: self
                .state
                .files
                .iter()
                .map(|file| view::short_name(&file.path))
                .collect(),
            file_first: self.file_scroll,
            agent_tab: self.state.shown_agent,
            column: self.column,
            menu_column: self.menu_column,
            pane_size: self.config.pane_font_size,
            pane_column: self.pane_column,
            input_h: self.input_height(),
            left_width: self.left_width,
            top_height: self.top_height,
            settings_rail: self.settings_rail,
            popup: self.state.popped(),
        }
    }

    /// How tall the prompt strip is. The rows the settings ask for, whether or
    /// not anything has been typed into them.
    fn input_height(&self) -> f32 {
        view::input_height(
            self.config.prompt_rows,
            noob_draw::Text::line_for(self.config.font_size),
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

    /// Start the agent on what was chosen: a folder, and the session to carry on
    /// in it when the row was a saved one. A failure is shown in the window
    /// rather than printed to a terminal nobody is watching.
    ///
    /// One argument for both, so the id cannot be dropped on the way from the
    /// row that was pressed to the process that is started. `serve` takes
    /// `--resume <id>` and replays the transcript itself; nothing here reads it.
    ///
    /// Re-entrant: the picker calls it after the window is already up, so an
    /// agent from an earlier call has to be let go first. Nothing clears the
    /// transcript, because the only way here twice is through the picker before
    /// a turn has been taken and what is in it is the picker's own messages.
    fn connect(&mut self, chosen: Chosen) {
        if let Some(mut link) = self.link.take() {
            link.shutdown();
        }
        self.trouble = None;
        let program = std::env::var("NOOB_BIN").unwrap_or_else(|_| String::from("noob"));
        let proxy = self.proxy.clone();
        match Link::spawn(
            &program,
            &chosen.workspace,
            chosen.session.as_deref(),
            &agent::OWNED,
            move || {
                let _ = proxy.send_event(Wake);
            },
        ) {
            Ok(link) => {
                self.link = Some(link);
                self.state.workspace = chosen.workspace.display().to_string();
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

    /// A row was chosen: remember its folder and start the agent on it.
    ///
    /// A resumed session remembers its folder too. It is a folder you have just
    /// said you are working in, which is the whole of what the list is for.
    fn choose(&mut self, chosen: Chosen) {
        self.picker = None;
        if let Some(file) = picker::recents_path() {
            // Read again immediately before writing, rather than reusing what
            // the picker opened with, so a second window that chose a folder in
            // the meantime does not have its entry erased.
            let list = picker::remember(&picker::load_recents(&file), &chosen.workspace);
            // A recents file that cannot be written costs the next launch one
            // keystroke. Not worth a line in a conversation that is about to
            // start.
            let _ = picker::save_recents(&file, &list);
        }
        self.connect(chosen);
    }

    /// Ctrl-R: the saved sessions, or back to the folders.
    ///
    /// A toggle because it is one key. The two buttons in the picker's head are
    /// not toggles: each of them names the list it puts there, so pressing the
    /// one already showing has to do nothing.
    fn toggle_sessions(&mut self) {
        match self.picker.as_ref().is_some_and(Picker::on_sessions) {
            true => self.show_folders_now(),
            false => self.show_sessions_now(),
        }
    }

    /// The Sessions button: the saved sessions, whatever is showing now.
    ///
    /// The reading happens here rather than in the picker because this is the
    /// half of the window that is allowed to touch the disk, and it happens on
    /// the press rather than when the window opens: a machine with a year of
    /// sessions on it should not pay for the list nobody asked to see. Already
    /// on them, nothing is read at all: the press is a no-op, not a reload.
    fn show_sessions_now(&mut self) {
        if self.picker.as_ref().is_some_and(Picker::on_sessions) {
            return;
        }
        let listing = self.saved_sessions();
        if let Some(picker) = self.picker.as_mut() {
            picker.show_sessions(listing);
        }
        self.dirty = true;
        self.reveal_picker_cursor();
    }

    /// The Folders button, and what Escape falls back to. Nothing is read off
    /// the disk to go back: the tree is still in the picker.
    fn show_folders_now(&mut self) {
        if let Some(picker) = self.picker.as_mut() {
            picker.show_folders();
        }
        self.dirty = true;
        self.reveal_picker_cursor();
    }

    /// Every session on disk, with the folder each one was started in.
    ///
    /// Nowhere to read from is an empty list rather than a refusal: that is a
    /// machine with no home directory, where there are no sessions either.
    fn saved_sessions(&self) -> sessions::Listing {
        let index = sessions::index_path()
            .map(|path| sessions::load_index(&path))
            .unwrap_or_default();
        match sessions::dir() {
            Some(dir) => sessions::read(&dir, &index, &picker::Disk),
            None => sessions::Listing::default(),
        }
    }

    /// Write down which folder a session was started in.
    ///
    /// The transcript the agent writes does not record it, so this is the only
    /// note anywhere that ties the two together, and without it a session in the
    /// list would have nowhere to be resumed. On the frame that says a session
    /// has started, which is once per agent.
    fn remember_session(&self, id: &str, workspace: &str) {
        let Some(path) = sessions::index_path() else {
            return;
        };
        if id.is_empty() || workspace.is_empty() {
            return;
        }
        // Read again immediately before writing, so a second window that
        // started a session in the meantime keeps its note.
        let index = sessions::load_index(&path).plus(id, Path::new(workspace));
        // A note that cannot be written costs a row in the session list its
        // folder. Not worth a line in a conversation that is starting.
        let _ = sessions::save_index(&path, &index);
    }

    /// Write down how full this session's context window is, on the note that
    /// already says which folder it belongs to.
    ///
    /// The session list has no other way to know. The transcript records what
    /// each request spent, not what the window was holding, and summing those
    /// deltas would mean reading every byte of every session file to draw one
    /// column. This is the figure the running window already has, written once
    /// per turn beside the folder, which means a session shows a reading from
    /// the moment a window has watched it run and a dash before that. Sessions
    /// written by the CLI on its own never get one, exactly as they never get a
    /// folder.
    fn remember_context(&self) {
        let Some((id, workspace)) = self.session.as_ref() else {
            return;
        };
        let Some(context) = context_reading(self.state.context, self.state.usage) else {
            return;
        };
        let Some(path) = sessions::index_path() else {
            return;
        };
        if id.is_empty() || workspace.is_empty() {
            return;
        }
        // Read again immediately before writing, the same as the folder note:
        // a second window that started a session in the meantime keeps its own.
        let index = sessions::load_index(&path).plus_context(id, Path::new(workspace), context);
        let _ = sessions::save_index(&path, &index);
    }

    /// Open the settings panel over the window.
    fn open_settings(&mut self) {
        self.settings = Some(Settings::open(
            &self.config,
            self.settings_path().as_deref(),
            self.read_agent(),
        ));
        self.ask_for_env();
        self.dirty = true;
    }

    /// Ask the CLI what the prompt's environment tail is, on a thread of its
    /// own.
    ///
    /// The block shows what the agent's prompt really ends in, and the only
    /// thing that knows that is the CLI: `noob debug env` prints the tail a
    /// session sends. It reads a config directory, a workspace and every
    /// skill on the machine, so it runs off the interface thread and wakes
    /// the event loop when it answers, the same way the agent's own output
    /// arrives.
    fn ask_for_env(&mut self) {
        let program = std::env::var("NOOB_BIN").unwrap_or_else(|_| String::from("noob"));
        // The folder the session is running in, since the project's own
        // AGENTS.md, its skills and its mcp.json are all found relative to
        // it. A window with no folder open yet asks in the one the process is
        // in, which is where a session started now would run.
        let workspace = match self.state.workspace.is_empty() {
            false => PathBuf::from(&self.state.workspace),
            true => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.asking = Some(rx);
        let proxy = self.proxy.clone();
        let at = workspace.display().to_string();
        std::thread::spawn(move || {
            let answer = match link::env_command(&program, &workspace, &agent::OWNED).output() {
                Ok(out) => link::env_from(out.status.success(), &out.stdout, &out.stderr),
                Err(e) => Err(format!(
                    "cannot run {program:?}: {e}; is noob on PATH, or set NOOB_BIN"
                )),
            };
            let _ = tx.send((at, answer));
            let _ = proxy.send_event(Wake);
        });
    }

    /// Answer the connection card's button: write whatever endpoint is being
    /// typed, then ask the CLI whether it answers.
    ///
    /// The write first, because the point of pressing it after typing an
    /// address is to check that address; leaving the buffer open would check
    /// the old one and read as a button that does nothing. Then `noob doctor`
    /// on a thread of its own, since it opens a socket and this is the
    /// interface thread.
    fn check_connection(&mut self) {
        if let Some(path) = self
            .settings
            .as_ref()
            .and_then(Settings::agent_file)
            .map(std::path::Path::to_path_buf)
            && let Some((key, value)) = self.settings.as_mut().and_then(Settings::finish_edit)
        {
            self.write_agent_setting(&path, key, &value);
        }
        if let Some(panel) = self.settings.as_mut() {
            let config = self.config.clone();
            panel.adopt_health(String::from(settings::ASKING), &config);
        }
        let program = std::env::var("NOOB_BIN").unwrap_or_else(|_| String::from("noob"));
        let workspace = match self.state.workspace.is_empty() {
            false => PathBuf::from(&self.state.workspace),
            true => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.checking = Some(rx);
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            // Two words on the card and the reason on the footer: a card that
            // answers "can you reach the model" with a paragraph about HTTP
            // is answering a question nobody asked.
            let answer = match link::doctor_command(&program, &workspace, &agent::OWNED).output() {
                Ok(out) => (link::online_from(&out.stdout), None),
                Err(e) => (
                    false,
                    Some(format!(
                        "cannot run {program:?}: {e}; is noob on PATH, or set NOOB_BIN"
                    )),
                ),
            };
            let _ = tx.send(answer);
            let _ = proxy.send_event(Wake);
        });
        self.dirty = true;
    }

    /// Answer the restore under the connection card: the endpoint goes back
    /// to the address the CLI would have autodetected, through the same write
    /// the field itself goes through.
    fn default_endpoint(&mut self) {
        let Some(path) = self
            .settings
            .as_ref()
            .and_then(Settings::agent_file)
            .map(std::path::Path::to_path_buf)
        else {
            if let Some(panel) = self.settings.as_mut() {
                panel.say_trouble(String::from("there is no config directory to write it in"));
            }
            self.dirty = true;
            return;
        };
        self.write_agent_setting(&path, agent::ENDPOINT, agent::ENDPOINT_DEFAULT);
    }

    /// Take what the connection check answered, if it has.
    fn take_health(&mut self) {
        let Some(rx) = self.checking.as_ref() else {
            return;
        };
        let Ok((online, trouble)) = rx.try_recv() else {
            return;
        };
        self.checking = None;
        let config = self.config.clone();
        if let Some(panel) = self.settings.as_mut() {
            let word = match online {
                true => settings::ONLINE,
                false => settings::OFFLINE,
            };
            panel.adopt_health(String::from(word), &config);
            // A check that could not be run at all is still offline on the
            // card; why it could not is the footer's, where every other
            // reason on this panel goes.
            if let Some(why) = trouble {
                panel.say_trouble(why);
            }
        }
        self.dirty = true;
    }

    /// Take what that thread answered, if it has.
    fn take_env(&mut self) {
        let Some(rx) = self.asking.as_ref() else {
            return;
        };
        let Ok((at, answer)) = rx.try_recv() else {
            return;
        };
        self.asking = None;
        let config = self.config.clone();
        if let Some(panel) = self.settings.as_mut() {
            panel.adopt_env(at, answer, &config);
        }
        self.dirty = true;
    }

    /// Install what has been typed into the skills section, on a thread of its
    /// own.
    ///
    /// Never through [`App::do_deed`], which is synchronous: a clone is given
    /// two minutes and a window frozen for two minutes is a window that has
    /// crashed as far as anybody watching it is concerned. A thread, a
    /// channel, and a wake of the event loop when it answers.
    /// Answer the validate press: what the typed source would install, or
    /// why it would not, said under the card. Synchronous on purpose: the
    /// check reads a string and at most a local directory.
    fn validate_source(&mut self) {
        let config = self.config.clone();
        let Some(panel) = self.settings.as_mut() else {
            return;
        };
        let source = panel.take_source();
        let verdict = install::check(&source);
        panel.note_check(source, verdict, &config);
        self.dirty = true;
    }

    fn start_install(&mut self) {
        let already = self.installing.is_some();
        let (source, skills_at) = match self.settings.as_mut() {
            Some(panel) => (panel.take_source(), panel.skills_at().map(Path::to_path_buf)),
            None => return,
        };
        self.dirty = true;
        // Every refusal is said on the panel rather than dropped: a button that
        // answers a press with nothing is a button that reads as broken.
        let trouble = match (already, source.is_empty(), &skills_at) {
            (true, ..) => Some(String::from(
                "an install is already running: wait for it to answer",
            )),
            (_, true, _) => Some(String::from(
                "type a repository, an owner/name or a path first",
            )),
            (_, _, None) => Some(String::from(
                "there is no config directory to install a skill into",
            )),
            _ => None,
        };
        if let Some(why) = trouble {
            if let Some(panel) = self.settings.as_mut() {
                panel.say_trouble(why);
            }
            return;
        }
        let Some(skills_at) = skills_at else {
            return;
        };
        let config = self.config.clone();
        if let Some(panel) = self.settings.as_mut() {
            panel.begin_install(source.clone(), &config);
        }
        self.spawn_install(source, skills_at);
    }

    /// The clone itself, off the interface thread: the shared tail of the
    /// panel's install button and /skill_install. The answer comes back
    /// through [`App::take_install`].
    fn spawn_install(&mut self, source: String, skills_at: PathBuf) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.installing = Some(rx);
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let answer = install::install(&source, &skills_at);
            let _ = tx.send((source, answer));
            let _ = proxy.send_event(Wake);
        });
    }

    /// Start the install a /skill_install asked for: the source has already
    /// been validated by the dispatcher, the same check the panel's button
    /// runs. The answer is reported in the transcript when it lands.
    fn command_install(&mut self, source: String, said: String) {
        if self.installing.is_some() {
            self.state
                .output
                .say("an install is already running: wait for it to answer", Tone::Bad);
            return;
        }
        let Some(skills_at) = agent::config_dir().map(|dir| dir.join("skills")) else {
            self.state
                .output
                .say("there is no config directory to install a skill into", Tone::Bad);
            return;
        };
        self.state.output.say(said, Tone::Dim);
        self.install_from_command = true;
        // The open panel says an install is running too, so the SKILLS
        // section and the transcript tell one story.
        let config = self.config.clone();
        if let Some(panel) = self.settings.as_mut() {
            panel.begin_install(source.clone(), &config);
        }
        self.spawn_install(source, skills_at);
    }

    /// Take what that thread answered, if it has, and read the disk back with
    /// it.
    ///
    /// The list is rebuilt from the reading and not from what the install said,
    /// which is the rule every write on this panel goes through: a skill is on
    /// the list because its directory is there.
    fn take_install(&mut self) {
        let Some(rx) = self.installing.as_ref() else {
            return;
        };
        let Ok((source, answer)) = rx.try_recv() else {
            return;
        };
        self.installing = None;
        let agent = self.read_agent();
        let config = self.config.clone();
        if let Some(panel) = self.settings.as_mut() {
            panel.adopt_install(source, answer.clone(), agent, &config);
        }
        // An install a /command started answers where it was asked for: in
        // the transcript, whether or not the panel is up to say it too.
        if self.install_from_command {
            self.install_from_command = false;
            match &answer {
                Ok(name) => self.state.output.say(
                    format!("installed {name}; the agent picks it up on its next session"),
                    Tone::Dim,
                ),
                Err(why) => {
                    for line in why.lines() {
                        self.state.output.say(line.to_string(), Tone::Bad);
                    }
                }
            }
        }
        self.dirty = true;
    }

    /// What the agent's own files say right now: its `.env`, the skills beside
    /// it, its MCP servers and the sessions it has written.
    ///
    /// Read here rather than in the panel because the sessions come off the same
    /// reader the picker uses and reading a disk is the window's job, not the
    /// model's. Once, when the panel opens, and again after the panel writes.
    fn read_agent(&self) -> agent::Agent {
        agent::Agent::read(
            agent::config_dir().as_deref(),
            match self.state.workspace.is_empty() {
                true => None,
                false => Some(Path::new(&self.state.workspace)),
            },
            self.saved_sessions(),
        )
    }

    /// Where the settings file is, or nothing when there is no home directory to
    /// put one in. The panel says so rather than failing at the first change.
    fn settings_path(&self) -> Option<std::path::PathBuf> {
        config::path()
    }

    /// Put the panel away. The panes come back exactly as they were: nothing
    /// about the window is held on the panel, only the file is.
    fn close_settings(&mut self) {
        if self.settings.take().is_some() {
            self.dirty = true;
        }
        // A highlight in the document goes with the panel that held it. Left
        // behind, it is what the next Ctrl-C would copy with nothing on screen
        // saying so, and the text it names is not even in the window any more.
        self.forget_doc_selection();
    }

    /// Keys while the settings panel is up. It is a takeover, so this answers for
    /// the whole keyboard: nothing here falls through to the prompt.
    fn key_in_settings(&mut self, event: &winit::event::KeyEvent, event_loop: &ActiveEventLoop) {
        // The two keys that mean the same thing everywhere in this window. Every
        // other key here belongs to the panel, so without this arm the panel
        // would be a surface Ctrl-Q could not be pressed on and Ctrl-C could not
        // copy off.
        if self.modifiers.control_key() {
            match control_in_settings(event.logical_key.as_ref()) {
                Control::Quit => event_loop.exit(),
                // What was highlighted in the document beside the list. The
                // panel covers the panes and the prompt, so there is nothing
                // else here for a copy to mean.
                Control::Copy => {
                    self.copy_selection();
                }
                // Every conversation on the list and not only the twelve the
                // table's body is showing. Nothing at all anywhere else on the
                // panel, which is what it did before.
                Control::MarkAll => {
                    if let Some(panel) = self.settings.as_mut() {
                        let at = panel.table_at_cursor().map(|(at, _)| at);
                        if let Some(at) = at {
                            self.dirty |= panel.mark_all(at, true);
                        }
                    }
                }
                // The document editor's save. With no editor open there is
                // nothing to write, and nothing happens.
                Control::Save => self.save_instructions(),
                Control::Nothing => {}
            }
            return;
        }
        // Any other key is about the rows, and the rows decide which document is
        // beside them. A highlight over the one that was showing does not
        // survive the cursor moving off it.
        self.forget_doc_selection();
        let rows = self.settings_rows();
        // Read before the panel is borrowed: shift is what tells the rail key
        // which way to walk and what takes the nudge off left and right.
        let shift = self.modifiers.shift_key();
        let Some(panel) = self.settings.as_mut() else {
            return;
        };
        // Typing into the endpoint takes the whole keyboard: the arrow keys
        // would otherwise walk away from a half typed URL and lose it.
        if panel.editing().is_some() {
            match event.logical_key.as_ref() {
                Key::Named(NamedKey::Escape) => self.dirty |= panel.cancel_edit(),
                Key::Named(NamedKey::Backspace) => self.dirty |= panel.backspace(),
                Key::Named(NamedKey::Enter) => self.save_endpoint(),
                _ => {
                    if let Some(text) = event.text.as_ref() {
                        self.dirty |= panel.type_text(text);
                    }
                }
            }
            self.dirty = true;
            return;
        }
        // The document editor takes the whole keyboard the same way: Enter is
        // a newline, the arrows are the caret, Escape abandons the edit with
        // the file untouched, and the save is Ctrl-S, answered above with the
        // other control keys.
        if panel.editing_instructions() {
            let config = self.config.clone();
            match event.logical_key.as_ref() {
                Key::Named(NamedKey::Escape) => self.dirty |= panel.cancel_instructions(&config),
                Key::Named(NamedKey::Backspace) => {
                    self.dirty |= panel.instructions_backspace(&config);
                }
                Key::Named(NamedKey::Enter) => self.dirty |= panel.instructions_newline(&config),
                Key::Named(NamedKey::ArrowUp) => {
                    self.dirty |= panel.instructions_step(false, &config);
                }
                Key::Named(NamedKey::ArrowDown) => {
                    self.dirty |= panel.instructions_step(true, &config);
                }
                Key::Named(NamedKey::ArrowLeft) => {
                    self.dirty |= panel.instructions_cross(false, &config);
                }
                Key::Named(NamedKey::ArrowRight) => {
                    self.dirty |= panel.instructions_cross(true, &config);
                }
                _ => {
                    if let Some(text) = event.text.as_ref() {
                        self.dirty |= panel.type_instructions(text, &config);
                    }
                }
            }
            self.reveal_settings_cursor();
            return;
        }
        // The two keys that move the panel rather than the row: the rail, and
        // the crossing of a form row. Answered first and on their own, because
        // both of them are arrow keys or a key an arrow key used to be, and the
        // arms below are all about the row under the cursor.
        if let Some(walk) = walk_in_settings(event.logical_key.as_ref(), shift) {
            self.dirty |= match walk {
                Walk::Section(forward) => panel.walk_section(forward),
                Walk::Cross(side) => panel.cross(side),
            };
            self.reveal_settings_cursor();
            return;
        }
        let mut nudge = None;
        let mut edit = false;
        let mut edit_doc = false;
        let mut flip = None;
        // Which conversation the keys are on, when they are on the table at
        // all: read before the match, because three of the arms below act on it
        // and the panel is borrowed for the whole of them.
        let table = panel
            .table_at_cursor()
            .map(|(at, table)| (at, table.cursor));
        let mut forget = None;
        match event.logical_key.as_ref() {
            Key::Named(NamedKey::Escape) => {
                self.close_settings();
                return;
            }
            // These walk the rows of whatever section is showing, and never the
            // rail: a section is pressed or tabbed to, so a list being read with
            // the arrow keys cannot be swapped out from under the reader.
            Key::Named(NamedKey::ArrowUp) => self.dirty |= panel.step(false),
            Key::Named(NamedKey::ArrowDown) => self.dirty |= panel.step(true),
            Key::Named(NamedKey::PageUp) => self.dirty |= panel.page(rows, false),
            Key::Named(NamedKey::PageDown) => self.dirty |= panel.page(rows, true),
            Key::Named(NamedKey::Home) => self.dirty |= panel.jump(false),
            Key::Named(NamedKey::End) => self.dirty |= panel.jump(true),
            // Left is the nudge on a row that has something to nudge, and
            // nothing at all on one that has not: no arrow key walks the rail,
            // so there is nowhere for it to go back to.
            Key::Named(NamedKey::ArrowLeft) => {
                if panel.on_row() {
                    match panel.at_cursor() {
                        // An entry has two states, so either arrow means the
                        // other one, the way a flag does.
                        Some(settings::Row::Entry(_)) => flip = Some(panel.cursor()),
                        // A table is walked up and down, marked with space and
                        // deleted with delete: there is nothing on a row of it
                        // for left and right to nudge.
                        Some(settings::Row::Table(_)) => {}
                        _ => nudge = Some(false),
                    }
                }
            }
            // Right nudges. Enter is the same, except on the endpoint, where it
            // starts typing, and on an entry, where it turns the thing on or
            // off.
            Key::Named(NamedKey::ArrowRight) | Key::Named(NamedKey::Enter) => {
                match panel.at_cursor() {
                    Some(settings::Row::Field { .. }) => edit = true,
                    Some(settings::Row::Entry(_)) => flip = Some(panel.cursor()),
                    // A document that is edited here: Enter is the keyboard's
                    // enable-edition. Any other block answers nothing.
                    Some(settings::Row::Paper(_)) => edit_doc = true,
                    // Enter marks the conversation the keys are on, the same as
                    // space: enter on a row of a list is expected to do
                    // something to that row, and marking it is the one thing
                    // here that cannot lose anything.
                    Some(settings::Row::Table(_)) => {
                        if let Some((index, at)) = table {
                            self.dirty |= panel.mark(index, at);
                        }
                    }
                    _ => nudge = Some(true),
                }
                self.dirty = true;
            }
            // The same mark, on the key a list is marked with everywhere else.
            Key::Named(NamedKey::Space) => {
                if let Some((index, at)) = table {
                    self.dirty |= panel.mark(index, at);
                }
            }
            // The delete under the table, from the keyboard: the first press
            // arms it and the second takes what is marked, which is the two
            // presses the button itself takes.
            Key::Named(NamedKey::Delete) => {
                let card = panel.row(panel.cursor()).is_some_and(|row| {
                    matches!(row, settings::Row::Card(card)
                        if card.does.is_some_and(settings::Doing::dangerous))
                });
                match (table, card) {
                    (Some((index, _)), _) => forget = Some(index),
                    // The restore under the palette, from the keyboard: the
                    // same two presses the button takes, on the key everything
                    // destructive on this panel is on.
                    (None, true) if panel.on_row() => forget = Some(panel.cursor()),
                    _ => {}
                }
            }
            _ => {}
        }
        if edit {
            self.dirty |= panel.edit();
        }
        if edit_doc {
            let config = self.config.clone();
            if let Some(panel) = self.settings.as_mut() {
                self.dirty |= panel.toggle_edition(panel.cursor(), &config);
            }
        }
        if let Some(index) = flip {
            let deed = self.settings.as_ref().and_then(|panel| panel.toggle(index));
            self.do_deed(deed);
        }
        if let Some(index) = forget {
            let deed = self
                .settings
                .as_mut()
                .and_then(|panel| panel.uninstall(index));
            self.dirty = true;
            self.do_deed(deed);
        }
        if let Some(forward) = nudge {
            self.change_setting(forward);
        }
        self.reveal_settings_cursor();
    }

    /// Write what was typed into the endpoint field, into the agent's own file,
    /// and read the whole file back.
    ///
    /// The same rule the window's own settings go through, on the other file:
    /// the writer keeps every other line and every comment, and what the panel
    /// shows next comes off the disk rather than out of what was typed. A file
    /// that refuses is said on the panel, where the edit is.
    fn save_endpoint(&mut self) {
        // The load line on the prompt section: Enter reads the named file
        // into the editor and writes nothing at all.
        if self.settings.as_ref().is_some_and(Settings::loading) {
            self.load_prompt_md();
            return;
        }
        // The install field is a field of the panel and not a line of any file,
        // so Enter on it starts the install instead. Branched here rather than
        // in the model, because this is the one place that turns a finished
        // edit into a write: without it, the address typed into the skills
        // section would be written into the agent's `.env` under a key nothing
        // reads.
        let installing = matches!(
            self.settings.as_ref().and_then(Settings::at_cursor),
            Some(settings::Row::Field { key, .. }) if *key == settings::SKILL_SOURCE
        );
        if installing {
            // The same two steps the card's button walks: Enter validates
            // what was typed, and Enter again installs what checked out.
            match self
                .settings
                .as_ref()
                .is_some_and(settings::Settings::checked_ok)
            {
                true => self.start_install(),
                false => self.validate_source(),
            }
            return;
        }
        let Some(panel) = self.settings.as_mut() else {
            return;
        };
        // A pressed colour: what was typed is a hex value for the window's own
        // file, under the key the swatch names. The panel refuses a value the
        // parser cannot read, on the footer, and nothing is written; a good one
        // lands where every other appearance change lands.
        if panel.picked().is_some() {
            let change = panel.finish_swatch_edit();
            if let Some(change) = change {
                self.write_setting(&change);
            }
            self.dirty = true;
            return;
        }
        // The add card's two fields live on the panel until its button writes
        // the file, the way the skill source does; Enter only keeps the text.
        if panel.keep_server_edit(&self.config) {
            self.dirty = true;
            return;
        }
        let Some(path) = panel.agent_file().map(std::path::Path::to_path_buf) else {
            panel.say_trouble(String::from("there is no config directory to write it in"));
            self.dirty = true;
            return;
        };
        let Some((key, value)) = panel.finish_edit() else {
            return;
        };
        self.write_agent_setting(&path, key, &value);
    }

    /// Read the `.md` file named on the load line into the AGENTS.md editor,
    /// as an unsaved edit: the reading happens here because the model owns no
    /// I/O, and a refusal is said on the footer with the typed path kept.
    fn load_prompt_md(&mut self) {
        let Some(path) = self.settings.as_ref().and_then(Settings::load_path) else {
            return;
        };
        let config = self.config.clone();
        match agent::load_md(&path) {
            Ok(body) => {
                if let Some(panel) = self.settings.as_mut() {
                    panel.take_loaded(body, &config);
                }
            }
            Err(why) => {
                if let Some(panel) = self.settings.as_mut() {
                    panel.say_trouble(why);
                }
            }
        }
        self.dirty = true;
    }

    /// Write the whole instructions file from the document editor, and read
    /// the agent back. Ctrl-S, because Enter there is how a line breaks.
    /// With no editor open there is no deed and nothing happens.
    fn save_instructions(&mut self) {
        let deed = self
            .settings
            .as_ref()
            .and_then(Settings::finish_instructions);
        self.do_deed(deed);
        self.dirty = true;
    }

    /// Write one setting into the agent's own file and read the whole file back.
    ///
    /// Shared by the endpoint field and by the two tracks beside it, because a
    /// nudged number and a typed URL are the same write to the same file. What
    /// the panel shows next is what the file answered.
    fn write_agent_setting(&mut self, path: &Path, key: &str, value: &str) {
        if let Err(why) = self.agent_setting(path, key, value)
            && let Some(panel) = self.settings.as_mut()
        {
            panel.say_trouble(why);
        }
        self.dirty = true;
    }

    /// The write itself, shared with the command path: the line lands, the
    /// agent's files are read back, and the open panel follows them. The
    /// reason comes back instead of being put anywhere.
    fn agent_setting(&mut self, path: &Path, key: &str, value: &str) -> Result<(), String> {
        settings::write_endpoint(path, key, value)?;
        let agent = self.read_agent();
        let config = self.config.clone();
        if let Some(panel) = self.settings.as_mut() {
            panel.adopt_agent(agent, &config);
        }
        Ok(())
    }

    /// Do what a press on an entry's toggle or its uninstall asked for, and read
    /// the agent's files back.
    ///
    /// The same rule every other write on this panel goes through: the disk is
    /// changed, the whole of the agent is read again, and what the panel shows
    /// next is what came back off the disk. So a skill's row says on or off
    /// because its directory is where it is, never because a press was
    /// remembered, and a move that failed leaves the row exactly as it was with
    /// the reason on the footer.
    fn do_deed(&mut self, deed: Option<settings::Deed>) {
        let Some(deed) = deed else {
            return;
        };
        // A set of conversations is its own path, because it is the one deed
        // that can half succeed. Every id is tried, the agent is read back
        // whatever happened, and what failed is said after the read: a delete of
        // five where the third refuses must not leave four rows on screen whose
        // transcripts are gone, and `refresh` clears the trouble line, so the
        // reason goes on after it and not before.
        if let settings::Deed::ForgetSessions { ids } = &deed {
            let failed = forget_sessions(sessions::dir(), sessions::index_path(), ids);
            let agent = self.read_agent();
            let config = self.config.clone();
            if let Some(panel) = self.settings.as_mut() {
                panel.adopt_agent(agent, &config);
                if !failed.is_empty() {
                    panel.say_trouble(failed.join("; "));
                }
            }
            self.dirty = true;
            return;
        }
        // Restoring is the other deed that is not the agent's: it writes the
        // window's own settings file, and what comes back is a whole Config
        // rather than an Ok, so it lands where a nudged setting lands instead of
        // going through the agent read below.
        if let settings::Deed::RestoreLooks = &deed {
            self.restore_looks();
            return;
        }
        let panel = match self.settings.as_ref() {
            Some(panel) => panel,
            None => return,
        };
        let done = deed_on_disk(
            &deed,
            panel.skills_at(),
            panel.mcp_file(false),
            panel.mcp_file(true),
        );
        match done {
            Ok(()) => {
                let agent = self.read_agent();
                let config = self.config.clone();
                if let Some(panel) = self.settings.as_mut() {
                    // A landed add leaves its card empty for the next one; a
                    // refusal keeps what was typed.
                    if matches!(&deed, settings::Deed::AddServer { .. }) {
                        panel.clear_server_fields();
                    }
                    // A landed save or restore closes the editor, so the block
                    // shows the file read back; a refusal keeps the buffer,
                    // with the reason on the footer.
                    if matches!(
                        &deed,
                        settings::Deed::SaveInstructions { .. }
                            | settings::Deed::RestorePrompt { .. }
                    ) {
                        panel.end_instructions_edit();
                    }
                    panel.adopt_agent(agent, &config);
                }
            }
            Err(why) => {
                if let Some(panel) = self.settings.as_mut() {
                    panel.say_trouble(why);
                }
            }
        }
        self.dirty = true;
    }

    /// Do one deed for a slash command, without needing the panel: the same
    /// disk writes [`App::do_deed`] routes, against the same places the agent
    /// snapshot names, with the whole of the agent read back after. The
    /// reason comes back to the transcript instead of landing on a footer
    /// nobody can see.
    fn command_deed(&mut self, deed: &settings::Deed) -> Result<(), String> {
        match deed {
            // The one deed that can half succeed: every id is tried, the
            // agent is read back whatever happened, and what failed is the
            // answer.
            settings::Deed::ForgetSessions { ids } => {
                let failed = forget_sessions(sessions::dir(), sessions::index_path(), ids);
                self.adopt_fresh_agent();
                match failed.is_empty() {
                    true => Ok(()),
                    false => Err(failed.join("; ")),
                }
            }
            // The window's own file rather than the agent's: what comes back
            // is a whole Config, applied the way any appearance change is.
            settings::Deed::RestoreLooks => {
                let path = self.settings_path().ok_or_else(|| {
                    String::from("there is no home directory to write settings in")
                })?;
                let config = settings::restore(&path)?;
                self.adopt(config);
                if let Some(panel) = self.settings.as_mut() {
                    panel.refresh(&self.config);
                }
                Ok(())
            }
            deed => {
                let agent = self.read_agent();
                deed_on_disk(
                    deed,
                    agent.skills_at.as_deref(),
                    agent.mcp.global.as_deref(),
                    agent.mcp.project.as_deref(),
                )?;
                self.adopt_fresh_agent();
                Ok(())
            }
        }
    }

    /// Read the agent's files again and hand them to the panel when it is
    /// open, so a command run behind it still leaves it saying the disk.
    fn adopt_fresh_agent(&mut self) {
        let agent = self.read_agent();
        let config = self.config.clone();
        if let Some(panel) = self.settings.as_mut() {
            panel.adopt_agent(agent, &config);
        }
    }

    /// Take every appearance line out of the settings file, and apply the file
    /// that is left.
    ///
    /// The same path a nudged setting takes, with a writer that comments lines
    /// out instead of writing one: the whole file is read back, the window is
    /// restyled from it, and the panel rebuilds off the Config that came out of
    /// it. So the sizes, the transparency and the palette all go at once and the
    /// next launch reads the same window this one is now showing.
    fn restore_looks(&mut self) {
        let Some(path) = self.settings_path() else {
            if let Some(panel) = self.settings.as_mut() {
                panel.say_trouble(String::from("there is no home directory to write settings in"));
            }
            self.dirty = true;
            return;
        };
        match settings::restore(&path) {
            Ok(config) => {
                self.adopt(config);
                if let Some(panel) = self.settings.as_mut() {
                    panel.refresh(&self.config);
                }
            }
            Err(why) => {
                if let Some(panel) = self.settings.as_mut() {
                    panel.say_trouble(why);
                }
            }
        }
        self.dirty = true;
    }

    /// A press inside the panel: a section, a row, the control on it, or the
    /// mark that closes.
    fn click_in_settings(&mut self, hit: Hit) {
        // Everything but the document itself moves the cursor or presses a
        // control, and which document is showing follows the cursor.
        if !matches!(hit, Hit::SettingsDoc) {
            self.forget_doc_selection();
        }
        match hit {
            Hit::SettingsClose => self.close_settings(),
            // Pressed, not clicked, the way a pane is: the anchor goes down here
            // and the far end of it follows the pointer until the button comes
            // up. A press that never moves selects nothing, which is what keeps
            // a click in the document from swallowing the next Ctrl-C.
            Hit::SettingsDoc => self.begin_doc_selection(),
            Hit::SettingsSection(index) => {
                if let Some(panel) = self.settings.as_mut() {
                    self.dirty |= panel.choose(index);
                }
            }
            Hit::SettingsRow(index, side) => {
                if let Some(panel) = self.settings.as_mut() {
                    self.dirty |= panel.point_at(index, side);
                }
                // A press in a document block's text is the start of a drag
                // over it, the way a press in a pane is: these are the two
                // files the prompt is made of, and nothing could be copied
                // out of either.
                let paper = self
                    .settings
                    .as_ref()
                    .is_some_and(|panel| panel.paper(index).is_some());
                if paper {
                    self.begin_paper_selection(index);
                }
            }
            // The value is the control, so clicking it does what the right arrow
            // does, on the row it is on rather than on the row the cursor was
            // left on. On the endpoint that is starting to type into it.
            Hit::SettingsValue(index, side) => {
                let field = match self.settings.as_mut() {
                    Some(panel) => {
                        self.dirty |= panel.point_at(index, side);
                        matches!(panel.cell(index, side), Some(settings::Row::Field { .. }))
                    }
                    None => false,
                };
                match field {
                    true => {
                        if let Some(panel) = self.settings.as_mut() {
                            self.dirty |= panel.edit();
                        }
                    }
                    false => self.change_setting(true),
                }
            }
            // Pressed, not clicked, the way a divider is: a slider says what it
            // means while the pointer moves, and it is written when the button
            // comes up. The press itself already moves it, so a click on the
            // track jumps the thumb there rather than doing nothing.
            Hit::SettingsSlider(index, side) => {
                self.sliding = Some((index, side));
                self.drag_slider();
            }
            // Pressed, not clicked, the way a pane divider is: the rail follows
            // the pointer while the button is down and the fraction it was left
            // at is written when the button comes up.
            Hit::SettingsRailDivider => self.sizing = Some(hit),
            // One option of a choice. It writes that option rather than the
            // next one along: the options are all on screen, so pressing the
            // one wanted is the whole gesture. The theme goes through the same
            // writer the arrow keys use, which is what clears the colour lines
            // that were overriding it; the custom option arms on its first
            // press instead, with the footer saying what a second one writes.
            Hit::SettingsChoice(index, side, at) => {
                let change = match self.settings.as_mut() {
                    Some(panel) => {
                        self.dirty |= panel.point_at(index, side);
                        panel.press_option(index, side, at)
                    }
                    None => None,
                };
                if let Some(change) = change {
                    self.write_setting(&change);
                }
                // Armed or written, the footer changed either way.
                self.dirty = true;
            }
            // A colour on the grid. Nothing is written: the panel says which key
            // in the settings file writes that colour, which is the one thing a
            // block of colour cannot say for itself.
            Hit::SettingsSwatch(index, cell) => {
                if let Some(panel) = self.settings.as_mut() {
                    self.dirty |= panel.pick(index, cell);
                }
            }
            // The toggle on a skill or a server, and the uninstall beside it.
            // Both put the cursor on the row first, so what the panel says at
            // the bottom is about the row that was just acted on.
            Hit::SettingsToggle(index) => {
                let deed = match self.settings.as_mut() {
                    Some(panel) => {
                        self.dirty |= panel.point_at(index, settings::Side::Left);
                        panel.toggle(index)
                    }
                    None => None,
                };
                self.do_deed(deed);
            }
            Hit::SettingsRemove(index) => {
                let deed = match self.settings.as_mut() {
                    Some(panel) => {
                        self.dirty |= panel.point_at(index, settings::Side::Left);
                        panel.uninstall(index)
                    }
                    None => None,
                };
                self.do_deed(deed);
            }
            // One conversation on the table: the press puts the keys on it, the
            // way a press on any other row of the panel does.
            Hit::SettingsPick(index, at) => {
                if let Some(panel) = self.settings.as_mut() {
                    self.dirty |= panel.point_at_row(index, at);
                }
            }
            // The mark in front of it. It does not move the keys: a mark is
            // pressed down a column, and a cursor dragged along with it would
            // disarm whatever the delete under the list was armed on.
            Hit::SettingsMark(index, at) => {
                if let Some(panel) = self.settings.as_mut() {
                    self.dirty |= panel.mark(index, at);
                }
            }
            // The three buttons under the table. The delete arms on the first
            // press and takes what is marked on the second, the same two presses
            // every other delete on this panel takes.
            Hit::SettingsAct(index, act) => {
                let deed = match self.settings.as_mut() {
                    Some(panel) => match act {
                        settings::Act::All => {
                            self.dirty |= panel.mark_all(index, true);
                            None
                        }
                        settings::Act::None => {
                            self.dirty |= panel.mark_all(index, false);
                            None
                        }
                        // Both of these arm on the first press and act on the
                        // second: one deletes transcripts, the other takes
                        // lines out of a file somebody may have edited by hand.
                        settings::Act::Forget | settings::Act::Restore => panel.uninstall(index),
                        // The two buttons under the skills table, on the row
                        // the keys are on: the move that turns one off, and the
                        // delete, which arms first the way every delete here
                        // does.
                        settings::Act::Turn => panel.turn_row(index),
                        settings::Act::Uninstall => panel.uninstall(index),
                        // The add card's button: the deed writes the file.
                        settings::Act::AddServer => {
                            let config = self.config.clone();
                            panel.cancel_edit_elsewhere(settings::SERVER_NAME, settings::SERVER_HOW);
                            self.dirty |= panel.point_at(index, settings::Side::Left);
                            panel.add_server_deed(&config)
                        }
                        // The validate half of the install card's button:
                        // answered here and now, on the panel.
                        settings::Act::Validate => {
                            let elsewhere = !matches!(
                                panel.at_cursor(),
                                Some(settings::Row::Field { key, .. })
                                    if *key == settings::SKILL_SOURCE
                            );
                            if elsewhere {
                                panel.cancel_edit();
                            }
                            self.dirty |= panel.point_at(index, settings::Side::Left);
                            None
                        }
                        // The enable-edition checkbox on a prompt document:
                        // ticked opens the editor, ticked again drops it.
                        settings::Act::EditPrompt => {
                            let config = self.config.clone();
                            self.dirty |= panel.toggle_edition(index, &config);
                            None
                        }
                        // The save in that footer: the same whole-file deed
                        // Ctrl-S asks for. Both it and the restore stand there
                        // with edition off, drawn dim, and a dim button does
                        // nothing when it is pressed.
                        settings::Act::SavePrompt => match panel.edition_on(index) {
                            true => panel.finish_instructions(),
                            false => None,
                        },
                        // The restore beside it: armed on the first press,
                        // acted on the second, like every button that loses
                        // something.
                        settings::Act::RestorePrompt => match panel.edition_on(index) {
                            true => panel.restore_prompt(index),
                            false => None,
                        },
                        // The load beside those: opens the path line; Enter
                        // then reads the file ([`App::load_prompt_md`]).
                        settings::Act::LoadPrompt => {
                            self.dirty |= panel.begin_load(index);
                            None
                        }
                        // The install card's own button. Not a deed: a deed is
                        // done here and now, and this is a clone with two
                        // minutes to answer in.
                        settings::Act::Install => {
                            // An edit running on another row is dropped rather
                            // than carried onto this one, the way Escape drops
                            // it: the address that gets installed is the one in
                            // this field and never half of somebody's endpoint.
                            let elsewhere = !matches!(
                                panel.at_cursor(),
                                Some(settings::Row::Field { key, .. })
                                    if *key == settings::SKILL_SOURCE
                            );
                            if elsewhere {
                                panel.cancel_edit();
                            }
                            self.dirty |= panel.point_at(index, settings::Side::Left);
                            None
                        }
                        // The credential's own button: dots, or the value.
                        // Nothing on a disk changes.
                        settings::Act::Reveal => {
                            let config = self.config.clone();
                            panel.flip_key(&config);
                            self.dirty = true;
                            None
                        }
                        // The connection card's button, answered by a process
                        // ([`App::check_connection`]).
                        settings::Act::Check => {
                            self.dirty |= panel.point_at(index, settings::Side::Left);
                            None
                        }
                        // The way back under it: the address the CLI would
                        // have found on its own, written as a line.
                        settings::Act::DefaultEndpoint => {
                            panel.cancel_edit();
                            self.dirty |= panel.point_at(index, settings::Side::Left);
                            None
                        }
                    },
                    None => None,
                };
                if matches!(act, settings::Act::Install) {
                    self.start_install();
                }
                if matches!(act, settings::Act::Validate) {
                    self.validate_source();
                }
                if matches!(act, settings::Act::Check) {
                    self.check_connection();
                }
                if matches!(act, settings::Act::DefaultEndpoint) {
                    self.default_endpoint();
                }
                self.do_deed(deed);
            }
            // The panel's own margin. Swallowed: it covers the window, so a
            // press here has nothing behind it to reach.
            _ => {}
        }
        // No reveal here, ever: the row pressed is already on screen, and a
        // press that lands beside the rows (the margin, the document, the gap
        // under a card) leaves the cursor where it was, which can be a screen
        // away. Revealing that cursor scrolled the list back out from under
        // the click. Only keyboard movement reveals the cursor.
    }

    /// Move the slider under the button to where the pointer is now, and take
    /// its value while it moves.
    ///
    /// Nothing is written here, the same way a divider writes nothing while it
    /// is being dragged: a drag across the window is hundreds of motion events
    /// and the file is rewritten once, when the button comes up. What the window
    /// looks like is not deferred with it. A slider that only moves the window
    /// on release is a control you cannot aim: the opacity you are dragging to
    /// is the one thing that would tell you where to stop.
    fn drag_slider(&mut self) {
        let Some((index, side)) = self.sliding else {
            return;
        };
        let layout = self.layout();
        let Some(at) = layout.slider_at(index, side, self.cursor.x as f32) else {
            return;
        };
        if let Some(panel) = self.settings.as_mut() {
            self.dirty |= panel.slide(index, side, at);
        }
        self.preview_setting();
    }

    /// Apply what a slider is being dragged to, without writing the file.
    ///
    /// Through [`Config::apply`], which is the setter [`Config::parse`] reads
    /// the file with, so the live value is held to the bounds the file holds it
    /// to and a drag cannot show the window a value the next launch would
    /// refuse. Then through [`App::adopt`], so the palette, the column widths
    /// and the divider ratios all follow, exactly as they do when the value is
    /// written on the way up.
    fn preview_setting(&mut self) {
        let Some(change) = self.settings.as_ref().and_then(Settings::previewed) else {
            return;
        };
        // Nothing of the agent's is live in this window: the CLI reads its file
        // on its next request, so the drag has nothing to show until the button
        // comes up and the line is written. The row's own number still follows
        // the thumb, which is what a drag needs to be aimed.
        if change.file == settings::File::Agent {
            return;
        }
        let mut config = self.config.clone();
        if !config.apply(change.key, &change.value) || config == self.config {
            return;
        }
        self.adopt(config);
        self.dirty = true;
    }

    /// The button came up on a slider: write where it was left, once.
    fn drop_slider(&mut self) {
        self.sliding = None;
        let Some(change) = self.settings.as_mut().and_then(Settings::drop_slider) else {
            return;
        };
        self.write_setting(&change);
        self.dirty = true;
    }

    /// Nudge the row under the cursor, write it, and take the file's answer.
    ///
    /// The file is the source of truth all the way through: the change is written
    /// with the settings writer, the whole file is read back, and the window is
    /// restyled from that. So the panel, the window and the next launch cannot
    /// disagree about what a setting is, and a value the parser clamps shows up
    /// clamped rather than as what was asked for.
    fn change_setting(&mut self, forward: bool) {
        let Some(change) = self.settings.as_mut().and_then(|panel| panel.nudged(forward)) else {
            // Nothing to write, but the nudge may have armed the custom option
            // and put its warning on the footer.
            self.dirty = true;
            return;
        };
        self.write_setting(&change);
    }

    /// Write one change and take the file's answer. The half of
    /// [`App::change_setting`] the slider shares, since a drag decides its value
    /// the other way round and lands in exactly the same place. A refusal is
    /// said on the panel rather than in the activity pane, which is behind the
    /// takeover and cannot be read from here.
    fn write_setting(&mut self, change: &settings::Change) {
        if let Err(why) = self.apply_change(change)
            && let Some(panel) = self.settings.as_mut()
        {
            panel.say_trouble(why);
        }
        self.dirty = true;
    }

    /// Route one change to the file it belongs to and apply what comes back.
    /// The one write path the panel's nudges and the slash commands share, so
    /// the two cannot land the same key differently; the caller decides where
    /// the reason goes when nothing lands.
    fn apply_change(&mut self, change: &settings::Change) -> Result<(), String> {
        // A setting of the agent's goes to the agent's file, through the agent's
        // writer. Same nudge, same track, other file. The path is the panel's
        // when it is open and the agent's own config directory when it is not:
        // the same place, since the panel read it from there.
        if change.file == settings::File::Agent {
            let path = self
                .settings
                .as_ref()
                .and_then(Settings::agent_file)
                .map(std::path::Path::to_path_buf)
                .or_else(|| agent::config_dir().map(|dir| dir.join(".env")))
                .ok_or_else(|| String::from("there is no config directory to write it in"))?;
            return self.agent_setting(&path, change.key, &change.value);
        }
        let path = self
            .settings_path()
            .ok_or_else(|| String::from("there is no home directory to write settings in"))?;
        let config = settings::commit(&path, change)?;
        self.adopt(config);
        if let Some(panel) = self.settings.as_mut() {
            panel.refresh(&self.config);
        }
        Ok(())
    }

    /// Take a settings file the panel just wrote and apply all of it.
    ///
    /// Everything the window reads out of the config is rebuilt here, because a
    /// setting that only takes effect on the next launch reads as a setting that
    /// does nothing: the palette, the two font sizes (which are column widths in
    /// the renderer, not just text sizes) and the two views that can be turned
    /// off.
    fn adopt(&mut self, config: Config) {
        let panes = pane_changes(&self.config, &config);
        self.config = config;
        // A ratio changed on the panel is a divider moved, so the window takes
        // it the way it takes a colour: the file is what both of them read.
        self.left_width = [self.config.left_width, self.config.left_width_bottom];
        self.top_height = [self.config.top_height, self.config.top_height_right];
        self.settings_rail = self.config.settings_rail;
        self.restyle();
        let mut changed = false;
        for (view, wanted) in panes {
            match wanted {
                true => {
                    changed |= self.dock.unhide(view);
                }
                false => {
                    if self.dock.hide(view) {
                        self.forget_selection_in(view);
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.save_dock();
        }
    }

    /// The skin and the column widths, from the config as it now is.
    ///
    /// A surface that refused alpha keeps its opaque palette: the setting is
    /// still written and still read, and it is the surface that cannot honour it.
    fn restyle(&mut self) {
        self.skin = Skin::from(&self.config);
        if self.gpu.as_ref().is_some_and(|gpu| !gpu.caps.transparent) {
            self.skin = self.skin.opaque();
        }
        if let Some(renderer) = self.renderer.as_mut() {
            self.column = renderer.column_width(self.config.font_size);
            self.pane_column = renderer.column_width(self.config.pane_font_size);
            self.menu_column = renderer.column_width(menu::paint::MENU_SIZE);
        }
        self.dirty = true;
    }

    /// How many rows the panel's list can show right now.
    fn settings_rows(&self) -> usize {
        self.layout().settings_capacity(self.config.pane_font_size)
    }

    /// How wide those rows are, in characters. A row carrying an entry is as
    /// tall as its description wraps to in this width, so anything that counts
    /// rows has to be told it.
    fn settings_cols(&self) -> usize {
        self.layout().settings_entry_columns(self.pane_column)
    }

    /// Bring the cursor on screen, measured against the layout the panel is
    /// drawn in rather than against the panel alone.
    fn reveal_settings_cursor(&mut self) {
        let rows = self.settings_rows();
        let cols = self.settings_cols();
        if let Some(panel) = self.settings.as_mut() {
            self.dirty |= panel.reveal(rows, cols);
        }
    }

    /// Keys while the picker is up. Nothing else in the window is live, so this
    /// is the whole keyboard rather than a branch inside [`App::key`].
    fn key_in_picker(&mut self, event: &winit::event::KeyEvent, event_loop: &ActiveEventLoop) {
        let rows = self.picker_rows();
        // The same swap the button beside Open does, before the picker is
        // borrowed: reading the sessions off the disk is the window's job.
        if matches!(event.logical_key.as_ref(), Key::Character("r")) && self.modifiers.control_key()
        {
            self.toggle_sessions();
            return;
        }
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
            // pane to fall back to: Escape drops what has been typed, then the
            // session list if that is what is showing, and with neither there
            // it closes the window.
            Key::Named(NamedKey::Escape) => {
                if !picker.clear_filter() && !picker.show_folders() {
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
        if let Some(chosen) = chosen {
            self.choose(chosen);
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

    /// A press inside the picker: a row, the mark that opens one, or one of the
    /// three buttons in its head.
    fn click_in_picker(&mut self, hit: Hit, double: bool) {
        // Both mode buttons before the picker is borrowed, because putting the
        // sessions up reads the disk and that is the window's job rather than
        // the model's. Folders does not, and goes through the same pair so the
        // two presses cannot drift apart.
        match hit {
            Hit::PickerSessions => return self.show_sessions_now(),
            Hit::PickerFolders => return self.show_folders_now(),
            _ => {}
        }
        let mut chosen = None;
        if let Some(picker) = self.picker.as_mut() {
            self.dirty |= match hit {
                // The mark before the row, and it does not move the cursor: the
                // press that opens a folder is a press that asks what is in it,
                // and answering by also selecting it would make every look a
                // choice.
                Hit::PickerMark(index) => picker.toggle(index),
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
            Some(chosen) => self.choose(chosen),
            None => self.reveal_picker_cursor(),
        }
    }

    /// Carry on the session on the row at `index`, which is what the menu's Open
    /// row does. The same call a double click makes, so the two cannot drift.
    fn open_session(&mut self, index: usize) {
        let chosen = self.picker.as_mut().and_then(|picker| picker.double(index));
        self.dirty = true;
        match chosen {
            Some(chosen) => self.choose(chosen),
            None => self.reveal_picker_cursor(),
        }
    }

    /// Delete the session on the row at `index`: its transcript, and the note
    /// saying which folder it belonged to.
    ///
    /// Here rather than in the picker because this is the half of the window
    /// that touches the disk, the same rule the session list itself is read
    /// under. The id comes off the row the menu was opened over and goes through
    /// [`sessions::forget`], which is the only thing anywhere that removes a
    /// file and refuses any name that could reach out of the sessions directory.
    ///
    /// The list is then read again rather than mended in place, so what is on
    /// screen is what is on the disk, and it is read again through
    /// [`Picker::refresh_sessions`], which keeps the cursor where it was:
    /// deleting three sessions in a row should be three presses.
    fn delete_session(&mut self, index: usize) {
        let Some(id) = self
            .picker
            .as_ref()
            .and_then(|picker| picker.session(index))
            .map(|saved| saved.id.clone())
        else {
            return;
        };
        self.dirty = true;
        if let Err(why) = forget_session(sessions::dir(), sessions::index_path(), &id) {
            // The picker's own line for a press that did nothing. A delete that
            // silently fails is a row that comes back on the next refresh with
            // nothing anywhere saying why.
            if let Some(picker) = self.picker.as_mut() {
                picker.refuse(why);
            }
            return;
        }
        let listing = self.saved_sessions();
        if let Some(picker) = self.picker.as_mut() {
            picker.refresh_sessions(listing);
        }
        self.reveal_picker_cursor();
    }

    fn drain(&mut self) {
        // Before the agent's own frames and outside the check for one: the
        // environment tail is read whether or not a session is running, and
        // the panel can be open on a window that has no agent at all.
        self.take_env();
        // The same, for the skill being installed: it answers on its own
        // thread and wakes the loop.
        self.take_install();
        // And for the connection check, which is one more process answering
        // on a thread.
        self.take_health();
        let Some(link) = self.link.as_mut() else {
            return;
        };
        let incoming = link.drain();
        let mut turn_ended = false;
        for item in incoming {
            match item {
                Incoming::Frame(event) => {
                    let at = self.epoch.elapsed().as_secs_f64();
                    turn_ended |= matches!(event, noob_proto::Event::TurnEnd { .. });
                    // The prompt arrived: the turn it asked for is running.
                    if matches!(event, noob_proto::Event::TurnStart { .. }) {
                        self.answer_by = None;
                    }
                    // The one frame that says which session is running and where
                    // it is running, which is the note the session list needs.
                    if let noob_proto::Event::SessionStart { id, workspace, .. } = &event {
                        self.remember_session(id, workspace);
                        // Kept for the rest of the session, so how full its
                        // context window got can be written on the same note
                        // later: no other frame says which session it belongs
                        // to.
                        self.session = Some((id.clone(), workspace.clone()));
                    }
                    self.dirty |= self.state.apply_at(event, Some(at));
                }
                Incoming::Diagnostic(line) => {
                    self.state.output.say(format!("noob: {line}"), Tone::Bad);
                    self.dirty = true;
                }
                Incoming::Ended(reason) => {
                    self.state.phase = state::Phase::Gone;
                    self.trouble = Some(reason);
                    // A child that is gone is its own answer; the wait would
                    // otherwise land on top of it ten seconds later.
                    self.answer_by = None;
                    self.dirty = true;
                }
            }
        }
        if turn_ended {
            self.remember_context();
        }
        // The output tab follows its agent out: a finished child leaves the
        // fleet, and a tab over "no agent chosen" is a tab with no job.
        if self.state.shown_agent.is_none() && !self.dock.is_hidden(View::Agent) {
            self.dock.hide(View::Agent);
            self.dirty = true;
        }
        self.follow_open_file();
    }

    /// Keep the file the agent just touched on screen in the explorer.
    ///
    /// Here rather than in `State`, because how many rows the list can show is a
    /// question about the window, and the layout is the only thing that knows.
    fn follow_open_file(&mut self) {
        if self.last_open_file != self.state.open_file {
            self.last_open_file = self.state.open_file;
            // Showing another file drops a selection made in the one before
            // it: the selection holds line numbers and the view it was made
            // in, so it would otherwise band the same line numbers of a
            // different file. A selection somewhere else is untouched.
            if self.selection.map(|s| s.at) == Some(select::Where::Pane(dock::View::Files)) {
                self.selection = None;
            }
            self.dirty = true;
        }
        let layout = self.layout();
        if layout.file_list.h < 1.0 {
            return;
        }
        let rows = layout.rows(layout.file_list, self.config.pane_font_size);
        let next = scroll::reveal_file(
            self.file_scroll,
            self.state.open_file,
            self.state.files.len(),
            rows,
        );
        if next != self.file_scroll {
            self.file_scroll = next;
            self.dirty = true;
        }
    }

    fn submit(&mut self) {
        let text = self.prompt.text().trim().to_string();
        if text.is_empty() {
            return;
        }
        self.prompt.clear();
        // A line that starts with / is for the window, never for the agent:
        // no turn starts, and the answer is a local line in the transcript.
        if commands::is_command(&text) {
            self.run_command(&text);
            self.dirty = true;
            return;
        }
        // A prompt typed while the agent has the turn queues behind it, on
        // both sides of the wire: serve holds the text until the turn ends,
        // and the state pins a dim [queued] row until the turn that takes it
        // starts. Echoing it into the transcript now would show it answered
        // by a turn that never saw it.
        let waits = self.link.as_ref().is_some_and(link::Link::is_alive)
            && (self.state.phase.busy() || !self.state.queued.is_empty());
        if waits {
            self.state.enqueue(&text);
            if let Some(link) = self.link.as_mut() {
                link.send(Cmd::PromptQueue { text });
            }
        } else {
            self.state.submitted(&text);
            match self.link.as_mut() {
                Some(link) if link.is_alive() => {
                    link.send(Cmd::PromptSubmit { text });
                    // The turn this asks for has to start, or the window says
                    // it did not: the phase is Thinking from here, and only a
                    // frame moves it off.
                    self.answer_by = Some(Instant::now() + ANSWER_WAIT);
                }
                _ => self.state.output.say("no agent is running", Tone::Bad),
            }
        }
        self.dirty = true;
    }

    /// Run one slash command: parse it against the registry, do what it asks
    /// through the same writes the settings panel does, and answer in the
    /// transcript.
    ///
    /// The snapshot handed to the dispatcher is the same reading the panel
    /// opens over, so a skill or a server is named by what is really on the
    /// disk right now.
    fn run_command(&mut self, text: &str) {
        self.state.commanded(text);
        match commands::dispatch(text, &self.read_agent()) {
            commands::Answer::Say(lines) => {
                for line in lines {
                    self.state.output.say(line, Tone::Dim);
                }
            }
            commands::Answer::Refuse(why) => self.state.output.say(why, Tone::Bad),
            commands::Answer::Do(act) => match act {
                commands::Act::Change { change, said } => match self.apply_change(&change) {
                    Ok(()) => self.state.output.say(said, Tone::Dim),
                    Err(why) => self.state.output.say(why, Tone::Bad),
                },
                commands::Act::Deed { deed, said } => match self.command_deed(&deed) {
                    Ok(()) => self.state.output.say(said, Tone::Dim),
                    Err(why) => self.state.output.say(why, Tone::Bad),
                },
                commands::Act::Install { source, said } => self.command_install(source, said),
                commands::Act::Open { section, said } => {
                    if self.settings.is_none() {
                        self.open_settings();
                    }
                    if let Some(at) = section
                        && let Some(panel) = self.settings.as_mut()
                    {
                        panel.choose(at);
                    }
                    self.state.output.say(said, Tone::Dim);
                }
            },
        }
    }

    fn cancel(&mut self) {
        if let Some(link) = self.link.as_mut() {
            link.send(Cmd::TurnCancel);
        }
        self.esc_armed = None;
        self.state.status = String::from("cancelling");
        self.dirty = true;
    }

    /// Put the window in or out of maximized, the state the desktop puts it in.
    ///
    /// One place, called by both the maximize button and the double click on
    /// the bar, so the two cannot drift into meaning different things. The
    /// compositor answers with a `Resized`, which is what actually redraws the
    /// window; the flag is set here as well so a request that is refused still
    /// leaves the window drawing what it is.
    fn toggle_maximized(&mut self, window: &Window) {
        window.set_maximized(!window.is_maximized());
        self.dirty = true;
    }

    /// Collapse a space to its tab strip, or open it again. The strip keeps its
    /// tabs, so a folded pane is still a place to click rather than a gone one.
    ///
    /// Unreachable, kept whole: it was the click on the tab already showing, and
    /// no gesture collapses a pane any more. Everything the layout and the
    /// drawing need for a folded space is still here and still tested, so
    /// putting it back is one call from [`App::release`].
    #[allow(dead_code)]
    fn fold(&mut self, space: Space) {
        let slot = self.dock.slot_mut(space);
        slot.folded = !slot.folded;
        self.dirty = true;
    }

    /// Collapse the window to its title bar, or restore it. The bar keeps
    /// showing what the agent is doing, so a shaded window is still a status
    /// light rather than a hidden one.
    ///
    /// Unreachable, kept whole: the double click that called it is the desktop's
    /// maximize toggle now, and the shading was buggy enough to take off the
    /// bar rather than fix in place. Everything it needs is still here and still
    /// tested, so putting it back is one call.
    #[allow(dead_code)]
    fn shade(&mut self, window: &Window) {
        self.shaded = !self.shaded;
        if self.shaded {
            self.unshaded = Some(window.inner_size());
            // Read before anything is asked for, and kept until the strip is
            // opened again: a window maximized on purpose is still a maximized
            // window afterwards.
            self.was_maximized = window.is_maximized();
        }
        let ask = shade_request(self.shaded, self.unshaded, self.was_maximized);
        // The minimum goes first. A resize request is clamped to it, and
        // asking a 680x380 minimum for a 30 pixel strip left the surface at
        // full height with the strip painted across the top of it, which is
        // the black bar the window showed when it was shaded.
        window.set_min_inner_size(ask.min);
        // Then the maximized state, before the size and never after it: a
        // maximized window ignores a resize request, so the strip is asked for
        // once the window is in a state that can take it.
        if let Some(maximized) = ask.maximized {
            window.set_maximized(maximized);
        }
        // Every one of those is a request, and what comes back is read in
        // `reconcile_shade`. Only leaving maximized puts a round trip in the way
        // of the strip, so only that case waits: every other shade is answered
        // once, and what it is answered with is the answer. Set before the size
        // is asked for, because that answer can arrive before the call returns.
        self.settling = ask.maximized == Some(false);
        if let Some(size) = ask.size {
            self.ask_for_size(window, size);
        }
        if !self.shaded {
            self.unshaded = None;
            self.was_maximized = false;
        }
        self.dirty = true;
    }

    /// Read the shaded state off the surface the compositor just handed back,
    /// and drop it when the surface is not a strip.
    ///
    /// The rule is [`shade_of`]. Dropping the state asks for no size: the surface
    /// on screen is what the compositor decided, and a window it snapped
    /// maximized out of a shade is a maximized window, not a window owed the
    /// size it had before. Only the minimum goes back, so the next resize by
    /// hand is held to it again.
    fn reconcile_shade(&mut self, height: u32) {
        let Some(window) = self.window.clone() else {
            return;
        };
        let strip = view::strip_height() as u32;
        let shade = shade_of(
            self.shaded,
            window.is_maximized(),
            self.settling,
            height,
            strip,
        );
        // Everything but the un-maximize still being in flight is an answer, and
        // an answer ends the wait for one.
        self.settling = shade == Shade::Leaving;
        match shade {
            Shade::Strip | Shade::Leaving => {}
            // The window has left maximized and this surface is that. Ask for
            // the strip again: the request that went out beside the un-maximize
            // was made to a window that was still maximized and still able to
            // refuse it, and this one is not. At the width it has now rather
            // than the width it was remembered at, which was the width of the
            // whole screen.
            Shade::Asking => {
                let size = PhysicalSize::new(window.inner_size().width, strip);
                self.ask_for_size(&window, size);
            }
            Shade::Open if self.shaded => {
                self.shaded = false;
                self.unshaded = None;
                self.was_maximized = false;
                window.set_min_inner_size(Some(MIN_SIZE));
                self.dirty = true;
            }
            Shade::Open => {}
        }
    }

    /// Ask the window for a size, and read the answer it gives back on the spot.
    ///
    /// `request_inner_size` answers one of two ways. `None` means the request
    /// went to the compositor and a `Resized` will follow, which is the event
    /// path. `Some` means there will be no event at all, and what comes back is
    /// either the size that was applied or the size the window kept when it
    /// refused. Read back off this machine's compositor, a shade is answered the
    /// second way both times: granted, it returns the strip, and refused by a
    /// maximized window it returns the full screen it stayed at. So the answer
    /// has to go through the same reconcile the event does, or a refusal is
    /// never heard about and the window is drawn as a strip it never became.
    fn ask_for_size(&mut self, window: &Window, size: PhysicalSize<u32>) {
        if let Some(applied) = window.request_inner_size(size) {
            self.reconcile_shade(applied.height);
        }
    }

    /// Hand the compositor an interactive move, once the pointer has moved far
    /// enough from the press that it cannot be the first click of a double
    /// click. The rule is [`began_move`].
    fn maybe_move(&mut self, window: &Window) {
        let Some(from) = self.moving else {
            return;
        };
        let moved = (self.cursor.x - from.x).abs() + (self.cursor.y - from.y).abs();
        if !began_move(true, moved) {
            return;
        }
        // Cleared before the call, not after: the compositor takes the pointer
        // for the length of the move and the button coming up never arrives
        // here, so nothing else would ever clear it.
        self.moving = None;
        let _ = window.drag_window();
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
        // The popup is on the same floating layer and closes the same way:
        // a press anywhere off it only puts it away, and does nothing else,
        // so a click aimed past a popup at a pane cannot also start a
        // selection in that pane. A press ON it falls through to its own
        // arms: the close mark, the track, and the selection its box starts.
        if self.state.open_call.is_some()
            && !matches!(
                hit,
                Hit::CallPopup | Hit::CallPopupClose | Hit::CallPopupScrollbar
            )
        {
            self.state.open_call = None;
            self.dirty = true;
            return;
        }

        let now = Instant::now();
        let double = matches!(self.last_click, Some((last, at))
            if last == hit && now.duration_since(at) < DOUBLE_CLICK);
        self.last_click = Some((hit, now));

        // The title bar is still the title bar under either takeover: the window
        // can be moved, shaded and closed while one is up.
        let chrome = matches!(hit, Hit::TitleBar | Hit::Close | Hit::Maximize | Hit::Minimize);
        if self.picker.is_some() && !chrome {
            self.click_in_picker(hit, double);
            return;
        }
        if self.settings.is_some() && !chrome {
            self.click_in_settings(hit);
            return;
        }

        match hit {
            Hit::Close => event_loop.exit(),
            Hit::Minimize => window.set_minimized(true),
            Hit::Maximize => self.toggle_maximized(window),
            Hit::TitleBar => match title_click(double) {
                TitleClick::Maximize => {
                    // The first click of the pair is still holding a move that
                    // never began. It ends here rather than moving the window
                    // while the compositor is resizing it under the pointer.
                    self.moving = None;
                    self.toggle_maximized(window);
                }
                TitleClick::ArmMove => {
                    // Pressed, not moved: what this is gets decided by the
                    // pointer, in `maybe_move`. Handing the compositor a move
                    // now would eat the second click of a double click, and on
                    // GNOME a press near the top of the screen snaps the window
                    // maximized before that second click arrives.
                    self.moving = Some(self.cursor);
                }
            },
            Hit::Tab(view, space) => {
                // Pressed, not yet clicked: a tab is also a drag handle, so
                // what this was is only decided when the pointer moves or is
                // released.
                self.holding = Some((view, space, self.cursor));
            }
            Hit::TabsLeft(space) => self.walk_strip(space, false),
            Hit::TabsRight(space) => self.walk_strip(space, true),
            // Pressed, not clicked, the way a tab is: what a divider does
            // happens while the pointer moves, and letting go of one that never
            // moved has to leave the window exactly as it found it.
            Hit::ColumnDivider(_) | Hit::RowDivider(_) => self.sizing = Some(hit),
            Hit::File(index, _) => {
                self.state.show_file(index);
                self.follow_open_file();
                self.dirty = true;
            }
            // Both, in that order. A press in ACTIVITY on a row that belongs to
            // a call opens that call, and the selection is still begun under it:
            // a press that turns into a drag is a drag, and `extend_selection`
            // takes the popup back down the moment one has selected anything.
            // Opening the popup instead of the selection would make every call
            // row in the pane uncopyable.
            Hit::Body(space) => {
                self.begin_selection(space);
                self.open_call_under_pointer(space);
                self.open_agent_under_pointer(space);
            }
            // A press on a live track takes the thumb; on the gutter of a
            // pane with nothing to scroll it is a press on the pane.
            Hit::Scrollbar(space) => {
                if self.begin_thumb(space) {
                    self.drag_thumb();
                } else {
                    self.begin_selection(space);
                    self.open_call_under_pointer(space);
                    self.open_agent_under_pointer(space);
                }
            }
            // All four are handled above, while the picker is up, which is the
            // only time any of them can be hit at all.
            Hit::PickerRow(_)
            | Hit::PickerMark(_)
            | Hit::PickerOpen
            | Hit::PickerFolders
            | Hit::PickerSessions
            | Hit::Picker => {}
            // The same for the nine the settings panel owns.
            Hit::SettingsSection(_)
            | Hit::SettingsRow(..)
            | Hit::SettingsValue(..)
            | Hit::SettingsSlider(..)
            | Hit::SettingsSwatch(_, _)
            | Hit::SettingsChoice(..)
            | Hit::SettingsToggle(_)
            | Hit::SettingsRemove(_)
            | Hit::SettingsPick(..)
            | Hit::SettingsMark(..)
            | Hit::SettingsAct(..)
            | Hit::SettingsRailDivider
            | Hit::SettingsDoc
            | Hit::SettingsClose
            | Hit::Settings => {}
            Hit::Input => {
                // A press, not a placement: the anchor stays here so motion
                // with the button still down selects the span between the two.
                // An anchor sitting on the caret is not a selection, so a click
                // that never moves still reads as one that selected nothing.
                self.prompt.press(self.caret_under_pointer());
                self.prompt_selecting = true;
                self.dirty = true;
            }
            // Both handled above, while a menu is open, which is the only
            // time either can be hit at all.
            Hit::MenuRow(_) | Hit::Menu => {}
            // A press on the popup starts a selection over its document; the
            // band and the copy resolve against the same lines the blocks
            // are drawn from.
            Hit::CallPopup => {
                let spot = {
                    let layout = self.layout();
                    let frame = self.frame(&layout);
                    crate::widgets::popup::spot_at(
                        &frame,
                        self.cursor.x as f32,
                        self.cursor.y as f32,
                    )
                };
                if let Some(spot) = spot {
                    self.selection =
                        Some(select::Selection::new(select::Where::CallPopup, spot));
                    self.selecting = true;
                    self.dirty = true;
                }
            }
            // The same close the settings panel has: one press, the popup
            // goes, and the pane under it is exactly as it was.
            Hit::CallPopupClose => {
                self.state.open_call = None;
                self.dirty = true;
            }
            // The popup's own track: the press takes the thumb, like every
            // other scrollbar in the window.
            Hit::CallPopupScrollbar => {
                self.thumbing = Some(Thumb::Popup);
                self.drag_thumb();
            }
        }
    }

    /// A press in the AGENTS pane opens the pressed child's own output as
    /// the `[N] AGENT - OUTPUT` tab, in the top-left space the first time
    /// and wherever it has been dragged to after.
    fn open_agent_under_pointer(&mut self, space: Space) {
        if self.dock.slot(space).active() != Some(View::Agents) {
            return;
        }
        let ordinal = {
            let layout = self.layout();
            let panel = layout.placed(space).body;
            let frame = self.frame(&layout);
            crate::widgets::agents::agent_at(
                &frame,
                panel,
                self.cursor.x as f32,
                self.cursor.y as f32,
            )
        };
        let Some(ordinal) = ordinal else {
            return;
        };
        self.state.show_agent(ordinal);
        if self.dock.is_hidden(View::Agent) {
            self.dock.unhide(View::Agent);
        } else {
            self.dock.reveal(View::Agent);
        }
        self.dirty = true;
    }

    /// A press in the ACTIVITY pane, resolved back to the call that wrote the
    /// row under it.
    ///
    /// The row is found through the same [`state::Pane::spot_in`] a selection
    /// uses, so the row the popup is about and the row the pointer is over
    /// cannot come apart. A row belonging to no call, a progress line or an
    /// empty pane leaves the popup shut and the selection alone.
    fn open_call_under_pointer(&mut self, space: Space) {
        if self.dock.slot(space).active() != Some(View::Activity) {
            return;
        }
        let layout = self.layout();
        // Strict, unlike a selection: a press on the empty space under the
        // list is a press on nothing, not on the nearest row. The selection
        // clamp is for sweeps; opening the last call from a click that
        // landed on air was a popup nobody asked for.
        let Some(ordinal) = self.call_strictly_under_pointer(&layout, space) else {
            return;
        };
        self.state.open_call = Some(ordinal);
        self.popup_scroll = 0;
        self.dirty = true;
    }

    /// The call whose row is exactly under the pointer, or nothing: the same
    /// geometry the pane is drawn and hovered with, with no clamp.
    fn call_strictly_under_pointer(&self, layout: &view::Layout, space: Space) -> Option<usize> {
        let (row, at) = layout.cell_in(
            space,
            self.cursor.x as f32,
            self.cursor.y as f32,
            self.config.pane_font_size,
            self.pane_column,
        )?;
        let body = layout.content(space);
        let rows = layout.rows(body, self.config.pane_font_size);
        let (cols, chrome) = view::text_columns(View::Activity, body, self.pane_column);
        let (line, _) = self
            .state
            .activity
            .spot_in(rows, cols, row, at.saturating_sub(chrome))?;
        self.state.call_at_line(line)
    }

    /// Move the popup's window by pages of itself, the way every pane moves.
    fn scroll_popup(&mut self, layout: &view::Layout, pages: f32) {
        let geometry = {
            let frame = self.frame(layout);
            crate::widgets::popup::scroll_geometry(&frame)
        };
        let Some((total, fit)) = geometry else {
            return;
        };
        let by = ((fit.saturating_sub(1).max(1) as f32 * pages.abs()).round() as usize).max(1);
        let most = total.saturating_sub(fit);
        let next = match pages > 0.0 {
            true => self.popup_scroll.saturating_sub(by),
            false => (self.popup_scroll + by).min(most),
        };
        self.dirty |= next != self.popup_scroll;
        self.popup_scroll = next.min(most);
    }

    /// Walk one space's tab strip by one tab, which is what its arrows do. The
    /// rule is [`walk_tabs`]; this only tells it where the strip is now.
    fn walk_strip(&mut self, space: Space, forward: bool) {
        let showing = self.layout().placed(space).first_tab;
        self.dirty |= walk_tabs(&mut self.dock, space, showing, forward);
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
            self.prompt.caret(),
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
            self.selection,
            self.picker.as_ref(),
        );
        self.dirty |= had || self.menu.is_some();
    }

    /// Do what the row at `index` says, and put the menu away either way.
    ///
    /// A greyed row still closes it: leaving a menu open under a pointer that
    /// has already committed to a row reads as a click that missed the window.
    fn pick(&mut self, index: usize) {
        let Some(mut menu) = self.menu.take() else {
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
            // The panel is a takeover, so the menu goes with the press. It
            // opens on whatever section its rail was left on; the rail is
            // where a section is chosen.
            (Item::Settings, _) => {
                self.open_settings();
            }
            (Item::NewSession, _) => {
                self.new_session();
            }
            // The flyout header: its rows come out beside it and the menu
            // stays open over them. A press on a header the rollover already
            // opened leaves it open, or the click would take back what the
            // hover just did.
            (Item::Widgets(open), _) => {
                if !open {
                    menu.fold(index, &self.dock);
                }
                self.menu = Some(menu);
            }
            (Item::CopySelection, _) => {
                self.copy_selection();
            }
            (Item::Close, Target::Widget(view, _)) => self.close_view(view),
            // The prompt's menu has no Close row, so this cannot happen; it is
            // matched rather than caught by a wildcard so adding one is a
            // compile error here instead of a click that silently does nothing.
            (Item::Close, Target::Input | Target::Session(_) | Target::SettingsDoc) => {}
            // The same two things pressing the row and pressing Delete do, from
            // the menu the right button opened over it.
            (Item::OpenSession, Target::Session(index)) => self.open_session(index),
            // Delete is the one row here that is pressed twice. The first press
            // only arms it, and the menu stays open under the pointer so the
            // second press has something to land on; the rule is
            // [`Menu::press_delete`], and the panel's delete asks the same
            // question in the same words.
            (Item::DeleteSession(_), Target::Session(session)) => {
                match menu.press_delete(index) {
                    true => self.delete_session(session),
                    false => self.menu = Some(menu),
                }
            }
            // Neither row is on any menu but a session's.
            (Item::OpenSession | Item::DeleteSession(_),
                Target::Input | Target::Widget(..) | Target::SettingsDoc) => {}
            // A switch rather than a destination, so the menu stays open over it
            // and can be switched again. See [`toggle_view`] for the one case
            // where it cannot.
            (Item::Widget(view, _), _) => {
                let toggled = toggle_view(&mut self.dock, &mut menu, view);
                if toggled.hidden {
                    self.forget_selection_in(view);
                }
                if toggled.keep_open {
                    self.menu = Some(menu);
                }
            }
        }
    }

    /// Take a widget out of the window.
    ///
    /// The way back is the Widgets group on the same menu, and the two settings
    /// that carry a pane of their own. The arrangement survives a close because
    /// a space with no tabs gives its room to its neighbour rather than leaving
    /// a hole; see `Layout::compute`.
    fn close_view(&mut self, view: View) {
        if self.dock.hide(view) {
            self.forget_selection_in(view);
            self.save_dock();
            self.dirty = true;
        }
    }

    /// Write the arrangement down, so the next launch opens the panes where
    /// they were left. On every user change: a moved tab, a shown or closed
    /// widget, a switched tab. Never for the agent view's comings and goings,
    /// which are the session's, not the user's, and are not in the word anyway.
    fn save_dock(&mut self) {
        let Some(path) = config::path() else {
            return;
        };
        let _ = config::write_setting(&path, "dock", Some(&self.dock.arrangement()));
    }

    /// Right click > New session: stop this agent and go back to the first
    /// screen, where a folder or a saved session is chosen. Dropping the link
    /// stops the child; the window state that belonged to the session goes
    /// with it, and the picker owns the screen until something is chosen.
    fn new_session(&mut self) {
        self.link = None;
        self.state = State::new();
        self.state.day_zero = local_day_second();
        self.selection = None;
        self.scrolls = scroll::Scrolls::default();
        self.file_scroll = 0;
        self.last_open_file = 0;
        self.popup_scroll = 0;
        self.trouble = None;
        self.esc_armed = None;
        self.session = None;
        self.open_picker();
    }


    /// Drop a selection that belonged to a pane which is no longer on screen.
    /// Left behind it would still be what Ctrl-C copied, with nothing drawn
    /// anywhere saying so.
    fn forget_selection_in(&mut self, view: View) {
        if self
            .selection
            .is_some_and(|selection| selection.at == select::Where::Pane(view))
        {
            self.selection = None;
        }
    }

    /// Drop a selection made in the settings document.
    ///
    /// Called by everything on the panel that is not the document itself: which
    /// document is showing is a property of the row the cursor is on, so a key
    /// or a press that moves the cursor puts other text under the same line
    /// numbers, and a band left behind would be over words nobody highlighted.
    fn forget_doc_selection(&mut self) {
        if self.selection.is_some_and(|selection| {
            matches!(
                selection.at,
                select::Where::SettingsDoc | select::Where::SettingsPaper(_)
            )
        }) {
            self.selection = None;
            self.dirty = true;
        }
    }

    /// Put the clipboard into the prompt at the caret.
    fn paste(&mut self) {
        use copypasta::ClipboardProvider;
        if self.clipboard.is_none() {
            self.clipboard = copypasta::ClipboardContext::new().ok();
        }
        let now = self.now();
        let got = match self.clipboard.as_mut() {
            Some(clipboard) => clipboard.get_contents(),
            None => {
                self.state.noted(
                    now,
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
                    .noted(now, format!("nothing to paste: {e}"), state::Tone::Bad);
                self.dirty = true;
            }
        }
    }

    /// Press inside a pane: put the anchor where the pointer is, or clear the
    /// selection when the press is somewhere with no text to select.
    fn begin_selection(&mut self, space: Space) {
        let previous = self.selection.take();
        let Some(view) = self.dock.slot(space).active() else {
            self.dirty = previous.is_some();
            return;
        };
        let layout = self.layout();
        let spot = self.spot_at(&layout, space, view);
        self.selection = spot.map(|spot| select::Selection::new(select::Where::Pane(view), spot));
        self.selecting = self.selection.is_some();
        self.dirty = true;
    }

    /// The same for a press on the settings document. The panel covers every
    /// space, so this one is not begun through a space at all.
    fn begin_doc_selection(&mut self) {
        let layout = self.layout();
        let spot = self.doc_spot_at(&layout);
        self.selection = spot.map(|spot| select::Selection::new(select::Where::SettingsDoc, spot));
        self.selecting = self.selection.is_some();
        self.dirty = true;
    }

    /// A press inside one of the list's own document blocks: the prompt's
    /// files, which are read here far more often than they are written.
    fn begin_paper_selection(&mut self, index: usize) {
        let layout = self.layout();
        let spot = self.paper_spot_at(&layout, index);
        self.selection =
            spot.map(|spot| select::Selection::new(select::Where::SettingsPaper(index), spot));
        self.selecting = self.selection.is_some();
        self.dirty = true;
    }

    /// The character of one document block under the pointer.
    fn paper_spot_at(&self, layout: &view::Layout, index: usize) -> Option<select::Spot> {
        spot_in_paper(
            layout,
            self.settings.as_ref()?,
            index,
            self.cursor.x as f32,
            self.cursor.y as f32,
            self.config.pane_font_size,
            self.pane_column,
        )
    }

    /// The character of the settings document under the pointer.
    fn doc_spot_at(&self, layout: &view::Layout) -> Option<select::Spot> {
        spot_in_doc(
            layout,
            self.settings.as_ref()?,
            self.cursor.x as f32,
            self.cursor.y as f32,
            self.config.pane_font_size,
            self.pane_column,
        )
    }

    /// The character under the pointer, as a line and a column the pane can
    /// still resolve after it has scrolled.
    fn spot_at(&self, layout: &view::Layout, space: Space, view: View) -> Option<select::Spot> {
        let pane = self.state.pane_of(view)?;
        // The output pane is drawn at the transcript size, not the pane size.
        // Hit testing it with the smaller one put every click a growing number
        // of rows away from the character under the pointer.
        let (size, column) = self.metrics_of(view);
        let reserved = match view {
            View::Output => self
                .state
                .output_reserved(layout.rows(layout.content(space), size)),
            _ => 0,
        };
        spot_in_pane(
            layout,
            space,
            view,
            pane,
            self.cursor.x as f32,
            self.cursor.y as f32,
            size,
            column,
            reserved,
        )
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
        let Some(mut selection) = self.selection else {
            return;
        };
        let layout = self.layout();
        let spot = match selection.at {
            select::Where::Pane(view) => match self.dock_space_of(view) {
                Some(space) => self.spot_at(&layout, space, view),
                None => return,
            },
            select::Where::SettingsDoc => self.doc_spot_at(&layout),
            select::Where::SettingsPaper(index) => self.paper_spot_at(&layout, index),
            select::Where::CallPopup => {
                let frame = self.frame(&layout);
                crate::widgets::popup::spot_at(&frame, self.cursor.x as f32, self.cursor.y as f32)
            }
        };
        if let Some(spot) = spot {
            selection.extend(spot);
            // A drag over a pane is a selection, not a look at one call: the
            // popup the press put up goes away as soon as the drag has
            // selected something. A drag inside the popup is the opposite -
            // selecting its text is what the popup is open for.
            if !selection.is_empty() && selection.at != select::Where::CallPopup {
                self.state.open_call = None;
            }
            self.selection = Some(selection);
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
        let Some(selection) = self.selection else {
            return false;
        };
        let text = match selection.at {
            select::Where::Pane(view) => match self.state.pane_of(view) {
                Some(pane) => selection.text(pane),
                None => return false,
            },
            // Off the same pane the band was painted over, unscrolled: what a
            // copy returns is the characters between the two ends of the drag,
            // and where the column happens to be scrolled to is none of it.
            select::Where::SettingsDoc => match self.settings.as_ref() {
                Some(panel) => selection.text(&panel.doc_pane()),
                None => return false,
            },
            // The same, off the block the band was painted over: the whole of
            // its text, whatever the block happens to be scrolled to.
            select::Where::SettingsPaper(index) => match self.settings.as_ref() {
                Some(panel) => selection.text(&panel.paper_pane(index)),
                None => return false,
            },
            // Off the popup's own document, whole lines included however the
            // clip drew them.
            select::Where::CallPopup => match self.state.popped() {
                Some(call) => selection.text(&crate::widgets::popup::document(call)),
                None => return false,
            },
        };
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
        let now = self.now();
        match self.clipboard.as_mut() {
            Some(clipboard) => {
                // A clipboard that will not take it is worth saying out loud:
                // the alternative is a copy that silently did nothing.
                if let Err(e) = clipboard.set_contents(text) {
                    self.state.noted(
                        now,
                        format!("could not reach the clipboard: {e}"),
                        state::Tone::Bad,
                    );
                }
            }
            None => self.state.noted(
                now,
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
        self.thumbing = None;
        // A press on the title bar that never moved far enough to become a move
        // of the window. Nothing happens on the way up; it was a click, and a
        // second one of them shades.
        self.moving = None;
        // Here rather than on every motion event: a drag across the window is
        // hundreds of events, and rewriting the settings file at each one is
        // hundreds of rename-over-the-file writes for one decision.
        if let Some(grip) = self.sizing.take() {
            self.remember_divider(grip);
            return;
        }
        // And the same for a slider on the settings panel, for the same reason.
        if self.sliding.is_some() {
            self.drop_slider();
            return;
        }
        let layout = self.layout();
        if let Some(drag) = self.drag.take() {
            let landing = layout.landing(self.cursor.x as f32, self.cursor.y as f32);
            land(&mut self.dock, drag.view, landing);
            if self.dock.is_hidden(drag.view) {
                self.forget_selection_in(drag.view);
            }
            self.save_dock();
            self.holding = None;
            self.dirty = true;
            return;
        }
        if let Some((view, space, _)) = self.holding.take() {
            click_tab(self.dock.slot_mut(space), view);
            self.save_dock();
            self.dirty = true;
        }
    }

    /// Whether a press on a space's scroll gutter has a thumb to take, and
    /// which track it is: the explorer's own when the files pane is showing
    /// and the press is on its list, the pane's otherwise. Arms the drag
    /// when it does.
    fn begin_thumb(&mut self, space: Space) -> bool {
        let layout = self.layout();
        let Some(view) = self.dock.slot(space).active() else {
            return false;
        };
        // The explorer's track stands inside the files pane; the press's x
        // says which of the two tracks was taken.
        if view == View::Files
            && layout.files_in == Some(space)
            && layout.file_list.w >= 1.0
            && (self.cursor.x as f32) < layout.file_list.x + layout.file_list.w + view::SCROLL_GAP
        {
            let rows = layout.rows(layout.file_list, self.config.pane_font_size);
            if scroll::file_thumb(self.file_scroll, self.state.files.len(), rows).is_none() {
                return false;
            }
            self.thumbing = Some(Thumb::Explorer(space));
            return true;
        }
        let panel = layout.placed(space).body;
        let (size, column) = self.metrics_of(view);
        let reserved = match view {
            View::Output => self.state.output_reserved(layout.rows(panel, size)),
            _ => 0,
        };
        let rows = (layout.rows(panel, size) - reserved).max(1);
        let (cols, _) = view::text_columns(view, panel, column);
        let has = match self.state.pane_of(view) {
            Some(pane) => pane.thumb(rows, cols).is_some(),
            None => {
                let frame = self.frame(&layout);
                view::scroll_extent(&frame, view, panel)
                    .is_some_and(|(heights, fit)| self.scrolls.thumb(view, &heights, fit).is_some())
            }
        };
        if has {
            self.thumbing = Some(Thumb::Pane(space));
        }
        has
    }

    /// The pane under a held scrollbar follows the pointer down its track,
    /// through the same geometry the bar is drawn with.
    fn drag_thumb(&mut self) {
        let Some(grip) = self.thumbing else {
            return;
        };
        let layout = self.layout();
        // The popup's track first: while it is up it covers the panes.
        if matches!(grip, Thumb::Popup) {
            let track = view::scroll_track(layout.call_popup);
            let fraction = ((self.cursor.y as f32 - track.y) / track.h.max(1.0)).clamp(0.0, 1.0);
            let geometry = {
                let frame = self.frame(&layout);
                crate::widgets::popup::scroll_geometry(&frame)
            };
            let Some((total, fit)) = geometry else {
                return;
            };
            let most = total.saturating_sub(fit);
            let next = ((fraction * most as f32).round() as usize).min(most);
            self.dirty |= next != self.popup_scroll;
            self.popup_scroll = next;
            return;
        }
        let (space, explorer) = match grip {
            Thumb::Pane(space) => (space, false),
            Thumb::Explorer(space) => (space, true),
            Thumb::Popup => unreachable!("handled above"),
        };
        let Some(view) = self.dock.slot(space).active() else {
            return;
        };
        if explorer {
            let track = view::scroll_track(layout.file_list);
            let fraction = ((self.cursor.y as f32 - track.y) / track.h.max(1.0)).clamp(0.0, 1.0);
            let rows = layout.rows(layout.file_list, self.config.pane_font_size);
            let next = scroll::file_scroll_to(fraction, self.state.files.len(), rows);
            self.dirty |= next != self.file_scroll;
            self.file_scroll = next;
            return;
        }
        let panel = layout.placed(space).body;
        let track = view::scroll_track(panel);
        let fraction = ((self.cursor.y as f32 - track.y) / track.h.max(1.0)).clamp(0.0, 1.0);
        let (size, column) = self.metrics_of(view);
        let reserved = match view {
            View::Output => self.state.output_reserved(layout.rows(panel, size)),
            _ => 0,
        };
        let rows = (layout.rows(panel, size) - reserved).max(1);
        let (cols, _) = view::text_columns(view, panel, column);
        let open_file = self.state.open_file;
        let pane = match view {
            View::Output => Some(&mut self.state.output),
            View::Activity => Some(&mut self.state.activity),
            View::Files => self
                .state
                .files
                .get_mut(open_file)
                .map(|file| &mut file.pane),
            View::Agent => self.state.agent_shown_mut().map(|agent| &mut agent.pane),
            _ => None,
        };
        if let Some(pane) = pane {
            self.dirty |= pane.scroll_to(fraction, rows, cols);
            return;
        }
        let extent = {
            let frame = self.frame(&layout);
            view::scroll_extent(&frame, view, panel)
        };
        let Some((heights, fit)) = extent else {
            return;
        };
        self.dirty |= self.scrolls.scroll_to(view, fraction, &heights, fit);
    }

    /// Move the divider under the button to where the pointer is now.
    ///
    /// No slop to get past first, unlike a held tab: a divider has no second
    /// meaning that a small movement could be mistaken for, and a press that
    /// never moves writes the ratio it already had.
    fn drag_divider(&mut self) {
        let Some(grip) = self.sizing else {
            return;
        };
        let layout = self.layout();
        let (x, y) = (self.cursor.x as f32, self.cursor.y as f32);
        // The half the press landed on and no other: dragging one line leaves
        // the one beside it exactly where it was.
        let (slot, next) = match grip {
            Hit::RowDivider(half) => (
                &mut self.top_height[half],
                layout.row_ratio_at(half, y),
            ),
            Hit::ColumnDivider(half) => (
                &mut self.left_width[half],
                layout.column_ratio_at(half, x),
            ),
            // The settings panel's rail, which is dragged by the same three
            // steps: live while the pointer moves, written on the way up.
            Hit::SettingsRailDivider => {
                (&mut self.settings_rail, layout.settings_rail_ratio_at(x))
            }
            // `sizing` is only ever one of the three.
            _ => return,
        };
        if (*slot - next).abs() < f32::EPSILON {
            return;
        }
        *slot = next;
        self.dirty = true;
    }

    /// Where the divider under `grip` is now, as the fraction its key holds.
    fn divider_ratio(&self, grip: Hit) -> Option<f32> {
        match grip {
            Hit::ColumnDivider(half) => self.left_width.get(half).copied(),
            Hit::RowDivider(half) => self.top_height.get(half).copied(),
            Hit::SettingsRailDivider => Some(self.settings_rail),
            _ => None,
        }
    }

    /// Write where a divider was left into the settings file.
    ///
    /// Through the same writer every other setting goes through, so the comments
    /// in the file survive a drag. A file that cannot be written is said once, in
    /// the activity pane, rather than silently losing the arrangement.
    fn remember_divider(&mut self, grip: Hit) {
        let (Some(key), Some(ratio)) = (divider_key(grip), self.divider_ratio(grip)) else {
            return;
        };
        // Three places is finer than a pixel on any window this can be dragged
        // in, so what is written and what is on screen are the same arrangement.
        let value = format!("{ratio:.3}");
        // The panel's own rail goes through the panel, because it is the one
        // line that can be dragged while the panel is up: the row that carries
        // the same key has to read back as where the drag left it, and a write
        // that failed has to be said on the panel rather than in the activity
        // pane behind it.
        if grip == Hit::SettingsRailDivider {
            self.write_setting(&settings::Change {
                key,
                value,
                file: settings::File::Window,
            });
            return;
        }
        let Some(path) = config::path() else {
            return;
        };
        let now = self.now();
        match config::write_setting(&path, key, Some(&value)) {
            Ok(()) => match key {
                "left_width" => self.config.left_width = ratio,
                "left_width_bottom" => self.config.left_width_bottom = ratio,
                "top_height" => self.config.top_height = ratio,
                // The rail is written above, so the fourth grid line is all
                // that is left to be.
                _ => self.config.top_height_right = ratio,
            },
            Err(why) => {
                self.state
                    .noted(now, format!("cannot save the layout: {why}"), state::Tone::Bad);
            }
        }
        self.dirty = true;
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
            // The same question the release asks, asked on every move: what the
            // frame draws as the target is what a drop would actually do.
            landing: layout.landing(self.cursor.x as f32, self.cursor.y as f32),
        });
        self.dirty = true;
    }

    /// The keys an open menu answers, and whether this was one of them.
    ///
    /// The menu shipped with no keyboard route at all: any key put it away. A
    /// menu whose rows open groups needs one, because opening a group with the
    /// pointer moves the rows under the pointer, and because a row that opens
    /// and a row that acts are two different presses and only the keyboard can
    /// say which without aiming.
    ///
    /// Up and down walk the rows that can act. Right opens the group the cursor
    /// is on and steps into one already open; left shuts it, or steps out to
    /// the header of the group the cursor is inside. Enter presses the row.
    /// Everything else falls through and puts the menu away.
    fn menu_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        let rows = self.layout().menu_capacity();
        let Some(menu) = self.menu.as_mut() else {
            return false;
        };
        let dock = &self.dock;
        let mut act = None;
        let answered = match event.logical_key.as_ref() {
            Key::Named(NamedKey::ArrowDown) => {
                menu.walk(true, rows);
                true
            }
            Key::Named(NamedKey::ArrowUp) => {
                menu.walk(false, rows);
                true
            }
            Key::Named(NamedKey::ArrowRight) => {
                menu.unfold_here(dock, rows);
                true
            }
            Key::Named(NamedKey::ArrowLeft) => {
                menu.fold_here(dock, rows);
                true
            }
            Key::Named(NamedKey::Enter | NamedKey::Space) => {
                act = menu.cursor;
                act.is_some()
            }
            _ => false,
        };
        if !answered {
            return false;
        }
        // The keys are saying which row is next now, so the pointer's own
        // highlight goes out. Two lit rows in one menu is two answers to the
        // question Enter is about to ask.
        self.hot = None;
        self.dirty = true;
        if let Some(index) = act {
            self.pick(index);
        }
        true
    }

    fn key(&mut self, event: winit::event::KeyEvent, event_loop: &ActiveEventLoop) {
        if event.state != ElementState::Pressed {
            return;
        }
        // An open menu takes the keys it can use before anything else does, the
        // same way it takes the click: it is above the window. Everything else
        // puts it away, because a menu left floating over text being typed
        // under it is worse than losing it, and Escape stops there rather than
        // falling through, so putting it away does not also drop a selection or
        // a half typed line.
        if self.menu.is_some() && self.menu_key(&event) {
            return;
        }
        if self.menu.take().is_some() {
            self.dirty = true;
            if matches!(event.logical_key.as_ref(), Key::Named(NamedKey::Escape)) {
                return;
            }
        }
        // The activity popup closes the same way, and for the same reason: it is
        // a box floating over the window for a pointer, and the keyboard has
        // moved on. Escape stops here so putting it away does not also drop a
        // selection or cancel the turn.
        if self.state.open_call.take().is_some() {
            self.dirty = true;
            if matches!(event.logical_key.as_ref(), Key::Named(NamedKey::Escape)) {
                return;
            }
        }
        if self.picker.is_some() {
            self.key_in_picker(&event, event_loop);
            return;
        }
        if self.settings.is_some() {
            self.key_in_settings(&event, event_loop);
            return;
        }
        let ctrl = self.modifiers.control_key();
        // Any key that is not the second ESC stands down a pending cancel: the
        // hand has moved on to something else, and the arm must not sit there
        // waiting to spend a stray ESC minutes later.
        if !matches!(event.logical_key.as_ref(), Key::Named(NamedKey::Escape)) {
            self.dirty |= self.esc_armed.take().is_some();
        }
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
            // reaches for the turn when there is neither. Losing a typed prompt
            // to a key meant for the agent is the worse mistake of the two, and
            // losing a turn to one meant for a selection is worse still. The
            // turn itself takes two: the first ESC arms and the input row says
            // so, the second inside the window cancels.
            Key::Named(NamedKey::Escape) => {
                if self.selection.take().is_some() {
                    self.dirty = true;
                } else if self.prompt.is_empty() {
                    let now = Instant::now();
                    if self.esc_armed.is_some_and(|until| now < until) {
                        self.esc_armed = None;
                        self.cancel();
                    } else {
                        self.esc_armed = Some(now + ESC_CANCEL_WINDOW);
                        self.dirty = true;
                    }
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
                if self.dock.slot_mut(space).cycle() {
                    self.save_dock();
                    self.dirty = true;
                }
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
                    self.save_dock();
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
        // The popup covers the panes, so while it is up the wheel is its.
        if self.state.open_call.is_some() {
            self.scroll_popup(&layout, pages);
            return;
        }
        // A menu floats above the window, so while the pointer is on one the
        // wheel moves the menu rather than the pane it covers. First, for the
        // same reason the menu is hit tested first.
        if matches!(
            layout.hit(self.cursor.x as f32, self.cursor.y as f32),
            Some(Hit::Menu | Hit::MenuRow(_))
        ) && let Some(menu) = self.menu.as_mut()
        {
            let rows = layout.menu_capacity();
            let by = ((rows as f32 * pages.abs()).round() as usize).max(1);
            self.dirty |= menu.scroll(by, pages < 0.0, rows);
            return;
        }
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
        // And the same for the settings panel, which is the only thing on screen
        // while it is up.
        if self.settings.is_some() {
            // Which row of the list the pointer is on, if any. A block of text
            // and a table both scroll inside themselves and both answer for the
            // rows in them, so all three hits name the row the wheel is over.
            // Which of the three regions the wheel then moves is
            // [`Settings::wheel`], because the choice is the model's and this is
            // the only place in the window that knows where the pointer is.
            let over = match layout.hit(self.cursor.x as f32, self.cursor.y as f32) {
                Some(
                    Hit::SettingsRow(index, _)
                    | Hit::SettingsPick(index, _)
                    | Hit::SettingsMark(index, _),
                ) => Some(index),
                _ => None,
            };
            // Over the column beside the entry list, the wheel moves that
            // document rather than the list: the pointer is on the thing being
            // scrolled, which is what every two-column view in this window does.
            // The document is counted in the rows and columns it is drawn in,
            // which is the box inside its wrapper rather than the whole column:
            // a page that moved the text by more rows than the box shows skips
            // lines nobody read. A column too short to hold a row of text is not
            // scrolled at all, since nothing in it is on screen to scroll.
            let doc = layout.settings_doc;
            let doc_rows = layout.settings_doc_rows(self.config.pane_font_size);
            let doc_cols = layout.settings_doc_columns(self.pane_column);
            let on_doc = doc.w >= 1.0
                && doc_rows > 0
                && doc.contains(self.cursor.x as f32, self.cursor.y as f32);
            let rows = layout.settings_capacity(self.config.pane_font_size);
            let by = ((doc_rows as f32 * pages.abs()).round() as usize).max(1);
            let list_cols = layout.settings_entry_columns(self.pane_column);
            if let Some(panel) = self.settings.as_mut() {
                self.dirty |= match on_doc {
                    true => panel.scroll_doc(by, pages < 0.0, doc_cols, doc_rows),
                    false => panel.wheel(over, pages, rows, list_cols),
                };
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
            {
                let next = scroll::scroll_files(
                    self.file_scroll,
                    by,
                    pages < 0.0,
                    self.state.files.len(),
                    rows,
                );
                self.dirty |= next != self.file_scroll;
                self.file_scroll = next;
            }
            return;
        }
        let Some((view, panel)) = self.under_pointer(&layout) else {
            return;
        };
        let (size, column) = self.metrics_of(view);
        // The OUTPUT pane's bottom rows belong to the queued messages while
        // any wait, so the wheel pages by the rows the transcript really has.
        let reserved = match view {
            View::Output => self.state.output_reserved(layout.rows(panel, size)),
            _ => 0,
        };
        let rows = (layout.rows(panel, size) - reserved).saturating_sub(1).max(1);
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
            View::Agent => self.state.agent_shown_mut().map(|agent| &mut agent.pane),
            _ => None,
        };
        // A pane with a scrollback of its own, which is a transcript: it follows
        // the live end, so its position is counted back from there.
        if let Some(pane) = pane {
            self.dirty |= if pages > 0.0 {
                pane.scroll_back(by, rows, cols)
            } else {
                pane.scroll_forward(by)
            };
            return;
        }
        // Everything else is a list, scrolled from the top. The extent is asked
        // for rather than worked out here, because a page of a monitor pane is a
        // page of its own rows, which are taller than a line of text.
        let extent = {
            let frame = self.frame(&layout);
            view::scroll_extent(&frame, view, panel)
        };
        let Some((heights, fit)) = extent else {
            return;
        };
        let by = ((fit.saturating_sub(1).max(1) as f32 * pages.abs()).round() as usize).max(1);
        self.dirty |= self.scrolls.scroll(view, by, pages < 0.0, &heights, fit);
    }

    /// Everything a frame is drawn from, for a layout that has already been
    /// computed.
    ///
    /// One builder, because the wheel and the per-frame clamp need the same
    /// bundle the drawing does: how tall a pane's content is depends on the skin,
    /// the monitor and the pane metrics, and a second hand-rolled copy of it is a
    /// pane that scrolls by a different number of rows than it drew.
    fn frame<'a>(&'a self, layout: &'a Layout) -> view::Frame<'a> {
        view::Frame {
            state: &self.state,
            monitor: &self.monitor,
            dock: &self.dock,
            skin: &self.skin,
            layout,
            prompt: &self.prompt,
            column: self.column,
            pane_column: self.pane_column,
            body_size: self.config.font_size,
            pane_size: self.config.pane_font_size,
            drag: self.drag,
            hot: self.hot,
            trouble: self.trouble.as_deref(),
            esc_armed: self.esc_armed.is_some(),
            popup_scroll: self.popup_scroll,
            cursor: (self.cursor.x as f32, self.cursor.y as f32),
            selection: self.selection,
            scrolls: &self.scrolls,
            file_scroll: self.file_scroll,
            // The same menu the layout was computed from, or the rows would be
            // drawn somewhere other than where they are hit tested.
            menu: self.menu.as_ref(),
            picker: self.picker.as_ref(),
            settings: self.settings.as_ref(),
            // The orb's clock, and how far it is through the move between its
            // two formations. Read here rather than inside the scene, so a
            // frame stays a function of what it is handed.
            clock: self.epoch.elapsed().as_secs_f32(),
            orb_morph: self.orb.showing(),
        }
    }

    fn render(&mut self) {
        if self.gpu.is_none() || self.renderer.is_none() {
            return;
        }
        // Computed before the surface is borrowed: the prompt's height is read
        // off the whole app and the renderer holds it mutably.
        let layout = self.layout();
        // A table is laid out for the panel it is in, so it is settled before
        // the pane is measured: its rows are what the scroll extent, the bands
        // and the clipboard are all counted in.
        self.lay_out_tables(&layout);
        // Every scrolling pane is clamped against what it currently holds, before
        // anything is drawn from it. This is the only place that catches a window
        // dragged shorter or a list that shrank while it was scrolled to the end,
        // neither of which goes anywhere near the pointer.
        self.settle_scrolls(&layout);
        let scene = view::build(&self.frame(&layout));
        let (Some(gpu), Some(renderer)) = (self.gpu.as_mut(), self.renderer.as_mut()) else {
            return;
        };
        let Some(frame) = gpu.acquire() else {
            return;
        };
        renderer.draw(gpu, &scene, frame);
        self.dirty = false;
    }

    /// Lay the transcript's tables out for the panel the transcript is in.
    ///
    /// Only while it is on screen: a folded space has no width to lay anything
    /// out for, and the block is laid out again the moment it comes back.
    fn lay_out_tables(&mut self, layout: &Layout) {
        let Some(space) = Space::ALL.into_iter().find(|space| {
            let slot = self.dock.slot(*space);
            slot.active() == Some(View::Output) && !slot.folded
        }) else {
            return;
        };
        let panel = layout.placed(space).body;
        let (_, column) = self.metrics_of(View::Output);
        let (cols, _) = view::text_columns(View::Output, panel, column);
        self.dirty |= self.state.output.reflow(cols);
    }

    /// Pull every scrolling pane's offset back inside the content it is showing.
    ///
    /// Only the panes on screen: a folded space or a covered window has nothing
    /// to clamp against, and clamping a pane by the box it does not currently have
    /// would lose the reader's place.
    fn settle_scrolls(&mut self, layout: &Layout) {
        let mut want = Vec::new();
        {
            let frame = self.frame(layout);
            for space in Space::ALL {
                let slot = self.dock.slot(space);
                let Some(view) = slot.active().filter(|_| !slot.folded) else {
                    continue;
                };
                let panel = layout.placed(space).body;
                if let Some((heights, rows)) = view::scroll_extent(&frame, view, panel) {
                    want.push((view, heights, rows));
                }
            }
        }
        for (view, heights, rows) in want {
            self.dirty |= self.scrolls.settle(view, &heights, rows);
        }
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
                let now = self.now();
                for key in &self.config.unknown {
                    self.state
                        .noted(now, format!("settings: {key:?} is not a setting"), Tone::Bad);
                }
                let mut renderer = noob_draw::Renderer::new(&gpu);
                self.column = renderer.column_width(self.config.font_size);
                self.pane_column = renderer.column_width(self.config.pane_font_size);
                self.menu_column = renderer.column_width(menu::paint::MENU_SIZE);
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
        // there is nothing to ask: the newest session saved for that folder
        // carries on, and only a folder with none starts fresh. Without one,
        // the picker is the first thing on screen and it calls `connect`
        // itself; right click > New session is the way back to it.
        match self.workspace.take() {
            Some(workspace) => {
                let session = sessions::latest_for(&self.saved_sessions(), &workspace);
                self.connect(Chosen { workspace, session });
            }
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
                // Before the frame, not after it: this is the one place the
                // window finds out what the compositor did with a shade, and a
                // surface that is no longer a strip has to stop being drawn as
                // one on this frame rather than the next.
                self.reconcile_shade(size.height);
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
                // A press on the title bar becomes a move of the window here,
                // once the pointer has left the place it went down.
                if let Some(window) = self.window.clone() {
                    self.maybe_move(&window);
                }
                self.maybe_drag();
                self.drag_divider();
                self.drag_slider();
                self.drag_thumb();
                if self.selecting {
                    self.extend_selection();
                }
                if self.prompt_selecting {
                    self.prompt.drag_to(self.caret_under_pointer());
                    self.dirty = true;
                }
                let (x, y) = (position.x as f32, position.y as f32);
                let layout = self.layout();
                let under = layout.hit(x, y);
                let hot = match under {
                    Some(
                        hit @ (Hit::Close
                        | Hit::Maximize
                        | Hit::Minimize
                        | Hit::MenuRow(_)
                        | Hit::PickerOpen
                        | Hit::PickerFolders
                        | Hit::PickerSessions
                        | Hit::PickerMark(_)
                        | Hit::SettingsClose
                        | Hit::CallPopupClose
                        | Hit::SettingsSection(_)
                        | Hit::SettingsSlider(..)
                        | Hit::SettingsSwatch(_, _)
                        | Hit::SettingsChoice(..)
                        | Hit::SettingsToggle(_)
                        | Hit::SettingsRemove(_)
                        | Hit::SettingsMark(..)
                        | Hit::SettingsAct(..)
                        | Hit::SettingsValue(..)),
                    ) => Some(hit),
                    _ => None,
                };
                if hot != self.hot {
                    self.hot = hot;
                    self.dirty = true;
                }
                // The activity list lights the row under the pointer, and it
                // works that out while drawing, so nothing above marks the
                // frame dirty for it. Without this the highlight waited for
                // whatever redrew next (a monitor sample, a line of output),
                // which is seconds of a row not answering the hand.
                if let Some(space) = self.dock_space_of(View::Activity)
                    && layout.placed(space).body.contains(x, y)
                {
                    self.dirty = true;
                }
                // An armed Delete on an open menu is disarmed by the pointer
                // leaving its row, so the second press can only be made by a
                // pointer that is still on the row which asked for it.
                if let Some(menu) = self.menu.as_mut() {
                    self.dirty |= menu.hover(
                        match under {
                            Some(Hit::MenuRow(row)) => Some(row),
                            _ => None,
                        },
                        &self.dock,
                    );
                }
                // The pointer shape is the only thing telling a user that an
                // undecorated window can be resized at all, and the only thing
                // saying that a tab let go outside the window is closed.
                if let Some(window) = &self.window {
                    window.set_cursor(cursor_for(
                        self.drag.is_some(),
                        layout.landing(x, y),
                        view::edge(x, y, layout.width, layout.height),
                        // The divider being dragged, if there is one, before
                        // whatever the pointer is over: a drag that ran off the
                        // divider is still that drag.
                        self.sizing.or(under),
                    ));
                }
                self.redraw();
            }
            WindowEvent::CursorLeft { .. } => {
                let mut changed = self.hot.take().is_some();
                // The pointer left the window, which is further away than the
                // other rows: an armed Delete goes back to asking.
                if let Some(menu) = self.menu.as_mut() {
                    changed |= menu.point_at(None);
                }
                if changed {
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
        let covered = self.picker.is_some() || self.settings.is_some();
        if sampling(self.shaded, covered, &self.dock) {
            let now = Instant::now();
            if self.next_sample.is_none_or(|at| now >= at) {
                // The state and nothing else. Every number the monitor draws is
                // this run: nothing on screen outlives the window any more.
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
        //
        // The move between the two formations is stepped first, because the
        // clock outlives the turn by exactly as long as the orb takes to travel
        // back to its square: the frames it needs on the way out are already
        // there, since the turn is running the whole time.
        let now = Instant::now();
        self.orb.step(self.state.phase.busy(), now);
        let next = orb_deadline(now, self.state.phase.busy() || self.orb.moving(), self.next_orb);
        self.dirty |= next.is_some() && next != self.next_orb;
        self.next_orb = next;
        // A first ESC nobody followed lapses here, taking its hint with it.
        if self.esc_armed.is_some_and(|until| now >= until) {
            self.esc_armed = None;
            self.dirty = true;
        }
        // A prompt that went out and started nothing. Said once, and the phase
        // goes back to where another prompt can be typed: an orb turning over a
        // conversation that has stopped is the window lying about what is
        // happening.
        if self.answer_by.is_some_and(|by| now >= by) {
            self.answer_by = None;
            self.state.output.say(unanswered(), Tone::Bad);
            if self.state.phase == state::Phase::Thinking {
                self.state.phase = state::Phase::Ready;
                self.state.status = String::from("ready");
            }
            self.dirty = true;
        }
        self.redraw();
        // Every clock, never one of them: the earliest deadline wins and the
        // others are still there when it comes round.
        event_loop.set_control_flow(match soonest([
            self.next_sample,
            self.next_orb,
            self.esc_armed,
            self.answer_by,
        ]) {
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




fn main() {
    // `no0b --set theme=noob-red` edits the settings file and exits. Not an
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

mod shell;
#[allow(clippy::wildcard_imports)]
use shell::*;

#[cfg(test)]
mod tests;
