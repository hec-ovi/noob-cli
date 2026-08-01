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
use style::{markdown, skin, syntax};
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

/// What letting go of a held tab turned out to be. See [`click_tab`].
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum TabClick {
    /// A tab that was not the one on screen: it is now.
    Show,
    /// The tab that was already showing. Nothing happens on it.
    Nothing,
}

/// What a click on a tab does to its space.
///
/// Showing that tab, and only that. Clicking the tab already showing used to
/// fold the space away to its strip; that path is still in this file, in
/// [`App::fold`], and no gesture reaches it any more.
///
/// A free function over the slot because [`App::release`] needs a live window
/// and cannot be driven in a test, the same reason [`title_click`] and
/// [`began_move`] are out here.
fn click_tab(slot: &mut dock::Slot, view: View) -> TabClick {
    let showing = slot.active() == Some(view);
    if !showing {
        slot.show(view);
    }
    // Belt and braces. Nothing sets this any more, so it only ever holds it at
    // false; it stays so a space can never be left stuck at its strip.
    slot.folded = false;
    if showing {
        TabClick::Nothing
    } else {
        TabClick::Show
    }
}

/// When the orb wants its next frame, given the deadline it is already holding.
///
/// `None` is the point of this function: the clock exists only while the orb has
/// something to animate and disappears as soon as it does not. An earlier version
/// of this window free-ran at 3,500 frames a second drawing text that was not
/// changing and spent a third of the graphics pipe on it, which is what
/// `noob-gpu` warns about and why nothing here ever asks for `ControlFlow::Poll`.
///
/// `animating` is a running turn or the orb still on its way back from one. The
/// way back is finite by construction: [`Morph`] steps to exactly zero and stops
/// there, and the clock goes with it.
///
/// Pure so the rule can be tested without a window: an animation deadline is not
/// something to find out about by watching a fan.
fn orb_deadline(now: Instant, animating: bool, pending: Option<Instant>) -> Option<Instant> {
    if !animating {
        return None;
    }
    match pending {
        // Still waiting on the frame that was asked for.
        Some(at) if now < at => Some(at),
        _ => Some(now + ORB_EVERY),
    }
}

/// How long the orb takes to travel between its resting square and its turning
/// circles.
///
/// Nine frames at [`ORB_EVERY`]: long enough to see the dots move into place and
/// short enough that the first thing the agent says does not arrive halfway
/// through it.
const ORB_MORPH: Duration = Duration::from_millis(300);

/// Where the orb is between its two formations, and what it is travelling from.
///
/// It lives here rather than in the state model: it is a property of the
/// window's clock, not of the conversation, and nothing the agent says depends
/// on it.
///
/// Measured from the moment the turn started or ended rather than stepped by
/// however long the last wake took. A window that sat idle for an hour and then
/// got a prompt would step by an hour and arrive at the far end on the first
/// frame, which is the cut this replaces.
#[derive(Clone, Copy, Debug)]
struct Morph {
    /// Where it is now: 0 is the resting square, 1 is the turning circles.
    at: f32,
    /// Where it stood when the turn last started or ended, so a turn that ends
    /// mid-move turns the orb around from where it had got to rather than from
    /// the end it never reached.
    from: f32,
    since: Instant,
    /// Whether a turn was running at that moment.
    busy: bool,
}

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

/// Take one session off the disk: its transcript, and the line in the note that
/// says which folder it belonged to.
///
/// Both halves in one place, and a free function over the two paths rather than
/// a method reading them itself, so the whole of what a delete does can be
/// driven over a temp directory. The note is rewritten only once the file is
/// gone: a note without a transcript is a row that cannot be opened, and a
/// transcript without a note is a row that opens in the wrong folder.
///
/// The note not being writable is not a failure. The file it describes is
/// already gone, the row will not come back on the next read, and the stale line
/// costs nothing but a line in a file: refusing here would say the session was
/// not deleted when it was.
fn forget_session(
    dir: Option<PathBuf>,
    index: Option<PathBuf>,
    id: &str,
) -> Result<(), String> {
    let dir = dir.ok_or_else(|| String::from("there is nowhere to read sessions from"))?;
    sessions::forget(&dir, id).map_err(|why| format!("{id} was not deleted: {why}"))?;
    if let Some(path) = index {
        let _ = sessions::save_index(&path, &sessions::load_index(&path).minus(id));
    }
    Ok(())
}

/// Delete a set of saved conversations, and say which of them refused.
///
/// Every id is tried, whatever happened to the one before it: a set where the
/// third file is read only must still take the other four, because a delete that
/// stopped at the first refusal would leave the list half taken with nothing
/// saying which half. Each one goes through [`forget_session`], so the batch
/// cannot skip the path guard the single delete has, and each failure already
/// names the conversation it belongs to.
///
/// A free function so the loop can be tested without a window, the same way the
/// single delete under it is.
fn forget_sessions(dir: Option<PathBuf>, index: Option<PathBuf>, ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter_map(|id| forget_session(dir.clone(), index.clone(), id).err())
        .collect()
}

/// What to write down as a session's context reading, out of the two places the
/// window hears about one, or nothing when neither has said yet.
///
/// The agent's own reading first and the last request's usage as the fallback,
/// which is the order [`State::context_fraction`] reads them in: one moves
/// during a turn and the other describes the request that already went out. A
/// window whose size was never reported is not a reading at all, because a
/// number of tokens with nothing to compare it against says nothing on a row.
///
/// A free function so the rule can be tested without a window, and the only
/// place the figure that goes in the file is decided.
fn context_reading(
    fill: Option<state::ContextFill>,
    usage: Option<noob_proto::Usage>,
) -> Option<sessions::Context> {
    if let Some(fill) = fill.filter(|fill| fill.total > 0) {
        return Some(sessions::Context {
            used: fill.used,
            total: fill.total,
        });
    }
    let usage = usage.filter(|usage| usage.context_total > 0)?;
    Some(sessions::Context {
        used: usage.prompt,
        total: usage.context_total,
    })
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
    selection: Option<select::Selection>,
    picker: Option<&Picker>,
) -> Option<Menu> {
    // A click that never moved leaves an empty selection behind, and a Copy row
    // that lit up for one would copy nothing.
    let selection = selection.filter(|selection| !selection.is_empty());
    let widget = |view: View, space: Space| {
        Some(Menu::for_widget(
            at,
            view,
            space,
            selection.and_then(|selection| selection.view()) == Some(view),
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
        //
        // The popup floats on the same layer and gets the same answer: it is
        // about one call rather than about a widget, so there is no pane for a
        // Close or a Copy row to act on.
        Hit::Menu | Hit::MenuRow(_) | Hit::CallPopup => None,
        // A row of the session list is the one thing in the picker a menu can
        // act on: it names a file, so there is something to open and something
        // to delete. A folder row is not, because pressing it is the whole of
        // what it does and nothing here deletes a folder.
        Hit::PickerRow(index) => {
            let saved = picker?.session(index)?;
            Some(Menu::for_session(at, index, saved.gone))
        }
        // The rest of the picker is not a widget: there is no pane to close, no
        // settings behind it, and nothing in it to select.
        Hit::Picker
        | Hit::PickerMark(_)
        | Hit::PickerOpen
        | Hit::PickerFolders
        | Hit::PickerSessions => None,
        // The one thing on that panel a menu can act on: the document is a page
        // of prose, so there is something in it to highlight and something to
        // copy. The menu is already built and the row costs one line, which is
        // the cheaper half of the same act the drag and Ctrl-C are.
        Hit::SettingsDoc => Some(Menu::for_settings_doc(
            at,
            selection.is_some_and(|selection| selection.at == select::Where::SettingsDoc),
        )),
        // Nothing else on the settings panel is. A Settings row on a menu opened
        // over the settings panel would be a row that opens what is already
        // open, and there is no pane behind it to close.
        Hit::Settings
        | Hit::SettingsSection(_)
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
        | Hit::SettingsClose => None,
        // A divider is the gap between two widgets and belongs to neither of
        // them, so there is no one widget for a menu opened here to act on.
        Hit::ColumnDivider(_) | Hit::RowDivider(_) | Hit::SettingsRailDivider => None,
    }
}

/// What picking a row of the menu's Widgets group does, and what becomes of the
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
    Some(spot_in_rows(pane, rows, cols, row, at))
}

/// The character a row and a column of one box land on, in the pane that box is
/// drawn from.
///
/// The half of [`spot_in_pane`] that is about the text rather than about the
/// window, so the settings document reaches the same answer through the same
/// call instead of through a second copy of it.
fn spot_in_rows(
    pane: &state::Pane,
    rows: usize,
    cols: usize,
    row: usize,
    at: usize,
) -> select::Spot {
    let Some((line, column)) = pane.spot_in(rows, cols, row, at) else {
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
        // Counted in what is drawn: the last column of a Markdown line is the
        // last glyph of it, not the last character the model wrote.
        let end = pane.line(last).map_or(0, |l| l.shown().chars().count());
        return select::Spot::new(last, end);
    };
    // The column is the character `at` columns into the row that was pointed
    // at, which on the second row of a wrapped line is past the wrap and, since
    // the pane breaks at blanks, is not `row * cols` characters in. A pointer
    // out past the end of the row takes that row's last character: the pane
    // clamps it, so a drag off the right of a short line still reaches its end
    // without running into the row below.
    select::Spot::new(line, column)
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
    Nothing,
}

/// Which of those a key is.
///
/// A free function because the panel is a takeover and this is the whole of
/// what reaches it with control held: the list used to be Ctrl-Q alone, which
/// made the panel a surface you could read a document on and not copy a word
/// off. Nothing here touches the window, so the list is testable.
fn control_in_settings(key: Key<&str>) -> Control {
    match key {
        Key::Character("q") => Control::Quit,
        Key::Character("c") => Control::Copy,
        Key::Character("a") => Control::MarkAll,
        _ => Control::Nothing,
    }
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

/// Which of those a key is, with shift as the second half of the answer.
///
/// A free function for the same reason [`control_in_settings`] is one: these are
/// the two bindings that changed meaning, and the window they live in cannot be
/// built in a test. Tab is the rail, because the arrow keys are the rows of one
/// section now and without a key of its own the rail is reachable only by
/// pointer. It used to cross a form row, which is the shifted arrow now: shift
/// takes the nudge off left and right and leaves them pointing at the half they
/// land on.
fn walk_in_settings(key: Key<&str>, shift: bool) -> Option<Walk> {
    match (key, shift) {
        (Key::Named(NamedKey::Tab), _) => Some(Walk::Section(!shift)),
        (Key::Named(NamedKey::ArrowLeft), true) => Some(Walk::Cross(settings::Side::Left)),
        (Key::Named(NamedKey::ArrowRight), true) => Some(Walk::Cross(settings::Side::Right)),
        _ => None,
    }
}

/// The same for the document beside the settings panel's entry list, free of
/// the window so a test can drive it.
///
/// The panel is a takeover, so there is no space and no view here: the box is
/// named on the layout and the text comes from whichever entry the panel is
/// showing. Nothing at all when the column is not drawn, which is what a narrow
/// window and a section with no entries both look like.
fn spot_in_doc(
    layout: &view::Layout,
    panel: &settings::Settings,
    x: f32,
    y: f32,
    size: f32,
    column: f32,
) -> Option<select::Spot> {
    let (cols, rows) = (
        layout.settings_doc_columns(column),
        layout.settings_doc_rows(size),
    );
    if cols == 0 || rows == 0 {
        return None;
    }
    let (row, at) = layout.settings_doc_cell(x, y, size, column)?;
    let pane = panel.doc_pane_at(cols, rows);
    // An entry with no document has nothing to point at, and a spot in it would
    // read as a selection that copies nothing.
    (pane.last() > 0).then(|| spot_in_rows(&pane, rows, cols, row, at))
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
    /// The settings row and half whose slider the button came down on, while it
    /// is being dragged. The same cycle again, on the panel: the value follows
    /// the pointer and the file is written once, when the button comes up.
    sliding: Option<(usize, settings::Side)>,
    /// The thread reading the whole prompt out of the CLI, while it is still
    /// reading it. Dropped as soon as it answers, so a panel opened twice does
    /// not take the first run's answer for the second one's.
    asking: Option<std::sync::mpsc::Receiver<link::Asked>>,
    /// The thread installing a skill, while it is still installing it: the
    /// source it was given, and the name it landed under or why it did not.
    ///
    /// Its own thread for the same reason the prompt has one, only more so: a
    /// clone is given two minutes and the interface is one thread. Some, so a
    /// second press while one is running is refused rather than starting a race
    /// between two installs for one directory.
    installing: Option<std::sync::mpsc::Receiver<(String, Result<String, String>)>>,
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
    /// size anything else in the window is written at. See [`view::MENU_SIZE`].
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

/// What time it is on the reader's own clock, as seconds past local midnight.
///
/// `std` has a wall clock and no timezone at all, and this window keeps no date
/// crate in its graph, so the local reading is asked of the one program every
/// unix ships. A window that gets no answer leaves `day_zero` unset and its
/// rows carry no clock, which is better than a column of times an hour out.
fn local_day_second() -> Option<u64> {
    let told = std::process::Command::new("date")
        .arg("+%H:%M:%S")
        .output()
        .ok()?;
    state::day_second(std::str::from_utf8(&told.stdout).ok()?)
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
        App {
            dock: Dock::hiding(&hidden),
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
            workspace,
            session: None,
            picker: None,
            settings: None,
            prompt: Prompt::default(),
            menu: None,
            holding: None,
            sizing: None,
            sliding: None,
            asking: None,
            installing: None,
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
        self.ask_for_prompt();
        self.dirty = true;
    }

    /// Ask the CLI what the whole prompt is, on a thread of its own.
    ///
    /// The panel shows what the agent is really told, and the only thing that
    /// knows that is the CLI: `noob debug prompt` prints the artifact a session
    /// sends. It reads a config directory, a workspace and every skill on the
    /// machine, so it runs off the interface thread and wakes the event loop
    /// when it answers, the same way the agent's own output arrives. A slow
    /// endpoint or a big skills directory cannot hold up a frame.
    fn ask_for_prompt(&mut self) {
        let program = std::env::var("NOOB_BIN").unwrap_or_else(|_| String::from("noob"));
        // The folder the session is running in, since the project's own
        // AGENTS.md, its skills and its mcp.json are all found relative to it. A
        // window with no folder open yet asks in the one the process is in,
        // which is where a session started now would run.
        let workspace = match self.state.workspace.is_empty() {
            false => PathBuf::from(&self.state.workspace),
            true => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        self.asking = Some(rx);
        let proxy = self.proxy.clone();
        let at = workspace.display().to_string();
        std::thread::spawn(move || {
            let answer = match link::prompt_command(&program, &workspace, &agent::OWNED).output() {
                Ok(out) => link::prompt_from(out.status.success(), &out.stdout, &out.stderr),
                Err(e) => Err(format!(
                    "cannot run {program:?}: {e}; is noob on PATH, or set NOOB_BIN"
                )),
            };
            let _ = tx.send((at, answer));
            let _ = proxy.send_event(Wake);
        });
    }

    /// Take what that thread answered, if it has.
    fn take_prompt(&mut self) {
        let Some(rx) = self.asking.as_ref() else {
            return;
        };
        let Ok((at, answer)) = rx.try_recv() else {
            return;
        };
        self.asking = None;
        let config = self.config.clone();
        if let Some(panel) = self.settings.as_mut() {
            panel.adopt_prompt(at, answer, &config);
        }
        self.dirty = true;
    }

    /// Install what has been typed into the skills section, on a thread of its
    /// own.
    ///
    /// Never through [`App::do_deed`], which is synchronous: a clone is given
    /// two minutes and a window frozen for two minutes is a window that has
    /// crashed as far as anybody watching it is concerned. The same shape the
    /// prompt block runs in instead: a thread, a channel, and a wake of the
    /// event loop when it answers.
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
        let (tx, rx) = std::sync::mpsc::channel();
        self.installing = Some(rx);
        let proxy = self.proxy.clone();
        let config = self.config.clone();
        if let Some(panel) = self.settings.as_mut() {
            panel.begin_install(source.clone(), &config);
        }
        std::thread::spawn(move || {
            let answer = install::install(&source, &skills_at);
            let _ = tx.send((source, answer));
            let _ = proxy.send_event(Wake);
        });
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
            panel.adopt_install(source, answer, agent, &config);
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
        let mut flip = None;
        let mut make = None;
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
                    // A block with nothing in it offers to write the file it
                    // was looking for.
                    Some(settings::Row::Paper(_)) => make = Some(panel.cursor()),
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
        if let Some(index) = flip {
            let deed = self.settings.as_ref().and_then(|panel| panel.toggle(index));
            self.do_deed(deed);
        }
        if let Some(index) = make {
            let deed = self.settings.as_ref().and_then(|panel| panel.make(index));
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

    /// Write one setting into the agent's own file and read the whole file back.
    ///
    /// Shared by the endpoint field and by the two tracks beside it, because a
    /// nudged number and a typed URL are the same write to the same file. What
    /// the panel shows next is what the file answered.
    fn write_agent_setting(&mut self, path: &Path, key: &str, value: &str) {
        match settings::write_endpoint(path, key, value) {
            Ok(()) => {
                let agent = self.read_agent();
                let config = self.config.clone();
                if let Some(panel) = self.settings.as_mut() {
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
        let done = match &deed {
            settings::Deed::TurnSkill { dir, on } => match panel.skills_at() {
                Some(at) => agent::set_skill(at, dir, *on),
                None => Err(String::from("there is no skills directory to move it in")),
            },
            settings::Deed::RemoveSkill { dir, on } => match panel.skills_at() {
                Some(at) => agent::remove_skill(at, dir, *on),
                None => Err(String::from("there is no skills directory to remove it from")),
            },
            settings::Deed::TurnServer { name, project, on } => {
                match panel.mcp_file(*project) {
                    Some(path) => agent::set_server(path, name, *on),
                    None => Err(String::from("there is no file to write that server in")),
                }
            }
            settings::Deed::RemoveServer { name, project } => match panel.mcp_file(*project) {
                Some(path) => agent::remove_server(path, name),
                None => Err(String::from("there is no file to take that server out of")),
            },
            // The path came off the block that offered it, which read it off the
            // agent's own config directory: nothing here spells one out.
            settings::Deed::StartInstructions { path } => agent::start_instructions(path),
            // Both handled above: a set of conversations cannot answer with one
            // result, since some of it can land and the rest fail, and a
            // restore writes the window's own file rather than the agent's.
            settings::Deed::ForgetSessions { .. } | settings::Deed::RestoreLooks => Ok(()),
        };
        match done {
            Ok(()) => {
                let agent = self.read_agent();
                let config = self.config.clone();
                if let Some(panel) = self.settings.as_mut() {
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
            // that were overriding it.
            Hit::SettingsChoice(index, side, at) => {
                let change = match self.settings.as_mut() {
                    Some(panel) => {
                        self.dirty |= panel.point_at(index, side);
                        panel.choose_option(index, side, at)
                    }
                    None => None,
                };
                if let Some(change) = change {
                    self.write_setting(&change);
                }
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
                        view::Act::All => {
                            self.dirty |= panel.mark_all(index, true);
                            None
                        }
                        view::Act::None => {
                            self.dirty |= panel.mark_all(index, false);
                            None
                        }
                        // Both of these arm on the first press and act on the
                        // second: one deletes transcripts, the other takes
                        // lines out of a file somebody may have edited by hand.
                        view::Act::Forget | view::Act::Restore => panel.uninstall(index),
                        // The validate half of the install card's button:
                        // answered here and now, on the panel.
                        view::Act::Validate => {
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
                        // The install card's own button. Not a deed: a deed is
                        // done here and now, and this is a clone with two
                        // minutes to answer in.
                        view::Act::Install => {
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
                    },
                    None => None,
                };
                if matches!(act, view::Act::Install) {
                    self.start_install();
                }
                if matches!(act, view::Act::Validate) {
                    self.validate_source();
                }
                self.do_deed(deed);
            }
            // The panel's own margin. Swallowed: it covers the window, so a
            // press here has nothing behind it to reach.
            _ => {}
        }
        self.reveal_settings_cursor();
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
        let Some(change) = self.settings.as_ref().and_then(|panel| panel.change(forward)) else {
            return;
        };
        self.write_setting(&change);
    }

    /// Write one change and take the file's answer. The half of
    /// [`App::change_setting`] the slider shares, since a drag decides its value
    /// the other way round and lands in exactly the same place.
    fn write_setting(&mut self, change: &settings::Change) {
        // A setting of the agent's goes to the agent's file, through the agent's
        // writer. Same nudge, same track, other file.
        if change.file == settings::File::Agent {
            let path = self
                .settings
                .as_ref()
                .and_then(Settings::agent_file)
                .map(std::path::Path::to_path_buf);
            match path {
                Some(path) => self.write_agent_setting(&path, change.key, &change.value),
                None => {
                    if let Some(panel) = self.settings.as_mut() {
                        panel.say_trouble(String::from(
                            "there is no config directory to write it in",
                        ));
                    }
                    self.dirty = true;
                }
            }
            return;
        }
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
                if let Some(panel) = self.settings.as_mut() {
                    panel.refresh(&self.config);
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
        self.left_width = [self.config.left_width, self.config.left_width_bottom];
        self.top_height = [self.config.top_height, self.config.top_height_right];
        self.settings_rail = self.config.settings_rail;
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
            self.menu_column = renderer.column_width(view::MENU_SIZE);
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
        // prompt is read whether or not a session is running, and the panel can
        // be open on a window that has no agent at all.
        self.take_prompt();
        // The same, for the skill being installed: it answers on its own thread
        // and wakes the loop, and a window with no agent running still has to
        // pick the answer up.
        self.take_install();
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
                    self.dirty = true;
                }
            }
        }
        if turn_ended {
            self.remember_context();
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
        // The popup is on the same floating layer and closes the same way: its
        // own box swallows the press, and anywhere else only puts it away. The
        // press that closes it does nothing else, so a click aimed past a popup
        // at a pane cannot also start a selection in that pane.
        if self.state.open_call.is_some() {
            if !matches!(hit, Hit::CallPopup) {
                self.state.open_call = None;
                self.dirty = true;
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
            // All three handled above, while a menu or the popup is open, which
            // is the only time any of them can be hit at all.
            Hit::MenuRow(_) | Hit::Menu | Hit::CallPopup => {}
        }
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
        let Some(spot) = self.spot_at(&layout, space, View::Activity) else {
            return;
        };
        let Some(ordinal) = self.state.call_at_line(spot.line) else {
            return;
        };
        self.state.open_call = Some(ordinal);
        self.dirty = true;
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
            // The flyout header: its rows come out beside it and the menu
            // stays open over them, which is the whole of what the row is for.
            (Item::Widgets(_), _) => {
                menu.fold(index, &self.dock);
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
            self.dirty = true;
        }
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
        if self
            .selection
            .is_some_and(|selection| selection.at == select::Where::SettingsDoc)
        {
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
        };
        if let Some(spot) = spot {
            selection.extend(spot);
            // A drag is a selection, not a look at one call. The popup the press
            // put up goes away as soon as the drag has actually selected
            // something, which is what lets one press mean either.
            if !selection.is_empty() {
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
            click_tab(self.dock.slot_mut(space), view);
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
                if self.selection.take().is_some() {
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
                self.menu_column = renderer.column_width(view::MENU_SIZE);
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
                        | Hit::PickerFolders
                        | Hit::PickerSessions
                        | Hit::PickerMark(_)
                        | Hit::SettingsClose
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
                // An armed Delete on an open menu is disarmed by the pointer
                // leaving its row, so the second press can only be made by a
                // pointer that is still on the row which asked for it.
                if let Some(menu) = self.menu.as_mut() {
                    self.dirty |= menu.point_at(match under {
                        Some(Hit::MenuRow(row)) => Some(row),
                        _ => None,
                    });
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
        self.redraw();
        // Both clocks, never one of them: the earlier deadline wins and the other
        // is still there when it comes round.
        event_loop.set_control_flow(match soonest([self.next_sample, self.next_orb]) {
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
/// Which setting a divider writes when a drag of it ends.
///
/// One key per line rather than one per axis, so the two halves of an axis are
/// remembered apart: dragging the line over the right column has to leave the
/// one over the left column alone in the file as well as on screen.
fn divider_key(grip: Hit) -> Option<&'static str> {
    match grip {
        Hit::ColumnDivider(0) => Some("left_width"),
        Hit::ColumnDivider(_) => Some("left_width_bottom"),
        Hit::RowDivider(0) => Some("top_height"),
        Hit::RowDivider(_) => Some("top_height_right"),
        Hit::SettingsRailDivider => Some("settings_rail"),
        _ => None,
    }
}

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
        Some(Hit::ColumnDivider(_) | Hit::SettingsRailDivider) => return CursorIcon::ColResize,
        Some(Hit::RowDivider(_)) => return CursorIcon::RowResize,
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

    /// A tab click shows that tab and nothing else.
    ///
    /// Clicking the tab already showing used to fold the space away to its
    /// strip. It does not any more: no gesture collapses a pane, so the second
    /// click of a pair on the same tab has to be as inert as the first.
    #[test]
    fn a_click_on_the_tab_already_showing_does_nothing_and_never_folds() {
        let mut slot = dock::Slot::default();
        slot.views = vec![View::Output, View::Files];
        assert_eq!(slot.active(), Some(View::Output));
        // Once, then again fast enough to be a double click, then a third time.
        for _ in 0..3 {
            assert_eq!(
                click_tab(&mut slot, View::Output),
                TabClick::Nothing,
                "the tab on screen is already the tab on screen"
            );
            assert_eq!(slot.active(), Some(View::Output));
            assert!(!slot.folded, "no click folds a space");
        }
        assert_eq!(
            click_tab(&mut slot, View::Files),
            TabClick::Show,
            "the other tab is the part he is keeping"
        );
        assert_eq!(slot.active(), Some(View::Files));
        assert!(!slot.folded);
    }

    /// Every tab of every space, clicked twice: the flag never comes on.
    ///
    /// The rule the item is really about. `folded` still exists and the layout
    /// still honours it, so what has to hold is that no input path writes it,
    /// and `click_tab` is the only one that ever did.
    #[test]
    fn no_tab_click_anywhere_in_the_dock_folds_a_space() {
        let mut dock = Dock::new();
        for space in Space::ALL {
            let views = dock.slot(space).views.clone();
            for view in views {
                for _ in 0..2 {
                    click_tab(dock.slot_mut(space), view);
                    assert_eq!(dock.slot(space).active(), Some(view));
                }
            }
        }
        for space in Space::ALL {
            assert!(
                !dock.slot(space).folded,
                "{space:?} folded from a tab click"
            );
        }
    }

    /// And a space that somehow arrived folded opens on the next tab click.
    ///
    /// Nothing sets the flag any more and it is never written to the settings
    /// file, so this cannot happen; the clearing line stays because a pane stuck
    /// at its strip with no way out is the one failure worth being sure of.
    #[test]
    fn a_folded_space_opens_again_on_a_tab_click() {
        let mut slot = dock::Slot::default();
        slot.views = vec![View::Output, View::Files];
        slot.folded = true;
        assert_eq!(click_tab(&mut slot, View::Output), TabClick::Nothing);
        assert!(!slot.folded, "the tab on screen still opens it");
        slot.folded = true;
        assert_eq!(click_tab(&mut slot, View::Files), TabClick::Show);
        assert!(!slot.folded, "and so does the other one");
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

    /// The animation clock exists while the orb has something to animate and at
    /// no other time. A deadline that outlives that is a window animating with
    /// nothing to animate, which is the 3,500 frames a second `noob-gpu` warns
    /// about.
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

    /// The orb travels between its two formations over [`ORB_MORPH`] and is
    /// settled at both ends. Settled is `None` to the scene, which is what keeps
    /// every frame outside a transition the frame it always was.
    #[test]
    fn the_orb_travels_between_its_two_formations_over_the_move() {
        let now = Instant::now();
        let mut orb = Morph::new(now);
        assert_eq!(orb.showing(), None, "a window opens settled on its square");
        assert!(!orb.moving());

        // The turn starts: at the square still, and travelling.
        orb.step(true, now);
        assert_eq!(orb.showing(), Some(0.0));
        orb.step(true, now + ORB_MORPH / 3);
        assert!((orb.showing().expect("moving") - 1.0 / 3.0).abs() < 1e-5);
        orb.step(true, now + ORB_MORPH / 2);
        assert!((orb.showing().expect("moving") - 0.5).abs() < 1e-5);
        // Arrived, and it stays arrived however long the turn runs for.
        orb.step(true, now + ORB_MORPH);
        assert_eq!(orb.showing(), None, "the move did not finish");
        orb.step(true, now + Duration::from_secs(90));
        assert_eq!(orb.showing(), None);

        // And back, which is the direction that costs a clock: the turn is over
        // and the window would otherwise have stopped redrawing.
        let ended = now + Duration::from_secs(90);
        orb.step(false, ended);
        assert_eq!(orb.showing(), Some(1.0), "the turn ending is not a cut");
        assert!(orb.moving());
        orb.step(false, ended + ORB_MORPH / 2);
        assert!((orb.showing().expect("moving") - 0.5).abs() < 1e-5);
        orb.step(false, ended + ORB_MORPH);
        assert_eq!(orb.showing(), None, "the orb never settled back");
        assert!(!orb.moving(), "the clock would be held open forever");
    }

    /// The move is measured from the moment the turn started, not from the last
    /// wake. An idle window blocks indefinitely, so the wake that starts a turn
    /// can be an hour after the one before it, and stepping by that would put
    /// the orb at the far end on the first frame, which is the cut this is here
    /// to replace.
    #[test]
    fn a_window_that_sat_idle_still_gets_the_whole_move() {
        let now = Instant::now();
        let mut orb = Morph::new(now);
        let hour = now + Duration::from_secs(3600);
        orb.step(false, hour);
        orb.step(true, hour);
        assert_eq!(orb.showing(), Some(0.0), "the move was skipped");
        orb.step(true, hour + ORB_MORPH / 2);
        assert!((orb.showing().expect("moving") - 0.5).abs() < 1e-5);
    }

    /// A turn that ends mid-move turns the orb around from where it had got to,
    /// rather than from the end it never reached. Otherwise a turn one frame
    /// long makes the orb jump out to the circles to come back from them.
    #[test]
    fn a_turn_ending_mid_move_turns_the_orb_around_where_it_stands() {
        let now = Instant::now();
        let mut orb = Morph::new(now);
        orb.step(true, now);
        orb.step(true, now + ORB_MORPH / 2);
        let half = orb.showing().expect("moving");

        let ended = now + ORB_MORPH / 2;
        orb.step(false, ended);
        assert!((orb.showing().expect("moving") - half).abs() < 1e-5, "it jumped");
        orb.step(false, ended + ORB_MORPH / 4);
        assert!(orb.showing().expect("moving") < half, "it did not turn around");
        orb.step(false, ended + ORB_MORPH);
        assert_eq!(orb.showing(), None);
    }

    /// The clock outlives the turn by exactly the move and then goes, which is
    /// the one place the "no turn, no frames" rule gives way. The way out costs
    /// nothing: the turn is running the whole time, so the clock is already
    /// there.
    #[test]
    fn the_orb_clock_outlives_a_turn_by_the_length_of_the_move() {
        let now = Instant::now();
        let mut orb = Morph::new(now);
        orb.step(true, now);
        orb.step(true, now + ORB_MORPH);

        let ended = now + ORB_MORPH;
        orb.step(false, ended);
        let animating = |orb: &Morph, busy: bool| busy || orb.moving();
        assert!(
            orb_deadline(ended, animating(&orb, false), None).is_some(),
            "the orb is left halfway to its square with no clock to finish on"
        );
        orb.step(false, ended + ORB_MORPH);
        assert_eq!(
            orb_deadline(ended + ORB_MORPH, animating(&orb, false), None),
            None,
            "the clock outlived the move"
        );
    }

    const W: f32 = 1400.0;
    const H: f32 = 900.0;
    const COLUMN: f32 = 8.0;
    const SIZE: f32 = 14.0;

    fn laid_out<'a>(dock: &'a Dock, menu: Option<&'a Menu>) -> Layout {
        laid_out_at(dock, menu, W, H)
    }

    /// The same at a size of its own. The tab strip's arrows only exist on a
    /// strip too narrow to hold its tabs, and at `W` every tab fits.
    fn laid_out_at<'a>(dock: &'a Dock, menu: Option<&'a Menu>, w: f32, h: f32) -> Layout {
        let shape = Shape {
            shaded: false,
            dock,
            menu,
            picker: None,
            settings: None,
            file_labels: Vec::new(),
            file_first: 0,
            column: COLUMN,
            menu_column: COLUMN,
            pane_size: Config::default().pane_font_size,
            pane_column: COLUMN,
            input_h: view::input_height(
                Config::default().prompt_rows,
                noob_draw::Text::line_for(SIZE),
            ),
            left_width: [Config::default().left_width; 2],
            top_height: [Config::default().top_height; 2],
            settings_rail: Config::default().settings_rail,
            popup: None,
        };
        Layout::compute(w, h, &shape)
    }

    fn middle(panel: noob_draw::Panel) -> (f32, f32) {
        (panel.x + panel.w * 0.5, panel.y + panel.h * 0.5)
    }

    fn opened(layout: &Layout, dock: &Dock, at: (f32, f32)) -> Option<Menu> {
        menu_for(layout.hit(at.0, at.1), at, dock, false, None, None)
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
            let layout = laid_out_at(dock, None, NARROW.0, NARROW.1);
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
            let at = laid_out_at(&dock, None, NARROW.0, NARROW.1)
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
        let layout = laid_out_at(&dock, None, 680.0, 380.0);
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
        let layout = laid_out(&dock, None);

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
            Hit::SettingsRow(3, settings::Side::Left),
            Hit::SettingsValue(3, settings::Side::Right),
            Hit::SettingsClose,
        ] {
            assert!(
                menu_for(Some(hit), (600.0, 400.0), &dock, true, a_selection_in(View::Output), None).is_none(),
                "{hit:?}"
            );
        }
        // And the open menu itself: the second right click puts it away rather
        // than opening a menu for what it covers.
        let menu = Menu::for_widget((500.0, 400.0), View::Plan, Space::TopRight, false);
        let over = laid_out(&dock, Some(&menu));
        let at = middle(over.menu_rows[0].1);
        assert!(opened(&over, &dock, at).is_none());
    }

    /// A picker with two saved sessions showing, one of them in a folder that
    /// has been deleted since.
    fn a_session_picker() -> Picker {
        let saved = |id: &str, gone: bool| sessions::Saved {
            id: String::from(id),
            when: std::time::SystemTime::UNIX_EPOCH,
            workspace: Some(PathBuf::from("/home/hec/workspace")),
            gone,
            bytes: 1_000,
            context: None,
            opening: String::from("carry this on"),
        };
        let mut picker = Picker::open(
            Box::new(picker::Fixed(vec![String::from("gui")])),
            PathBuf::from("/home/hec"),
            Vec::new(),
        );
        picker.show_sessions_at(
            sessions::Listing {
                sessions: vec![saved("live", false), saved("orphan", true)],
                skipped: Vec::new(),
            },
            std::time::SystemTime::UNIX_EPOCH,
        );
        picker
    }

    /// A right click on a saved session opens the menu for it: carry it on, or
    /// delete it. Nothing else in the picker has a menu, because pressing a
    /// folder row is the whole of what that row does and nothing here deletes a
    /// folder.
    #[test]
    fn a_right_click_on_a_saved_session_offers_opening_it_and_deleting_it() {
        let dock = Dock::new();
        let at = (500.0, 400.0);
        let picker = a_session_picker();

        let menu = menu_for(
            Some(Hit::PickerRow(0)),
            at,
            &dock,
            false,
            None,
            Some(&picker),
        )
        .expect("a session row has a menu");
        assert_eq!(menu.target, Target::Session(0));
        assert_eq!(menu.pick(0), Some(Item::OpenSession));
        assert_eq!(menu.pick(1), Some(Item::DeleteSession(false)));

        // The row whose folder has gone keeps both rows and cannot be opened.
        let dead = menu_for(
            Some(Hit::PickerRow(1)),
            at,
            &dock,
            false,
            None,
            Some(&picker),
        )
        .expect("that row has one too");
        assert_eq!(dead.rows.len(), menu.rows.len());
        assert_eq!(dead.pick(0), None, "it cannot be resumed anywhere");
        assert_eq!(dead.pick(1), Some(Item::DeleteSession(false)));

        // A row that is not there, and the rest of the picker.
        assert!(
            menu_for(Some(Hit::PickerRow(9)), at, &dock, false, None, Some(&picker)).is_none()
        );
        for hit in [
            Hit::Picker,
            Hit::PickerMark(0),
            Hit::PickerOpen,
            Hit::PickerFolders,
            Hit::PickerSessions,
        ] {
            assert!(
                menu_for(Some(hit), at, &dock, false, None, Some(&picker)).is_none(),
                "{hit:?}"
            );
        }

        // And on the folder list there is no menu at all, on the same hit.
        let folders = Picker::open(
            Box::new(picker::Fixed(vec![String::from("gui")])),
            PathBuf::from("/home/hec"),
            Vec::new(),
        );
        assert!(
            menu_for(
                Some(Hit::PickerRow(0)),
                at,
                &dock,
                false,
                None,
                Some(&folders)
            )
            .is_none()
        );
    }

    /// The whole of what deleting a session does, over a real directory: the
    /// transcript goes, the line about it in the note goes with it, and every
    /// other session is left exactly as it was.
    ///
    /// Nothing in this window destroyed anything before item A7, so this is the
    /// one path where a wrong answer costs somebody a conversation.
    #[test]
    fn deleting_a_session_takes_its_file_and_its_line_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("no0b-forget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sessions_dir = dir.join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("a sessions dir");
        let note = dir.join("no0b.sessions");
        for id in ["keep", "drop"] {
            std::fs::write(
                sessions_dir.join(format!("{id}.jsonl")),
                format!("{{\"t\":\"meta\",\"v\":1,\"id\":\"{id}\"}}\n"),
            )
            .expect("a transcript");
        }
        let index = sessions::Index::default()
            .plus("keep", Path::new("/home/hec/one"))
            .plus_context(
                "drop",
                Path::new("/home/hec/two"),
                sessions::Context {
                    used: 10,
                    total: 100,
                },
            );
        sessions::save_index(&note, &index).expect("the note is writable");

        assert_eq!(
            forget_session(Some(sessions_dir.clone()), Some(note.clone()), "drop"),
            Ok(())
        );
        assert!(!sessions_dir.join("drop.jsonl").exists());
        assert!(sessions_dir.join("keep.jsonl").exists());
        let after = sessions::load_index(&note);
        assert_eq!(after, sessions::Index::default().plus("keep", Path::new("/home/hec/one")));

        // A name that would reach out of the sessions directory is refused
        // before anything is removed, and says so in one line the picker can
        // put on screen.
        let outside = dir.join("no0b.sessions");
        assert!(outside.exists());
        let why = forget_session(
            Some(sessions_dir.clone()),
            Some(note.clone()),
            "../no0b.sessions",
        )
        .expect_err("a path out of the directory was accepted");
        assert!(why.starts_with("../no0b.sessions was not deleted"), "{why}");
        assert!(outside.exists(), "it deleted a file outside the directory");
        assert_eq!(sessions::load_index(&note), after, "and the note is intact");

        // Nowhere to delete from at all, which is a machine with no config
        // directory rather than a session that refused to go.
        assert!(forget_session(None, Some(note.clone()), "keep").is_err());
        assert!(sessions_dir.join("keep.jsonl").exists());

        // And a window with no note file still deletes the transcript.
        assert_eq!(forget_session(Some(sessions_dir.clone()), None, "keep"), Ok(()));
        assert!(!sessions_dir.join("keep.jsonl").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The right click's Delete asks before it acts too, so the two routes to
    /// the same transcript are guarded the same way.
    ///
    /// The menu decides and this file acts, which is how [`App::pick`] is
    /// written: [`Menu::press_delete`] answers `false` for the first press and
    /// the window puts the menu back rather than calling anything, and answers
    /// `true` for the second, which is the only path to `forget_session` from
    /// here. Before this the first press deleted the file, while the settings
    /// panel two rooms away asked twice for the same act.
    #[test]
    fn the_picker_s_delete_row_asks_once_and_then_takes_the_file_and_its_line() {
        let dir = std::env::temp_dir().join(format!("no0b-menu-forget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sessions_dir = dir.join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("a sessions dir");
        let note = dir.join("no0b.sessions");
        for id in ["keep", "drop"] {
            std::fs::write(
                sessions_dir.join(format!("{id}.jsonl")),
                format!("{{\"t\":\"meta\",\"v\":1,\"id\":\"{id}\"}}\n"),
            )
            .expect("a transcript");
        }
        sessions::save_index(
            &note,
            &sessions::Index::default()
                .plus("keep", Path::new("/home/hec/one"))
                .plus("drop", Path::new("/home/hec/two")),
        )
        .expect("the note is writable");

        // What the window does with the answer, in one place, so the test
        // cannot delete by a route the window does not have.
        let press = |menu: &mut Menu, row: usize, id: &str| match menu.press_delete(row) {
            true => Some(forget_session(
                Some(sessions_dir.clone()),
                Some(note.clone()),
                id,
            )),
            false => None,
        };

        let mut menu = Menu::for_session((500.0, 400.0), 0, false);
        assert_eq!(press(&mut menu, 1, "drop"), None, "the first press deleted it");
        assert!(sessions_dir.join("drop.jsonl").exists(), "it went anyway");
        assert_eq!(menu.arming(), Some(1), "the row is not asking");

        // Moving off the row cancels, so the press after that is a first press
        // again and still takes nothing.
        assert!(menu.point_at(Some(0)));
        assert_eq!(press(&mut menu, 1, "drop"), None);
        assert!(sessions_dir.join("drop.jsonl").exists());

        // And closing the menu cancels, which is what every key and every press
        // off the menu does: the arming is on the menu and goes with it.
        drop(menu);
        let mut menu = Menu::for_session((500.0, 400.0), 0, false);
        assert_eq!(press(&mut menu, 1, "drop"), None, "it reopened armed");
        assert!(sessions_dir.join("drop.jsonl").exists());

        // The second press on the row that asked: the transcript and its line
        // in the note, and nothing else.
        assert_eq!(press(&mut menu, 1, "drop"), Some(Ok(())));
        assert!(!sessions_dir.join("drop.jsonl").exists());
        assert!(sessions_dir.join("keep.jsonl").exists());
        assert_eq!(
            sessions::load_index(&note),
            sessions::Index::default().plus("keep", Path::new("/home/hec/one"))
        );

        // The guard under both routes is untouched: a confirmed delete of a
        // name that would reach out of the sessions directory is still refused,
        // and the file it points at is still there.
        let outside = dir.join("no0b.sessions");
        let mut menu = Menu::for_session((500.0, 400.0), 0, false);
        assert_eq!(press(&mut menu, 1, "../no0b.sessions"), None);
        let why = press(&mut menu, 1, "../no0b.sessions")
            .expect("the second press did nothing")
            .expect_err("a path out of the directory was accepted");
        assert!(why.starts_with("../no0b.sessions was not deleted"), "{why}");
        assert!(outside.exists(), "it deleted a file outside the directory");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A panel over a scratch directory of saved conversations, and where the
    /// files are: the transcripts, the note beside them, and the panel row the
    /// table stands on.
    fn a_panel_over_sessions(
        name: &str,
        ids: &[&str],
    ) -> (PathBuf, PathBuf, PathBuf, settings::Settings, usize) {
        let dir = std::env::temp_dir().join(format!("no0b-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let sessions_dir = dir.join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("a sessions dir");
        let note = dir.join("no0b.sessions");
        let mut index = sessions::Index::default();
        for id in ids {
            std::fs::write(
                sessions_dir.join(format!("{id}.jsonl")),
                format!(
                    "{{\"t\":\"meta\",\"v\":1,\"id\":\"{id}\"}}\n{{\"t\":\"user\",\"text\":\"{id} said this\"}}\n"
                ),
            )
            .expect("a transcript");
            index = index.plus(id, &dir);
        }
        sessions::save_index(&note, &index).expect("the note is writable");
        let listing = sessions::read(&sessions_dir, &index, &picker::Disk);
        assert_eq!(listing.sessions.len(), ids.len(), "{listing:?}");
        let agent = agent::Agent {
            sessions: listing,
            ..agent::Agent::default()
        };
        let mut panel = settings::Settings::open(&config::Config::default(), None, agent);
        let at = panel
            .section_names()
            .iter()
            .position(|name| *name == settings::SESSIONS)
            .expect("the sessions section");
        panel.choose(at);
        let table = panel
            .rows()
            .iter()
            .position(|row| matches!(row, settings::Row::Table(_)))
            .expect("the section carries a table");
        (dir, sessions_dir, note, panel, table)
    }

    /// Which conversations the panel is listing, in the order it lists them.
    fn conversations(panel: &settings::Settings) -> Vec<String> {
        panel
            .rows()
            .iter()
            .find_map(|row| match row {
                settings::Row::Table(table) => {
                    Some(table.rows.iter().map(|row| row.id.clone()).collect())
                }
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Item H3: several conversations are marked and taken in one press.
    ///
    /// The panel decides and this file acts: the first press answers with
    /// nothing and puts the question on the footer, naming how many would go,
    /// and the second answers with a [`settings::Deed::ForgetSessions`] carrying
    /// the ids the rows were built from rather than anything read back off what
    /// was drawn. The deed is then run through the same free function the folder
    /// picker's own delete goes through, which is what keeps one route from
    /// deleting more or less than the other.
    ///
    /// This was one trash per row and one id per deed, so deleting four
    /// conversations was eight presses and four confirmations.
    #[test]
    fn the_panel_takes_every_marked_conversation_in_one_press() {
        let (dir, sessions_dir, note, mut panel, table) =
            a_panel_over_sessions("panel-forget-many", &["keep", "drop", "gone"]);
        let listed = conversations(&panel);
        assert_eq!(listed.len(), 3, "{listed:?}");

        // Two of the three, marked. The marks are not cleared by the arrow keys
        // the way an armed delete is: marking three rows means moving between
        // them, and a set the arrow keys emptied could only ever hold one.
        let ids: Vec<String> = listed
            .iter()
            .filter(|id| *id != "keep")
            .cloned()
            .collect();
        for id in &ids {
            let at = listed.iter().position(|row| row == id).expect("the row");
            assert!(panel.mark(table, at));
        }
        assert!(panel.step(true));
        assert!(panel.step(false));
        let marked = panel.table(table).expect("the table").chosen();
        assert_eq!(marked, 2, "an arrow key took the marks off");

        // Once: nothing happens, and the panel says how many would go rather
        // than asking "sure?" over a list of three.
        assert_eq!(panel.uninstall(table), None, "the first press deleted them");
        assert_eq!(panel.arming(), Some(table));
        let asked = panel.says();
        assert!(asked.contains("2 conversations"), "{asked}");
        assert!(asked.contains("press delete again"), "{asked}");
        assert!(sessions_dir.join("drop.jsonl").exists(), "it went anyway");

        // Anything else at all puts it back, so an armed delete cannot be left
        // sitting there for the next pointer that goes past.
        panel.step(true);
        assert_eq!(panel.arming(), None);
        assert_eq!(panel.uninstall(table), None, "it stayed armed");

        // Twice: the deed, naming both conversations by the ids the rows carry.
        let deed = panel.uninstall(table);
        assert_eq!(
            deed,
            Some(settings::Deed::ForgetSessions { ids: ids.clone() })
        );
        assert_eq!(panel.arming(), None, "it is still armed after it fired");

        // And what the deed asks for, done: both halves of both of them, and
        // nothing else.
        assert!(
            forget_sessions(Some(sessions_dir.clone()), Some(note.clone()), &ids).is_empty(),
            "a conversation refused"
        );
        assert!(!sessions_dir.join("drop.jsonl").exists());
        assert!(!sessions_dir.join("gone.jsonl").exists());
        assert!(sessions_dir.join("keep.jsonl").exists());
        assert_eq!(
            sessions::load_index(&note),
            sessions::Index::default().plus("keep", &dir),
            "the lines about them are still in the note"
        );

        // The panel re-read off the disk has lost both rows and every mark with
        // them, which is what the window does with `adopt_agent` after a write.
        let after = sessions::read(&sessions_dir, &sessions::load_index(&note), &picker::Disk);
        panel.adopt_agent(
            agent::Agent {
                sessions: after,
                ..agent::Agent::default()
            },
            &config::Config::default(),
        );
        assert_eq!(conversations(&panel), vec![String::from("keep")]);
        let table = panel
            .rows()
            .iter()
            .position(|row| matches!(row, settings::Row::Table(_)))
            .expect("the table is still there");
        assert_eq!(
            panel.table(table).expect("the table").chosen(),
            0,
            "a mark survived the conversation it was on"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The single row path is still one row: with nothing marked, the delete
    /// takes the conversation the keys are on and says which one it is.
    #[test]
    fn the_delete_with_nothing_marked_takes_the_row_the_keys_are_on() {
        let (dir, sessions_dir, note, mut panel, table) =
            a_panel_over_sessions("panel-forget-one", &["first", "second"]);
        let listed = conversations(&panel);
        // Down one row, so what goes is the row the keys are on rather than the
        // first one on the list.
        assert!(panel.step(true));
        let id = listed[1].clone();

        assert_eq!(panel.uninstall(table), None, "the first press deleted it");
        let asked = panel.says();
        assert!(asked.contains("press delete again"), "{asked}");
        assert!(
            !asked.contains("conversations"),
            "one conversation is named, not counted: {asked}"
        );
        let deed = panel.uninstall(table);
        assert_eq!(
            deed,
            Some(settings::Deed::ForgetSessions {
                ids: vec![id.clone()]
            })
        );
        assert!(forget_sessions(Some(sessions_dir.clone()), Some(note.clone()), &[id]).is_empty());
        assert_eq!(
            std::fs::read_dir(&sessions_dir)
                .expect("the directory")
                .count(),
            1,
            "it took more than the row the keys were on"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Select all marks every conversation on the list, not only the ones the
    /// table's body is showing, and select none takes them all off again.
    #[test]
    fn select_all_marks_every_conversation_on_the_list() {
        let ids: Vec<String> = (0..settings::TABLE_ROWS + 4)
            .map(|at| format!("s{at:02}"))
            .collect();
        let (dir, _, _, mut panel, table) = a_panel_over_sessions(
            "panel-select-all",
            &ids.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let all = conversations(&panel).len();
        assert!(all > settings::TABLE_ROWS, "the list fits in one screenful");

        assert!(panel.mark_all(table, true));
        assert_eq!(panel.table(table).expect("the table").chosen(), all);
        // And the delete then takes every one of them, which is what the header
        // and the button both say before it is pressed.
        let deed = panel.uninstall(table).is_none().then(|| panel.uninstall(table));
        let Some(Some(settings::Deed::ForgetSessions { ids })) = deed else {
            panic!("the delete did not take the marked list: {deed:?}");
        };
        assert_eq!(ids.len(), all);

        assert!(panel.mark_all(table, false));
        assert_eq!(panel.table(table).expect("the table").chosen(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One conversation of several refusing does not stop the rest: what could
    /// be deleted is deleted, and what refused is named.
    #[test]
    fn a_conversation_that_refuses_is_named_and_the_others_still_go() {
        let (dir, sessions_dir, note, ..) =
            a_panel_over_sessions("panel-forget-partial", &["one", "two", "three"]);
        // The middle one cannot be deleted: its transcript is a directory, which
        // the remove refuses, and the guard that resolves the name is the same
        // one either route goes through.
        std::fs::remove_file(sessions_dir.join("two.jsonl")).expect("the transcript");
        std::fs::create_dir(sessions_dir.join("two.jsonl")).expect("something in its place");
        std::fs::write(sessions_dir.join("two.jsonl").join("held"), "no").expect("a file in it");

        let ids: Vec<String> = ["one", "two", "three"]
            .into_iter()
            .map(String::from)
            .collect();
        let failed = forget_sessions(Some(sessions_dir.clone()), Some(note.clone()), &ids);
        assert_eq!(failed.len(), 1, "{failed:?}");
        assert!(
            failed[0].starts_with("two was not deleted"),
            "the failure does not name which one: {failed:?}"
        );
        // The other two went, and the note lost their lines with them.
        assert!(!sessions_dir.join("one.jsonl").exists());
        assert!(!sessions_dir.join("three.jsonl").exists());
        assert!(sessions_dir.join("two.jsonl").exists(), "it went after all");
        let left = sessions::load_index(&note);
        assert_eq!(left, sessions::Index::default().plus("two", &dir), "{left:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }


    /// What gets written down as a session's context reading, and when nothing
    /// does. The agent's own figure wins over the last request's, and a window
    /// whose size was never reported is not a reading at all: a number of tokens
    /// with nothing to compare it against says nothing on a row.
    #[test]
    fn the_context_written_down_is_the_agent_s_reading_before_the_last_request_s() {
        let fill = |used: u64, total: u64| state::ContextFill {
            used,
            total,
            compact_at: 0,
        };
        let usage = |prompt: u64, total: u64| noob_proto::Usage {
            prompt,
            cached_prompt: 0,
            completion: 12,
            context_total: total,
        };
        assert_eq!(
            context_reading(Some(fill(48_000, 200_000)), Some(usage(9, 10))),
            Some(sessions::Context {
                used: 48_000,
                total: 200_000
            })
        );
        assert_eq!(
            context_reading(None, Some(usage(30_000, 120_000))),
            Some(sessions::Context {
                used: 30_000,
                total: 120_000
            }),
            "the last request's, when the agent has not said"
        );
        assert_eq!(
            context_reading(Some(fill(5, 0)), Some(usage(30_000, 120_000))),
            Some(sessions::Context {
                used: 30_000,
                total: 120_000
            }),
            "a window of no size is not a reading, so it falls through"
        );
        assert_eq!(context_reading(None, None), None);
        assert_eq!(context_reading(Some(fill(5, 0)), Some(usage(9, 0))), None);
    }

    /// Ctrl-C reaches the panel. Every control-modified key but Ctrl-Q used to
    /// be swallowed there, which made the settings panel a surface you could
    /// read a skill's document on and not copy one word off it.
    #[test]
    fn control_c_copies_while_the_settings_panel_is_up() {
        assert_eq!(control_in_settings(Key::Character("c")), Control::Copy);
        assert_eq!(control_in_settings(Key::Character("q")), Control::Quit);
        // And the key every list is selected with, which the table of saved
        // conversations reads and nothing else on the panel does.
        assert_eq!(control_in_settings(Key::Character("a")), Control::MarkAll);
        // Nothing else has a meaning here: the panel owns the keyboard, so a
        // key that fell through would fall through to nothing.
        for key in [
            Key::Character("v"),
            Key::Named(NamedKey::Enter),
            Key::Named(NamedKey::ArrowDown),
        ] {
            let named = format!("{key:?}");
            assert_eq!(control_in_settings(key), Control::Nothing, "{named}");
        }
    }

    /// Item F1: Tab is the rail and the shifted arrow is the form.
    ///
    /// The panel's arrow keys are the rows of one section, so the rail had no
    /// key at all and a section pushed off a short rail could not be reached
    /// from the keyboard. Tab takes it, forward and back, and what Tab used to
    /// do (cross the two halves of a form row) moves onto shift with the arrow
    /// that points at the half. The plain arrows keep every meaning they had:
    /// nothing here answers for them.
    #[test]
    fn tab_walks_the_settings_sections_and_shift_crosses_the_form() {
        assert_eq!(
            walk_in_settings(Key::Named(NamedKey::Tab), false),
            Some(Walk::Section(true))
        );
        assert_eq!(
            walk_in_settings(Key::Named(NamedKey::Tab), true),
            Some(Walk::Section(false)),
            "shift-tab goes back"
        );
        assert_eq!(
            walk_in_settings(Key::Named(NamedKey::ArrowLeft), true),
            Some(Walk::Cross(settings::Side::Left))
        );
        assert_eq!(
            walk_in_settings(Key::Named(NamedKey::ArrowRight), true),
            Some(Walk::Cross(settings::Side::Right))
        );
        // Everything else belongs to the row under the cursor, the unshifted
        // arrows included: left and right are still the nudge.
        for key in [
            Key::Named(NamedKey::ArrowLeft),
            Key::Named(NamedKey::ArrowRight),
            Key::Named(NamedKey::ArrowUp),
            Key::Named(NamedKey::ArrowDown),
            Key::Named(NamedKey::Enter),
            Key::Named(NamedKey::Escape),
            Key::Character("a"),
        ] {
            let named = format!("{key:?}");
            assert_eq!(walk_in_settings(key.clone(), false), None, "{named}");
            if !matches!(
                key,
                Key::Named(NamedKey::ArrowLeft) | Key::Named(NamedKey::ArrowRight)
            ) {
                assert_eq!(walk_in_settings(key, true), None, "shifted {named}");
            }
        }
    }

    /// A selection worth copying, in one pane. Two spots apart, because a click
    /// that never moved is not a selection and lights no Copy row.
    fn a_selection_in(view: View) -> Option<select::Selection> {
        let mut selection =
            select::Selection::new(select::Where::Pane(view), select::Spot::new(0, 0));
        selection.extend(select::Spot::new(0, 4));
        Some(selection)
    }

    /// The copy row belongs to the pane the menu opened over. A selection in
    /// some other pane must not light it up, because copying would then hand
    /// over text from a pane nobody pointed at.
    #[test]
    fn the_copy_row_reads_the_selection_of_its_own_pane() {
        let dock = Dock::new();
        let layout = laid_out(&dock, None);
        let (view, tab) = layout.placed(Space::TopRight).tabs[0];
        let at = middle(tab);
        let hit = layout.hit(at.0, at.1);

        let mine = menu_for(hit, at, &dock, false, a_selection_in(view), None).unwrap();
        assert_eq!(mine.pick(1), Some(Item::CopySelection));
        let elsewhere = menu_for(hit, at, &dock, false, a_selection_in(View::Output), None).unwrap();
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
            menu_for(hit, at, &dock, true, None, None).unwrap().pick(0),
            Some(Item::Copy)
        );
        assert_eq!(menu_for(hit, at, &dock, false, None, None).unwrap().pick(0), None);
    }

    /// A row of the list is a switch: a widget in the window goes out, and one
    /// that is out comes back. This asserted the half of that which shipped,
    /// where a widget already in the window was only revealed, so the list could
    /// add a widget and never take one away.
    #[test]
    fn picking_a_widget_takes_it_out_of_the_window_or_puts_it_back() {
        let mut dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        menu.fold(3, &dock);

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
            menu.pick(4 + 7),
            Some(Item::Widget(View::Files, false)),
            "FILES is the eighth widget and it is back in the window"
        );
        toggle_view(&mut dock, &mut menu, View::Files);
        assert_eq!(
            menu.pick(4 + 7),
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
        menu.fold(3, &dock);

        // Another widget, either way round: the menu stays.
        assert!(toggle_view(&mut dock, &mut menu, View::Hardware).keep_open);
        assert!(toggle_view(&mut dock, &mut menu, View::Hardware).keep_open);
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
        menu.fold(3, &dock);
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
                menu.pick(4 + step),
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
        let layout = laid_out_at(&dock, None, 1200.0, 800.0);
        let column = layout.column_divider[0].band;
        // The right column's line: it is the half of the grid the window opens
        // with split, and each half now carries a line of its own.
        let row = layout.row_divider[1].band;
        let at = |panel: noob_draw::Panel| {
            let (x, y) = middle(panel);
            layout.hit(x, y)
        };
        assert_eq!(at(column), Some(Hit::ColumnDivider(0)));
        assert_eq!(at(row), Some(Hit::RowDivider(1)));
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
            cursor_for(false, Landing::Nowhere, Some(Dir::West), Some(Hit::ColumnDivider(0))),
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
            cursor_for(true, Landing::Out, None, Some(Hit::ColumnDivider(0))),
            CursorIcon::Crosshair
        );
    }

    /// Each line writes a key of its own, so a drag of one is not read back at
    /// the next launch as a drag of the line beside it.
    #[test]
    fn each_divider_remembers_its_own_half() {
        let keys = config::keys();
        let mut written = Vec::new();
        for grip in [
            Hit::ColumnDivider(0),
            Hit::ColumnDivider(1),
            Hit::RowDivider(0),
            Hit::RowDivider(1),
        ] {
            let key = divider_key(grip).unwrap_or_else(|| panic!("{grip:?} writes nothing"));
            assert!(keys.contains(&key), "{key} is not a key the file carries");
            written.push(key);
        }
        written.sort_unstable();
        written.dedup();
        assert_eq!(written.len(), 4, "two of the four lines share a key");
        // And nothing that is not a divider writes one of them.
        assert_eq!(divider_key(Hit::Body(Space::TopLeft)), None);
        assert_eq!(divider_key(Hit::TitleBar), None);
    }

    /// The settings panel's rail is dragged, remembered and pointed at the way
    /// the lines between panes are, and it writes a key of its own: a drag of it
    /// must not come back at the next launch as a column.
    #[test]
    fn the_settings_rail_is_a_divider_with_a_key_of_its_own() {
        let key = divider_key(Hit::SettingsRailDivider).expect("the rail writes nothing");
        assert_eq!(key, "settings_rail");
        assert!(config::keys().contains(&key), "{key} is not in the file");
        for grip in [
            Hit::ColumnDivider(0),
            Hit::ColumnDivider(1),
            Hit::RowDivider(0),
            Hit::RowDivider(1),
        ] {
            assert_ne!(divider_key(grip), Some(key), "{grip:?} writes the rail");
        }
        // And the pointer says it can be dragged at all, which is the only thing
        // that says so: the line itself is a hairline six pixels wide.
        assert_eq!(
            cursor_for(
                false,
                Landing::Nowhere,
                None,
                Some(Hit::SettingsRailDivider)
            ),
            CursorIcon::ColResize
        );
    }

    /// The settings the panel stopped listing still come out of the file and
    /// still arrange the window: PANES was removed from the panel, not from the
    /// window.
    ///
    /// Its rows were which panes are open and where the dividers sit. Both are
    /// set by using the window (a closed pane comes back off the right click
    /// menu, a line is dragged), and both are still written to and read from the
    /// same keys, so an arrangement survives a restart with nothing on the panel
    /// to type it into. This is the half of item D1 that must not have changed.
    #[test]
    fn the_layout_the_panel_stopped_listing_still_comes_out_of_the_file() {
        let config = Config::parse(
            "show_activity = off\nleft_width = 0.30\nleft_width_bottom = 0.70\ntop_height = 0.35\ntop_height_right = 0.65\nsettings_rail = 0.40\n",
        );
        // Every key the panel dropped is still a key the file understands, and
        // the parser still read all six of these off it.
        for key in settings::OFF_PANEL {
            assert!(config::keys().contains(&key), "{key} left the file as well");
        }
        assert!(!config.show_activity && config.show_files);
        assert_eq!(
            [
                config.left_width,
                config.left_width_bottom,
                config.top_height,
                config.top_height_right,
                config.settings_rail
            ],
            [0.30, 0.70, 0.35, 0.65, 0.40]
        );

        // The pane the file turns off is out of the window at launch, exactly
        // the way `App::new` puts it out, and the right click menu is the way
        // back in: that list is the affordance the settings rows duplicated.
        let mut hidden = Vec::new();
        if !config.show_activity {
            hidden.push(View::Activity);
        }
        if !config.show_files {
            hidden.push(View::Files);
        }
        let mut dock = Dock::hiding(&hidden);
        assert!(dock.is_hidden(View::Activity));
        assert!(!dock.is_hidden(View::Files));
        let mut menu = Menu::for_widget((0.0, 0.0), View::Output, Space::TopLeft, false);
        menu.fold(3, &dock);
        assert!(!toggle_view(&mut dock, &mut menu, View::Activity).hidden);
        assert!(!dock.is_hidden(View::Activity), "the menu cannot reopen it");

        // And the grid breaks where the file says it does. The line is drawn at
        // the fraction the file carries, which is the same arithmetic a drag
        // reads a pointer with, so what is on screen and what is written are one
        // number.
        let dock = Dock::new();
        let shape = Shape {
            shaded: false,
            dock: &dock,
            menu: None,
            picker: None,
            settings: None,
            file_labels: Vec::new(),
            file_first: 0,
            column: COLUMN,
            menu_column: COLUMN,
            pane_size: config.pane_font_size,
            pane_column: COLUMN,
            input_h: view::input_height(config.prompt_rows, noob_draw::Text::line_for(SIZE)),
            left_width: [config.left_width, config.left_width_bottom],
            top_height: [config.top_height, config.top_height_right],
            settings_rail: config.settings_rail,
            popup: None,
        };
        let layout = Layout::compute(W, H, &shape);
        // One line cuts the grid in half and each half is cut by one of its own,
        // and a line beside a space standing empty is not there at all, so which
        // of the four are on screen is the dock's business. Every line that is
        // there is at the fraction its own key carries: the same arithmetic a
        // drag reads a pointer with, so what is on screen and what is written
        // are one number.
        let column_wants = [config.left_width, config.left_width_bottom];
        let row_wants = [config.top_height, config.top_height_right];
        let there = |band: noob_draw::Panel| band.w >= 1.0 && band.h >= 1.0;
        let (mut columns, mut rows) = (0, 0);
        for half in 0..2 {
            let band = layout.column_divider[half].band;
            if there(band) {
                let (x, _) = middle(band);
                assert!(
                    (layout.column_ratio_at(half, x) - column_wants[half]).abs() < 0.01,
                    "the {half} column line is not at {}",
                    column_wants[half]
                );
                columns += 1;
            }
            let band = layout.row_divider[half].band;
            if there(band) {
                let (_, y) = middle(band);
                assert!(
                    (layout.row_ratio_at(half, y) - row_wants[half]).abs() < 0.01,
                    "the {half} row line is not at {}",
                    row_wants[half]
                );
                rows += 1;
            }
        }
        assert!(columns >= 1 && rows >= 1, "the grid was cut by nothing at all");
        // Where both halves of an axis are on screen they kept their own numbers
        // rather than both taking one, which is the whole reason there are four
        // keys and not two.
        if there(layout.column_divider[0].band) && there(layout.column_divider[1].band) {
            assert_ne!(
                layout.column_divider[0].band.x,
                layout.column_divider[1].band.x
            );
        }
        if there(layout.row_divider[0].band) && there(layout.row_divider[1].band) {
            assert_ne!(layout.row_divider[0].band.y, layout.row_divider[1].band.y);
        }

        // And dragging any of the five still writes its own key, which is what
        // the panel rows were standing in for.
        for (grip, key) in [
            (Hit::ColumnDivider(0), "left_width"),
            (Hit::ColumnDivider(1), "left_width_bottom"),
            (Hit::RowDivider(0), "top_height"),
            (Hit::RowDivider(1), "top_height_right"),
            (Hit::SettingsRailDivider, "settings_rail"),
        ] {
            assert_eq!(divider_key(grip), Some(key));
            assert!(settings::OFF_PANEL.contains(&key), "{key} is on the panel");
        }
    }

    /// The landing the cursor is driven from is the layout's own, so the shape
    /// the pointer takes and the move the release makes come from one answer.
    #[test]
    fn the_delete_cursor_comes_from_the_same_landing_the_drop_does() {
        let dock = Dock::new();
        let layout = laid_out_at(&dock, None, 1200.0, 800.0);
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
            laid_out_at(dock, None, AT.0, AT.1).grid[space.index()]
        };
        let drop_at = |dock: &mut Dock, view: View, (x, y): (f32, f32)| {
            let landing = laid_out_at(dock, None, AT.0, AT.1).landing(x, y);
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
        let layout = laid_out_at(&dock, None, AT.0, AT.1);
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
            View::Hardware,
            (bottom.x + bottom.w * 0.5, bottom.y + bottom.h * 0.5),
        );
        assert_eq!(landing, Landing::In(Space::BottomRight, None));
        assert!(moved);
        assert_eq!(dock.slot(Space::BottomRight).views, vec![View::Hardware]);
        assert_eq!(
            dock.cover()[Space::BottomRight.index()],
            Some(Space::BottomRight),
            "two panes, one cell each"
        );
        let layout = laid_out_at(&dock, None, AT.0, AT.1);
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
        let layout = laid_out(&dock, None);
        let y = layout.input.y + layout.input.h * 0.5;
        let chars = prompt.len();
        let caret = |x: f32| layout.input_caret(x, y, SIZE, COLUMN, chars, 0);
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

    /// A press on an activity row opens the call that wrote that row, and a
    /// press anywhere else puts the popup away again.
    ///
    /// The two halves `App::open_call_under_pointer` is: `spot_in_pane` for the
    /// row under the pointer and `State::call_at_line` for the call that wrote
    /// it. Driven through the real layout at a real pixel, so a row that is drawn
    /// somewhere other than where it is tested would fail here.
    #[test]
    fn a_press_on_an_activity_row_opens_the_call_under_the_pointer() {
        let mut state = State::new();
        state.apply(noob_proto::Event::TurnStart { turn: 1 });
        state.apply(noob_proto::Event::ToolStart {
            call_id: "a".into(),
            name: "read".into(),
            brief: "src/lib.rs".into(),
            args: serde_json::json!({"path": "src/lib.rs"}),
        });
        state.apply(noob_proto::Event::ToolStart {
            call_id: "b".into(),
            name: "bash".into(),
            brief: String::new(),
            args: serde_json::json!({"cmd": "cargo test"}),
        });

        let dock = Dock::new();
        let space = Space::ALL
            .into_iter()
            .find(|space| dock.slot(*space).active() == Some(View::Activity))
            .expect("the activity list is in the window");
        let layout = laid_out(&dock, None);
        let size = Config::default().pane_font_size;
        let inner = layout.content(space).inset(9.0);
        let line = noob_draw::Text::line_for(size);
        let row = |n: usize| (inner.x + 2.0, inner.y + (n as f32 + 0.5) * line);
        let call_at = |n: usize| {
            let (x, y) = row(n);
            let spot = spot_in_pane(&layout, space, View::Activity, &state.activity, x, y, size, COLUMN)
                .expect("a row under the pointer");
            state.call_at_line(spot.line)
        };

        // Each of the two rows resolves to its own call, not to the other one.
        let first = call_at(0).expect("the first row is a call");
        let second = call_at(1).expect("the second row is a call");
        assert_ne!(first, second);
        assert_eq!(state.call(first).expect("held").call_id, "a");
        assert_eq!(state.call(second).expect("held").call_id, "b");

        // Well below the last row there is no call to open, and the press stays
        // a selection.
        let (x, y) = row(40);
        let spot = spot_in_pane(&layout, space, View::Activity, &state.activity, x, y, size, COLUMN)
            .expect("a press below the text still selects");
        assert_eq!(state.call_at_line(spot.line), Some(second), "the last row is the bash");
        assert_eq!(state.call_at_line(spot.line + 5), None);
    }

    /// The popup takes the press that lands on it and lets every other press
    /// through to close it, which is the whole of how it closes.
    #[test]
    fn the_popup_swallows_its_own_box_and_nothing_else() {
        let mut state = State::new();
        state.apply(noob_proto::Event::ToolStart {
            call_id: "a".into(),
            name: "read".into(),
            brief: "src/lib.rs".into(),
            args: serde_json::json!({"path": "src/lib.rs"}),
        });
        state.open_call = Some(0);

        let dock = Dock::new();
        let call = state.popped().expect("the popup is up");
        let layout = laid_out_with_popup(&dock, Some(call));
        let box_ = layout.call_popup;
        assert!(box_.w >= 1.0 && box_.h >= 1.0, "it has a box: {box_:?}");
        assert_eq!(layout.hit(box_.x + 4.0, box_.y + 4.0), Some(Hit::CallPopup));
        assert_eq!(middle(box_), (W * 0.5, H * 0.5), "centred on the window");

        // A press outside it is not the popup's, so `App::click` closes the
        // popup and stops there.
        let outside = layout.hit(box_.x - 8.0, box_.y + box_.h * 0.5);
        assert!(!matches!(outside, Some(Hit::CallPopup)), "{outside:?}");

        // And with nothing open there is no region at all, so nothing can be
        // clicked onto a popup that is not there.
        let shut = laid_out_with_popup(&dock, None);
        assert!(shut.call_popup.w < 1.0);
        assert!(!matches!(shut.hit(W * 0.5, H * 0.5), Some(Hit::CallPopup)));
    }

    fn laid_out_with_popup<'a>(dock: &'a Dock, popup: Option<&'a state::Call>) -> Layout {
        let shape = Shape {
            shaded: false,
            dock,
            menu: None,
            picker: None,
            settings: None,
            file_labels: Vec::new(),
            file_first: 0,
            column: COLUMN,
            menu_column: COLUMN,
            pane_size: Config::default().pane_font_size,
            pane_column: COLUMN,
            input_h: view::input_height(
                Config::default().prompt_rows,
                noob_draw::Text::line_for(SIZE),
            ),
            left_width: [Config::default().left_width; 2],
            top_height: [Config::default().top_height; 2],
            settings_rail: Config::default().settings_rail,
            popup,
        };
        Layout::compute(W, H, &shape)
    }

    /// A pane of known text, and the pixel-to-character half of a selection in
    /// it. `Model::spot_at` is a two line wrapper around `spot_in_pane`;
    /// everything that decides which character is under the pointer is here.
    fn output_pane(lines: &[&str]) -> (Dock, Layout, Space, state::Pane) {
        let dock = Dock::new();
        let layout = laid_out(&dock, None);
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
        let mut selection = select::Selection::new(select::Where::Pane(View::Output), spot(at(0, 0)));
        selection.extend(spot(at(0, 11)));
        assert_eq!(selection.text(&pane), "hello world");

        // And the last character of the whole buffer, from the start of it.
        let mut selection = select::Selection::new(select::Where::Pane(View::Output), spot(at(0, 0)));
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
        let mut selection = select::Selection::new(select::Where::Pane(View::Output), start);
        selection.extend(spot(W + 500.0, inner.y + line * 0.5));
        assert_eq!(
            selection.text(&pane),
            "hello world",
            "a drag off the right takes the rest of the row"
        );

        // And below the pane, which is the sweep to the end of the text.
        let mut selection = select::Selection::new(select::Where::Pane(View::Output), start);
        selection.extend(spot(W + 500.0, body.y + body.h + 400.0));
        assert_eq!(selection.text(&pane), "hello world\nsecond line");

        // Above and to the left of it, which is the same sweep backwards.
        let mut selection = select::Selection::new(select::Where::Pane(View::Output), spot(W, body.y + body.h));
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

    /// A click on a row that continues a file line lands on the character under
    /// the pointer.
    ///
    /// The file view spends four columns on its line numbers and the hit test
    /// takes them off every row, which is only right if every row is drawn four
    /// columns in. The number is written on the first row of a line and the
    /// rows it wraps onto are indented under the text, so the four columns are
    /// there to take off on all of them. While the continuation rows started at
    /// the left edge, a click on one landed four characters along.
    #[test]
    fn a_click_on_a_row_that_continues_a_file_line_lands_on_the_character_under_it() {
        let dock = Dock::new();
        let layout = laid_out(&dock, None);
        let space = Space::ALL
            .into_iter()
            .find(|space| dock.slot(*space).active() == Some(View::Files))
            .expect("the file view is in the window");
        let body = layout.content(space);
        let size = Config::default().pane_font_size;
        let (cols, chrome) = view::text_columns(View::Files, body, COLUMN);
        assert!(chrome > 0, "the file view has no gutter to take off");

        let long: String = std::iter::repeat_n("a word of it ", cols / 4)
            .collect::<String>()
            .trim_end()
            .to_string();
        let mut pane = state::Pane::new(100);
        pane.push(state::Line::new("fn main() {}", state::Tone::Body).at(6));
        pane.push(state::Line::new(long.clone(), state::Tone::Body).at(7));

        let rows = layout.rows(body, size);
        let spans = pane.rows_of_line(1, cols);
        assert!(spans.len() >= 3, "the line under test does not wrap far enough");
        let inner = body.inset(9.0);
        let line_h = noob_draw::Text::line_for(size);

        // Row 0 is the short line, so the wrapped line starts on row 1 and every
        // row after that continues it.
        for (wrapped, span) in spans.iter().enumerate() {
            let row = 1 + wrapped;
            for at in [0usize, 1, span.len().saturating_sub(1)] {
                let x = inner.x + (chrome + at) as f32 * COLUMN;
                let y = inner.y + (row as f32 + 0.5) * line_h;
                let spot = spot_in_pane(&layout, space, View::Files, &pane, x, y, size, COLUMN)
                    .expect("a row of the file has a character under the pointer");
                assert_eq!(
                    spot,
                    select::Spot::new(1, span.start + at),
                    "row {row}, column {at}: the pointer is over {:?}",
                    long.chars().nth(span.start + at)
                );
            }
        }
        assert!(rows > spans.len() + 1, "the file did not fit in the pane");

        // And a press on the gutter itself is the first character of that row,
        // not a character of the row above.
        let last = spans.last().expect("the line has rows");
        let y = inner.y + (spans.len() as f32 + 0.5) * line_h;
        let spot = spot_in_pane(&layout, space, View::Files, &pane, inner.x, y, size, COLUMN)
            .expect("a press on the gutter still selects");
        assert_eq!(spot, select::Spot::new(1, last.start));
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
        let layout = laid_out(&dock, None);
        let prompt = Prompt::default();
        let frame = view::Frame {
            state: &state,
            scrolls: &scroll::Scrolls::default(),
            file_scroll: 0,
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
            orb_morph: None,
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
