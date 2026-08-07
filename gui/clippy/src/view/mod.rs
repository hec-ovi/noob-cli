//! Layout, hit regions, and turning state into a scene.
//!
//! One surface carved into a 2x2 grid, never several OS windows. The window
//! has no system chrome, so the title bar, its three buttons, the tab strips,
//! the scrollbars and the resize edges are all rectangles here and hit regions
//! in [`Layout`]. Drawing and hit testing take the same numbers from the same
//! place, which is the only way they can never disagree.
//!
//! Every view is a tab in one cell of that grid and can be dragged into another
//! cell, or onto the line between two cells, which gives its pane the pair;
//! [`crate::dock`] owns that arrangement, and this module only asks it where
//! things are and turns cells into boxes.
//!
//! The window has three shapes. Open, it is the grid. Shaded, it is one strip
//! carrying [`State::headline`] and nothing else, the way Winamp collapsed to
//! its title; double-click the bar to go between those two. Before a folder has
//! been chosen it is the picker and nothing else, because there is no agent to
//! arrange panes around yet.

use noob_draw::{Panel, Run, Scene, Text};

use crate::dock::{Dock, Space, View};
use crate::menu::paint::{overlay, place_menu, MenuPlaces};
use crate::menu::Menu;
use crate::monitor::Monitor;
use crate::picker::Picker;
use crate::settings::{Act, Settings, Side};
use crate::skin::Skin;
use crate::state::State;
#[allow(clippy::wildcard_imports)]
pub(crate) use chrome::*;
#[allow(clippy::wildcard_imports)]
pub(crate) use place::*;
#[allow(clippy::wildcard_imports)]
pub(crate) use draw::*;
use crate::widgets::files::{
    DIFF_MIN_COLUMNS, GUTTER, LIST_MAX_COLUMNS, LIST_MIN_COLUMNS, ROW_ICON_COLUMNS,
    ROW_MARK_COLUMNS,
};

pub const TITLE_H: f32 = 30.0;
const INPUT_H: f32 = 36.0;
const TAB_H: f32 = 22.0;
const RESIZE_EDGE: f32 = 6.0;
pub(crate) const GAP: f32 = 6.0;
pub(crate) const PAD: f32 = 9.0;
pub(crate) const SMALL: f32 = 12.0;
pub(crate) const SCROLL_W: f32 = 4.0;
const BUTTON_W: f32 = 26.0;
/// The square at the left end of the title strip that the orb is drawn in.
///
/// The strip's text starts after this, so the orb sits in a slot of its own
/// instead of over the name, and the strip reads
/// `[orb] NO0B \u{25b8} version` left to right. The orb sizes itself to whatever
/// square it is handed, so this is the only number that decides how big it is.
pub const ORB_W: f32 = TITLE_H;



const PROMPT_COLUMNS: usize = 2;
/// The three dots that stand in for the prompt's marker while a turn runs: how
/// big one is, the gap between two of them, and how far the raised one rises,
/// all in pixels.
///
/// They fit inside [`PROMPT_COLUMNS`], which at the default 8 pixel column is 16
/// across: three threes and two gaps is thirteen, with a pixel and a half either
/// side. Fitting them in the slot the marker already had is what keeps the
/// caret, the selection band, the prompt's height and the click inverse out of
/// this: all four only ever add [`PROMPT_COLUMNS`].
const PROMPT_DOT: f32 = 3.0;
const PROMPT_DOT_GAP: f32 = 2.0;
const PROMPT_DOT_LIFT: f32 = 3.0;
/// How long one dot holds the top, in seconds. The redraw while a turn runs is
/// 30 frames a second, so this is about five frames a dot: slow enough to read
/// as a wave rather than as a flicker.
const PROMPT_DOT_STEP: f32 = 0.18;
pub(crate) const INPUT_PAD: f32 = 6.0;
/// How far the 45 degree cut reaches along each edge of a panel's top-right
/// corner. One corner, so the shape reads as a mark rather than as a rounded
/// box, and always the same corner so two panels side by side still line up.
pub(crate) const CUT: f32 = 10.0;
/// Columns each of the two arrows on an overflowing tab strip takes.
///
/// Three, which is what a tab spends on padding around its label, so an arrow is
/// a target about as wide as the narrowest tab could be rather than one glyph
/// wide. Both come off the strip's right end before any tab is placed.
const TAB_ARROW_COLUMNS: usize = 3;
/// The accent line along the top of the tab that is showing. Two pixels: one
/// reads as the hairline every other edge in the window is, and the tab has to
/// say which view it is holding from further away than that.
const ACCENT_H: f32 = 2.0;
/// The border down the showing tab's left and right edges. One pixel, the
/// hairline every straight edge in the window is drawn at, so the sides outline
/// the tab without competing with the accent along its top.
const TAB_EDGE_H: f32 = ACCENT_H * 0.5;
/// The weight a cut corner's diagonal is drawn at, on a tab and on a pane alike.
///
/// Twice the hairline the straight sides take. The diagonal is the one edge that
/// says what shape the box is, and at the same weight as the sides it read as a
/// gap in the border rather than as the corner turning.
const CUT_EDGE_H: f32 = 2.0;
/// How far a scrollbar sits in from the right edge of the pane it belongs to.
pub(crate) const SCROLL_GAP: f32 = 2.0;
/// How wide the folder picker gets, in pane columns.
///
/// One width for both of its lists, because the box must not move when the
/// button that swaps them is pressed. That makes this the width of the wider of
/// the two, which is the session table: five columns of content, four of them a
/// fixed size ([`SESSION_COLUMNS`]) and the last one holding what was first said
/// in the session. Sixty-four columns fitted the folder list alone and left the
/// opening line four words wide.
pub(crate) const PICKER_COLUMNS: usize = 96;
/// Where the dividers sit on a window nobody has dragged one in: a column takes
/// this much of the width, and a top space this much of the height.
/// How far either side of the gap between two panes the pointer still counts as
/// being on the divider between them.
///
/// The gap is [`GAP`], six pixels, which is a line you can see and not a target
/// you can hit. This takes the target to fourteen without widening anything that
/// is drawn.
pub(crate) const GRAB: f32 = 4.0;
/// How far either side of a grid line a drop counts as being between the two
/// cells rather than inside one of them.
///
/// Wider than [`GRAB`], which is a target for a pointer that can see the line it
/// is aiming at. This one is aimed at with a tab in the air over the line, so it
/// is worth more room. Cut down on a grid whose cells are shorter than the band
/// is deep, so it can never swallow a whole cell and leave no way to drop inside
/// one.
const SPAN_BAND: f32 = 16.0;
/// The least height a space can be dragged down to: its tab strip ([`TAB_H`]),
/// the [`PAD`] above and below its content, and the shortest gauge block that
/// still reads as a block ([`DOT_ROWS`] rows of [`SMALL_DOT`]). Fifty-six pixels.
const MIN_SPACE_H: f32 = TAB_H
    + PAD * 2.0
    + crate::widgets::gauges::DOT_ROWS as f32 * crate::widgets::gauges::SMALL_DOT;

/// The least room the panes keep, whatever the prompt was set to: one space at
/// [`MIN_SPACE_H`] and the [`GAP`] the body is inset by on both sides. Sixty
/// eight pixels.
///
/// The same floor a drag already refuses to go under, applied to the other thing
/// that can take the room: `prompt_rows` is a fixed height now, so a big number
/// on a short window would push the conversation off the bottom of it.
const PANES_FLOOR: f32 = MIN_SPACE_H + GAP * 2.0;





/// How wide the mark down the left of the selected row is. A tab's accent runs
/// along its top edge because a strip is read left to right; a row is entered
/// from the left, so its accent runs down that edge instead.
pub(crate) const MARK_W: f32 = 2.0;

/// How wide the caret standing in the gap a dragged tab would drop into is.
///
/// Three, one more than every hairline in the window, because it is the one mark
/// that has to be read while something else is moving.
pub(crate) const CARET_W: f32 = 3.0;

/// The version this build was cut from, and the version the title strip reads.
///
/// It comes from the crate rather than from a string typed into the strip, so
/// the window cannot claim a release the package does not carry. The two cargo
/// workspaces, the CLI and this window, set the same number and ship as one
/// release.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");


/// Something the pointer can land on. Returned by [`Layout::hit`] so every
/// click is resolved in one place instead of in a chain of `if` in the event
/// handler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hit {
    TitleBar,
    Minimize,
    Maximize,
    Close,
    /// The window itself: the room below the title strip that no pane, no
    /// divider and no prompt claimed. All of it once every widget is closed.
    ///
    /// A hit of its own rather than nothing, so a right click there has
    /// something to open a menu for. Under a takeover the picker and the
    /// settings panel answer for their own emptiness before this is reached.
    Window,
    /// A view's tab, in the space it currently lives in.
    Tab(View, Space),
    /// The two arrows a strip with more tabs than room grows, at its right end.
    /// One step along the strip each. In the strip beside the tabs rather than
    /// on the floating layer, so they are hit tested with them.
    TabsLeft(Space),
    TabsRight(Space),
    /// The body of a space: where a dragged tab lands.
    Body(Space),
    /// The scroll track down a pane's right edge. Pressed and dragged, the
    /// way a divider is: the pane follows the pointer down the track. The
    /// shell falls back to a body press when the pane has no bar, so the
    /// gutter of a short pane still starts a selection.
    Scrollbar(Space),
    /// A row of the file view's explorer list, and the space it is showing in.
    /// The space is carried so a view dropped on the list still lands somewhere:
    /// a drop target is a place on screen, not a widget.
    File(usize, Space),
    Input,
    /// A row of the open menu, by position in it. The overlay is hit tested
    /// before anything else, so a menu takes the click that lands on it rather
    /// than letting it through to the pane it opened over.
    MenuRow(usize),
    /// The open menu's box, away from any row. Swallowed for the same reason:
    /// a press on its margin must not reach what is behind it.
    Menu,
    /// The box of the activity popup. Its close mark and its scroll track
    /// answer for themselves; the rest of the box swallows the press, which
    /// is what lets a press anywhere else close the popup without also doing
    /// whatever it landed on. The same bargain the menu makes.
    CallPopup,
    /// The cross at the popup's top right, the same close the settings panel
    /// has.
    CallPopupClose,
    /// The popup's own scroll track, pressed and dragged like every other.
    CallPopupScrollbar,
    /// A row of the folder picker, by position in its list.
    PickerRow(usize),
    /// The mark in front of a folder on that row, which puts what is inside it
    /// into the list under it. Its own region inside the row and tested before
    /// it: pressing the mark opens the folder, pressing the row selects it, and
    /// one region for both would make every press do both things.
    PickerMark(usize),
    /// The button at the right of the picker's head, which confirms the row the
    /// cursor is on: a folder while the folders are showing, a session while the
    /// sessions are. How the mouse chooses without a keyboard.
    PickerOpen,
    /// The two buttons at the left of that head, which say which list is
    /// showing and put the other one there. Neither is a toggle: pressing the
    /// one already lit does nothing.
    PickerFolders,
    PickerSessions,
    /// The picker's box, away from any row. Swallowed, so a press on its margin
    /// does not read as a press on the window behind it.
    Picker,
    /// A section on the settings panel's rail, by position in it. Choosing one
    /// swaps what is beside the rail.
    SettingsSection(usize),
    /// A row of the settings panel, by position in its list and which half of it
    /// was pressed. Puts the cursor there and nothing else: a click anywhere on
    /// a row that also changed the setting would change one every time the
    /// pointer missed the value.
    ///
    /// The half is [`crate::settings::Side::Left`] for every row that is not a
    /// form, which is all of them outside the AGENT section.
    SettingsRow(usize, Side),
    /// The value at the end of that row, which is the control. Clicking it is
    /// the same nudge the right arrow is, or the start of an edit on a field.
    SettingsValue(usize, Side),
    /// The track at the end of a row whose setting has a range. Pressed and
    /// dragged, the way a divider is: what it means is where the pointer is,
    /// and it is written when the button comes up.
    SettingsSlider(usize, Side),
    /// One cell of the palette grid, as the row its card is on and the colour
    /// along it. A card of colours is several controls wide, so the row on its
    /// own cannot say which colour the pointer is over.
    SettingsSwatch(usize, usize),
    /// One option of a choice, as the row its card is on, which field of that
    /// card it is, and which option along it. Pressing one writes that option:
    /// the options are all drawn, so all of them can be aimed at, and the
    /// keyboard's own way round them is still the arrow keys.
    SettingsChoice(usize, Side, usize),
    /// The toggle on an entry row, by the row it is on. Pressing it turns that
    /// skill or that server on or off, which is a move on the disk: nothing in
    /// this window remembers a flag for either.
    SettingsToggle(usize),
    /// The uninstall beside that toggle, on the rows that have one. Its own
    /// region and tested before the row, the way the toggle is: one region for
    /// the row and the button would delete a skill or a server every time
    /// somebody pressed the row to read it.
    SettingsRemove(usize),
    /// One conversation on the table, as the row the table is on and the row of
    /// the table itself. Puts the keys on it, the way a press on any other row
    /// of the panel does.
    SettingsPick(usize, usize),
    /// The mark in front of that conversation. Its own region and tested before
    /// the row, the way the picker's own mark is: one region for both would make
    /// every press that picks a row also mark it.
    SettingsMark(usize, usize),
    /// One of the buttons in the table's footer, by the row the table is on.
    SettingsAct(usize, Act),
    /// The line between the rail of section names and the settings beside it.
    /// Dragging it decides how much of the panel each of the two takes.
    ///
    /// Its own hit rather than a [`Hit::ColumnDivider`]: the panel is a takeover,
    /// so while it is up there are no panes and no grid for a column divider to
    /// mean anything about.
    SettingsRailDivider,
    /// The text of the document beside the entry list. Its own region so a
    /// press there can start a selection: everywhere else on the panel a press
    /// is a control, and the one thing to do with a page of prose is to take
    /// some of it away with you.
    ///
    /// The text box inside the wrapper rather than the whole column, so the
    /// title over it and the border around it are still panel and still
    /// swallow their press.
    SettingsDoc,
    /// The mark that closes the panel, for a pointer with no Escape key handy.
    SettingsClose,
    /// The panel's box, away from any row. Swallowed, like the picker's.
    Settings,
    /// The band between the left column and the right one, and the row it runs
    /// in. Dragging it decides how much of that row's width each column gets.
    ColumnDivider(usize),
    /// The band between a column's two spaces, and the column it runs in, which
    /// decides how that column's height is shared between them.
    RowDivider(usize),
}

impl Hit {
    /// The space a drop here would move a view into.
    pub fn space(self) -> Option<Space> {
        match self {
            Hit::Tab(_, space)
            | Hit::TabsLeft(space)
            | Hit::TabsRight(space)
            | Hit::Body(space)
            | Hit::Scrollbar(space)
            | Hit::File(_, space) => Some(space),
            _ => None,
        }
    }
}

/// Where a dragged tab ends up when the button comes up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    /// Into a space, which moves the view there. The number is the place it
    /// takes among that space's tabs, counted against the tabs the space holds
    /// now, and is only there when the drop was on the tab strip: dropping onto
    /// a strip says where in the order the tab goes, dropping into the pane
    /// below says which space and nothing more. Without one, a drop back into
    /// the space the view is already in is the no-op it always was.
    In(Space, Option<usize>),
    /// Onto the line between two cells, which gives the pane both of them: the
    /// two are merged into one and it spans the pair. The two always share a
    /// divider, never a corner.
    Span(Space, Space),
    /// Off the window entirely, which takes the view out of it.
    Out,
    /// Somewhere in the window that is not a space: the title strip, the
    /// prompt, the margin around the panes. Nothing happens.
    Nowhere,
}

impl Landing {
    /// A pair, with the two cells in grid order.
    ///
    /// So the same pair is one answer rather than two: a drop a pixel above the
    /// line and one a pixel below it name the same pair, and nothing downstream
    /// has to know that `(a, b)` and `(b, a)` mean the same drop.
    pub fn span(a: Space, b: Space) -> Landing {
        match a.index() < b.index() {
            true => Landing::Span(a, b),
            false => Landing::Span(b, a),
        }
    }
}

/// Where one space is, and where its tabs are.
pub struct Placed {
    pub strip: Panel,
    pub body: Panel,
    /// The tabs on screen, left to right, each with the view it names. Only the
    /// ones that fit: a strip too narrow for all of them shows a window of them
    /// starting at [`Placed::first_tab`], and grows two arrows to walk it.
    pub tabs: Vec<(View, Panel)>,
    /// Which of the space's tabs `tabs` starts at. Zero unless the strip is
    /// scrolled, and never past the last tab.
    pub first_tab: usize,
    /// The two arrows, in that order along the strip. Both empty when every tab
    /// fits, which is when the strip loses no room to them.
    pub arrow_left: Panel,
    pub arrow_right: Panel,
}

/// A divider between two spaces, and everything a drag of it needs.
///
/// Not a thing that is drawn: what the eye reads as the divider is the [`GAP`]
/// between the two panes, and this is the target around it. So the window has
/// exactly one line between two panes, and it is the one that was always there.
#[derive(Clone, Copy, Debug)]
pub struct Divider {
    /// The band the pointer can grab it by, [`GRAB`] wider than the gap on each
    /// side.
    pub band: Panel,
    /// The box it slides in. The ratio is a fraction of this, gap included, and
    /// it is what turns a pointer position back into a ratio.
    pub track: Panel,
    /// The least room it will leave on either side of itself.
    pub floor: f32,
}

impl Divider {
    pub(crate) fn none() -> Divider {
        Divider {
            band: nowhere(),
            track: nowhere(),
            floor: 0.0,
        }
    }

    /// Whether there is anything here to drag. A divider beside an empty space
    /// is not there at all: the empty space gave its room away, so there are no
    /// longer two things for it to be between.
    pub(crate) fn live(self) -> bool {
        self.band.w >= 1.0 && self.band.h >= 1.0
    }
}

/// Where everything is this frame. Built from the window size and the dock, so
/// nothing else has to recompute it.
pub struct Layout {
    pub width: f32,
    pub height: f32,
    pub shaded: bool,

    pub title: Panel,
    pub minimize: Panel,
    pub maximize: Panel,
    pub close: Panel,

    /// One per [`Space`], in `Space::ALL` order. A space with no tabs gets an
    /// empty one, and a space covering two cells gets the pair's box.
    pub spaces: [Placed; 4],
    /// The four cells of the grid, in `Space::ALL` order, from the two ratios
    /// alone.
    ///
    /// Where the dividers are, rather than what is drawn: a cell standing empty
    /// still has its box here, because that is the box a drop into it would
    /// take. Empty in every shape that has no panes.
    pub grid: [Panel; 4],
    /// Whether the grid reads as two rows or as two columns, which is what says
    /// which of the two lines below runs the whole way across it. Read off the
    /// dock, and false in every shape that has no panes.
    pub rows_first: bool,
    /// The dividers, one per half of the grid: the vertical lines by the row
    /// they run in, the horizontal ones by the column.
    ///
    /// Only one axis has two live lines. The grid is cut in half by a single
    /// line first, and each half is then cut by a line of its own, so a grid
    /// reading in columns has one vertical divider (`column_divider[0]`, the
    /// whole way down) and two horizontal ones, and a grid reading in rows has
    /// it the other way round. The unused second entry is empty, as is every one
    /// of them in a shape with no panes and beside a space standing empty or
    /// folded away.
    pub column_divider: [Divider; 2],
    pub row_divider: [Divider; 2],
    /// The file view's explorer column, down the left of its pane. Zero sized
    /// when the file view is not showing, or when nothing has been touched yet.
    pub file_list: Panel,
    /// The rest of that pane, where the open file is drawn.
    pub file_diff: Panel,
    /// One panel per visible row of the explorer, with the file it names. Only
    /// the rows that fit: the list scrolls rather than squeezing.
    pub file_rows: Vec<(usize, Panel)>,
    pub files_in: Option<Space>,
    pub input: Panel,

    /// True while the folder picker is up, which is a shape of its own: there is
    /// no arrangement of panes and no prompt, because there is no agent yet.
    pub picking: bool,
    /// Its box, its list, one panel per visible row of that list, the mark that
    /// opens and shuts each row that is a folder, and the button that confirms.
    /// All empty when it is not up.
    pub picker: Panel,
    pub picker_list: Panel,
    pub picker_rows: Vec<(usize, Panel)>,
    pub picker_marks: Vec<(usize, Panel)>,
    pub picker_open: Panel,
    /// The bordered field what has been typed goes in, above the list. A panel
    /// rather than a rectangle worked out where it is drawn, so the border and
    /// the writing inside it come off one shape.
    pub picker_filter: Panel,
    /// The two buttons at the left of the head that choose the list. Both empty
    /// when the box is too narrow to hold them and the Open button, which is the
    /// only reason a button in this box ever goes away.
    pub picker_folders: Panel,
    pub picker_sessions: Panel,

    /// True while the settings panel is up, which is the third shape of its own:
    /// the panes and the prompt are still there behind it, and the panel covers
    /// the lot, because it is a takeover rather than a window over a window.
    pub in_settings: bool,
    /// Its box, the rail of section names down its left, the chosen section's
    /// list, one panel per visible row of that list, the control at the end of
    /// each of those rows, and the mark that closes it. All empty when it is
    /// not up.
    ///
    /// A row's control is either a value (a flag, a preset, the endpoint) or a
    /// track (anything with a range), never both: what a press means has to be
    /// one thing.
    ///
    /// A row of the palette grid carries neither and is split into cells
    /// instead, one per colour on it, each one carrying its row and its place
    /// along that row.
    pub settings: Panel,
    /// A box per section name, every one of them, wrapped into as many columns
    /// of the rail as the window's height needs (see [`settings_rail_cells`]).
    pub settings_rail: Vec<(usize, Panel)>,
    /// The line between the rail and the list, and the band it is grabbed by.
    /// Empty in every shape but the panel's, the way a pane divider is empty in
    /// every shape with no panes.
    pub settings_rail_divider: Divider,
    pub settings_list: Panel,
    pub settings_rows: Vec<(usize, Side, Panel)>,
    pub settings_values: Vec<(usize, Side, Panel)>,
    pub settings_tracks: Vec<(usize, Side, Panel)>,
    pub settings_cells: Vec<(usize, usize, Panel)>,
    /// One box per option of a choice: the row its card is on, which field of
    /// that card it is, and its place along the options. Every option is drawn,
    /// so every option is a press.
    pub settings_choices: Vec<(usize, Side, usize, Panel)>,
    /// The two controls an entry row carries: the toggle that turns it on and
    /// off, and the uninstall on the rows that have one. Empty on every section
    /// that lists no entries, which is every section but the skills and the
    /// servers.
    pub settings_toggles: Vec<(usize, Panel)>,
    pub settings_removes: Vec<(usize, Panel)>,
    /// The rows of the saved-conversations table that are inside its body right
    /// now, each one carrying the panel row the table is on and its own place on
    /// the list; the mark in front of each of them; and the buttons in the
    /// table's footer. All three empty in every section but that one.
    pub settings_picks: Vec<(usize, usize, Panel)>,
    pub settings_marks: Vec<(usize, usize, Panel)>,
    pub settings_acts: Vec<(usize, Act, Panel)>,
    /// The column beside that list, where the entry under the cursor is shown:
    /// a skill's own `SKILL.md`, or a server's entry out of its file. Empty in
    /// every section that has no entries, which is what leaves those sections
    /// one column wide.
    pub settings_doc: Panel,
    /// The box the document's own text is written in, inside the outlined
    /// wrapper and under the title over it. Empty wherever `settings_doc` is.
    pub settings_doc_text: Panel,
    pub settings_close: Panel,

    /// The floating layer. The open menu's box, and one panel per row on
    /// screen, both empty when no menu is open. Drawn last and hit tested
    /// first.
    ///
    /// Each row carries its place in the menu, the way the picker's and the
    /// settings panel's rows do, because the menu scrolls: the third panel down
    /// is not always the third row.
    ///
    /// Two boxes at most: the menu's own column, and the widgets flyout
    /// beside its header while it is open. The flyout's rows carry their
    /// global place in the same menu, so a hit is one number either way.
    pub menu: Panel,
    pub menu_rows: Vec<(usize, Panel)>,
    pub menu_fly: Panel,
    pub menu_fly_rows: Vec<(usize, Panel)>,
    /// The one call the activity list was clicked into, on the same floating
    /// layer as the menu and under it. Empty when nothing is open, and empty
    /// under every takeover: the panel and the picker cover the pane it was
    /// opened from, so a popup left floating over either would be a box about
    /// something nobody can see.
    ///
    /// One box and no rows: nothing inside it can be clicked, so it needs no
    /// region beyond its own. It swallows the press that lands on it, the way
    /// the menu's margin does.
    pub call_popup: Panel,
    /// The popup's close mark, when the popup is up.
    pub call_popup_close: Panel,
}

/// What the layout needs beyond the window size.
pub struct Shape<'a> {
    pub shaded: bool,
    pub dock: &'a Dock,
    /// The open menu, if there is one. Part of the shape because the overlay is
    /// hit tested off the same layout the rest of the window is, which is the
    /// only way a click on a menu row and the row it looks like it landed on
    /// can never come apart.
    pub menu: Option<&'a Menu>,
    /// The folder picker, while it is up. Part of the shape because the picker
    /// replaces the whole window: with it up there are no panes and no prompt.
    pub picker: Option<&'a Picker>,
    /// The settings panel, while it is up, for the same reason: it covers the
    /// panes, so where its rows are is where every hit region in the window is.
    pub settings: Option<&'a Settings>,
    /// One label per file in the explorer, in order.
    pub file_labels: Vec<String>,
    /// Which row the explorer list starts on, counted from its top. The layout
    /// turns it into the rows that are actually on screen, so the drawing and
    /// the hit testing read one answer.
    pub file_first: usize,
    /// The agent the output tab is on, by ordinal, so the tab is measured at
    /// the width of the `[N] AGENT - OUTPUT` label it will be drawn with.
    pub agent_tab: Option<usize>,
    pub column: f32,
    /// The width of one column at [`MENU_SIZE`], which is the size a menu's
    /// rows are written at. Its own number rather than `column`, because the
    /// menu is the one surface in the window not written at the title bar's
    /// size: measured in one and drawn in the other, every row came out short
    /// of its own box and the group chevron floated away from its label.
    pub menu_column: f32,
    /// The size and column width the panes are drawn at. The explorer's rows are
    /// pane text, so their height and the width of the column they sit in are
    /// the pane's, not the title bar's.
    pub pane_size: f32,
    pub pane_column: f32,
    /// How tall the prompt is. It grows with what has been typed, so it is an
    /// input to the layout rather than a constant.
    pub input_h: f32,
    /// How much of the body's width the left column takes, in each row.
    pub left_width: [f32; 2],
    /// How much of the body's height the top space takes, in each column.
    ///
    /// A pair each, because only one line runs the whole way across the grid.
    /// Reading in columns, `left_width[0]` is that line and each column breaks at
    /// its own `top_height`; reading in rows it is the other way round. The pair
    /// on the axis with the single line keeps its second number rather than
    /// dropping it, so turning the grid round and back finds both lines where
    /// they were left.
    ///
    /// All of them are inputs for the same reason `input_h` is: they are dragged
    /// while the window is up and read back out of the settings file at the next
    /// launch. Held here rather than trusted: [`held`] keeps whatever arrives
    /// inside what the window can actually draw, so neither a file with a silly
    /// number in it nor a drag thrown past the edge can collapse a space.
    pub top_height: [f32; 2],
    /// How much of the settings panel's width the rail of section names takes.
    /// An input for the same reason the pane ratios are: it is dragged while the
    /// panel is up and read back out of the settings file at the next launch.
    pub settings_rail: f32,
    /// The call the activity popup is showing, while it is up. Part of the shape
    /// for the reason the menu is: how tall the box is comes from what is in it,
    /// so the layout has to see the call to know where its edges are.
    pub popup: Option<&'a crate::state::Call>,
}





impl Layout {
    pub fn compute(width: f32, height: f32, shape: &Shape) -> Layout {
        let whole = Panel::new(0.0, 0.0, width, height);
        let (title, rest) = whole.split_top(TITLE_H.min(height));
        // As tall as the strip turned out to be, not as tall as a strip usually
        // is. A button box that reaches past the surface is a button whose mark
        // is written where nothing is drawn, and a hit region over a row of
        // pixels the window does not have.
        let buttons = [
            Panel::new(width - BUTTON_W * 3.0, title.y, BUTTON_W, title.h),
            Panel::new(width - BUTTON_W * 2.0, title.y, BUTTON_W, title.h),
            Panel::new(width - BUTTON_W, title.y, BUTTON_W, title.h),
        ];
        // Placed before the shape is decided, because the overlay is above the
        // window in both shapes: a menu that survived a double click on the
        // title bar would still be hit tested and would have nothing drawn.
        let places = match shape.menu {
            Some(menu) => place_menu(menu, shape.menu_column, width, height),
            None => MenuPlaces {
                box_: nowhere(),
                rows: Vec::new(),
                fly: nowhere(),
                fly_rows: Vec::new(),
            },
        };
        let (menu, menu_rows) = (places.box_, places.rows);
        let (menu_fly, menu_fly_rows) = (places.fly, places.fly_rows);
        // Only in the shape that has panes. The three takeovers below collapse
        // it along with every other pane region.
        let (call_popup, call_popup_close) = match shape.popup {
            Some(_) => {
                let box_ = place_popup(width, height);
                let mark = Text::line_for(shape.pane_size);
                (
                    box_,
                    Panel::new(
                        box_.x + box_.w - mark - crate::widgets::popup::POPUP_PAD,
                        box_.y + crate::widgets::popup::POPUP_PAD,
                        mark,
                        mark,
                    ),
                )
            }
            None => (nowhere(), nowhere()),
        };

        if shape.shaded {
            // One strip and nothing else. Every other region collapses to
            // nothing so a stale hit region cannot survive the shape change.
            return Layout {
                width,
                height,
                shaded: true,
                title,
                minimize: buttons[0],
                maximize: buttons[1],
                close: buttons[2],
                spaces: [
                    empty_placed(),
                    empty_placed(),
                    empty_placed(),
                    empty_placed(),
                ],
                grid: [nowhere(), nowhere(), nowhere(), nowhere()],
                rows_first: false,
                column_divider: [Divider::none(); 2],
                row_divider: [Divider::none(); 2],
                file_list: nowhere(),
                file_diff: nowhere(),
                file_rows: Vec::new(),
                files_in: None,
                input: nowhere(),
                picking: false,
                picker: nowhere(),
                picker_list: nowhere(),
                picker_rows: Vec::new(),
                picker_marks: Vec::new(),
                picker_open: nowhere(),
                picker_filter: nowhere(),
                picker_folders: nowhere(),
                picker_sessions: nowhere(),
                in_settings: false,
                settings: nowhere(),
                settings_rail: Vec::new(),
                settings_rail_divider: Divider::none(),
                settings_list: nowhere(),
                settings_rows: Vec::new(),
                settings_values: Vec::new(),
                settings_tracks: Vec::new(),
                settings_cells: Vec::new(),
                settings_choices: Vec::new(),
                settings_toggles: Vec::new(),
                settings_removes: Vec::new(),
                settings_picks: Vec::new(),
                settings_marks: Vec::new(),
                settings_acts: Vec::new(),
                settings_doc: nowhere(),
                settings_doc_text: nowhere(),
                settings_close: nowhere(),
                menu,
                menu_rows,
                menu_fly,
                menu_fly_rows,
                call_popup: nowhere(),
                call_popup_close: nowhere(),
            };
        }

        // The picker is the whole window while it is up, and every other region
        // collapses to nothing the way it does when the window is shaded: a
        // stale hit region left behind here would let a click reach a pane that
        // has no agent behind it.
        if let Some(picker) = shape.picker {
            let places = crate::picker::places::place_picker(rest.inset(GAP), shape, picker);
            return Layout {
                width,
                height,
                shaded: false,
                title,
                minimize: buttons[0],
                maximize: buttons[1],
                close: buttons[2],
                spaces: [
                    empty_placed(),
                    empty_placed(),
                    empty_placed(),
                    empty_placed(),
                ],
                grid: [nowhere(), nowhere(), nowhere(), nowhere()],
                rows_first: false,
                column_divider: [Divider::none(); 2],
                row_divider: [Divider::none(); 2],
                file_list: nowhere(),
                file_diff: nowhere(),
                file_rows: Vec::new(),
                files_in: None,
                input: nowhere(),
                picking: true,
                picker: places.box_,
                picker_list: places.list,
                picker_rows: places.rows,
                picker_marks: places.marks,
                picker_open: places.open,
                picker_filter: places.filter,
                picker_folders: places.folders,
                picker_sessions: places.sessions,
                in_settings: false,
                settings: nowhere(),
                settings_rail: Vec::new(),
                settings_rail_divider: Divider::none(),
                settings_list: nowhere(),
                settings_rows: Vec::new(),
                settings_values: Vec::new(),
                settings_tracks: Vec::new(),
                settings_cells: Vec::new(),
                settings_choices: Vec::new(),
                settings_toggles: Vec::new(),
                settings_removes: Vec::new(),
                settings_picks: Vec::new(),
                settings_marks: Vec::new(),
                settings_acts: Vec::new(),
                settings_doc: nowhere(),
                settings_doc_text: nowhere(),
                settings_close: nowhere(),
                menu,
                menu_rows,
                menu_fly,
                menu_fly_rows,
                call_popup: nowhere(),
                call_popup_close: nowhere(),
            };
        }

        // The settings panel takes the whole surface under the title strip, and
        // every pane region collapses the way it does for the picker. A takeover
        // rather than a box over the panes: half a window of live panes behind a
        // list of settings is two scroll regions over each other, and a click
        // that missed the panel would land in a transcript nobody can see.
        if let Some(panel) = shape.settings {
            let places = crate::settings::places::place_settings(rest.inset(GAP), shape, panel);
            return Layout {
                width,
                height,
                shaded: false,
                title,
                minimize: buttons[0],
                maximize: buttons[1],
                close: buttons[2],
                spaces: [
                    empty_placed(),
                    empty_placed(),
                    empty_placed(),
                    empty_placed(),
                ],
                grid: [nowhere(), nowhere(), nowhere(), nowhere()],
                rows_first: false,
                column_divider: [Divider::none(); 2],
                row_divider: [Divider::none(); 2],
                file_list: nowhere(),
                file_diff: nowhere(),
                file_rows: Vec::new(),
                files_in: None,
                input: nowhere(),
                picking: false,
                picker: nowhere(),
                picker_list: nowhere(),
                picker_rows: Vec::new(),
                picker_marks: Vec::new(),
                picker_open: nowhere(),
                picker_filter: nowhere(),
                picker_folders: nowhere(),
                picker_sessions: nowhere(),
                in_settings: true,
                settings: places.box_,
                settings_rail: places.rail,
                settings_rail_divider: places.divider,
                settings_list: places.list,
                settings_rows: places.rows,
                settings_values: places.values,
                settings_tracks: places.tracks,
                settings_cells: places.cells,
                settings_choices: places.choices,
                settings_toggles: places.toggles,
                settings_removes: places.removes,
                settings_picks: places.picks,
                settings_marks: places.marks,
                settings_acts: places.acts,
                settings_doc: places.doc,
                settings_doc_text: places.doc_text,
                settings_close: places.close,
                menu,
                menu_rows,
                menu_fly,
                menu_fly_rows,
                call_popup: nowhere(),
                call_popup_close: nowhere(),
            };
        }

        // The prompt is the height it was set to, and the conversation is what
        // the window is for: a row count that would leave the panes under
        // [`PANES_FLOOR`] takes what is left over instead of the last of it, so
        // the strip gives room back rather than squeezing them away. Whenever
        // the room is there the setting is honoured to the pixel. Under
        // `PANES_FLOOR + INPUT_H` the window is too short for both and the strip
        // keeps its own one-row height, which is the least it can be read in.
        let room = (rest.h - PANES_FLOOR).max(INPUT_H.min(rest.h));
        let (body, input) = rest.split_bottom(shape.input_h.max(INPUT_H).min(room));
        let body = body.inset(GAP);

        // The grid is the two ratios and nothing else: four cells, whether or
        // not anything is standing in them, because that is what a drop is read
        // off. What is drawn comes next.
        let column_floor = min_column_w(shape.pane_column);
        let rows_first = shape.dock.rows_first();
        let grid = grid_cells(
            body,
            rows_first,
            shape.left_width,
            shape.top_height,
            column_floor,
        );

        // An empty space gives its room away rather than leaving a hole: the
        // dock says which pane covers which cell, and a pane's box is the box
        // around the cells it covers.
        let cover = shape.dock.cover();
        let mut areas = [nowhere(); 4];
        for cell in Space::ALL {
            if let Some(head) = cover[cell.index()] {
                areas[head.index()] = around(areas[head.index()], grid[cell.index()]);
            }
        }

        let folded = |space: Space| shape.dock.slot(space).folded;
        let live = |space: Space| !shape.dock.slot(space).is_empty();
        // The two cells of one half of the grid: a column apiece when the grid
        // splits into columns first, a row apiece when it splits into rows.
        let cell = |major: usize, minor: usize| match rows_first {
            true => Space::at(major, minor),
            false => Space::at(minor, major),
        };
        // A half is split when both of its cells are standing in, which is also
        // when the divider inside it is there at all.
        let split = |major: usize| live(cell(major, 0)) && live(cell(major, 1));

        // A folded space is already as short as it goes and its neighbour takes
        // the rest of the column. Only down a column: a tab strip runs across
        // the top of a pane, so folding collapses it downwards, and the pane
        // that can take the room is the one under it. Folded beside another
        // pane, it keeps its own cell and leaves the rest of it empty.
        if !rows_first {
            for column in 0..2 {
                let (top, bottom) = (Space::at(0, column), Space::at(1, column));
                if !(live(top) && live(bottom)) {
                    continue;
                }
                let whole = around(areas[top.index()], areas[bottom.index()]);
                let (top_h, bottom_h) = match (folded(top), folded(bottom)) {
                    (true, false) => {
                        let taken = TAB_H.min(whole.h);
                        (taken, whole.h - taken - GAP)
                    }
                    (false, true) => {
                        let taken = TAB_H.min(whole.h);
                        (whole.h - taken - GAP, taken)
                    }
                    _ => continue,
                };
                areas[top.index()] = Panel::new(whole.x, whole.y, whole.w, top_h);
                areas[bottom.index()] =
                    Panel::new(whole.x, whole.y + top_h + GAP, whole.w, bottom_h);
            }
        }

        // The line between the two halves, and the line inside them. Which of
        // the two is the column divider and which the row divider is the same
        // transpose the cells are read with.
        let across = Divider {
            band: match rows_first {
                true => Panel::new(body.x, grid[0].y + grid[0].h - GRAB, body.w, GRAB * 2.0 + GAP),
                false => Panel::new(grid[0].x + grid[0].w - GRAB, body.y, GRAB * 2.0 + GAP, body.h),
            },
            track: body,
            floor: match rows_first {
                true => MIN_SPACE_H,
                false => column_floor,
            },
        };
        // Inside a half the line is that half's own: it is there when the half
        // is split and only as long as the two panes it divides. A half whose
        // split came from a fold has no line to drag, because the fold owns
        // where it sits until it is opened again, and a half that is not split
        // has nothing to divide. Either way the other half keeps its line and
        // keeps where it was dragged to.
        let inside = |major: usize| -> Divider {
            let dragged =
                split(major) && (rows_first || !(folded(cell(major, 0)) || folded(cell(major, 1))));
            // The panes on either side of it, not the cells: a half whose
            // opposite number is empty has taken that room, and the line
            // between its two panes runs the whole way across with them.
            let reach = around(areas[cell(major, 0).index()], areas[cell(major, 1).index()]);
            if !dragged || reach.w < 1.0 || reach.h < 1.0 {
                return Divider::none();
            }
            // Where the line sits is the half's own first cell, not the grid's:
            // that is the whole point of a ratio per half.
            let head = grid[cell(major, 0).index()];
            Divider {
                band: match rows_first {
                    true => Panel::new(head.x + head.w - GRAB, reach.y, GRAB * 2.0 + GAP, reach.h),
                    false => Panel::new(reach.x, head.y + head.h - GRAB, reach.w, GRAB * 2.0 + GAP),
                },
                track: body,
                floor: match rows_first {
                    true => column_floor,
                    false => MIN_SPACE_H,
                },
            }
        };
        // The line between the halves is there only when both halves have
        // something in them, or there is nothing left for it to divide.
        let halves = (live(cell(0, 0)) || live(cell(0, 1))) && (live(cell(1, 0)) || live(cell(1, 1)));
        let across = match halves {
            true => across,
            false => Divider::none(),
        };
        // Filed by the axis each line runs on rather than by the part it plays,
        // so what a press means never depends on which way the grid is reading.
        let mut column_divider = [Divider::none(); 2];
        let mut row_divider = [Divider::none(); 2];
        match rows_first {
            true => {
                row_divider[0] = across;
                column_divider = [inside(0), inside(1)];
            }
            false => {
                column_divider[0] = across;
                row_divider = [inside(0), inside(1)];
            }
        }

        let place = |space: Space, area: Panel| -> Placed {
            if area.w < 1.0 || area.h < 1.0 {
                return empty_placed();
            }
            let (strip, rest) = area.split_top(TAB_H.min(area.h));
            let slot = shape.dock.slot(space);
            let widths: Vec<usize> = slot
                .views
                .iter()
                .map(|v| tab_label(*v, shape.agent_tab).chars().count())
                .collect();
            let laid = strip_tabs(
                strip,
                &widths,
                shape.column,
                slot.tab_first(),
                slot.active_index(),
            );
            let tabs = laid
                .tabs
                .into_iter()
                .enumerate()
                // Counted from the tab the strip starts at, not from the first
                // tab of the space. Zipping the panels with the views by
                // position alone is what would label every tab of a scrolled
                // strip with the wrong view.
                .map(|(i, panel)| (slot.views[laid.first + i], panel))
                .collect();
            Placed {
                strip,
                body: if slot.folded {
                    Panel::new(rest.x, rest.y, rest.w, 0.0)
                } else {
                    rest
                },
                tabs,
                first_tab: laid.first,
                arrow_left: laid.left,
                arrow_right: laid.right,
            }
        };

        let spaces = [
            place(Space::TopLeft, areas[0]),
            place(Space::BottomLeft, areas[1]),
            place(Space::TopRight, areas[2]),
            place(Space::BottomRight, areas[3]),
        ];

        // The file view's explorer runs down the left of whichever space is
        // showing it, with the open file in the room that is left.
        let files_in = shape.dock.space_of(View::Files).filter(|space| {
            shape.dock.slot(*space).active() == Some(View::Files)
                && !shape.dock.slot(*space).folded
        });
        let (file_list, file_diff, file_rows) = match files_in {
            Some(space) => place_files(spaces[space.index()].body, shape),
            None => (nowhere(), nowhere(), Vec::new()),
        };

        Layout {
            width,
            height,
            shaded: false,
            title,
            minimize: buttons[0],
            maximize: buttons[1],
            close: buttons[2],
            spaces,
            grid,
            rows_first,
            column_divider,
            row_divider,
            file_list,
            file_diff,
            file_rows,
            files_in,
            input: input.inset(GAP),
            picking: false,
            picker: nowhere(),
            picker_list: nowhere(),
            picker_rows: Vec::new(),
            picker_marks: Vec::new(),
            picker_open: nowhere(),
            picker_filter: nowhere(),
            picker_folders: nowhere(),
            picker_sessions: nowhere(),
            in_settings: false,
            settings: nowhere(),
            settings_rail: Vec::new(),
            settings_rail_divider: Divider::none(),
            settings_list: nowhere(),
            settings_rows: Vec::new(),
            settings_values: Vec::new(),
            settings_tracks: Vec::new(),
            settings_cells: Vec::new(),
            settings_choices: Vec::new(),
            settings_toggles: Vec::new(),
            settings_removes: Vec::new(),
            settings_picks: Vec::new(),
            settings_marks: Vec::new(),
            settings_acts: Vec::new(),
            settings_doc: nowhere(),
            settings_doc_text: nowhere(),
            settings_close: nowhere(),
            menu,
            menu_rows,
            menu_fly,
            menu_fly_rows,
            call_popup,
            call_popup_close,
        }
    }

    pub fn placed(&self, space: Space) -> &Placed {
        &self.spaces[space.index()]
    }

    /// What is under a point. One place, so a click and the thing it appears to
    /// land on can never come apart.
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        // The floating layer first. A menu is above the window, so it takes the
        // click even when a window button, a tab or a pane is under it; without
        // this the menu would be drawn over things it could be clicked through
        // onto, which is worse than having no menu.
        //
        // Everything but the notch in its top right corner. Those pixels are
        // cut out of the fill and out of the border, so the pane behind them is
        // what is on screen there and the pane behind them is what a press
        // reaches. Asked once, on the box, rather than per row: a row is as wide
        // as the box, so the box's notch is the first row's notch.
        if self.menu.w >= 1.0 && self.menu.contains(x, y) && !in_cut(self.menu, x, y) {
            for (index, row) in &self.menu_rows {
                if row.contains(x, y) {
                    return Some(Hit::MenuRow(*index));
                }
            }
            // The margin above the first row and below the last swallows the
            // press rather than letting it through to a pane.
            return Some(Hit::Menu);
        }
        // The widgets flyout is the same overlay: its rows answer by their
        // global place in the menu, and its box swallows what its rows do not.
        if self.menu_fly.w >= 1.0 && self.menu_fly.contains(x, y) {
            for (index, row) in &self.menu_fly_rows {
                if row.contains(x, y) {
                    return Some(Hit::MenuRow(*index));
                }
            }
            return Some(Hit::Menu);
        }
        // Under the menu on the same layer, and above everything else. A menu
        // opened over the popup is the newer thing and takes the click; the
        // popup takes it from the panes it is drawn over. Inside it, the
        // close mark and the scroll track answer before the box swallows.
        if self.call_popup.w >= 1.0 && self.call_popup.contains(x, y) {
            if self.call_popup_close.w >= 1.0 && self.call_popup_close.contains(x, y) {
                return Some(Hit::CallPopupClose);
            }
            let track = scroll_track(self.call_popup);
            let band = Panel::new(
                track.x - SCROLL_GAP * 2.0,
                track.y,
                track.w + SCROLL_GAP * 3.0,
                track.h,
            );
            if band.contains(x, y) {
                return Some(Hit::CallPopupScrollbar);
            }
            return Some(Hit::CallPopup);
        }
        for (panel, hit) in [
            (self.close, Hit::Close),
            (self.maximize, Hit::Maximize),
            (self.minimize, Hit::Minimize),
        ] {
            if panel.contains(x, y) {
                return Some(hit);
            }
        }
        if self.title.contains(x, y) {
            return Some(Hit::TitleBar);
        }
        if self.shaded {
            return None;
        }
        // Nothing else exists while the picker is up, so this answers for the
        // whole window below the title strip.
        if self.picking {
            // The three buttons in the head, above every row. Each is empty in
            // a box with no room for it, and an empty panel answers for
            // nothing rather than for the point it collapsed onto.
            if self.picker_folders.w >= 1.0 && self.picker_folders.contains(x, y) {
                return Some(Hit::PickerFolders);
            }
            if self.picker_sessions.w >= 1.0 && self.picker_sessions.contains(x, y) {
                return Some(Hit::PickerSessions);
            }
            if self.picker_open.w >= 1.0 && self.picker_open.contains(x, y) {
                return Some(Hit::PickerOpen);
            }
            // The mark before the row it sits in, because it sits inside it.
            // The other way round the mark could never be pressed.
            for (index, panel) in &self.picker_marks {
                if panel.contains(x, y) {
                    return Some(Hit::PickerMark(*index));
                }
            }
            for (index, panel) in &self.picker_rows {
                if panel.contains(x, y) {
                    return Some(Hit::PickerRow(*index));
                }
            }
            if self.picker.w >= 1.0 && self.picker.contains(x, y) {
                return Some(Hit::Picker);
            }
            return None;
        }
        // The same for the settings panel: it covers the panes, so nothing
        // underneath it can answer for a point inside it.
        if self.in_settings {
            if self.settings_close.w >= 1.0 && self.settings_close.contains(x, y) {
                return Some(Hit::SettingsClose);
            }
            // The control before the row it sits in, because it sits inside it.
            for (index, side, panel) in &self.settings_tracks {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsSlider(*index, *side));
                }
            }
            // Every option of a choice before the field they stand in: they
            // fill it, and a field that answered as one press would be the one
            // word box the options replaced.
            for (index, side, at, panel) in &self.settings_choices {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsChoice(*index, *side, *at));
                }
            }
            for (index, side, panel) in &self.settings_values {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsValue(*index, *side));
                }
            }
            // A cell before its row, for the same reason: it sits inside it, and
            // a grid row that answered as a row would make every colour on it
            // the same press.
            for (index, cell, panel) in &self.settings_cells {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsSwatch(*index, *cell));
                }
            }
            // And an entry's two controls before the row they stand in, for the
            // same reason: the uninstall deletes a skill's directory or a
            // server's entry, so it must never be a press that also reads as a
            // press on the row.
            for (index, panel) in &self.settings_removes {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsRemove(*index));
                }
            }
            for (index, panel) in &self.settings_toggles {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsToggle(*index));
                }
            }
            // The table's buttons, then the mark on a row, then the row itself,
            // all before the card they stand in: each one sits inside the last,
            // and the more particular thing is what the press means.
            for (index, act, panel) in &self.settings_acts {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsAct(*index, *act));
                }
            }
            for (index, at, panel) in &self.settings_marks {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsMark(*index, *at));
                }
            }
            for (index, at, panel) in &self.settings_picks {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsPick(*index, *at));
                }
            }
            // The line between the rail and the list, before either of them: the
            // band is wider than the gap it stands in, so it reaches a little
            // way into the rail, and the more particular thing there is to do at
            // the edge of a name is to move the edge.
            if self.settings_rail_divider.live() && self.settings_rail_divider.band.contains(x, y) {
                return Some(Hit::SettingsRailDivider);
            }
            for (index, panel) in &self.settings_rail {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsSection(*index));
                }
            }
            for (index, side, panel) in &self.settings_rows {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsRow(*index, *side));
                }
            }
            // The document's text, which is the one place on this panel with
            // characters to point at rather than a control to press.
            if self.settings_doc_text.w >= 1.0 && self.settings_doc_text.contains(x, y) {
                return Some(Hit::SettingsDoc);
            }
            if self.settings.w >= 1.0 && self.settings.contains(x, y) {
                return Some(Hit::Settings);
            }
            return None;
        }
        for space in Space::ALL {
            let placed = self.placed(space);
            for (view, panel) in &placed.tabs {
                if panel.contains(x, y) {
                    return Some(Hit::Tab(*view, space));
                }
            }
            // With the tabs rather than on the floating layer: the arrows stand
            // in the strip, and a strip that holds all of its tabs has none, so
            // there is nothing to test for and nothing drawn.
            for (panel, hit) in [
                (placed.arrow_left, Hit::TabsLeft(space)),
                (placed.arrow_right, Hit::TabsRight(space)),
            ] {
                if panel.w >= 1.0 && panel.contains(x, y) {
                    return Some(hit);
                }
            }
        }
        if let Some(space) = self.files_in {
            for (index, panel) in &self.file_rows {
                if panel.contains(x, y) {
                    return Some(Hit::File(*index, space));
                }
            }
        }
        // The scroll gutter before the dividers and the bodies: it is the
        // smallest target of the three, and a track pressed at the pane's
        // edge must not read as the divider running beside it. Widened a
        // little past the drawn track, because four pixels is a target only
        // a plotter could hit. The explorer's own track first: it stands
        // inside the files pane, so the pane's band cannot cover it.
        if let Some(space) = self.files_in
            && self.file_list.w >= 1.0
        {
            let track = scroll_track(self.file_list);
            let band = Panel::new(
                track.x - SCROLL_GAP * 2.0,
                track.y,
                track.w + SCROLL_GAP * 3.0,
                track.h,
            );
            if band.contains(x, y) {
                return Some(Hit::Scrollbar(space));
            }
        }
        for space in Space::ALL {
            let placed = self.placed(space);
            if placed.body.h < 2.0 {
                continue;
            }
            let track = scroll_track(placed.body);
            let band = Panel::new(
                track.x - SCROLL_GAP * 2.0,
                track.y,
                track.w + SCROLL_GAP * 3.0,
                track.h,
            );
            if band.contains(x, y) {
                return Some(Hit::Scrollbar(space));
            }
        }
        // Before the bodies, because the band is wider than the gap it stands
        // in and so it reaches a little way into the pane on either side. After
        // the tabs and the file rows, which are smaller targets inside those
        // panes: the more particular thing under the pointer wins.
        // The vertical lines before the horizontal ones, so a point where two of
        // them cross moves the columns. Which half each one belongs to comes
        // back with it: two lines on the same axis are dragged apart.
        for (half, divider) in self.column_divider.iter().enumerate() {
            if divider.live() && divider.band.contains(x, y) {
                return Some(Hit::ColumnDivider(half));
            }
        }
        for (half, divider) in self.row_divider.iter().enumerate() {
            if divider.live() && divider.band.contains(x, y) {
                return Some(Hit::RowDivider(half));
            }
        }
        for space in Space::ALL {
            let placed = self.placed(space);
            if placed.body.contains(x, y) || placed.strip.contains(x, y) {
                return Some(Hit::Body(space));
            }
        }
        if self.input.contains(x, y) {
            return Some(Hit::Input);
        }
        // Whatever is left is the window itself: the gaps around the panes, and
        // the whole of it once every widget has been closed.
        Some(Hit::Window)
    }

    /// Where a tab released here lands.
    ///
    /// Off the window is its own answer rather than a miss. There is nowhere
    /// outside to put a pane, so the only two readings of a tab thrown out of
    /// the window are "close it" and "put it back where it was", and a tab that
    /// snaps back after being thrown away is the more surprising of the two.
    ///
    /// Everything below the tab strips is read off the grid rather than off the
    /// arrangement, so a pane spanning two cells can be dropped into either half
    /// of itself and a pair of cells can be merged whether or not anything is
    /// standing in them. See [`Layout::grid_landing`].
    pub fn landing(&self, x: f32, y: f32) -> Landing {
        if x < 0.0 || y < 0.0 || x > self.width || y > self.height {
            return Landing::Out;
        }
        // On a strip, the drop names a place among that space's tabs, which is
        // how a strip is reordered. The strip belongs to the pane rather than to
        // the grid, so it answers before the cells do, and the band around a
        // grid line stops where the strip under it starts. The gap that is drawn
        // between two panes is always outside a strip, so the line itself is
        // always a pair.
        if let Some(space) = self.hit(x, y).and_then(Hit::space)
            && self.placed(space).strip.contains(x, y)
        {
            return Landing::In(space, Some(self.insertion(space, x)));
        }
        self.grid_landing(x, y)
    }

    /// Which cell of the grid a point is in, or which two it is between.
    ///
    /// Inside a cell the drop takes that one cell, which is what breaks a span:
    /// the pane that was covering the pair keeps the other half of it. On or
    /// near the line between two cells the drop takes both, and the pane spans
    /// the pair. The band around a line is [`SPAN_BAND`] either side of it, cut
    /// down so it can never cover a whole cell.
    ///
    /// Where the two lines cross, the nearer of them wins, measured against its
    /// own band so a shallow band and a deep one are compared fairly. A dead
    /// heat goes to the horizontal line, which is the pair that spans a column:
    /// that is the arrangement the window opens with, so it is the one a tie
    /// should not surprise anyone by inverting.
    fn grid_landing(&self, x: f32, y: f32) -> Landing {
        let whole = around(around(self.grid[0], self.grid[1]), self.grid[3]);
        if !whole.contains(x, y) {
            return Landing::Nowhere;
        }
        // The middle of the gap between two cells, which is the line itself.
        // Read off the half it belongs to, never off the first cell of the grid:
        // with each column breaking at a height of its own, the line over the
        // right column is not the line over the left one, and a band measured
        // from the wrong one lands the drop in the wrong pair.
        let mid_x = |row: usize| {
            let head = self.grid[Space::at(row, 0).index()];
            head.x + head.w + GAP * 0.5
        };
        let mid_y = |column: usize| {
            let head = self.grid[Space::at(0, column).index()];
            head.y + head.h + GAP * 0.5
        };
        // The axis with one line across the whole grid answers first, because it
        // is the same wherever the pointer is. The half it names is then what
        // the other axis is read against.
        let (row, column) = match self.rows_first {
            true => {
                let row = usize::from(y >= mid_y(0));
                (row, usize::from(x >= mid_x(row)))
            }
            false => {
                let column = usize::from(x >= mid_x(0));
                (usize::from(y >= mid_y(column)), column)
            }
        };
        let (line_x, line_y) = (mid_x(row), mid_y(column));
        let beside = |a: Space, b: Space| (self.grid[a.index()], self.grid[b.index()]);
        let (left, right) = beside(Space::at(row, 0), Space::at(row, 1));
        let (top, bottom) = beside(Space::at(0, column), Space::at(1, column));
        let reach_x = GAP * 0.5 + SPAN_BAND.min(left.w.min(right.w) * 0.5);
        let reach_y = GAP * 0.5 + SPAN_BAND.min(top.h.min(bottom.h) * 0.5);
        let (off_x, off_y) = ((x - line_x).abs(), (y - line_y).abs());
        let near_x = off_x <= reach_x && reach_x > 0.0;
        let near_y = off_y <= reach_y && reach_y > 0.0;
        let cell = Space::at(row, column);
        match (near_x, near_y) {
            (true, true) => match off_x / reach_x < off_y / reach_y {
                true => Landing::span(cell, cell.in_row()),
                false => Landing::span(cell, cell.in_column()),
            },
            (true, false) => Landing::span(cell, cell.in_row()),
            (false, true) => Landing::span(cell, cell.in_column()),
            (false, false) => Landing::In(cell, None),
        }
    }

    /// Which place among a space's tabs a drop at this `x` takes.
    ///
    /// In front of the tab under the pointer or behind it, depending on which
    /// half of it the pointer is in, the way every tab strip does it. Counted
    /// against the space's whole list of tabs rather than the ones on screen, so
    /// a scrolled strip names the same tab the pointer is over.
    ///
    /// Past the last tab on screen is behind it, which is what the empty end of a
    /// strip and the strip's own arrows resolve to. Here rather than in
    /// [`crate::dock`] because it is the tab panels that answer it, and the caret
    /// drawn between two tabs has to come from the same arithmetic as the drop, or
    /// the mark and the move disagree.
    fn insertion(&self, space: Space, x: f32) -> usize {
        let placed = self.placed(space);
        for (step, (_, panel)) in placed.tabs.iter().enumerate() {
            if x < panel.x + panel.w * 0.5 {
                return placed.first_tab + step;
            }
        }
        placed.first_tab + placed.tabs.len()
    }

    /// Where the gap between two tabs is, for the place a drop would take.
    ///
    /// The left edge of the tab that would be pushed along, or the right edge of
    /// the last tab on screen when the drop is behind all of them. Clamped into
    /// the strip so a caret at either end is drawn on the strip rather than off
    /// the side of it.
    fn insertion_gap(&self, space: Space, at: usize) -> f32 {
        let placed = self.placed(space);
        // A place in front of the first tab on screen is that tab's own edge: a
        // scrolled strip has tabs off its left end, and the caret cannot be drawn
        // where they are.
        let step = at.saturating_sub(placed.first_tab);
        let gap = match placed.tabs.get(step) {
            Some((_, panel)) => panel.x,
            None => match placed.tabs.last() {
                Some((_, panel)) => panel.x + panel.w,
                None => placed.strip.x,
            },
        };
        gap.clamp(placed.strip.x, placed.strip.x + placed.strip.w)
    }

    /// Where a pointer at `x` puts the vertical divider in `half`, as the
    /// fraction [`Shape::left_width`] is.
    ///
    /// The inverse of the arithmetic [`Layout::compute`] lays the columns out
    /// with, off the same box, so the divider lands under the pointer rather
    /// than near it. Held by the same rule as well, so a drag thrown past either
    /// end of the window stops at the floor instead of collapsing a column.
    ///
    /// Off the half's own divider, so dragging one line answers for that line
    /// alone and the other half keeps the fraction it was left at.
    pub fn column_ratio_at(&self, half: usize, x: f32) -> f32 {
        let divider = self.column_divider[half];
        let track = divider.track;
        let room = (track.w - GAP).max(1.0);
        held((x - track.x - GAP * 0.5) / room, room, divider.floor)
    }

    /// The same for the settings panel's rail: where a pointer at `x` puts the
    /// line between the names and the settings, as the fraction
    /// [`Shape::settings_rail`] is.
    ///
    /// The same arithmetic off the same box the panel is laid out with, and held
    /// by the same floor, so the line lands under the pointer and a drag thrown
    /// off either end of the panel stops where the names still fit.
    pub fn settings_rail_ratio_at(&self, x: f32) -> f32 {
        let divider = self.settings_rail_divider;
        let track = divider.track;
        // The list stands PAD in from the line, so the room the two sides
        // share is a padding smaller than the track; the same subtraction the
        // placement makes, or the line stops landing under the pointer.
        let room = (track.w - GAP - PAD).max(1.0);
        held((x - track.x - GAP * 0.5) / room, room, divider.floor)
    }

    /// The same for the horizontal divider in `half`.
    pub fn row_ratio_at(&self, half: usize, y: f32) -> f32 {
        let divider = self.row_divider[half];
        let track = divider.track;
        let room = (track.h - GAP).max(1.0);
        held((y - track.y - GAP * 0.5) / room, room, divider.floor)
    }

    /// Rows a panel can show. The header line is content, not scrollback.
    pub fn rows(&self, panel: Panel, size: f32) -> usize {
        Text::rows_for(size, panel.inset(PAD).h)
    }

    /// How many rows the picker's list can show. Its box is already the content
    /// box, so this does not inset again; taking [`Layout::rows`] to it would
    /// lose a row and put the cursor's own row off screen at the bottom.
    pub fn picker_capacity(&self, size: f32) -> usize {
        Text::rows_for(size, self.picker_list.h)
    }

    /// How many rows the settings panel's list can show, on the same terms.
    pub fn settings_capacity(&self, size: f32) -> usize {
        Text::rows_for(size, self.settings_list.h)
    }

    /// How many characters wide the list's own text is, which is what a row
    /// carrying a wrapped description is as tall as.
    ///
    /// The window asks here for the same reason the drawing does: the keys, the
    /// wheel and the scrollbar all count a row's height, and a height counted in
    /// a width the row was not drawn in scrolls the list past what is on screen.
    pub fn settings_entry_columns(&self, column: f32) -> usize {
        crate::settings::places::settings_entry_cols(crate::settings::places::settings_list_rows(self.settings_list).w, column)
    }

    /// How many rows of the document beside it are on screen, and how many
    /// columns it wraps in.
    ///
    /// One answer for the drawing, for the wheel and for the scrollbar. They
    /// were three: the column was drawn in `rows_for` of a box with no padding
    /// and paged by `rows` of the same box inset by [`PAD`], so a wheel moved
    /// the text by a row or two more than it showed.
    pub fn settings_doc_rows(&self, size: f32) -> usize {
        Text::rows_for(size, self.settings_doc_text.h)
    }

    pub fn settings_doc_columns(&self, column: f32) -> usize {
        match self.settings_doc_text.w >= 1.0 {
            true => columns_in(self.settings_doc_text.w, column),
            false => 0,
        }
    }

    /// The character cell of that document nearest the pointer, wherever the
    /// pointer is.
    ///
    /// The document's twin of [`Layout::cell_in`], and clamped for the same
    /// reason: a drag that ran off the box keeps running to the nearest cell,
    /// which is what puts the last characters of the bottom row within reach.
    /// Its box is [`Layout::settings_doc_text`], which is already the rectangle
    /// the glyphs are in, so nothing is inset here: the wrapper's padding was
    /// taken off when the box was placed, and taking it off twice would put
    /// every column one to the left of the glyph under the pointer.
    pub fn settings_doc_cell(&self, x: f32, y: f32, size: f32, column: f32) -> Option<(usize, usize)> {
        let body = self.settings_doc_text;
        if self.shaded || column <= 0.0 || body.w < 1.0 || body.h < 1.0 {
            return None;
        }
        let line = Text::line_for(size);
        let rows = Text::rows_for(size, body.h);
        let row = ((y - body.y) / line).floor().max(0.0) as usize;
        let at = (((x - body.x) / column).round().max(0.0)) as usize;
        Some((row.min(rows.saturating_sub(1)), at.min(columns_in(body.w, column))))
    }

    /// Where along one row's track a pointer sits, 0 at the low end and 1 at the
    /// high. Nothing when that row has no track.
    ///
    /// Clamped rather than dropped outside the track, so a drag that ran off the
    /// end of it holds the end instead of stopping: the pointer is still down,
    /// and a slider that goes dead when the pointer overshoots by a pixel is a
    /// slider that cannot reach its own ends.
    pub fn slider_at(&self, index: usize, side: Side, x: f32) -> Option<f32> {
        let (_, _, track) = self
            .settings_tracks
            .iter()
            .find(|(at, half, _)| *at == index && *half == side)
            .filter(|(_, _, track)| track.w >= 1.0)?;
        Some(((x - track.x) / track.w).clamp(0.0, 1.0))
    }

    /// How many of the menu's rows are on screen, for the wheel and for the
    /// keyboard. Read off the rows the layout actually placed, so the scroll is
    /// bounded by what is drawn rather than by an arithmetic of its own.
    pub fn menu_capacity(&self) -> usize {
        self.menu_rows.len()
    }

    /// The box a space's text is drawn in.
    ///
    /// The whole body for every view but the file one, which gives its left
    /// column to the explorer. Selection and hit testing have to ask here
    /// rather than taking the body, or a click in a file lands a list's width
    /// away from the character under the pointer.
    pub fn content(&self, space: Space) -> Panel {
        match self.files_in == Some(space) && self.file_diff.w >= 1.0 {
            true => self.file_diff,
            false => self.placed(space).body,
        }
    }

    /// Which pane the pointer is over, and which character cell of it.
    ///
    /// Arithmetic rather than a layout query, which is what a monospace grid
    /// buys: the renderer never has to be asked where a glyph landed. The
    /// column is rounded to the nearest boundary rather than floored, so
    /// pressing on the right half of a character puts the caret after it, the
    /// way a text cursor behaves everywhere else.
    ///
    /// Built for the DEBUG pane, whose rows were clicked rather than selected.
    /// That pane is gone and every press now arrives with the space already
    /// named by its [`Hit`], so the window asks [`Layout::cell_in`] instead and
    /// what is left of this is the layout tests: it is the one query that
    /// answers "is this point in any text box at all", which is what a shaded
    /// window and an open settings panel are checked with.
    #[cfg(test)]
    pub fn cell(&self, x: f32, y: f32, size: f32, column: f32) -> Option<(Space, usize, usize)> {
        if self.shaded || column <= 0.0 {
            return None;
        }
        let line = Text::line_for(size);
        for space in Space::ALL {
            let body = self.content(space).inset(PAD);
            if !body.contains(x, y) {
                continue;
            }
            let row = ((y - body.y) / line).floor().max(0.0) as usize;
            let at = (((x - body.x) / column).round().max(0.0)) as usize;
            return Some((space, row, at));
        }
        None
    }

    /// The character cell of one named pane nearest the pointer, wherever the
    /// pointer is.
    ///
    /// [`Layout::cell`] answers "which pane, which cell" and so has to say
    /// nothing when the point is outside every text box. Selecting needs the
    /// other question: the pane is already known (the press picked it, or the
    /// drag belongs to it) and what is wanted is the cell to run to. Dropping
    /// the answer there is what made a sweep to the bottom right stop short of
    /// the last characters, and what made a press in the nine pixel padding
    /// start no selection at all. So the point is clamped into the box rather
    /// than refused, and the cell comes back clamped to the box's own grid:
    /// `rows - 1` down and `cols` across, one past the last character being
    /// where a caret sits after it.
    pub fn cell_in(&self, space: Space, x: f32, y: f32, size: f32, column: f32) -> Option<(usize, usize)> {
        if self.shaded || column <= 0.0 {
            return None;
        }
        let body = self.content(space).inset(PAD);
        let line = Text::line_for(size);
        let rows = Text::rows_for(size, body.h);
        let row = ((y - body.y) / line).floor().max(0.0) as usize;
        let at = (((x - body.x) / column).round().max(0.0)) as usize;
        Some((row.min(rows.saturating_sub(1)), at.min(columns_in(body.w, column))))
    }

    /// Where a click in the prompt puts the caret, as a character offset into
    /// the typed text.
    ///
    /// The inverse of the arithmetic [`input_row`] draws the caret with, off
    /// the same box, so the caret lands under the pointer instead of near it.
    /// A click past the end of the text lands at the end, which is why `chars`
    /// is passed in, and it lands on the row that is on screen rather than on
    /// the row that would be there unscrolled, which is why `caret` is.
    pub fn input_caret(
        &self,
        x: f32,
        y: f32,
        size: f32,
        column: f32,
        chars: usize,
        caret: usize,
    ) -> usize {
        if column <= 0.0 {
            return chars;
        }
        let line = Text::line_for(size);
        let box_ = input_box(self.input, line);
        let columns = columns_in(box_.w, column);
        let skip = prompt_skip(caret, columns, rows_in(box_, line));
        let row = skip + (((y - box_.y) / line).floor().max(0.0) as usize);
        // Rounded, not floored, so pressing on the right half of a character
        // puts the caret after it, the way a text cursor behaves everywhere.
        let at = ((((x - box_.x) / column).round().max(0.0)) as usize).min(columns);
        // The marker in front of the text owns the first columns of the first
        // row, so a click on it means the start of the text.
        (row * columns + at).saturating_sub(PROMPT_COLUMNS).min(chars)
    }
}










/// A tab being dragged, and where it would land if it were dropped now.
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub view: View,
    pub at: (f32, f32),
    /// What a drop right now would do, taken from [`Layout::landing`]. The same
    /// answer the release acts on, so the box drawn over the target, the caret
    /// between two tabs and the move that happens cannot disagree.
    pub landing: Landing,
}

pub struct Frame<'a> {
    pub state: &'a State,
    /// Where each list pane is scrolled to, owned by the shell.
    pub scrolls: &'a crate::scroll::Scrolls,
    /// The explorer's first visible row, owned by the shell.
    pub file_scroll: usize,
    pub monitor: &'a Monitor,
    pub dock: &'a Dock,
    pub skin: &'a Skin,
    pub layout: &'a Layout,
    /// What has been typed, where the caret is, and what is selected in it.
    pub prompt: &'a crate::prompt::Prompt,
    pub column: f32,
    /// The column width at `pane_size`. The panes are a different size from
    /// the transcript, so anything that lines text up with a rectangle has to
    /// use this one.
    pub pane_column: f32,
    pub body_size: f32,
    pub pane_size: f32,
    pub drag: Option<Drag>,
    /// What the pointer is over, for the button highlight.
    pub hot: Option<Hit>,
    /// Shown in the title bar when the agent could not be reached.
    pub trouble: Option<&'a str>,
    /// A first ESC has landed with nothing left for it to drop: the input row
    /// says the second one is the one that cancels the turn.
    pub esc_armed: bool,
    /// The call popup's first visible content row, shell-owned like every
    /// other scroll offset.
    pub popup_scroll: [usize; 2],
    /// Where the pointer is, for the hover highlights. Off screen when the
    /// window has never seen it.
    pub cursor: (f32, f32),
    /// A drag over one of the text panes, drawn as a band under the glyphs.
    pub selection: Option<crate::select::Selection>,
    /// The open menu. The same one the layout was computed from, or the rows
    /// would be drawn somewhere other than where they are hit tested.
    pub menu: Option<&'a Menu>,
    /// The folder picker, while it is up. The same one, for the same reason.
    pub picker: Option<&'a Picker>,
    /// The settings panel, while it is up. The same one the layout was computed
    /// from, or a row would be drawn somewhere other than where it is clicked.
    pub settings: Option<&'a Settings>,
    /// Seconds since the window opened, which is the orb's clock.
    ///
    /// Passed in rather than read here. A frame is a function of what it is
    /// given, so the same clock builds the same scene twice, which is the only
    /// way an animation is testable without a screen.
    pub clock: f32,
    /// How far the orb is through the move between its resting square and its
    /// turning circles, while it is moving.
    ///
    /// `None` is settled, and the phase says at which end: a turn running is the
    /// circles and no turn is the square. Every frame outside a transition is
    /// `None`, including every frame the window drew before there was one.
    pub orb_morph: Option<f32>,
}

impl Frame<'_> {
    /// The font size and column width a view is actually drawn with.
    ///
    /// The output pane uses the transcript size and every other pane the
    /// smaller one.
    /// Measuring a pane with the wrong one of the two is what put the
    /// selection band and the hit test off the glyphs they were describing,
    /// so nothing may reach for `body_size` or `pane_size` directly when the
    /// view is a variable.
    pub fn metrics_of(&self, view: View) -> (f32, f32) {
        match view {
            View::Output => (self.body_size, self.column),
            _ => (self.pane_size, self.pane_column),
        }
    }

    /// How far the orb is between its two formations, as [`crate::orb`] wants
    /// it: the transition's own progress while there is one, and the end the
    /// phase names when there is not.
    fn morph(&self) -> f32 {
        self.orb_morph.unwrap_or(match self.state.phase.busy() {
            true => 1.0,
            false => 0.0,
        })
    }
}

pub fn build(frame: &Frame) -> Scene {
    let mut scene = Scene::default();
    let layout = frame.layout;

    // Shaded, the bar is the whole window: no backdrop, no panes, no prompt.
    // A compositor is free to hand back a surface taller than the strip was
    // asked for, so what covers that surface has to be the bar itself (see
    // `title_bar`). A full-window backdrop under the strip is what drew the black
    // bar below it, and clearing to transparent instead drew the same black.
    if layout.shaded {
        title_bar(&mut scene, frame);
        overlay(&mut scene, frame);
        return scene;
    }

    scene.rect(Panel::new(0.0, 0.0, layout.width, layout.height).fill(frame.skin.backdrop));
    title_bar(&mut scene, frame);

    // No folder chosen yet, so there is nothing to arrange panes around and
    // nothing to type at. The menu still goes on top: a right click on a session
    // row opens one, and the layout has always placed and hit tested it here, so
    // returning before the overlay left a menu that answered presses and was
    // nowhere on screen.
    if layout.picking {
        crate::picker::paint::folder_picker(&mut scene, frame);
        overlay(&mut scene, frame);
        return scene;
    }

    // The settings panel covers the panes and the prompt, so nothing under it is
    // drawn: a pane painted behind a panel that fills the surface is a pane
    // nobody sees, redrawn on every keystroke.
    if layout.in_settings {
        crate::settings::paint::settings_panel(&mut scene, frame);
        return scene;
    }

    for space in Space::ALL {
        space_pane(&mut scene, frame, space);
    }
    input_row(&mut scene, frame);
    // All three on the floating layer, in the order they stack: the box over the
    // target space, the caret in the gap the tab would go into, then the tab
    // itself under the pointer, over both.
    drop_target(&mut scene, frame);
    dragging(&mut scene, frame);
    overlay(&mut scene, frame);
    scene
}














/// What a view's tab says. Every view's own label, except the agent-output
/// view, whose tab names the agent it is on: `[N] AGENT - OUTPUT`. One
/// function for the layout's widths and the painter's glyphs, so a tab is
/// never wider or narrower than the label drawn in it.
pub(crate) fn tab_label(view: View, agent_tab: Option<usize>) -> String {
    match (view, agent_tab) {
        (View::Agent, Some(ordinal)) => format!("[{ordinal}] AGENT - OUTPUT"),
        _ => view.label().to_string(),
    }
}



/// Everything about a box a band is painted in that is not the selection or the
/// pane it is over.
///
/// One struct rather than nine arguments, because a band drawn with any of them
/// off by one is a highlight over text the clipboard does not have.
pub(crate) struct Painted {
    /// The rectangle the glyphs are in, already inset.
    pub(crate) content: Panel,
    pub(crate) rows: usize,
    pub(crate) cols: usize,
    /// Columns in front of the text on every row, which the file view spends on
    /// line numbers and everything else spends on nothing.
    pub(crate) chrome: usize,
    pub(crate) size: f32,
    pub(crate) column: f32,
    pub(crate) tint: [f32; 4],
}



/// One logical line of a list pane: the runs that draw it, and the text they
/// draw.
///
/// The text is taken off the runs rather than passed in beside them, and the
/// height is measured from it by the same call the renderer breaks the rows
/// with. A row counted one way and drawn another is a pane that scrolls by a
/// different number of rows than it has, and a row of prose with blanks in it
/// wraps at a different place from a row of the same length without them.
pub(crate) struct ListRow {
    runs: Vec<Run>,
    text: String,
}

impl ListRow {
    pub(crate) fn new(runs: Vec<Run>) -> ListRow {
        let text = runs.iter().map(|run| run.text.as_str()).collect();
        ListRow { runs, text }
    }

    /// How many rows this takes in a box `cols` wide.
    pub(crate) fn rows(&self, cols: usize) -> usize {
        text_geometry::rows_in(&self.text, cols, crate::state::PANE_WRAP).len()
    }
}



/// How tall a scrolling pane's content is and how much of it is on screen, as the
/// heights and the row count every [`crate::scroll::Scrolls`] operation takes.
///
/// `None` for a view that keeps its own scrollback on a [`crate::state::Pane`]
/// (OUTPUT, ACTIVITY and the open file), and for one with nothing to scroll.
///
/// The one place outside the drawing that knows how tall a pane's content is: the
/// wheel, the page keys and the per-frame clamp all ask here. Anything that
/// worked it out for itself would eventually scroll a pane by a different number
/// of rows than the pane drew, which is the class of bug `text_geometry` exists
/// to end.
pub fn scroll_extent(frame: &Frame, view: View, panel: Panel) -> Option<(Vec<usize>, usize)> {
    let fit = frame.layout.rows(panel, frame.pane_size);
    let cols = cols_of(panel, frame.pane_column);
    let lines = |rows: Vec<ListRow>| {
        Some((rows.iter().map(|row| row.rows(cols)).collect(), fit))
    };
    match view {
        View::Plan => lines(crate::widgets::plan::plan_rows(frame.state, frame.skin)),
        View::Agents => lines(crate::widgets::agents::agent_rows(frame.state, frame.skin, cols)),
        View::Hardware => crate::widgets::gauges::gauge_extent(frame, panel, frame.monitor.hardware()),
        View::Session => crate::widgets::gauges::gauge_extent(frame, panel, frame.monitor.session()),
        // The readings sit under the header, in a box of their own, and it is that
        // box they scroll in.
        View::Context => crate::widgets::gauges::gauge_extent(
            frame,
            crate::widgets::gauges::gauge_area(panel, frame.pane_size)?,
            frame.monitor.context(),
        ),
        View::Output | View::Activity | View::Files | View::Agent => None,
    }
}



















/// A file's label cut to fit `cols` columns of the explorer.
///
/// The column is narrow, so a name that does not fit loses its parent directory
/// first: `src/mod.rs` cut to `src/mo…` says less than `mod.rs` does, and the
/// parent is only ever there to tell two `mod.rs` apart. If the name itself
/// still does not fit, its tail goes and an ellipsis says so. The tail rather
/// than the head because the row already carries a type icon, so the extension
/// is not what the last characters are needed for.
pub(crate) fn fit_name(path: &str, cols: usize) -> String {
    let full = short_name(path);
    if full.chars().count() <= cols {
        return full;
    }
    let base = full.rsplit('/').next().unwrap_or(&full);
    if base.chars().count() <= cols {
        return base.to_string();
    }
    // One column short, because `clip` spends one on the ellipsis it adds. With
    // one column there is room for the ellipsis alone, and with none for
    // nothing: a pane can be dragged to any width and a label that came back
    // wider than the room it was given would wrap, which would put two rows
    // where the list has one.
    match cols {
        0 => String::new(),
        1 => String::from("\u{2026}"),
        _ => clip(base, cols - 1),
    }
}

/// A path shortened to its tail, so a deep workspace reads as one line. Drawn by
/// the session monitor, which is where the workspace reading went when the title
/// strip was cut back to the build stamp.
fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.rsplit('/').take(2).collect();
    match parts.len() {
        0 => String::new(),
        1 => parts[0].to_string(),
        _ => format!("{}/{}", parts[1], parts[0]),
    }
}

pub(crate) mod chrome;
pub(crate) mod place;
pub(crate) mod draw;

#[cfg(test)]
pub(crate) mod testkit;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::icons;
    use noob_draw::Rect;
    #[allow(clippy::wildcard_imports)]
    use super::testkit::*;
    use crate::config::Config;
    
    
    
    
    use crate::state::Tone;









    /// The window says which release it is, and no longer which commit.
    ///
    /// The commit was seven characters of hex nobody read off a title strip,
    /// and the room it took says where the agent is working instead. `build.rs`
    /// still stamps it; nothing draws it.
    #[test]
    fn the_title_bar_names_the_release_and_not_the_commit() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &[]);
        let text = text_of(&out.scene);
        // The window is NO0B and has one name. It used to draw the product name
        // twice, "NO0B \u{25b8} CLIppy", and the second one is gone.
        assert!(text.contains("NO0B"), "{text}");
        assert!(!text.contains("CLIppy"), "the old name is still drawn: {text}");
        assert!(text.contains(VERSION), "the version is not on screen: {text}");
        let commit = env!("NO0B_BUILD")
            .strip_prefix(VERSION)
            .unwrap_or("")
            .trim()
            .trim_end_matches('+');
        if !commit.is_empty() {
            assert!(
                !text.contains(commit),
                "the build commit {commit:?} is still drawn: {text}"
            );
        }
    }

    /// Read left to right: the orb, the name, the marker, the version, the same
    /// marker again, and the folder the agent is working in.
    ///
    /// The path is what the commit hash used to be. Both halves of the orb are
    /// here too: it is drawn, as discs inside the leftmost [`ORB_W`] of the
    /// strip, and no text starts inside that square, so the two share the strip
    /// instead of overlapping.
    #[test]
    fn the_title_strip_reads_orb_then_name_then_version_then_path() {
        let state = busy_state();
        let out = render(&state, 1400.0, 900.0, &Dock::new(), &[]);
        assert!(
            discs_of(&out.scene).len() > 100,
            "the orb is not drawn: {} discs",
            discs_of(&out.scene).len()
        );
        let title = out
            .scene
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text.contains("NO0B")))
            .expect("the title strip names the window");
        let line: String = title.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(
            line,
            format!(
                "NO0B \u{25b8} {VERSION} \u{25b8} {}",
                short_path(&state.workspace)
            ),
            "the strip reads {line:?}"
        );
        // The same marker between each pair, and only those two.
        assert_eq!(line.matches('\u{25b8}').count(), 2, "{line:?}");
        // Before the agent says where it is, the marker has nothing to point at
        // and is not drawn either.
        let quiet = render(&State::new(), 1400.0, 900.0, &Dock::new(), &[]);
        let bare: String = quiet
            .scene
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text.contains("NO0B")))
            .expect("the strip names the window before a session starts")
            .runs
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        assert_eq!(bare, format!("NO0B \u{25b8} {VERSION}"), "{bare:?}");
        assert!(title.at.x >= ORB_W, "the text starts at {}", title.at.x);
        for text in &out.scene.texts {
            if text.at.y >= TITLE_H {
                continue;
            }
            assert!(
                text.at.x >= ORB_W,
                "text at x={} is in the orb's room: {:?}",
                text.at.x,
                text.runs.iter().map(|run| run.text.as_str()).collect::<String>()
            );
        }
    }

    /// The title strip wears the theme: its fill is the skin's bar and its
    /// writing is the skin's own inks, under every preset.
    ///
    /// "this should also change the bar top where no0b has version". Rendered
    /// under all three themes rather than only the green one, so a tint
    /// hardcoded anywhere in the strip shows up as a matrix colour in a red
    /// window.
    #[test]
    fn the_title_strip_wears_the_theme_it_is_given() {
        let state = busy_state();
        for name in crate::config::THEMES {
            let config = crate::config::theme(name).expect(name);
            let out = render_skinned(
                &state,
                &crate::scroll::Scrolls::default(),
                1400.0,
                900.0,
                &Dock::new(),
                &[],
                &Monitor::new(),
                None,
                Skin::from(&config),
            );
            let (skin, strip) = (&out.skin, out.layout.title);
            // The strip's own fill is the theme's bar, exactly where the
            // layout put the strip.
            assert!(
                out.scene.rects.iter().any(|rect| {
                    rect.rgba() == skin.bar && rect.xywh() == [strip.x, strip.y, strip.w, strip.h]
                }),
                "{name}: the strip is not filled with the theme's bar"
            );
            // The name is the theme's loud ink and the version its title ink.
            let title = out
                .scene
                .texts
                .iter()
                .find(|text| text.runs.iter().any(|run| run.text.contains("NO0B")))
                .unwrap_or_else(|| panic!("{name}: the strip does not name the window"));
            assert_eq!(title.runs[0].color, Some(skin.bright), "{name}: the name");
            assert_eq!(title.runs[1].color, Some(skin.title), "{name}: the version");
            // The window buttons come off the skin too: the quiet pair in the
            // theme's dim ink, close in its title ink.
            for (glyph, ink, what) in [
                (crate::design::icons::MINIMIZE, skin.dim, "minimize"),
                (crate::design::icons::MAXIMIZE, skin.dim, "maximize"),
                (crate::design::icons::CLOSE, skin.title, "close"),
            ] {
                let button = out
                    .scene
                    .texts
                    .iter()
                    .flat_map(|text| text.runs.iter())
                    .find(|run| run.icon && run.text.contains(glyph))
                    .unwrap_or_else(|| panic!("{name}: no {what} button drawn"));
                assert_eq!(button.color, Some(ink), "{name}: the {what} button's ink");
            }
        }
    }




    /// The orb is in the strip in both states, and it stays in its own square.
    /// A disc outside it would be the animation drawing over the window's name.
    #[test]
    fn the_orb_is_in_the_strip_and_stays_in_its_square() {
        for (state, name) in [(busy_state(), "working"), (State::new(), "resting")] {
            let out = render_at(&state, 4.0);
            let discs = discs_of(&out.scene);
            assert!(discs.len() > 100, "{name}: {} discs", discs.len());
            for disc in discs {
                let [x, y, w, h] = disc.xywh();
                assert!(x >= 0.0 && x + w <= ORB_W + 0.01, "{name}: {disc:?} left the square");
                assert!(y >= 0.0 && y + h <= TITLE_H + 0.01, "{name}: {disc:?} left the strip");
            }
        }
    }

    /// The two states, as the window sees them. Working animates and carries more
    /// discs, because the runners are only there while there is a turn to run;
    /// resting is one frozen frame, so the clock cannot change it.
    #[test]
    fn the_orb_animates_while_a_turn_runs_and_is_frozen_otherwise() {
        let boxes = |out: &Rendered| -> Vec<[f32; 4]> {
            discs_of(&out.scene).iter().map(|disc| disc.xywh()).collect()
        };

        let busy = busy_state();
        let first = boxes(&render_at(&busy, 0.0));
        let later = boxes(&render_at(&busy, 0.4));
        assert_ne!(first, later, "the orb does not move while a turn runs");

        let quiet = State::new();
        assert_eq!(
            boxes(&render_at(&quiet, 0.0)),
            boxes(&render_at(&quiet, 90.0)),
            "the resting orb moved, so the window would never stop redrawing"
        );
        assert!(
            boxes(&render_at(&quiet, 0.0)).len() < first.len(),
            "resting draws as much as working"
        );
    }

    /// A turn starting is not a swap. For the length of the move the strip draws
    /// a frame that is neither formation: the resting dots are on their way to
    /// the circles, so they are somewhere else than either end put them, and
    /// they are part of the way round from a square to a disc.
    ///
    /// The state is the resting one, which is the point: what the orb draws
    /// during the move comes from the transition and not from the phase, or the
    /// window would cut to the far end the moment the turn started.
    #[test]
    fn the_orb_draws_the_move_rather_than_cutting_between_the_two_formations() {
        let quiet = State::new();
        let boxes = |out: &Rendered| -> Vec<[f32; 4]> {
            discs_of(&out.scene).iter().map(|disc| disc.xywh()).collect()
        };
        let settled = boxes(&render_moving(&quiet, 3.0, None));
        let turning = boxes(&render_moving(&quiet, 3.0, Some(1.0)));

        let moving = render_moving(&quiet, 3.0, Some(0.5));
        let halfway = boxes(&moving);
        assert_ne!(halfway, settled, "the orb has not left its square");
        assert_ne!(halfway, turning, "the orb cut straight to the circles");
        assert!(halfway.len() > settled.len(), "no circles are coming up");
        assert!(halfway.len() <= turning.len(), "the move draws more than the circles");

        // Halfway round as well as halfway there: a square dot has no corner
        // radius and a disc has half its width.
        for disc in discs_of(&moving.scene) {
            let [_, _, w, _] = disc.xywh();
            assert!(
                (disc.extra()[0] - w * 0.25).abs() < 1e-4,
                "{disc:?} is not halfway between a square and a disc"
            );
        }
    }

    /// Shaded, the strip is the whole window, so the orb is the only thing in it
    /// besides the headline. It is drawn there too: the strip is the same strip.
    #[test]
    fn the_shaded_strip_keeps_the_orb() {
        let state = busy_state();
        let dock = Dock::new();
        let mut shape = shape(&dock, &[]);
        shape.shaded = true;
        let layout = Layout::compute(1180.0, TITLE_H, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 2.0,
            orb_morph: None,
            drag: None,
            hot: None,
            trouble: None,
            esc_armed: false,
            popup_scroll: [0, 0],
            cursor: (-100.0, -100.0),
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        });
        assert!(discs_of(&scene).len() > 100, "{} discs", discs_of(&scene).len());
    }

    /// The reading after the name starts with the version the crate carries.
    ///
    /// The first question about a build is which release it is, and a commit
    /// cannot answer it. What the strip draws is taken from the crate, not typed
    /// in beside it, so the window cannot show a version the package does not
    /// have.
    #[test]
    fn the_title_bar_reads_the_crate_version() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &[]);
        let title = out
            .scene
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text.contains("NO0B")))
            .expect("the title strip names the window");
        let reading = title
            .runs
            .iter()
            .find(|run| run.text.trim().starts_with(VERSION))
            .unwrap_or_else(|| {
                let line: String = title.runs.iter().map(|run| run.text.as_str()).collect();
                panic!("the strip does not read the version {VERSION:?}: {line}")
            });
        assert_eq!(
            reading.text.trim().split(' ').next(),
            Some(env!("CARGO_PKG_VERSION")),
            "the version on screen is not the crate's: {:?}",
            reading.text
        );
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
    }

    /// The CLI and the window are separate cargo workspaces with separate
    /// lockfiles and they ship as one release, so their versions move together.
    /// A version that moves in one workspace only is the same defect as a
    /// version that does not move at all.
    #[test]
    fn both_workspaces_carry_the_same_version() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
        // A source tarball of the window alone has no CLI beside it, and it
        // still has to build and test.
        let Ok(manifest) = std::fs::read_to_string(&root) else {
            return;
        };
        let line = manifest
            .lines()
            .find(|line| line.starts_with("version = "))
            .expect("the CLI workspace sets a version");
        assert!(
            line.contains(&format!("\"{VERSION}\"")),
            "the CLI workspace is on {line:?} and the window on {VERSION:?}"
        );
    }

    /// The strip carries the name, the release and the folder, and nothing else.
    ///
    /// It used to carry the phase, the model, the workspace, a resumed marker
    /// and the whole token budget on one unlabelled line. Those are readings
    /// and they moved to the monitors, so this asserts they are gone from here
    /// rather than that they are here, which is what it asserted before. The
    /// folder came back on its own terms: with a marker in front of it and
    /// nothing else competing for the line.
    #[test]
    fn the_title_strip_carries_only_the_name_the_release_and_the_folder() {
        let state = busy_state();
        let out = render(&state, 1400.0, 900.0, &Dock::new(), &[]);
        let title = out
            .scene
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text.contains("NO0B")))
            .expect("the title strip names the window");
        let line: String = title.runs.iter().map(|run| run.text.as_str()).collect();
        assert!(line.contains(VERSION), "{line}");
        assert!(line.contains(&short_path(&state.workspace)), "{line}");
        // The budget was a whole line of readings up here. It is a set of
        // monitor rows now, so what is asserted is that none of its words are.
        for evicted in [
            state.phase.word().to_lowercase(),
            state.model.clone(),
            String::from("prefilled"),
            String::from("requests"),
        ] {
            assert!(
                !line.contains(&evicted),
                "{evicted:?} is still in the title strip: {line}"
            );
        }
        // And the reading is readable. It was in the dim tint, the faintest the
        // palette has, and two builds could not be told apart by it.
        let reading = title
            .runs
            .iter()
            .find(|run| run.text.contains(VERSION))
            .expect("the version is a run of its own");
        assert_eq!(reading.color, Some(out.skin.title));
        assert_ne!(reading.color, Some(out.skin.dim));
    }

    /// The bar along the bottom is gone and nothing was put back down there.
    #[test]
    fn nothing_is_drawn_along_the_bottom() {
        let (w, h) = (1400.0, 900.0);
        let out = render(&busy_state(), w, h, &Dock::new(), &[]);

        // The input row now runs to the bottom of the window. It used to stop
        // 24 pixels short, and those pixels were the bar.
        let floor = out.layout.input.y + out.layout.input.h;
        // Only the window's own bottom margin is left, not a reserved strip.
        assert!(
            h - floor <= GAP + 0.01,
            "the input row stops {} short of the bottom, more than the {GAP}px margin, \
             so something is still reserved down there",
            h - floor
        );
    }

    /// The context gauge moved to the bottom edge of the title strip. It is two
    /// pixels either way; what matters is that it is still drawn and still
    /// scales with how full the context is.
    #[test]
    fn the_context_gauge_is_a_hairline_under_the_title_strip() {
        let mut state = busy_state();
        state.context = Some(crate::state::ContextFill {
            used: 4_000,
            total: 16_000,
            compact_at: 12_000,
        });
        let out = render(&state, 1400.0, 900.0, &Dock::new(), &[]);
        let edge = out.layout.title.y + out.layout.title.h - 2.0;
        let hairlines: Vec<[f32; 4]> = out
            .scene
            .rects
            .iter()
            .map(|r| r.xywh())
            .filter(|[_, y, _, h]| (*y - edge).abs() < 0.01 && (*h - 2.0).abs() < 0.01)
            .collect();
        assert!(
            hairlines.len() >= 2,
            "expected a track and a fill on the strip's bottom edge, got {hairlines:?}"
        );
        let fill = hairlines.iter().map(|[_, _, w, _]| *w).fold(f32::INFINITY, f32::min);
        let track = hairlines.iter().map(|[_, _, w, _]| *w).fold(0.0f32, f32::max);
        assert!(fill > 0.0 && fill < track, "the fill has to be part of the track: {hairlines:?}");
    }

    /// The arrow at the end of each tab strip is gone. Clicking the tab already
    /// showing still collapses its space, so nothing was lost with it, and the
    /// square it occupied is now available to tabs.
    #[test]
    fn no_control_sits_at_the_end_of_a_tab_strip() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &[]);
        for space in Space::ALL {
            let strip = out.layout.placed(space).strip;
            if strip.w < 1.0 {
                continue;
            }
            let (x, y) = (strip.x + strip.w - TAB_H * 0.5, strip.y + strip.h * 0.5);
            // Whatever is under the square the arrow used to occupy, it is
            // not a control of its own: a strip resolves only to its tabs now.
            let hit = out.layout.hit(x, y);
            assert!(
                matches!(hit, None | Some(Hit::Tab(..)) | Some(Hit::Body(_)) | Some(Hit::TitleBar)),
                "{space:?} still has a control at the end of its strip: {hit:?}"
            );
        }
    }





    /// A first ESC arms rather than cancels, and the armed frame is the one
    /// that says so: the input row carries the second-tap hint, and an
    /// unarmed frame makes no such promise anywhere.
    #[test]
    fn an_armed_cancel_writes_its_hint_into_the_input_row() {
        let state = busy_state();
        let dock = Dock::new();
        let shape = shape(&dock, &[]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let skin = Skin::from(&Config::default());
        let mut frame = Frame {
            state: &state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot: None,
            trouble: None,
            esc_armed: true,
            popup_scroll: [0, 0],
            cursor: (-100.0, -100.0),
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        };
        assert!(text_of(&build(&frame)).contains("press ESC again to cancel"));
        frame.esc_armed = false;
        assert!(!text_of(&build(&frame)).contains("press ESC again to cancel"));
    }






    #[test]
    fn the_default_arrangement_puts_every_space_on_screen() {
        let dock = Dock::new();
        for (w, h) in [(1200.0, 800.0), (700.0, 460.0), (2200.0, 1400.0)] {
            let out = render(&busy_state(), w, h, &dock, &["a.rs"]);
            for space in occupied(&dock) {
                let placed = out.layout.placed(space);
                assert!(placed.strip.w > 1.0, "{space:?} at {w}x{h}");
                assert!(placed.strip.x >= 0.0 && placed.strip.y >= TITLE_H - 0.1);
                assert!(
                    placed.body.x + placed.body.w <= w + 0.01,
                    "{space:?} {:?} at {w}x{h}",
                    placed.body
                );
                assert!(placed.body.y + placed.body.h <= h + 0.01, "{space:?}");
            }
        }
    }



    /// The showing tab is the pane's own surface with the accent line on top,
    /// and it takes the pane's cut corner. It used to be a block in a colour of
    /// its own, standing on a filled strip, which read as a button.
    #[test]
    fn the_showing_tab_wears_the_pane_s_surface_and_the_accent() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        for space in Space::ALL {
            let Some(active) = dock.slot(space).active() else {
                continue;
            };
            let (_, tab) = out
                .layout
                .placed(space)
                .tabs
                .iter()
                .find(|(view, _)| *view == active)
                .expect("the showing view has a tab");
            assert!(
                covered(&out, *tab, tab.h, out.skin.tab),
                "{space:?}: {active:?} does not carry the pane's surface"
            );
            let accent = topped(&out, *tab, ACCENT_H, out.skin.tab_accent)
                .unwrap_or_else(|| panic!("{space:?}: {active:?} has no accent line"));
            // The accent stops where the cut starts, so no line ends in a
            // corner that is not there.
            assert!(
                (accent.xywh()[2] - (tab.w - cut_of(*tab))).abs() < 0.01,
                "{space:?}: the accent runs {:?} across a {}px tab cut by {}",
                accent.xywh(),
                tab.w,
                cut_of(*tab)
            );
            // The same accent the focus edge carries: one hue for every mark
            // that says "this one", so the border follows the theme.
            assert_eq!(out.skin.tab_accent[..3], out.skin.edge_focus[..3]);
        }
    }

    /// The accent turns the corner instead of stopping in mid air: on down the
    /// cut and then down the right edge, with a hairline down the left as well.
    /// The diagonal is the heaviest stroke on the tab and the two sides are the
    /// thinnest, which is the whole shape of the border: the cut says what shape
    /// a tab is, the sides only say where it ends.
    ///
    /// Every box is read off the tab and off the border's own rectangles rather
    /// than off the constants, so a layout change moves the assertion with the
    /// drawing.
    #[test]
    fn the_showing_tab_s_border_is_bold_on_the_cut_and_thin_down_its_sides() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        let drawn = |box_: Panel| {
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                (x - box_.x).abs() < 0.01
                    && (y - box_.y).abs() < 0.01
                    && (w - box_.w).abs() < 0.01
                    && (h - box_.h).abs() < 0.01
                    && rect.rgba() == out.skin.tab_accent
            })
        };
        let mut checked = 0;
        for space in Space::ALL {
            let placed = out.layout.placed(space);
            let Some(active) = dock.slot(space).active() else {
                continue;
            };
            let (_, tab) = placed
                .tabs
                .iter()
                .find(|(view, _)| *view == active)
                .expect("the showing view has a tab");
            let accent = topped(&out, *tab, ACCENT_H, out.skin.tab_accent)
                .unwrap_or_else(|| panic!("{space:?}: {active:?} has no accent line"));
            let cut = cut_of(*tab);
            let right = tab.x + tab.w;
            // Picked up exactly where the accent stops, and stepping down to
            // where the right edge starts.
            assert!(
                (accent.xywh()[0] + accent.xywh()[2] - (right - cut)).abs() < 0.01,
                "{space:?}: the accent ends at {:?}, the cut starts at {}",
                accent.xywh(),
                right - cut
            );
            // The thin weight is read off the left side, which runs the tab's
            // whole height because there is no bottom border to stop it.
            let side = out
                .scene
                .rects
                .iter()
                .find(|rect| {
                    let [x, y, _, h] = rect.xywh();
                    (x - tab.x).abs() < 0.01
                        && (y - tab.y).abs() < 0.01
                        && (h - tab.h).abs() < 0.01
                        && rect.rgba() == out.skin.tab_accent
                })
                .unwrap_or_else(|| panic!("{space:?}: no border down the tab's left edge"));
            let thin = side.xywh()[2];
            assert!(
                (thin - TAB_EDGE_H).abs() < 0.01,
                "{space:?}: the sides are {thin} thick"
            );
            // And the bold one off the cut's first row, measured across, which
            // is the axis a stair of one-pixel rows has a thickness on.
            let first = out
                .scene
                .rects
                .iter()
                .find(|rect| {
                    let [x, y, _, h] = rect.xywh();
                    (x - (right - cut)).abs() < 0.01
                        && (y - tab.y).abs() < 0.01
                        && (h - 1.0).abs() < 0.01
                        && rect.rgba() == out.skin.tab_accent
                })
                .unwrap_or_else(|| panic!("{space:?}: the cut has no line on its first row"));
            let bold = first.xywh()[2];
            assert!(
                bold > thin + 0.01,
                "{space:?}: the cut is {bold} against sides of {thin}"
            );
            // Nothing the tab draws is heavier than its diagonal.
            assert!(
                bold >= accent.xywh()[3] - 0.01,
                "{space:?}: the accent is {:?} against a cut of {bold}",
                accent.xywh()[3]
            );
            // One row per pixel of the cut, each a pixel further right than the
            // one above it, narrowing to the hairline where the right edge takes
            // over. No two of them overlap, which is what keeps a translucent
            // border from stacking darker on its own corner.
            for row in 0..cut as usize {
                let at = row as f32;
                assert!(
                    drawn(Panel::new(
                        right - cut + at,
                        tab.y + at,
                        bold.min(cut - at),
                        1.0
                    )),
                    "{space:?}: the cut has no line on row {row}"
                );
            }
            // On down the right edge, from where the cut leaves off to the foot.
            assert!(
                drawn(Panel::new(right - thin, tab.y + cut, thin, tab.h - cut)),
                "{space:?}: the border does not run down the right edge"
            );
            assert!(
                (tab.y + tab.h - placed.body.y).abs() < 0.01,
                "{space:?}: the tab does not sit on the pane, so its foot is not the seam"
            );
            checked += 1;
        }
        assert_eq!(checked, 4, "only {checked} spaces had a showing tab");
    }

    /// No tab has a bottom border. The showing tab's border used to turn a second
    /// time and run back along its foot, and that run sits exactly on the seam
    /// where the tab meets its pane: a line there is the rule under the strip
    /// that item 12 took away, drawn a pixel higher.
    ///
    /// Every tab is walked, showing or not, and anything lying across the tab's
    /// last rows counts. A line down a side is not one: it is taller than it is
    /// wide, and the right edge is meant to reach the seam.
    #[test]
    fn no_tab_carries_a_line_along_its_foot() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &["calc.py"]);
        let tabs: Vec<Panel> = Space::ALL
            .iter()
            .flat_map(|space| out.layout.placed(*space).tabs.iter().map(|(_, tab)| *tab))
            .collect();
        assert!(tabs.len() >= 8, "only {} tabs on screen", tabs.len());
        for tab in tabs {
            let foot = tab.y + tab.h;
            for rect in &out.scene.rects {
                let [x, y, w, h] = rect.xywh();
                let inside = x >= tab.x - 0.01 && x + w <= tab.x + tab.w + 0.01;
                let across = w > h;
                let a_line = h <= 3.0;
                let on_the_last_rows = y < foot - 0.01 && y + h > foot - 3.01;
                assert!(
                    !(inside && across && a_line && on_the_last_rows),
                    "{:?} runs along the foot of the tab at {:?}",
                    rect.xywh(),
                    (tab.x, tab.y)
                );
            }
        }
    }

    /// One green for every view. The line said which pane you were on and the
    /// hue said which pane it was, which is the label's job, so nine hues on
    /// nine tabs was a harlequin strip answering a question nobody asked.
    ///
    /// Every view is walked rather than a couple of them, and the colour is read
    /// off the rectangle rather than looked up: a hue coming back for one view
    /// is exactly what this has to catch.
    #[test]
    fn every_view_carries_the_same_accent() {
        let mut state = busy_state();
        // The agent-output view only stands in a space once an agent is
        // chosen, so give it one and put it in.
        state.apply(noob_proto::Event::AgentSpawn {
            agent_id: "kid".into(),
            prompt: "look".into(),
            tools: "read".into(),
        });
        assert!(state.show_agent(1));
        let mut seen = Vec::new();
        for view in View::ALL {
            let mut dock = Dock::new();
            if view == View::Agent {
                assert!(dock.unhide(View::Agent));
            }
            assert!(dock.reveal(view), "{view:?} is in no space");
            let out = render(&state, 1400.0, 900.0, &dock, &["a.rs"]);
            let tab = Space::ALL
                .iter()
                .find_map(|space| {
                    out.layout
                        .placed(*space)
                        .tabs
                        .iter()
                        .find(|(shown, _)| *shown == view)
                        .map(|(_, panel)| *panel)
                })
                .unwrap_or_else(|| panic!("{view:?} has no tab on screen"));
            let line = out
                .scene
                .rects
                .iter()
                .find(|rect| {
                    let [x, y, _, h] = rect.xywh();
                    (x - tab.x).abs() < 0.01
                        && (y - tab.y).abs() < 0.01
                        && (h - ACCENT_H).abs() < 0.01
                })
                .unwrap_or_else(|| panic!("{view:?} has no accent line"));
            seen.push((view, line.rgba()));
        }
        assert_eq!(seen.len(), View::ALL.len());
        let skin = Skin::from(&Config::default());
        for (view, colour) in &seen {
            assert_eq!(*colour, skin.tab_accent, "{view:?} has an accent of its own");
        }
    }

    /// A tab strip is the window, not a surface. Nothing spans it: no fill, and
    /// no hairline along its foot either. Both were square rectangles, and the
    /// right end of both ran past the cut corner of the pane below.
    #[test]
    fn a_tab_strip_has_no_surface_of_its_own() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &["a.rs"]);
        for space in Space::ALL {
            let strip = out.layout.placed(space).strip;
            if strip.w < 1.0 {
                continue;
            }
            for rect in &out.scene.rects {
                let [x, y, w, h] = rect.xywh();
                let spans = x <= strip.x + 0.01 && x + w >= strip.x + strip.w - 0.01;
                let inside = y >= strip.y - 0.01 && y + h <= strip.y + strip.h + 0.01;
                assert!(
                    !(spans && inside),
                    "{space:?}: {:?} runs the width of the strip",
                    rect.xywh()
                );
            }
        }
    }

    /// Nothing is drawn in the triangle the cut takes out of a pane's top-right
    /// corner. The strip's floor sat one pixel above the pane and ran the full
    /// width, and the scrollbar started three pixels down the right edge; both
    /// drew into a corner that is not there.
    ///
    /// The cut line itself is the one thing allowed to touch it, and it lies just
    /// inside the corner rather than in it: drawn on the line at [`CUT_EDGE_H`] it
    /// clears by `cut - CUT_EDGE_H`, its own thickness short of the empty
    /// triangle, which is what a border following an edge means. Anything further
    /// in than the border is thick is still in the corner and still fails.
    #[test]
    fn nothing_is_drawn_in_the_corner_the_cut_takes_away() {
        let mut state = busy_state();
        // Enough transcript that the panes want scrollbars, which is the other
        // half of what this is checking.
        for i in 0..200 {
            state.apply(noob_proto::Event::TextDelta {
                d: format!("line {i}\n"),
            });
        }
        let out = render(&state, 1400.0, 900.0, &Dock::new(), &["calc.py"]);
        assert!(
            out.scene
                .rects
                .iter()
                .any(|rect| rect.rgba() == out.skin.scroll_thumb),
            "no scrollbar was drawn, so this proves nothing about the one that was in the corner"
        );
        for space in Space::ALL {
            let body = out.layout.placed(space).body;
            if body.w < 1.0 || body.h < 1.0 {
                continue;
            }
            let right = body.x + body.w;
            let cut = cut_of(body);
            for rect in &out.scene.rects {
                let [x, y, w, _] = rect.xywh();
                // Only what is drawn inside this pane's own corner: the
                // backdrop and the title strip are wider than the pane.
                if x < body.x - 0.01 || x + w > right + 0.01 || y < body.y - CUT || y > body.y + CUT
                {
                    continue;
                }
                // The pane's fill and outline are the shape, cut and all.
                if rect.extra()[1] > 0.0 {
                    continue;
                }
                let clear = (right - (x + w)) + (y - body.y);
                assert!(
                    clear >= cut - CUT_EDGE_H - 0.01,
                    "{space:?}: {:?} is {clear}px into a {cut}px cut",
                    rect.xywh()
                );
            }
        }
    }

    /// A pane's cut corner is a drawn edge and not only a missing one. Every
    /// other chromed box in the window is stroked all the way round, so a line
    /// already ran down their diagonal; the pane, which cannot be stroked because
    /// that would paint the top edge back on, had a corner that was a gap in its
    /// own border.
    ///
    /// In the colour the other three sides are drawn in, read off the left edge
    /// rather than looked up, so a corner in a colour of its own is caught, and
    /// heavier than they are: the diagonal is the mark that says what shape a
    /// pane is, and at the hairline the sides take it was lost among them.
    #[test]
    fn a_pane_s_cut_corner_is_bolder_than_its_other_sides() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        for space in occupied(&dock) {
            let panel = out.layout.placed(space).body;
            let cut = cut_of(panel);
            let right = panel.x + panel.w;
            let side = out
                .scene
                .rects
                .iter()
                .find(|r| {
                    let [x, y, _, h] = r.xywh();
                    (x - panel.x).abs() < 0.01
                        && (y - panel.y).abs() < 0.01
                        && (h - panel.h).abs() < 0.01
                        && r.rgba() == out.skin.edge
                })
                .unwrap_or_else(|| panic!("{space:?}: no left edge to take the colour from"));
            let hairline = side.xywh()[2];
            let at = |box_: [f32; 4]| {
                out.scene
                    .rects
                    .iter()
                    .any(|r| r.xywh() == box_ && r.rgba() == side.rgba())
            };
            // The diagonal's weight is read across, off its first row.
            let first = out
                .scene
                .rects
                .iter()
                .find(|r| {
                    let [x, y, _, h] = r.xywh();
                    (x - (right - cut)).abs() < 0.01
                        && (y - panel.y).abs() < 0.01
                        && (h - 1.0).abs() < 0.01
                        && r.rgba() == side.rgba()
                })
                .unwrap_or_else(|| panic!("{space:?}: the cut has no line on its first row"));
            let bold = first.xywh()[2];
            assert!(
                bold > hairline + 0.01,
                "{space:?}: the cut is {bold} against sides of {hairline}"
            );
            // From where a top edge would have stopped down to the row the right
            // edge starts on, one row per pixel and no two of them overlapping:
            // `skin.edge` is translucent, and a stair of overlapping squares
            // composites darker on the corner than on the sides it meets.
            for row in 0..cut as usize {
                let a = row as f32;
                assert!(
                    at([right - cut + a, panel.y + a, bold.min(cut - a), 1.0]),
                    "{space:?}: the cut has no line on row {row}"
                );
            }
            // And the last row is the hairline the right edge starts with, so
            // the two are one border and not two.
            assert!(
                at([right - hairline, panel.y + cut, hairline, panel.h - cut]),
                "{space:?}: the cut line ends nowhere near the right edge"
            );
        }
    }

    /// Every tab takes the same cut the panes take, whichever strip it is in.
    #[test]
    fn every_tab_is_cut_the_way_a_pane_is() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &["calc.py"]);
        let boxes: Vec<Panel> = Space::ALL
            .iter()
            .flat_map(|space| out.layout.placed(*space).tabs.iter().map(|(_, tab)| *tab))
            .collect();
        assert!(boxes.len() >= 8, "only {} tabs on screen", boxes.len());
        for tab in boxes {
            let cut = out
                .scene
                .rects
                .iter()
                .find(|rect| {
                    let [x, y, w, h] = rect.xywh();
                    (x - tab.x).abs() < 0.01
                        && (y - tab.y).abs() < 0.01
                        && (w - tab.w).abs() < 0.01
                        && (h - tab.h).abs() < 0.01
                })
                .unwrap_or_else(|| panic!("no surface under the tab at {:?}", (tab.x, tab.y)));
            assert_eq!(cut.extra()[1], cut_of(tab), "{:?}", cut.xywh());
            assert_eq!(cut.extra()[2], Rect::TOP_RIGHT as f32, "{:?}", cut.xywh());
        }
    }

    /// A tab that is not showing is the same tab with less weight: the same
    /// surface at a lower alpha, a dimmer label, and no accent line. It used to
    /// have no fill at all and a rule beside it, which only worked while the
    /// strip behind it was a surface of its own.
    #[test]
    fn a_tab_that_is_not_showing_is_the_same_tab_with_less_weight() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        assert!(out.skin.tab_idle[3] < out.skin.tab[3]);
        let mut checked = 0;
        for space in Space::ALL {
            let active = dock.slot(space).active();
            for (view, tab) in &out.layout.placed(space).tabs {
                if Some(*view) == active {
                    continue;
                }
                checked += 1;
                assert!(
                    covered(&out, *tab, tab.h, out.skin.tab_idle),
                    "{view:?} is not drawn at the idle weight"
                );
                assert!(
                    topped(&out, *tab, ACCENT_H, out.skin.tab_accent).is_none(),
                    "{view:?} has an accent line and is not showing"
                );
                // And nothing of the border that accent turns into either: the
                // showing tab's cut, right edge and foot are the accent
                // continued, so a tab wearing them without the accent would be
                // saying it is showing at half volume.
                assert!(
                    !out.scene.rects.iter().any(|rect| {
                        let [x, y, w, h] = rect.xywh();
                        rect.rgba() == out.skin.tab_accent
                            && x >= tab.x - 0.01
                            && y >= tab.y - 0.01
                            && x + w <= tab.x + tab.w + 0.01
                            && y + h <= tab.y + tab.h + 0.01
                    }),
                    "{view:?} carries the showing tab's border and is not showing"
                );
                let label = out
                    .scene
                    .texts
                    .iter()
                    .find(|text| tab.contains(text.at.x, text.at.y))
                    .unwrap_or_else(|| panic!("{view:?} has no label"));
                assert_eq!(label.color, out.skin.dim, "{view:?} is not dimmed");
            }
        }
        assert!(checked >= 4, "only {checked} tabs were not showing");
    }













    /// A name too long for the column loses its parent directory first and its
    /// own tail second. The row's type icon already says what the extension
    /// would, so the front of the name is what carries the identity.
    #[test]
    fn a_name_that_does_not_fit_loses_its_directory_then_its_tail() {
        assert_eq!(fit_name("crates/noob/src/mod.rs", 20), "src/mod.rs");
        assert_eq!(fit_name("crates/noob/src/mod.rs", 10), "src/mod.rs");
        assert_eq!(fit_name("crates/noob/src/mod.rs", 9), "mod.rs");
        assert_eq!(fit_name("src/calc.py", 7), "calc.py");
        assert_eq!(fit_name("src/averyverylongname.rs", 8), "averyve\u{2026}");
        for cols in 1..24 {
            let cut = fit_name("crates/noob/src/somelongmodule.rs", cols);
            assert!(
                cut.chars().count() <= cols,
                "{cut:?} is wider than the {cols} columns it was given"
            );
        }
    }

    /// Every click resolves in one place, so what a region looks like and what
    /// it does can never come apart.
    #[test]
    fn every_tab_is_hit_where_it_is_drawn() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs", "b.md"]);
        let middle = |p: Panel| (p.x + p.w * 0.5, p.y + p.h * 0.5);
        for space in Space::ALL {
            let placed = out.layout.placed(space);
            for (view, panel) in &placed.tabs {
                let (x, y) = middle(*panel);
                assert_eq!(
                    out.layout.hit(x, y),
                    Some(Hit::Tab(*view, space)),
                    "{view:?} in {space:?}"
                );
            }
        }
        for (index, panel) in &out.layout.file_rows {
            let (x, y) = middle(*panel);
            let hit = out.layout.hit(x, y).expect("a row of the file list");
            assert_eq!(hit, Hit::File(*index, Space::BottomRight));
            // And it still names a space, so a tab dropped here lands.
            assert_eq!(hit.space(), Some(Space::BottomRight));
        }
        for (panel, hit) in [
            (out.layout.close, Hit::Close),
            (out.layout.maximize, Hit::Maximize),
            (out.layout.minimize, Hit::Minimize),
            (out.layout.input, Hit::Input),
        ] {
            let (x, y) = middle(panel);
            assert_eq!(out.layout.hit(x, y), Some(hit));
        }
    }

    /// Every point inside a pane's body or its strip is hit as that pane,
    /// wherever the pane sits on the grid and however many cells it covers.
    ///
    /// The pointer, not the drop: a drop below the strips is read off the grid
    /// rather than off the pane that happens to be drawn there, which is what
    /// lets a pane covering two cells be dropped into either half of itself.
    #[test]
    fn every_point_in_a_space_is_hit_as_that_space() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        // How far in from an edge a point has to be to be in the pane rather
        // than on a divider: the band a divider is grabbed by is [`GRAB`] wider
        // than the gap it stands in on each side, so the outermost few pixels of
        // a pane beside one belong to the divider.
        let in_ = GRAB + 4.0;
        for space in occupied(&dock) {
            let placed = out.layout.placed(space);
            for point in [
                (placed.body.x + in_, placed.body.y + in_),
                (
                    placed.body.x + placed.body.w - in_,
                    placed.body.y + placed.body.h - in_,
                ),
                (placed.strip.x + placed.strip.w - TAB_H - 4.0, placed.strip.y + 4.0),
            ] {
                let hit = out.layout.hit(point.0, point.1).expect("a hit");
                assert_eq!(hit.space(), Some(space), "{point:?} in {space:?}: {hit:?}");
            }
        }
    }

    /// The arrangement drives the layout: a view dragged elsewhere is drawn
    /// elsewhere, and its old space keeps working.
    #[test]
    fn a_moved_view_is_drawn_in_its_new_space() {
        let mut dock = Dock::new();
        dock.move_view(View::Session, Space::TopLeft);
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        let left: Vec<View> = out
            .layout
            .placed(Space::TopLeft)
            .tabs
            .iter()
            .map(|(v, _)| *v)
            .collect();
        assert!(left.contains(&View::Session), "{left:?}");
        assert!(left.contains(&View::Output), "{left:?}");
        let top: Vec<View> = out
            .layout
            .placed(Space::TopRight)
            .tabs
            .iter()
            .map(|(v, _)| *v)
            .collect();
        assert!(!top.contains(&View::Session), "{top:?}");
    }

    /// An emptied space gives its room away rather than leaving a hole.
    #[test]
    fn an_empty_space_gives_its_room_to_its_neighbour() {
        let full = Dock::new();
        let mut emptied = Dock::new();
        for view in [
            View::Activity,
            View::Plan,
            View::Agents,
            View::Hardware,
            View::Context,
            View::Session,
        ] {
            emptied.move_view(view, Space::BottomRight);
        }
        let a = render(&busy_state(), 1200.0, 800.0, &full, &["a.rs"]);
        let b = render(&busy_state(), 1200.0, 800.0, &emptied, &["a.rs"]);
        assert_eq!(b.layout.placed(Space::TopRight).strip.w, 0.0);
        assert!(
            b.layout.placed(Space::BottomRight).body.h
                > a.layout.placed(Space::BottomRight).body.h,
            "the other space grew"
        );
    }

    /// With nothing on the left, the right column takes the whole width rather
    /// than leaving half the window empty.
    #[test]
    fn an_empty_left_column_hands_the_width_over() {
        let mut dock = Dock::new();
        for view in [View::Output, View::Activity, View::Files, View::Plan] {
            dock.move_view(view, Space::TopRight);
        }
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        assert_eq!(out.layout.placed(Space::TopLeft).strip.w, 0.0);
        let top = out.layout.placed(Space::TopRight);
        assert!(top.body.w > 1000.0, "{:?}", top.body);
    }


    /// Item 16: the two ratios decide where the dividers fall, and the spaces
    /// still cover the body between them with one gap in each direction.
    #[test]
    fn the_two_ratios_decide_where_the_dividers_fall() {
        let dock = Dock::new();
        for (left_width, top_height) in [(0.3, 0.3), (crate::config::LEFT_WIDTH, crate::config::TOP_HEIGHT), (0.7, 0.7)] {
            let layout = Layout::compute(1400.0, 900.0, &split_shape(&dock, left_width, top_height));
            let body = layout.column_divider[0].track;
            let room = body.w - GAP;
            let (left_w, _) = box_of(&layout, Space::TopLeft);
            let (right_w, _) = box_of(&layout, Space::TopRight);
            assert!(
                (left_w - room * left_width).abs() <= 1.0,
                "{left_width}: the left column is {left_w} of {room}"
            );
            assert!(
                (left_w + right_w + GAP - body.w).abs() <= 1.0,
                "{left_w} + {right_w} does not fill {}",
                body.w
            );

            // The right column's own line, which is the one its two spaces
            // break at.
            let right = layout.row_divider[1].track;
            let room = right.h - GAP;
            let (_, top_h) = box_of(&layout, Space::TopRight);
            let (_, bottom_h) = box_of(&layout, Space::BottomRight);
            assert!(
                (top_h - room * top_height).abs() <= 1.0,
                "{top_height}: the top space is {top_h} of {room}"
            );
            assert!(
                (top_h + bottom_h + GAP - right.h).abs() <= 1.0,
                "{top_h} + {bottom_h} does not fill {}",
                right.h
            );

            // And the band the pointer grabs straddles the gap it stands in.
            assert_eq!(layout.column_divider[0].band.w, GAP + GRAB * 2.0);
            assert!((layout.column_divider[0].band.x + GRAB - left_w - body.x).abs() <= 0.01);
            assert_eq!(layout.row_divider[1].band.h, GAP + GRAB * 2.0);
            assert!((layout.row_divider[1].band.y + GRAB - right.y - top_h).abs() <= 0.01);
            // One line down the whole grid and one inside each half of it: the
            // second vertical line is not there while the grid reads in columns.
            assert!(!layout.column_divider[1].live());
        }
    }

    /// The pointer puts a divider where the pointer is: the ratio a drag reads
    /// off a position, laid out again, puts the gap back under that position.
    #[test]
    fn a_dragged_divider_lands_under_the_pointer() {
        let dock = Dock::new();
        let layout = Layout::compute(1400.0, 900.0, &split_shape(&dock, crate::config::LEFT_WIDTH, crate::config::TOP_HEIGHT));
        let body = layout.column_divider[0].track;
        let right = layout.row_divider[1].track;

        for x in [body.x + 400.0, body.x + 700.0, body.x + 1000.0] {
            let ratio = layout.column_ratio_at(0, x);
            let moved =
                Layout::compute(1400.0, 900.0, &halves_shape(&dock, [ratio; 2], [0.46; 2]));
            let gap = moved.placed(Space::TopLeft).strip.x + moved.placed(Space::TopLeft).strip.w;
            assert!((gap + GAP * 0.5 - x).abs() <= 1.0, "{x} put the gap at {gap}");
        }
        // And the half that was dragged is the half that moved: the right
        // column's line follows the pointer, the left column's stays put.
        for y in [right.y + 200.0, right.y + 400.0, right.y + 600.0] {
            let ratio = layout.row_ratio_at(1, y);
            let moved = Layout::compute(
                1400.0,
                900.0,
                &halves_shape(&dock, [0.54; 2], [crate::config::TOP_HEIGHT, ratio]),
            );
            let top = moved.placed(Space::TopRight);
            let gap = top.body.y + top.body.h;
            assert!((gap + GAP * 0.5 - y).abs() <= 1.0, "{y} put the gap at {gap}");
            assert_eq!(
                moved.grid[Space::TopLeft.index()],
                layout.grid[Space::TopLeft.index()],
                "{y} moved the left column as well"
            );
        }
    }

    /// A drag thrown past either end of the window stops at the floor. Nothing
    /// collapses: the smallest a space goes is a tab strip and enough pane to
    /// read under it.
    #[test]
    fn a_divider_dragged_past_the_end_stops_at_the_floor() {
        let dock = Dock::new();
        let layout = Layout::compute(1400.0, 900.0, &split_shape(&dock, crate::config::LEFT_WIDTH, crate::config::TOP_HEIGHT));
        let column_floor = layout.column_divider[0].floor;
        let row_floor = layout.row_divider[1].floor;
        assert!(column_floor > 0.0 && row_floor > 0.0);

        for x in [-4000.0, -1.0, 700.0, 1401.0, 9000.0] {
            let ratio = layout.column_ratio_at(0, x);
            let moved = Layout::compute(1400.0, 900.0, &split_shape(&dock, ratio, crate::config::TOP_HEIGHT));
            let (left_w, _) = box_of(&moved, Space::TopLeft);
            let (right_w, _) = box_of(&moved, Space::TopRight);
            assert!(left_w >= column_floor, "{x}: the left column is {left_w}");
            assert!(right_w >= column_floor, "{x}: the right column is {right_w}");
        }
        for y in [-4000.0, -1.0, 500.0, 901.0, 9000.0] {
            let ratio = layout.row_ratio_at(1, y);
            let moved = Layout::compute(1400.0, 900.0, &split_shape(&dock, crate::config::LEFT_WIDTH, ratio));
            let (_, top_h) = box_of(&moved, Space::TopRight);
            let (_, bottom_h) = box_of(&moved, Space::BottomRight);
            assert!(top_h >= row_floor, "{y}: the top space is {top_h}");
            assert!(bottom_h >= row_floor, "{y}: the bottom space is {bottom_h}");
        }

        // A ratio out of a settings file nobody clamped is held the same way.
        for ratio in [0.0, 1.0, -5.0, 12.0] {
            let moved = Layout::compute(1400.0, 900.0, &split_shape(&dock, ratio, ratio));
            assert!(box_of(&moved, Space::TopLeft).0 >= column_floor, "{ratio}");
            assert!(box_of(&moved, Space::TopRight).0 >= column_floor, "{ratio}");
            assert!(box_of(&moved, Space::TopRight).1 >= row_floor, "{ratio}");
            assert!(box_of(&moved, Space::BottomRight).1 >= row_floor, "{ratio}");
        }
    }

    /// A window with no room for two floors and the gap between them splits down
    /// the middle instead of giving one space everything and the other nothing.
    #[test]
    fn a_window_too_small_for_two_floors_splits_down_the_middle() {
        let dock = Dock::new();
        // 320 wide leaves about 300 for two columns that want 210 each.
        let layout = Layout::compute(320.0, 240.0, &split_shape(&dock, 0.9, 0.9));
        let (left_w, _) = box_of(&layout, Space::TopLeft);
        let (right_w, _) = box_of(&layout, Space::TopRight);
        assert!(left_w < layout.column_divider[0].floor, "the floor still fits");
        assert!((left_w - right_w).abs() <= 1.0, "{left_w} against {right_w}");
        assert!(left_w > 1.0 && right_w > 1.0, "a column collapsed");

        // The same downward: a right column with no room for two floors.
        let short = Layout::compute(1400.0, 180.0, &split_shape(&dock, crate::config::LEFT_WIDTH, 0.9));
        let (_, top_h) = box_of(&short, Space::TopRight);
        let (_, bottom_h) = box_of(&short, Space::BottomRight);
        assert!((top_h - bottom_h).abs() <= 1.0, "{top_h} against {bottom_h}");
        assert!(top_h > 1.0 && bottom_h > 1.0, "a space collapsed");
    }

    /// The band is wider than the gap it stands in, or a six pixel line is a
    /// target nobody can hit. It wins against the pane it reaches into on its
    /// right; on its left the pane's own scroll track wins back the pixels it
    /// is drawn in, because a bar that loses to an invisible band is a bar
    /// that cannot be dragged at all.
    #[test]
    fn a_divider_is_grabbed_by_a_band_wider_than_the_gap() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &[]);
        let band = out.layout.column_divider[0].band;
        assert!(band.w > GAP, "the band is no wider than the gap");
        let y = band.y + TAB_H + 20.0;
        for x in [band.x + band.w * 0.5, band.x + band.w - 0.5] {
            assert_eq!(out.layout.hit(x, y), Some(Hit::ColumnDivider(0)), "at {x}");
        }
        assert_eq!(
            out.layout.hit(band.x + 0.5, y),
            Some(Hit::Scrollbar(Space::TopLeft)),
            "the reach into the left pane is the pane's own track"
        );
        assert_eq!(
            out.layout.hit(band.x + band.w + 1.0, y),
            Some(Hit::Body(Space::TopRight))
        );

        // The right column's line, which is the half of the grid that is split
        // in the arrangement the window opens with.
        let band = out.layout.row_divider[1].band;
        assert!(band.h > GAP);
        let x = band.x + band.w * 0.5;
        for y in [band.y + 0.5, band.y + band.h * 0.5, band.y + band.h - 0.5] {
            assert_eq!(out.layout.hit(x, y), Some(Hit::RowDivider(1)), "at {y}");
        }
        assert_eq!(
            out.layout.hit(x, band.y - 1.0),
            Some(Hit::Body(Space::TopRight))
        );
        assert_eq!(
            out.layout.hit(x, band.y + band.h + 1.0),
            Some(Hit::Body(Space::BottomRight))
        );

        // A tab is a smaller target inside the same band, so it still takes the
        // press: the band reaches a few pixels into the strips as well.
        let tab = out.layout.placed(Space::TopRight).tabs[0];
        let (tx, ty) = middle(tab.1);
        assert_eq!(out.layout.hit(tx, ty), Some(Hit::Tab(tab.0, Space::TopRight)));
        // A divider is a drop target of its own: a tab let go on the line
        // between two cells takes both of them. It is the same band the pointer
        // grabs the divider by, read for a drop rather than for a press.
        assert_eq!(
            out.layout.landing(x, band.y + 1.0),
            Landing::Span(Space::TopRight, Space::BottomRight)
        );
    }

    /// A divider beside an empty space has nothing to divide, so it is not
    /// there. The space that gave its room away is what makes it so, and the
    /// remaining spaces still fill the window.
    #[test]
    fn a_divider_beside_an_empty_or_folded_space_is_not_there() {
        // Nothing on the left: one column, so no divider between two of them.
        let mut dock = Dock::new();
        for view in [View::Output, View::Activity, View::Files, View::Plan] {
            dock.move_view(view, Space::TopRight);
        }
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        assert!(!out.layout.column_divider[0].live());
        assert!(out.layout.row_divider[1].live(), "the two right spaces are still there");
        assert!(box_of(&out.layout, Space::TopRight).0 > 1000.0, "the width was handed over");

        // Nothing in the bottom right: one space in that column, so no divider
        // across it, and the vertical one is still there.
        let mut dock = Dock::new();
        dock.move_view(View::Agents, Space::TopRight);
        dock.move_view(View::Plan, Space::TopLeft);
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        assert!(out.layout.column_divider[0].live());
        assert!(out.layout.row_divider.iter().all(|line| !line.live()));
        let (_, top_h) = box_of(&out.layout, Space::TopRight);
        let body = out.layout.column_divider[0].track;
        assert!((top_h - body.h).abs() <= 1.0, "the height was handed over");

        // A folded space is already as short as it goes: the fold owns the
        // height until it is opened, so there is nothing to drag.
        let mut dock = Dock::new();
        dock.slot_mut(Space::TopRight).folded = true;
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        assert!(!out.layout.row_divider[1].live());
        assert!(out.layout.column_divider[0].live(), "the columns can still be moved");

        // And with no divider under it, the point that would have been on one
        // belongs to the pane again.
        let mut dock = Dock::new();
        for view in [View::Output, View::Activity, View::Files, View::Plan] {
            dock.move_view(view, Space::TopRight);
        }
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        let full = render(&busy_state(), 1200.0, 800.0, &Dock::new(), &[]);
        let band = full.layout.column_divider[0].band;
        let (x, y) = (band.x + band.w * 0.5, band.y + TAB_H + 20.0);
        assert_eq!(full.layout.hit(x, y), Some(Hit::ColumnDivider(0)));
        assert_eq!(out.layout.hit(x, y), Some(Hit::Body(Space::TopRight)));
    }

    /// The three shapes with no panes in them have no dividers either. A band
    /// left behind by a shape change is a press that lands on something nobody
    /// can see.
    #[test]
    fn no_divider_survives_a_shape_change() {
        let dock = Dock::new();
        let open = Layout::compute(1200.0, 800.0, &split_shape(&dock, crate::config::LEFT_WIDTH, crate::config::TOP_HEIGHT));
        assert!(open.column_divider[0].live() && open.row_divider[1].live());
        let band = open.column_divider[0].band;
        let (x, y) = (band.x + band.w * 0.5, band.y + TAB_H + 20.0);

        let picker = a_picker(&["src", "docs"], &[]);
        let panel = a_settings_panel(&Config::default());
        for (what, shape) in [
            ("shaded", Shape { shaded: true, ..split_shape(&dock, crate::config::LEFT_WIDTH, crate::config::TOP_HEIGHT) }),
            ("picking", Shape { picker: Some(&picker), ..split_shape(&dock, crate::config::LEFT_WIDTH, crate::config::TOP_HEIGHT) }),
            ("settings", Shape { settings: Some(&panel), ..split_shape(&dock, crate::config::LEFT_WIDTH, crate::config::TOP_HEIGHT) }),
        ] {
            let layout = Layout::compute(1200.0, 800.0, &shape);
            let lines = layout.column_divider.iter().chain(layout.row_divider.iter());
            assert!(lines.map(|line| line.live()).all(|live| !live), "{what}");
            assert!(
                !matches!(
                    layout.hit(x, y),
                    Some(Hit::ColumnDivider(_) | Hit::RowDivider(_))
                ),
                "{what} still hits a divider"
            );
        }
    }

    /// Item 17: a drag puts a full green transparent box over the space the drop
    /// would land in, and the tab follows the pointer. Both on the floating
    /// layer, or the feedback is painted under the pane's own text, which is the
    /// version that could not be seen.
    #[test]
    fn a_dragged_tab_boxes_its_target_space_in_green_on_the_overlay() {
        let dock = Dock::new();
        let plain = render(&busy_state(), 1200.0, 800.0, &dock, &["a.rs"]);
        assert!(
            plain.scene.over_rects.is_empty(),
            "something floats over a window with nothing being dragged"
        );

        let dragging = render_with(
            &busy_state(),
            1200.0,
            800.0,
            &dock,
            &["a.rs"],
            &Monitor::new(),
            Some(Drag {
                view: View::Activity,
                at: (400.0, 500.0),
                landing: Landing::In(Space::TopLeft, None),
            }),
        );
        // The box covers the whole space: its tab strip and its pane.
        let placed = dragging.layout.placed(Space::TopLeft);
        let want = [
            placed.strip.x,
            placed.strip.y,
            placed.strip.w,
            placed.strip.h + placed.body.h,
        ];
        let box_ = dragging
            .scene
            .over_rects
            .iter()
            .find(|rect| rect.rgba() == dragging.skin.drop_target)
            .expect("no green box over the target space");
        assert_eq!(box_.xywh(), want, "the box is not the space");
        assert!(
            box_.rgba()[3] < dragging.skin.panel[3],
            "the box is more solid than the pane it covers: {:?}",
            box_.rgba()
        );
        // Nothing of it in the base layer, where the pane's glyphs would be
        // painted over it.
        assert!(
            !dragging
                .scene
                .rects
                .iter()
                .any(|rect| rect.rgba() == dragging.skin.drop_target),
            "the box is in the base layer, under the pane's text"
        );
        // There is pane text under it, or this proves nothing.
        assert!(
            text_over(&dragging.scene.texts, Panel::new(want[0], want[1], want[2], want[3])),
            "nothing is written under the box"
        );

        // The tab itself follows the pointer, over the box.
        let ghost = dragging
            .scene
            .over_rects
            .iter()
            .map(|r| r.xywh())
            .find(|[x, y, w, h]| {
                *x < 400.0 && *x + *w > 400.0 && *y < 500.0 && *y + *h > 500.0 && *h <= TAB_H + 1.0
            });
        assert!(ghost.is_some(), "no ghost under the pointer");

        // And the space that is not being dropped on is not boxed.
        for space in [Space::TopRight, Space::BottomRight] {
            let placed = dragging.layout.placed(space);
            assert!(
                !dragging.scene.over_rects.iter().any(|rect| {
                    let [x, y, ..] = rect.xywh();
                    rect.rgba() == dragging.skin.drop_target
                        && (x - placed.strip.x).abs() < 0.01
                        && (y - placed.strip.y).abs() < 0.01
                }),
                "{space:?} is boxed too"
            );
        }
        // The pane's own edges are the ordinary ones. The lit outline this
        // replaced could not close around a pane with no top edge.
        assert!(
            !dragging
                .scene
                .rects
                .iter()
                .any(|rect| rect.rgba() == dragging.skin.edge_focus
                    && rect.xywh()[0] == placed.body.x
                    && rect.xywh()[1] == placed.body.y),
            "the target pane is still outlined in the focus colour"
        );
    }

    /// The grid is the two ratios, and it tiles the room the panes share with
    /// one gap each way. Four cells, whatever is standing in them.
    #[test]
    fn the_four_cells_tile_the_room_the_panes_share() {
        let dock = Dock::new();
        for (w, h) in [(1400.0, 900.0), (700.0, 460.0), (2200.0, 1400.0)] {
            for (left_width, top_height) in [(0.3, 0.3), (crate::config::LEFT_WIDTH, crate::config::TOP_HEIGHT), (0.7, 0.72)] {
                let layout =
                    Layout::compute(w, h, &split_shape(&dock, left_width, top_height));
                let cells = layout.grid;
                let at = |space: Space| cells[space.index()];
                let what = format!("{w}x{h} at {left_width},{top_height}");
                for space in Space::ALL {
                    assert!(at(space).w > 1.0 && at(space).h > 1.0, "{space:?} {what}");
                }
                // One gap between the columns and one between the rows, and the
                // cells of a column are the same width as each other.
                assert_eq!(at(Space::TopLeft).w, at(Space::BottomLeft).w, "{what}");
                assert_eq!(at(Space::TopRight).w, at(Space::BottomRight).w, "{what}");
                assert_eq!(at(Space::TopLeft).h, at(Space::TopRight).h, "{what}");
                assert_eq!(at(Space::BottomLeft).h, at(Space::BottomRight).h, "{what}");
                let gap_x = at(Space::TopRight).x - (at(Space::TopLeft).x + at(Space::TopLeft).w);
                let gap_y = at(Space::BottomLeft).y - (at(Space::TopLeft).y + at(Space::TopLeft).h);
                assert!((gap_x - GAP).abs() < 0.01, "{gap_x} {what}");
                assert!((gap_y - GAP).abs() < 0.01, "{gap_y} {what}");
                // And the whole grid is the box the panes share: the window
                // under the title strip, less the prompt and the margin.
                let whole = around(at(Space::TopLeft), at(Space::BottomRight));
                assert!((whole.x - GAP).abs() < 0.01, "{whole:?} {what}");
                assert!((whole.x + whole.w - (w - GAP)).abs() < 0.01, "{whole:?} {what}");
                assert!(
                    (whole.y + whole.h - (layout.input.y - GAP * 2.0)).abs() < 0.01,
                    "{whole:?} {what}"
                );
            }
        }
    }

    /// Every pane is the box around the cells it covers, so the arrangement on
    /// screen is exactly the arrangement the dock describes. This is what makes
    /// the drop preview and the drop agree: both read the same cells.
    #[test]
    fn every_pane_is_the_cells_it_covers() {
        let mut docks = vec![Dock::new()];
        let mut split = Dock::new();
        split.move_view(View::Hardware, Space::BottomLeft);
        docks.push(split.clone());
        let mut rows = Dock::new();
        rows.span_view(View::Files, Space::TopLeft, Space::TopRight);
        docks.push(rows);
        let mut one = Dock::new();
        for view in View::ALL {
            if view != View::Output {
                one.hide(view);
            }
        }
        docks.push(one);
        for dock in docks {
            let layout = Layout::compute(1400.0, 900.0, &shape(&dock, &[]));
            let cover = dock.cover();
            for space in Space::ALL {
                let want = Space::ALL
                    .into_iter()
                    .filter(|cell| cover[cell.index()] == Some(space))
                    .fold(nowhere(), |box_, cell| {
                        around(box_, layout.grid[cell.index()])
                    });
                let placed = layout.placed(space);
                let got = Panel::new(
                    placed.strip.x,
                    placed.strip.y,
                    placed.strip.w,
                    placed.strip.h + placed.body.h,
                );
                assert_eq!(got, want, "{space:?} in {dock:?}");
            }
        }
    }

    /// Each half of the grid breaks where its own line was left. One ratio for
    /// both halves is what made a left column split 30/70 and a right one split
    /// 70/30 impossible, and it is the thing this buys.
    #[test]
    fn each_half_of_the_grid_breaks_where_its_own_line_was_left() {
        let dock = Dock::new();
        let layout =
            Layout::compute(1400.0, 900.0, &halves_shape(&dock, [crate::config::LEFT_WIDTH; 2], [0.3, 0.7]));
        let at = |space: Space| layout.grid[space.index()];
        // What the two cells of a column share, gap included, is what a ratio is
        // a fraction of.
        let room = at(Space::TopLeft).h + at(Space::BottomLeft).h;
        assert!(
            (at(Space::TopLeft).h - room * 0.3).abs() <= 1.0,
            "the left column is {:?} of {room}",
            at(Space::TopLeft).h
        );
        assert!(
            (at(Space::TopRight).h - room * 0.7).abs() <= 1.0,
            "the right column is {:?} of {room}",
            at(Space::TopRight).h
        );
        assert!(
            at(Space::TopRight).h - at(Space::TopLeft).h > 100.0,
            "the two columns still break together"
        );
        // Both columns still fill the body, and the width is untouched: one
        // line runs the whole way down, so the columns line up as they did.
        for column in 0..2 {
            let (top, bottom) = (Space::at(0, column), Space::at(1, column));
            assert!((at(top).h + at(bottom).h - room).abs() <= 1.0, "{column}");
            assert_eq!(at(top).w, at(bottom).w, "{column}");
            assert_eq!(at(top).x, at(bottom).x, "{column}");
        }
    }

    /// The two lines are dragged apart: each band says which half it belongs to,
    /// each ratio comes off that half's own track, and moving one leaves the
    /// other exactly where it was.
    #[test]
    fn the_two_halves_are_dragged_apart_and_neither_moves_the_other() {
        let mut dock = Dock::new();
        dock.move_view(View::Hardware, Space::BottomLeft);
        let start = Layout::compute(1400.0, 900.0, &split_shape(&dock, crate::config::LEFT_WIDTH, crate::config::TOP_HEIGHT));
        assert!(
            start.row_divider[0].live() && start.row_divider[1].live(),
            "both columns are split, so both have a line"
        );
        for half in 0..2 {
            let band = start.row_divider[half].band;
            let (x, y) = (band.x + band.w * 0.5, band.y + band.h * 0.5);
            assert_eq!(start.hit(x, y), Some(Hit::RowDivider(half)), "half {half}");
        }

        // The left column's line dragged up and the right column's down.
        let track = start.row_divider[0].track;
        let (up, down) = (track.y + track.h * 0.3, track.y + track.h * 0.7);
        let ratios = [start.row_ratio_at(0, up), start.row_ratio_at(1, down)];
        let moved = Layout::compute(1400.0, 900.0, &halves_shape(&dock, [crate::config::LEFT_WIDTH; 2], ratios));
        let line_of = |layout: &Layout, space: Space| {
            let cell = layout.grid[space.index()];
            cell.y + cell.h + GAP * 0.5
        };
        assert!((line_of(&moved, Space::TopLeft) - up).abs() <= 1.0);
        assert!((line_of(&moved, Space::TopRight) - down).abs() <= 1.0);

        // And with only the right column's ratio changed, the left column's
        // cells are the ones it started with, to the pixel.
        let one = Layout::compute(
            1400.0,
            900.0,
            &halves_shape(&dock, [crate::config::LEFT_WIDTH; 2], [crate::config::TOP_HEIGHT, ratios[1]]),
        );
        for space in [Space::TopLeft, Space::BottomLeft] {
            assert_eq!(
                one.grid[space.index()],
                start.grid[space.index()],
                "{space:?} moved with the other column"
            );
        }
        assert_eq!(
            one.grid[Space::TopRight.index()],
            moved.grid[Space::TopRight.index()],
            "the right column did not take its own drag"
        );
    }

    /// A drop near a line is read off the line of the half the pointer is over.
    ///
    /// The quiet one. Reading the first cell's line wherever the pointer is
    /// still compiles and still answers, and with the two columns breaking at
    /// heights of their own it answers with the wrong pair.
    #[test]
    fn a_drop_reads_the_line_of_the_half_the_pointer_is_over() {
        let mut dock = Dock::new();
        dock.move_view(View::Hardware, Space::BottomLeft);
        let layout =
            Layout::compute(1400.0, 900.0, &halves_shape(&dock, [crate::config::LEFT_WIDTH; 2], [0.3, 0.7]));
        let at = |space: Space| layout.grid[space.index()];
        let line_of = |space: Space| at(space).y + at(space).h + GAP * 0.5;
        let (left_line, right_line) = (line_of(Space::TopLeft), line_of(Space::TopRight));
        assert!(
            right_line - left_line > SPAN_BAND * 4.0,
            "the two lines are not far enough apart to tell the readings apart"
        );
        let over = |space: Space| at(space).x + at(space).w * 0.5;
        let (over_left, over_right) = (over(Space::TopLeft), over(Space::TopRight));

        // On the left column's line the left pair is the answer, and at the same
        // height the right column is not near a line at all.
        assert_eq!(
            layout.landing(over_left, left_line),
            Landing::Span(Space::TopLeft, Space::BottomLeft)
        );
        assert_eq!(
            layout.landing(over_right, left_line),
            Landing::In(Space::TopRight, None),
            "the right column was read off the left column's line"
        );
        // And the other way round on the right column's line.
        assert_eq!(
            layout.landing(over_right, right_line),
            Landing::Span(Space::TopRight, Space::BottomRight)
        );
        assert_eq!(
            layout.landing(over_left, right_line),
            Landing::In(Space::BottomLeft, None)
        );

        // And with the two columns out of step there is still no hole: every
        // point in the room the panes share names a cell or a pair, including
        // the two gaps, which are no longer at the same height.
        let whole = around(at(Space::TopLeft), at(Space::BottomRight));
        let mut x = whole.x;
        while x < whole.x + whole.w {
            let mut y = whole.y + TAB_H + 1.0;
            while y < whole.y + whole.h {
                assert!(
                    matches!(layout.landing(x, y), Landing::In(..) | Landing::Span(..)),
                    "nothing at {x},{y}"
                );
                y += 7.0;
            }
            x += 11.0;
        }
    }

    /// Turned the other way round, the two lines inside the halves are the
    /// vertical ones, one per row, and the single line across the grid is the
    /// horizontal one. The transpose is what keeps this one rule.
    #[test]
    fn the_two_lines_inside_the_halves_turn_with_the_grid() {
        let mut dock = Dock::new();
        // A span across a row turns the grid; the cells it emptied are filled
        // again so both rows have two panes to divide.
        assert!(dock.span_view(View::Files, Space::TopLeft, Space::TopRight));
        assert!(dock.move_view(View::Session, Space::TopRight));
        assert!(dock.move_view(View::Files, Space::BottomLeft));
        assert!(dock.move_view(View::Hardware, Space::BottomRight));
        assert!(dock.rows_first(), "a tab moved does not turn the grid back");

        let layout =
            Layout::compute(1400.0, 900.0, &halves_shape(&dock, [0.3, 0.7], [crate::config::TOP_HEIGHT; 2]));
        let at = |space: Space| layout.grid[space.index()];
        let room = at(Space::TopLeft).w + at(Space::TopRight).w;
        assert!((at(Space::TopLeft).w - room * 0.3).abs() <= 1.0, "the top row");
        assert!(
            (at(Space::BottomLeft).w - room * 0.7).abs() <= 1.0,
            "the bottom row"
        );
        assert_eq!(
            at(Space::TopLeft).h,
            at(Space::TopRight).h,
            "one line runs across the whole grid"
        );
        assert!(
            layout.column_divider[0].live() && layout.column_divider[1].live(),
            "a vertical line for each row"
        );
        assert!(layout.row_divider[0].live());
        assert!(!layout.row_divider[1].live(), "one horizontal line, not two");
        for row in 0..2 {
            let band = layout.column_divider[row].band;
            let (x, y) = (band.x + band.w * 0.5, band.y + band.h * 0.5);
            assert_eq!(layout.hit(x, y), Some(Hit::ColumnDivider(row)), "row {row}");
        }

        // And a drop near the top row's line is read off the row the pointer is
        // in, the same way it is read off the column when the grid reads in
        // columns.
        let top = at(Space::TopLeft);
        let line = top.x + top.w + GAP * 0.5;
        assert_eq!(
            layout.landing(line, top.y + top.h - 4.0),
            Landing::Span(Space::TopLeft, Space::TopRight)
        );
        let bottom = at(Space::BottomLeft);
        assert_eq!(
            layout.landing(line, bottom.y + bottom.h * 0.5),
            Landing::In(Space::BottomLeft, None),
            "the bottom row was read off the top row's line"
        );
    }

    /// A divider is as long as the panes it divides. With one column standing
    /// empty the two panes in the other one are full width, so the line between
    /// them is too, and it can be grabbed at either end of the window.
    #[test]
    fn a_divider_runs_as_far_as_the_panes_it_divides() {
        let mut dock = Dock::new();
        for view in [
            View::Activity,
            View::Plan,
            View::Agents,
            View::Hardware,
            View::Context,
            View::Session,
        ] {
            dock.move_view(view, Space::TopLeft);
        }
        dock.move_view(View::Files, Space::BottomLeft);
        let layout = Layout::compute(1200.0, 800.0, &shape(&dock, &[]));
        assert!(!layout.column_divider[0].live(), "there is one column");
        assert!(layout.row_divider[0].live());
        let (band, body) = (layout.row_divider[0].band, layout.row_divider[0].track);
        assert!((band.x - body.x).abs() < 0.01, "{band:?} in {body:?}");
        assert!((band.w - body.w).abs() < 0.01, "{band:?} in {body:?}");
        let y = band.y + band.h * 0.5;
        for x in [band.x + 1.0, band.x + band.w - 1.0] {
            assert_eq!(layout.hit(x, y), Some(Hit::RowDivider(0)), "at {x}");
        }
    }

    /// A folded pane collapses to its strip and the pane under it in the same
    /// column takes the room, in either column now that both of them can be
    /// split. Nothing in the other column moves, and the line the fold decided
    /// cannot be dragged until it is opened again.
    #[test]
    fn folding_a_pane_hands_its_room_down_its_own_column() {
        let mut dock = Dock::new();
        dock.move_view(View::Hardware, Space::BottomLeft);
        let open = Layout::compute(1200.0, 800.0, &shape(&dock, &[]));
        dock.slot_mut(Space::TopLeft).folded = true;
        let folded = Layout::compute(1200.0, 800.0, &shape(&dock, &[]));

        assert_eq!(folded.placed(Space::TopLeft).body.h, 0.0);
        assert!(folded.placed(Space::TopLeft).strip.h > 1.0, "no strip left");
        assert!(
            folded.placed(Space::BottomLeft).body.h
                > open.placed(Space::BottomLeft).body.h + TAB_H,
            "the pane under it did not take the room"
        );
        for space in [Space::TopRight, Space::BottomRight] {
            assert_eq!(
                folded.placed(space).body,
                open.placed(space).body,
                "{space:?} moved"
            );
        }
        // The line inside the folded column has nothing to drag; the one inside
        // the other column still has, and it is a line of its own now rather
        // than the same line reaching over both columns.
        assert!(!folded.row_divider[0].live(), "the fold owns where it sits");
        assert!(folded.row_divider[1].live(), "the other column can still move");
        assert_eq!(
            folded.row_divider[1].band.x,
            folded.grid[Space::TopRight.index()].x,
            "the line left standing is the right column's alone"
        );
    }

    /// A folded half keeps the ratio it was left at. The fold decides where the
    /// line sits while it lasts, and the number underneath is untouched, so
    /// opening the pane again finds the line where it was rather than snapping
    /// it to the other column's.
    #[test]
    fn a_folded_half_gets_its_own_line_back_when_it_opens() {
        let mut dock = Dock::new();
        dock.move_view(View::Hardware, Space::BottomLeft);
        let halves = [0.3, 0.7];
        let open = Layout::compute(
            1200.0,
            800.0,
            &halves_shape(&dock, [crate::config::LEFT_WIDTH; 2], halves),
        );
        dock.slot_mut(Space::TopLeft).folded = true;
        let folded = Layout::compute(
            1200.0,
            800.0,
            &halves_shape(&dock, [crate::config::LEFT_WIDTH; 2], halves),
        );
        // While it is folded the pane is its strip and the one under it has the
        // rest, whatever the ratio says.
        assert_eq!(folded.placed(Space::TopLeft).body.h, 0.0);
        assert!(!folded.row_divider[0].live());

        dock.slot_mut(Space::TopLeft).folded = false;
        let opened = Layout::compute(
            1200.0,
            800.0,
            &halves_shape(&dock, [crate::config::LEFT_WIDTH; 2], halves),
        );
        for space in Space::ALL {
            assert_eq!(
                opened.placed(space).body,
                open.placed(space).body,
                "{space:?} came back somewhere else"
            );
        }
        assert!(opened.row_divider[0].live(), "the line is draggable again");
    }

    /// A drop is read off the grid: inside a cell it takes that one cell, and on
    /// or near the line between two it takes both. Walked over the whole of the
    /// room the panes share, every band and every corner included.
    #[test]
    fn a_drop_names_one_cell_inside_it_and_two_between_them() {
        let dock = Dock::new();
        let layout = Layout::compute(1400.0, 900.0, &shape(&dock, &[]));
        let cells = layout.grid;
        let line_x = cells[0].x + cells[0].w + GAP * 0.5;
        let line_y = cells[0].y + cells[0].h + GAP * 0.5;
        let far = SPAN_BAND + GAP + 4.0;

        // Inside a cell, clear of both lines and below any tab strip: one cell,
        // and the cell the pointer is actually in.
        for cell in Space::ALL {
            let box_ = cells[cell.index()];
            for (x, y) in [
                (box_.x + far, box_.y + TAB_H + far),
                (box_.x + box_.w - far, box_.y + TAB_H + far),
                (box_.x + far, box_.y + box_.h - far),
                (box_.x + box_.w - far, box_.y + box_.h - far),
                (box_.x + box_.w * 0.5, box_.y + box_.h * 0.5),
            ] {
                assert_eq!(
                    layout.landing(x, y),
                    Landing::In(cell, None),
                    "{cell:?} at {x},{y}"
                );
            }
        }

        // On the line between two cells of a column: both of them, from above
        // it and from the gap itself. The band stops where the lower pane's tab
        // strip starts, because a strip names a place among its tabs and that is
        // the older answer; the gap that is drawn is always outside it.
        for column in 0..2 {
            let (top, bottom) = (Space::at(0, column), Space::at(1, column));
            let x = cells[top.index()].x + cells[top.index()].w * 0.5;
            for y in [line_y - SPAN_BAND, line_y - 1.0, line_y, line_y + 2.0] {
                assert_eq!(
                    layout.landing(x, y),
                    Landing::Span(top, bottom),
                    "the column line at {x},{y}"
                );
            }
            // And a hair outside the band is one cell again.
            for (y, want) in [
                (line_y - SPAN_BAND - GAP, top),
                (line_y + SPAN_BAND + GAP + TAB_H, bottom),
            ] {
                assert_eq!(layout.landing(x, y), Landing::In(want, None), "at {x},{y}");
            }
        }
        // Under the line the band runs its full depth wherever no strip is in
        // the way, which is a cell standing empty: every space holds a pane in
        // the arrangement the window opens with, so the cell is emptied here.
        let mut bare = Dock::new();
        for view in bare.slot(Space::BottomLeft).views.clone() {
            bare.move_view(view, Space::TopLeft);
        }
        let bare = Layout::compute(1400.0, 900.0, &shape(&bare, &[]));
        let x = cells[0].x + cells[0].w * 0.5;
        assert_eq!(
            bare.landing(x, line_y + SPAN_BAND),
            Landing::Span(Space::TopLeft, Space::BottomLeft),
        );

        // The same across a row. The top row's line runs behind its tab strips,
        // so it is walked below them: a strip names a place among tabs, which is
        // the older answer and still the right one there.
        for row in 0..2 {
            let (left, right) = (Space::at(row, 0), Space::at(row, 1));
            let box_ = cells[left.index()];
            let y = box_.y + box_.h - far;
            for x in [line_x - SPAN_BAND, line_x - 1.0, line_x, line_x + SPAN_BAND] {
                assert_eq!(
                    layout.landing(x, y),
                    Landing::Span(left, right),
                    "the row line at {x},{y}"
                );
            }
        }

        // Where the two lines cross, the nearer one wins, and a dead heat goes
        // to the pair that spans a column.
        assert_eq!(
            layout.landing(line_x, line_y),
            Landing::Span(Space::TopRight, Space::BottomRight),
            "the crossing"
        );
        assert_eq!(
            layout.landing(line_x + 2.0, line_y - SPAN_BAND),
            Landing::Span(Space::TopLeft, Space::TopRight),
            "hard against the vertical line, so the pair is across the row"
        );
        assert_eq!(
            layout.landing(line_x - SPAN_BAND, line_y - 2.0),
            Landing::Span(Space::TopLeft, Space::BottomLeft),
            "hard against the horizontal one, so the pair is down the column"
        );

        // Nothing below the strips is a miss: every point in the grid names a
        // cell or a pair, and every point outside it names nothing.
        let whole = around(cells[0], cells[3]);
        let mut x = whole.x;
        while x < whole.x + whole.w {
            let mut y = whole.y + TAB_H + 1.0;
            while y < whole.y + whole.h {
                // A cell, a pair, or a place among the tabs of a strip. Never
                // a miss: the room the panes share is all target.
                assert!(
                    matches!(layout.landing(x, y), Landing::In(..) | Landing::Span(..)),
                    "nothing at {x},{y}"
                );
                y += 7.0;
            }
            x += 11.0;
        }
        for (x, y) in [
            (700.0, 10.0),
            (700.0, whole.y - 2.0),
            (whole.x - 2.0, 400.0),
            (700.0, layout.input.y + 4.0),
        ] {
            assert_eq!(layout.landing(x, y), Landing::Nowhere, "at {x},{y}");
        }
    }

    /// The band around a line never swallows a cell whole, however short the
    /// cells are: there is always a way to drop into one alone.
    #[test]
    fn a_short_cell_still_has_room_to_be_dropped_into() {
        let dock = Dock::new();
        for (w, h) in [(680.0, 380.0), (680.0, 420.0), (900.0, 400.0)] {
            let layout = Layout::compute(w, h, &shape(&dock, &[]));
            for cell in Space::ALL {
                let box_ = layout.grid[cell.index()];
                let (x, y) = (box_.x + box_.w * 0.5, box_.y + box_.h * 0.5);
                assert_eq!(
                    layout.landing(x, y),
                    Landing::In(cell, None),
                    "{cell:?} at {w}x{h} is all band"
                );
            }
        }
    }

    /// The green box is the room the drop would take: one cell for a drop
    /// inside one, both for a drop between two, and the pair a pane would end up
    /// covering when its neighbour is left empty.
    #[test]
    fn the_green_box_is_the_room_the_drop_would_take() {
        let dock = Dock::new();
        let boxed_view = |view: View, landing: Landing| {
            let out = render_with(
                &busy_state(),
                1200.0,
                800.0,
                &dock,
                &["a.rs"],
                &Monitor::new(),
                Some(Drag {
                    view,
                    at: (600.0, 400.0),
                    landing,
                }),
            );
            let rect = out
                .scene
                .over_rects
                .iter()
                .find(|rect| rect.rgba() == out.skin.drop_target)
                .unwrap_or_else(|| panic!("no green box for {landing:?}"))
                .xywh();
            (rect, out.layout.grid)
        };
        let boxed = |landing: Landing| boxed_view(View::Session, landing);
        let want = |cells: &[Space], grid: [Panel; 4]| {
            let box_ = cells
                .iter()
                .fold(nowhere(), |box_, cell| around(box_, grid[cell.index()]));
            [box_.x, box_.y, box_.w, box_.h]
        };

        // One cell: the pane that takes it is beside another, so it gets that
        // cell and no more.
        let (rect, grid) = boxed(Landing::In(Space::BottomRight, None));
        assert_eq!(rect, want(&[Space::BottomRight], grid), "one cell");

        // Two: the drop merges the pair, so the box is both.
        let (rect, grid) = boxed(Landing::span(Space::TopRight, Space::BottomRight));
        assert_eq!(
            rect,
            want(&[Space::TopRight, Space::BottomRight], grid),
            "a column pair"
        );
        let (rect, grid) = boxed(Landing::span(Space::TopLeft, Space::TopRight));
        assert_eq!(
            rect,
            want(&[Space::TopLeft, Space::TopRight], grid),
            "a row pair"
        );

        // A cell whose neighbour is left empty by the drop: the pane covers
        // both, so the box says both. The plan dropped into the conversation's
        // cell leaves the cell under it empty, and the pane keeps the whole
        // column.
        let (rect, grid) = boxed_view(View::Plan, Landing::In(Space::TopLeft, None));
        assert_eq!(
            rect,
            want(&[Space::TopLeft, Space::BottomLeft], grid),
            "the pane still spans the column it is alone in"
        );
        // A pane that leaves something behind it is what keeps the two cells
        // apart: the conversation dropped into the cell under it leaves its own
        // cell occupied, so the box is one cell.
        let (rect, grid) = boxed_view(View::Output, Landing::In(Space::BottomLeft, None));
        assert_eq!(
            rect,
            want(&[Space::BottomLeft], grid),
            "the cell it came from still holds panes"
        );
    }

    /// Item 7's other half: the tab in the air over the outside of the window
    /// says the drop closes it. There is nowhere to land, so there is no green
    /// box either, and the ghost's edge and label go to the bad colour.
    #[test]
    fn a_tab_dragged_off_the_window_is_drawn_in_the_bad_tint() {
        let dock = Dock::new();
        let ghost_of = |landing: Landing, at: (f32, f32)| {
            render_with(
                &busy_state(),
                1200.0,
                800.0,
                &dock,
                &[],
                &Monitor::new(),
                Some(Drag {
                    view: View::Plan,
                    at,
                    landing,
                }),
            )
        };

        let out = ghost_of(Landing::Out, (1210.0, 400.0));
        assert!(
            out.scene
                .over_rects
                .iter()
                .any(|rect| rect.rgba() == out.skin.drop_out),
            "the ghost is not marked for deletion"
        );
        assert!(
            !out.scene
                .over_rects
                .iter()
                .any(|rect| rect.rgba() == out.skin.drop_target),
            "a space is boxed as a target for a drop that lands outside"
        );
        let label = out
            .scene
            .over_texts
            .iter()
            .flat_map(|text| text.runs.iter())
            .find(|run| run.text == View::Plan.label())
            .expect("the ghost has no label");
        assert_eq!(label.color, Some(out.skin.bad));

        // Back over a space it is the ordinary ghost again, over a green box.
        let in_ = ghost_of(Landing::In(Space::TopLeft, None), (400.0, 500.0));
        assert!(
            !in_.scene
                .over_rects
                .iter()
                .any(|rect| rect.rgba() == in_.skin.drop_out),
            "a tab over a space is drawn as if it were being thrown away"
        );
        assert!(
            in_.scene
                .over_rects
                .iter()
                .any(|rect| rect.rgba() == in_.skin.drop_target)
        );
        let label = in_
            .scene
            .over_texts
            .iter()
            .flat_map(|text| text.runs.iter())
            .find(|run| run.text == View::Plan.label())
            .expect("the ghost has no label");
        assert_eq!(label.color, Some(in_.skin.bright));
    }

    /// The other half of item 17: which place in the strip the drop would take,
    /// as a caret standing in that gap. A drop that names no place draws none.
    #[test]
    fn a_drop_that_names_a_place_draws_a_caret_in_that_gap() {
        let dock = Dock::new();
        let layout = Layout::compute(1200.0, 800.0, &shape(&dock, &[]));
        let tabs = layout.placed(Space::TopRight).tabs.clone();
        assert!(tabs.len() > 2);

        for (step, (_, tab)) in tabs.iter().enumerate() {
            // A pointer on the left half of a tab lands in front of it, so the
            // caret stands on that tab's own left edge.
            let x = tab.x + 1.0;
            let at = layout.insertion(Space::TopRight, x);
            assert_eq!(at, step);
            let out = render_with(
                &busy_state(),
                1200.0,
                800.0,
                &dock,
                &[],
                &Monitor::new(),
                Some(Drag {
                    view: View::Files,
                    at: (x, tab.y + tab.h * 0.5),
                    landing: Landing::In(Space::TopRight, Some(at)),
                }),
            );
            let caret = out
                .scene
                .over_rects
                .iter()
                .map(|rect| (rect.xywh(), rect.rgba()))
                .find(|(_, rgba)| *rgba == out.skin.drop_mark)
                .unwrap_or_else(|| panic!("no caret for a drop in front of tab {step}"));
            let ([cx, cy, cw, ch], _) = caret;
            assert!((cx - tab.x).abs() < 0.01, "tab {step}: the caret is at {cx}, not {}", tab.x);
            assert_eq!(cy, tab.y, "tab {step}: the caret is not in the strip");
            assert_eq!(ch, tab.h, "tab {step}");
            assert!(cw > 1.0 && cw < 6.0, "tab {step}: a caret {cw} wide");
        }

        // Behind the last tab: on its right edge, and still inside the strip.
        let placed = layout.placed(Space::TopRight);
        let end = placed.first_tab + placed.tabs.len();
        let last = placed.tabs.last().expect("tabs").1;
        let out = render_with(
            &busy_state(),
            1200.0,
            800.0,
            &dock,
            &[],
            &Monitor::new(),
            Some(Drag {
                view: View::Files,
                at: (last.x + last.w, last.y),
                landing: Landing::In(Space::TopRight, Some(end)),
            }),
        );
        let caret = out
            .scene
            .over_rects
            .iter()
            .find(|rect| rect.rgba() == out.skin.drop_mark)
            .expect("no caret behind the last tab")
            .xywh();
        assert!((caret[0] - (last.x + last.w)).abs() < 0.01, "{caret:?}");
        assert!(
            caret[0] + caret[2] <= placed.strip.x + placed.strip.w + 0.01,
            "the caret hangs off the strip: {caret:?}"
        );

        // A drop in the body of a pane names a space and no place in its strip,
        // so there is a box and no caret: the tab goes to the end.
        let out = render_with(
            &busy_state(),
            1200.0,
            800.0,
            &dock,
            &[],
            &Monitor::new(),
            Some(Drag {
                view: View::Files,
                at: middle(placed.body),
                landing: Landing::In(Space::TopRight, None),
            }),
        );
        assert!(
            out.scene
                .over_rects
                .iter()
                .any(|rect| rect.rgba() == out.skin.drop_target),
            "the target is not boxed"
        );
        assert!(
            !out.scene
                .over_rects
                .iter()
                .any(|rect| rect.rgba() == out.skin.drop_mark),
            "a caret promises a place the drop does not name"
        );
    }

    /// The band has to land on the text it selects. This is the geometry that
    /// can be silently wrong: the selection model is right, the copy is right,
    /// and the highlight sits a line off.
    #[test]
    fn the_selection_band_covers_the_rows_it_selects() {
        let mut state = busy_state();
        // Three known lines at the end of the conversation.
        for text in ["alpha alpha", "beta beta", "gamma gamma"] {
            state.output.say(text, Tone::Body);
        }
        let last = state.output.last() - 1;
        let mut selection =
            crate::select::Selection::new(crate::select::Where::Pane(View::Output), crate::select::Spot::new(last - 2, 6));
        selection.extend(crate::select::Spot::new(last, 5));

        let dock = Dock::new();
        let shape = shape(&dock, &["a.rs"]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot: None,
            trouble: None,
            esc_armed: false,
            popup_scroll: [0, 0],
            cursor: (-100.0, -100.0),
            selection: Some(selection),
            menu: None,
            picker: None,
            settings: None,
        });

        let body = layout.placed(Space::TopLeft).body.inset(PAD);
        let bands: Vec<[f32; 4]> = scene
            .rects
            .iter()
            .filter(|r| r.rgba() == skin.select)
            .map(|r| r.xywh())
            .collect();
        // One per selected line, no more: a single rectangle over the block
        // would cover text on the first and last lines that is not selected.
        assert_eq!(bands.len(), 3, "{bands:?}");
        // Consecutive rows, top to bottom, each one line tall.
        //
        // At the size the output pane is *drawn* with, not the pane size. This
        // assertion used to read `line_for(13.0)` while the transcript rendered
        // at 14.0, so it passed while the highlight sat a growing fraction of a
        // row above the glyphs it was supposed to cover.
        let line = Text::line_for(14.0);
        let mut ys: Vec<f32> = bands.iter().map(|b| b[1]).collect();
        ys.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in ys.windows(2) {
            assert!((pair[1] - pair[0] - line).abs() < 0.01, "{ys:?}");
        }
        for band in &bands {
            assert!((band[3] - line).abs() < 0.01, "band is {} tall", band[3]);
            assert!(band[0] >= body.x - 0.01, "{band:?} starts left of the pane");
            assert!(
                band[0] + band[2] <= body.x + body.w + 0.01,
                "{band:?} runs past the pane"
            );
        }
        // The first line starts six columns in, the last starts at the edge.
        let first = bands
            .iter()
            .min_by(|a, b| a[1].partial_cmp(&b[1]).unwrap())
            .unwrap();
        assert!((first[0] - (body.x + 6.0 * 8.0)).abs() < 0.01, "{first:?}");
    }

    /// A band over a wrapped line has to sit on the glyphs of the row it is
    /// drawn on, not out in the empty space to the right of them. The rows and
    /// the clipboard already agree (widgets::output proves that); this is the
    /// rectangle, which is the half a reader sees.
    #[test]
    fn a_band_over_a_wrapped_line_stays_on_the_glyphs_of_its_row() {
        let mut state = busy_state();
        // Enough wrapped lines to have something to scroll back through.
        for _ in 0..30 {
            state.output.say(
                "| websearch | Search the web, fetch pages as Markdown, find \
                 papers/repos via SearXNG (init \u{2192} search/fetch/arxiv/github) |",
                Tone::Body,
            );
        }
        for text in [
            "\u{2039} [background sub-agent result agent-1]",
            "{\"job_id\":\"agent-1\",\"result\":\"Done. `/tmp/claude-1000/-home-hec/\
             scratchpad/live-web/hello.txt` contains the single line `Hello, world!`.\",\
             \"source\":\"noob_background_subagent\",\"status\":\"ok\",\
             \"trust\":\"untrusted_data_not_human_instruction\"}",
            "1. Websearch tool: \u{2705} Online and working. SearXNG 2026.8.1 with 83 \
             engines, plus DuckDuckGo and Google as keyless providers.",
            "1. **Websearch tool**: Online and working. `SearXNG 2026.8.1` with 83 \
             engines, plus DuckDuckGo and Google as keyless providers.",
            "2. Search result for `llama.cpp grammar tool calls`: the top hit is a \
             GitHub discussion asking whether **custom grammars** can compose with \
             tool calls ([link](https://github.com/ggml-org/llama.cpp/discussions/22408)).",
            "| skill | Load a **specialized** skill (e.g., `cloudflare`, \
             workers-best-practices) to guide your actions |",
            "| context | Report estimated context window usage |",
        ] {
            state.output.say(text, Tone::Body);
        }
        let dock = Dock::new();
        let shape = shape(&dock, &["a.rs"]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        // Scrolled back so the window starts partway down a wrapped line, which
        // is what a reader looking at older output is always doing.
        let panel0 = layout.placed(Space::TopLeft).body;
        let cols0 = crate::view::draw::text_columns(View::Output, panel0, 8.0).0;
        let fit0 = layout.rows(panel0, 14.0);
        let rows0 = fit0 - state.output_reserved(fit0);
        // Whatever the pane is scrolled to: at least one of these starts the
        // window partway down a wrapped line, which is the case a reader
        // looking at older output is always in.
        let back = (0..8)
            .find(|back| {
                state.output.scrollback = *back;
                state.output.window(rows0, cols0).skip > 0
            })
            .expect("some scrollback starts the window mid-line");
        state.output.scrollback = back;
        // Selected across lines the scrolled window is actually showing.
        let top = state.output.showing_from(rows0, cols0);
        let mut selection = crate::select::Selection::new(
            crate::select::Where::Pane(View::Output),
            crate::select::Spot::new(top + 1, 4),
        );
        selection.extend(crate::select::Spot::new(top + 3, 12));
        let skin = Skin::from(&Config::default());
        let panel = layout.placed(Space::TopLeft).body;
        let scene = build(&Frame {
            state: &state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot: None,
            trouble: None,
            esc_armed: false,
            popup_scroll: [0, 0],
            cursor: (-100.0, -100.0),
            selection: Some(selection),
            menu: None,
            picker: None,
            settings: None,
        });

        let content = panel.inset(PAD);
        let cols = crate::view::draw::text_columns(View::Output, panel, 8.0).0;
        let fit = layout.rows(panel, 14.0);
        let rows = fit - state.output_reserved(fit);
        let line_h = Text::line_for(14.0);
        let bands: Vec<[f32; 4]> = scene
            .rects
            .iter()
            .filter(|r| r.rgba() == skin.select)
            .map(|r| r.xywh())
            .collect();
        assert!(bands.len() >= 4, "only {} bands: {bands:?}", bands.len());
        for band in &bands {
            let row = ((band[1] - content.y) / line_h).round() as usize;
            let (line, start) = state
                .output
                .spot_in(rows, cols, row, 0)
                .unwrap_or_else(|| panic!("band {band:?} sits on row {row}, which holds no line"));
            let (_, end) = state
                .output
                .spot_in(rows, cols, row, cols + 9)
                .expect("the row a moment ago is still a row");
            // One column past the last glyph is the newline a full-line
            // selection carries; anything further is empty space.
            let widest = content.x + (end - start + 1) as f32 * 8.0;
            assert!(
                band[0] + band[2] <= widest + 0.01,
                "band {band:?} on row {row} (line {line}) runs {:.1}px past the {} glyphs on it",
                band[0] + band[2] - widest,
                end - start
            );
            assert!(band[0] >= content.x - 0.01, "band {band:?} starts left of the text");
        }
    }

    /// The transcript's bands sat one row above their text the moment the
    /// conversation held emoji. The bytes here are the session that showed it
    /// (19fd1a66dd3, 2026-08-05): a bold header, a row of spaced emoji, and
    /// the server's own error line. The claim is the pane's whole geometry:
    /// every line's counted rows equal the rows the renderer really lays out,
    /// so the selection band, the pointer and the scrollbar sit on the glyphs.
    #[test]
    fn the_transcript_is_counted_in_the_rows_it_is_drawn_in() {
        let mut state = State::new();
        state.apply(noob_proto::Event::SessionStart {
            id: "s1".into(),
            workspace: "/w".into(),
            model: "m".into(),
            resumed: false,
        });
        state.apply(noob_proto::Event::TurnStart { turn: 1 });
        state.apply(noob_proto::Event::TextDelta {
            d: "Here's a big list of emojis/icons I can use:\n\n\
                **Smileys & Emotions**\n\
                \u{1f600} \u{1f603} \u{1f604} \u{1f601} \u{1f606} \u{1f605} \u{1f923} \u{1f602}"
                .into(),
        });
        state.apply(noob_proto::Event::Error {
            line: "model response failed: The model produced output that does not match \
                   the expected peg-native format; the partial response was discarded and \
                   no tool calls were executed"
                .into(),
        });

        let dock = a_dock_showing(View::Output);
        let space = Space::ALL
            .into_iter()
            .find(|space| dock.slot(*space).active() == Some(View::Output))
            .expect("the output pane is in the window");
        let shape = shape(&dock, &[]);
        let layout = Layout::compute(1180.0, 760.0, &shape);
        let panel = layout.placed(space).body;
        let cols = cols_of(panel, 8.0);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot: None,
            trouble: None,
            esc_armed: false,
            popup_scroll: [0, 0],
            cursor: (-100.0, -100.0),
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        });

        let text = scene
            .texts
            .iter()
            .find(|text| text.at == panel.inset(PAD))
            .expect("the transcript draws its text");
        assert_eq!(text.wrap_cols, Some(cols), "the box names its columns");

        // The rows the renderer will really lay out, per logical line.
        let laid: String = noob_draw::Run::wrapped(&text.runs, cols, text.wrap_break)
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        // The painter ends every line with a newline, the last included, so
        // the laid buffer carries one empty row after the content. It is
        // clipped, nothing is counted against it, and it is not a content row.
        let drawn_rows = laid.strip_suffix('\n').unwrap_or(&laid).split('\n').count();

        let counted: usize = (0..)
            .map_while(|n| {
                let spans = state.output.rows_of_line(n, cols);
                (!spans.is_empty()).then_some(spans.len())
            })
            .sum();
        assert_eq!(
            drawn_rows, counted,
            "the renderer lays out {drawn_rows} rows but the pane counts {counted}: \
             every band and click below the difference lands on the wrong text\n{laid:?}"
        );
    }

    /// Throwaway probe: replay a session (PROBE_FRAMES=path), reflow at many
    /// widths, and report every line whose counted rows differ from the rows
    /// the renderer would lay out. Tables included, which is the point.
    #[test]
    fn probe_tables() {
        let Ok(path) = std::env::var("PROBE_FRAMES") else {
            return;
        };
        let mut state = State::new();
        for line in std::fs::read_to_string(path).unwrap().lines() {
            if let Some(frame) = noob_proto::decode::<noob_proto::Event>(line) {
                state.apply(frame.body);
            }
        }
        let skin = Skin::from(&Config::default());
        for cols in 40..90usize {
            state.output.reflow(cols);
            let mut fence = state.output.fence_before(0, cols);
            let mut cum_counted = 0usize;
            let mut cum_drawn = 0usize;
            for n in 0.. {
                let Some(line) = state.output.line(n) else {
                    break;
                };
                let counted = state.output.rows_of_line(n, cols).len();
                let mut runs = Vec::new();
                match line.tone {
                    crate::state::Tone::Body if line.table().is_some() => {
                        crate::widgets::output::table(line, &skin, &mut runs);
                    }
                    crate::state::Tone::Body => {
                        crate::markdown::line(&line.text, &mut fence, &skin, &mut runs);
                    }
                    tone => runs.push(noob_draw::Run::tinted(&line.text, skin.tone(tone))),
                }
                let laid: String = noob_draw::Run::wrapped(&runs, cols, text_geometry::Break::Word)
                    .iter()
                    .map(|run| run.text.as_str())
                    .collect();
                let drawn = laid.split('\n').count();
                let drawn_text: String = runs.iter().map(|r| r.text.as_str()).collect();
                cum_counted += counted;
                cum_drawn += drawn;
                if counted != drawn || drawn_text != line.shown() {
                    println!(
                        "cols={cols} line={n} counted={counted} drawn={drawn} table={:?}\n  shown={:?}\n  laid ={:?}",
                        line.table(),
                        line.shown().chars().take(90).collect::<String>(),
                        laid.chars().take(90).collect::<String>(),
                    );
                }
            }
            if cum_counted != cum_drawn {
                println!("cols={cols} TOTAL counted={cum_counted} drawn={cum_drawn}");
            }
        }
    }

    /// The activity pane is a clipped list: every entry is exactly one
    /// screen row, that row is the one the wrap rule would have drawn first
    /// (so it still breaks at a blank, never mid-word when a blank exists),
    /// and what is drawn is what a press or a selection there resolves to,
    /// with only the dim ellipsis decoration after an entry that goes on.
    #[test]
    fn a_pane_is_drawn_in_the_columns_its_selection_is_counted_in() {
        let mut state = busy_state();
        let dock = a_dock_showing(View::Activity);
        let space = Space::ALL
            .into_iter()
            .find(|space| dock.slot(*space).active() == Some(View::Activity))
            .expect("the activity pane is in the window");
        let shape = shape(&dock, &["a.rs"]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let panel = layout.placed(space).body;
        let cols = cols_of(panel, 8.0);
        for text in [
            "hello worldly people everywhere now and then and again and again \
             and once more for luck, with blanks all the way along it so the \
             wrap has plenty of chances to eat one of them on a boundary"
                .to_string(),
            "short and it fits".to_string(),
            String::new(),
            // A word with nowhere to break in it, wider than the pane.
            format!("a word   with   runs   of   blanks   {}   after it", "z".repeat(cols + 5)),
        ] {
            state.activity.say(text, Tone::Body);
        }
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot: None,
            trouble: None,
            esc_armed: false,
            popup_scroll: [0, 0],
            cursor: (-100.0, -100.0),
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        });

        let text = scene
            .texts
            .iter()
            .find(|text| text.at == panel.inset(PAD))
            .expect("the activity pane draws its text");
        assert_eq!(
            text.wrap_cols,
            Some(cols),
            "the box has to name the column count the pane is measured in"
        );

        // The rows the renderer will lay out, which is what the reader sees.
        let laid: String = noob_draw::Run::wrapped(&text.runs, cols, text.wrap_break)
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        let drawn: Vec<Vec<char>> = laid.split('\n').map(|row| row.chars().collect()).collect();

        let rows = layout.rows(panel, 13.0);
        let skip = state.activity.window(rows, cols).skip;
        let mut checked = 0;
        let mut clipped_rows = 0;
        let mut seen = Vec::new();
        for row in 0..rows {
            let Some((line, start)) = state.activity.spot_in(rows, cols, row, 0) else {
                break;
            };
            let (same, end) = state
                .activity
                .spot_in(rows, cols, row, cols + 9)
                .expect("the row a moment ago is still a row");
            assert_eq!(same, line, "row {row} lands on two different lines");
            // One row per entry, always: a line never takes two rows.
            assert!(!seen.contains(&line), "line {line} took a second row");
            seen.push(line);
            let text = &state
                .activity
                .line(line)
                .expect("a row of a line the pane still holds")
                .text;
            let source: Vec<char> = text.chars().take(end).skip(start).collect();
            assert!(end - start <= cols, "row {row} is wider than the pane");
            let whole = text.chars().count();
            let mut want = source.clone();
            if end < whole && end - start < cols {
                // The one decoration: a dim ellipsis after an entry that
                // goes on past its row.
                want.push('\u{2026}');
            }
            if end < whole {
                clipped_rows += 1;
            }
            assert_eq!(
                drawn[row + skip], want,
                "screen row {row} holds something other than what a selection there would copy"
            );
            // And the row still breaks where the wrap rule would have: at a
            // blank when the entry has one inside the pane's columns.
            if end < whole && end - start < cols {
                assert_eq!(
                    text.chars().nth(end),
                    Some(' '),
                    "row {row} was cut mid-word with a blank to break at"
                );
            }
            checked += 1;
        }
        assert!(checked > 3, "only {checked} rows were on screen");
        assert!(clipped_rows >= 2, "nothing overflowed: {clipped_rows}");
    }



    /// A selection in a pane that is not on screen must not paint anything.
    #[test]
    fn a_selection_in_a_hidden_pane_draws_nothing() {
        let mut state = busy_state();
        state.activity.say("something to select", Tone::Body);
        let last = state.activity.last() - 1;
        let mut selection =
            crate::select::Selection::new(crate::select::Where::Pane(View::Activity), crate::select::Spot::new(last, 0));
        selection.extend(crate::select::Spot::new(last, 9));

        // Fold every space away, so nothing is showing at all.
        let mut dock = Dock::new();
        for space in Space::ALL {
            dock.slot_mut(space).folded = true;
        }
        let shape = shape(&dock, &["a.rs"]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot: None,
            trouble: None,
            esc_armed: false,
            popup_scroll: [0, 0],
            cursor: (-100.0, -100.0),
            selection: Some(selection),
            menu: None,
            picker: None,
            settings: None,
        });
        assert!(!scene.rects.iter().any(|r| r.rgba() == skin.select));
    }

    /// Every text box must be able to hold at least one line of its own size.
    /// A box shorter than that draws the text and clips every pixel of it,
    /// which reads as the interface being broken.
    #[test]
    fn no_text_box_is_too_small_to_show_its_text() {
        let state = busy_state();
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        monitor.sample(&state);
        for (w, h) in [(1400.0, 900.0), (900.0, 520.0), (700.0, 400.0)] {
            for view in View::ALL {
                let mut dock = Dock::new();
                dock.reveal(view);
                let out = render_with(&state, w, h, &dock, &["calc.py"], &monitor, None);
                for text in &out.scene.texts {
                    assert!(text.at.w >= 1.0, "{view:?} {:?} at {w}x{h}", text.at);
                    assert!(
                        text.at.h >= Text::line_for(text.size),
                        "{view:?} {:?} cannot hold one {}pt line at {w}x{h}",
                        text.at,
                        text.size
                    );
                    assert!(text.at.x >= 0.0 && text.at.y >= 0.0, "{:?}", text.at);
                    assert!(text.at.x + text.at.w <= w + 0.01, "{view:?} {:?}", text.at);
                    assert!(text.at.y + text.at.h <= h + 0.01, "{view:?} {:?}", text.at);
                }
            }
        }
    }

    /// Every view, at every size, since a pane that scrolls draws a scrollbar and
    /// a bar in a pane two pixels tall is the sort of rectangle that ends up
    /// hanging outside the window.
    #[test]
    fn every_rectangle_is_inside_the_surface() {
        let state = crowded_state();
        let monitor = sampled(&state);
        for (w, h) in [(1400.0, 900.0), (700.0, 400.0), (320.0, 240.0)] {
            for view in View::ALL {
                let mut dock = Dock::new();
                dock.reveal(view);
                let out = render_with(&state, w, h, &dock, &["a.rs"], &monitor, None);
                assert!(!out.scene.rects.is_empty());
                for rect in &out.scene.rects {
                    let [x, y, rw, rh] = rect.xywh();
                    assert!(x >= 0.0 && y >= 0.0, "{view:?} {rect:?} at {w}x{h}");
                    assert!(
                        x + rw <= w + 0.01 && y + rh <= h + 0.01,
                        "{view:?} {rect:?} at {w}x{h}"
                    );
                }
            }
        }
    }

    /// Each view shows its own thing and not another's.
    #[test]
    fn each_view_shows_its_own_content() {
        let state = busy_state();
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        monitor.sample(&state);
        let seen = |view: View| {
            let mut dock = Dock::new();
            dock.reveal(view);
            text_of(&render_with(&state, 1400.0, 900.0, &dock, &["calc.py"], &monitor, None).scene)
        };
        assert!(seen(View::Activity).contains("cargo test --workspace"));
        let plan = seen(View::Plan);
        assert!(plan.contains("[x] read it"), "{plan}");
        assert!(!plan.contains("cargo test --workspace"), "activity leaked");
        assert!(seen(View::Agents).contains("search the web"));
        assert!(seen(View::Files).contains("return a + b"));
        // The monitors are three different lists: the machine, how full this run
        // is, and what it spent getting there.
        let hardware = seen(View::Hardware);
        assert!(hardware.contains("CPU") || hardware.contains("RAM"), "{hardware}");
        let context = seen(View::Context);
        assert!(context.contains("TOTAL TOOL CALLS"), "{context}");
        assert!(context.contains("LAST PREFILL"), "{context}");
        assert!(!context.contains("CPU"), "hardware leaked into CONTEXT: {context}");
        // The model and the workspace were rows of this pane. The strip says
        // where the agent is working and the settings panel says what it is.
        assert!(!context.contains("laguna-s21"), "the model row is back: {context}");
        let session = seen(View::Session);
        for wanted in ["PREFILLED", "GENERATED", "CACHED", "PREFILL", "DECODE"] {
            assert!(session.contains(wanted), "{wanted} is not in {session}");
        }
        assert!(!session.contains("TOOL CALLS"), "the other pane leaked: {session}");
        assert!(!session.contains("MEAN"), "the all-time readings are gone: {session}");
        assert!(!hardware.contains("DECODE"), "the reverse: {hardware}");
    }

    /// The tabs are the whole of what the rename changed, so this reads them off
    /// the strip rather than out of the scene as a whole: a label is matched
    /// inside the box the layout gave that tab, so a reading of the same name
    /// inside a pane (CONTEXT is also a gauge) cannot stand in for it.
    #[test]
    fn the_tab_strips_read_the_renamed_labels() {
        let out = render(&busy_state(), 1600.0, 1000.0, &Dock::new(), &["calc.py"]);
        let mut on_strip: Vec<&str> = Vec::new();
        for space in Space::ALL {
            for (view, tab) in &out.layout.placed(space).tabs {
                let drawn: Vec<&str> = out
                    .scene
                    .texts
                    .iter()
                    .filter(|text| {
                        text.at.x + 0.01 >= tab.x
                            && text.at.x <= tab.x + tab.w + 0.01
                            && text.at.y + 0.01 >= tab.y
                            && text.at.y <= tab.y + tab.h + 0.01
                    })
                    .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
                    .collect();
                assert!(
                    drawn.contains(&view.label()),
                    "{view:?} does not say {:?} on its own tab: {drawn:?}",
                    view.label()
                );
                on_strip.push(view.label());
            }
        }
        for wanted in ["OUTPUT", "CONTEXT", "SESSION"] {
            assert!(on_strip.contains(&wanted), "no {wanted} tab: {on_strip:?}");
        }
        // Renamed, not added: the old readings must not still have tabs of their
        // own beside the new ones.
        for gone in ["TALK", "OVERALL"] {
            assert!(!on_strip.contains(&gone), "{gone} still has a tab");
        }
        assert_eq!(on_strip.len(), View::ALL.len() - 1, "{on_strip:?}");
    }

    /// The conversation and what has been typed are on screen whichever tab is
    /// up, because they are in a different space.
    ///
    /// This also asserted the token budget, which the title strip used to
    /// carry. It does not any more, and the budget is a monitor reading now.
    #[test]
    fn the_conversation_stays_visible_whatever_the_other_space_shows() {
        let state = busy_state();
        for view in [View::Hardware, View::Context, View::Agents] {
            let mut dock = Dock::new();
            dock.reveal(view);
            let text = text_of(&render(&state, 1400.0, 900.0, &dock, &["calc.py"]).scene);
            assert!(text.contains("looking at it now"), "{view:?}");
            assert!(text.contains("type here"), "{view:?}");
        }
    }
















    /// Everything one space has written, as one string.
    fn written_in(out: &Rendered, space: Space) -> String {
        let body = out.layout.placed(space).body;
        out.scene
            .texts
            .iter()
            .filter(|t| body.contains(t.at.x, t.at.y))
            .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
            .collect()
    }

    /// Item 14: a widget whose content is taller than its box scrolls inside
    /// itself. One mechanism for every list pane, so this drives a list and a
    /// monitor.
    ///
    /// Both used to lose what did not fit: PLAN and AGENTS drew past the bottom
    /// edge of the pane, and a monitor stopped at the last reading that fitted.
    /// Neither had a bar, so nothing on screen even said there was more.
    #[test]
    fn a_pane_with_more_rows_than_its_box_scrolls_to_the_end() {
        for (view, w, h, last) in [
            (View::Plan, 1400.0, 900.0, "step 39"),
            (View::Agents, 1400.0, 900.0, "child 23 is reading"),
            // The monitor pane is five readings in a box that holds fewer.
            (View::Session, 900.0, 260.0, "DECODE"),
        ] {
            let state = crowded_state();
            let mut scrolls = crate::scroll::Scrolls::default();
            let monitor = sampled(&state);
            let mut dock = Dock::new();
            dock.reveal(view);
            let (space, heights, rows) = measured(&state, w, h, &dock, &monitor, view);
            let total: usize = heights.iter().sum();
            assert!(total > rows, "{view:?}: {total} rows in a box of {rows}");

            // At the top, the last item is off the bottom and the bar says so.
            let top = render_with(&state, w, h, &dock, &[], &monitor, None);
            let (track, thumb) = bar_in(&top, space)
                .unwrap_or_else(|| panic!("{view:?} drew no scrollbar for {total} rows in {rows}"));
            assert!(
                thumb[3] < track[3],
                "{view:?}: the thumb fills a track it cannot fill"
            );
            assert!(
                !written_in(&top, space).contains(last),
                "{view:?} already shows {last}, so it does not need a scroll"
            );

            // Scrolled to the end, the last item is on screen, and one notch
            // further moves nothing.
            assert!(
                scrolls.scroll(view, 9_999, true, &heights, rows),
                "{view:?} would not scroll"
            );
            assert!(
                !scrolls.scroll(view, 1, true, &heights, rows),
                "{view:?} scrolled past its own end"
            );
            let end = render_scrolled(&state, &scrolls, w, h, &dock, &[], &monitor, None);
            let written = written_in(&end, space);
            assert!(written.contains(last), "{view:?} cannot reach {last}: {written}");
            let (track, thumb) = bar_in(&end, space).expect("still a bar");
            assert!(
                (thumb[1] + thumb[3] - track[1] - track[3]).abs() < 1.5,
                "{view:?}: the thumb is not at the foot of its track: {thumb:?} in {track:?}"
            );

            // And back to the top, where it started.
            assert!(scrolls.scroll(view, 9_999, false, &heights, rows));
            assert_eq!(scrolls.first(view), 0, "{view:?}");
        }
    }

    /// A pane holding less than it can show draws no bar. A bar that is always
    /// there and always full says nothing, which is why `scrollbar` takes an
    /// option.
    #[test]
    fn a_pane_whose_content_fits_draws_no_bar() {
        let state = busy_state();
        let monitor = sampled(&state);
        for view in [View::Plan, View::Agents, View::Session] {
            let mut dock = Dock::new();
            dock.reveal(view);
            let (space, heights, rows) = measured(&state, 1400.0, 900.0, &dock, &monitor, view);
            let total: usize = heights.iter().sum();
            assert!(total <= rows, "{view:?}: {total} rows in a box of {rows}");
            let out = render_with(&state, 1400.0, 900.0, &dock, &[], &monitor, None);
            assert!(
                bar_in(&out, space).is_none(),
                "{view:?} drew a bar for content that fits"
            );
        }
    }

    /// The one that actually reaches a reader: a pane scrolled to its end whose
    /// content then shrank under it. The agent replaces a forty-item plan with a
    /// three-item one and nothing goes near the pointer.
    ///
    /// Two halves. What is drawn is never blank, because the window is taken from
    /// the offset through `text_geometry`, which clamps a position past the end
    /// rather than refusing it. And the offset itself is pulled back, which is what
    /// the frame does before it draws, so the next wheel notch moves one row
    /// instead of thirty-seven.
    #[test]
    fn a_pane_that_shrank_under_a_scroll_is_not_left_blank() {
        let mut state = crowded_state();
        let mut scrolls = crate::scroll::Scrolls::default();
        let monitor = sampled(&state);
        let mut dock = Dock::new();
        dock.reveal(View::Plan);
        let (space, heights, rows) = measured(&state, 1400.0, 900.0, &dock, &monitor, View::Plan);
        scrolls.scroll(View::Plan, 9_999, true, &heights, rows);
        let scrolled = scrolls.first(View::Plan);
        assert!(scrolled > 0, "the pane did not scroll");

        state.apply(noob_proto::Event::ToolStart {
            call_id: "plan-3".into(),
            name: "plan".into(),
            brief: "3 items".into(),
            args: serde_json::json!({"todos": [
                {"content": "late 00", "status": "completed"},
                {"content": "late 01", "status": "in_progress"},
                {"content": "late 02", "status": "pending"},
            ]}),
        });
        let out = render_scrolled(&state, &scrolls, 1400.0, 900.0, &dock, &[], &monitor, None);
        let written = written_in(&out, space);
        for wanted in ["late 00", "late 01", "late 02"] {
            assert!(
                written.contains(wanted),
                "the pane is blank at row {scrolled} of three: {written:?}"
            );
        }

        let (_, short, rows) = measured(&state, 1400.0, 900.0, &dock, &monitor, View::Plan);
        assert!(
            scrolls.settle(View::Plan, &short, rows),
            "the offset was left past the end"
        );
        assert_eq!(scrolls.first(View::Plan), 0);
        let after = render_scrolled(&state, &scrolls, 1400.0, 900.0, &dock, &[], &monitor, None);
        assert!(
            bar_in(&after, space).is_none(),
            "three todos in eighteen rows still drew a bar"
        );
    }

    /// A frame that is nothing but a prompt: the strip it landed in, its
    /// layout, and the scene, at the default 14pt body size. Idle, with the
    /// clock at zero.
    fn render_prompt(prompt: &crate::prompt::Prompt, rows: usize) -> (Panel, Layout, Scene) {
        render_prompt_at(prompt, rows, &State::new(), 0.0)
    }

    /// The same with the window's state and the moment on its clock given, which
    /// is what the marker slot is drawn from.
    fn render_prompt_at(
        prompt: &crate::prompt::Prompt,
        rows: usize,
        state: &State,
        clock: f32,
    ) -> (Panel, Layout, Scene) {
        let dock = Dock::new();
        let skin = Skin::from(&Config::default());
        let mut shape = shape(&dock, &[]);
        shape.input_h = input_height(rows, Text::line_for(14.0));
        let layout = Layout::compute(1200.0, 800.0, &shape);
        let scene = build(&Frame {
            state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt,
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock,
            orb_morph: None,
            drag: None,
            hot: None,
            trouble: None,
            esc_armed: false,
            popup_scroll: [0, 0],
            cursor: (-100.0, -100.0),
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        });
        (layout.input, layout, scene)
    }

    /// The prompt is the height it was set to whatever is in it, and the caret
    /// follows the wrap rather than running off the end of the first line.
    ///
    /// It used to grow a row at a time as characters arrived, and this test
    /// asserted that it did. Hector asked for the opposite: the box is the rows
    /// he chose whether he has typed anything or not, so what was `many.h >
    /// one.h` is now the two being equal. The caret half of it is unchanged.
    #[test]
    fn the_prompt_is_the_height_it_was_set_to_and_the_caret_stays_inside_it() {
        let line = Text::line_for(14.0);
        let short = typed_prompt("short", 5);
        let long = typed_prompt(&"x".repeat(600), 600);
        let (one, ..) = render_prompt(&short, 8);
        let (many, _, scene) = render_prompt(&long, 8);
        assert!(
            (many.h - one.h).abs() < 0.01,
            "the prompt moved: {} then {}",
            one.h,
            many.h
        );
        assert!((many.h - (8.0 * line + 2.0 * INPUT_PAD)).abs() < 0.01, "{}", many.h);
        let caret = scene
            .rects
            .iter()
            .map(|r| r.xywh())
            .rfind(|[x, y, w, _]| *w <= 3.0 && many.contains(*x, *y))
            .expect("the caret is drawn");
        assert!(
            caret[1] + caret[3] <= many.y + many.h + 0.5,
            "the caret left the prompt: {caret:?} in {many:?}"
        );
        assert!(caret[1] > many.y, "and it is not still on the first row");
    }

    /// How tall it is is a setting, not a constant. Two rows and twenty rows
    /// are both a window somebody wants, and it is that many rows empty.
    ///
    /// The last assertion here said the opposite until this build: twenty rows
    /// with nothing typed into them was one row of prompt. That is the growing
    /// this item took out, so it is inverted rather than dropped.
    #[test]
    fn the_prompt_is_the_configured_row_count_empty_or_full() {
        let line = Text::line_for(14.0);
        // More than twenty rows of it, so the setting is what holds it.
        let long = typed_prompt(&"x".repeat(3000), 3000);
        let (two, ..) = render_prompt(&long, 2);
        let (twenty, ..) = render_prompt(&long, 20);
        assert!(twenty.h > two.h, "{} is not taller than {}", twenty.h, two.h);
        // The strip holds that many rows and the padding around them.
        assert!((two.h - (2.0 * line + 2.0 * INPUT_PAD)).abs() < 0.01, "{}", two.h);
        assert!(
            (twenty.h - (20.0 * line + 2.0 * INPUT_PAD)).abs() < 0.01,
            "{}",
            twenty.h
        );
        // And nothing typed at all is the same box, not a box one row tall.
        for rows in [2usize, 20] {
            let (empty, ..) = render_prompt(&crate::prompt::Prompt::default(), rows);
            let (full, ..) = render_prompt(&long, rows);
            assert!(
                (empty.h - (rows as f32 * line + 2.0 * INPUT_PAD)).abs() < 0.01,
                "{rows} rows of empty prompt is {}",
                empty.h
            );
            assert!((empty.h - full.h).abs() < 0.01, "{rows}: {} {}", empty.h, full.h);
        }
    }

    /// A click lands on the character it is over, on any row of a wrapped
    /// prompt. This is the arithmetic that can be silently wrong: the caret is
    /// drawn from it, so an inverse that disagrees puts the caret elsewhere.
    #[test]
    fn a_click_in_the_prompt_lands_on_the_character_under_it() {
        let typed = "0123456789".repeat(50);
        let prompt = typed_prompt(&typed, 0);
        let (strip, layout, scene) = render_prompt(&prompt, 8);
        let line = Text::line_for(14.0);
        let box_ = input_box(strip, line);
        let columns = columns_in(box_.w, 8.0);
        for at in [0usize, 1, 7, columns, columns + 3, columns * 2 + 9] {
            let column = (at + PROMPT_COLUMNS) % columns;
            let row = (at + PROMPT_COLUMNS) / columns;
            // The middle of that cell, which is where a pointer would be.
            let x = box_.x + column as f32 * 8.0 + 3.0;
            let y = box_.y + row as f32 * line + line * 0.5;
            assert_eq!(
                layout.input_caret(x, y, 14.0, 8.0, prompt.len(), prompt.caret()),
                at,
                "row {row} column {column}"
            );
        }
        // Past the end of the text, and past the end of a row.
        let below = box_.y + box_.h - 1.0;
        assert_eq!(
            layout.input_caret(
                box_.x + box_.w - 1.0,
                below,
                14.0,
                8.0,
                prompt.len(),
                prompt.caret(),
            ),
            prompt.len()
        );
        // And the caret the click asks for is where the frame draws it.
        let mut moved = typed_prompt(&typed, 0);
        moved.place(columns + 3);
        let (_, _, after) = render_prompt(&moved, 8);
        let caret = |scene: &Scene| {
            scene
                .rects
                .iter()
                .map(|r| r.xywh())
                .rfind(|[x, y, w, _]| *w <= 3.0 && strip.contains(*x, *y))
                .expect("the caret is drawn")
        };
        assert_ne!(caret(&scene), caret(&after));
        let placed = caret(&after);
        assert_eq!(
            layout.input_caret(
                placed[0] + 1.0,
                placed[1] + 1.0,
                14.0,
                8.0,
                moved.len(),
                moved.caret(),
            ),
            columns + 3
        );
    }

    /// Past its allowance the prompt scrolls inside itself instead of growing,
    /// and what it scrolls to is the row the caret is on.
    ///
    /// The other half of `prompt_rows`. The strip stopped growing at the row
    /// count all along, but the text was drawn from its first row whatever the
    /// caret was doing, so everything past the allowance was typed into a box
    /// that could not show it: a setting of one row read as a setting that did
    /// nothing at all.
    #[test]
    fn a_prompt_past_its_allowance_scrolls_instead_of_growing() {
        let line = Text::line_for(14.0);
        let typed = "x".repeat(600);
        let long = typed_prompt(&typed, 600);
        let (strip, _, scene) = render_prompt(&long, 1);
        // One row and the padding around it, which is the whole point.
        assert!((strip.h - (line + 2.0 * INPUT_PAD)).abs() < 0.01, "{}", strip.h);
        let box_ = input_box(strip, line);
        let columns = columns_in(box_.w, 8.0);
        let rows = (600 + PROMPT_COLUMNS + 1).div_ceil(columns);
        assert!(rows > 1, "600 characters fit on one row at this width");
        let drawn = scene
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text == typed))
            .expect("the prompt is drawn");
        assert_eq!(
            drawn.scroll_lines,
            (rows - 1) as f32,
            "the box is not scrolled to the caret's row"
        );
        // The caret is on screen, on the last row the box can show.
        let caret = scene
            .rects
            .iter()
            .map(|r| r.xywh())
            .rfind(|[x, y, w, _]| *w <= 3.0 && strip.contains(*x, *y))
            .expect("the caret is drawn");
        assert!(caret[1] >= box_.y - 0.5, "the caret is above the box: {caret:?}");
        assert!(
            caret[1] + caret[3] <= box_.y + box_.h + 0.5,
            "the caret is below the box: {caret:?}"
        );
        // A prompt that fits is not scrolled at all.
        let (_, _, short) = render_prompt(&typed_prompt("hello", 5), 1);
        let drawn = short
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text == "hello"))
            .expect("the prompt is drawn");
        assert_eq!(drawn.scroll_lines, 0.0);
    }

    /// The prompt is the rows it was set to whenever the window has the room,
    /// and never the last of it: the panes keep [`PANES_FLOOR`] and the strip
    /// takes what is left over.
    ///
    /// A fixed height is what `prompt_rows` was asked to be, but nothing was
    /// holding room back for the conversation: twenty rows on a short window
    /// left the panes a few pixels tall, which is the window with nothing in it
    /// but the box you type into.
    #[test]
    fn the_prompt_is_capped_at_what_the_panes_can_spare() {
        let dock = Dock::new();
        let line = Text::line_for(14.0);
        for height in [900.0f32, 600.0, 400.0, 200.0] {
            for rows in [1usize, 2, 4, 9, 40] {
                let mut shape = shape(&dock, &[]);
                shape.input_h = input_height(rows, line);
                let layout = Layout::compute(1200.0, height, &shape);
                // The strip around the box, which is what `input_height` is in.
                let strip = layout.input.h + 2.0 * GAP;
                let panes = height - TITLE_H - strip;
                assert!(
                    panes >= PANES_FLOOR - 0.01,
                    "{rows} rows on a {height} window left the panes {panes}"
                );
                let want = input_height(rows, line);
                let spare = height - TITLE_H - PANES_FLOOR;
                match want <= spare {
                    // Room for it, so it is the number the file asked for.
                    true => assert!(
                        (strip - want).abs() < 0.01,
                        "{rows} rows fit in {height} and came out {strip} instead of {want}"
                    ),
                    // No room, so it is everything the panes could spare.
                    false => assert!(
                        (strip - spare).abs() < 0.01,
                        "{rows} rows on a {height} window took {strip} of the {spare} spare"
                    ),
                }
            }
        }
        // Shorter than the floor and one row together, the window cannot give
        // both: the strip keeps the one row it takes to read, and the panes take
        // what is under the floor rather than the strip disappearing.
        let mut shape = shape(&dock, &[]);
        shape.input_h = input_height(30, line);
        let short = Layout::compute(1200.0, TITLE_H + PANES_FLOOR, &shape);
        assert!((short.input.h + 2.0 * GAP - INPUT_H).abs() < 0.01, "{}", short.input.h);
    }

    /// Wherever the caret is, it is on a row the box is showing, and a click on
    /// it reads back as the character it is on.
    ///
    /// Typing is the caret moving one character at a time, so a caret that goes
    /// off the bottom of a one-row prompt is the prompt going blind halfway
    /// through a sentence. The click half is here too because the scroll offset
    /// has to be in the drawing and in the hit testing or a press lands a row
    /// off on anything that has scrolled.
    #[test]
    fn the_caret_stays_on_a_visible_row_however_far_the_prompt_runs() {
        let line = Text::line_for(14.0);
        let typed = "0123456789".repeat(60);
        for max_rows in [1usize, 2, 5] {
            for at in [0usize, 1, 137, 299, 480, 600] {
                let prompt = typed_prompt(&typed, at);
                let (strip, layout, scene) = render_prompt(&prompt, max_rows);
                let box_ = input_box(strip, line);
                let caret = scene
                    .rects
                    .iter()
                    .map(|r| r.xywh())
                    .rfind(|[x, y, w, _]| *w <= 3.0 && strip.contains(*x, *y))
                    .unwrap_or_else(|| panic!("no caret at {at} in {max_rows} rows"));
                assert!(
                    caret[1] >= box_.y - 0.5 && caret[1] + caret[3] <= box_.y + box_.h + 0.5,
                    "the caret left the box at {at} in {max_rows} rows: {caret:?} in {box_:?}"
                );
                assert_eq!(
                    layout.input_caret(
                        caret[0] + 1.0,
                        caret[1] + 1.0,
                        14.0,
                        8.0,
                        prompt.len(),
                        prompt.caret(),
                    ),
                    prompt.caret(),
                    "a click on the caret at {at} in {max_rows} rows means somewhere else"
                );
            }
        }
    }

    /// Select-all bands every row the text covers, and nothing outside the
    /// prompt. A selection you cannot see is a selection you delete by
    /// accident.
    #[test]
    fn the_prompt_bands_what_it_has_selected() {
        let mut prompt = typed_prompt(&"y".repeat(400), 0);
        prompt.select_all();
        let (strip, _, scene) = render_prompt(&prompt, 8);
        let skin = Skin::from(&Config::default());
        let bands: Vec<[f32; 4]> = scene
            .rects
            .iter()
            .filter(|r| r.rgba() == skin.select)
            .map(|r| r.xywh())
            .collect();
        let line = Text::line_for(14.0);
        let box_ = input_box(strip, line);
        let columns = columns_in(box_.w, 8.0);
        assert_eq!(bands.len(), (400 + PROMPT_COLUMNS).div_ceil(columns));
        for band in &bands {
            assert!(band[1] >= box_.y - 0.01, "{band:?} is above the prompt");
            assert!(
                band[1] + band[3] <= box_.y + box_.h + 0.5,
                "{band:?} runs below the prompt"
            );
            assert!(
                band[0] + band[2] <= box_.x + box_.w + 0.01,
                "{band:?} runs past the right edge"
            );
        }
        // The first row starts after the marker, not at the left edge.
        let first = bands
            .iter()
            .min_by(|a, b| a[1].total_cmp(&b[1]))
            .expect("a first row");
        assert!((first[0] - (box_.x + PROMPT_COLUMNS as f32 * 8.0)).abs() < 0.01);
        // Nothing selected is nothing banded.
        let (_, _, plain) = render_prompt(&typed_prompt("hello", 5), 8);
        assert!(!plain.rects.iter().any(|r| r.rgba() == skin.select));
    }

    /// The wave the three dots run while a turn does: one of them up, the other
    /// two down, then the next one, and around again. One dot is up at any
    /// moment, never two and never none, and the one that is up is the next one
    /// along each step of the clock.
    #[test]
    fn one_prompt_dot_is_up_at_a_time_and_it_is_the_next_one_each_step() {
        let seen: Vec<[f32; 3]> = (0..7)
            .map(|step| prompt_wave(step as f32 * PROMPT_DOT_STEP + PROMPT_DOT_STEP * 0.5, true))
            .collect();
        let up = |lift: &[f32; 3]| lift.iter().position(|l| *l > 0.0).expect("a dot is up");
        assert_eq!(
            seen.iter().map(up).collect::<Vec<usize>>(),
            vec![0, 1, 2, 0, 1, 2, 0],
            "{seen:?}"
        );
        for lift in &seen {
            assert_eq!(lift.iter().filter(|l| **l > 0.0).count(), 1, "{lift:?}");
            assert_eq!(lift[up(lift)], PROMPT_DOT_LIFT);
        }
        // A step is held, not crossed: the same dot is up right through it.
        assert_eq!(prompt_wave(0.0, true), prompt_wave(PROMPT_DOT_STEP * 0.9, true));

        // And nothing moves while nothing is running. The window holds no redraw
        // deadline at rest, so a lift that depended on the clock there would be a
        // dot frozen wherever the last frame left it.
        for clock in [0.0, 0.09, 0.4, 6.5, 900.0] {
            assert_eq!(prompt_wave(clock, false), [0.0; 3], "at {clock}s");
        }
    }

    /// The row itself: while a turn runs the marker is three dots in its own two
    /// columns and they move with the clock; at rest it is the chevron and no
    /// dots at all.
    #[test]
    fn the_prompt_marker_is_three_moving_dots_while_a_turn_runs() {
        let skin = Skin::from(&Config::default());
        let prompt = typed_prompt("hello", 5);
        let dots = |scene: &Scene| -> Vec<[f32; 4]> {
            scene
                .rects
                .iter()
                .filter(|r| {
                    let [_, _, w, h] = r.xywh();
                    r.rgba() == skin.caret && w == PROMPT_DOT && h == PROMPT_DOT
                })
                .map(|r| r.xywh())
                .collect()
        };
        let first_run = |scene: &Scene| -> String {
            scene
                .texts
                .iter()
                .find(|text| text.runs.iter().any(|run| run.text == "hello"))
                .expect("the prompt is drawn")
                .runs[0]
                .text
                .clone()
        };

        let (strip, _, resting) = render_prompt_at(&prompt, 8, &State::new(), 0.0);
        assert!(dots(&resting).is_empty(), "the resting prompt drew dots");
        assert_eq!(first_run(&resting), "\u{203a} ", "the resting marker");

        let busy = busy_state();
        let (_, _, scene) = render_prompt_at(&prompt, 8, &busy, 0.0);
        let drawn = dots(&scene);
        assert_eq!(drawn.len(), 3, "{drawn:?}");
        // Two blank columns where the ellipsis was, so every other piece of
        // arithmetic in the row still counts the same two.
        assert_eq!(first_run(&scene), "  ", "the working marker");
        let line = Text::line_for(14.0);
        let box_ = input_box(strip, line);
        let slot = PROMPT_COLUMNS as f32 * 8.0;
        for dot in &drawn {
            assert!(dot[0] >= box_.x, "{dot:?} is left of the box");
            assert!(dot[0] + dot[2] <= box_.x + slot, "{dot:?} is past the marker's columns");
            assert!(dot[1] >= box_.y, "{dot:?} is above the first row");
            assert!(dot[1] + dot[3] <= box_.y + line, "{dot:?} is below the first row");
        }
        // In a row, left to right, and the first one is the one that is up.
        assert!(drawn[0][0] < drawn[1][0] && drawn[1][0] < drawn[2][0], "{drawn:?}");
        assert!(drawn[0][1] < drawn[1][1], "the first dot is not raised: {drawn:?}");
        assert_eq!(drawn[1][1], drawn[2][1], "the other two are not level: {drawn:?}");

        // A step later it is the next dot's turn, and the row is a different
        // picture, which is the whole of the animation.
        let (_, _, later) = render_prompt_at(&prompt, 8, &busy, PROMPT_DOT_STEP * 1.5);
        let after = dots(&later);
        assert_ne!(drawn, after, "the dots did not move");
        assert!(after[1][1] < after[0][1], "the second dot is not raised: {after:?}");
        assert_eq!(after[0][1], after[2][1], "the other two are not level: {after:?}");
    }

    /// Two dark panels side by side over a busy desktop read as one region
    /// with a gap in it. The border is what tells them apart.
    ///
    /// Item 12: a pane gets three sides of that border and no top. This asserted
    /// one stroked rectangle around the whole pane, which is what drew the line
    /// under every tab strip. The prompt and the picker still take the stroke, so
    /// `panel_edge` is still the way a box that wants four sides is drawn.
    #[test]
    fn a_pane_is_bordered_on_three_sides_and_open_at_the_top() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        for space in occupied(&dock) {
            let panel = out.layout.placed(space).body;
            let cut = cut_of(panel);
            let edge = |box_: [f32; 4], side: &str| {
                assert!(
                    out.scene
                        .rects
                        .iter()
                        .any(|r| r.xywh() == box_ && r.rgba() == out.skin.edge),
                    "{space:?}: no {side} edge at {box_:?}"
                );
            };
            edge([panel.x, panel.y, 1.0, panel.h], "left");
            edge([panel.x, panel.y + panel.h - 1.0, panel.w, 1.0], "bottom");
            // Started at the top and the last pixels of it would hang in the
            // corner the cut takes away.
            edge(
                [panel.x + panel.w - 1.0, panel.y + cut, 1.0, panel.h - cut],
                "right",
            );

            // The fill is the only rect the size of the pane, and it is a fill:
            // a stroke that size is the outline this test used to demand, and it
            // paints the top edge along with the other three.
            let over_panel: Vec<_> = out
                .scene
                .rects
                .iter()
                .filter(|r| r.xywh() == [panel.x, panel.y, panel.w, panel.h])
                .collect();
            assert_eq!(over_panel.len(), 1, "{space:?} is more than one fill");
            assert_eq!(over_panel[0].rgba(), out.skin.panel, "{space:?}");
            assert_eq!(over_panel[0].extra()[3], 0.0, "{space:?} is still stroked");

            // And nothing else runs along the top of it, whatever its size.
            for rect in &out.scene.rects {
                let [x, y, w, h] = rect.xywh();
                let across = x <= panel.x + panel.w * 0.5 && x + w >= panel.x + panel.w * 0.5;
                let on_top = y <= panel.y + 1.5 && y + h >= panel.y + 0.5;
                let a_line = h <= 3.0 || rect.extra()[3] > 0.0;
                assert!(
                    !(across && on_top && a_line),
                    "{space:?}: {:?} is a line under the tabs",
                    rect.xywh()
                );
            }
        }
    }

    /// The other half of item 12. Losing the line is only half of making a tab
    /// and its pane one surface: a step in colour where the line was reads the
    /// same way. The showing tab carries the pane's own fill, and it sits flush on
    /// it, so the two composite to one shape. What tells the other tabs apart is
    /// weight, which
    /// `a_tab_that_is_not_showing_is_the_same_tab_with_less_weight` asserts.
    #[test]
    fn the_showing_tab_and_its_pane_are_one_surface() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        let mut checked = 0;
        for space in Space::ALL {
            let placed = out.layout.placed(space);
            let Some(active) = dock.slot(space).active() else {
                continue;
            };
            let (_, tab) = placed
                .tabs
                .iter()
                .find(|(view, _)| *view == active)
                .expect("the showing view has a tab");
            let fill_of = |box_: Panel, what: &str| {
                *out.scene
                    .rects
                    .iter()
                    .find(|r| r.xywh() == [box_.x, box_.y, box_.w, box_.h])
                    .unwrap_or_else(|| panic!("{space:?}: no {what} fill at {box_:?}"))
            };
            let pane = fill_of(placed.body, "pane");
            let showing = fill_of(*tab, "tab");
            assert_eq!(
                showing.rgba(),
                pane.rgba(),
                "{space:?}: {active:?} is a different colour from its pane"
            );
            // Flush, so there is no backdrop showing in a seam between them.
            assert!(
                (tab.y + tab.h - placed.body.y).abs() < 0.01,
                "{space:?}: the tab stops {} above the pane",
                placed.body.y - (tab.y + tab.h)
            );
            checked += 1;
        }
        assert_eq!(checked, 4, "only {checked} spaces had a showing tab");
    }

    /// The cut corner, on the fill and on the border alike. A square fill under
    /// a cut border leaves a triangle of panel colour outside its own edge.
    ///
    /// A pane is a fill on its own since item 12 took its top edge away, and the
    /// prompt is still a fill plus a stroke, so the count is per box rather than
    /// two everywhere.
    #[test]
    fn a_panel_is_cut_on_its_top_right_corner_only() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        let boxes: Vec<(Panel, usize)> = occupied(&dock)
            .iter()
            .map(|space| (out.layout.placed(*space).body, 1))
            .chain(std::iter::once((out.layout.input, 2)))
            .collect();
        for (panel, want) in boxes {
            let shaped: Vec<_> = out
                .scene
                .rects
                .iter()
                .filter(|r| r.xywh() == [panel.x, panel.y, panel.w, panel.h])
                .collect();
            assert_eq!(shaped.len(), want, "{panel:?} is not {want} rect(s)");
            for rect in shaped {
                let [_, chamfer, corners, _] = rect.extra();
                assert_eq!(chamfer, CUT, "{rect:?} is not cut");
                assert_eq!(corners, Rect::TOP_RIGHT as f32, "{rect:?} cuts elsewhere");
            }
        }
    }

    /// One shaded frame at a given surface size, in a given palette.
    ///
    /// The size is an argument because the surface a shaded window actually gets
    /// is the compositor's answer to a 30 pixel request, not the request.
    fn shaded_scene(state: &State, w: f32, h: f32, skin: &Skin) -> Scene {
        let dock = Dock::new();
        let mut shape = shape(&dock, &["a.rs"]);
        shape.shaded = true;
        let layout = Layout::compute(w, h, &shape);
        build(&Frame {
            state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &Monitor::new(),
            dock: &dock,
            skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot: None,
            trouble: None,
            esc_armed: false,
            popup_scroll: [0, 0],
            cursor: (-100.0, -100.0),
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        })
    }

    /// The height shading asks a window for is the height the strip needs, and
    /// it is a whole number of pixels because that is how a window is asked.
    #[test]
    fn the_strip_height_holds_the_line_the_strip_writes() {
        let line = Text::line_for(SMALL);
        assert!(
            strip_height() >= line,
            "a {} pixel strip cannot draw a {line} pixel line",
            strip_height()
        );
        assert_eq!(strip_height(), strip_height().ceil());
        // The layout gives a surface of that height a strip of exactly it, so
        // the number asked for and the number drawn are one number.
        let dock = Dock::new();
        let mut shape = shape(&dock, &[]);
        shape.shaded = true;
        let layout = Layout::compute(900.0, strip_height(), &shape);
        assert_eq!(layout.title.h, strip_height());
    }

    /// The regression this round was opened on: double clicking the title bar
    /// shaded the window and the strip lost every glyph it had, keeping only its
    /// green bar.
    ///
    /// The strip laid its writing out against [`TITLE_H`] whatever surface it
    /// was given: a 17 pixel line centred in 30, at y 6 to 23. glyphon clips a
    /// run to the surface as well as to the box it was handed, so a surface that
    /// came back under 23 pixels tall cut the line off from the bottom and one
    /// under about 10 drew none of it at all, while the bar rectangle still
    /// filled the surface because a rectangle is clamped rather than dropped.
    /// The name, the version, the build stamp and all three window buttons went
    /// together, which is every glyph a shaded window has.
    ///
    /// So this asserts the two halves that matter, at every height down to two
    /// pixels. Present: the name, the version and each of the three button
    /// codepoints are in the scene. And drawable: each of their boxes is inside
    /// the surface and as tall as the surface can give it, up to a whole line. A
    /// run present in the scene inside a box the window has no pixels for is
    /// exactly the bug, and a test that only asked whether the text was there
    /// would have passed straight through it.
    #[test]
    fn the_shaded_strip_writes_inside_the_surface_it_was_given() {
        let skin = Skin::from(&Config::default());
        let state = busy_state();
        let line = Text::line_for(SMALL);
        for height in [strip_height(), 24.0, 20.0, line, 12.0, 8.0, 2.0] {
            let scene = shaded_scene(&state, 900.0, height, &skin);
            let written = text_of(&scene);
            for wanted in [
                "NO0B",
                VERSION,
                &crate::design::icons::MINIMIZE.to_string(),
                &crate::design::icons::MAXIMIZE.to_string(),
                &crate::design::icons::CLOSE.to_string(),
            ] {
                assert!(written.contains(wanted), "{height}: the strip lost {wanted:?}");
            }
            // Every glyph in a shaded window belongs to the strip, so the rest
            // of this holds for the whole scene.
            assert_eq!(scene.texts.len(), 4, "{height}: the strip is four runs");
            for text in &scene.texts {
                let written: String = text.runs.iter().map(|run| run.text.as_str()).collect();
                assert!(
                    text.at.y >= 0.0 && text.at.y + text.at.h <= height + 0.01,
                    "{height}: {written:?} is written at {:?}, outside the surface",
                    text.at
                );
                assert_eq!(
                    text.at.h,
                    line.min(height),
                    "{height}: {written:?} was given {} of the {} the surface had",
                    text.at.h,
                    line.min(height)
                );
                // A box one line tall with a taller line box inside it is the
                // same loss one step down: the glyphs are laid out below the box
                // and clipped away.
                assert!(
                    text.line_height <= text.at.h + 0.01,
                    "{height}: {written:?} has a {} line in a {} box",
                    text.line_height,
                    text.at.h
                );
                assert!(text.at.w >= 1.0, "{height}: {written:?} has no width");
            }
        }
    }

    /// Item 19: shading collapsed the window to black rather than green unless
    /// it was maximized.
    ///
    /// Asking for a 30 pixel window is a request. Maximized the compositor grants
    /// it, and otherwise it keeps the surface it had, so everything under the
    /// strip was cleared to transparent and composited as black. Whatever surface
    /// comes back is filled in the bar's colour, so a tall one reads as a green
    /// bar and a short one is unchanged.
    #[test]
    fn a_shaded_surface_the_compositor_kept_tall_is_all_bar() {
        let skin = Skin::from(&Config::default());
        let state = busy_state();
        for height in [TITLE_H, 31.0, 120.0, 640.0] {
            let scene = shaded_scene(&state, 900.0, height, &skin);
            let bar: Vec<&Rect> = scene
                .rects
                .iter()
                .filter(|rect| rect.rgba() == skin.bar)
                .collect();
            assert_eq!(bar.len(), 1, "{height}: {} rects in the bar colour", bar.len());
            assert_eq!(
                bar[0].xywh(),
                [0.0, 0.0, 900.0, height],
                "{height}: the bar is not the whole surface"
            );
            assert_eq!(bar[0].extra()[3], 0.0, "{height}: the bar is an outline");

            // Nothing from the open window is under it: no pane, no prompt, no
            // gauge dot. A dot is the only round thing in a pane, and the orb's
            // discs are the only round thing in the strip.
            for (name, fill) in [
                ("pane", skin.panel),
                ("prompt", skin.input),
                ("backdrop", skin.backdrop),
            ] {
                assert!(
                    !scene.rects.iter().any(|r| r.rgba() == fill),
                    "{height}: a {name} is drawn under the shaded bar"
                );
            }
            for rect in &scene.rects {
                let [_, y, _, h] = rect.xywh();
                if rect.extra()[0] > 0.0 {
                    assert!(
                        y + h <= TITLE_H + 0.01,
                        "{height}: a gauge dot is drawn below the strip: {rect:?}"
                    );
                }
            }
            // And the strip's own contents are at the top of the bar rather than
            // spread down it.
            for text in &scene.texts {
                assert!(
                    text.at.y + text.at.h <= TITLE_H + 0.01,
                    "{height}: {:?} is written below the strip",
                    text.runs.iter().map(|run| run.text.as_str()).collect::<String>()
                );
            }
        }

        // The bar carries the window's transparency, so a surface that stayed
        // tall is as see-through as the strip is. A colour of its own here would
        // have made shading opaque at every opacity setting.
        let sheer = Skin::from(&Config {
            opacity: 0.2,
            ..Config::default()
        });
        let scene = shaded_scene(&state, 900.0, 500.0, &sheer);
        let bar = scene
            .rects
            .iter()
            .find(|rect| rect.xywh() == [0.0, 0.0, 900.0, 500.0])
            .expect("the surface is not filled");
        assert_eq!(bar.rgba(), sheer.bar, "the shaded bar is not the bar colour");
        assert!(bar.rgba()[3] < 1.0, "{:?} ignored the opacity", bar.rgba());
    }

    /// Shaded, the window is one bar. Every other region has to be gone, or
    /// a click lands on a pane that is not on screen.
    #[test]
    fn shading_leaves_the_bar_and_nothing_else() {
        let dock = Dock::new();
        let mut shape = shape(&dock, &["a.rs"]);
        shape.shaded = true;
        let layout = Layout::compute(1200.0, 800.0, &shape);
        assert!(layout.shaded);
        for space in Space::ALL {
            assert_eq!(layout.placed(space).body.h, 0.0);
            assert!(layout.placed(space).tabs.is_empty());
            assert_eq!(layout.placed(space).arrow_left.w, 0.0, "{space:?}");
            assert_eq!(layout.placed(space).arrow_right.w, 0.0, "{space:?}");
        }
        assert_eq!(layout.hit(600.0, 400.0), None);
        assert_eq!(layout.hit(600.0, 10.0), Some(Hit::TitleBar));

        let skin = Skin::from(&Config::default());
        let state = busy_state();
        let scene = shaded_scene(&state, 1200.0, 800.0, &skin);
        let text = text_of(&scene);
        assert!(text.contains("INFERRING"), "{text}");
        assert!(!text.contains("looking at it now"), "no pane content");

        // The surface the compositor kept is the bar, all 800 pixels of it, and
        // the strip's own contents are the only other thing on it. A backdrop
        // over that height was a black bar with a green strip across the top;
        // clearing it to transparent composited as the same black.
        let bar = scene
            .rects
            .iter()
            .find(|rect| rect.rgba() == skin.bar && rect.xywh()[3] > TITLE_H)
            .unwrap_or_else(|| panic!("the shaded surface is not filled: {:?}", scene.rects[0]));
        assert_eq!(bar.xywh(), [0.0, 0.0, 1200.0, 800.0], "the bar is not the surface");
        assert_eq!(
            scene.rects.iter().filter(|r| r.rgba() == skin.bar).count(),
            1,
            "the strip is painted over the bar, so it is a different alpha"
        );
        for rect in scene.rects.iter().filter(|r| r.xywh() != bar.xywh()) {
            let [_, y, _, h] = rect.xywh();
            assert!(
                y + h <= TITLE_H + 0.01,
                "{rect:?} reaches {} past the strip",
                y + h - TITLE_H
            );
        }
        for (name, fill) in [
            ("pane", skin.panel),
            ("prompt", skin.input),
            ("backdrop", skin.backdrop),
        ] {
            assert!(
                !scene.rects.iter().any(|r| r.rgba() == fill),
                "a {name} is drawn under the shaded bar"
            );
        }
    }

    #[test]
    fn the_buttons_win_against_the_title_bar_they_sit_on() {
        let out = render(&State::new(), 1200.0, 800.0, &Dock::new(), &[]);
        assert!(out.layout.title.contains(out.layout.close.x + 1.0, 10.0));
        assert_eq!(
            out.layout.hit(out.layout.close.x + 1.0, 10.0),
            Some(Hit::Close)
        );
        assert_eq!(out.layout.hit(200.0, 10.0), Some(Hit::TitleBar));
    }

    /// A strip drops the tabs it cannot hold rather than squeezing them into
    /// slivers, and grows the two arrows that reach them. This used to be
    /// asserted on the file strip, which no longer exists: a list too long for
    /// its pane scrolls now, and
    /// `a_list_longer_than_the_pane_scrolls_instead_of_dropping_files` is what
    /// says so.
    #[test]
    fn tabs_that_do_not_fit_are_dropped_not_squeezed() {
        // Every view but one in the left space, which is more than its strip can
        // hold. The one left behind keeps the space split, so the strip is the
        // width it usually is rather than the whole window.
        let mut dock = a_crowded_dock(Space::TopLeft);
        dock.move_view(View::Files, Space::TopRight);
        dock.move_view(View::Output, Space::TopLeft);
        let out = render(&busy_state(), 900.0, 700.0, &dock, &["calc.py"]);
        let placed = out.layout.placed(Space::TopLeft);
        let tabs = &placed.tabs;
        assert!(tabs.len() < View::ALL.len(), "every tab fitted");
        for (_, panel) in tabs {
            assert!(panel.w > 20.0, "no slivers: {panel:?}");
        }
        assert!(
            placed.arrow_left.w >= 1.0 && placed.arrow_right.w >= 1.0,
            "the tabs it dropped cannot be reached"
        );
    }

    /// A tab strip's widths, in label characters, for driving [`strip_tabs`]
    /// without a window. Six, the number the top right space opens with.
    const LABELS: [usize; 6] = [8, 4, 6, 8, 7, 7];

    /// What one of those tabs is drawn at, in pixels, at `COLUMN` 8: three
    /// columns of padding around the label.
    fn tab_w(chars: usize) -> f32 {
        (chars as f32 + 3.0) * 8.0
    }

    /// A strip that holds all of its tabs shows no arrows and loses no room to
    /// them, whatever it was asked to scroll to. The offset only means anything
    /// while there is something off the edge.
    #[test]
    fn a_strip_that_fits_has_no_arrows_and_is_never_scrolled() {
        let total: f32 = LABELS.iter().copied().map(tab_w).sum();
        let bar = Panel::new(10.0, 30.0, total + 1.0, TAB_H);
        for asked in [0, 1, 5, 99] {
            let laid = strip_tabs(bar, &LABELS, 8.0, asked, Some(0));
            assert_eq!(laid.tabs.len(), LABELS.len(), "asked for {asked}");
            assert_eq!(laid.first, 0, "asked for {asked}");
            assert_eq!(laid.left.w, 0.0, "asked for {asked}");
            assert_eq!(laid.right.w, 0.0, "asked for {asked}");
        }
        // One pixel narrower and the last tab is off the edge, which is what the
        // arrows are for.
        let tight = Panel::new(10.0, 30.0, total - 1.0, TAB_H);
        let laid = strip_tabs(tight, &LABELS, 8.0, 0, Some(0));
        assert!(laid.left.w >= 1.0 && laid.right.w >= 1.0);
    }

    /// The arrows take their room before the window of tabs is chosen. Taking it
    /// afterwards would push one more tab off the edge, which is the same
    /// complaint one tab further along, and the tab at the end would be drawn
    /// under the arrow that is supposed to reach it.
    #[test]
    fn the_arrows_take_their_room_before_the_tabs_are_placed() {
        let bar = Panel::new(10.0, 30.0, 300.0, TAB_H);
        let laid = strip_tabs(bar, &LABELS, 8.0, 0, Some(0));
        assert!(laid.left.w >= 1.0, "300 pixels does not hold six tabs");
        let last = *laid.tabs.last().expect("some tabs fit");
        assert!(
            last.x + last.w <= laid.left.x + 0.01,
            "the last tab {last:?} runs under the arrow at {:?}",
            laid.left
        );
        // The pair sits at the right end of the strip, in reading order.
        assert_eq!(laid.left.x + laid.left.w, laid.right.x);
        assert_eq!(laid.right.x + laid.right.w, bar.x + bar.w);
        assert_eq!(laid.left.h, bar.h);
        // And each is a target, not a hairline.
        assert!(laid.left.w >= 20.0, "{:?}", laid.left);
    }

    /// The clamp, tested hardest: whatever offset a space carries, and however
    /// narrow the strip has become since, the strip shows tabs. A space left
    /// scrolled past its last tab is the empty strip a resize or a closed tab
    /// would otherwise leave behind.
    #[test]
    fn a_strip_is_never_scrolled_past_the_tabs_it_can_show() {
        for room in [180.0, 200.0, 260.0, 300.0, 420.0, 500.0, 700.0] {
            let bar = Panel::new(0.0, 0.0, room, TAB_H);
            for asked in 0..12 {
                let laid = strip_tabs(bar, &LABELS, 8.0, asked, None);
                let at = laid.first;
                assert!(
                    !laid.tabs.is_empty(),
                    "{room} pixels, asked for {asked}: an empty strip"
                );
                assert!(at <= asked, "{room}, {asked}: walked past what it was asked");
                assert!(
                    at + laid.tabs.len() <= LABELS.len(),
                    "{room}, {asked}: {} tabs from {at} is past the end",
                    laid.tabs.len()
                );
                // Asked for more than there is, it lands where the last tab
                // shows rather than off the end of the strip.
                if asked >= LABELS.len() {
                    assert_eq!(
                        at + laid.tabs.len(),
                        LABELS.len(),
                        "{room}, {asked}: {} tabs from {at} leaves the end unreachable",
                        laid.tabs.len()
                    );
                }
            }
        }
    }

    /// The showing tab is always in its own strip, whatever the space is scrolled
    /// to. Otherwise the keyboard walk, or a drop into another space, leaves a
    /// pane on screen whose tab is not.
    #[test]
    fn the_showing_tab_is_always_in_its_own_strip() {
        for room in [180.0, 260.0, 340.0, 500.0] {
            let bar = Panel::new(0.0, 0.0, room, TAB_H);
            for active in 0..LABELS.len() {
                for asked in 0..LABELS.len() + 2 {
                    let laid = strip_tabs(bar, &LABELS, 8.0, asked, Some(active));
                    let showing = laid.first..laid.first + laid.tabs.len();
                    assert!(
                        showing.contains(&active),
                        "{room} pixels, tab {active} showing, scrolled to {asked}: \
                         the strip holds {showing:?}"
                    );
                }
            }
        }
    }

    /// Item 18's own report: a space full of tabs, and a window narrowed until
    /// they will not all fit. Nothing disappears without a way back to it, and
    /// widening the window again brings every tab back rather than leaving the
    /// strip where it was scrolled to.
    #[test]
    fn narrowing_the_window_puts_the_tabs_behind_arrows_rather_than_losing_them() {
        let mut dock = a_crowded_dock(Space::TopRight);
        let all = dock.slot(Space::TopRight).views.len();
        let roomy = Layout::compute(1400.0, 900.0, &shape(&dock, &[]));
        let placed = roomy.placed(Space::TopRight);
        assert_eq!(placed.tabs.len(), all, "every tab fits in a wide window");
        assert_eq!(placed.arrow_left.w, 0.0, "arrows on a strip that fits");

        // At the smallest the window goes, most of them are off the edge.
        let narrow = Layout::compute(680.0, 380.0, &shape(&dock, &[]));
        let placed = narrow.placed(Space::TopRight);
        assert!(placed.tabs.len() < all, "every tab fitted at 680 pixels");
        assert!(!placed.tabs.is_empty());
        assert!(placed.arrow_left.w >= 1.0 && placed.arrow_right.w >= 1.0);

        // Walked to the end, the last tab is on screen and the strip is full.
        let last = *dock.slot(Space::TopRight).views.last().unwrap();
        dock.slot_mut(Space::TopRight).scroll_tabs(all - 1);
        dock.slot_mut(Space::TopRight).show(last);
        let scrolled = Layout::compute(680.0, 380.0, &shape(&dock, &[]));
        let placed = scrolled.placed(Space::TopRight);
        assert!(
            placed.tabs.iter().any(|(view, _)| *view == last),
            "the last tab is still out of reach: {:?}",
            placed.tabs
        );
        assert_eq!(placed.first_tab + placed.tabs.len(), all);

        // Wide again, with that offset still stored: every tab is back and the
        // strip is not left scrolled.
        let roomy = Layout::compute(1400.0, 900.0, &shape(&dock, &[]));
        let placed = roomy.placed(Space::TopRight);
        assert_eq!(placed.tabs.len(), all);
        assert_eq!(placed.first_tab, 0);
        assert_eq!(placed.arrow_left.w, 0.0);
    }

    /// Every tab carries its own view's label. The tabs are the laid out panels
    /// zipped with the space's views, and a scrolled strip starts at the tab it
    /// is scrolled to, so zipping by position alone would put the wrong label on
    /// every tab of it.
    #[test]
    fn a_scrolled_strip_labels_every_tab_with_its_own_view() {
        let mut dock = Dock::new();
        for asked in 0..dock.slot(Space::TopRight).views.len() {
            dock.slot_mut(Space::TopRight).scroll_tabs(asked);
            let views = dock.slot(Space::TopRight).views.clone();
            let layout = Layout::compute(680.0, 380.0, &shape(&dock, &[]));
            let placed = layout.placed(Space::TopRight);
            for (step, (view, panel)) in placed.tabs.iter().enumerate() {
                assert_eq!(
                    *view,
                    views[placed.first_tab + step],
                    "scrolled to {asked}: tab {step} names the wrong view"
                );
                // As wide as its own label, and left to right in order.
                assert!(panel.w >= tab_w(view.label().chars().count()) - 0.01);
                if step > 0 {
                    let before = placed.tabs[step - 1].1;
                    assert!(panel.x >= before.x + before.w - 0.01);
                }
            }
            // A tab that is scrolled out has no panel at all, so nothing can be
            // dragged by it and nothing indexes past the end.
            assert!(placed.first_tab + placed.tabs.len() <= views.len());
        }
    }

    /// Item 23's arithmetic: which place in a strip a drop takes. The left half
    /// of a tab is in front of it, the right half is behind it, and past the last
    /// tab is behind the lot.
    #[test]
    fn a_drop_lands_in_front_of_the_tab_its_left_half_is_under() {
        let dock = Dock::new();
        let layout = Layout::compute(1400.0, 900.0, &shape(&dock, &[]));
        let placed = layout.placed(Space::TopRight);
        let tabs = dock.slot(Space::TopRight).views.len();
        assert_eq!(placed.tabs.len(), tabs, "the whole strip is on screen");
        assert_eq!(placed.first_tab, 0);

        for (step, (_, panel)) in placed.tabs.iter().enumerate() {
            assert_eq!(
                layout.insertion(Space::TopRight, panel.x + 1.0),
                step,
                "the left edge of tab {step}"
            );
            assert_eq!(
                layout.insertion(Space::TopRight, panel.x + panel.w - 1.0),
                step + 1,
                "the right edge of tab {step}"
            );
        }
        // In front of the first tab, and past the end of them all.
        assert_eq!(layout.insertion(Space::TopRight, placed.strip.x - 20.0), 0);
        assert_eq!(
            layout.insertion(Space::TopRight, placed.strip.x + placed.strip.w),
            tabs
        );
        // A space with no tabs has one place: the start.
        let mut empty = Dock::new();
        assert!(empty.hide(View::Files));
        let layout = Layout::compute(1400.0, 900.0, &shape(&empty, &[]));
        assert_eq!(layout.insertion(Space::BottomRight, 500.0), 0);
    }

    /// A scrolled strip names the tab the pointer is over, not the tab that many
    /// places from the left of the strip. Counting the panels alone is what would
    /// drop a tab five places from where it was let go.
    #[test]
    fn a_scrolled_strip_names_the_place_the_pointer_is_over() {
        let mut dock = Dock::new();
        let all = dock.slot(Space::TopRight).views.len();
        for asked in 0..all {
            dock.slot_mut(Space::TopRight).scroll_tabs(asked);
            let layout = Layout::compute(680.0, 380.0, &shape(&dock, &[]));
            let placed = layout.placed(Space::TopRight);
            assert!(!placed.tabs.is_empty());
            for (step, (_, panel)) in placed.tabs.iter().enumerate() {
                let at = layout.insertion(Space::TopRight, panel.x + 1.0);
                assert_eq!(
                    at,
                    placed.first_tab + step,
                    "scrolled to {asked}: tab {step} names the wrong place"
                );
                // And the place it names is a place in the space's own tabs.
                assert!(at <= all, "scrolled to {asked}: {at} is past the end");
            }
            let end = layout.insertion(
                Space::TopRight,
                placed.strip.x + placed.strip.w,
            );
            assert_eq!(end, placed.first_tab + placed.tabs.len());
            assert!(end <= all);
        }
    }

    /// The arrows are hit tested in the strip, beside the tabs, rather than on
    /// the floating layer. Without their own regions the strip behind them
    /// answers, and a click on one would land in the pane's body instead.
    #[test]
    fn an_arrow_is_hit_tested_in_the_strip_beside_the_tabs() {
        let dock = a_crowded_dock(Space::TopRight);
        let layout = Layout::compute(680.0, 380.0, &shape(&dock, &[]));
        let placed = layout.placed(Space::TopRight);
        for (panel, hit) in [
            (placed.arrow_left, Hit::TabsLeft(Space::TopRight)),
            (placed.arrow_right, Hit::TabsRight(Space::TopRight)),
        ] {
            assert!(panel.w >= 1.0);
            let (x, y) = middle(panel);
            assert_eq!(layout.hit(x, y), Some(hit));
            assert!(placed.strip.contains(x, y), "the arrow is in the strip");
            // A drop lands in the space the arrow belongs to, the way it does
            // anywhere else in that strip.
            assert_eq!(hit.space(), Some(Space::TopRight));
            // In the strip, so the drop names a place among the tabs: behind the
            // last one on screen, which is what the arrows stand in front of.
            let placed = layout.placed(Space::TopRight);
            assert_eq!(
                layout.landing(x, y),
                Landing::In(
                    Space::TopRight,
                    Some(placed.first_tab + placed.tabs.len())
                )
            );
        }
        // A strip that fits has no arrow to hit: the point they would be at is
        // the space itself.
        let wide = Layout::compute(1400.0, 900.0, &shape(&dock, &[]));
        let strip = wide.placed(Space::TopRight).strip;
        let at = (strip.x + strip.w - 4.0, strip.y + strip.h * 0.5);
        assert_eq!(wide.hit(at.0, at.1), Some(Hit::Body(Space::TopRight)));
    }

    /// Both arrows are drawn, as the glyphs the symbol font carries, and the
    /// direction with nowhere left to go is dimmed rather than taken away.
    #[test]
    fn the_arrows_are_drawn_and_the_spent_one_is_dimmed() {
        let mut dock = a_crowded_dock(Space::TopRight);
        let out = render(&busy_state(), 680.0, 380.0, &dock, &[]);
        let arrows: Vec<&Run> = out
            .scene
            .texts
            .iter()
            .flat_map(|text| text.runs.iter())
            .filter(|run| {
                run.text == icons::TABS_LEFT.to_string()
                    || run.text == icons::TABS_RIGHT.to_string()
            })
            .collect();
        assert_eq!(arrows.len(), 2, "the pair is not drawn");
        // The first tab is showing, so there is nothing to the left of it.
        let left = arrows
            .iter()
            .find(|run| run.text == icons::TABS_LEFT.to_string())
            .unwrap();
        let right = arrows
            .iter()
            .find(|run| run.text == icons::TABS_RIGHT.to_string())
            .unwrap();
        assert_eq!(left.color, Some(out.skin.dim), "nothing is off to the left");
        assert_eq!(right.color, Some(out.skin.bright));

        // Showing the last tab turns the pair round.
        let last = *dock.slot(Space::TopRight).views.last().unwrap();
        dock.slot_mut(Space::TopRight).show(last);
        let out = render(&busy_state(), 680.0, 380.0, &dock, &[]);
        let tints: Vec<(String, Option<[u8; 4]>)> = out
            .scene
            .texts
            .iter()
            .flat_map(|text| text.runs.iter())
            .filter(|run| {
                run.text == icons::TABS_LEFT.to_string()
                    || run.text == icons::TABS_RIGHT.to_string()
            })
            .map(|run| (run.text.clone(), run.color))
            .collect();
        assert_eq!(
            tints,
            vec![
                (icons::TABS_LEFT.to_string(), Some(out.skin.bright)),
                (icons::TABS_RIGHT.to_string(), Some(out.skin.dim)),
            ]
        );
    }

    #[test]
    fn the_resize_edges_are_the_border_and_nothing_else() {
        use winit::window::ResizeDirection as Dir;
        assert_eq!(edge(0.0, 0.0, 800.0, 600.0), Some(Dir::NorthWest));
        assert_eq!(edge(799.0, 599.0, 800.0, 600.0), Some(Dir::SouthEast));
        assert_eq!(edge(400.0, 300.0, 800.0, 600.0), None);
    }

    #[test]
    fn a_file_tab_says_enough_to_tell_two_of_them_apart() {
        assert_eq!(short_name("src/calc.py"), "calc.py");
        assert_eq!(short_name("crates/noob/src/mod.rs"), "src/mod.rs");
        assert_eq!(short_name("README"), "README");
    }

    #[test]
    fn a_deep_workspace_shows_its_last_two_segments() {
        assert_eq!(
            short_path("/home/hec/workspace/noob-cli"),
            "workspace/noob-cli"
        );
        assert_eq!(short_path("noob-cli"), "noob-cli");
    }





























    /// A tab thrown out of the window is its own answer, not a miss: there is
    /// nowhere outside to put a pane, and a tab that snaps back after being
    /// thrown away is the more surprising of the two readings.
    #[test]
    fn a_tab_released_off_the_window_lands_out() {
        let dock = Dock::new();
        let (w, h) = (1400.0, 900.0);
        let layout = Layout::compute(w, h, &shape(&dock, &[]));
        for (x, y) in [
            (-1.0, 400.0),
            (w + 1.0, 400.0),
            (700.0, -1.0),
            (700.0, h + 1.0),
        ] {
            assert_eq!(layout.landing(x, y), Landing::Out, "at {x},{y}");
        }
        // Inside a cell the drop names that cell and no place in any strip. The
        // conversation covers both cells of the left column, and each half of it
        // still names its own cell: that is what takes a span back apart.
        let (x, y) = middle(layout.grid[Space::TopLeft.index()]);
        assert_eq!(layout.landing(x, y), Landing::In(Space::TopLeft, None));
        let (x, y) = middle(layout.grid[Space::BottomLeft.index()]);
        assert_eq!(layout.landing(x, y), Landing::In(Space::BottomLeft, None));
        // A tab does name a place: the middle of the first tab is its right half,
        // so a drop there goes behind it.
        let (x, y) = middle(layout.placed(Space::TopRight).tabs[0].1);
        assert_eq!(layout.landing(x, y), Landing::In(Space::TopRight, Some(1)));
        // Inside the window but on nothing that holds panes.
        assert_eq!(layout.landing(400.0, 10.0), Landing::Nowhere);
    }

    /// Closing the only widget in a space leaves that space with no tabs, which
    /// the layout has to read as room to give away rather than as a hole.
    #[test]
    fn an_emptied_space_gives_its_room_away() {
        let full = Layout::compute(1400.0, 900.0, &shape(&Dock::new(), &[]));
        let mut dock = Dock::new();
        // The bottom right space's only tab.
        assert!(dock.hide(View::Agents));
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &[]);

        assert_eq!(out.layout.placed(Space::BottomRight).body.h, 0.0);
        assert!(out.layout.placed(Space::BottomRight).tabs.is_empty());
        assert!(
            out.layout.placed(Space::TopRight).body.h
                > full.placed(Space::TopRight).body.h + TAB_H,
            "the space above it took the room"
        );
        // The left column is untouched and the prompt is still there.
        assert_eq!(
            out.layout.placed(Space::TopLeft).body,
            full.placed(Space::TopLeft).body
        );
        assert_eq!(out.layout.input, full.input);

        // And with everything closed the window is empty rather than broken.
        let empty = Dock::hiding(&View::ALL);
        let out = render(&busy_state(), 1400.0, 900.0, &empty, &[]);
        for space in Space::ALL {
            assert!(out.layout.placed(space).tabs.is_empty(), "{space:?}");
        }
        assert!(out.layout.input.h > 0.0, "the prompt survives");
        // The room the panes had answers as the window rather than as a space
        // with nothing in it, which is what a right click there opens the
        // window's own menu from.
        assert_eq!(out.layout.hit(700.0, 450.0), Some(Hit::Window));
    }




























































































































}
