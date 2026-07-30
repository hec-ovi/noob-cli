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
mod scroll;
mod select;
mod sessions;
mod settings;
mod skin;
mod state;
mod syntax;
mod totals;
mod view;

use std::path::{Path, PathBuf};
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
use picker::{Chosen, Picker};
use settings::Settings;
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
///
/// `covered` is either takeover, the folder picker or the settings panel. Both
/// draw over every pane, so a monitor behind one is not on screen and the clock
/// that reads the kernel for it has nothing to feed.
fn sampling(shaded: bool, covered: bool, dock: &Dock) -> bool {
    !shaded
        && !covered
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

/// Everything shading asks the window for at once, so the order the three are
/// asked in lives in one place.
#[derive(Debug, PartialEq)]
struct ShadeRequest {
    /// The minimum inner size to hold the window to, or none at all.
    min: Option<LogicalSize<f64>>,
    /// The inner size to become, if a size is asked for.
    size: Option<PhysicalSize<u32>>,
    /// The maximized state to put the window in, when it has to change.
    maximized: Option<bool>,
}

/// What shading asks the window for. Split out from [`App::shade`] so the rule
/// can be tested without a compositor.
///
/// Shaded there is no minimum at all. `MIN_SIZE` is taller than the strip, and
/// a window that keeps its minimum while shaded simply does not shrink.
///
/// The height asked for is `view::strip_height`, in physical pixels, which is
/// the one number the strip is drawn from. Physical because that is the space
/// the layout works in: `Layout::compute` is handed the surface configuration,
/// which is `Window::inner_size` verbatim, and nothing on the way applies a
/// scale factor. A logical request would come back multiplied by that factor,
/// and the strip would be drawn across the top of a surface twice its height.
///
/// `maximized` is whether the window is maximized in the open state it is
/// leaving or coming back to: read off the window while shading, remembered
/// while unshading. A maximized window ignores a resize request, so it has to
/// leave that state to become a strip, and shading is not a way of un-maximizing
/// a window, so opening the strip puts it back. Unshading into maximized asks
/// for no size at all: the compositor owns the size of a maximized window and a
/// request beside it is a second answer to a question already settled.
#[allow(dead_code)]
fn shade_request(
    shaded: bool,
    remembered: Option<PhysicalSize<u32>>,
    maximized: bool,
) -> ShadeRequest {
    match (shaded, remembered) {
        (true, was) => ShadeRequest {
            min: None,
            size: was.map(|was| PhysicalSize::new(was.width, view::strip_height() as u32)),
            maximized: maximized.then_some(false),
        },
        (false, was) => ShadeRequest {
            min: Some(MIN_SIZE),
            size: if maximized { None } else { was },
            maximized: maximized.then_some(true),
        },
    }
}

/// What a shaded window turned out to be, read off the surface the compositor
/// handed back. See [`shade_of`].
#[derive(Debug, PartialEq, Clone, Copy)]
enum Shade {
    /// Not shaded, or not shaded any longer because the surface says otherwise.
    Open,
    /// Shaded, and the surface is the strip.
    Strip,
    /// Shaded, and this surface is the window leaving maximized on its way to
    /// the strip. The strip has to be asked for again, now that the window is
    /// in a state that can take the request.
    Asking,
    /// Shaded, and the window has been asked to leave maximized but has not left
    /// it yet. Nothing a maximized window says about its size answers anything
    /// about the strip, so this waits for the surface that comes after it.
    Leaving,
}

/// Whether a window that thinks it is shaded still is, read off the surface the
/// compositor actually handed back.
///
/// Shading is a request, and a compositor is free to answer it with something
/// else. Dragging a window by its title bar near the top of the screen is a
/// maximize gesture on GNOME, so the press that begins a move can leave the
/// window maximized, and a maximized window ignores `request_inner_size`: the
/// strip is asked for, the surface stays full screen, and the title bar is
/// painted across the whole of it. Rather than predict what a compositor
/// decided, this reads what it did. A shaded window that came back maximized, or
/// simply far taller than a strip, is not shaded any more and the state is
/// dropped.
///
/// `strip.saturating_mul(2)` is the line, and both sides of it have a reason. A
/// surface a few pixels off the strip is a compositor rounding a request to
/// whole scaled pixels or to its own increment, and it is still a strip. Two
/// title bars is not something rounding produces, and it is far below `MIN_SIZE`,
/// which is the shortest an open window can be asked for, so nothing between the
/// two is a window state either.
///
/// `settling` is the one span where a surface is not an answer, and it is
/// deliberately the narrowest one: shading a maximized window asks it to leave
/// that state first, and both the refusal it gives while it is still maximized
/// ([`Shade::Leaving`]) and the restored size that arrives once it has left
/// ([`Shade::Asking`]) are the round trip rather than a verdict. It is set only
/// for that case. Shading a window that was not maximized sets no `settling` at
/// all, so the first surface that comes back is read, and a compositor that
/// refuses the strip is answered on the spot.
fn shade_of(shaded: bool, maximized: bool, settling: bool, height: u32, strip: u32) -> Shade {
    if !shaded {
        return Shade::Open;
    }
    if maximized {
        // Nothing a maximized window reports is about the strip: while its own
        // un-maximize is in flight it is still on the way there, and otherwise
        // it is a maximized window, which is not a shaded one.
        return if settling { Shade::Leaving } else { Shade::Open };
    }
    if height <= strip.saturating_mul(2) {
        return Shade::Strip;
    }
    if settling {
        return Shade::Asking;
    }
    Shade::Open
}

/// Whether a press on the title bar has become a move of the window.
///
/// The title bar both moves the window and maximizes it, and the compositor's
/// interactive move is the one that cannot be taken back: once `drag_window` is
/// called the pointer belongs to the compositor, the second click of a double
/// click never arrives here, and on GNOME a pointer near the top of the screen
/// has already snapped the window maximized by then. So a press waits, the same
/// `DRAG_SLOP` a held tab waits for, and a double click that never moves the
/// pointer never reaches the compositor at all.
fn began_move(pressed: bool, moved: f64) -> bool {
    pressed && moved >= DRAG_SLOP
}

/// What a press on the title bar turns out to be. See [`title_click`].
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum TitleClick {
    /// The second click of a pair: put the window in or out of maximized.
    Maximize,
    /// A single press. Nothing happens on it; whether it becomes a move of the
    /// window is decided later by the pointer, in [`App::maybe_move`].
    ArmMove,
}

/// What a click on the title bar does.
///
/// A double click is the desktop's own maximize toggle, the same thing the
/// maximize button does, so the bar behaves the way every other window on the
/// desktop does. It used to collapse the window to its strip; that path is
/// still in this file and nothing reaches it any more.
///
/// A free function because [`App::click`] needs a live window and cannot be
/// driven in a test, the same reason [`began_move`] and [`shade_of`] are out
/// here.
fn title_click(double: bool) -> TitleClick {
    if double {
        TitleClick::Maximize
    } else {
        TitleClick::ArmMove
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
        // A pane, the rows of its own file list and the arrows of its own strip
        // are all the same widget: the menu acts on whatever that space is
        // showing.
        Hit::Body(space) | Hit::File(_, space) | Hit::TabsLeft(space) | Hit::TabsRight(space) => {
            widget(dock.slot(space).active()?, space)
        }
        Hit::TitleBar | Hit::Close | Hit::Maximize | Hit::Minimize => None,
        // The menu already open. Its own right click is handled before this is
        // reached, and a row is picked with the left button.
        Hit::Menu | Hit::MenuRow(_) => None,
        // The picker is not a widget: there is no pane to close, no settings
        // behind it, and nothing in it to select.
        Hit::Picker
        | Hit::PickerRow(_)
        | Hit::PickerMark(_)
        | Hit::PickerOpen
        | Hit::PickerSessions => None,
        // Neither is the settings panel. A Settings row on a menu opened over
        // the settings panel would be a row that opens what is already open,
        // and there is no pane behind it to close.
        Hit::Settings
        | Hit::SettingsSection(_)
        | Hit::SettingsRow(_)
        | Hit::SettingsValue(_)
        | Hit::SettingsSlider(_)
        | Hit::SettingsClose => None,
        // A divider is the gap between two widgets and belongs to neither of
        // them, so there is no one widget for a menu opened here to act on.
        Hit::ColumnDivider | Hit::RowDivider => None,
    }
}

/// What picking a row of the menu's widget list does, and what becomes of the
/// menu afterwards.
struct Toggled {
    /// The widget went out of the window rather than coming back into it.
    hidden: bool,
    /// The menu can stay open. It cannot when the widget that went out is the
    /// one the menu was opened over: its Close row and its Copy row would be
    /// pointed at a pane that is no longer in the window.
    keep_open: bool,
}

/// Picking a widget hides it or shows it. The list is a set of switches rather
/// than a set of destinations: a widget in the window goes out, and one that is
/// out comes back into the space it opens in by default. Where it used to be is
/// not remembered, and an arrangement dragged around since it went would have
/// nowhere to put it back.
///
/// The menu stays open over it, with its marks read off the dock again, so a
/// second widget can be switched without opening the menu and its list a second
/// time. The one exception is the menu's own widget going out, which takes the
/// thing the rest of the menu acts on with it.
///
/// A free function over the dock and the menu, like [`menu_for`] above it, so
/// the rule can be tested without a window.
fn toggle_view(dock: &mut Dock, menu: &mut Menu, view: View) -> Toggled {
    let hidden = !dock.is_hidden(view);
    match hidden {
        true => dock.hide(view),
        false => dock.unhide(view),
    };
    menu.relist(dock);
    Toggled {
        hidden,
        keep_open: !(hidden && menu.target_view() == Some(view)),
    }
}

/// Which views a settings change turns on or off.
///
/// Only the ones whose own setting moved. Applying both flags on every change
/// would put back a widget that was closed by hand, since closing one does not
/// write anything to the file: turn the font size up once and ACTIVITY comes
/// back, which is not what either action asked for.
///
/// Pure so the rule can be tested without a window, like [`land`] beside it.
fn pane_changes(was: &Config, now: &Config) -> Vec<(View, bool)> {
    [
        (View::Activity, was.show_activity, now.show_activity),
        (View::Files, was.show_files, now.show_files),
    ]
    .into_iter()
    .filter(|(_, was, now)| was != now)
    .map(|(view, _, now)| (view, now))
    .collect()
}

/// Where a tab strip starts after one of its arrows is clicked.
///
/// `showing` is the tab the strip actually starts at this frame, not the number
/// stored for it: a resize or a closed tab can have clamped the strip since, and
/// stepping from the stored number would spend clicks catching up with what is on
/// screen before anything moved. Clamped to the tabs there are for the same
/// reason the slot clamps: a strip cannot be walked past its last tab.
///
/// Pure so the rule can be tested without a window, like [`land`] beside it.
fn walked(showing: usize, forward: bool, tabs: usize) -> usize {
    match forward {
        true => (showing + 1).min(tabs.saturating_sub(1)),
        false => showing.saturating_sub(1),
    }
}

/// One click on one of a strip's arrows: the strip moves by a tab, and the tab it
/// is showing moves with it. Says whether anything moved.
///
/// The showing tab comes along because the layout puts a strip back where the
/// showing tab is on the frame after it is scrolled away from it (see
/// `view::strip_tabs`), so an arrow that only scrolled would do nothing at all
/// while the leftmost tab was the one showing, which is the state the window
/// opens in.
///
/// `showing` is where the strip actually starts this frame, which only the layout
/// knows. Pure so the rule can be tested without a window, like [`land`] beside
/// it.
fn walk_tabs(dock: &mut Dock, space: Space, showing: usize, forward: bool) -> bool {
    let slot = dock.slot_mut(space);
    let tabs = slot.views.len();
    let Some(active) = slot.active_index() else {
        return false;
    };
    let stepped = slot.scroll_tabs(walked(showing, forward, tabs));
    let showed = slot.show_at(walked(active, forward, tabs));
    stepped || showed
}

/// What a released tab does to the arrangement.
///
/// A drop on a tab strip names a place among that space's tabs, so it reorders
/// them; one inside a cell of the grid names that cell and puts the tab at the
/// end of it; one on the line between two cells merges the pair and gives the
/// pane both. Off the window closes the widget. Pure so the rule can be tested
/// without a compositor, and so the one place a drop changes the dock is the one
/// place a test drives.
fn land(dock: &mut Dock, view: View, landing: Landing) -> bool {
    match landing {
        Landing::In(space, Some(at)) => dock.place_view(view, space, at),
        Landing::In(space, None) => dock.move_view(view, space),
        Landing::Span(a, b) => dock.span_view(view, a, b),
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

/// The whole pixel-to-character path for one pane, free of the window so it
/// can be driven by a test.
///
/// The pane is given rather than looked up, because both callers already know
/// which one they mean: a press picked it, and a drag belongs to the pane it
/// began in. That is also why the point is clamped into the box instead of
/// being refused when it falls outside ([`view::Layout::cell_in`]): a drag that
/// left the pane keeps running to the nearest cell, which is what puts the last
/// characters of the bottom line inside reach, and a press in the padding
/// anchors on the nearest cell instead of throwing the selection away.
#[allow(clippy::too_many_arguments)]
fn spot_in_pane(
    layout: &view::Layout,
    space: Space,
    view: View,
    pane: &state::Pane,
    x: f32,
    y: f32,
    size: f32,
    column: f32,
) -> Option<select::Spot> {
    let (row, at) = layout.cell_in(space, x, y, size, column)?;
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
        // Below the last line on screen the selection runs to the end of the
        // text that is on screen. The end of the whole ring would be wrong
        // whenever the pane is scrolled back: sweeping to the bottom of a pane
        // showing older output would silently take everything down to the live
        // end with it.
        let window = pane.window(rows, cols);
        let last = match window.count {
            0 => pane.last().saturating_sub(1),
            count => pane.showing_from(rows, cols) + count - 1,
        };
        let end = pane.line(last).map_or(0, |l| l.text.chars().count());
        return Some(select::Spot::new(last, end));
    };
    // `offset` is where this visual row starts inside its logical line, so a
    // click on the second row of a wrapped line lands past the wrap. The
    // column is not clamped to the line's own length: a drag that ran off the
    // right of a short line has to reach that line's last character, and
    // `Selection::text` is what trims the overshoot.
    Some(select::Spot::new(line, offset + at))
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
    /// Only ever [`Hit::ColumnDivider`] or [`Hit::RowDivider`].
    sizing: Option<Hit>,
    /// The settings row whose slider the button came down on, while it is being
    /// dragged. The same cycle again, on the panel: the value follows the
    /// pointer and the file is written once, when the button comes up.
    sliding: Option<usize>,
    /// Where the two dividers are, as fractions: how much of the width the left
    /// column takes, and how much of the right column's height the top space
    /// takes. Read out of the settings file at launch and written back when a
    /// drag ends.
    left_width: f32,
    top_height: f32,
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
            left_width: config.left_width,
            top_height: config.top_height,
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
            settings: None,
            prompt: Prompt::default(),
            menu: None,
            holding: None,
            sizing: None,
            sliding: None,
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
            settings: self.settings.as_ref(),
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
            left_width: self.left_width,
            top_height: self.top_height,
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

    /// The button beside Open: the saved sessions, or back to the folders.
    ///
    /// The reading happens here rather than in the picker because this is the
    /// half of the window that is allowed to touch the disk, and it happens on
    /// the press rather than when the window opens: a machine with a year of
    /// sessions on it should not pay for the list nobody asked to see.
    fn toggle_sessions(&mut self) {
        let showing = self.picker.as_ref().is_some_and(Picker::on_sessions);
        let listing = (!showing).then(|| self.saved_sessions());
        if let Some(picker) = self.picker.as_mut() {
            match listing {
                Some(listing) => picker.show_sessions(listing),
                None => {
                    picker.show_folders();
                }
            }
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

    /// Open the settings panel over the window.
    ///
    /// The all-time totals go on it with this session already added in. The file
    /// on disk holds the sessions that came before, and adding the live one here
    /// is the same sum `remember` writes, so the panel and the next write agree.
    fn open_settings(&mut self) {
        let totals = self.totals.plus(&self.state);
        self.settings = Some(Settings::open(
            &self.config,
            &totals,
            self.settings_path().as_deref(),
            self.read_agent(),
        ));
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
    }

    /// Keys while the settings panel is up. It is a takeover, so this answers for
    /// the whole keyboard: nothing here falls through to the prompt.
    fn key_in_settings(&mut self, event: &winit::event::KeyEvent, event_loop: &ActiveEventLoop) {
        // The one key that means the same thing everywhere in this window. Every
        // other key here belongs to the panel, so without this arm the panel
        // would be a surface Ctrl-Q could not be pressed on.
        if self.modifiers.control_key() {
            if matches!(event.logical_key.as_ref(), Key::Character("q")) {
                event_loop.exit();
            }
            return;
        }
        let rows = self.settings_rows();
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
        let mut nudge = None;
        let mut edit = false;
        match event.logical_key.as_ref() {
            Key::Named(NamedKey::Escape) => {
                self.close_settings();
                return;
            }
            // On the rail these walk the sections; inside one they walk its rows.
            Key::Named(NamedKey::ArrowUp) => self.dirty |= panel.step(false),
            Key::Named(NamedKey::ArrowDown) => self.dirty |= panel.step(true),
            Key::Named(NamedKey::PageUp) => self.dirty |= panel.page(rows, false),
            Key::Named(NamedKey::PageDown) => self.dirty |= panel.page(rows, true),
            Key::Named(NamedKey::Home) => self.dirty |= panel.jump(false),
            Key::Named(NamedKey::End) => self.dirty |= panel.jump(true),
            // Left is the way back to the rail from a row that has nothing to
            // nudge, and the nudge itself from a row that has. A section of
            // readings would otherwise be a place the keyboard cannot leave
            // without the pointer.
            Key::Named(NamedKey::ArrowLeft) => match panel.on_row() {
                true => nudge = Some(false),
                false => self.dirty |= panel.leave(),
            },
            // Right goes into the section from the rail, and nudges inside it.
            // Enter is the same, except on the endpoint, where it starts typing.
            Key::Named(NamedKey::ArrowRight) | Key::Named(NamedKey::Enter) => {
                if !panel.enter() {
                    match matches!(
                        panel.row(panel.cursor()),
                        Some(settings::Row::Field { .. })
                    ) {
                        true => edit = true,
                        false => nudge = Some(true),
                    }
                }
                self.dirty = true;
            }
            // Between the rail and the rows, both ways. The one key that does it
            // without also meaning something to a row.
            Key::Named(NamedKey::Tab) => {
                self.dirty |= match self.modifiers.shift_key() {
                    true => panel.leave(),
                    false => panel.enter(),
                }
            }
            _ => {}
        }
        if edit {
            self.dirty |= panel.edit();
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
        let Some(panel) = self.settings.as_mut() else {
            return;
        };
        let Some(path) = panel.agent_file().map(std::path::Path::to_path_buf) else {
            panel.say_trouble(String::from("there is no config directory to write it in"));
            self.dirty = true;
            return;
        };
        let Some((key, value)) = panel.finish_edit() else {
            return;
        };
        match settings::write_endpoint(&path, key, &value) {
            Ok(()) => {
                let totals = self.totals.plus(&self.state);
                let agent = self.read_agent();
                let config = self.config.clone();
                if let Some(panel) = self.settings.as_mut() {
                    panel.adopt_agent(agent, &config, &totals);
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
        match hit {
            Hit::SettingsClose => self.close_settings(),
            Hit::SettingsSection(index) => {
                if let Some(panel) = self.settings.as_mut() {
                    self.dirty |= panel.choose(index);
                }
            }
            Hit::SettingsRow(index) => {
                if let Some(panel) = self.settings.as_mut() {
                    self.dirty |= panel.point_at(index);
                }
            }
            // The value is the control, so clicking it does what the right arrow
            // does, on the row it is on rather than on the row the cursor was
            // left on. On the endpoint that is starting to type into it.
            Hit::SettingsValue(index) => {
                let field = match self.settings.as_mut() {
                    Some(panel) => {
                        self.dirty |= panel.point_at(index);
                        matches!(panel.row(index), Some(settings::Row::Field { .. }))
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
            Hit::SettingsSlider(index) => {
                self.sliding = Some(index);
                self.drag_slider();
            }
            // The panel's own margin. Swallowed: it covers the window, so a
            // press here has nothing behind it to reach.
            _ => {}
        }
        self.reveal_settings_cursor();
    }

    /// Move the slider under the button to where the pointer is now.
    ///
    /// Nothing is written here. A drag across the window is hundreds of motion
    /// events, and the panel carries the value it is being dragged to until the
    /// button comes up, the same way a divider does.
    fn drag_slider(&mut self) {
        let Some(index) = self.sliding else {
            return;
        };
        let layout = self.layout();
        let Some(at) = layout.slider_at(index, self.cursor.x as f32) else {
            return;
        };
        if let Some(panel) = self.settings.as_mut() {
            self.dirty |= panel.slide(index, at);
        }
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
        let Some(change) = self.settings.as_ref().and_then(|panel| panel.change(forward)) else {
            return;
        };
        self.write_setting(&change);
    }

    /// Write one change and take the file's answer. The half of
    /// [`App::change_setting`] the slider shares, since a drag decides its value
    /// the other way round and lands in exactly the same place.
    fn write_setting(&mut self, change: &settings::Change) {
        let Some(path) = self.settings_path() else {
            if let Some(panel) = self.settings.as_mut() {
                panel.say_trouble(String::from("there is no home directory to write settings in"));
            }
            self.dirty = true;
            return;
        };
        match settings::commit(&path, change) {
            Ok(config) => {
                self.adopt(config);
                let totals = self.totals.plus(&self.state);
                if let Some(panel) = self.settings.as_mut() {
                    panel.refresh(&self.config, &totals);
                }
            }
            // Said on the panel rather than in the activity pane, which is
            // behind the takeover and cannot be read from here.
            Err(why) => {
                if let Some(panel) = self.settings.as_mut() {
                    panel.say_trouble(why);
                }
            }
        }
        self.dirty = true;
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
        self.left_width = self.config.left_width;
        self.top_height = self.config.top_height;
        self.restyle();
        for (view, wanted) in panes {
            match wanted {
                true => {
                    self.dock.unhide(view);
                }
                false => {
                    if self.dock.hide(view) {
                        self.forget_selection_in(view);
                    }
                }
            }
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
        }
        self.dirty = true;
    }

    /// How many rows the panel's list can show right now.
    fn settings_rows(&self) -> usize {
        self.layout().settings_capacity(self.config.pane_font_size)
    }

    /// Bring the cursor on screen, measured against the layout the panel is
    /// drawn in rather than against the panel alone.
    fn reveal_settings_cursor(&mut self) {
        let rows = self.settings_rows();
        if let Some(panel) = self.settings.as_mut() {
            self.dirty |= panel.reveal(rows);
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
    /// two buttons.
    fn click_in_picker(&mut self, hit: Hit, double: bool) {
        // Before the picker is borrowed, because swapping the list reads the
        // disk and that is the window's job rather than the model's.
        if hit == Hit::PickerSessions {
            self.toggle_sessions();
            return;
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
                    // The one frame that says which session is running and where
                    // it is running, which is the note the session list needs.
                    if let noob_proto::Event::SessionStart { id, workspace, .. } = &event {
                        self.remember_session(id, workspace);
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
            Hit::ColumnDivider | Hit::RowDivider => self.sizing = Some(hit),
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
            // All four are handled above, while the picker is up, which is the
            // only time any of them can be hit at all.
            Hit::PickerRow(_)
            | Hit::PickerMark(_)
            | Hit::PickerOpen
            | Hit::PickerSessions
            | Hit::Picker => {}
            // The same for the six the settings panel owns.
            Hit::SettingsSection(_)
            | Hit::SettingsRow(_)
            | Hit::SettingsValue(_)
            | Hit::SettingsSlider(_)
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
            // Both handled above, while a menu is open, which is the only time
            // either can be hit at all.
            Hit::MenuRow(_) | Hit::Menu => {}
        }
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
            (Item::Settings, _) => self.open_settings(),
            (Item::CopySelection, _) => {
                self.copy_selection();
            }
            (Item::Close, Target::Widget(view, _)) => self.close_view(view),
            // The prompt's menu has no Close row, so this cannot happen; it is
            // matched rather than caught by a wildcard so adding one is a
            // compile error here instead of a click that silently does nothing.
            (Item::Close, Target::Input) => {}
            // The row that opens the list beside itself, which is the thing the
            // row is for: closing the menu would take the list with it.
            (Item::Widgets(_), _) => {
                menu.toggle_widgets(&self.dock);
                self.menu = Some(menu);
            }
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
    /// The way back is the widget list on the same menu, and the two settings
    /// that carry a pane of their own. The arrangement survives a close because
    /// a space with no tabs gives its room to its neighbour rather than leaving
    /// a hole; see `Layout::compute`.
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
    ///
    /// The pane scrolls, so the row under the pointer is a row of the window and
    /// not of the list: what the window starts at has to be added back, or a
    /// scrolled pane opens the arguments of a different call than the one clicked.
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
        let first = self.scroll_first(&layout, View::Debug, layout.placed(space).body);
        self.dirty |= self.state.toggle_failure(first + row);
    }

    /// The first row of the content a scrolling pane is currently showing.
    fn scroll_first(&self, layout: &Layout, view: View, panel: noob_draw::Panel) -> usize {
        let frame = self.frame(layout);
        match view::scroll_extent(&frame, view, panel) {
            Some((heights, rows)) => self.state.scrolls.window(view, &heights, rows).first,
            None => 0,
        }
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
        spot_in_pane(
            layout,
            space,
            view,
            pane,
            self.cursor.x as f32,
            self.cursor.y as f32,
            size,
            column,
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
        let (slot, next) = match grip {
            Hit::RowDivider => (&mut self.top_height, layout.row_ratio_at(y)),
            _ => (&mut self.left_width, layout.column_ratio_at(x)),
        };
        if (*slot - next).abs() < f32::EPSILON {
            return;
        }
        *slot = next;
        self.dirty = true;
    }

    /// Write where a divider was left into the settings file.
    ///
    /// Through the same writer every other setting goes through, so the comments
    /// in the file survive a drag. A file that cannot be written is said once, in
    /// the activity pane, rather than silently losing the arrangement.
    fn remember_divider(&mut self, grip: Hit) {
        let (key, ratio) = match grip {
            Hit::RowDivider => ("top_height", self.top_height),
            _ => ("left_width", self.left_width),
        };
        let Some(path) = config::path() else {
            return;
        };
        // Three places is finer than a pixel on any window this can be dragged
        // in, so what is written and what is on screen are the same arrangement.
        let value = format!("{ratio:.3}");
        match config::write_setting(&path, key, Some(&value)) {
            Ok(()) => match key {
                "top_height" => self.config.top_height = ratio,
                _ => self.config.left_width = ratio,
            },
            Err(why) => self
                .state
                .activity
                .say(format!("cannot save the layout: {why}"), state::Tone::Bad),
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
        if self.settings.is_some() {
            self.key_in_settings(&event, event_loop);
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
        // A menu floats above the window, so while the pointer is on one the
        // wheel moves its widget list rather than the pane the menu covers.
        // First, for the same reason the menu is hit tested first.
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
            let rows = layout.settings_capacity(self.config.pane_font_size);
            let by = ((rows as f32 * pages.abs()).round() as usize).max(1);
            if let Some(panel) = self.settings.as_mut() {
                self.dirty |= panel.scroll(by, pages < 0.0, rows);
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
        self.dirty |= self
            .state
            .scrolls
            .scroll(view, by, pages < 0.0, &heights, fit);
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
            selection: self.state.selection,
            // The same menu the layout was computed from, or the rows would be
            // drawn somewhere other than where they are hit tested.
            menu: self.menu.as_ref(),
            picker: self.picker.as_ref(),
            settings: self.settings.as_ref(),
            // The orb's clock. Read here rather than inside the scene, so a
            // frame stays a function of what it is handed.
            clock: self.epoch.elapsed().as_secs_f32(),
        }
    }

    fn render(&mut self) {
        if self.gpu.is_none() || self.renderer.is_none() {
            return;
        }
        // Computed before the surface is borrowed: the prompt's height is read
        // off the whole app and the renderer holds it mutably.
        let layout = self.layout();
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
            self.dirty |= self.state.scrolls.settle(view, &heights, rows);
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
            Some(workspace) => self.connect(Chosen::folder(workspace)),
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
                        | Hit::PickerMark(_)
                        | Hit::SettingsClose
                        | Hit::SettingsSection(_)
                        | Hit::SettingsSlider(_)
                        | Hit::SettingsValue(_)),
                    ) => Some(hit),
                    _ => None,
                };
                if hot != self.hot {
                    self.hot = hot;
                    self.dirty = true;
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
        let covered = self.picker.is_some() || self.settings.is_some();
        if sampling(self.shaded, covered, &self.dock) {
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

/// What the pointer looks like at a point in the window.
///
/// With a tab in the air it says what letting go there would do, and the one
/// thing it has to say is that a tab dropped outside the window closes that
/// widget: nothing else out there tells you, because out there is somebody
/// else's window. `Crosshair` rather than `NoDrop` for it. NoDrop is the slashed
/// circle every toolkit uses for "this drop will be refused", and the drop is not
/// refused: it is accepted and it deletes the widget, so the one cursor that
/// promises nothing will happen is the wrong one. A cross is also what was asked
/// for.
///
/// With nothing in the air it is the divider under the pointer and then the
/// resize edges, which are the only thing telling anyone that an undecorated
/// window can be resized at all. A drag crossing an edge does not show a resize
/// arrow: what the drag does is the more urgent of the two answers, and the
/// button is already down so nothing can start a resize anyway.
///
/// A divider is nothing but the gap between two panes, so this is the only thing
/// that says one can be moved at all. It wins against an edge, which is the same
/// rule the other way round: a divider drag that wandered onto the border is
/// still a divider drag, and the two cannot overlap otherwise (the border is the
/// outside six pixels, and both dividers stand inside the panes).
///
/// Pure so the rule can be tested without a compositor, like [`land`].
fn cursor_for(
    dragging: bool,
    landing: Landing,
    edge: Option<winit::window::ResizeDirection>,
    over: Option<Hit>,
) -> CursorIcon {
    if dragging {
        return match landing {
            Landing::Out => CursorIcon::Crosshair,
            Landing::In(..) | Landing::Span(..) | Landing::Nowhere => CursorIcon::Default,
        };
    }
    match over {
        Some(Hit::ColumnDivider) => return CursorIcon::ColResize,
        Some(Hit::RowDivider) => return CursorIcon::RowResize,
        _ => {}
    }
    match edge {
        Some(dir) => resize_cursor(dir),
        None => CursorIcon::Default,
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

        let ask = shade_request(true, Some(open), false);
        assert_eq!(ask.min, None, "a minimum taller than the strip refuses it");
        assert_eq!(
            ask.size,
            Some(PhysicalSize::new(1180, view::strip_height() as u32))
        );
        assert_eq!(ask.maximized, None, "an ordinary window is left as it is");

        let ask = shade_request(false, Some(open), false);
        assert_eq!(ask.min, Some(MIN_SIZE));
        assert_eq!(ask.size, Some(open), "and it goes back to the size it was");
        assert_eq!(ask.maximized, None);
    }

    /// Shading a maximized window: it leaves that state to become a strip and
    /// is put back into it when the strip is opened.
    ///
    /// A maximized window ignores `request_inner_size`, so without the first
    /// half the surface stays full screen and the title bar is painted across
    /// the whole of it, which is a window that reads as a screenful of the bar
    /// colour. Without the second half, shading a window twice would be a way of
    /// un-maximizing it, which is not what either click asked for.
    #[test]
    fn shading_a_maximized_window_leaves_maximized_and_unshading_puts_it_back() {
        let full = PhysicalSize::new(2560, 1400);

        let ask = shade_request(true, Some(full), true);
        assert_eq!(
            ask.maximized,
            Some(false),
            "a maximized window has to leave that state to be a strip"
        );
        assert_eq!(
            ask.size,
            Some(PhysicalSize::new(2560, view::strip_height() as u32)),
            "and it still asks for the strip, at the width it had"
        );
        assert_eq!(ask.min, None);

        let ask = shade_request(false, Some(full), true);
        assert_eq!(ask.maximized, Some(true), "and it goes back maximized");
        assert_eq!(
            ask.size, None,
            "the compositor owns the size of a maximized window"
        );
        assert_eq!(ask.min, Some(MIN_SIZE));
    }

    /// A shaded window that comes back a size no strip can be is not shaded any
    /// more.
    ///
    /// The window is dragged by its title bar; on GNOME a pointer near the top
    /// of the screen snaps it maximized; a maximized window ignores the resize
    /// that shading asks for, so the surface stays full screen and the strip is
    /// painted across all of it. Nothing here predicts that gesture. It reads
    /// the surface that arrived and drops a state the surface contradicts.
    #[test]
    fn a_shaded_window_the_compositor_kept_tall_is_not_shaded_any_more() {
        let strip = view::strip_height() as u32;

        // The ordinary case: the request was granted, and it is a strip.
        assert_eq!(shade_of(true, false, false, strip, strip), Shade::Strip);
        // A few pixels either way is a compositor rounding a request, not a
        // different window state.
        assert_eq!(shade_of(true, false, false, strip + 4, strip), Shade::Strip);
        assert_eq!(shade_of(true, false, false, strip * 2, strip), Shade::Strip);

        // Two title bars is not rounding, and it is far below `MIN_SIZE`, the
        // shortest an open window is ever asked for, so nothing in between is a
        // window state either.
        assert_eq!(
            shade_of(true, false, false, strip * 2 + 1, strip),
            Shade::Open
        );
        assert_eq!(shade_of(true, false, false, 760, strip), Shade::Open);
        assert!(
            (MIN_SIZE.height as u32) > strip * 2,
            "the line is below any open window"
        );

        // Maximized is the case he hit, and it is dropped whatever the height
        // says: a maximized window cannot be a strip, and the height it reports
        // is its surface rather than the window.
        assert_eq!(shade_of(true, true, false, strip, strip), Shade::Open);
        assert_eq!(shade_of(true, true, false, 1400, strip), Shade::Open);

        // The two surfaces that are not answers, both of them the un-maximize a
        // shade of a maximized window asks for. First the window while it is
        // still maximized: read back off this machine, `request_inner_size`
        // answers a maximized window on the spot with the full screen it stayed
        // at, and that arrives before the un-maximize has landed.
        assert_eq!(shade_of(true, true, true, 1400, strip), Shade::Leaving);
        // Then the restored size, once it has left. The strip goes out again
        // rather than the state being dropped on it.
        assert_eq!(shade_of(true, false, true, 760, strip), Shade::Asking);
        // And once it is a strip it is a strip, whether or not one is expected.
        assert_eq!(shade_of(true, false, true, strip, strip), Shade::Strip);

        // A window that is not shaded is not made shaded by any of this.
        for height in [strip, 760] {
            for maximized in [false, true] {
                for settling in [false, true] {
                    assert_eq!(
                        shade_of(false, maximized, settling, height, strip),
                        Shade::Open
                    );
                }
            }
        }
    }

    /// A double click on the title bar maximizes the window, and a single click
    /// only arms the move.
    ///
    /// The double click used to collapse the window to its strip, which was
    /// buggy, so it does what a double click on a title bar does everywhere
    /// else on the desktop. The closed set of two is the point: there is no
    /// shade to return any more, and the single click still does nothing on the
    /// press itself, because that is what leaves the second click reachable.
    #[test]
    fn a_double_click_on_the_title_bar_maximizes_and_a_single_one_only_waits() {
        assert_eq!(
            title_click(true),
            TitleClick::Maximize,
            "the second click of a pair toggles the maximized state"
        );
        assert_eq!(
            title_click(false),
            TitleClick::ArmMove,
            "and one click on its own only arms a move for the pointer to decide"
        );
    }

    /// The title bar waits before it hands the compositor a move.
    ///
    /// `drag_window` is one way: after it the pointer belongs to the compositor
    /// and the second click of a double click never arrives, so a press that
    /// began a move immediately could not also be the first half of a maximize.
    /// The same slop a held tab waits for, so a click that wobbled is still a
    /// click.
    #[test]
    fn the_title_bar_only_moves_the_window_once_the_pointer_has_moved() {
        assert!(!began_move(true, 0.0), "a still pointer is a click");
        assert!(
            !began_move(true, DRAG_SLOP - 0.5),
            "a wobble is still a click"
        );
        assert!(began_move(true, DRAG_SLOP), "and moving away is a move");
        assert!(began_move(true, 400.0));
        // Nothing is held: motion over the title bar with the button up does
        // not move the window.
        assert!(!began_move(false, 400.0));
        // The same threshold a held tab uses, so the two decisions cannot drift
        // apart into a press that drags a tab but not a window.
        assert_eq!(DRAG_SLOP, 5.0);
    }

    /// The height shading asks for is the height the strip is laid out at, in
    /// the space the layout works in.
    ///
    /// Two things are being pinned. The number: it comes from the strip itself,
    /// so a request can never be short of what the strip has to draw, and it is
    /// whole pixels because a window is asked in whole pixels. And the space:
    /// physical, because `Layout::compute` is handed the surface configuration
    /// `noob-gpu` reports and nothing between winit and it applies a scale
    /// factor. Sent as a logical size instead, the request comes back multiplied
    /// by the scale factor and the strip is painted across the top of a surface
    /// twice its height. `view::strip_height` is asserted against what the strip
    /// writes over in `view`, where the text size lives.
    #[test]
    fn the_shade_request_is_the_strip_the_layout_draws() {
        let asked = shade_request(true, Some(PhysicalSize::new(1180, 760)), false)
            .size
            .expect("shading asks for a size");
        let strip = view::strip_height();
        assert_eq!(asked.height as f32, strip, "the request is not the strip");
        assert_eq!(strip, strip.ceil(), "a window is asked in whole pixels");
        assert_eq!(asked.width, 1180, "shading keeps the width it had");
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
            // the picker and the settings panel are full takeovers.
            assert!(!sampling(true, false, &dock), "{view:?} while shaded");
            assert!(
                !sampling(false, true, &dock),
                "{view:?} behind a takeover"
            );
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
        laid_out_at(dock, menu, chars, W, H)
    }

    /// The same at a size of its own. The tab strip's arrows only exist on a
    /// strip too narrow to hold its tabs, and at `W` every tab fits.
    fn laid_out_at<'a>(
        dock: &'a Dock,
        menu: Option<&'a Menu>,
        chars: usize,
        w: f32,
        h: f32,
    ) -> Layout {
        let shape = Shape {
            shaded: false,
            dock,
            menu,
            picker: None,
            settings: None,
            file_labels: Vec::new(),
            file_first: 0,
            column: COLUMN,
            pane_size: Config::default().pane_font_size,
            pane_column: COLUMN,
            input_h: view::input_height(
                w,
                COLUMN,
                chars,
                noob_draw::Text::line_for(SIZE),
                Config::default().max_input_rows,
            ),
            left_width: Config::default().left_width,
            top_height: Config::default().top_height,
        };
        Layout::compute(w, h, &shape)
    }

    fn middle(panel: noob_draw::Panel) -> (f32, f32) {
        (panel.x + panel.w * 0.5, panel.y + panel.h * 0.5)
    }

    fn opened(layout: &Layout, dock: &Dock, at: (f32, f32)) -> Option<Menu> {
        menu_for(layout.hit(at.0, at.1), at, dock, false, None)
    }

    /// One step of the walk. Rebased on the tab the strip actually starts at, so
    /// a resize that clamped the strip since does not cost a click before
    /// anything moves, and stopped at either end rather than wrapping: an arrow
    /// that came back round would say the strip had more to show when it does
    /// not.
    #[test]
    fn a_strip_walks_one_tab_at_a_time_and_stops_at_both_ends() {
        assert_eq!(walked(0, true, 6), 1);
        assert_eq!(walked(3, true, 6), 4);
        assert_eq!(walked(5, true, 6), 5, "it does not wrap");
        assert_eq!(walked(9, true, 6), 5, "nor past the end from a stale offset");
        assert_eq!(walked(3, false, 6), 2);
        assert_eq!(walked(0, false, 6), 0, "and it does not wrap back");
        assert_eq!(walked(0, true, 0), 0, "a space with no tabs stays put");
    }

    /// Item 18, end to end: the six tabs of the top right space in a window at
    /// its narrowest, walked to the last one and back with the arrows the strip
    /// grew. Every tab is reachable, the pane on screen always has its own tab in
    /// the strip, and the walk stops rather than wrapping.
    #[test]
    fn the_arrows_walk_a_narrow_strip_to_its_last_tab_and_back() {
        const NARROW: (f32, f32) = (680.0, 380.0);
        let mut dock = Dock::new();
        let views = dock.slot(Space::TopRight).views.clone();
        let showing = |dock: &Dock| {
            let layout = laid_out_at(dock, None, 0, NARROW.0, NARROW.1);
            let placed = layout.placed(Space::TopRight);
            (
                placed.first_tab,
                placed.tabs.iter().map(|(view, _)| *view).collect::<Vec<_>>(),
            )
        };
        let (_, tabs) = showing(&dock);
        assert!(tabs.len() < views.len(), "every tab fits at 680 pixels");

        // Forward to the end, one click at a time. Each click moves something,
        // and the showing tab is in the strip on every frame of the way.
        let mut seen = vec![views[0]];
        for step in 1..views.len() {
            let at = laid_out_at(&dock, None, 0, NARROW.0, NARROW.1)
                .placed(Space::TopRight)
                .first_tab;
            assert!(
                walk_tabs(&mut dock, Space::TopRight, at, true),
                "click {step} did nothing"
            );
            let (first, tabs) = showing(&dock);
            let active = dock.slot(Space::TopRight).active().unwrap();
            assert_eq!(active, views[step], "click {step} showed the wrong tab");
            assert!(tabs.contains(&active), "click {step}: {active:?} not in {tabs:?}");
            assert!(first + tabs.len() <= views.len());
            seen.push(active);
        }
        assert_eq!(seen, views, "the walk did not reach every tab");

        // At the end it stops rather than wrapping.
        let at = showing(&dock).0;
        assert!(!walk_tabs(&mut dock, Space::TopRight, at, true));
        assert_eq!(
            dock.slot(Space::TopRight).active(),
            views.last().copied(),
            "the walk wrapped round"
        );

        // And back, which brings the strip back with it.
        for step in (0..views.len() - 1).rev() {
            let at = showing(&dock).0;
            assert!(walk_tabs(&mut dock, Space::TopRight, at, false));
            let (_, tabs) = showing(&dock);
            let active = dock.slot(Space::TopRight).active().unwrap();
            assert_eq!(active, views[step]);
            assert!(tabs.contains(&active), "{active:?} not in {tabs:?}");
        }
        assert_eq!(showing(&dock).0, 0, "the strip did not come back");
        let at = showing(&dock).0;
        assert!(!walk_tabs(&mut dock, Space::TopRight, at, false), "it wrapped");
    }

    /// The arrows belong to the space whose strip they are in, so a right click
    /// on one opens the menu for that space's widget, the way a click on its
    /// pane or on one of its file rows does.
    #[test]
    fn an_arrow_carries_the_menu_of_the_space_it_is_in() {
        let dock = Dock::new();
        let layout = laid_out_at(&dock, None, 0, 680.0, 380.0);
        let showing = dock.slot(Space::TopRight).active().unwrap();
        for panel in [
            layout.placed(Space::TopRight).arrow_left,
            layout.placed(Space::TopRight).arrow_right,
        ] {
            assert!(panel.w >= 1.0, "the strip grew no arrows");
            let menu = opened(&layout, &dock, middle(panel)).expect("an arrow has a menu");
            assert_eq!(menu.target, Target::Widget(showing, Space::TopRight));
        }
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

        let showing = dock.slot(Space::TopLeft).active().unwrap();
        let menu = opened(&layout, &dock, middle(layout.placed(Space::TopLeft).body))
            .expect("a pane has a menu");
        assert_eq!(menu.target, Target::Widget(showing, Space::TopLeft));

        // Nothing a menu could act on.
        for at in [middle(layout.close), (400.0, 8.0)] {
            assert!(opened(&layout, &dock, at).is_none(), "at {at:?}");
        }
        // Nor is anything in the settings panel: it covers the panes, so there
        // is no widget under a right click, and a Settings row there would open
        // what is already open.
        for hit in [
            Hit::Settings,
            Hit::SettingsRow(3),
            Hit::SettingsValue(3),
            Hit::SettingsClose,
        ] {
            assert!(
                menu_for(Some(hit), (600.0, 400.0), &dock, true, Some(View::Output)).is_none(),
                "{hit:?}"
            );
        }
        // And the open menu itself: the second right click puts it away rather
        // than opening a menu for what it covers.
        let menu = Menu::for_widget((500.0, 400.0), View::Plan, Space::TopRight, false);
        let over = laid_out(&dock, Some(&menu), 0);
        let at = middle(over.menu_rows[0].1);
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

    /// A row of the list is a switch: a widget in the window goes out, and one
    /// that is out comes back. This asserted the half of that which shipped,
    /// where a widget already in the window was only revealed, so the list could
    /// add a widget and never take one away.
    #[test]
    fn picking_a_widget_takes_it_out_of_the_window_or_puts_it_back() {
        let mut dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        menu.toggle_widgets(&dock);

        // In the window, and out: no tab, no space, nothing walks to it. Dragged
        // somewhere else first, so where it comes back to says something.
        assert!(dock.move_view(View::Files, Space::TopLeft));
        let toggled = toggle_view(&mut dock, &mut menu, View::Files);
        assert!(toggled.hidden);
        assert!(dock.is_hidden(View::Files));
        assert_eq!(dock.space_of(View::Files), None);
        assert!(!dock.walk().contains(&View::Files));
        assert!(dock.is_sound(), "{dock:?}");

        // Out, and back: in the window, walked to, in the space it opens in by
        // default rather than wherever it was before.
        let toggled = toggle_view(&mut dock, &mut menu, View::Files);
        assert!(!toggled.hidden);
        assert!(!dock.is_hidden(View::Files));
        let home = dock.space_of(View::Files).expect("it is somewhere");
        assert_eq!(dock.slot(home).active(), Some(View::Files), "and showing");
        assert_eq!(
            home,
            Dock::new()
                .space_of(View::Files)
                .expect("its default space"),
            "back where it opens rather than where it was"
        );
        assert_ne!(home, Space::TopLeft, "which is not where it was dragged to");
        assert!(dock.is_sound(), "{dock:?}");

        // And the marks follow, so the row says which way it will go next.
        assert_eq!(
            menu.pick(menu.top + 8),
            Some(Item::Widget(View::Files, false)),
            "FILES is the ninth widget and it is back in the window"
        );
        toggle_view(&mut dock, &mut menu, View::Files);
        assert_eq!(
            menu.pick(menu.top + 8),
            Some(Item::Widget(View::Files, true))
        );
    }

    /// The menu stays open over the list, so a second widget can be switched
    /// without opening the menu again. The exception is the widget the menu was
    /// opened over going out: the rest of its rows act on that widget, and a
    /// Close row pointed at a pane that is no longer in the window is a row that
    /// does nothing.
    #[test]
    fn the_menu_stays_open_over_the_list_unless_its_own_widget_goes_out() {
        let mut dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        menu.toggle_widgets(&dock);

        // Another widget, either way round: the menu stays.
        assert!(toggle_view(&mut dock, &mut menu, View::Debug).keep_open);
        assert!(toggle_view(&mut dock, &mut menu, View::Debug).keep_open);
        // Its own, coming back in, is not its own going out.
        assert!(dock.hide(View::Plan));
        menu.relist(&dock);
        assert!(toggle_view(&mut dock, &mut menu, View::Plan).keep_open);
        // Its own, going out.
        assert!(!toggle_view(&mut dock, &mut menu, View::Plan).keep_open);
        assert!(dock.is_hidden(View::Plan));
        // The prompt's menu has no widget of its own, so nothing on the list can
        // take one away from it. It has no list either, but the rule is the
        // rule wherever it is asked.
        let mut input = Menu::for_input((0.0, 0.0), false);
        assert!(toggle_view(&mut dock, &mut input, View::Output).keep_open);
    }

    /// Switching every widget off empties the window one space at a time, and
    /// the dock is sound at every step of it, including at the end where there
    /// is nothing left in any space. Switching them all back on fills it again.
    #[test]
    fn switching_every_widget_off_and_back_on_keeps_the_dock_sound() {
        let mut dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Output, Space::TopLeft, false);
        menu.toggle_widgets(&dock);
        for view in View::ALL {
            assert!(toggle_view(&mut dock, &mut menu, view).hidden);
            assert!(dock.is_sound(), "after {view:?} went out: {dock:?}");
            assert!(dock.is_hidden(view));
        }
        assert!(dock.walk().is_empty(), "the window is empty");
        for space in Space::ALL {
            assert!(dock.slot(space).views.is_empty());
        }
        for view in View::ALL {
            assert!(!toggle_view(&mut dock, &mut menu, view).hidden);
            assert!(dock.is_sound(), "after {view:?} came back: {dock:?}");
        }
        assert_eq!(dock.walk().len(), View::ALL.len());
        // Every row of the list says the widget is in the window again.
        for (step, view) in View::ALL.into_iter().enumerate() {
            assert_eq!(
                menu.pick(menu.top + step),
                Some(Item::Widget(view, false)),
                "{view:?}"
            );
        }
    }

    /// A settings change turns a pane on or off only when that pane's own setting
    /// moved, so an unrelated edit cannot put back a widget that was closed by
    /// hand. Closing one writes nothing to the file, so the file still says the
    /// pane is on and every change would resurrect it.
    #[test]
    fn only_the_pane_setting_that_moved_turns_a_pane_on_or_off() {
        let on = Config::default();
        assert!(on.show_activity && on.show_files);

        // A change to something else moves neither.
        let bigger = Config::parse("font_size = 20");
        assert_eq!(pane_changes(&on, &bigger), Vec::new());

        // And one that does moves only its own.
        let off = Config::parse("show_activity = off");
        assert_eq!(pane_changes(&on, &off), vec![(View::Activity, false)]);
        assert_eq!(pane_changes(&off, &on), vec![(View::Activity, true)]);
        let neither = Config::parse("show_activity = off\nshow_files = off");
        assert_eq!(
            pane_changes(&on, &neither),
            vec![(View::Activity, false), (View::Files, false)]
        );

        // The dock does what the answer says, both ways round.
        let mut dock = Dock::new();
        for (view, wanted) in pane_changes(&on, &neither) {
            assert!(!wanted);
            assert!(dock.hide(view));
        }
        assert!(dock.is_hidden(View::Activity) && dock.is_hidden(View::Files));
        for (view, wanted) in pane_changes(&neither, &on) {
            assert!(wanted);
            assert!(dock.unhide(view));
        }
        assert!(!dock.is_hidden(View::Activity) && !dock.is_hidden(View::Files));
    }

    /// Dropped on a space a tab moves; dropped off the window it is closed, the
    /// same as picking Close; dropped on neither it stays where it was.
    #[test]
    fn a_tab_dropped_off_the_window_is_closed_rather_than_moved() {
        let mut dock = Dock::new();
        assert!(land(&mut dock, View::Files, Landing::In(Space::TopLeft, None)));
        assert_eq!(dock.space_of(View::Files), Some(Space::TopLeft));

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
        assert!(!land(&mut dock, View::Files, Landing::In(Space::TopLeft, None)));
    }

    /// Item 7: while a tab is being dragged outside the window the pointer says
    /// the drop will delete it, and nothing else in the window can say that,
    /// because out there is not the window.
    #[test]
    fn a_tab_dragged_out_of_the_window_takes_the_delete_cursor() {
        use winit::window::ResizeDirection as Dir;

        assert_eq!(
            cursor_for(true, Landing::Out, None, None),
            CursorIcon::Crosshair,
            "a drag over nothing does not say it deletes"
        );
        // Even over a resize edge: the tab in the air is the more urgent answer,
        // and with the button already down nothing can start a resize.
        assert_eq!(
            cursor_for(true, Landing::Out, Some(Dir::SouthEast), None),
            CursorIcon::Crosshair
        );
        // Back inside, it is an ordinary pointer again.
        for landing in [
            Landing::In(Space::TopLeft, None),
            Landing::In(Space::TopRight, Some(2)),
            Landing::Nowhere,
        ] {
            assert_eq!(
                cursor_for(true, landing, Some(Dir::East), None),
                CursorIcon::Default,
                "{landing:?}"
            );
        }
        // With nothing in the air the edges are what the pointer is for.
        assert_eq!(
            cursor_for(false, Landing::Nowhere, Some(Dir::West), None),
            CursorIcon::WResize
        );
        assert_eq!(cursor_for(false, Landing::Nowhere, None, None), CursorIcon::Default);
        // And a pointer outside the window that is not carrying anything is not
        // promising to delete something.
        assert_eq!(cursor_for(false, Landing::Out, None, None), CursorIcon::Default);
    }

    /// Item 16: the pointer is the only thing that says a divider can be moved
    /// at all, since a divider is nothing but the gap between two panes. It says
    /// so over the band, on the axis that divider moves in, and it keeps saying
    /// it while a drag of one wanders over the window's own resize border.
    #[test]
    fn the_pointer_says_a_divider_can_be_dragged() {
        use winit::window::ResizeDirection as Dir;

        let dock = Dock::new();
        let layout = laid_out_at(&dock, None, 0, 1200.0, 800.0);
        let column = layout.column_divider.band;
        let row = layout.row_divider.band;
        let at = |panel: noob_draw::Panel| {
            let (x, y) = middle(panel);
            layout.hit(x, y)
        };
        assert_eq!(at(column), Some(Hit::ColumnDivider));
        assert_eq!(at(row), Some(Hit::RowDivider));
        assert_eq!(
            cursor_for(false, Landing::Nowhere, None, at(column)),
            CursorIcon::ColResize
        );
        assert_eq!(
            cursor_for(false, Landing::Nowhere, None, at(row)),
            CursorIcon::RowResize
        );
        // A drag that ran onto the border is still that drag.
        assert_eq!(
            cursor_for(false, Landing::Nowhere, Some(Dir::West), Some(Hit::ColumnDivider)),
            CursorIcon::ColResize
        );
        // Off the band it is the ordinary pointer again, and the border still
        // answers where there is no divider.
        assert_eq!(
            cursor_for(false, Landing::Nowhere, None, Some(Hit::Body(Space::TopLeft))),
            CursorIcon::Default
        );
        assert_eq!(
            cursor_for(false, Landing::Nowhere, Some(Dir::South), Some(Hit::Body(Space::TopLeft))),
            CursorIcon::SResize
        );
        // And a tab in the air outranks both: what the drop will do is the more
        // urgent answer, and the button is already down.
        assert_eq!(
            cursor_for(true, Landing::Out, None, Some(Hit::ColumnDivider)),
            CursorIcon::Crosshair
        );
    }

    /// The landing the cursor is driven from is the layout's own, so the shape
    /// the pointer takes and the move the release makes come from one answer.
    #[test]
    fn the_delete_cursor_comes_from_the_same_landing_the_drop_does() {
        let dock = Dock::new();
        let layout = laid_out_at(&dock, None, 0, 1200.0, 800.0);
        for (x, y) in [(-2.0, 400.0), (1201.0, 400.0), (600.0, 801.0)] {
            let landing = layout.landing(x, y);
            assert_eq!(landing, Landing::Out, "at {x},{y}");
            assert_eq!(cursor_for(true, landing, None, None), CursorIcon::Crosshair);
            // And that is the release that closes the widget.
            let mut dock = Dock::new();
            assert!(land(&mut dock, View::Plan, landing));
            assert!(dock.is_hidden(View::Plan));
        }
        let inside = layout.landing(600.0, 400.0);
        assert!(matches!(inside, Landing::In(..)), "{inside:?}");
        assert_eq!(cursor_for(true, inside, None, None), CursorIcon::Default);
    }

    /// A drop that names a place in a strip reorders the tabs; one that names
    /// only a space puts the tab at the end of that space, the way it always did.
    #[test]
    fn a_drop_that_names_a_place_in_the_strip_reorders_the_tabs() {
        let mut dock = Dock::new();
        let order = |dock: &Dock| dock.slot(Space::TopRight).views.clone();
        assert_eq!(order(&dock)[0], View::Activity);

        // In front of the first tab of the space it is already in.
        assert!(land(&mut dock, View::Session, Landing::In(Space::TopRight, Some(0))));
        assert_eq!(order(&dock)[0], View::Session);
        assert_eq!(dock.slot(Space::TopRight).active(), Some(View::Session));

        // The same drop again is where it already is, so nothing happens.
        let before = dock.clone();
        assert!(!land(&mut dock, View::Session, Landing::In(Space::TopRight, Some(0))));
        assert_eq!(dock, before);
        assert!(!land(&mut dock, View::Session, Landing::In(Space::TopRight, Some(1))));
        assert_eq!(dock, before, "behind itself is also where it is");

        // From another space, into a named place rather than onto the end.
        assert!(land(&mut dock, View::Output, Landing::In(Space::TopRight, Some(2))));
        assert_eq!(order(&dock)[2], View::Output);
        // And with no place named, onto the end.
        assert!(land(&mut dock, View::Output, Landing::In(Space::BottomRight, None)));
        assert_eq!(
            dock.slot(Space::BottomRight).views.last(),
            Some(&View::Output)
        );
    }

    /// The whole drop path, from a pointer position to the arrangement it
    /// leaves: on the line between two cells the pane takes both of them, and
    /// inside one cell it takes that one and the span comes apart.
    ///
    /// Driven through `Layout::landing` rather than by naming a landing, so the
    /// pixels a hand actually aims at are what is under test.
    #[test]
    fn a_drop_between_two_cells_spans_them_and_one_inside_a_cell_splits_them() {
        const AT: (f32, f32) = (1400.0, 900.0);
        let mut dock = Dock::new();
        let cell = |dock: &Dock, space: Space| {
            laid_out_at(dock, None, 0, AT.0, AT.1).grid[space.index()]
        };
        let drop_at = |dock: &mut Dock, view: View, (x, y): (f32, f32)| {
            let landing = laid_out_at(dock, None, 0, AT.0, AT.1).landing(x, y);
            let moved = land(dock, view, landing);
            assert!(dock.is_sound(), "{landing:?}: {dock:?}");
            (landing, moved)
        };

        // The line between the two cells of the right column, aimed at the gap
        // that is drawn there.
        let top = cell(&dock, Space::TopRight);
        let line = (top.x + top.w * 0.5, top.y + top.h + 2.0);
        let (landing, moved) = drop_at(&mut dock, View::Output, line);
        assert_eq!(landing, Landing::span(Space::TopRight, Space::BottomRight));
        assert!(moved);
        assert_eq!(dock.space_of(View::Output), Some(Space::TopRight));
        assert!(dock.slot(Space::BottomRight).is_empty(), "{dock:?}");
        assert_eq!(
            dock.cover()[Space::BottomRight.index()],
            Some(Space::TopRight),
            "the pane covers the pair"
        );
        // Which is what the layout draws: one pane down the whole column.
        let layout = laid_out_at(&dock, None, 0, AT.0, AT.1);
        let placed = layout.placed(Space::TopRight);
        let (over, under) = (
            layout.grid[Space::TopRight.index()],
            layout.grid[Space::BottomRight.index()],
        );
        assert!((placed.strip.y - over.y).abs() < 0.01);
        assert!((placed.body.y + placed.body.h - (under.y + under.h)).abs() < 0.01);

        // And a drop inside the lower cell of that column takes the span apart:
        // the pane that was covering both keeps the upper cell.
        let bottom = cell(&dock, Space::BottomRight);
        let (landing, moved) = drop_at(
            &mut dock,
            View::Debug,
            (bottom.x + bottom.w * 0.5, bottom.y + bottom.h * 0.5),
        );
        assert_eq!(landing, Landing::In(Space::BottomRight, None));
        assert!(moved);
        assert_eq!(dock.slot(Space::BottomRight).views, vec![View::Debug]);
        assert_eq!(
            dock.cover()[Space::BottomRight.index()],
            Some(Space::BottomRight),
            "two panes, one cell each"
        );
        let layout = laid_out_at(&dock, None, 0, AT.0, AT.1);
        assert!(
            layout.placed(Space::TopRight).body.y + layout.placed(Space::TopRight).body.h
                < layout.placed(Space::BottomRight).strip.y,
            "the two panes overlap"
        );
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

    /// A pane of known text, and the pixel-to-character half of a selection in
    /// it. `Model::spot_at` is a two line wrapper around `spot_in_pane`;
    /// everything that decides which character is under the pointer is here.
    fn output_pane(lines: &[&str]) -> (Dock, Layout, Space, state::Pane) {
        let dock = Dock::new();
        let layout = laid_out(&dock, None, 0);
        let space = Space::ALL
            .into_iter()
            .find(|space| dock.slot(*space).active() == Some(View::Output))
            .expect("the conversation is in the window");
        let mut pane = state::Pane::new(100);
        for text in lines {
            pane.push(state::Line::new(*text, state::Tone::Body));
        }
        (dock, layout, space, pane)
    }

    /// The last character of a line and the last character of the buffer are
    /// both selectable.
    ///
    /// The range is half-open at the end, so taking the last character means
    /// landing the focus on column `len`: the boundary after it, which is half
    /// a column of pixels short of the right hand edge of the box. Nothing may
    /// clamp that back to `len - 1`, and nothing may refuse the pixel for being
    /// past the last glyph.
    #[test]
    fn a_drag_can_reach_the_last_character_of_a_line_and_of_the_buffer() {
        let (_dock, layout, space, pane) = output_pane(&["hello world", "second line"]);
        // The nine pixel padding the panes are drawn with, and one text row.
        let inner = layout.content(space).inset(9.0);
        let line = noob_draw::Text::line_for(SIZE);
        let at = |row: usize, column: usize| {
            (
                inner.x + column as f32 * COLUMN,
                inner.y + (row as f32 + 0.5) * line,
            )
        };
        let spot = |(x, y): (f32, f32)| {
            spot_in_pane(&layout, space, View::Output, &pane, x, y, SIZE, COLUMN)
                .expect("a pane with text has a nearest character everywhere")
        };

        // The boundary after the final 'd' of the first line.
        assert_eq!(spot(at(0, 11)), select::Spot::new(0, 11));
        let mut selection = select::Selection::new(View::Output, spot(at(0, 0)));
        selection.extend(spot(at(0, 11)));
        assert_eq!(selection.text(&pane), "hello world");

        // And the last character of the whole buffer, from the start of it.
        let mut selection = select::Selection::new(View::Output, spot(at(0, 0)));
        selection.extend(spot(at(1, 11)));
        assert_eq!(selection.text(&pane), "hello world\nsecond line");
    }

    /// A drag that leaves the box keeps extending to the nearest cell instead
    /// of stopping where it was.
    ///
    /// Sweeping to the bottom right is how anyone selects to the end of a pane,
    /// and the pointer is past the text by the time the button comes up. The
    /// hit test used to answer nothing outside the inset box, so the focus
    /// froze on the last cell the pointer happened to cross and the sweep took
    /// everything but the end of it.
    #[test]
    fn a_drag_that_leaves_the_pane_keeps_running_to_the_nearest_cell() {
        let (_dock, layout, space, pane) = output_pane(&["hello world", "second line"]);
        let body = layout.content(space);
        let inner = body.inset(9.0);
        let line = noob_draw::Text::line_for(SIZE);
        let spot = |x: f32, y: f32| {
            spot_in_pane(&layout, space, View::Output, &pane, x, y, SIZE, COLUMN)
                .expect("a drag off the pane still has a nearest character")
        };

        let start = spot(inner.x, inner.y + line * 0.5);
        assert_eq!(start, select::Spot::new(0, 0));

        // Off the right hand edge of the window entirely, on the first row.
        let mut selection = select::Selection::new(View::Output, start);
        selection.extend(spot(W + 500.0, inner.y + line * 0.5));
        assert_eq!(
            selection.text(&pane),
            "hello world",
            "a drag off the right takes the rest of the row"
        );

        // And below the pane, which is the sweep to the end of the text.
        let mut selection = select::Selection::new(View::Output, start);
        selection.extend(spot(W + 500.0, body.y + body.h + 400.0));
        assert_eq!(selection.text(&pane), "hello world\nsecond line");

        // Above and to the left of it, which is the same sweep backwards.
        let mut selection = select::Selection::new(View::Output, spot(W, body.y + body.h));
        selection.extend(spot(-200.0, -200.0));
        assert_eq!(selection.text(&pane), "hello world\nsecond line");
    }

    /// A press in the pane's padding anchors on the nearest character rather
    /// than throwing the selection away.
    ///
    /// The press lands on `Hit::Body`, which is the whole pane, while the text
    /// sits in a box nine pixels inside it. A press in that margin used to
    /// resolve to no character at all, which cleared the selection and left
    /// `selecting` false, so the drag that followed did nothing whatsoever.
    #[test]
    fn a_press_in_the_padding_still_anchors_a_selection() {
        let (_dock, layout, space, pane) = output_pane(&["hello world", "second line"]);
        let body = layout.content(space);
        let spot = |x: f32, y: f32| {
            spot_in_pane(&layout, space, View::Output, &pane, x, y, SIZE, COLUMN)
        };

        // The press is inside the pane and outside the text box, on all four
        // sides of it.
        for (name, x, y) in [
            ("left", body.x + 1.0, body.y + 20.0),
            ("top", body.x + 20.0, body.y + 1.0),
            ("right", body.x + body.w - 1.0, body.y + 20.0),
            ("bottom", body.x + 20.0, body.y + body.h - 1.0),
        ] {
            assert!(
                spot(x, y).is_some(),
                "a press in the {name} padding resolved to no character"
            );
        }
        // The top left corner of the padding anchors on the first character,
        // which is what a drag from there then extends.
        assert_eq!(
            spot(body.x + 1.0, body.y + 1.0),
            Some(select::Spot::new(0, 0))
        );
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

    /// The same path with the pane scrolled. The row under the pointer is a row
    /// of the window, so which failure it opens depends on where the window
    /// starts: `App::open_failure_under_pointer` adds that back, and without it a
    /// scrolled pane expands a call nobody clicked.
    #[test]
    fn a_click_in_a_scrolled_debug_pane_opens_the_call_under_the_pointer() {
        let mut state = State::new();
        for i in 0..30 {
            let id = format!("bad-{i:02}");
            state.apply(noob_proto::Event::ToolStart {
                call_id: id.clone(),
                name: "bash".into(),
                brief: format!("call {i:02}"),
                args: noob_proto::Value::Object(
                    [(String::from("which"), noob_proto::Value::String(id.clone()))]
                        .into_iter()
                        .collect(),
                ),
            });
            state.apply(noob_proto::Event::ToolEnd {
                call_id: id,
                summary: "no".into(),
                elapsed_ms: 1,
                error: Some(noob_proto::ToolError {
                    kind: "denied".into(),
                    code: None,
                    message: format!("boom {i:02}"),
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
        // Every row is clipped to one row, which is what the pane's own extent
        // reports for this list.
        let heights = view::flat_heights(state.debug_rows().len());
        let rows = layout.rows(body, pane_size);
        assert!(heights.len() > rows, "{} rows in a box of {rows}", heights.len());

        assert!(state.scrolls.scroll(View::Debug, 5, true, &heights, rows));
        let first = state.scrolls.window(View::Debug, &heights, rows).first;
        assert_eq!(first, 5, "the window starts five rows down");

        // The third row of the pane, the same pixel arithmetic the window uses.
        let (x, y) = (body.x + 20.0, body.y + 9.0 + 2.5 * line);
        let (space, row, _) = layout
            .cell(x, y, pane_size, COLUMN)
            .expect("the pointer is over a pane");
        assert_eq!((space, row), (Space::BottomRight, 2));

        // The count is the first row of the list, so row seven is the seventh
        // failure.
        let under = state.debug_rows()[first + row].text.clone();
        assert!(under.contains("boom 06"), "row {} reads {under:?}", first + row);
        assert!(state.toggle_failure(first + row));
        assert_eq!(state.open_failure, Some(6));
        let list: Vec<String> = state.debug_rows().into_iter().map(|r| r.text).collect();
        assert!(
            list.iter().any(|text| text.contains("which = bad-06")),
            "the arguments of the call under the pointer are not shown"
        );
        // Unscrolled, that same row is the second failure and nothing else.
        state.open_failure = None;
        assert!(state.scrolls.scroll(View::Debug, 999, false, &heights, rows));
        assert!(state.toggle_failure(row));
        assert_eq!(state.open_failure, Some(1));
    }

    /// The wheel and the page keys reach every pane. A view either keeps its own
    /// scrollback, which is a transcript counted back from the live end, or
    /// reports an extent, which is a list counted from the top. One that did
    /// neither is a pane nothing can move, which is what item 14 reported for
    /// four of them.
    #[test]
    fn every_pane_the_wheel_lands_on_can_be_scrolled() {
        let mut state = State::new();
        state.apply(noob_proto::Event::TextDelta { d: "hello".into() });
        state.apply(noob_proto::Event::ToolStart {
            call_id: "p".into(),
            name: "plan".into(),
            brief: "1 item".into(),
            args: serde_json::json!({"todos": [{"content": "read it", "status": "pending"}]}),
        });
        state.apply(noob_proto::Event::AgentSpawn {
            agent_id: "kid".into(),
            prompt: "look".into(),
            tools: "read".into(),
        });
        state.apply(noob_proto::Event::FileEdit {
            path: "src/calc.py".into(),
            span: noob_proto::Span {
                start: 1,
                end: 1,
                kind: None,
                name: None,
            },
            before: "a".into(),
            after: "b".into(),
            call_id: None,
        });
        state.apply(noob_proto::Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 100,
                cached_prompt: 10,
                completion: 5,
                context_total: 1000,
            },
        });
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        monitor.sample(&state);

        let skin = Skin::from(&Config::default());
        let dock = Dock::new();
        let layout = laid_out(&dock, None, 0);
        let prompt = Prompt::default();
        let frame = view::Frame {
            state: &state,
            monitor: &monitor,
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &prompt,
            column: COLUMN,
            pane_column: COLUMN,
            body_size: SIZE,
            pane_size: Config::default().pane_font_size,
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            menu: None,
            picker: None,
            settings: None,
            clock: 0.0,
        };
        let panel = layout.placed(Space::TopRight).body;
        for view in View::ALL {
            // A machine that reports no hardware at all has no rows there to
            // scroll, and that is the one honest exception.
            if view == View::Hardware && monitor.hardware().is_empty() {
                continue;
            }
            let scrollback = state.pane_of(view).is_some();
            let extent = view::scroll_extent(&frame, view, panel).is_some();
            assert!(
                scrollback != extent,
                "{view:?} keeps a scrollback: {scrollback}, reports an extent: {extent}"
            );
        }
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
