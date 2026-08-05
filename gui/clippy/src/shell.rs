//! What the window's events mean, as plain functions: whether a press was a
//! double click, which tab it landed on, where a spot in a pane is, what a key
//! asks the settings panel for, and what a deed writes to disk.
//!
//! Split from `main` so the event loop reads as a loop. Nothing here holds
//! state; the App calls in and takes the answer.

use std::path::PathBuf;
use std::time::Instant;

use winit::dpi::LogicalSize;
use winit::keyboard::Key;
use winit::window::CursorIcon;

#[allow(clippy::wildcard_imports)]
use crate::*;

/// Whether the sampling clock should be running: one of [`SAMPLED`] is the
/// showing view of an unfolded space, and nothing is covering the window.
///
/// `covered` is either takeover, the folder picker or the settings panel. Both
/// draw over every pane, so a monitor behind one is not on screen and the clock
/// that reads the kernel for it has nothing to feed.
pub(crate) fn sampling(shaded: bool, covered: bool, dock: &Dock) -> bool {
    !shaded
        && !covered
        && Space::ALL.into_iter().any(|space| {
            let slot = dock.slot(space);
            !slot.folded && slot.active().is_some_and(|view| SAMPLED.contains(&view))
        })
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
pub(crate) fn shade_request(
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
/// Everything shading asks the window for at once, so the order the three are
/// asked in lives in one place.
#[derive(Debug, PartialEq)]
pub(crate) struct ShadeRequest {
    /// The minimum inner size to hold the window to, or none at all.
    pub(crate) min: Option<LogicalSize<f64>>,
    /// The inner size to become, if a size is asked for.
    pub(crate) size: Option<PhysicalSize<u32>>,
    /// The maximized state to put the window in, when it has to change.
    pub(crate) maximized: Option<bool>,
}
/// What a shaded window turned out to be, read off the surface the compositor
/// handed back. See [`shade_of`].
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum Shade {
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
pub(crate) fn shade_of(shaded: bool, maximized: bool, settling: bool, height: u32, strip: u32) -> Shade {
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
pub(crate) fn began_move(pressed: bool, moved: f64) -> bool {
    pressed && moved >= DRAG_SLOP
}
/// What a press on the title bar turns out to be. See [`title_click`].
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum TitleClick {
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
pub(crate) fn title_click(double: bool) -> TitleClick {
    if double {
        TitleClick::Maximize
    } else {
        TitleClick::ArmMove
    }
}
/// What letting go of a held tab turned out to be. See [`click_tab`].
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum TabClick {
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
pub(crate) fn click_tab(slot: &mut dock::Slot, view: View) -> TabClick {
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
pub(crate) fn orb_deadline(now: Instant, animating: bool, pending: Option<Instant>) -> Option<Instant> {
    if !animating {
        return None;
    }
    match pending {
        // Still waiting on the frame that was asked for.
        Some(at) if now < at => Some(at),
        _ => Some(now + ORB_EVERY),
    }
}
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
pub(crate) struct Morph {
    /// Where it is now: 0 is the resting square, 1 is the turning circles.
    pub(crate) at: f32,
    /// Where it stood when the turn last started or ended, so a turn that ends
    /// mid-move turns the orb around from where it had got to rather than from
    /// the end it never reached.
    pub(crate) from: f32,
    pub(crate) since: Instant,
    /// Whether a turn was running at that moment.
    pub(crate) busy: bool,
}
/// The one moment the event loop waits until, out of every clock the window
/// holds.
///
/// Composed rather than assigned. Two clocks that each set the control flow
/// leave whichever ran last in charge, and the other one either wakes late or
/// never wakes at all, which is a monitor that stops sampling as soon as
/// something animates.
pub(crate) fn soonest(deadlines: [Option<Instant>; 4]) -> Option<Instant> {
    deadlines.into_iter().flatten().min()
}
/// Which scrollbar a press took: a pane's, the file explorer's own, or the
/// call popup's. One press, one track, decided where the button went down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Thumb {
    Pane(Space),
    Explorer(Space),
    Popup(crate::widgets::popup::Half),
}
/// The folder named on the command line, if one was.
///
/// The first argument that is not a flag. Without one the window opens on the
/// picker: `current_dir()` under a desktop launcher is `$HOME`, and handing the
/// agent the home directory because nobody said otherwise is what this replaces.
pub(crate) fn workspace_arg(args: &[String]) -> Option<PathBuf> {
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
pub(crate) fn forget_session(
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
pub(crate) fn forget_sessions(dir: Option<PathBuf>, index: Option<PathBuf>, ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter_map(|id| forget_session(dir.clone(), index.clone(), id).err())
        .collect()
}
/// The disk half of a panel deed: which agent file it touches, and the write
/// itself. One function shared by the panel's button path ([`App::do_deed`])
/// and the slash command path ([`App::command_deed`]), so the two cannot do
/// different things to the same file.
///
/// A free function so an add or a turn can be proven against a scratch file
/// without a window. The two deeds that are not the agent's answer Ok here
/// and are handled by their callers: a set of conversations cannot answer
/// with one result, and a restore writes the window's own file.
pub(crate) fn deed_on_disk(
    deed: &settings::Deed,
    skills_at: Option<&Path>,
    mcp_global: Option<&Path>,
    mcp_project: Option<&Path>,
) -> Result<(), String> {
    match deed {
        settings::Deed::TurnSkill { dir, on } => match skills_at {
            Some(at) => agent::set_skill(at, dir, *on),
            None => Err(String::from("there is no skills directory to move it in")),
        },
        settings::Deed::RemoveSkill { dir, on } => match skills_at {
            Some(at) => agent::remove_skill(at, dir, *on),
            None => Err(String::from("there is no skills directory to remove it from")),
        },
        settings::Deed::TurnServer { name, project, on } => {
            match *project {
                true => mcp_project,
                false => mcp_global,
            }
            .map(|path| agent::set_server(path, name, *on))
            .unwrap_or_else(|| Err(String::from("there is no file to write that server in")))
        }
        settings::Deed::RemoveServer { name, project } => match *project {
            true => mcp_project,
            false => mcp_global,
        }
        .map(|path| agent::remove_server(path, name))
        .unwrap_or_else(|| Err(String::from("there is no file to take that server out of"))),
        // Always the global file: it is the one the add card names, and
        // the one the agent reads in every project.
        settings::Deed::AddServer { name, how } => match mcp_global {
            Some(path) => agent::add_server(path, name, how),
            None => Err(String::from("there is no config directory to write a server in")),
        },
        // The editor's save: the whole file at once, by the same rename
        // every write in the agent box arrives by.
        settings::Deed::SaveInstructions { path, text } => agent::write_instructions(path, text),
        // The restore: the file parks in the .bak beside it, then the shipped
        // default lands as the file. The path and the text both came off the
        // panel, which read them off the agent box's own constants.
        settings::Deed::RestorePrompt { path, default } => agent::restore_prompt(path, default),
        settings::Deed::ForgetSessions { .. } | settings::Deed::RestoreLooks => Ok(()),
    }
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
pub(crate) fn context_reading(
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
pub(crate) fn menu_for(
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
        Hit::Body(space)
        | Hit::Scrollbar(space)
        | Hit::File(_, space)
        | Hit::TabsLeft(space)
        | Hit::TabsRight(space) => widget(dock.slot(space).active()?, space),
        Hit::TitleBar | Hit::Close | Hit::Maximize | Hit::Minimize => None,
        // The menu already open. Its own right click is handled before this is
        // reached, and a row is picked with the left button.
        //
        // The popup floats on the same layer and gets the same answer: it is
        // about one call rather than about a widget, so there is no pane for a
        // Close or a Copy row to act on.
        Hit::Menu | Hit::MenuRow(_) | Hit::CallPopup | Hit::CallPopupClose
        | Hit::CallPopupScrollbar => None,
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
        // A conversation on the SESSIONS table can be opened and deleted, the
        // same two acts its row in the picker carries, reachable while a
        // window is connected.
        Hit::SettingsPick(index, row) | Hit::SettingsMark(index, row) => {
            Some(Menu::for_kept(at, index, row))
        }
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
        | Hit::SettingsAct(..)
        | Hit::SettingsClose => None,
        // A divider is the gap between two widgets and belongs to neither of
        // them, so there is no one widget for a menu opened here to act on.
        Hit::ColumnDivider(_) | Hit::RowDivider(_) | Hit::SettingsRailDivider => None,
    }
}
/// What picking a row of the menu's Widgets group does, and what becomes of the
/// menu afterwards.
pub(crate) struct Toggled {
    /// The widget went out of the window rather than coming back into it.
    pub(crate) hidden: bool,
    /// The menu can stay open. It cannot when the widget that went out is the
    /// one the menu was opened over: its Close row and its Copy row would be
    /// pointed at a pane that is no longer in the window.
    pub(crate) keep_open: bool,
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
pub(crate) fn toggle_view(dock: &mut Dock, menu: &mut Menu, view: View) -> Toggled {
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
pub(crate) fn pane_changes(was: &Config, now: &Config) -> Vec<(View, bool)> {
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
pub(crate) fn walked(showing: usize, forward: bool, tabs: usize) -> usize {
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
pub(crate) fn walk_tabs(dock: &mut Dock, space: Space, showing: usize, forward: bool) -> bool {
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
pub(crate) fn land(dock: &mut Dock, view: View, landing: Landing) -> bool {
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
pub(crate) fn pasted(raw: &str) -> String {
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
pub(crate) fn spot_in_pane(
    layout: &view::Layout,
    space: Space,
    view: View,
    pane: &state::Pane,
    x: f32,
    y: f32,
    size: f32,
    column: f32,
    reserved: usize,
) -> Option<select::Spot> {
    let (row, at) = layout.cell_in(space, x, y, size, column)?;
    // The box the glyphs are in, which is not the whole pane in the file
    // view: its left column is the explorer.
    let body = layout.content(space);
    // Minus the rows the caller says are pinned under the transcript: a
    // click counted in rows the text was not drawn in lands that many lines
    // away from the character under the pointer.
    let rows = layout.rows(body, size).saturating_sub(reserved);
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
pub(crate) fn spot_in_rows(
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
/// Which of those a key is.
///
/// A free function because the panel is a takeover and this is the whole of
/// what reaches it with control held: the list used to be Ctrl-Q alone, which
/// made the panel a surface you could read a document on and not copy a word
/// off. Nothing here touches the window, so the list is testable.
pub(crate) fn control_in_settings(key: Key<&str>) -> Control {
    match key {
        Key::Character("q") => Control::Quit,
        Key::Character("c") => Control::Copy,
        Key::Character("a") => Control::MarkAll,
        Key::Character("s") => Control::Save,
        _ => Control::Nothing,
    }
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
pub(crate) fn walk_in_settings(key: Key<&str>, shift: bool) -> Option<Walk> {
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
pub(crate) fn spot_in_doc(
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
/// Where in a document block on the settings list the pointer is: the row it
/// is on, and the character along it.
///
/// The block's text box is worked out with the same three calls the painter
/// draws it with, so a drag lands on the character under the pointer rather
/// than on one a rounding away.
pub(crate) fn spot_in_paper(
    layout: &view::Layout,
    panel: &settings::Settings,
    index: usize,
    x: f32,
    y: f32,
    size: f32,
    column: f32,
) -> Option<select::Spot> {
    use settings::places::{settings_card, settings_card_parts, settings_paper_text};
    let line = noob_draw::Text::line_for(size);
    let (_, _, row) = *layout
        .settings_rows
        .iter()
        .find(|(at, _, _)| *at == index)?;
    let card = settings_card(row, line);
    let cols = layout.settings_entry_columns(column);
    let parts = settings_card_parts(card, line, size, column, cols, true);
    let text = settings_paper_text(&parts, line);
    if text.w < 1.0 || text.h < 1.0 || !holds(text, x, y) {
        return None;
    }
    let rows = (text.h / line).floor().max(1.0) as usize;
    let body_cols = view::columns_in(text.w, column);
    let at_row = (((y - text.y) / line).floor().max(0.0) as usize).min(rows.saturating_sub(1));
    let at_col = (((x - text.x) / column).round().max(0.0) as usize).min(body_cols);
    let pane = panel.paper_pane_at(index, rows);
    (pane.last() > 0).then(|| spot_in_rows(&pane, rows, body_cols, at_row, at_col))
}
/// Whether a box holds a point. The layout's own hit tests are private to it,
/// and this is the one place outside them that has to ask.
pub(crate) fn holds(box_: noob_draw::Panel, x: f32, y: f32) -> bool {
    x >= box_.x && x < box_.x + box_.w && y >= box_.y && y < box_.y + box_.h
}
/// What time it is on the reader's own clock, as seconds past local midnight.
///
/// `std` has a wall clock and no timezone at all, and this window keeps no date
/// crate in its graph, so the local reading is asked of the one program every
/// unix ships. A window that gets no answer leaves `day_zero` unset and its
/// rows carry no clock, which is better than a column of times an hour out.
pub(crate) fn local_day_second() -> Option<u64> {
    let told = std::process::Command::new("date")
        .arg("+%H:%M:%S")
        .output()
        .ok()?;
    state::day_second(std::str::from_utf8(&told.stdout).ok()?)
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
pub(crate) fn divider_key(grip: Hit) -> Option<&'static str> {
    match grip {
        Hit::ColumnDivider(0) => Some("left_width"),
        Hit::ColumnDivider(_) => Some("left_width_bottom"),
        Hit::RowDivider(0) => Some("top_height"),
        Hit::RowDivider(_) => Some("top_height_right"),
        Hit::SettingsRailDivider => Some("settings_rail"),
        _ => None,
    }
}
pub(crate) fn cursor_for(
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
pub(crate) fn resize_cursor(dir: winit::window::ResizeDirection) -> CursorIcon {
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
/// The line for a prompt that went out and started nothing.
pub(crate) fn unanswered() -> String {
    format!(
        "nothing came back in {}s. the agent has the prompt and started no turn; \
         run `noob doctor` to check its endpoint.",
        ANSWER_WAIT.as_secs()
    )
}
