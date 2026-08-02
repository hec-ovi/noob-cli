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

use noob_draw::{Panel, Rect, Run, Scene, Text};

use crate::dock::{Dock, Space, View};
use crate::design::icons;
use crate::menu::{MARKER_COLUMNS, Menu};
use crate::monitor::Monitor;
use crate::picker::Picker;
use crate::settings::{Act, Settings, Side};
use crate::skin::Skin;
use crate::state::State;
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
const SMALL: f32 = 12.0;
pub(crate) const SCROLL_W: f32 = 4.0;
const BUTTON_W: f32 = 26.0;
/// The square at the left end of the title strip that the orb is drawn in.
///
/// The strip's text starts after this, so the orb sits in a slot of its own
/// instead of over the name, and the strip reads
/// `[orb] NO0B \u{25b8} version` left to right. The orb sizes itself to whatever
/// square it is handed, so this is the only number that decides how big it is.
pub const ORB_W: f32 = TITLE_H;

/// How tall a window has to be to be a title strip and nothing else.
///
/// What shading asks the window for, in the space the layout works in. That
/// space is physical pixels: [`Layout::compute`] is handed the surface
/// configuration `noob-gpu` reports, which is `Window::inner_size` verbatim, and
/// nothing between winit and here applies a scale factor. So the number a window
/// is asked for is this number, not this number through a conversion.
///
/// [`TITLE_H`] and never less than the line the strip writes, because a strip
/// too short to draw its own name is not a strip. Whole pixels, rounded up: a
/// window is asked for in integers and a request half a pixel short would come
/// back half a pixel short.
pub fn strip_height() -> f32 {
    TITLE_H.max(Text::line_for(SMALL)).ceil()
}

/// The box the title strip writes one line into, given the strip it actually
/// has.
///
/// Every run in the strip goes through here, so none of them can be written
/// outside the surface. glyphon clips a run to the surface as well as to the box
/// it was given, so a 17 pixel line centred in a 30 pixel box is drawn nowhere
/// at all once the surface comes back 12 pixels tall: the strip kept its bar and
/// lost the name, the version, the build stamp and all three window buttons,
/// which is every glyph it has. A strip shorter than a line keeps its line at
/// the top and gives it every pixel there is instead, because the writing is
/// what a strip is for and is the last thing that should go.
fn strip_row(panel: Panel) -> Panel {
    panel.row(0.0, Text::line_for(SMALL))
}

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
/// The size a menu's rows are written at, and the size its box is measured in
/// columns of.
///
/// Public because the box's width is a character count times the width of one
/// column at this size, and only the renderer can measure a column. It used to
/// be measured at the title bar's size while being drawn at this one, which at
/// the defaults left every row about 23 pixels short of its own box and put the
/// group chevron most of an inch past the end of its label. Two sizes, two
/// column widths, and the one that owns the geometry is the one the text is in.
pub const MENU_SIZE: f32 = SMALL;
/// One row of a menu. Taller than a tab: a tab is read, a menu row is aimed at,
/// and 22 pixels is already tight for a pointer.
const MENU_ROW_H: f32 = 24.0;
/// The border hairline a menu's rows sit flush against, top and bottom: the
/// lit row's band runs to the frame, with no dark strip between them.
const MENU_EDGE: f32 = 1.0;
/// The margin around a menu's rows, top and bottom and on either side of a
/// label. Also what keeps the first row off the pointer that opened it.
const MENU_PAD: f32 = 5.0;
/// Columns every menu row leaves in front of its label for an icon, whether it
/// has one or not, so labels line up in a column instead of stepping in and out
/// with whichever rows happen to be marked.
const MENU_GUTTER: usize = 2;
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

/// The least width a column can be dragged down to, for text drawn at `column`
/// pixels a character: [`DIFF_MIN_COLUMNS`] of them, which is the floor the file
/// view already refuses to go below (a line-number gutter and twenty columns of
/// code beside it), plus the [`PAD`] on either side of a pane's content.
///
/// It moves with the font size, because the floor is about columns of text
/// rather than about pixels: the same 24 columns cost more room at 20 point.
fn min_column_w(column: f32) -> f32 {
    DIFF_MIN_COLUMNS as f32 * column.max(1.0) + PAD * 2.0
}

/// A divider's ratio, held so neither side of it ends up smaller than `floor`.
///
/// `room` is what the two sides share once the gap between them is taken off. A
/// box with no room for two floors and the gap splits down the middle: both
/// sides are then equally short of what they wanted, which is the only answer
/// that does not collapse one of them to nothing.
pub(crate) fn held(ratio: f32, room: f32, floor: f32) -> f32 {
    if room <= floor * 2.0 {
        return 0.5;
    }
    let edge = floor / room;
    ratio.clamp(edge, 1.0 - edge)
}



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
    fn live(self) -> bool {
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

pub(crate) fn nowhere() -> Panel {
    Panel::new(0.0, 0.0, 0.0, 0.0)
}

/// The box around two boxes. A box with nothing in it is not a corner of the
/// answer: two cells and one empty cell would otherwise reach back to the
/// window's origin.
fn around(a: Panel, b: Panel) -> Panel {
    if a.w < 1.0 || a.h < 1.0 {
        return b;
    }
    if b.w < 1.0 || b.h < 1.0 {
        return a;
    }
    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
    Panel::new(
        x,
        y,
        (a.x + a.w).max(b.x + b.w) - x,
        (a.y + a.h).max(b.y + b.h) - y,
    )
}

/// The four cells of the grid, in [`Space::ALL`] order, from the box the panes
/// share and the two ratios.
///
/// One line across the whole box and then a line inside each half of it, so the
/// left column and the right one can break at heights of their own. Which axis
/// gets the single line is `rows_first`: a grid reading in columns is cut down
/// the middle once and each column is then cut across, and a grid reading in
/// rows is cut across once and each row is then cut down.
///
/// The pair on the axis with the single line is not read past its first number.
/// It is still carried, because turning the grid round makes the second one live
/// again and a number that was thrown away comes back as the default rather than
/// as where the line was left.
///
/// Every ratio is held off the same floors the dividers are dragged with, so a
/// cell is never narrower than a file view can be read in nor shorter than a
/// gauge can be drawn in.
fn grid_cells(
    body: Panel,
    rows_first: bool,
    left_width: [f32; 2],
    top_height: [f32; 2],
    column_floor: f32,
) -> [Panel; 4] {
    let room_w = (body.w - GAP).max(0.0);
    let room_h = (body.h - GAP).max(0.0);
    // The width of the left cell of a row, and the height of the top cell of a
    // column. One answer for every row, or for every column, on whichever axis
    // the single line runs.
    let left_w = |row: usize| {
        let ratio = left_width[if rows_first { row } else { 0 }];
        (room_w * held(ratio, room_w, column_floor)).floor()
    };
    let top_h = |column: usize| {
        let ratio = top_height[if rows_first { 0 } else { column }];
        (room_h * held(ratio, room_h, MIN_SPACE_H)).floor()
    };
    let cell = |row: usize, column: usize| {
        let (left, top) = (left_w(row), top_h(column));
        let (x, w) = match column {
            0 => (body.x, left),
            _ => (body.x + left + GAP, body.w - left - GAP),
        };
        let (y, h) = match row {
            0 => (body.y, top),
            _ => (body.y + top + GAP, body.h - top - GAP),
        };
        Panel::new(x, y, w, h)
    };
    [cell(0, 0), cell(1, 0), cell(0, 1), cell(1, 1)]
}

fn empty_placed() -> Placed {
    Placed {
        strip: nowhere(),
        body: nowhere(),
        tabs: Vec::new(),
        first_tab: 0,
        arrow_left: nowhere(),
        arrow_right: nowhere(),
    }
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
        None
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

/// The boxes an open menu puts on screen, and where every row on screen is
/// inside them: the menu's own column, and the widgets flyout while open.
struct MenuPlaces {
    box_: Panel,
    rows: Vec<(usize, Panel)>,
    fly: Panel,
    fly_rows: Vec<(usize, Panel)>,
}

/// Where an open menu's box is, and where each of its rows on screen is inside
/// it.
///
/// Clamped into the window. A menu opened near the right edge or a row from the
/// bottom would otherwise hang off the surface, and the part that hangs off is
/// not merely invisible: no pointer can reach it, so the rows down there cannot
/// be picked at all.
///
/// Two boxes at most: the menu's own column, and the widgets flyout beside
/// its header while it is open. The flyout hangs off the header's row, out
/// to the right, or out to the left when the right has no room; its rows
/// carry their global place in the same menu.
///
/// The rows sit flush against the box's border: the first row starts at the
/// hairline and the last ends at it, so a lit row's band runs to the frame
/// with no dark strip breaking it. The padding a menu needs is inside the
/// row, in front of its text, not around the block of rows.
///
/// The window is the last word on both axes. A menu wider than the surface is
/// cut to it rather than run off the right edge, and one with more rows than
/// the surface is tall shows as many as there is room for and scrolls through
/// the rest ([`Menu::first`]), which is what stops a long menu from silently
/// dropping the rows past the bottom.
///
/// `column` is the width of one column at [`MENU_SIZE`], which is the size the
/// rows are written at. Anything else and the box is measured in one font and
/// filled in another.
fn place_menu(menu: &Menu, column: f32, width: f32, height: f32) -> MenuPlaces {
    let column = column.max(1.0);
    let main = menu.main_len();
    // One column of slack past the measured width, in both boxes: the icon
    // glyphs come from the symbol font and can run a hair wider than a text
    // column, and a row measured to an exact fit wraps its label out of its
    // one-line row, which draws as a row with no name at all.
    let slack = 1;
    let w = ((menu.width_chars() + MENU_GUTTER + slack) as f32 * column + MENU_PAD * 2.0)
        .min(width.max(1.0));
    let room = (((height - MENU_EDGE * 2.0) / MENU_ROW_H).floor() as usize).max(1);
    let shown = main.min(room);
    let h = shown as f32 * MENU_ROW_H + MENU_EDGE * 2.0;
    let x = menu.at.0.min(width - w).max(0.0);
    let y = menu.at.1.min(height - h).max(0.0);
    // Where in the menu the box starts. Clamped here rather than in the model,
    // so a wheel that ran past the end does not leave the box half empty.
    let first = menu.first.min(main.saturating_sub(shown));
    let row_at = |box_x: f32, box_y: f32, box_w: f32, step: usize| {
        Panel::new(
            box_x,
            box_y + MENU_EDGE + step as f32 * MENU_ROW_H,
            box_w,
            MENU_ROW_H,
        )
    };
    let rows: Vec<(usize, Panel)> = (0..shown)
        .map(|step| (first + step, row_at(x, y, w, step)))
        .collect();
    let box_ = Panel::new(x, y, w, h);
    let (fly, fly_rows) = match menu.fly_start {
        None => (nowhere(), Vec::new()),
        Some(fly_start) => {
            let count = menu.rows.len() - fly_start;
            let fw = ((menu.fly_width_chars() + MENU_GUTTER + slack) as f32 * column
                + MENU_PAD * 2.0)
                .min(width.max(1.0));
            let fh = count as f32 * MENU_ROW_H + MENU_EDGE * 2.0;
            // Top-aligned with the header's row, on whichever side has room.
            let anchor = menu.fly_anchor().unwrap_or(first);
            let anchor_y = rows
                .iter()
                .find(|(index, _)| *index == anchor)
                .map(|(_, panel)| panel.y - MENU_EDGE)
                .unwrap_or(y);
            let fx = match x + w + fw <= width {
                true => x + w,
                false => (x - fw).max(0.0),
            };
            let fy = anchor_y.min(height - fh).max(0.0);
            let fly_rows = (0..count)
                .map(|step| (fly_start + step, row_at(fx, fy, fw, step)))
                .collect();
            (Panel::new(fx, fy, fw, fh), fly_rows)
        }
    };
    MenuPlaces {
        box_,
        rows,
        fly,
        fly_rows,
    }
}

/// Whether a point is inside the notch cut out of a box's top right corner:
/// pixels the box does not paint and must not answer for.
///
/// The shader removes every point where the distance in from the right and the
/// distance down from the top add up to less than the cut ([`cut_of`]). Nothing
/// else in this window asks: [`Panel::contains`] is a bare rectangle and every
/// other surface has the pane behind it, so a click in the notch lands on the
/// same thing it looks like it landed on. A menu floats over the window and
/// takes the click before anything else, so its notch answered for its first
/// row: a press on transparent pixels opened the settings panel.
fn in_cut(panel: Panel, x: f32, y: f32) -> bool {
    (panel.x + panel.w - x) + (y - panel.y) < cut_of(panel)
}

/// Where the activity popup sits: the whole surface under the title strip,
/// a margin in from every edge.
///
/// It was a floating note sized to its lines. Full panel now, so the call's
/// blocks have the room a pretty-printed argument object and a stack trace
/// need, and the window behind it stops competing with them. Nothing inside
/// it scrolls; the cells are bounded at the source (`state::CELL_LINES`).
fn place_popup(width: f32, height: f32) -> Panel {
    let margin = 2.0 * GAP;
    Panel::new(
        margin,
        TITLE_H + margin,
        (width - margin * 2.0).max(1.0),
        (height - TITLE_H - margin * 2.0).max(1.0),
    )
}

/// One row per file, as heights the scroll window can be taken from.
///
/// The explorer clips a name that does not fit rather than wrapping it, so a row
/// is always exactly one row. That is what keeps a click from resolving to a
/// different file than the one under the pointer. Written as heights, and read
/// through [`text_geometry`], so the window and the clamp come from the one
/// place that owns them rather than from arithmetic at two call sites.
pub fn file_heights(count: usize) -> Vec<usize> {
    flat_heights(count)
}

/// The file view's two columns, and where each visible row of the list is.
///
/// The list is as wide as the longest name it holds and no wider, capped twice:
/// at [`LIST_MAX_COLUMNS`], and at whatever leaves the file [`DIFF_MIN_COLUMNS`]
/// to be read in. The file is the thing being looked at, so it is the half with
/// the floor; below the size where even that cannot be met the two split what
/// there is, because a pane that hid either half would be worse than a cramped
/// one.
fn place_files(body: Panel, shape: &Shape) -> (Panel, Panel, Vec<(usize, Panel)>) {
    if body.w < 1.0 || body.h < 1.0 {
        return (nowhere(), nowhere(), Vec::new());
    }
    // Nothing touched yet: no column, no divider, and the pane says so where the
    // file would be.
    if shape.file_labels.is_empty() {
        return (nowhere(), body, Vec::new());
    }
    let column = shape.pane_column.max(1.0);
    let total = cols_of(body, column);
    let widest = shape
        .file_labels
        .iter()
        .map(|label| label.chars().count())
        .max()
        .unwrap_or(0);
    let want = (widest + ROW_ICON_COLUMNS + ROW_MARK_COLUMNS).clamp(LIST_MIN_COLUMNS, LIST_MAX_COLUMNS);
    // Two columns cost two sets of margins, so the split itself spends columns
    // before either half gets a character. Leaving that out is how the file
    // ended up two columns under its floor at the smallest window size.
    let split_cost = (PAD * 2.0 / column).ceil() as usize;
    let cols = match total.checked_sub(DIFF_MIN_COLUMNS + split_cost) {
        Some(room) if room >= 1 => want.min(room),
        // Nothing to protect at this size. The two halves split what there is,
        // rather than one of them disappearing: a file view with no list cannot
        // be navigated, and a list with no file shows nothing.
        _ => (total / 2).max(1),
    };
    let (list, diff) = body.split_left((cols as f32 * column + PAD * 2.0).min(body.w));

    let line = Text::line_for(shape.pane_size);
    let content = list.inset(PAD);
    let rows = Text::rows_for(shape.pane_size, content.h);
    let heights = file_heights(shape.file_labels.len());
    let back = text_geometry::scrollback_for(&heights, rows, shape.file_first);
    let window = text_geometry::window(&heights, rows, back);
    let panels = (0..window.count)
        .map(|step| {
            // The full width of the column, so the whole row answers the click
            // the way a row of an explorer does, not just the characters of the
            // name.
            let index = window.first + step;
            (
                index,
                Panel::new(list.x, content.y + step as f32 * line, list.w, line),
            )
        })
        .collect();
    (list, diff, panels)
}

/// One strip's tabs, and the arrows for reaching the ones that did not fit.
struct Strip {
    /// The tabs on screen, left to right, starting at `first`.
    tabs: Vec<Panel>,
    first: usize,
    /// Both `nowhere()` when every tab fits.
    left: Panel,
    right: Panel,
}

/// Lay tabs left to right at the width their labels need, dropping any that do
/// not fit rather than squeezing them into unreadable slivers.
///
/// A strip that cannot hold all of its tabs keeps room for two arrows at its
/// right end and shows a window of tabs starting at `first`, so nothing is a
/// sliver and nothing is unreachable either. The room comes off before the window
/// of tabs is chosen: reserving it afterwards would push one more tab off the
/// edge, which is the same complaint one tab further along. A strip that fits
/// them all gets no arrows and loses no room to them.
///
/// `first` is a request rather than an instruction, and is answered twice. It is
/// clamped so the tabs at the end of the strip always fill it, because a space
/// left scrolled past its last tab (by a resize, or by closing the tabs it was
/// scrolled to) would show an empty strip. Then it is moved far enough that
/// `active` is on screen, because the pane below the strip belongs to that tab
/// and a pane whose own tab is missing cannot be read. Both answers are given
/// here, on every frame, rather than at each of the several places that can move
/// a tab or resize a window: a rule that runs every time cannot be forgotten by
/// the next thing that moves a tab.
///
/// That second rule is why the strip's arrows walk the showing tab as well as the
/// strip ([`crate::main`]'s `walk_tabs`): a scroll that left the showing tab
/// behind would be undone here, on the same frame.
fn strip_tabs(
    bar: Panel,
    widths: &[usize],
    column: f32,
    first: usize,
    active: Option<usize>,
) -> Strip {
    let each: Vec<f32> = widths
        .iter()
        .map(|chars| (*chars as f32 + 3.0) * column)
        .collect();
    // As many tabs as fit in `room`, starting at `from`.
    let lay = |from: usize, room: f32| -> Vec<Panel> {
        let mut out = Vec::new();
        let mut x = bar.x;
        for w in each.iter().skip(from) {
            if x + w > bar.x + room {
                break;
            }
            out.push(Panel::new(x, bar.y, *w, bar.h));
            x += w;
        }
        out
    };
    let plain = |room: f32| Strip {
        tabs: lay(0, room),
        first: 0,
        left: nowhere(),
        right: nowhere(),
    };
    let total: f32 = each.iter().sum();
    if total <= bar.w {
        return plain(bar.w);
    }
    let arrow = TAB_ARROW_COLUMNS as f32 * column;
    let room = bar.w - arrow * 2.0;
    // A strip too narrow to hold the arrows and the widest of its tabs both keeps
    // the tabs: two arrows over an empty strip are a control for reaching
    // nothing. Measured against the widest rather than the narrowest so that
    // every offset shows at least one tab, which is what makes the clamp below
    // enough on its own. The window has no size where this happens: the widest
    // label is eleven columns and the narrowest strip is over thirty.
    if each.iter().copied().fold(0.0, f32::max) > room {
        return plain(bar.w);
    }
    // The furthest it can be scrolled: past this the tabs at the end no longer
    // fill it and the strip is showing gap.
    let mut furthest = each.len().saturating_sub(1);
    let mut used = 0.0;
    for (i, w) in each.iter().enumerate().rev() {
        used += w;
        if used > room {
            break;
        }
        furthest = i;
    }
    let mut at = first.min(furthest);
    if let Some(active) = active {
        // Behind the window, the strip starts at the showing tab; ahead of it,
        // it walks forward until that tab is in view. `max(1)` is only there so
        // an offset showing no tabs cannot spin the loop; the check above rules
        // that out for every offset a caller can reach.
        at = at.min(active);
        while active >= at + lay(at, room).len().max(1) {
            at += 1;
        }
    }
    let x = bar.x + bar.w - arrow * 2.0;
    Strip {
        tabs: lay(at, room),
        first: at,
        left: Panel::new(x, bar.y, arrow, bar.h),
        right: Panel::new(x + arrow, bar.y, arrow, bar.h),
    }
}

/// Which edge, if any, a point is on. An undecorated window loses the window
/// manager's resize handles, so these are ours to provide.
pub fn edge(x: f32, y: f32, width: f32, height: f32) -> Option<winit::window::ResizeDirection> {
    use winit::window::ResizeDirection as Dir;
    let left = x <= RESIZE_EDGE;
    let right = x >= width - RESIZE_EDGE;
    let top = y <= RESIZE_EDGE;
    let bottom = y >= height - RESIZE_EDGE;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(Dir::NorthWest),
        (_, true, true, _) => Some(Dir::NorthEast),
        (true, _, _, true) => Some(Dir::SouthWest),
        (_, true, _, true) => Some(Dir::SouthEast),
        (true, ..) => Some(Dir::West),
        (_, true, ..) => Some(Dir::East),
        (_, _, true, _) => Some(Dir::North),
        (_, _, _, true) => Some(Dir::South),
        _ => None,
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
    pub popup_scroll: usize,
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

/// What a drop would do, drawn over the room it would take: a translucent green
/// box, and a caret in the gap between the two tabs the tab would land between.
///
/// On the floating layer, so it covers the pane rather than being painted under
/// the pane's own text the way a base-layer rectangle is (see [`overlay`]). A
/// wash under the glyphs is exactly the feedback item 17 said it could not see.
///
/// The box is the room the pane would have after the drop, which is one cell of
/// the grid or two, so a drop between two cells shows the pair before the button
/// comes up. It is taken by making the move on a copy of the dock and asking that
/// copy which cells the pane would cover: the box and the move come off one
/// answer rather than two, so they cannot promise different things.
///
/// A drop on a tab strip is the exception: it names a place among tabs and does
/// not move anything on the grid, so the box is the pane that is already drawn
/// there. Folded, that pane is its strip and nothing else, which is all there is
/// of it to point at.
///
/// The caret is only drawn for a drop that names a place, which is a drop on a
/// tab strip. In the body of a pane there is no gap being aimed at: the tab goes
/// to the end of the space, and a caret standing between two tabs would promise
/// a position the drop does not name.
fn drop_target(scene: &mut Scene, frame: &Frame) {
    let Some(drag) = frame.drag else {
        return;
    };
    let (skin, layout) = (frame.skin, frame.layout);
    let box_ = match drag.landing {
        Landing::In(space, Some(_)) => {
            let placed = layout.placed(space);
            Panel::new(
                placed.strip.x,
                placed.strip.y,
                placed.strip.w,
                placed.strip.h + placed.body.h,
            )
        }
        Landing::In(..) | Landing::Span(..) => drop_room(layout, frame.dock, drag),
        Landing::Out | Landing::Nowhere => return,
    };
    if box_.w < 1.0 || box_.h < 1.0 {
        return;
    }
    // The same cut corner every panel in the window has, so the box lies on the
    // pane instead of squaring off its top right corner.
    scene.over_rect(box_.fill(skin.drop_target).chamfer(cut_of(box_), Rect::TOP_RIGHT));
    let Landing::In(space, Some(at)) = drag.landing else {
        return;
    };
    let placed = layout.placed(space);
    let x = layout
        .insertion_gap(space, at)
        .min(placed.strip.x + placed.strip.w - CARET_W);
    scene.over_rect(Panel::new(x, placed.strip.y, CARET_W, placed.strip.h).fill(skin.drop_mark));
}

/// The room a pane would have once this drop had happened.
///
/// The move is made on a copy of the dock, so the cells it answers with are the
/// cells the real move would give it, spans and emptied neighbours included.
fn drop_room(layout: &Layout, dock: &Dock, drag: Drag) -> Panel {
    let mut after = dock.clone();
    match drag.landing {
        Landing::In(space, None) => {
            after.move_view(drag.view, space);
        }
        Landing::Span(a, b) => {
            after.span_view(drag.view, a, b);
        }
        _ => return nowhere(),
    }
    let Some(head) = after.space_of(drag.view) else {
        return nowhere();
    };
    let cover = after.cover();
    Space::ALL
        .into_iter()
        .filter(|cell| cover[cell.index()] == Some(head))
        .fold(nowhere(), |box_, cell| around(box_, layout.grid[cell.index()]))
}

/// The floating layer, and the last thing painted.
///
/// Drawn after everything else and hit tested before everything else, which
/// together are the whole of what floating means here. With only one of the two
/// a menu is either painted under the pane it opened over, or clicked straight
/// through onto it.
///
/// "After everything else" is `Scene::over_rect` and `Scene::over_text`, not
/// merely being pushed last. Pushed last onto the base layer, the menu's box was
/// still drawn before every glyph in the window, because the renderer paints a
/// layer's rectangles in one pass and its glyphs in a later one. The box landed
/// under the pane text it covered and the rows were illegible over anything with
/// writing in it. Every rectangle and every run here belongs to the overlay.
fn overlay(scene: &mut Scene, frame: &Frame) {
    // The popup first, so a menu opened over it is drawn on top of it, which is
    // the order it is hit tested in.
    crate::widgets::popup::popup(scene, frame);
    let Some(menu) = frame.menu else {
        return;
    };
    let (skin, layout) = (frame.skin, frame.layout);
    if layout.menu.w < 1.0 {
        return;
    }
    scene.over_rect(panel_fill(layout.menu, skin.menu));
    let chars = menu.width_chars();
    for (index, panel) in &layout.menu_rows {
        let Some(row) = menu.rows.get(*index) else {
            continue;
        };
        menu_row(scene, frame, *row, *index, *panel, chars, layout.menu);
    }
    // The border last, so the outline is unbroken across a lit row. Drawn
    // first, a row's own fill composited over the two hairlines it spans and
    // brightened them for exactly the height of the pointer, which reads as the
    // outline coming apart where the pointer is.
    scene.over_rect(panel_edge(layout.menu, skin.edge_focus));
    // The widgets flyout: the same overlay, one box further out.
    if layout.menu_fly.w >= 1.0 {
        scene.over_rect(panel_fill(layout.menu_fly, skin.menu));
        let fly_chars = menu.fly_width_chars();
        for (index, panel) in &layout.menu_fly_rows {
            let Some(row) = menu.rows.get(*index) else {
                continue;
            };
            menu_row(scene, frame, *row, *index, *panel, fly_chars, layout.menu_fly);
        }
        scene.over_rect(panel_edge(layout.menu_fly, skin.edge_focus));
    }
}

/// The rectangle a lit menu row is painted with: its own band, less the two
/// hairlines the box's border stands in, and with the box's own corner taken
/// out of it when it is the first row on screen.
///
/// Exactly the row vertically, so a highlight says which row without reaching
/// into the one above or below it. The pixels it gives up are the ones that do
/// not belong to it: the left and right border columns, and the notch the box
/// itself does not paint.
///
/// The chamfer is [`cut_of`] the box, less the margin the row already starts
/// below the top of it and the border column it already starts left of. Both
/// diagonals are at 45 degrees, so a cut that short reproduces the box's own
/// exactly from where the row begins.
fn menu_hot_box(row: Panel, box_: Panel, rgba: [f32; 4]) -> Rect {
    let edge = MENU_EDGE;
    let fill = Panel::new(
        row.x + edge,
        row.y,
        (row.w - edge * 2.0).max(1.0),
        row.h.max(1.0),
    )
    .fill(rgba);
    match row.y <= box_.y + edge + 0.01 {
        true => fill.chamfer((cut_of(box_) - edge * 2.0).max(0.0), Rect::TOP_RIGHT),
        false => fill,
    }
}

/// One row of a menu: the mark in the gutter, the label, and the group chevron
/// at the far end for a row that opens one.
///
/// `chars` is how many columns the labels in this box are laid out across, which
/// is what puts the chevron at the end of the row rather than after the label,
/// and what one step of indent is measured in. `box_` is the menu's own box,
/// which the lit row's corner is taken from.
fn menu_row(
    scene: &mut Scene,
    frame: &Frame,
    row: crate::menu::Row,
    index: usize,
    panel: Panel,
    chars: usize,
    box_: Panel,
) {
    let skin = frame.skin;
    // Only a row that can act lights up. Highlighting a greyed one promises
    // something will happen when the button comes down and it will not.
    //
    // Two ways for a row to be the one that is next: the pointer is on it, or
    // the keys are. Never both at once, because each of the two takes the other
    // down when it moves (`Menu::point_at`, `Menu::walk`).
    let lit = frame.hot == Some(Hit::MenuRow(index))
        || frame.menu.and_then(|menu| menu.cursor) == Some(index);
    if row.enabled && lit {
        scene.over_rect(menu_hot_box(panel, box_, skin.hot));
    }
    // Three things the tint says, in the order they win. A row waiting for a
    // second press before it destroys something is in the colour this window
    // uses for everything that throws work away, which is the colour the
    // settings panel's own delete asks the same question in. A row that cannot
    // act says so by weight, the way a tab that is not showing does, rather
    // than by being missing. And a group's header is brighter than the rows
    // that act, because it is a name over them rather than one of them.
    let tint = match (row.enabled, row.item.warns(), row.item.group().is_some()) {
        (true, true, _) => skin.bad,
        (true, false, true) => skin.bright,
        (true, false, false) => skin.body,
        (false, ..) => skin.dim,
    };
    let mut runs = Vec::new();
    match row.item.icon() {
        Some(icon) => runs.push(Run::icon(icon.to_string(), tint)),
        // The gutter is spent either way, so the labels line up.
        None => runs.push(Run::tinted(" ", tint)),
    }
    runs.push(Run::tinted(format!(" {}", row.item.label()), tint));
    let text = Panel::new(
        panel.x + MENU_PAD,
        panel.y,
        (panel.w - MENU_PAD * 2.0).max(1.0),
        panel.h,
    );
    // One column of the box, off the same arithmetic the chevron's own box uses,
    // so the label and the chevron cannot drift apart at any font size.
    let column = text.w / (chars + MENU_GUTTER) as f32;
    let line = Text::line_for(MENU_SIZE);
    scene.over_text(Text::rich(runs, text.row(0.0, line), MENU_SIZE, tint));

    let Some(mark) = row.item.marker() else {
        return;
    };
    // The last columns of the row, in a box of their own rather than spaces
    // written after the label. Padding a label out to the edge puts the mark
    // hard against the wrap width, where a column of drift between the symbol
    // font and the monospace one carries it onto a second line the row is not
    // tall enough to show, and a mark that is not drawn at all is the one
    // failure this window has already had once.
    let room = column * MARKER_COLUMNS as f32;
    let at = Panel::new(text.x + text.w - room, text.y, room, text.h);
    scene.over_text(Text::rich(
        vec![Run::icon(mark.to_string(), tint)],
        at.row(0.0, line),
        MENU_SIZE,
        tint,
    ));
}

fn title_bar(scene: &mut Scene, frame: &Frame) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    // Open, the bar is a strip across the top. Shaded, it is the whole surface,
    // with the strip's contents drawn at the top of it.
    //
    // Asking for a 30 pixel window is a request, not an instruction: a
    // compositor is free to hand back a taller surface, and this one does unless
    // the window is maximized. Everything under the strip was then cleared to
    // transparent, which composites as black, so shading drew a green strip on a
    // black block. Filling the whole surface in the bar's own colour makes a
    // surface that stays tall read as a green bar, and one that does shrink is
    // pixel for pixel what it was.
    //
    // The bar colour rather than a new one, so the opacity setting still reaches
    // it, and one rectangle rather than a full-surface one under the strip's own:
    // two translucent fills over each other would leave the top 30 pixels more
    // solid than the rest of the bar.
    let surface = if layout.shaded {
        Panel::new(0.0, 0.0, layout.width, layout.height.max(layout.title.h))
    } else {
        layout.title
    };
    scene.rect(surface.fill(skin.bar));

    // How full the context is, as a hairline along the bottom of the strip.
    // It was a bar of its own at the foot of the window; two pixels at the top
    // of the window says the same thing and costs no rows.
    let gauge = Panel::new(0.0, layout.title.y + layout.title.h - 2.0, layout.width, 2.0);
    scene.rect(gauge.fill(skin.gauge_track));
    let used = state.context_fraction();
    if used > 0.0 {
        scene.rect(Panel::new(0.0, gauge.y, layout.width * used, 2.0).fill(skin.gauge));
    }

    // The orb, in the square the strip keeps for it: turning while there is a
    // turn to animate, one frozen dimmer square of dots otherwise, and on its
    // way between the two while a turn is starting or ending. The base layer is
    // enough for it, unlike the menu, because [`ORB_W`] is reserved and no glyph
    // in the window starts inside it, so there is nothing here for a disc to be
    // painted under. It also costs a draw call fewer that way, and there are
    // 516 of these a frame.
    let block = Panel::new(
        layout.title.x,
        layout.title.y,
        ORB_W.min(layout.title.w),
        layout.title.h,
    );
    for disc in crate::orb::discs(block, frame.clock, frame.morph(), skin) {
        scene.rect(disc);
    }

    // These were three hand-drawn rectangles, because the Unicode glyphs the
    // first version asked for were not on this machine and a missing glyph
    // draws as nothing. The symbol font ships in the binary now, so they are
    // the same marks every other window on the desktop uses.
    for (panel, hit, tint, glyph, quiet) in [
        (layout.minimize, Hit::Minimize, skin.hot, crate::design::icons::MINIMIZE, true),
        (layout.maximize, Hit::Maximize, skin.hot, crate::design::icons::MAXIMIZE, true),
        (layout.close, Hit::Close, skin.close_hot, crate::design::icons::CLOSE, false),
    ] {
        let lit = frame.hot == Some(hit);
        if lit {
            scene.rect(panel.fill(tint));
        }
        // Close reads at full strength because it is the one that cannot be
        // undone; the other two sit back until the pointer is on them.
        let ink = match (lit, quiet) {
            (true, _) => skin.bright,
            (false, true) => skin.dim,
            (false, false) => skin.title,
        };
        // The box runs to the button's right edge rather than being sized to
        // one estimated glyph. A box exactly one guessed advance wide clipped
        // these: the maximize mark lost all but its left edge and close all but
        // one arm of its cross.
        let left = ((panel.w - SMALL * 0.6) * 0.5).max(0.0).floor();
        let row = strip_row(panel);
        scene.text(
            Text::rich(
                vec![Run::icon(glyph.to_string(), ink)],
                Panel::new(row.x + left, row.y, (row.w - left).max(1.0), row.h),
                SMALL,
                ink,
            )
            // The mark's own line box, capped at the room the row turned out to
            // have. Left at a full line in a shorter row, the glyph is laid out
            // below the box and clipped away: the whole mark lost to keep the
            // two pixels of air under it.
            .line_height(row.h),
        );
    }

    // The name, then the marker, and nothing else at full strength. It read
    // "NO0B \u{25b8} CLIppy" while the window had two names; it has one.
    let room = (layout.width - BUTTON_W * 3.0 - ORB_W - 12.0).max(1.0);
    let mut runs = vec![
        Run::tinted("NO0B \u{25b8}", skin.bright),
        // Which release this is. At the text tint, not the dim one: dim is the
        // faintest thing the palette has, and the version is the answer to the
        // first question anyone asks about a build.
        //
        // The commit used to follow it, out of a build.rs stamp. It is gone:
        // seven characters of hex is not something anyone reads off a title,
        // and the room they took now says where the agent is working.
        Run::tinted(format!(" {VERSION}"), skin.title),
    ];
    // Then the folder this session is in, after the same marker that separates
    // the name from the version. Clipped by column against the room the strip
    // actually has, because the strip is one box with no ellipsis of its own
    // and a deep path would be cut mid-glyph instead of shortened. Before
    // SessionStart there is no folder, and a marker with nothing after it is
    // worse than no marker.
    if !state.workspace.is_empty() {
        // One estimated advance, the guess the rest of this strip measures with.
        let taken = "NO0B \u{25b8}".chars().count() + VERSION.chars().count() + 4;
        let space = columns_in(room, SMALL * 0.6).saturating_sub(taken);
        if space > 1 {
            runs.push(Run::tinted(" \u{25b8}", skin.bright));
            runs.push(Run::tinted(
                format!(" {}", clip(&short_path(&state.workspace), space)),
                skin.title,
            ));
        }
    }
    // Open, the strip says which build this is and where it is working. The
    // phase, the model and the token budget were readings squeezed into a title
    // with no room to label them; they belong in the monitors, which have both.
    // Trouble stays because it is the one thing that makes the rest of the
    // window meaningless.
    if let Some(trouble) = frame.trouble {
        runs.push(Run::tinted(format!("   {trouble}"), skin.bad));
    } else if layout.shaded {
        // Shaded, this strip is the whole window, so it carries the one thing
        // worth knowing while there is nowhere else to read it. In the bad
        // colour while a turn is running, the same as the phase reads in the
        // pane it comes from: the word and the orb beside it then say the same
        // thing, which is the whole job of a strip this small.
        let tint = match state.phase.busy() {
            true => skin.bad,
            false => skin.good,
        };
        runs.push(Run::tinted(format!("   {}", state.headline()), tint));
    }
    let row = strip_row(Panel::new(ORB_W, layout.title.y, room, layout.title.h));
    scene.text(Text::rich(runs, row, SMALL, skin.title).line_height(row.h));
}

/// The body of a panel: the fill, cut corner and all.
///
/// The cut lives on the fill as well as on the outline because they are the
/// same shape twice. A square fill under a cut outline shows a triangle of the
/// wrong colour poking out of the corner.
pub(crate) fn panel_fill(panel: Panel, rgba: [f32; 4]) -> Rect {
    panel.fill(rgba).chamfer(CUT, Rect::TOP_RIGHT)
}

/// How far the cut actually reaches on a box this size.
///
/// The shader caps the reach at half the shorter side, so a short box loses a
/// smaller corner than [`CUT`]. Anything that has to stop where the cut starts
/// has to cap it the same way, or it stops short of a corner nothing took.
pub(crate) fn cut_of(panel: Panel) -> f32 {
    CUT.min(panel.w * 0.5).min(panel.h * 0.5).max(0.0)
}

/// A line along the cut itself, so the corner is a drawn edge rather than only a
/// missing one.
///
/// [`Rect::stroke`] follows the whole shape and cannot be asked for one side, so
/// a box that wants its diagonal without its other three sides has to draw the
/// diagonal itself. One rectangle per pixel row of the cut, `weight` wide and a
/// single pixel tall, each one starting a pixel further right than the row above
/// it: a stair whose thickness is read across, which is how a `weight` that is
/// not the hairline gets to be bolder without any two rectangles overlapping.
/// They must not overlap, because the colours these are drawn in are translucent
/// and two of them stacked composite darker than the straight edges they meet.
///
/// It runs from where the top edge would stop, `(right - cut, top)`, down to the
/// row the right edge starts on, `top + cut`. The last rows are clipped to the
/// box's right edge, so the stair narrows to a hairline exactly where a hairline
/// right edge picks it up.
pub(crate) fn cut_line(scene: &mut Scene, panel: Panel, rgba: [f32; 4], weight: f32) {
    let cut = cut_of(panel);
    if cut < weight {
        // A box squeezed smaller than the line is meant to be thick lost its
        // corner to the cap in `cut_of`; there is no diagonal left to draw.
        return;
    }
    let right = panel.x + panel.w;
    for row in 0..cut as usize {
        let at = row as f32;
        let x = right - cut + at;
        scene.rect(Panel::new(x, panel.y + at, weight.min(right - x), 1.0).fill(rgba));
    }
}

/// Its hairline border, as one rectangle. Four of them could not follow the
/// cut.
///
/// For a box that wants all four sides: the prompt, the picker, the menu. A
/// pane's body uses [`pane_edges`] instead, which leaves the top one out.
pub(crate) fn panel_edge(panel: Panel, rgba: [f32; 4]) -> Rect {
    panel_fill(panel, rgba).stroke(1.0)
}

/// A pane's border, minus the top edge.
///
/// That top edge was the line under every tab strip. A tab and the pane below it
/// are one surface (the same fill, and the strip is flush with the body), so a
/// hairline between them read as the pane being a box hung off the strip instead
/// of the strip being the top of the pane. The other three sides still tell two
/// panes over a busy desktop apart, so only the top one goes.
///
/// Thin rectangles rather than the one stroked rect [`panel_edge`] draws, because
/// a stroke follows the whole shape and cannot leave a side out. The three that
/// are left are straight lines, and the cut is on the top right, which is the
/// corner the top edge had.
///
/// The cut is bordered too ([`cut_line`]), in the same colour as the other three
/// sides and at [`CUT_EDGE_H`], twice their weight. Same colour because a pane is
/// one material and a corner in a second colour reads as a second thing stuck on
/// it; heavier because the diagonal is the mark that says what shape the pane is,
/// and a hairline down it was lost against three hairline sides.
pub(crate) fn pane_edges(scene: &mut Scene, panel: Panel, rgba: [f32; 4]) {
    let cut = cut_of(panel);
    scene.rect(panel.left_edge(rgba));
    scene.rect(panel.bottom_edge(rgba));
    scene.rect(
        Panel::new(
            panel.x + panel.w - 1.0,
            panel.y + cut,
            1.0,
            (panel.h - cut).max(0.0),
        )
        .fill(rgba),
    );
    cut_line(scene, panel, rgba, CUT_EDGE_H);
}

/// One tab of a strip, before its label goes on.
///
/// A tab is not a button. Both states carry the pane's own surface and the same
/// cut corner the pane has, so the tab reads as the top of the pane; what says
/// which one is showing is weight. The showing tab is that surface at full
/// strength with an accent line along its top, the rest are the same colour at
/// a lower alpha. A filled block over a filled strip is what made these look
/// like a row of buttons.
///
/// One green for every view, not a hue each. Nine hues on nine tabs is a
/// harlequin strip, and it was answering a question nobody asked: which pane
/// this is is written on it, and all the line has to say is which one you are
/// looking at.
///
/// `Skin::tab` is exactly `Skin::panel`, and the showing tab sits flush on the
/// pane, so the two composite to one surface with nothing between them. That is
/// the other half of losing the line under the strip ([`pane_edges`]): a step in
/// colour where the line was is the same complaint as the line.
fn tab_block(scene: &mut Scene, skin: &Skin, tab: Panel, active: bool) {
    let cut = cut_of(tab);
    scene.rect(
        tab.fill(if active { skin.tab } else { skin.tab_idle })
            .chamfer(cut, Rect::TOP_RIGHT),
    );
    if !active {
        return;
    }
    // Stopped where the cut starts. Run to the full width and the last pixels
    // of the line hang in a corner that is not there any more.
    scene.rect(
        Panel::new(tab.x, tab.y, (tab.w - cut).max(1.0), ACCENT_H.min(tab.h))
            .fill(skin.tab_accent),
    );
    // And picked up again there, so the accent turns the corner instead of
    // stopping in mid air: down the cut, then on down the right edge. The
    // diagonal is the heavy one ([`CUT_EDGE_H`]) and the two sides are
    // hairlines, because the diagonal is the mark that says what shape a tab is
    // and the sides only have to say where it ends.
    let thin = TAB_EDGE_H.min(tab.h);
    cut_line(scene, tab, skin.tab_accent, CUT_EDGE_H.min(tab.h));
    scene.rect(
        Panel::new(
            tab.x + tab.w - thin,
            tab.y + cut,
            thin,
            (tab.h - cut).max(0.0),
        )
        .fill(skin.tab_accent),
    );
    // The left side runs the whole height, since nothing stops it: the accent
    // starts at the same x and there is no bottom border for it to meet.
    scene.rect(Panel::new(tab.x, tab.y, thin, tab.h).fill(skin.tab_accent));
    // And no foot. A line on the tab's last row is a line at the pane's top
    // edge, which is the rule under the strip that item 12 took away; the tab
    // and its pane are one surface and nothing is drawn across the seam.
}

/// The two arrows at the right end of a strip that holds more tabs than it can
/// show, and nothing at all on one that fits.
///
/// Glyphs and no box. The strip itself is not a surface (see [`space_pane`]), and
/// a filled block at that end of it would sit square over the cut corner of the
/// pane below, which is the stray corner the strip's own fill was taken away for.
/// The direction that has nowhere left to go is dimmed instead of hidden, so the
/// pair does not move under the pointer at either end of the walk.
fn strip_arrows(scene: &mut Scene, frame: &Frame, space: Space) {
    let placed = frame.layout.placed(space);
    if placed.arrow_left.w < 1.0 {
        return;
    }
    let slot = frame.dock.slot(space);
    // Live while there is another tab that way at all, which is what an arrow
    // walks to. Not whether the strip itself can still move: at the end of the
    // strip the last few tabs are all on screen together, and the arrow still
    // steps the showing tab through them.
    let at = slot.active_index().unwrap_or(0);
    let line = Text::line_for(SMALL);
    for (panel, glyph, live) in [
        (placed.arrow_left, icons::TABS_LEFT, at > 0),
        (
            placed.arrow_right,
            icons::TABS_RIGHT,
            at + 1 < slot.views.len(),
        ),
    ] {
        let ink = if live {
            frame.skin.bright
        } else {
            frame.skin.dim
        };
        // The box runs to the arrow's right edge rather than being sized to one
        // guessed advance, the way the window buttons do it: a box exactly one
        // estimated advance wide clips the glyph in it.
        let left = ((panel.w - SMALL * 0.6) * 0.5).max(0.0).floor();
        scene.text(Text::rich(
            vec![Run::icon(glyph.to_string(), ink)],
            Panel::new(
                panel.x + left,
                panel.y + ((panel.h - line) * 0.5).max(0.0).floor(),
                panel.w - left,
                line,
            ),
            SMALL,
            ink,
        ));
    }
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

fn space_pane(scene: &mut Scene, frame: &Frame, space: Space) {
    let skin = frame.skin;
    let placed = frame.layout.placed(space);
    let slot = frame.dock.slot(space);
    if placed.strip.w < 1.0 {
        return;
    }

    // The strip itself is not drawn. It is the window, not a toolbar, and the
    // tabs standing in it are the only thing up here. Its fill and the hairline
    // along its foot were both square, so they ran past the cut corner of the
    // pane below and left a stray stroke there. Nothing spans the strip now, and
    // nothing runs along the pane's top edge either: the tab and the pane are one
    // surface, which is what item 12 asked for.
    for (view, panel) in &placed.tabs {
        let active = slot.active() == Some(*view);
        let lifted = frame.drag.is_some_and(|drag| drag.view == *view);
        tab_block(scene, skin, *panel, active);
        // Not showing reads as not showing. This was the title tint, as strong
        // as the showing tab's, which left the fill to carry the whole
        // difference and is why the fill had to be so heavy.
        let color = if active && !lifted {
            skin.bright
        } else {
            skin.dim
        };
        scene.text(Text::rich(
            vec![Run::tinted(
                tab_label(*view, frame.state.shown_agent),
                color,
            )],
            panel.row(SMALL * 0.6, Text::line_for(SMALL)),
            SMALL,
            color,
        ));
    }
    strip_arrows(scene, frame, space);
    if slot.folded || placed.body.h < 2.0 {
        return;
    }
    let panel = placed.body;
    scene.rect(panel_fill(panel, skin.panel));
    // Three sides, not four. The missing one is the top, which was the line under
    // the tabs; see [`pane_edges`]. The fill still carries the cut, so the corner
    // is unchanged.
    //
    // The same three edges whether or not a drop would land here. A space being
    // dragged onto used to be lit in `edge_focus` instead, and once the top edge
    // went the lit outline no longer closed around the pane, so it read as a
    // pane with a coloured left side rather than as a target. What says a drop
    // lands here now is a box over the whole space; see [`drop_target`].
    pane_edges(scene, panel, skin.edge);

    // Banded in the box the text is actually in, which is the whole body for
    // every pane but the file one: the file view spends its left column on the
    // explorer, and banding the body there put the highlight a list's width off
    // the glyphs it was meant to cover.
    selection_band(scene, frame, frame.layout.content(space), slot.active());

    match slot.active() {
        None => {}
        Some(View::Output) => crate::widgets::output::output(scene, frame, panel),
        Some(View::Activity) => crate::widgets::activity::activity(scene, frame, panel),
        Some(View::Plan) => crate::widgets::plan::plan(scene, frame, panel),
        Some(View::Agents) => crate::widgets::agents::agents(scene, frame, panel),
        Some(View::Agent) => crate::widgets::agent::agent(scene, frame, panel),
        Some(View::Hardware) => {
            crate::widgets::gauges::gauges(scene, frame, panel, View::Hardware, frame.monitor.hardware())
        }
        // The monitor's lists are named for the panes they feed, so a reading in
        // the wrong pane is a rename away from being obvious rather than two
        // files away.
        Some(View::Context) => crate::widgets::context::context(scene, frame, panel),
        Some(View::Session) => crate::widgets::gauges::gauges(scene, frame, panel, View::Session, frame.monitor.session()),
        Some(View::Files) => crate::widgets::files::files(scene, frame, panel),
    }
}

/// The band behind selected text, drawn before the glyphs go over it.
///
/// One rectangle per visible line of the selection rather than one for the
/// whole block, because the first and last lines start and stop mid-line and a
/// single rectangle would cover text that is not selected.
pub(crate) fn selection_band(scene: &mut Scene, frame: &Frame, panel: Panel, showing: Option<View>) {
    let (Some(selection), Some(view)) = (frame.selection, showing) else {
        return;
    };
    if selection.at != crate::select::Where::Pane(view) || selection.is_empty() {
        return;
    }
    let Some(pane) = frame.state.pane_of(view) else {
        return;
    };
    // The pane's own size, not the pane size for everything: the output pane is
    // drawn at the transcript size, and banding it at the smaller one is what
    // put the highlight off the glyphs it was supposed to cover.
    let (size, column) = frame.metrics_of(view);
    let content = panel.inset(PAD);
    let fit = frame.layout.rows(panel, size);
    // The transcript gives its bottom rows to the queued messages, so the
    // band is measured over the rows the text was really drawn in.
    let rows = match view {
        View::Output => fit - frame.state.output_reserved(fit),
        _ => fit,
    };
    // The columns the text is in and the columns in front of it, from the one
    // place that says so: the file view keeps four for its line numbers, and a
    // band measured in the full width of the box was four columns wide of the
    // glyphs on every row of a file.
    let (cols, chrome) = text_columns(view, panel, column);
    paint_selection(scene, selection, pane, Painted {
        content,
        rows,
        cols,
        chrome,
        size,
        column,
        tint: frame.skin.select,
    });
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

/// The rectangles behind one selection, over the pane that resolves it.
///
/// Split out of [`selection_band`] so the settings document bands with the same
/// arithmetic rather than with a second copy of it: the two boxes differ in
/// where they are and in nothing else, and a document highlighted by its own
/// rule would be a highlight the copy disagreed with.
pub(crate) fn paint_selection(
    scene: &mut Scene,
    selection: crate::select::Selection,
    pane: &crate::state::Pane,
    at: Painted,
) {
    let Painted {
        content,
        rows,
        cols,
        chrome,
        size,
        column,
        tint,
    } = at;
    let line_h = Text::line_for(size);
    let window = pane.window(rows, cols);
    let first = pane.showing_from(rows, cols);
    for step in 0..window.count {
        let number = first + step;
        let Some(line) = pane.line(number) else {
            continue;
        };
        // Counted in what is on screen: a Markdown line is drawn without its
        // marks, and a band measured on the source runs past the glyphs.
        let chars = line.shown().chars().count();
        let Some((from, to)) = selection.columns_on(number, chars) else {
            continue;
        };
        let Some((top, height)) = pane.band_of(rows, cols, number) else {
            continue;
        };
        // A wrapped line needs one rectangle per visual row, each covering only
        // the part of the selection that lands on that row. The first line in
        // the window may start partway down, which is what `skip` records.
        // Which characters a row holds comes from the pane, which is the same
        // answer the renderer breaks the rows by: a band drawn on its own
        // arithmetic is a highlight over text the clipboard does not have.
        let from_row = if step == 0 { window.skip } else { 0 };
        let spans = pane.rows_of_line(number, cols);
        for i in 0..height {
            let Some(span) = spans.get(from_row + i) else {
                continue;
            };
            let (row_start, row_end) = (span.start, span.end);
            let a = from.max(row_start);
            let b = to.min(row_end);
            if a >= b {
                continue;
            }
            // Past the chrome, which every row of the line carries: the gutter
            // on the first row and the indent under it on the rest.
            let x = content.x + (chrome + a - row_start) as f32 * column;
            let width = ((b - a) as f32 * column).min(content.x + content.w - x);
            let y = content.y + (top + i) as f32 * line_h;
            if width <= 0.0 || y + line_h > content.y + content.h {
                continue;
            }
            scene.rect(Panel::new(x, y, width, line_h).fill(tint));
        }
    }
}

pub(crate) fn text_box(scene: &mut Scene, frame: &Frame, panel: Panel, size: f32, runs: Vec<Run>) {
    scene.text(Text::rich(runs, panel.inset(PAD), size, frame.skin.body));
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

/// A pane that is a list of lines, scrolled inside its own box.
///
/// PLAN and AGENTS. Both drew every row they had, with no window and no bar,
/// into one text box that ran off the bottom of the pane. What was past the edge
/// could not be reached at all, which is what item 14 reported.
///
/// The window, the clamp and the thumb come from `text_geometry` through
/// [`crate::scroll::Scrolls`], the same numbers the transcript is drawn from, so a row of
/// a list and a row of a transcript mean the same thing. A line partly scrolled
/// off the top is drawn in full and offset by `skip` rather than dropped, which is
/// what lets a wrapped todo scroll a row at a time.
pub(crate) fn list_pane(scene: &mut Scene, frame: &Frame, panel: Panel, view: View, rows: Vec<ListRow>) {
    let size = frame.pane_size;
    let fit = frame.layout.rows(panel, size);
    let cols = cols_of(panel, frame.pane_column);
    let heights: Vec<usize> = rows.iter().map(|row| row.rows(cols)).collect();
    let scrolls = frame.scrolls;
    let window = scrolls.window(view, &heights, fit);
    let mut runs = Vec::new();
    for row in rows.into_iter().skip(window.first).take(window.count) {
        runs.extend(row.runs);
        runs.push(Run::plain("\n"));
    }
    if !runs.is_empty() {
        scene.text(
            Text::rich(runs, panel.inset(PAD), size, frame.skin.body)
                .scrolled(window.skip as f32)
                .wrap_at(cols),
        );
    }
    scrollbar(scene, frame.skin, panel, scrolls.thumb(view, &heights, fit));
}

/// One row each, for a list of lines that are clipped rather than wrapped.
///
/// Written as heights and read through [`text_geometry`] so the window and the
/// clamp come from the one place that owns them.
pub fn flat_heights(count: usize) -> Vec<usize> {
    text_geometry::heights((0..count).map(|_| 0), 1)
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

/// Which list row is under a point of a list pane, by index into the same
/// rows the painter draws. The window, the skip and the heights come from
/// the one geometry [`list_pane`] draws with, so a press lands on the row
/// the eye is on however the list is scrolled or wrapped.
pub(crate) fn list_row_at(
    frame: &Frame,
    panel: Panel,
    view: View,
    rows: &[ListRow],
    x: f32,
    y: f32,
) -> Option<usize> {
    let inset = panel.inset(PAD);
    if !inset.contains(x, y) {
        return None;
    }
    let size = frame.pane_size;
    let line = Text::line_for(size);
    let fit = frame.layout.rows(panel, size);
    let cols = cols_of(panel, frame.pane_column);
    let heights: Vec<usize> = rows.iter().map(|row| row.rows(cols)).collect();
    let window = frame.scrolls.window(view, &heights, fit);
    let visual = ((y - inset.y) / line).floor().max(0.0) as usize + window.skip;
    let mut above = 0usize;
    for (at, tall) in heights.iter().enumerate().skip(window.first) {
        if visual < above + tall {
            return Some(at);
        }
        above += tall;
    }
    None
}

/// The tab under the pointer while it is being dragged, so the drag has
/// something following it and the drop has somewhere to be aimed.
///
/// On the floating layer, like the box under it: a tab in the air is the most
/// floating thing there is, and in the base layer its own box was painted before
/// every glyph in the window, so it slid under the text of whatever pane it
/// crossed.
fn dragging(scene: &mut Scene, frame: &Frame) {
    let Some(drag) = frame.drag else {
        return;
    };
    let skin = frame.skin;
    let label = tab_label(drag.view, frame.state.shown_agent);
    let w = (label.chars().count() as f32 + 3.0) * frame.column;
    let ghost = Panel::new(drag.at.0 - w * 0.5, drag.at.1 - TAB_H * 0.5, w, TAB_H);
    // Out of the window, letting go closes the widget, so the tab in the air says
    // so: its edge and its label go to the bad colour, and there is no green box
    // anywhere on screen because there is no space to land in. The pointer says
    // the same thing (`main`'s `cursor_for`), and neither is enough on its own:
    // the cursor is 20 pixels of somebody else's theme and the ghost is the thing
    // being carried.
    let out = drag.landing == Landing::Out;
    let (edge, ink) = match out {
        true => (skin.drop_out, skin.bad),
        false => (skin.edge_focus, skin.bright),
    };
    scene.over_rect(ghost.fill(skin.bar));
    scene.over_rect(ghost.outline(edge, 1.0));
    scene.over_text(Text::rich(
        vec![Run::tinted(label, ink)],
        ghost.row(SMALL * 0.6, Text::line_for(SMALL)),
        SMALL,
        ink,
    ));
}

/// The bar down the right edge of a pane. Absent when everything fits, because
/// a scrollbar that is always full length says nothing.
/// Where a pane's scroll track runs: down the right edge, below the cut.
///
/// One function for the drawing, the hit band and the drag arithmetic, so a
/// press lands on the track exactly where the thumb is drawn and a dragged
/// fraction means the same place on both.
pub(crate) fn scroll_track(panel: Panel) -> Panel {
    // The track runs down the right edge, which is the edge the cut takes a
    // triangle out of. Starting it three pixels down put its head inside that
    // triangle, hanging in the air outside the pane, so it starts below the cut
    // instead: the cut reaches `cut` in from the corner along both edges, and
    // the track is already `SCROLL_GAP` in from the right.
    let head = (cut_of(panel) - SCROLL_GAP).max(3.0);
    Panel::new(
        panel.x + panel.w - SCROLL_W - SCROLL_GAP,
        panel.y + head,
        SCROLL_W,
        (panel.h - head - 3.0).max(1.0),
    )
}

pub(crate) fn scrollbar(scene: &mut Scene, skin: &Skin, panel: Panel, thumb: Option<(f32, f32)>) {
    let Some((top, size)) = thumb else {
        return;
    };
    let track = scroll_track(panel);
    scene.rect(track.fill(skin.scroll_track));
    scene.rect(
        Panel::new(
            track.x,
            track.y + track.h * top,
            track.w,
            (track.h * size).max(8.0).min(track.h),
        )
        .fill(skin.scroll_thumb),
    );
}

fn input_row(scene: &mut Scene, frame: &Frame) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    scene.rect(panel_fill(layout.input, skin.input));
    scene.rect(panel_edge(layout.input, skin.edge_focus));
    let line = Text::line_for(frame.body_size);
    let box_ = input_box(layout.input, line);
    let columns = columns_in(box_.w, frame.column);
    // What the box can hold and how much of the prompt is above it. Everything
    // below is drawn from `top`, which is the first row's y whether or not that
    // row is on screen, so the rows that are on screen land where the caret
    // arithmetic and the click arithmetic both say they do.
    let skip = prompt_skip(frame.prompt.caret(), columns, rows_in(box_, line));
    let top = box_.y - skip as f32 * line;
    // Under the glyphs, like the band in a pane, so selected text stays
    // readable rather than being painted over.
    if let Some((from, to)) = frame.prompt.selection() {
        let mut at = from + PROMPT_COLUMNS;
        let end = to + PROMPT_COLUMNS;
        while at < end {
            let row = at / columns;
            // One rectangle per visual row: a selection that wrapped is not
            // one rectangle, it is a run on each row it crosses.
            let stop = end.min((row + 1) * columns);
            let band = Panel::new(
                box_.x + (at % columns) as f32 * frame.column,
                top + row as f32 * line,
                (stop - at) as f32 * frame.column,
                line,
            );
            if band.y >= box_.y - 0.5 && band.y + band.h <= box_.y + box_.h + 0.5 {
                scene.rect(band.fill(skin.select));
            }
            at = stop;
        }
    }
    // The marker slot, two columns of it. At rest it is the prompt's chevron and
    // a space. While a turn runs it is two blanks, and the three dots that used
    // to be an ellipsis glyph are drawn into it below as rectangles: one shaped
    // box has one baseline, so a glyph cannot be lifted off it on its own.
    let busy = state.phase.busy();
    let marker = if busy { "  " } else { "\u{203a} " };
    let mut runs = vec![
        Run::tinted(marker.to_string(), skin.dim),
        Run::tinted(frame.prompt.text(), skin.bright),
    ];
    // Armed, the empty prompt says what the second ESC does, in the colour
    // that means stop. It sits where the eye already is: on the line the
    // first ESC was aimed at.
    if frame.esc_armed {
        runs.push(Run::tinted("press ESC again to cancel", skin.bad));
    }
    scene.text(
        Text::rich(runs, box_, frame.body_size, skin.bright)
        // Broken on the column the caret is placed by, so counting columns
        // lands on the glyph that is really there. This is the one box in the
        // window that is not wrapped at blanks: a row that ended early would
        // put the caret a word away from the character it is on, since
        // everything here is `row * columns + column`. The panes wrap at
        // blanks, and their rows are counted the same way they are drawn.
        .break_at(columns)
        // The rows above the window are paid for and not drawn, the way a pane
        // showing the tail of a long stream is. Without this a prompt longer
        // than its allowance goes on being typed into a box that shows only its
        // first rows, which is a setting that appears to do nothing.
        .scrolled(skip as f32),
    );
    // The dots, on the first row of the box, in the marker's own two columns.
    // Round, because they stand in for three round glyphs and because the orb in
    // the strip is round for exactly as long as they are on screen. Pushed
    // before the caret so the caret is still the last thin rectangle in the row,
    // which is how the tests find it.
    // Skipped once the first row has scrolled off the top: the marker's two
    // blank columns went with it, and three dots over somebody's text is not a
    // marker, it is three dots in the way.
    if busy && skip == 0 {
        let span = 3.0 * PROMPT_DOT + 2.0 * PROMPT_DOT_GAP;
        let slack = (PROMPT_COLUMNS as f32 * frame.column - span).max(0.0) * 0.5;
        let rest = box_.y + (line - PROMPT_DOT) * 0.5;
        for (index, lift) in prompt_wave(frame.clock, busy).into_iter().enumerate() {
            let dot = Panel::new(
                box_.x + slack + index as f32 * (PROMPT_DOT + PROMPT_DOT_GAP),
                rest - lift,
                PROMPT_DOT,
                PROMPT_DOT,
            );
            scene.rect(dot.fill(skin.caret).radius(PROMPT_DOT * 0.5));
        }
    }
    let at = frame.prompt.caret() + PROMPT_COLUMNS;
    let (row, column) = (at / columns, at % columns);
    let caret = Panel::new(
        box_.x + column as f32 * frame.column,
        top + row as f32 * line,
        2.0,
        line,
    );
    // Always true, since the box is scrolled to the caret's row; the check is
    // what makes that a fact rather than an assumption.
    if caret.y >= box_.y - 0.5 && caret.y + caret.h <= box_.y + box_.h + 0.5 {
        scene.rect(caret.fill(skin.caret));
    }
}

/// How many whole rows of text a box holds. One at the least: a box too short
/// for a line still has a line in it, clipped, rather than dividing by nothing.
pub(crate) fn rows_in(box_: Panel, line: f32) -> usize {
    ((box_.h / line.max(1.0)).floor() as usize).max(1)
}

/// How many of the prompt's rows have scrolled off the top of its box.
///
/// As few as it takes to keep the caret's row inside the box: nothing while the
/// prompt fits, and then one row per row typed past the allowance, so what you
/// are typing is on screen and what you typed earlier is above it. Drawing and
/// hit testing both come through here, or a click would land a row off on
/// anything that had scrolled.
fn prompt_skip(caret: usize, columns: usize, rows: usize) -> usize {
    let row = (caret + PROMPT_COLUMNS) / columns.max(1);
    row.saturating_sub(rows.max(1) - 1)
}

/// How far each of the prompt's three dots is off its rest line, in pixels.
///
/// The wave as asked for: one dot up while the other two are down, then the next
/// one, and around again. Stepped rather than a sine, because three dots
/// bouncing smoothly read as a wobble and three dots taking turns read as a
/// wave.
///
/// Level whenever nothing is running, and that is not a nicety: the window holds
/// a redraw deadline only while a turn does, so a lift that moved at rest would
/// be a frame that never arrives and a dot stuck wherever the last redraw left
/// it.
fn prompt_wave(clock: f32, busy: bool) -> [f32; 3] {
    let mut lift = [0.0; 3];
    if busy {
        lift[(clock.max(0.0) / PROMPT_DOT_STEP) as usize % 3] = PROMPT_DOT_LIFT;
    }
    lift
}


/// The box the prompt's text is drawn in, inside the strip the layout gave it.
///
/// Top-aligned so the first line does not move as the prompt grows. Drawing
/// and hit testing both take it from here, which is the only way a click can
/// land on the column the glyph is actually in.
fn input_box(input: Panel, line: f32) -> Panel {
    Panel::new(
        input.x + PAD,
        input.y + INPUT_PAD,
        (input.w - 2.0 * PAD).max(1.0),
        (input.h - 2.0 * INPUT_PAD).max(line),
    )
}

/// How many characters fit across a box of this width.
pub(crate) fn columns_in(width: f32, column: f32) -> usize {
    ((width / column.max(1.0)).floor() as usize).max(1)
}

/// How a view's text sits in its box: the columns it wraps in, and the columns
/// of chrome drawn in front of it.
///
/// The file view spends four columns on its line-number gutter. The gutter is
/// drawn as part of each row but is no part of the line, so wrapping has to
/// happen in what is left and a click has to have it taken off again. Both
/// numbers come from here, because the wrapping and the hit testing being
/// derived separately is what put file selection four columns out.
/// One line number, in exactly `chrome` columns.
///
/// Fixed width because the wrap is: the text of a file line starts `chrome`
/// columns in on its first row and on every row it continues onto, so a number
/// that took one column more would push the first row one character out from
/// the rows under it. Three digits and a blank is the usual answer; a file long
/// enough spends the blank, and one longer still says it was cut rather than
/// quietly showing a different line's number.
pub(crate) fn file_number(number: u32, chrome: usize) -> String {
    let digits = number.to_string();
    let width = chrome.saturating_sub(1);
    if digits.chars().count() <= width {
        // Zero padded, so a column of numbers reads as a column, and a blank
        // between the number and the text.
        return format!("{digits:0>width$} ");
    }
    // A file past that spends the blank, and past that says it was cut. `clip`
    // adds the mark on top of what it kept, so it is asked for one less.
    match digits.chars().count() <= chrome {
        true => digits,
        false => clip(&digits, width),
    }
}

pub fn text_columns(view: View, panel: Panel, column: f32) -> (usize, usize) {
    match view {
        View::Files => (cols_of(panel, column).saturating_sub(GUTTER).max(1), GUTTER),
        _ => (cols_of(panel, column), 0),
    }
}

/// How many characters fit across a panel's content box.
///
/// The one place a pane's width becomes a column count. Wrapping, hit testing
/// and the selection band all have to agree on this number, so they all ask
/// here rather than each dividing by the column width themselves.
pub(crate) fn cols_of(panel: Panel, column: f32) -> usize {
    columns_in(panel.inset(PAD).w, column)
}

/// How tall the prompt is: the rows it was set to, whatever is in it.
///
/// It used to be the rows it took to hold what had been typed, climbing to
/// `rows` a line at a time, and that is what this stopped doing. A box that
/// grows moves the conversation above it on the character that wraps a line and
/// is a different size every time you look at it; the setting is a height, so
/// three rows is three rows empty and three rows full. Past that the text
/// scrolls inside the box, which is what [`prompt_skip`] is for.
pub fn input_height(rows: usize, line: f32) -> f32 {
    // The strip, not the box inside it: the layout insets this by `GAP` before
    // the prompt gets it, and forgetting that cost the last row of a full one.
    (rows.max(1) as f32 * line + 2.0 * INPUT_PAD + 2.0 * GAP).max(INPUT_H)
}

pub(crate) fn clip(text: &str, chars: usize) -> String {
    let mut out: String = text.chars().take(chars).collect();
    if text.chars().count() > chars {
        out.push('\u{2026}');
    }
    out
}

/// The same, from the other end: the last `chars` characters, with a mark where
/// the front was cut off.
///
/// For a value whose end is the part that changes. A URL clipped from the left
/// keeps the port and the path, which is what somebody typing one is looking at;
/// clipped from the right it says `http://localho…` on every endpoint there is.
pub(crate) fn tail(text: &str, chars: usize) -> String {
    let count = text.chars().count();
    if count <= chars {
        return text.to_string();
    }
    let mut out = String::from("\u{2026}");
    out.extend(text.chars().skip(count - chars.saturating_sub(1)));
    out
}

/// The file name, and enough of its parent to tell two `mod.rs` apart.
pub fn short_name(path: &str) -> String {
    let parts: Vec<&str> = path.rsplit('/').take(2).collect();
    match parts.as_slice() {
        [] => String::new(),
        [name] => (*name).to_string(),
        [name, parent, ..] if *name == "mod.rs" || *name == "index.ts" || *name == "__init__.py" => {
            format!("{parent}/{name}")
        }
        [name, ..] => (*name).to_string(),
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

#[cfg(test)]
pub(crate) mod testkit;

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use super::testkit::*;
    use crate::config::Config;
    use crate::design;
    #[allow(clippy::wildcard_imports)]
    use crate::settings::places::*;
    use crate::settings::paint::swatch;
    #[allow(clippy::wildcard_imports)]
    use crate::picker::metrics::*;
    use crate::picker::Row as PickerRow;
    use crate::settings::Row as SettingRow;
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
            popup_scroll: 0,
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
            popup_scroll: 0,
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
            popup_scroll: 0,
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
            popup_scroll: 0,
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
            popup_scroll: 0,
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
            popup_scroll: 0,
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
            popup_scroll: 0,
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

    /// The window with a menu open, laid out off the same shape the window is,
    /// which is what makes a row land where it is drawn.
    fn with_menu<'a>(dock: &'a Dock, menu: &'a Menu, w: f32, h: f32) -> Layout {
        let mut shape = shape(dock, &[]);
        shape.menu = Some(menu);
        Layout::compute(w, h, &shape)
    }

    fn render_menu(
        state: &State,
        w: f32,
        h: f32,
        dock: &Dock,
        menu: &Menu,
        hot: Option<Hit>,
    ) -> Rendered {
        render_menu_skinned(state, w, h, dock, menu, hot, Skin::from(&Config::default()))
    }

    /// The same menu under a palette of the caller's choosing, for the tests
    /// that open one in another theme.
    #[allow(clippy::too_many_arguments)]
    fn render_menu_skinned(
        state: &State,
        w: f32,
        h: f32,
        dock: &Dock,
        menu: &Menu,
        hot: Option<Hit>,
        skin: Skin,
    ) -> Rendered {
        let layout = with_menu(dock, menu, w, h);
        let scene = build(&Frame {
            state,
            scrolls: &crate::scroll::Scrolls::default(),
            file_scroll: 0,
            monitor: &Monitor::new(),
            dock,
            skin: &skin,
            layout: &layout,
            prompt: &typed_prompt("type here", 4),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot,
            trouble: None,
            esc_armed: false,
            popup_scroll: 0,
            cursor: (-100.0, -100.0),
            selection: None,
            menu: Some(menu),
            picker: None,
            settings: None,
        });
        Rendered {
            scene,
            layout,
            skin,
        }
    }


    /// The name one option box of the themes card carries: the presets down the
    /// left column, custom alone on the right.
    fn option_name(side: crate::settings::Side, option: usize) -> &'static str {
        match side {
            crate::settings::Side::Left => crate::config::THEMES[option],
            _ => "custom",
        }
    }

    /// The whole of what floating means, half one: an open menu takes the click
    /// that lands on it, even over a tab or a window button, and its margin
    /// swallows one rather than letting it through to what it covers.
    #[test]
    fn an_open_menu_takes_the_click_before_what_is_under_it() {
        let dock = Dock::new();
        let plain = Layout::compute(1400.0, 900.0, &shape(&dock, &[]));
        let (view, tab) = plain.placed(Space::TopRight).tabs[0];
        let at = middle(tab);
        assert_eq!(
            plain.hit(at.0, at.1),
            Some(Hit::Tab(view, Space::TopRight)),
            "the tab is what is under the pointer to begin with"
        );

        let menu = Menu::for_widget(at, view, Space::TopRight, false);
        let layout = with_menu(&dock, &menu, 1400.0, 900.0);
        assert_eq!(
            layout.hit(at.0, at.1),
            Some(Hit::Menu),
            "the pointer that opened it is on the menu's own margin"
        );
        for (index, row) in &layout.menu_rows {
            let (x, y) = middle(*row);
            assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(*index)));
        }
        // And over a window button, which is hit tested before everything else
        // in the window.
        let over_close = middle(plain.close);
        let menu = Menu::for_widget(over_close, view, Space::TopRight, false);
        let layout = with_menu(&dock, &menu, 1400.0, 900.0);
        assert!(matches!(
            layout.hit(over_close.0, over_close.1),
            Some(Hit::Menu | Hit::MenuRow(_))
        ));
    }

    /// The notch in the menu's top right corner is not the menu.
    ///
    /// Those pixels are cut out of the fill and out of the border, so what is on
    /// screen there is the pane behind them. `Panel::contains` is a plain
    /// rectangle and knew nothing about the cut, so a press on transparent
    /// pixels answered as the first row of the menu, which on a pane's menu was
    /// the row that opens the settings panel.
    #[test]
    fn a_press_in_the_menus_cut_corner_is_not_a_press_on_its_first_row() {
        let dock = Dock::new();
        let (w, h) = (1400.0, 900.0);
        let at = (500.0, 400.0);
        let plain = Layout::compute(w, h, &shape(&dock, &[]));
        let under = plain.hit(at.0 + 40.0, at.1 + 2.0);
        assert!(
            matches!(under, Some(Hit::Body(_))),
            "the corner is not over a pane, so this proves nothing"
        );

        let menu = Menu::for_widget(at, View::Plan, Space::TopLeft, false);
        let layout = with_menu(&dock, &menu, w, h);
        let box_ = layout.menu;
        let cut = cut_of(box_);
        assert!(cut > 2.0, "the box lost its corner, so there is nothing to test");

        // Every point strictly inside the triangle answers for whatever is
        // behind the menu, never for the menu or a row of it.
        let mut probed = 0;
        for down in 1..cut as usize {
            for left in 1..cut as usize {
                let (x, y) = (box_.x + box_.w - left as f32, box_.y + down as f32);
                if left as f32 + down as f32 >= cut {
                    continue;
                }
                probed += 1;
                assert!(
                    !matches!(layout.hit(x, y), Some(Hit::Menu | Hit::MenuRow(_))),
                    "({x}, {y}) is in the notch and answered as the menu"
                );
            }
        }
        assert!(probed > 4, "the probe covered nothing");

        // And a pixel just inside the diagonal on the same rows still is the
        // menu, so the rejection is the notch and not the whole corner. The
        // rows sit flush against the border now, so that pixel is the first
        // row itself.
        let (x, y) = (box_.x + box_.w - cut - 1.0, box_.y + 1.5);
        assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(0)));
    }

    /// The row under the pointer is the row that acts, and a greyed one acts
    /// on nothing while still keeping its place.
    #[test]
    fn the_row_under_the_pointer_is_the_row_that_acts() {
        use crate::menu::Item;
        let dock = Dock::new();
        let at = (600.0, 400.0);
        let picked = |menu: &Menu| -> Vec<Option<Item>> {
            let layout = with_menu(&dock, menu, 1400.0, 900.0);
            layout
                .menu_rows
                .iter()
                .map(|(_, row)| {
                    let (x, y) = middle(*row);
                    match layout.hit(x, y) {
                        Some(Hit::MenuRow(index)) => menu.pick(index),
                        other => panic!("{other:?} is not a row"),
                    }
                })
                .collect()
        };
        assert_eq!(
            picked(&Menu::for_widget(at, View::Plan, Space::TopRight, true)),
            vec![
                Some(Item::Settings),
                Some(Item::CopySelection),
                Some(Item::Close),
                Some(Item::Widgets(false)),
            ]
        );
        // The copy row is the greyed one when there is nothing selected, and it
        // keeps its place: the rows either side of it act as before.
        assert_eq!(
            picked(&Menu::for_widget(at, View::Plan, Space::TopRight, false)),
            vec![
                Some(Item::Settings),
                None,
                Some(Item::Close),
                Some(Item::Widgets(false)),
            ],
            "a row with nothing to copy is drawn and refuses to act"
        );
        // With the flyout open, the column's rows stay exactly where they
        // were, and the flyout's rows answer in their own box beside it.
        let mut open = Menu::for_widget(at, View::Plan, Space::TopRight, false);
        open.fold(3, &dock);
        assert_eq!(
            picked(&open),
            vec![
                Some(Item::Settings),
                None,
                Some(Item::Close),
                Some(Item::Widgets(true)),
            ],
            "opening the flyout moved a row of the column"
        );
        let layout = with_menu(&dock, &open, 1400.0, 900.0);
        let listed: Vec<View> = View::ALL
            .into_iter()
            .filter(|view| *view != View::Agent)
            .collect();
        assert_eq!(layout.menu_fly_rows.len(), listed.len());
        for ((index, row), view) in layout.menu_fly_rows.iter().zip(listed) {
            let (x, y) = middle(*row);
            assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(*index)));
            assert_eq!(open.pick(*index), Some(Item::Widget(view, false)));
        }
        // The flyout hangs beside the column, top-aligned with its header,
        // and never over it.
        assert!(layout.menu_fly.x >= layout.menu.x + layout.menu.w - 0.5);
        let header = layout
            .menu_rows
            .iter()
            .find(|(index, _)| *index == 3)
            .map(|(_, panel)| panel.y)
            .expect("the header is on screen");
        assert!((layout.menu_fly.y + MENU_EDGE - header).abs() < 0.6);
    }

    /// A menu opened in the corner has to stay on the surface. The part that
    /// hangs off is not merely invisible: no pointer can reach it, so the rows
    /// down there could not be picked at all.
    #[test]
    fn a_menu_opened_at_an_edge_stays_reachable() {
        let dock = Dock::new();
        let (w, h) = (1400.0, 900.0);
        for at in [(w - 2.0, h - 2.0), (w + 40.0, h + 40.0), (-10.0, -10.0)] {
            let menu = Menu::for_widget(at, View::Files, Space::BottomRight, false);
            let layout = with_menu(&dock, &menu, w, h);
            let box_ = layout.menu;
            assert!(box_.x >= 0.0 && box_.y >= 0.0, "{at:?}: {box_:?}");
            assert!(box_.x + box_.w <= w + 0.01, "{at:?}: {box_:?}");
            assert!(box_.y + box_.h <= h + 0.01, "{at:?}: {box_:?}");
            assert_eq!(layout.menu_rows.len(), menu.rows.len());
            for (index, row) in &layout.menu_rows {
                let (x, y) = middle(*row);
                assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(*index)), "{at:?}");
            }
        }
    }

    /// Opening the flyout moves nothing: the column keeps its four rows and
    /// its box, and the widgets answer in a second box beside the header.
    #[test]
    fn the_flyout_opens_beside_the_header_and_moves_no_row() {
        use crate::menu::Item;
        let dock = Dock::new();
        let (w, h) = (1400.0, 900.0);
        let at = (400.0, 300.0);
        let shut = Menu::for_widget(at, View::Plan, Space::TopLeft, false);
        let closed = with_menu(&dock, &shut, w, h);
        assert_eq!(closed.menu_rows.len(), 4);
        assert!(closed.menu_fly.w < 1.0, "a shut flyout has no box");

        // Opened the way the pointer opens it: whatever row the press lands on
        // is the row that folds, so the layout and the model cannot disagree
        // about which header was pressed.
        let mut menu = shut.clone();
        let header = closed
            .menu_rows
            .iter()
            .find(|(index, _)| matches!(menu.pick(*index), Some(Item::Widgets(_))))
            .map(|(_, panel)| *panel)
            .expect("the Widgets row is on screen");
        let (px, py) = middle(header);
        let Some(Hit::MenuRow(pressed)) = closed.hit(px, py) else {
            panic!("the Widgets row is not pressable")
        };
        assert!(menu.fold(pressed, &dock));
        let layout = with_menu(&dock, &menu, w, h);
        assert_eq!(layout.menu_rows.len(), 4, "a row of the column moved");
        assert_eq!(layout.menu.h, closed.menu.h, "the box changed size");
        assert_eq!(layout.menu.x, closed.menu.x, "the box moved sideways");
        assert_eq!(layout.menu_fly_rows.len(), View::ALL.len() - 1);
        // Every flyout row is in the flyout's box, in one column, and answers.
        for (index, row) in &layout.menu_fly_rows {
            assert_eq!(row.x, layout.menu_fly.x, "row {index} is in a second column");
            assert_eq!(row.w, layout.menu_fly.w);
            assert!(
                row.y >= layout.menu_fly.y
                    && row.y + row.h <= layout.menu_fly.y + layout.menu_fly.h
            );
            let (x, y) = middle(*row);
            assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(*index)));
        }
        assert_eq!(menu.pick(pressed), Some(Item::Widgets(true)));

        // A second press on the same row shuts it again, and the flyout's box
        // goes with it.
        let again = with_menu(&dock, &menu, w, h);
        let (px, py) = middle(again.menu_rows[pressed].1);
        assert_eq!(again.hit(px, py), Some(Hit::MenuRow(pressed)));
        let mut shut_again = menu.clone();
        assert!(shut_again.fold(pressed, &dock));
        let back = with_menu(&dock, &shut_again, w, h);
        assert_eq!(back.menu_rows.len(), closed.menu_rows.len());
        assert_eq!(back.menu.h, closed.menu.h);
        assert!(back.menu_fly.w < 1.0);
    }

    /// A menu opened near the bottom of a very short window shows what there
    /// is room for and scrolls through the rest. Rows past the bottom used to
    /// be dropped: not placed, not drawn and not reachable, with nothing on
    /// screen saying so.
    #[test]
    fn a_menu_too_tall_for_the_window_scrolls_instead_of_dropping_rows() {
        let dock = Dock::new();
        let (w, short) = (900.0, 60.0);
        let mut menu = Menu::for_widget((w - 2.0, short - 2.0), View::Plan, Space::TopLeft, false);
        let layout = with_menu(&dock, &menu, w, short);
        let placed: Vec<usize> = layout.menu_rows.iter().map(|(i, _)| *i).collect();
        assert!(
            placed.len() < menu.main_len(),
            "this window is not short enough to prove anything"
        );
        assert_eq!(placed.first().copied(), Some(0));
        assert!(layout.menu.y >= 0.0);
        assert!(layout.menu.y + layout.menu.h <= short + 0.01);
        let capacity = layout.menu_capacity();
        assert_eq!(capacity, placed.len());

        // Scrolled to the end, the last row is on screen and the first is not,
        // and no row has left the window.
        menu.scroll(menu.rows.len(), true, capacity);
        let layout = with_menu(&dock, &menu, w, short);
        let placed: Vec<usize> = layout.menu_rows.iter().map(|(i, _)| *i).collect();
        assert_eq!(
            placed.last().copied(),
            Some(menu.main_len() - 1),
            "the last row is reachable"
        );
        assert_ne!(placed.first().copied(), Some(0));
        for (index, row) in &layout.menu_rows {
            let (x, y) = middle(*row);
            assert!(row.y >= 0.0 && row.y + row.h <= short + 0.01, "{row:?}");
            assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(*index)));
            assert!(menu.pick(*index).is_some(), "row {index} is on screen and does nothing");
        }
        // The flyout stays whole and inside the window even here.
        menu.scroll(menu.rows.len(), false, capacity);
        menu.fold(3, &dock);
        let layout = with_menu(&dock, &menu, w, short);
        assert_eq!(layout.menu_fly_rows.len(), View::ALL.len() - 1);
        assert!(layout.menu_fly.y >= 0.0);
    }

    /// The box is measured in columns of the size its rows are written at.
    ///
    /// It was measured at the title bar's size and drawn at the menu's, so at
    /// the defaults every row ended about 23 pixels short of its own box and
    /// the group chevron floated most of an inch past the end of its label.
    #[test]
    fn the_box_is_as_wide_as_the_text_it_holds_and_no_wider() {
        let dock = Dock::new();
        let menu = Menu::for_widget((300.0, 200.0), View::Plan, Space::TopLeft, true);
        let mut shape = shape(&dock, &[]);
        shape.menu = Some(&menu);
        // The title bar's column is deliberately far off the menu's here, which
        // is the mismatch that produced the slab.
        shape.column = 16.0;
        shape.menu_column = 7.0;
        let layout = Layout::compute(1400.0, 900.0, &shape);
        // The gutter, and the one column of slack that keeps a wide icon
        // glyph from wrapping an exact-fit label out of its row.
        let want = (menu.width_chars() + MENU_GUTTER + 1) as f32 * 7.0 + MENU_PAD * 2.0;
        assert!(
            (layout.menu.w - want).abs() < 0.01,
            "the box is sized from the wrong font: {} against {want}",
            layout.menu.w
        );
    }

    /// A menu wider or taller than the window is cut to the window rather than
    /// run off the edge of it.
    #[test]
    fn a_menu_bigger_than_the_window_is_cut_to_it() {
        let dock = Dock::new();
        let mut menu = Menu::for_widget((0.0, 0.0), View::Plan, Space::TopLeft, false);
        menu.fold(3, &dock);
        let (w, h) = (90.0, 120.0);
        let layout = with_menu(&dock, &menu, w, h);
        assert!(layout.menu.x >= 0.0 && layout.menu.x + layout.menu.w <= w + 0.01);
        assert!(layout.menu.y >= 0.0 && layout.menu.y + layout.menu.h <= h + 0.01);
        for (index, row) in &layout.menu_rows {
            let (x, y) = middle(*row);
            assert!(row.x >= 0.0 && row.x + row.w <= w + 0.01, "{row:?}");
            assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(*index)));
        }
    }

    /// A lit row covers exactly its own band, keeps off the two hairlines the
    /// border stands in, and does not paint into the corner the box does not
    /// have.
    ///
    /// The hover fill used to be the row rectangle as it was placed: full box
    /// width, so it brightened the border for the height of the pointer, and
    /// square, so on the first row it painted a solid triangle out into the
    /// notch where the desktop shows through.
    #[test]
    fn a_lit_row_covers_its_own_band_and_nothing_else() {
        let dock = Dock::new();
        let (w, h) = (1400.0, 900.0);
        let mut menu = Menu::for_widget((400.0, 300.0), View::Plan, Space::TopLeft, true);
        menu.fold(3, &dock);
        for (index, _) in with_menu(&dock, &menu, w, h).menu_rows.clone() {
            let out = render_menu(
                &busy_state(),
                w,
                h,
                &dock,
                &menu,
                Some(Hit::MenuRow(index)),
            );
            let box_ = out.layout.menu;
            let row = out
                .layout
                .menu_rows
                .iter()
                .find(|(at, _)| *at == index)
                .map(|(_, panel)| *panel)
                .expect("the row is on screen");
            let lit: Vec<&Rect> = out
                .scene
                .over_rects
                .iter()
                .filter(|r| r.rgba() == out.skin.hot)
                .collect();
            assert_eq!(lit.len(), 1, "row {index} lit {} rectangles", lit.len());
            let [x, y, rw, rh] = lit[0].xywh();
            assert_eq!((y, rh), (row.y, row.h), "the highlight is not the row's band");
            assert!(x >= row.x + 1.0 - 0.01, "it covers the left hairline");
            assert!(
                x + rw <= row.x + row.w - 1.0 + 0.01,
                "it covers the right hairline"
            );
            // The first row on screen is the one the box's corner is taken out
            // of, and it is taken out at the same 45 degrees.
            let cut = lit[0].extra()[1];
            match row.y <= box_.y + MENU_EDGE + 0.01 {
                true => {
                    assert!(cut > 0.0, "the first row painted over the cut corner");
                    // The two diagonals start at the same x on the row's own
                    // first line, and both run at 45 degrees, so they are the
                    // same line.
                    assert!(
                        (x + rw - cut - (box_.x + box_.w - cut_of(box_) + MENU_EDGE)).abs() < 0.01,
                        "the row's diagonal does not follow the box's"
                    );
                }
                false => assert_eq!(cut, 0.0, "row {index} is not at the corner"),
            }
        }
    }

    /// No rectangle drawn for the menu crosses the notch in its corner, at any
    /// row and in either state a group can be in.
    #[test]
    fn nothing_the_menu_draws_reaches_into_its_cut_corner() {
        let dock = Dock::new();
        let (w, h) = (1400.0, 900.0);
        let mut opened = Menu::for_widget((400.0, 300.0), View::Plan, Space::TopLeft, true);
        opened.fold(0, &dock);
        for menu in [
            Menu::for_widget((400.0, 300.0), View::Plan, Space::TopLeft, true),
            opened,
        ] {
            let rows = with_menu(&dock, &menu, w, h).menu_rows.len();
            for hot in (0..rows).map(|at| Some(Hit::MenuRow(at))).chain([None]) {
                let out = render_menu(&busy_state(), w, h, &dock, &menu, hot);
                let box_ = out.layout.menu;
                let cut = cut_of(box_);
                for rect in &out.scene.over_rects {
                    let [x, y, rw, rh] = rect.xywh();
                    // A point of this rectangle inside the box's notch, unless
                    // the rectangle carries the same diagonal itself.
                    let reach = (box_.x + box_.w - (x + rw)) + (y - box_.y);
                    if reach >= cut - 0.01 {
                        continue;
                    }
                    let own = rect.extra()[1];
                    assert!(
                        own >= cut - reach - 0.01,
                        "{hot:?}: a rectangle at {:?} crosses the cut with a {own} cut of its own",
                        rect.xywh()
                    );
                    let _ = rh;
                }
            }
        }
    }

    /// The border is drawn after the rows, so a lit row cannot break the
    /// outline it sits inside.
    #[test]
    fn the_border_is_painted_over_the_rows_rather_than_under_them() {
        let dock = Dock::new();
        let menu = Menu::for_widget((400.0, 300.0), View::Plan, Space::TopLeft, true);
        let out = render_menu(
            &busy_state(),
            1400.0,
            900.0,
            &dock,
            &menu,
            Some(Hit::MenuRow(1)),
        );
        let lit = out
            .scene
            .over_rects
            .iter()
            .position(|r| r.rgba() == out.skin.hot)
            .expect("a row is lit");
        let edge = out
            .scene
            .over_rects
            .iter()
            .position(|r| r.rgba() == out.skin.edge_focus && r.extra()[3] > 0.0)
            .expect("the box has a border");
        assert!(edge > lit, "the border is painted under the lit row");
    }

    /// The flyout is a second box beside the column: its rows are written in
    /// it, its text clears both of its borders by the padding token, and the
    /// header's chevron keeps pointing out to the side where the rows are.
    #[test]
    fn the_open_flyout_is_a_box_beside_the_column() {
        use crate::menu::Item;
        let dock = Dock::new();
        let (w, h) = (1400.0, 900.0);
        let mut menu = Menu::for_widget((400.0, 300.0), View::Plan, Space::TopLeft, false);
        menu.fold(3, &dock);
        let out = render_menu(&busy_state(), w, h, &dock, &menu, None);

        let written = |label: &str| -> Panel {
            out.scene
                .over_texts
                .iter()
                .find(|text| text.runs.iter().any(|run| run.text.contains(label)))
                .unwrap_or_else(|| panic!("{label} is not drawn"))
                .at
        };
        // Every widget's label is written inside the flyout's box, not the
        // column's.
        let (column, fly) = (out.layout.menu, out.layout.menu_fly);
        assert!(fly.w >= 1.0, "the flyout has no box");
        for view in crate::dock::View::ALL {
            let row = written(view.label());
            assert!(
                row.x >= fly.x && row.x + row.w <= fly.x + fly.w + 0.01,
                "{} is not written in the flyout: {row:?} against {fly:?}",
                view.label()
            );
            // And every label fits its one-line row with the gutter and a
            // column to spare: a row measured to an exact fit wraps its
            // longest labels out of sight, which is how ACTIVITY and
            // HARDWARE shipped as two nameless checkboxes.
            // The column render_menu's shape carries; the box was placed
            // with it, so the row is measured in the same unit it was sized.
            let column = 7.0;
            let cols = (row.w / column).floor() as usize;
            assert!(
                view.label().chars().count() + MENU_GUTTER < cols,
                "{} has no slack in {cols} columns",
                view.label()
            );
        }
        // Text clears the borders by the padding token in both boxes.
        for text in &out.scene.over_texts {
            let box_ = match text.at.x >= fly.x - 0.01 && fly.w >= 1.0 {
                true => fly,
                false => column,
            };
            assert!(
                text.at.x >= box_.x + MENU_PAD - 0.01,
                "{:?} touches the left border",
                text.at
            );
            assert!(
                text.at.x + text.at.w <= box_.x + box_.w - MENU_PAD + 0.01,
                "{:?} touches the right border",
                text.at
            );
        }

        // The chevron points out to the side in both states: that is where
        // the rows go, and where they are.
        let marks: Vec<&str> = out
            .scene
            .over_texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
            .filter(|t| *t == icons::SUBMENU.to_string())
            .collect();
        assert_eq!(marks.len(), 1, "the header keeps its one side chevron");
        assert_eq!(menu.pick(3), Some(Item::Widgets(true)));
    }

    /// The row that opens a group is marked twice: the mark in the gutter in
    /// front, saying what the row is, and the chevron at its END, saying it
    /// opens.
    #[test]
    fn the_row_that_opens_is_marked_in_its_gutter_and_at_its_end() {
        use crate::menu::Item;
        let dock = Dock::new();
        for open in [false, true] {
            let mut menu = Menu::for_widget((400.0, 300.0), View::Plan, Space::TopLeft, false);
            if open {
                menu.fold(3, &dock);
            }
            let out = render_menu(&busy_state(), 1400.0, 900.0, &dock, &menu, None);
            let row = out
                .layout
                .menu_rows
                .iter()
                .find(|(index, _)| matches!(menu.pick(*index), Some(Item::Widgets(_))))
                .map(|(_, panel)| *panel)
                .expect("the Widgets row is on screen");
            // The same side chevron in both states: the rows fly out to the
            // side, and that is where the mark points.
            let want = icons::SUBMENU;
            let marks: Vec<&Text> = out
                .scene
                .over_texts
                .iter()
                .filter(|text| text.runs.iter().any(|run| run.text == want.to_string()))
                .filter(|text| {
                    text.at.y >= row.y - 0.01 && text.at.y + text.at.h <= row.y + row.h + 0.01
                })
                .collect();
            assert_eq!(marks.len(), 1, "the Widgets row has one chevron");
            let mark = marks[0];
            assert!(
                mark.at.y >= row.y - 0.01 && mark.at.y + mark.at.h <= row.y + row.h + 0.01,
                "the mark is not on the Widgets row: {:?} against {row:?}",
                mark.at
            );
            // At the end of the row, not after the label: the label starts at
            // the left of the row and the mark is over in the last columns.
            assert!(
                mark.at.x > row.x + row.w * 0.5,
                "the mark is not at the end of the row: {:?} in {row:?}",
                mark.at
            );
            assert!(mark.at.x + mark.at.w <= row.x + row.w - MENU_PAD + 0.01);
            // And nothing of the old plus and minus is anywhere on the overlay.
            // Written out rather than named: these are Font Awesome's filled
            // plus-square and minus-square, which the picker's mark used to be
            // drawn with and which no longer have a constant anywhere.
            let runs: Vec<&str> = out
                .scene
                .over_texts
                .iter()
                .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
                .collect();
            for gone in ['\u{f0fe}', '\u{f146}'] {
                assert!(
                    !runs.contains(&gone.to_string().as_str()),
                    "U+{:04X} is still drawn on a menu row",
                    gone as u32
                );
            }
            // The label is written from the left of the row, past the gutter.
            let label = out
                .scene
                .over_texts
                .iter()
                .find(|text| text.runs.iter().any(|run| run.text.contains("Widgets")))
                .expect("the label is drawn");
            assert!(label.at.x < mark.at.x, "the label is not before the mark");
            // And the gutter in front of that label holds the widgets grid, in
            // the same shaped line as the label so the two cannot come apart.
            assert_eq!(
                label.runs.first().map(|run| run.text.as_str()),
                Some(icons::WIDGETS.to_string().as_str()),
                "the Widgets row has nothing in its gutter"
            );
            assert!(
                label.runs[0].icon,
                "the mark is shaped in the label's font, so it draws as a box"
            );
        }
    }

    /// A row that opens a group and a row that acts do not read alike: the
    /// header is written in the brighter of the two inks and carries a chevron,
    /// and the rows that act are written in the body ink and carry none.
    #[test]
    fn a_row_that_opens_reads_differently_from_a_row_that_acts() {
        let dock = Dock::new();
        let menu = Menu::for_widget((400.0, 300.0), View::Plan, Space::TopLeft, true);
        let out = render_menu(&busy_state(), 1400.0, 900.0, &dock, &menu, None);
        let ink = |label: &str| -> [u8; 4] {
            out.scene
                .over_texts
                .iter()
                .find(|text| text.runs.iter().any(|run| run.text.contains(label)))
                .and_then(|text| text.runs.last().and_then(|run| run.color))
                .unwrap_or_else(|| panic!("{label} is not drawn"))
        };
        assert_eq!(ink("Widgets"), out.skin.bright);
        assert_eq!(ink("Settings"), out.skin.body, "Settings acts; it is not a header");
        assert_eq!(ink("Copy selection"), out.skin.body);
        assert_eq!(ink("Close this widget"), out.skin.body);
        assert_ne!(out.skin.bright, out.skin.body);
        // A row that cannot act is dimmer than either.
        let greyed = Menu::for_widget((400.0, 300.0), View::Plan, Space::TopLeft, false);
        let out = render_menu(&busy_state(), 1400.0, 900.0, &dock, &greyed, None);
        let dim = out
            .scene
            .over_texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text.contains("Copy selection")))
            .and_then(|text| text.runs.last().and_then(|run| run.color))
            .expect("the greyed row is drawn");
        assert_eq!(dim, out.skin.dim);
    }

    /// The menu floats: it is painted on the floating layer, above the pane
    /// text it covers, and inside its own box. In the base layer its rows would
    /// be written under the box that is meant to hold them.
    #[test]
    fn the_whole_menu_is_drawn_on_the_floating_layer() {
        let dock = Dock::hiding(&[View::Hardware]);
        let mut menu = Menu::for_widget((400.0, 200.0), View::Plan, Space::TopLeft, false);
        menu.fold(3, &dock);
        let out = render_menu(&busy_state(), 1400.0, 900.0, &dock, &menu, None);
        let box_ = out.layout.menu;
        let runs: Vec<String> = out
            .scene
            .over_texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.text.clone()))
            .collect();
        // Every switchable view; the agent-output one has no switch here.
        for view in View::ALL.into_iter().filter(|view| *view != View::Agent) {
            assert!(
                runs.iter().any(|text| text.contains(view.label())),
                "{} is not on the overlay: {runs:?}",
                view.label()
            );
        }
        // Switches, marked in the gutter: a ticked box for the widgets in
        // the window, an empty one for the widget that is out.
        let empty = runs
            .iter()
            .filter(|text| *text == &icons::UNCHECKED.to_string());
        assert_eq!(empty.count(), 1, "only one widget is closed");
        assert_eq!(
            runs.iter()
                .filter(|text| *text == &icons::CHECKED.to_string())
                .count(),
            View::ALL.len() - 2
        );
        // Everything is written inside one of the two boxes, and each box has
        // a surface under it on the overlay.
        let fly = out.layout.menu_fly;
        for text in &out.scene.over_texts {
            let inside = |b: Panel| {
                text.at.y >= b.y - 0.01
                    && text.at.y + text.at.h <= b.y + b.h + 0.01
                    && text.at.x >= b.x - 0.01
                    && text.at.x + text.at.w <= b.x + b.w + 0.01
            };
            assert!(
                inside(box_) || inside(fly),
                "{:?} is outside {box_:?} and {fly:?}",
                text.at
            );
        }
        for b in [box_, fly] {
            assert!(
                out.scene
                    .over_rects
                    .iter()
                    .any(|r| r.xywh() == [b.x, b.y, b.w, b.h] && r.extra()[3] == 0.0),
                "a menu box has no surface"
            );
        }
    }

    /// Every row of the menu is drawn with a mark in its gutter, and it is the
    /// mark the model names. Four of them shipped blank: copy selection, close
    /// this widget, Widgets and paste each spent the gutter on a space, which
    /// reads as a row whose icon failed to draw rather than a row without one.
    #[test]
    fn every_menu_row_is_drawn_with_its_own_mark_in_the_gutter() {
        use crate::menu::Item;
        let dock = Dock::hiding(&[View::Hardware]);
        let mut widget = Menu::for_widget((400.0, 300.0), View::Plan, Space::TopLeft, true);
        widget.fold(3, &dock);
        for menu in [widget, Menu::for_input((400.0, 300.0), true)] {
            let out = render_menu(&busy_state(), 1400.0, 900.0, &dock, &menu, None);
            let mut seen = 0;
            let placed = out
                .layout
                .menu_rows
                .iter()
                .chain(out.layout.menu_fly_rows.iter());
            for (index, panel) in placed {
                let item = menu.rows[*index].item;
                let icon = item.icon().expect("every row has a mark");
                let line = out
                    .scene
                    .over_texts
                    .iter()
                    .find(|text| {
                        text.at.y >= panel.y - 0.01
                            && text.at.y + text.at.h <= panel.y + panel.h + 0.01
                            && text.runs.iter().any(|run| run.text.contains(item.label()))
                    })
                    .unwrap_or_else(|| panic!("{item:?} is not drawn"));
                assert_eq!(
                    line.runs.first().map(|run| run.text.as_str()),
                    Some(icon.to_string().as_str()),
                    "{item:?} carries the wrong mark"
                );
                assert!(line.runs[0].icon, "{item:?}: the mark is not a symbol run");
                seen += 1;
            }
            assert_eq!(seen, menu.rows.len(), "not every row was placed");
        }
        // The four the requirement named, on the rows the requirement named.
        assert_eq!(Item::CopySelection.icon(), Some(icons::COPY));
        assert_eq!(Item::Close.icon(), Some(icons::CLOSE_WIDGET));
        assert_eq!(Item::Widgets(false).icon(), Some(icons::WIDGETS));
        assert_eq!(Item::Paste.icon(), Some(icons::PASTE));
    }




    /// The other half of floating: the menu is on the overlay layer, both its
    /// rectangles and its rows, and nothing of the window is up there with it.
    ///
    /// This used to assert that the menu's rectangles came last in the one
    /// rectangle list, which was true and useless. The renderer paints every
    /// rectangle of a layer and then every glyph of it, so being last among the
    /// rectangles still put the menu's box under all of the pane text it covered,
    /// and the rows were illegible over any pane with writing in it. Only the
    /// overlay can say "over that text", so that is what is asserted.
    #[test]
    fn the_menu_is_painted_on_the_overlay_layer() {
        let dock = Dock::new();
        let menu = Menu::for_widget((500.0, 400.0), View::Plan, Space::TopRight, false);
        let out = render_menu(&busy_state(), 1400.0, 900.0, &dock, &menu, None);
        let box_ = out.layout.menu;

        // The bug, in the one condition that reproduced it: there is pane text
        // under the menu. Without this the test would pass over an empty window.
        assert!(
            text_over(&out.scene.texts, box_),
            "nothing is written under the menu, so this proves nothing"
        );

        // Found by where it is, not by what colour it is: at the shipped
        // opacity every solid surface in the palette is already fully opaque,
        // so the menu's fill is the same colour as the prompt's.
        let surface = |rects: &[Rect]| {
            rects
                .iter()
                .any(|r| r.xywh() == [box_.x, box_.y, box_.w, box_.h] && r.extra()[3] == 0.0)
        };
        assert!(surface(&out.scene.over_rects), "the menu has no surface");
        assert!(
            !surface(&out.scene.rects),
            "the menu's surface is still in the base layer, under every glyph"
        );

        // Every rectangle and every text on the overlay belongs to the menu, and
        // nothing of the panes is up there.
        assert!(!out.scene.over_texts.is_empty(), "the rows are not drawn");
        for rect in &out.scene.over_rects {
            let [x, y, w, h] = rect.xywh();
            assert!(
                x >= box_.x - 0.01
                    && y >= box_.y - 0.01
                    && x + w <= box_.x + box_.w + 0.01
                    && y + h <= box_.y + box_.h + 0.01,
                "{:?} is on the overlay but is not the menu",
                rect.xywh()
            );
        }
        for text in &out.scene.over_texts {
            assert!(
                text.at.x >= box_.x - 0.01
                    && text.at.y >= box_.y - 0.01
                    && text.at.x + text.at.w <= box_.x + box_.w + 0.01,
                "{:?} is on the overlay but is not a menu row",
                text.at
            );
        }

        // The rows are legible, a row that opens a group is brighter than a
        // row that acts, and a row that cannot act says so by weight. Read off
        // the overlay: a label still in the base layer would be drawn under the
        // menu's own box.
        let runs: Vec<(&str, Option<[u8; 4]>)> = out
            .scene
            .over_texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| (r.text.as_str(), r.color)))
            .collect();
        let base = text_of(&out.scene);
        for (label, tint) in [
            ("Widgets", out.skin.bright),
            ("Settings", out.skin.body),
            ("Copy selection", out.skin.dim),
            ("Close this widget", out.skin.body),
        ] {
            let run = runs
                .iter()
                .find(|(text, _)| text.contains(label))
                .unwrap_or_else(|| panic!("{label} is not on the overlay: {runs:?}"));
            assert_eq!(run.1, Some(tint), "{label}");
            assert!(!base.contains(label), "{label} is drawn in the base layer");
        }
    }

    /// Shaded, the window is one strip and the menu is still reachable, so it
    /// still has to be drawn: the shaded path takes an early return and had to
    /// keep painting the overlay through it.
    #[test]
    fn a_menu_over_the_shaded_strip_is_still_drawn() {
        let dock = Dock::new();
        let menu = Menu::for_widget((300.0, 10.0), View::Plan, Space::TopRight, false);
        let mut shape = shape(&dock, &["a.rs"]);
        shape.shaded = true;
        shape.menu = Some(&menu);
        let layout = Layout::compute(1200.0, 800.0, &shape);
        assert!(layout.shaded);
        let skin = Skin::from(&Config::default());
        let state = busy_state();
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
            popup_scroll: 0,
            cursor: (-100.0, -100.0),
            selection: None,
            menu: Some(&menu),
            picker: None,
            settings: None,
        });
        assert!(!scene.over_rects.is_empty(), "the menu box is not drawn");
        let rows: String = scene
            .over_texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
            .collect();
        assert!(rows.contains("Close this widget"), "{rows}");
        // And the base layer is still the bar and the strip's contents: the menu
        // hangs below the strip, on the overlay, over the bar. The bar covering
        // the whole surface is what item 19 asked for and is checked by
        // `shading_leaves_the_bar_and_nothing_else`, so it is the one rect
        // allowed past the strip here.
        let bar = Panel::new(0.0, 0.0, 1200.0, 800.0);
        for rect in &scene.rects {
            let [_, y, _, h] = rect.xywh();
            let is_bar = rect.xywh() == [bar.x, bar.y, bar.w, bar.h];
            assert!(
                is_bar || y + h <= TITLE_H + 0.01,
                "{rect:?} reaches past the strip"
            );
        }
    }

    /// Only a row that can act lights up. Highlighting a greyed one promises
    /// something will happen when the button comes down and it will not.
    #[test]
    fn a_greyed_row_does_not_light_up_under_the_pointer() {
        let dock = Dock::new();
        // No selection, so the copy row is the one that cannot act.
        let menu = Menu::for_widget((500.0, 400.0), View::Plan, Space::TopRight, false);
        let lit = |hot: Option<Hit>| {
            let out = render_menu(&busy_state(), 1400.0, 900.0, &dock, &menu, hot);
            let box_ = out.layout.menu;
            out.scene
                .over_rects
                .iter()
                .filter(|r| r.rgba() == out.skin.hot && box_.contains(r.xywh()[0], r.xywh()[1]))
                .count()
        };
        assert_eq!(lit(Some(Hit::MenuRow(1))), 0, "copy has nothing to copy");
        assert_eq!(lit(Some(Hit::MenuRow(0))), 1, "settings opens the panel");
        assert_eq!(lit(Some(Hit::MenuRow(2))), 1, "close acts");
        assert_eq!(lit(None), 0);
    }

    /// The lit row's band is the skin's hover, which is the theme's accent:
    /// under every preset the menu answers the pointer in the window's own
    /// hue, never in another theme's.
    ///
    /// Rendered per theme the way the title strip test is, so a band tinted
    /// from anything but the skin shows up as a matrix colour in a red window.
    #[test]
    fn the_menus_lit_row_wears_the_theme_it_is_given() {
        let dock = Dock::new();
        let menu = Menu::for_widget((500.0, 400.0), View::Plan, Space::TopRight, false);
        for name in crate::config::THEMES {
            let config = crate::config::theme(name).expect(name);
            let out = render_menu_skinned(
                &busy_state(),
                1400.0,
                900.0,
                &dock,
                &menu,
                Some(Hit::MenuRow(0)),
                Skin::from(&config),
            );
            let box_ = out.layout.menu;
            let band = out
                .scene
                .over_rects
                .iter()
                .find(|rect| {
                    rect.rgba() == out.skin.hot && box_.contains(rect.xywh()[0], rect.xywh()[1])
                })
                .unwrap_or_else(|| panic!("{name}: the lit row has no band"));
            // The band's hue is the theme's accent, off the config itself.
            assert_eq!(
                [band.rgba()[0], band.rgba()[1], band.rgba()[2]],
                [
                    config.accent[0] as f32 / 255.0,
                    config.accent[1] as f32 / 255.0,
                    config.accent[2] as f32 / 255.0,
                ],
                "{name}: the band is not the theme's accent"
            );
        }
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
        // The room the panes had is unclaimed rather than claimed by a space
        // with nothing in it.
        assert_eq!(out.layout.hit(700.0, 450.0), None);
    }

    /// The window with the folder picker up, laid out and drawn off one shape,
    /// which is what makes a row land where it is drawn.
    fn render_picker(picker: &Picker, w: f32, h: f32, hot: Option<Hit>) -> Rendered {
        let dock = Dock::new();
        let state = State::new();
        let mut shape = shape(&dock, &[]);
        shape.picker = Some(picker);
        let layout = Layout::compute(w, h, &shape);
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
            hot,
            trouble: None,
            esc_armed: false,
            popup_scroll: 0,
            cursor: (-100.0, -100.0),
            selection: None,
            menu: None,
            picker: Some(picker),
            settings: None,
        });
        Rendered {
            scene,
            layout,
            skin,
        }
    }

    /// What the picker's row at `index` says, which is the folder's name for a
    /// row that is one.
    fn said(picker: &Picker, index: usize) -> String {
        picker
            .row(index)
            .map(|row| picker.label(row))
            .unwrap_or_default()
    }

    /// The hairline box the picker draws inside a mark's region, if it drew one.
    ///
    /// Found by shape rather than by position: it is the only stroked rectangle
    /// that fits inside the region, and everything else in there is a solid bar.
    fn outline_of(out: &Rendered, mark: Panel) -> Option<Rect> {
        out.scene
            .rects
            .iter()
            .find(|rect| rect.extra()[3] > 0.0 && inside(**rect, mark))
            .copied()
    }

    /// How many one pixel bars are drawn in that box: two for a plus, one for
    /// the minus an open folder carries.
    fn bars_in(out: &Rendered, mark: Panel) -> usize {
        out.scene
            .rects
            .iter()
            .filter(|rect| {
                let [_, _, w, h] = rect.xywh();
                rect.extra()[3] == 0.0 && (w == 1.0 || h == 1.0) && inside(**rect, mark)
            })
            .count()
    }


    /// The picker with the two sessions the swap test uses already showing.
    fn a_session_picker() -> Picker {
        let mut picker = a_picker(&["gui"], &[]);
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        picker.show_sessions_at(
            crate::sessions::Listing {
                sessions: vec![
                    a_saved("live", Some("/home/hec"), "carry this on", 600),
                    a_saved("older", Some("/home/hec"), "the one before", 86_400),
                ],
                skipped: Vec::new(),
            },
            now,
        );
        picker
    }

    fn a_picker(inside: &[&str], recents: &[&str]) -> Picker {
        Picker::open(
            Box::new(crate::picker::Fixed(
                inside.iter().map(|s| s.to_string()).collect(),
            )),
            std::path::PathBuf::from("/home/hec"),
            recents.iter().map(std::path::PathBuf::from).collect(),
        )
    }

    /// With no folder chosen there is nothing to arrange panes around and
    /// nothing to type at, so the picker is the window: no spaces, no prompt,
    /// and it answers for every point below the title strip.
    #[test]
    fn the_window_opens_on_the_picker_instead_of_a_workspace() {
        let picker = a_picker(&["gui", "crates", "docs"], &["/home/hec/workspace/noob-cli"]);
        let out = render_picker(&picker, 1205.0, 791.0, None);
        let layout = &out.layout;
        assert!(layout.picking);
        for space in Space::ALL {
            assert!(layout.placed(space).tabs.is_empty(), "{space:?}");
            assert_eq!(layout.placed(space).body.w, 0.0, "{space:?}");
            assert_eq!(layout.placed(space).arrow_left.w, 0.0, "{space:?}");
            assert_eq!(layout.placed(space).arrow_right.w, 0.0, "{space:?}");
        }
        assert_eq!(layout.input.w, 0.0, "there is nothing to type at yet");
        assert_eq!(layout.cell(600.0, 400.0, 13.0, 8.0), None);

        // Inside the surface, under the title strip, and centred.
        let box_ = layout.picker;
        assert!(box_.y >= TITLE_H, "it starts below the strip: {box_:?}");
        assert!(box_.y + box_.h <= 791.0 && box_.x + box_.w <= 1205.0, "{box_:?}");
        let left = box_.x;
        let right = 1205.0 - (box_.x + box_.w);
        assert!((left - right).abs() <= 1.0, "off centre: {left} then {right}");

        // Every row of the list is hit where it is drawn, and the button and the
        // margin answer for themselves.
        assert_eq!(layout.picker_rows.len(), picker.rows().len());
        for (index, row) in &layout.picker_rows {
            let (x, y) = middle(*row);
            assert_eq!(layout.hit(x, y), Some(Hit::PickerRow(*index)));
            assert!(layout.picker_list.contains(x, y), "row {index} is outside the list");
        }
        let (x, y) = middle(layout.picker_open);
        assert_eq!(layout.hit(x, y), Some(Hit::PickerOpen));
        assert_eq!(
            layout.hit(box_.x + box_.w - 2.0, box_.y + 2.0),
            Some(Hit::Picker),
            "its own margin swallows a press rather than passing it on"
        );
        assert_eq!(layout.hit(2.0, 400.0), None, "and outside it there is nothing");
        // The strip is still the strip: the window can be moved and closed
        // before a folder is chosen.
        assert_eq!(layout.hit(400.0, 8.0), Some(Hit::TitleBar));
        assert_eq!(layout.hit(middle(layout.close).0, middle(layout.close).1), Some(Hit::Close));

        // What it says: the heading, the folder being listed, the remembered
        // folder, the names inside, and the button.
        let text = text_of(&out.scene);
        for wanted in [
            PICKER_TITLE,
            "/home/hec",
            "/home/hec/workspace/noob-cli",
            "gui",
            "crates",
            "..",
            PICKER_OPEN_LABEL,
            // Both ends of the line of keys: it is clipped to the width of the
            // box, so one key too many silently costs the last one, and the
            // last one is how the window is closed.
            "enter opens",
            "esc quits",
        ] {
            assert!(text.contains(wanted), "{wanted:?} is not on screen: {text}");
        }

        // The row the cursor is on is a filled accent band with the dark ink
        // written over it. Item 4: the quiet band the file explorer marks its
        // open row with said almost nothing here.
        let (index, cursor_row) = layout.picker_rows[0];
        assert_eq!(index, picker.cursor());
        assert!(
            covered(&out, cursor_row, cursor_row.h, out.skin.picked),
            "the cursor's row has no band"
        );
        // And no other row is banded, or every row would read as the one. Only
        // a full width fill counts: `skin.mark_edge` is the same accent and the
        // hairline box in front of every folder is not a band, and neither is
        // an outline stroked in the focus colour.
        let banded = out
            .scene
            .rects
            .iter()
            .filter(|rect| {
                rect.extra()[3] == 0.0
                    && rect.rgba() == out.skin.picked
                    && rect.xywh()[2] >= cursor_row.w - 0.01
            })
            .count();
        assert_eq!(banded, 1, "more than one row is banded");
        // Everything written on that band is the dark ink. Accent text on the
        // accent band is the one thing the whole palette is built to avoid.
        let ink: Vec<Option<[u8; 4]>> = out
            .scene
            .texts
            .iter()
            .filter(|text| (text.at.y - cursor_row.y).abs() < 0.01)
            .flat_map(|text| text.runs.iter().map(|run| run.color))
            .collect();
        assert!(!ink.is_empty(), "the row on the band says nothing");
        for tint in ink {
            assert_eq!(tint, Some(out.skin.picked_ink), "not the dark ink");
        }

        // Nothing hangs off the surface.
        for rect in &out.scene.rects {
            let [x, y, w, h] = rect.xywh();
            assert!(
                x >= -0.01 && y >= -0.01 && x + w <= 1205.01 && y + h <= 791.01,
                "{:?} is outside the window",
                rect.xywh()
            );
        }
    }

    /// Item E1: the button that opens the row the cursor is on sits at the right
    /// limit of the picker's head, says Open selected, carries the cut corner
    /// every panel in this window carries, sits on a surface of its own and
    /// lights up under the pointer.
    #[test]
    fn the_open_button_sits_at_the_right_limit_and_reads_as_a_button() {
        let picker = a_picker(&["gui"], &["/home/hec/workspace/noob-cli"]);
        let cold = render_picker(&picker, 1205.0, 791.0, None);
        let warm = render_picker(&picker, 1205.0, 791.0, Some(Hit::PickerOpen));
        let button = cold.layout.picker_open;

        // Its own surface idle, a stronger one hot, and neither of them the tab
        // fill it used to borrow.
        assert!(covered(&cold, button, button.h, cold.skin.button));
        assert!(covered(&warm, button, button.h, warm.skin.button_hot));
        assert!(!covered(&cold, button, button.h, cold.skin.tab_idle));
        assert!(cold.skin.button_hot[3] > cold.skin.button[3]);

        // The same 45 degree cut on the same corner as every panel, on the fill
        // and on the edge, or the fill pokes a square corner out of a cut one.
        let shaped: Vec<Rect> = cold
            .scene
            .rects
            .iter()
            .filter(|rect| {
                let [x, y, w, h] = rect.xywh();
                (x - button.x).abs() < 0.01
                    && (y - button.y).abs() < 0.01
                    && (w - button.w).abs() < 0.01
                    && (h - button.h).abs() < 0.01
            })
            .copied()
            .collect();
        assert_eq!(shaped.len(), 2, "a fill and an edge, and nothing else");
        for rect in &shaped {
            let [_, chamfer, corners, _] = rect.extra();
            assert_eq!(chamfer, CUT, "the button has no corner cut");
            assert_eq!(corners as u32, Rect::TOP_RIGHT);
        }
        assert!(
            shaped.iter().any(|rect| rect.extra()[3] > 0.0),
            "one of the two is the outline"
        );

        // At the right limit of the box's content, in its head, not at the foot
        // where it used to be beside the button that swapped the list.
        let box_ = cold.layout.picker;
        assert!(
            (button.x + button.w - (box_.x + box_.w - PAD)).abs() < 0.01,
            "{button:?} is not at the right limit of {box_:?}"
        );
        assert!(
            button.y < cold.layout.picker_filter.y,
            "{button:?} is not in the head"
        );

        // It says "Open selected" and nothing else. The folder it would open is
        // written above the list, and spelling it out here made the button as
        // wide as a path and a different width every time the cursor moved.
        let inside: String = warm
            .scene
            .texts
            .iter()
            .filter(|text| {
                text.at.x >= button.x - 0.01
                    && text.at.x + text.at.w <= button.x + button.w + 0.01
                    && text.at.y >= button.y - 0.01
                    && text.at.y + text.at.h <= button.y + button.h + 0.01
            })
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect();
        assert!(inside.contains(PICKER_OPEN_LABEL), "the button says {inside:?}");
        assert!(
            !inside.contains("/home/hec"),
            "the button still names a folder: {inside:?}"
        );

        // Taller than its text, which is what stops it reading as a line with a
        // box round it, and the hit region is the rectangle that was drawn.
        assert!(button.h > Text::line_for(13.0));
        let (x, y) = middle(button);
        assert_eq!(cold.layout.hit(x, y), Some(Hit::PickerOpen));
        assert_eq!(
            cold.layout.hit(button.x + button.w + 4.0, y),
            Some(Hit::Picker),
            "the gap to the button beside it is the box's own margin, not either button"
        );
    }

    /// Item E1: the picker's head is one title and three buttons. Folders and
    /// Sessions at the left choose which list is showing, and the one whose list
    /// is in front of you is filled in the band the chosen row wears, because two
    /// buttons drawn the same way say nothing about where you are.
    #[test]
    fn the_head_s_two_buttons_choose_the_list_and_say_which_one_is_showing() {
        let mut picker = a_picker(&["gui"], &[]);
        let cold = render_picker(&picker, 1205.0, 791.0, None);
        let open = cold.layout.picker_open;
        let (folders, sessions) = (cold.layout.picker_folders, cold.layout.picker_sessions);

        // Both there, the same size, side by side at the left of the head, on
        // the same row as Open and clear of it.
        assert!(folders.w > 1.0 && folders.h > 1.0, "there is no Folders button");
        assert!(sessions.w > 1.0 && sessions.h > 1.0, "there is no Sessions button");
        assert!(
            (folders.w - sessions.w).abs() < 0.01 && (folders.h - sessions.h).abs() < 0.01,
            "the pair is not one size: {folders:?} then {sessions:?}"
        );
        assert!((folders.y - open.y).abs() < 0.01 && (sessions.y - open.y).abs() < 0.01);
        assert!((folders.x - (cold.layout.picker.x + PAD)).abs() < 0.01, "{folders:?}");
        assert!(sessions.x >= folders.x + folders.w, "{folders:?} then {sessions:?}");
        assert!(open.x >= sessions.x + sessions.w, "{sessions:?} then {open:?}");

        // Each is its own target, and none of the three answers for another.
        let (fx, fy) = middle(folders);
        let (x, y) = middle(sessions);
        assert_eq!(cold.layout.hit(fx, fy), Some(Hit::PickerFolders));
        assert_eq!(cold.layout.hit(x, y), Some(Hit::PickerSessions));
        assert_eq!(
            cold.layout.hit(middle(open).0, middle(open).1),
            Some(Hit::PickerOpen)
        );

        // The folders are showing, so Folders wears the band and Sessions is a
        // plain button. The two fills are not the same colour, or the state
        // would be a state nobody can see.
        assert_ne!(cold.skin.picked, cold.skin.button);
        assert!(
            covered(&cold, folders, folders.h, cold.skin.picked),
            "the showing mode has no band"
        );
        assert!(
            covered(&cold, sessions, sessions.h, cold.skin.button),
            "the mode that is not showing wears the band"
        );
        // The band is a fill: the focus outline every one of these buttons
        // wears is the same accent, and an outline is not a band.
        assert!(!cold.scene.rects.iter().any(|rect| {
            let [x, y, w, h] = rect.xywh();
            rect.extra()[3] == 0.0
                && rect.rgba() == cold.skin.picked
                && (x - sessions.x).abs() < 0.01
                && (y - sessions.y).abs() < 0.01
                && (w - sessions.w).abs() < 0.01
                && (h - sessions.h).abs() < 0.01
        }));
        // And it is written in the ink that reads on that band.
        let ink: Vec<Option<[u8; 4]>> = cold
            .scene
            .texts
            .iter()
            .filter(|text| {
                text.at.x >= folders.x
                    && text.at.x < folders.x + folders.w
                    && text.at.y >= folders.y
                    && text.at.y < folders.y + folders.h
            })
            .flat_map(|text| text.runs.iter().map(|run| run.color))
            .collect();
        assert!(!ink.is_empty(), "the showing mode says nothing");
        for tint in ink {
            assert_eq!(tint, Some(cold.skin.picked_ink), "not the dark ink");
        }

        // The pointer lights the mode that is not showing, and nothing else.
        let warm = render_picker(&picker, 1205.0, 791.0, Some(Hit::PickerSessions));
        assert!(covered(&warm, sessions, sessions.h, warm.skin.button_hot));
        assert!(
            covered(&warm, open, open.h, warm.skin.button),
            "the pointer on one button must not light the other"
        );
        assert!(
            covered(&warm, folders, folders.h, warm.skin.picked),
            "the showing mode changed under a pointer that is not on it"
        );

        // One title, and both words are on screen at once: the head says what
        // the box is for, the pair says which list is in it.
        let text = text_of(&cold.scene);
        assert!(text.contains(PICKER_TITLE), "{text}");
        assert!(text.contains(PICKER_FOLDERS_LABEL), "{text}");
        assert!(text.contains(PICKER_SESSIONS_LABEL), "{text}");

        // Pressed, the same box lists the sessions instead: same rectangle, same
        // buttons in the same places, same title, and the band has moved.
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        let mut gone = a_saved("old", Some("/home/hec/deleted"), "the one before", 86_400);
        gone.gone = true;
        picker.show_sessions_at(
            crate::sessions::Listing {
                sessions: vec![
                    a_saved("live", Some("/home/hec"), "carry this on", 600),
                    gone,
                ],
                skipped: Vec::new(),
            },
            now,
        );
        let after = render_picker(&picker, 1205.0, 791.0, None);
        assert_eq!(
            (
                after.layout.picker,
                after.layout.picker_open,
                after.layout.picker_folders,
                after.layout.picker_sessions,
                after.layout.picker_filter,
            ),
            (cold.layout.picker, open, folders, sessions, cold.layout.picker_filter),
            "swapping the list moved the box"
        );
        assert_eq!(after.layout.picker_rows.len(), 2);
        assert!(
            covered(&after, sessions, sessions.h, after.skin.picked),
            "the sessions are showing and their button has no band"
        );
        assert!(
            covered(&after, folders, folders.h, after.skin.button),
            "the folder button kept the band after the list swapped"
        );
        let text = text_of(&after.scene);
        for wanted in [
            PICKER_TITLE,
            "2 saved sessions",
            "10m ago",
            "carry this on",
            "deleted (gone)",
            // Still written above the list, because it is the folder a session
            // that never noted one would be resumed in.
            "/home/hec",
            PICKER_FOLDERS_LABEL,
            PICKER_SESSIONS_LABEL,
        ] {
            assert!(text.contains(wanted), "{wanted:?} is not on screen: {text}");
        }
        // The title is the one string in both lists: neither of the two it
        // replaced is anywhere in the window.
        for gone in ["OPEN A FOLDER", "OPEN A SESSION"] {
            assert!(!text.contains(gone), "{gone:?} is still drawn");
            assert!(!text_of(&cold.scene).contains(gone), "{gone:?} is still drawn");
        }

        // The row that cannot be opened is written in the colour every other
        // thing that cannot be opened is written in.
        let (_, dead) = after.layout.picker_rows[1];
        let tints: Vec<[u8; 4]> = after
            .scene
            .texts
            .iter()
            .filter(|text| (text.at.y - dead.y).abs() < 0.01)
            .flat_map(|text| text.runs.iter().filter_map(|run| run.color))
            .collect();
        assert!(!tints.is_empty());
        assert!(
            tints.iter().all(|tint| *tint == after.skin.bad),
            "a session whose folder has gone reads like any other row"
        );

        // And all three go with the picker. A button left behind by a shape
        // change is a press that lands on something nobody can see.
        let dock = Dock::new();
        let panel = a_settings_panel(&Config::default());
        for (what, shape) in [
            ("shaded", Shape { shaded: true, ..shape(&dock, &[]) }),
            ("settings", Shape { settings: Some(&panel), ..shape(&dock, &[]) }),
        ] {
            let layout = Layout::compute(1205.0, 791.0, &shape);
            assert_eq!(layout.picker_folders.w, 0.0, "{what}");
            assert_eq!(layout.picker_sessions.w, 0.0, "{what}");
            assert_eq!(layout.picker_open.w, 0.0, "{what}");
            assert_ne!(layout.hit(fx, fy), Some(Hit::PickerFolders), "{what}");
            assert_ne!(layout.hit(x, y), Some(Hit::PickerSessions), "{what}");
        }
    }

    /// One saved session, as the reader would have described it.
    fn a_saved(
        id: &str,
        at: Option<&str>,
        said: &str,
        ago: u64,
    ) -> crate::sessions::Saved {
        crate::sessions::Saved {
            id: String::from(id),
            when: std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(1_000_000_000 - ago),
            workspace: at.map(std::path::PathBuf::from),
            gone: false,
            bytes: 12_000,
            context: None,
            opening: String::from(said),
        }
    }

    /// Item 3: the box does not change shape under the pointer. Walking from a
    /// folder with two entries into one with sixty used to resize and recentre
    /// the whole dialog, because its height came from the number of rows it was
    /// holding.
    #[test]
    fn the_picker_s_box_is_one_shape_whatever_the_folder_holds() {
        let short = a_picker(&["one", "two"], &[]);
        let long_names: Vec<String> = (0..60).map(|n| format!("dir{n:02}")).collect();
        let long = a_picker(
            &long_names.iter().map(String::as_str).collect::<Vec<&str>>(),
            &[],
        );
        for (w, h) in [(1205.0, 791.0), (2200.0, 1400.0), (680.0, 380.0)] {
            let a = render_picker(&short, w, h, None).layout;
            let b = render_picker(&long, w, h, None).layout;
            assert_eq!(
                (a.picker.x, a.picker.y, a.picker.w, a.picker.h),
                (b.picker.x, b.picker.y, b.picker.w, b.picker.h),
                "the box moved between two folders at {w}x{h}"
            );
            assert_eq!(
                (a.picker_list.y, a.picker_list.h),
                (b.picker_list.y, b.picker_list.h),
                "the list moved at {w}x{h}"
            );
            assert_eq!(
                (a.picker_open.x, a.picker_open.y, a.picker_open.w, a.picker_open.h),
                (b.picker_open.x, b.picker_open.y, b.picker_open.w, b.picker_open.h),
                "the button moved at {w}x{h}"
            );
            // The short folder simply leaves the bottom of its list empty, which
            // is the price of a dialog that stays put.
            assert_eq!(
                a.picker_rows.len(),
                4,
                "this folder, the way out, and the two folders in it"
            );
            assert_eq!(b.picker_rows.len(), a.picker_capacity(13.0).min(62));
            let (x, y) = middle(a.picker_open);
            assert_eq!(a.hit(x, y), Some(Hit::PickerOpen), "at {w}x{h}");
            assert_eq!(b.hit(x, y), Some(Hit::PickerOpen), "at {w}x{h}");
        }

        // And walking really does keep it still: the same picker, before and
        // after it lists a folder with a very different number of entries.
        let mut walking = Picker::open(
            Box::new(crate::picker::Fixed(
                long_names.iter().map(|s| s.to_string()).collect(),
            )),
            std::path::PathBuf::from("/home/hec"),
            Vec::new(),
        );
        let before = render_picker(&walking, 1205.0, 791.0, None).layout.picker;
        assert!(walking.step(true) && walking.walk_in());
        let after = render_picker(&walking, 1205.0, 791.0, None).layout.picker;
        assert_eq!(
            (before.x, before.y, before.w, before.h),
            (after.x, after.y, after.w, after.h)
        );
    }

    /// Item 5: typing dims the rows it did not match instead of taking them
    /// away, and the cursor only lands where the model says a match is.
    #[test]
    fn typing_in_the_picker_dims_rows_rather_than_dropping_them() {
        let mut picker = a_picker(&["gui", "crates", "docs"], &[]);
        let before = render_picker(&picker, 1205.0, 791.0, None);
        let rows = before.layout.picker_rows.len();
        assert!(picker.type_text("cra"));
        let after = render_picker(&picker, 1205.0, 791.0, None);
        assert_eq!(
            after.layout.picker_rows.len(),
            rows,
            "typing took rows out of the list"
        );

        // Every name is still on screen; the ones that did not match are drawn
        // in the dim tint rather than the body one.
        let text = text_of(&after.scene);
        for name in ["gui", "crates", "docs"] {
            assert!(text.contains(name), "{name:?} left the list: {text}");
        }
        let tint_of = |out: &Rendered, name: &str| -> Vec<Option<[u8; 4]>> {
            out.scene
                .texts
                .iter()
                .flat_map(|text| text.runs.iter())
                .filter(|run| run.text.trim() == name)
                .map(|run| run.color)
                .collect()
        };
        assert_eq!(tint_of(&after, "gui"), vec![Some(after.skin.dim)]);
        assert_eq!(tint_of(&after, "docs"), vec![Some(after.skin.dim)]);
        assert_eq!(tint_of(&before, "gui"), vec![Some(before.skin.body)]);
        // The match is where the cursor went, so it is the row on the band, and
        // what is written on a green band is the dark ink.
        assert_eq!(said(&picker, picker.cursor()), "crates");
        assert_eq!(tint_of(&after, "crates"), vec![Some(after.skin.picked_ink)]);

        // One rule: the arrows walk the matches, and a click still lands on a
        // dim row, so what the pointer can reach is a superset of what the
        // arrows stop on.
        let dim = after
            .layout
            .picker_rows
            .iter()
            .find(|(index, _)| said(&picker, *index) == "gui")
            .copied()
            .expect("the dim row is still placed");
        let (x, y) = middle(dim.1);
        assert_eq!(after.layout.hit(x, y), Some(Hit::PickerRow(dim.0)));
        assert!(picker.point_at(dim.0), "a click on a dim row selects it");
        assert_eq!(
            picker.confirm(),
            Some(crate::picker::Chosen::folder(std::path::PathBuf::from(
                "/home/hec/gui"
            )))
        );
    }

    /// Item 4: the mark in front of a folder is a region of its own inside the
    /// row, pressing it opens the folder where it stands, and what comes out is
    /// drawn one step further in than the folder it came from.
    #[test]
    fn the_mark_in_front_of_a_folder_is_its_own_target() {
        let mut picker = a_picker(&["gui", "crates"], &["/home/hec/workspace"]);
        let out = render_picker(&picker, 1205.0, 791.0, None);

        // A mark only where there is a folder to open. The remembered folder,
        // the folder being listed and the way out of it are how the list is
        // walked rather than branches of the tree.
        let marked: Vec<String> = out
            .layout
            .picker_marks
            .iter()
            .map(|(index, _)| said(&picker, *index))
            .collect();
        assert_eq!(marked, ["crates", "gui"]);

        // Each one sits inside its own row, and the row still answers for the
        // rest of itself: the press that opens a folder and the press that
        // selects it are different presses.
        for (index, mark) in &out.layout.picker_marks {
            let row = out
                .layout
                .picker_rows
                .iter()
                .find(|(at, _)| at == index)
                .map(|(_, row)| *row)
                .expect("a mark with no row under it");
            assert!(mark.w > 1.0 && (mark.h - row.h).abs() < 0.01, "{mark:?}");
            assert!(
                mark.x >= row.x && mark.x + mark.w <= row.x + row.w,
                "the mark is outside its row: {mark:?} in {row:?}"
            );
            let (x, y) = middle(*mark);
            assert_eq!(out.layout.hit(x, y), Some(Hit::PickerMark(*index)));
            assert_eq!(
                out.layout.hit(mark.x + mark.w + 2.0, y),
                Some(Hit::PickerRow(*index)),
                "the row beside the mark stopped answering"
            );
        }
        // It lights up under the pointer, so it reads as something to press.
        // The colour is the mark's own green either way and what changes is the
        // weight of the box: the old glyph swapped tint instead, which it had to,
        // because a glyph has no border to thicken.
        let (index, mark) = out.layout.picker_marks[0];
        let warm = render_picker(&picker, 1205.0, 791.0, Some(Hit::PickerMark(index)));
        assert_eq!(outline_of(&out, mark).map(|rect| rect.extra()[3]), Some(1.0));
        assert_eq!(
            outline_of(&warm, mark).map(|rect| rect.extra()[3]),
            Some(2.0),
            "the mark does not thicken under the pointer"
        );
        for at in [&out, &warm] {
            assert_eq!(
                outline_of(at, mark).map(|rect| rect.rgba()),
                Some(at.skin.mark_edge)
            );
        }

        // Pressing it puts what is inside the folder in the list under it, at a
        // deeper indent, and the mark turns over.
        assert!(picker.toggle(index));
        let after = render_picker(&picker, 1205.0, 791.0, None);
        let deeper = after
            .layout
            .picker_marks
            .iter()
            .find(|(at, _)| picker.row(*at).map(PickerRow::depth) == Some(1))
            .copied()
            .expect("nothing came out of the folder");
        assert!(
            deeper.1.x > mark.x,
            "a child is not drawn further in than its parent: {:?} then {:?}",
            mark,
            deeper.1
        );
        // A shut folder carries a plus, an open one carries the same box with
        // the upright taken out of it, and neither is a glyph: nothing is drawn
        // as text inside a mark any more.
        assert_eq!(bars_in(&out, mark), 2, "a shut folder is not a plus");
        let (_, reopened) = after
            .layout
            .picker_marks
            .iter()
            .find(|(at, _)| *at == index)
            .copied()
            .expect("the folder that was opened lost its mark");
        assert_eq!(bars_in(&after, reopened), 1, "an open folder is not a minus");
        for (at, mark) in [(&out, mark), (&after, reopened)] {
            assert!(
                !at.scene.texts.iter().any(|text| {
                    (text.at.x - mark.x).abs() < 0.01 && (text.at.y - mark.y).abs() < 0.01
                }),
                "the mark is still drawn as a glyph"
            );
        }

        // And the name beside it is still drawn inside the box, however deep it
        // sits: the indent stops before it pushes a row off the right.
        for (index, row) in &after.layout.picker_rows {
            let said = said(&picker, *index);
            assert!(
                after
                    .scene
                    .texts
                    .iter()
                    .any(|text| text.runs.iter().any(|run| run.text.trim() == said)),
                "{said:?} is not drawn"
            );
            assert!(row.x + row.w <= after.layout.picker_list.x + after.layout.picker_list.w + 0.01);
        }
    }

    /// Item A6: the mark in front of a folder is a small unfilled green box with
    /// a green plus in it. It used to be Font Awesome's filled plus-square drawn
    /// at the row's own text size, which is a solid block at the front of every
    /// folder in the list.
    #[test]
    fn the_folder_mark_is_a_small_unfilled_green_box() {
        let picker = a_picker(&["gui", "crates"], &[]);
        let out = render_picker(&picker, 1205.0, 791.0, None);
        // Not the row the cursor is on, whose mark is drawn in the ink that
        // reads on the green band rather than in the green itself.
        let (index, mark) = *out
            .layout
            .picker_marks
            .iter()
            .find(|(index, _)| *index != picker.cursor())
            .expect("no mark off the cursor's row");

        let box_ = outline_of(&out, mark).expect("the mark has no box round it");
        let [x, y, w, h] = box_.xywh();
        // Square, odd sided so the plus has a middle to sit on, and well under
        // the region it is drawn in: smaller is the whole point.
        assert_eq!(w, h, "the mark is not square");
        assert_eq!(w as i32 % 2, 1, "an even side puts the plus off centre");
        assert!(
            w <= mark.w.min(mark.h) * 0.7,
            "{w} is not smaller than the {:?} it is drawn in",
            (mark.w, mark.h)
        );
        assert!(
            (x + w * 0.5 - (mark.x + mark.w * 0.5)).abs() <= 1.0
                && (y + h * 0.5 - (mark.y + mark.h * 0.5)).abs() <= 1.0,
            "the mark is not centred in its region"
        );
        // A border and nothing behind it: an outline is a stroke, and a filled
        // rectangle of this colour anywhere in the region would be the fill the
        // glyph used to be.
        assert_eq!(box_.extra()[3], 1.0, "the box is not a hairline");
        assert_eq!(box_.rgba(), out.skin.mark_edge, "the box is not the accent");
        assert_eq!(
            out.skin.mark_edge,
            out.skin.picked,
            "the folder mark is not the colour the window picks with"
        );
        let filled = out
            .scene
            .rects
            .iter()
            .filter(|rect| {
                let [_, _, w, h] = rect.xywh();
                rect.extra()[3] == 0.0 && w > 1.0 && h > 1.0 && inside(**rect, mark)
            })
            .count();
        assert_eq!(filled, 0, "something inside the mark is filled");

        // The plus is two bars, both green, both one pixel, and both clear of
        // the border round them.
        let bars: Vec<Rect> = out
            .scene
            .rects
            .iter()
            .filter(|rect| {
                let [_, _, w, h] = rect.xywh();
                rect.extra()[3] == 0.0 && (w == 1.0 || h == 1.0) && inside(**rect, mark)
            })
            .copied()
            .collect();
        assert_eq!(bars.len(), 2, "a shut folder does not carry a plus");
        for bar in &bars {
            assert_eq!(bar.rgba(), out.skin.mark_edge);
            let [bx, by, bw, bh] = bar.xywh();
            assert!(bx > x && by > y && bx + bw < x + w && by + bh < y + h, "{bar:?} touches the box");
        }
        assert!(
            bars.iter().any(|bar| bar.xywh()[2] > 1.0) && bars.iter().any(|bar| bar.xywh()[3] > 1.0),
            "the two bars do not cross"
        );

        // On the row the cursor is on the same box is drawn in the ink that
        // reads on the band, because the band there is already this green.
        let mut picker = picker;
        assert!(picker.point_at(index), "the cursor will not go on a folder");
        let banded = render_picker(&picker, 1205.0, 791.0, None);
        let (_, on_band) = *banded
            .layout
            .picker_marks
            .iter()
            .find(|(at, _)| *at == index)
            .expect("the row the cursor moved to lost its mark");
        assert_eq!(
            outline_of(&banded, on_band).map(|rect| rect.rgba()),
            Some(banded.skin.mark_on_band),
            "the mark is green on a green band"
        );
        assert_eq!(bars_in(&banded, on_band), 2, "and it is still a plus");
    }

    /// Item A6: what is typed to narrow the list sits in a field, with the
    /// magnifier that says type here and the cut corner every other box in this
    /// window carries. It was a line of writing with a funnel in front of it.
    #[test]
    fn the_picker_s_filter_is_a_bordered_field_with_a_search_icon() {
        let mut picker = a_picker(&["gui", "crates"], &[]);
        let out = render_picker(&picker, 1205.0, 791.0, None);
        let field = out.layout.picker_filter;

        // Under the two lines of writing, above the list, the full width of the
        // box's content, and taller than the line in it.
        let line = Text::line_for(13.0);
        assert!(field.w > 1.0 && field.h > line, "{field:?} is not a field");
        assert!(
            field.y > out.layout.picker.y && field.y + field.h <= out.layout.picker_list.y + 0.01,
            "{field:?} is not between the heading and the list"
        );

        // A surface, a hairline round it, and both take the window's cut corner.
        let shaped: Vec<Rect> = out
            .scene
            .rects
            .iter()
            .filter(|rect| {
                let [x, y, w, h] = rect.xywh();
                (x - field.x).abs() < 0.01
                    && (y - field.y).abs() < 0.01
                    && (w - field.w).abs() < 0.01
                    && (h - field.h).abs() < 0.01
            })
            .copied()
            .collect();
        assert_eq!(shaped.len(), 2, "the field is not a fill and an edge");
        for rect in &shaped {
            assert_eq!(rect.extra()[1], CUT, "the field has no cut corner");
            assert_eq!(rect.extra()[2], Rect::TOP_RIGHT as f32);
        }
        assert!(shaped.iter().any(|rect| rect.rgba() == out.skin.input));
        assert!(
            shaped
                .iter()
                .any(|rect| rect.rgba() == out.skin.edge_focus && rect.extra()[3] == 1.0)
        );

        // The magnifier is inside the field, and the funnel that was there is
        // gone from the window.
        let runs: Vec<&str> = out
            .scene
            .texts
            .iter()
            .filter(|text| {
                text.at.x >= field.x
                    && text.at.y >= field.y
                    && text.at.y < field.y + field.h
            })
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect();
        assert!(
            runs.contains(&icons::SEARCH.to_string().as_str()),
            "the search icon is not in the field: {runs:?}"
        );
        let every: String = out
            .scene
            .texts
            .iter()
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect();
        assert!(!every.contains('\u{eaf1}'), "the funnel is still drawn");
        assert!(every.contains("type to narrow the list"));

        // And what is typed goes in the same field.
        assert!(picker.type_text("cra"));
        let typed = render_picker(&picker, 1205.0, 791.0, None);
        assert_eq!(typed.layout.picker_filter, field, "the field moved");
        let said: String = typed
            .scene
            .texts
            .iter()
            .filter(|text| (text.at.y - field.y - PICKER_FIELD_PAD).abs() < 0.01)
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect();
        assert!(said.contains("cra"), "what was typed is not in the field: {said:?}");
    }

    /// The field keeps to the room above the list, whatever the window does.
    ///
    /// Its height was its own to choose while everything under it was measured
    /// from the head the box could not give it, so in a window short enough
    /// that the head has no room the field was drawn at full height out of the
    /// bottom of the box and over the Open button. It takes the room that is
    /// there now, down to none of it, which is a picker with no field rather
    /// than a field over the list.
    #[test]
    fn the_picker_s_field_stays_out_of_its_list_in_a_short_window() {
        for height in [100.0f32, 120.0, 160.0, 200.0, 240.0, 300.0, 420.0, 791.0] {
            // Both lists: the sessions keep a row above themselves for the
            // header, so the room over the list is not the same room.
            for sessions in [false, true] {
                let picker = match sessions {
                    true => a_session_picker(),
                    false => a_picker(&["gui", "crates", "docs"], &[]),
                };
                let out = render_picker(&picker, 900.0, height, None);
                let (box_, field, list, open) = (
                    out.layout.picker,
                    out.layout.picker_filter,
                    out.layout.picker_list,
                    out.layout.picker_open,
                );
                let what = format!("{height} tall, sessions {sessions}");
                assert!(
                    list.h < 1.0 || field.y + field.h <= list.y + 0.01,
                    "{what}: the field {field:?} runs into the list {list:?}"
                );
                assert!(
                    field.y + field.h <= box_.y + box_.h + 0.01,
                    "{what}: the field {field:?} runs out of the box {box_:?}"
                );
                assert!(
                    field.h < 1.0 || field.y >= open.y + open.h - 0.01,
                    "{what}: the field {field:?} is over the Open button {open:?}"
                );
                // And nothing is drawn as a field where there is no room for
                // one: the rows of the list own that space.
                if field.h < 1.0 {
                    assert!(
                        !out.scene.rects.iter().any(|rect| {
                            let [x, y, w, _] = rect.xywh();
                            (x - field.x).abs() < 0.01
                                && (y - field.y).abs() < 0.01
                                && (w - field.w).abs() < 0.01
                        }),
                        "{what}: a field with no room is still drawn"
                    );
                }
            }
        }
        // With room to spare it is the field it always was.
        let out = render_picker(&a_picker(&["gui"], &[]), 900.0, 791.0, None);
        let field = out.layout.picker_filter;
        assert!(
            (field.h - picker_field_h(Text::line_for(13.0))).abs() < 0.01,
            "{field:?} is not the height a field asks for"
        );
    }

    /// Item E1: Open selected is one route for both lists. On the folders it
    /// opens the folder the cursor is on, on the sessions it carries the session
    /// the cursor is on, and it is the same button in the same place either way.
    ///
    /// The picker used to have four affordances for these two acts: an Open
    /// button and a Folders/Sessions swap at the foot, and an arrow back to the
    /// folders in the heading. The arrow and the foot swap are gone.
    #[test]
    fn open_selected_opens_a_folder_on_one_list_and_a_session_on_the_other() {
        // The folder list, cursor moved onto the folder inside it.
        let mut folders = a_picker(&["gui"], &[]);
        let on_folders = render_picker(&folders, 1205.0, 791.0, None);
        let button = on_folders.layout.picker_open;
        let (x, y) = middle(button);
        assert_eq!(on_folders.layout.hit(x, y), Some(Hit::PickerOpen));
        // Past this folder and past the way out of it, onto the one inside.
        assert!(folders.step(true) && folders.step(true));
        let chosen = folders.confirm().expect("Open selected chose nothing");
        assert_eq!(chosen.workspace, std::path::PathBuf::from("/home/hec/gui"));
        assert_eq!(chosen.session, None, "a folder is a fresh session");

        // The session list, in the same window: the same button, in the same
        // place, and it answers for the same point.
        let mut sessions = a_session_picker();
        let on_sessions = render_picker(&sessions, 1205.0, 791.0, None);
        assert_eq!(on_sessions.layout.picker_open, button, "the button moved");
        assert_eq!(on_sessions.layout.hit(x, y), Some(Hit::PickerOpen));
        let chosen = sessions.confirm().expect("Open selected chose nothing");
        assert_eq!(chosen.workspace, std::path::PathBuf::from("/home/hec"));
        assert_eq!(
            chosen.session.as_deref(),
            Some("live"),
            "the session under the cursor is not the one that was opened"
        );

        // Nothing that was retired is still drawn or still answers: no arrow in
        // the heading, and no second button at the foot of the box.
        let box_ = on_sessions.layout.picker;
        let foot = Panel::new(box_.x, box_.y + box_.h - picker_open_h(Text::line_for(13.0)), box_.w, picker_open_h(Text::line_for(13.0)));
        for out in [&on_folders, &on_sessions] {
            assert!(
                !text_of(&out.scene).contains('\u{ea9b}'),
                "the back arrow is still drawn"
            );
            assert!(
                !out.scene.rects.iter().any(|rect| {
                    let [rx, ry, _, _] = rect.xywh();
                    rect.rgba() == out.skin.button && foot.contains(rx + 1.0, ry + 1.0)
                }),
                "there is still a button at the foot of the box"
            );
        }
        // The one thing left down there is the line of keys, and Escape is still
        // on it.
        let keys = text_of(&on_sessions.scene);
        assert!(keys.contains("esc quits"), "{keys}");
    }

    /// Everything drawn at this line, left to right, as one string.
    fn line_at(out: &Rendered, y: f32) -> String {
        let mut texts: Vec<&noob_draw::Text> = out
            .scene
            .texts
            .iter()
            .filter(|text| (text.at.y - y).abs() < 0.01)
            .collect();
        texts.sort_by(|a, b| a.at.x.total_cmp(&b.at.x));
        texts
            .iter()
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect()
    }

    /// The picker rendered with a menu open over it.
    fn render_picker_menu(picker: &Picker, menu: &Menu, w: f32, h: f32) -> Rendered {
        let dock = Dock::new();
        let state = State::new();
        let mut shape = shape(&dock, &[]);
        shape.picker = Some(picker);
        shape.menu = Some(menu);
        let layout = Layout::compute(w, h, &shape);
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
            popup_scroll: 0,
            cursor: (-100.0, -100.0),
            selection: None,
            menu: Some(menu),
            picker: Some(picker),
            settings: None,
        });
        Rendered {
            scene,
            layout,
            skin,
        }
    }

    /// Item A7: the session list is a table. A row that read
    /// "10m ago  hec  carry this on" said four things with nothing anywhere
    /// naming any of them, so every cell now sits in a column of its own under a
    /// row that says what that column is.
    #[test]
    fn the_session_list_is_a_table_under_a_row_naming_its_columns() {
        let picker = a_session_picker();
        let out = render_picker(&picker, 1205.0, 791.0, None);
        let line = Text::line_for(13.0);
        let list = out.layout.picker_list;
        let (at, _) = session_table(list, 8.0);

        // The header sits on the line the layout kept above the list, which is
        // not one of the list's rows: it names the columns, it is not one of
        // them, and pressing it must not select anything.
        let header = line_at(&out, list.y - line);
        assert!(
            out.layout
                .picker_rows
                .iter()
                .all(|(_, row)| (row.y - (list.y - line)).abs() > 0.01),
            "the header took a row of the list"
        );
        assert_eq!(
            out.layout.hit(at + 4.0, list.y - line + 2.0),
            Some(Hit::Picker),
            "the header answers as the box, not as a row"
        );

        // Each column's name starts exactly where that column starts, and the
        // last one takes whatever is left.
        let mut offset = 0;
        for (name, wide) in SESSION_COLUMNS {
            let cell: String = header.chars().skip(offset).take(wide).collect();
            assert!(
                cell.starts_with(name),
                "{name:?} does not begin column {offset}: {header:?}"
            );
            offset += wide;
        }
        assert!(
            header.chars().skip(offset).collect::<String>().starts_with(SESSION_OPENING),
            "{header:?}"
        );

        // And every row writes its cells into those same columns, at the same x
        // the header is drawn at.
        for (index, row) in &out.layout.picker_rows {
            let cells = match picker.row(*index) {
                Some(PickerRow::Session(saved)) => picker.session_cells(saved),
                other => panic!("not a session: {other:?}"),
            };
            let (row_at, _) = session_table(*row, 8.0);
            assert!((row_at - at).abs() < 0.01, "row {index} starts elsewhere");
            let text: String = out
                .scene
                .texts
                .iter()
                .filter(|text| (text.at.y - row.y).abs() < 0.01 && (text.at.x - at).abs() < 0.01)
                .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
                .collect();
            let mut offset = 0;
            for (step, (_, wide)) in SESSION_COLUMNS.iter().enumerate() {
                let cell: String = text.chars().skip(offset).take(*wide).collect();
                assert!(
                    cell.starts_with(&cells[step]),
                    "row {index} column {step} says {cell:?}, not {:?}",
                    cells[step]
                );
                offset += wide;
            }
            assert!(
                text.chars().skip(offset).collect::<String>().starts_with(&cells[4]),
                "row {index} lost what was said in it: {text:?}"
            );
        }

        // The two columns that were nowhere before: how big the transcript is,
        // and how full its context window was. Nothing has ever measured these
        // sessions, so the reading is a dash rather than a number nobody took.
        let first = line_at(&out, out.layout.picker_rows[0].1.y);
        assert!(first.contains("12 kB"), "{first:?}");
        assert!(first.contains(" - "), "{first:?}");

        // The folder list has no header at all: it is one column of names, and
        // a word over it would explain the obvious.
        let folders = a_picker(&["gui", "crates"], &[]);
        assert!(!folders.on_sessions());
        let out = render_picker(&folders, 1205.0, 791.0, None);
        let text = text_of(&out.scene);
        for name in ["when", "context", SESSION_OPENING] {
            assert!(!text.contains(name), "{name:?} is over the folder list");
        }
    }

    /// A right click on a session row opens a menu over the picker, and the
    /// picker's own drawing used to stop before the overlay: the menu was placed
    /// and it answered presses, and nothing was on screen.
    #[test]
    fn a_menu_over_the_picker_is_drawn_over_the_picker() {
        let picker = a_session_picker();
        let row = {
            let out = render_picker(&picker, 1205.0, 791.0, None);
            out.layout.picker_rows[0].1
        };
        let menu = Menu::for_session(middle(row), 0, false);
        let out = render_picker_menu(&picker, &menu, 1205.0, 791.0);

        assert!(out.layout.menu.w >= 1.0, "the menu was not placed");
        assert!(!out.scene.over_rects.is_empty(), "the menu box is not drawn");
        let rows: String = out
            .scene
            .over_texts
            .iter()
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect();
        for label in [
            crate::menu::Item::OpenSession.label(),
            crate::menu::Item::DeleteSession(false).label(),
        ] {
            assert!(rows.contains(label), "{label:?} is not on screen: {rows:?}");
        }

        // And it takes the press before the row it covers, which it always did.
        let (x, y) = middle(out.layout.menu_rows[1].1);
        assert_eq!(out.layout.hit(x, y), Some(Hit::MenuRow(1)));
    }

    /// Pressed once, the Delete row reads "sure?" in the colour this window
    /// gives everything that throws work away, and the box under it does not
    /// move: the second press lands on the same pixels the first one did.
    ///
    /// The wording is the settings panel's, because the panel's delete asks the
    /// same question and the two are one product.
    #[test]
    fn an_armed_delete_row_reads_sure_without_moving_the_menu() {
        let picker = a_session_picker();
        let row = {
            let out = render_picker(&picker, 1205.0, 791.0, None);
            out.layout.picker_rows[0].1
        };
        let mut menu = Menu::for_session(middle(row), 0, false);
        let before = render_picker_menu(&picker, &menu, 1205.0, 791.0);
        let (x, y) = middle(before.layout.menu_rows[1].1);

        assert!(!menu.press_delete(1), "the first press was the delete");
        let out = render_picker_menu(&picker, &menu, 1205.0, 791.0);
        let armed: Vec<&Run> = out
            .scene
            .over_texts
            .iter()
            .flat_map(|text| text.runs.iter())
            .filter(|run| run.text.contains("sure?"))
            .collect();
        assert_eq!(armed.len(), 1, "the row does not ask: {armed:?}");
        assert_eq!(armed[0].color, Some(out.skin.bad));
        let rows: String = out
            .scene
            .over_texts
            .iter()
            .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
            .collect();
        assert!(
            !rows.contains(crate::menu::Item::DeleteSession(false).label()),
            "both wordings are on screen: {rows:?}"
        );

        // The same box, the same rows, and the same press: a menu that narrowed
        // when it armed would slide out from under the pointer and cancel the
        // press it just asked for.
        assert_eq!(out.layout.menu, before.layout.menu);
        assert_eq!(out.layout.menu_rows, before.layout.menu_rows);
        assert_eq!(out.layout.hit(x, y), Some(Hit::MenuRow(1)));
    }

    /// A folder with more subfolders than the box has rows scrolls. The rows
    /// that are drawn are the rows the list is showing, and nothing is dropped
    /// off the bottom of the box.
    #[test]
    fn the_picker_s_list_scrolls_instead_of_dropping_folders() {
        let names: Vec<String> = (0..60).map(|n| format!("dir{n:02}")).collect();
        let inside: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut picker = a_picker(&inside, &[]);
        let out = render_picker(&picker, 1205.0, 791.0, None);
        let rows = out.layout.picker_capacity(13.0);
        assert!(
            (PICKER_MIN_ROWS..=PICKER_MAX_ROWS).contains(&rows),
            "{rows} rows"
        );
        assert_eq!(out.layout.picker_rows.len(), rows);
        assert_eq!(out.layout.picker_rows[0].0, 0, "anchored at the top");
        let last = out.layout.picker_rows.last().unwrap().1;
        assert!(
            last.y + last.h <= out.layout.picker_list.y + out.layout.picker_list.h + 0.01,
            "the last row hangs out of the list"
        );
        assert!(
            picker.thumb(rows).is_some(),
            "a list that does not fit says so"
        );

        // Moved down, the rows drawn are the rows the list moved to.
        assert!(picker.scroll(5, true, rows));
        let out = render_picker(&picker, 1205.0, 791.0, None);
        assert_eq!(out.layout.picker_rows[0].0, 5);
        assert_eq!(out.layout.picker_rows.len(), rows);
        for (index, row) in &out.layout.picker_rows {
            let (x, y) = middle(*row);
            assert_eq!(out.layout.hit(x, y), Some(Hit::PickerRow(*index)));
        }

        // A short list keeps the box a readable size rather than collapsing to
        // two rows, and a window too small for the whole box does not push it
        // off the surface.
        let short = render_picker(&a_picker(&["one"], &[]), 1205.0, 791.0, None);
        assert!(short.layout.picker_capacity(13.0) >= PICKER_MIN_ROWS);
        let tiny = render_picker(&picker, 680.0, 380.0, None);
        assert!(tiny.layout.picker.h <= 380.0 - TITLE_H);
        assert!(!tiny.layout.picker_rows.is_empty(), "it still lists folders");
        for rect in &tiny.scene.rects {
            let [x, y, w, h] = rect.xywh();
            assert!(x >= -0.01 && y >= -0.01 && x + w <= 680.01 && y + h <= 380.01);
        }
    }

    /// The window with the settings panel up, laid out and drawn off one shape,
    /// which is what makes a row land where it is drawn.
    fn render_settings(panel: &Settings, w: f32, h: f32, hot: Option<Hit>) -> Rendered {
        render_settings_at_rail(panel, w, h, hot, crate::config::SETTINGS_RAIL)
    }

    /// The same with the rail dragged to `rail` of the panel's width, which is
    /// the only thing a drag of the line beside it changes.
    fn render_settings_at_rail(
        panel: &Settings,
        w: f32,
        h: f32,
        hot: Option<Hit>,
        rail: f32,
    ) -> Rendered {
        render_settings_with(panel, w, h, hot, rail, None, PANE_TEXT)
    }

    /// And the same with a drag over the document, which is what puts a band
    /// under the glyphs.
    fn render_settings_selecting(
        panel: &Settings,
        w: f32,
        h: f32,
        selection: crate::select::Selection,
    ) -> Rendered {
        render_settings_with(panel, w, h, None, crate::config::SETTINGS_RAIL, Some(selection), PANE_TEXT)
    }

    /// The pane text every other settings test is laid out and drawn in: the
    /// size and the advance of one character at it.
    const PANE_TEXT: (f32, f32) = (13.0, 8.0);

    /// The biggest the settings file will carry, and what a character of a
    /// monospace face costs at that size. `font_size` and `pane_font_size` are
    /// both clamped to 40 by `Config::apply`, and `column_width` measures the
    /// real face and falls back to six tenths of the size, which is what a
    /// monospace advance is within a pixel either way.
    const BIGGEST_TEXT: (f32, f32) = (40.0, 24.0);

    /// The same panel at one font size, since the rail's layout is a question
    /// about how many lines of that size fit in the window.
    fn render_settings_at_font(panel: &Settings, w: f32, h: f32, font: (f32, f32)) -> Rendered {
        render_settings_with(panel, w, h, None, crate::config::SETTINGS_RAIL, None, font)
    }

    fn render_settings_with(
        panel: &Settings,
        w: f32,
        h: f32,
        hot: Option<Hit>,
        rail: f32,
        selection: Option<crate::select::Selection>,
        font: (f32, f32),
    ) -> Rendered {
        let dock = Dock::new();
        let state = busy_state();
        let mut shape = shape(&dock, &["a.rs"]);
        shape.settings = Some(panel);
        shape.settings_rail = rail;
        shape.pane_size = font.0;
        shape.pane_column = font.1;
        let layout = Layout::compute(w, h, &shape);
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
            pane_column: font.1,
            body_size: 14.0,
            pane_size: font.0,
            clock: 0.0,
            orb_morph: None,
            drag: None,
            hot,
            trouble: None,
            esc_armed: false,
            popup_scroll: 0,
            cursor: (-100.0, -100.0),
            selection,
            menu: None,
            picker: None,
            settings: Some(panel),
        });
        Rendered {
            scene,
            layout,
            skin,
        }
    }


    fn a_settings_panel(config: &Config) -> Settings {
        Settings::open(
            config,
            Some(std::path::Path::new("/home/hec/.config/noob/no0b.conf")),
            an_agent(),
        )
    }

    /// The panel with one section chosen, which is what a press on the rail or
    /// a Tab leaves behind. The keyboard is on the rows of it either way: no
    /// arrow key touches the rail.
    /// The arrangement the window opens with, with one view's tab brought to
    /// the front of the space it lives in: FILES and ACTIVITY are tabs of the
    /// conversation's own space now, so a test about either has to show it the
    /// way a press on its tab would.
    /// Every view in one space, for the tests about a strip with more tabs
    /// than it can draw. The arrangement the window opens with gives each
    /// space a few, which is not a strip that overflows.
    fn a_crowded_dock(space: Space) -> Dock {
        let mut dock = Dock::new();
        for view in View::ALL {
            // The conversation stays where it is, so the grid keeps its
            // columns: a window with one space in it gives that space the whole
            // width, and a strip that wide fits every tab there is.
            if view != View::Output && dock.space_of(view).is_some() {
                dock.move_view(view, space);
            }
        }
        dock.slot_mut(space).show_at(0);
        dock
    }


    fn a_panel_on(config: &Config, section: &str) -> Settings {
        let mut panel = a_settings_panel(config);
        let at = panel
            .section_names()
            .iter()
            .position(|name| *name == section)
            .unwrap_or_else(|| panic!("{section} is not a section"));
        panel.choose(at);
        panel
    }

    /// The end of the section that is showing, for a test about a row near the
    /// bottom of a list longer than a window.
    ///
    /// A section of cards is taller than the panel: the AGENT section is its
    /// cards with the prompt block as the last row of it. The window clamps
    /// whatever this asks for to the last screenful it can start on, which is
    /// exactly what the wheel does.
    fn scrolled_to_the_end(panel: &mut Settings) {
        let rows = 8;
        while panel.scroll(4, true, rows, 80) {}
    }

    /// The panel is a takeover: while it is up there are no panes, no tabs and
    /// no prompt, and it answers for every point under the title strip. The
    /// strip itself still works, so the window can be moved and closed from it.
    #[test]
    fn the_settings_panel_takes_the_whole_window() {
        let panel = a_settings_panel(&Config::default());
        let out = render_settings(&panel, 1205.0, 1600.0, None);
        let layout = &out.layout;
        assert!(layout.in_settings);
        assert!(!layout.picking, "the two takeovers are different shapes");
        for space in Space::ALL {
            assert!(layout.placed(space).tabs.is_empty(), "{space:?}");
            assert_eq!(layout.placed(space).body.w, 0.0, "{space:?}");
            assert_eq!(layout.placed(space).arrow_left.w, 0.0, "{space:?}");
            assert_eq!(layout.placed(space).arrow_right.w, 0.0, "{space:?}");
        }
        assert_eq!(layout.input.w, 0.0, "the prompt is behind the panel");
        assert_eq!(layout.cell(600.0, 400.0, 13.0, 8.0), None);

        // The whole surface under the strip, rather than a box in the middle of
        // it: sixty rows in a picker-sized box is six rows and a lot of margin.
        let box_ = layout.settings;
        assert!(box_.y >= TITLE_H, "it starts below the strip: {box_:?}");
        assert!(box_.y + box_.h <= 1600.0 && box_.x + box_.w <= 1205.0, "{box_:?}");
        assert!(box_.w >= 1205.0 - 4.0 * GAP, "not a takeover: {box_:?}");
        assert!(box_.h >= 1600.0 - TITLE_H - 4.0 * GAP, "not a takeover: {box_:?}");

        assert_eq!(
            layout.hit(box_.x + 1.0, box_.y + box_.h - 1.0),
            Some(Hit::Settings),
            "its own margin swallows a press rather than passing it on"
        );
        assert_eq!(layout.hit(400.0, 8.0), Some(Hit::TitleBar));
        let (x, y) = middle(layout.close);
        assert_eq!(layout.hit(x, y), Some(Hit::Close));
    }

    /// Every row of every section is hit where it is drawn, the control at the
    /// end of a row that can change is its own region, and a row that cannot
    /// change has none.
    #[test]
    fn every_settings_row_lands_where_it_is_drawn() {
        for section in crate::settings::SECTIONS {
            let panel = a_panel_on(&Config::default(), section);
            let out = render_settings(&panel, 1400.0, 1600.0, None);
            let layout = &out.layout;
            assert!(!layout.settings_rows.is_empty(), "{section} draws no rows");
            for (index, side, row) in &layout.settings_rows {
                assert!(
                    layout.settings_list.contains(row.x + 1.0, row.y + 1.0),
                    "row {index} of {section} is outside the list: {row:?}"
                );
                // The left of the row, which is the label, puts the cursor there.
                assert_eq!(
                    layout.hit(row.x + 2.0, row.y + row.h * 0.5),
                    Some(Hit::SettingsRow(*index, *side)),
                    "{section}"
                );
                // What a row carries: a track when its setting has a range, a
                // value when it is a flag, a preset or the endpoint, and nothing
                // at all when there is nothing to press.
                // A palette card is all controls: one cell per colour in its
                // body, each one hit where its block is drawn.
                if let Some(crate::settings::Row::Palette(palette)) = panel.row(*index) {
                    let cells: Vec<(usize, Panel)> = layout
                        .settings_cells
                        .iter()
                        .filter(|(at, ..)| at == index)
                        .map(|(_, cell, panel)| (*cell, *panel))
                        .collect();
                    assert_eq!(cells.len(), palette.cells.len(), "row {index} of {section}");
                    for (cell, at) in cells {
                        let (x, y) = middle(at);
                        assert_eq!(
                            layout.hit(x, y),
                            Some(Hit::SettingsSwatch(*index, cell)),
                            "{section}"
                        );
                        assert!(row.contains(x, y), "cell {cell} is outside its row");
                    }
                    continue;
                }
                // A choice is drawn as all of its options, so it is those
                // presses rather than one over the field: every option is hit
                // where its own box is.
                if let Some(crate::settings::Row::Setting {
                    kind: crate::settings::Kind::Choice(names),
                    ..
                }) = panel.cell(*index, *side)
                {
                    let boxes: Vec<(usize, Panel)> = layout
                        .settings_choices
                        .iter()
                        .filter(|(at, half, ..)| at == index && half == side)
                        .map(|(_, _, option, panel)| (*option, *panel))
                        .collect();
                    assert_eq!(boxes.len(), names.len(), "row {index} of {section}");
                    for (option, at) in boxes {
                        let (x, y) = middle(at);
                        assert_eq!(
                            layout.hit(x, y),
                            Some(Hit::SettingsChoice(*index, *side, option)),
                            "{section}"
                        );
                        assert!(row.contains(x, y), "option {option} is outside its row");
                    }
                    continue;
                }
                let wanted = match panel.cell(*index, *side) {
                    Some(crate::settings::Row::Setting { kind, .. })
                        if kind.fraction(0.0).is_some() =>
                    {
                        Some(Hit::SettingsSlider(*index, *side))
                    }
                    Some(crate::settings::Row::Setting { .. })
                    | Some(crate::settings::Row::Field { .. }) => {
                        Some(Hit::SettingsValue(*index, *side))
                    }
                    _ => None,
                };
                let control = layout
                    .settings_values
                    .iter()
                    .chain(layout.settings_tracks.iter())
                    .find(|(at, half, _)| at == index && half == side)
                    .map(|(_, _, panel)| *panel);
                match (wanted, control) {
                    (Some(hit), Some(control)) => {
                        let (x, y) = middle(control);
                        assert_eq!(layout.hit(x, y), Some(hit), "{section}");
                        assert!(row.contains(x, y), "the control is outside its row");
                    }
                    // The table of saved conversations is a card whose body is a
                    // list, so the middle of it is a row of that list rather
                    // than more of the panel row it stands in. The card itself
                    // still answers where its own border and header are.
                    (None, None)
                        if matches!(panel.row(*index), Some(crate::settings::Row::Table(_))) =>
                    {
                        assert_eq!(
                            layout.hit(row.x + 1.0, row.y + 1.0),
                            Some(Hit::SettingsRow(*index, *side)),
                            "{section}: the card's own edge is not the card"
                        );
                        let picks = layout
                            .settings_picks
                            .iter()
                            .filter(|(at, _, _)| at == index)
                            .count();
                        assert!(picks > 0, "{section}: the table has no rows to press");
                    }
                    // A heading, a note, a column name or a reading: the whole
                    // row is the row, and a press on its right hand end changes
                    // nothing.
                    (None, None) => assert_eq!(
                        layout.hit(row.x + row.w - 2.0, row.y + row.h * 0.5),
                        Some(Hit::SettingsRow(*index, *side)),
                        "{section}"
                    ),
                    other => panic!("row {index} of {section} carries {other:?}"),
                }
            }
            // The controls of a column all start in the same place, which is
            // what makes a screen of settings scannable rather than a wall of
            // words. Per column rather than across the panel, because a form row
            // has two of them: the right hand column lines up with itself.
            // Values with values and tracks with tracks: a slider stands a
            // column of air in from where an input box starts, on purpose.
            for want in [crate::settings::Side::Left, crate::settings::Side::Right] {
                for group in [&layout.settings_values, &layout.settings_tracks] {
                    let lefts: Vec<f32> = group
                        .iter()
                        .filter(|(_, side, _)| *side == want)
                        .map(|(_, _, p)| p.x)
                        .collect();
                    assert!(
                        lefts.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.01),
                        "{section}: {lefts:?}"
                    );
                }
            }
        }
    }

    /// An agent with instructions of its own, for the system prompt section's
    /// document.
    fn an_agent_with_instructions() -> crate::agent::Agent {
        crate::agent::Agent {
            instructions: crate::agent::Instructions {
                path: Some(std::path::PathBuf::from("/home/hec/.config/noob/AGENTS.md")),
                body: vec![
                    String::from("# Global instructions"),
                    String::new(),
                    String::from("Answer in as few words as carry the answer."),
                ],
                capped: false,
            },
            ..an_agent()
        }
    }

    /// The AGENT section is cards: every field a label over its value with the
    /// sentence that says what it is under that, and each one pressed where it
    /// is drawn.
    ///
    /// "all text looks the same name, description, repo, find a way each thing
    /// is different": a label, a value and a sentence are three sizes and three
    /// tints here, and none of them is the raw environment key the row used to
    /// be named with.
    #[test]
    fn the_agent_cards_draw_a_labelled_field_for_everything_they_hold() {
        let panel = a_panel_on(&Config::default(), crate::settings::AGENT);
        let out = render_settings(&panel, 1400.0, 1600.0, None);
        let layout = &out.layout;
        let line = Text::line_for(PANE_TEXT.0);
        let cols = layout.settings_entry_columns(PANE_TEXT.1);
        // Full width is the width the rows are drawn in, which is the list less
        // the gutter its own scrollbar stands in.
        let list = settings_list_rows(layout.settings_list);

        // Every card is full width and stands in the row the model counted for
        // it, with the space between two of them under the one above.
        let cards: Vec<(usize, Panel)> = layout
            .settings_rows
            .iter()
            .filter(|(index, ..)| matches!(panel.row(*index), Some(SettingRow::Card(_))))
            .map(|(index, _, at)| (*index, *at))
            .collect();
        assert!(cards.len() >= 4, "the section is not cards: {cards:?}");
        for (index, row) in &cards {
            let Some(SettingRow::Card(card)) = panel.row(*index) else {
                panic!("row {index} is not a card");
            };
            // Full width, or one half of a band two cards share.
            let (left, right) = settings_card_halves(list, PANE_TEXT.1);
            let paired = crate::settings::stands_beside(panel.rows(), *index)
                || (*index > 0 && crate::settings::stands_beside(panel.rows(), index - 1));
            match paired {
                true => assert!(
                    ((row.x - left.x).abs() < 0.01 || (row.x - right.x).abs() < 0.01)
                        && (row.w - left.w).abs() < 0.01,
                    "a card of a pair is not half the list: {row:?}"
                ),
                false => {
                    assert!((row.x - list.x).abs() < 0.01, "a card is not full width");
                    assert!((row.w - list.w).abs() < 0.01, "a card is not full width");
                }
            }
            let counted = crate::settings::band_lines(panel.rows(), *index, cols);
            assert!(
                (row.h - counted as f32 * line).abs() < 0.01,
                "card {index} is {row:?} and the model counted {counted} lines"
            );

            // Every field: its label, its value under it, its sentence under
            // that, and the whole of it inside the card.
            let (box_, parts) = the_card(&out, *row, false);
            let hints = crate::settings::card_hints(card);
            // The card's own width, which is half the list on one of a pair.
            let card_cols = settings_entry_cols(row.w, PANE_TEXT.1);
            let across = design::across(card.fields.len(), design::card_cols(card_cols));
            let slots = settings_card_slots(
                parts.body,
                line,
                &hints,
                across,
                card.group.as_ref().map(|group| group.at),
            );
            for (field, slot) in card.fields.iter().zip(&slots) {
                let (label_at, input_at) = settings_field_boxes(*slot, line);
                assert_eq!(
                    line_of(&out, label_at.x, label_at.y),
                    field.label,
                    "the label of a field is not on its own line"
                );
                assert!(
                    input_at.y >= label_at.y + label_at.h - 0.01,
                    "a value is beside its label rather than under it"
                );
                let Some(hint) = &field.hint else {
                    continue;
                };
                let hint_at = settings_hint_box(*slot, line, PANE_TEXT.0);
                // Clipped to what the field is wide enough for, so what is
                // drawn is the head of the sentence and the mark that says so.
                let said = line_of(&out, hint_at.x, hint_at.y);
                let head = said.trim_end_matches('\u{2026}');
                assert!(
                    !head.is_empty() && hint.starts_with(head),
                    "{said:?} is not the sentence under {}",
                    field.label
                );
                // Smaller than the value it explains, and inside its own card.
                assert!(hint_at.y + hint_at.h <= box_.y + box_.h + 0.01);
            }
        }

        // The two things this section can change are drawn as controls, in the
        // fields they belong to, and each is pressed where it is drawn: the
        // endpoint is typed into, the context window is dragged.
        let endpoint = *layout
            .settings_values
            .first()
            .expect("the endpoint has a box");
        let ctx = *layout
            .settings_tracks
            .first()
            .expect("the context window has a track");
        for (index, _, at) in [endpoint, ctx] {
            let row = cards
                .iter()
                .find(|(card, _)| *card == index)
                .map(|(_, row)| *row)
                .unwrap_or_else(|| panic!("row {index} is not a card"));
            assert!(row.contains(at.x + 1.0, at.y + 1.0), "{at:?} is outside {row:?}");
        }
        let (x, y) = middle(endpoint.2);
        assert_eq!(
            layout.hit(x, y),
            Some(Hit::SettingsValue(endpoint.0, endpoint.1))
        );
        let (x, y) = middle(ctx.2);
        assert_eq!(layout.hit(x, y), Some(Hit::SettingsSlider(ctx.0, ctx.1)));

        // What is drawn: plain-words names, the values off the file, and the
        // keys in the sentences under them rather than as the names.
        let text = text_of(&out.scene);
        for wanted in [
            "endpoint",
            "context window",
            "THE SETTINGS FILE",
            "http://localhost:8080/v1",
            crate::agent::ENDPOINT,
            crate::agent::CTX,
        ] {
            assert!(text.contains(wanted), "{wanted} is not drawn: {text}");
        }
    }

    /// A slider is a bare track: no input box behind it at rest, and neither
    /// rollover nor the cursor adds one rectangle to it.
    ///
    /// The flat rows kept this rule already; the card fields did not. A card
    /// slider stood in the filled, edged, cut-cornered box a typed value
    /// wears, and the track lit under the pointer: an input's costume on a
    /// control that is dragged, and a rollover effect nobody asked for.
    #[test]
    fn a_card_slider_is_a_bare_track_that_rollover_does_not_change() {
        let panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 1000.0, None);
        assert!(!out.layout.settings_tracks.is_empty(), "no slider to look at");
        let holds = |out: &Rendered, track: Panel, rgba: [f32; 4]| {
            let (cx, cy) = middle(track);
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == rgba && cx >= x && cx <= x + w && cy >= y && cy <= y + h
            })
        };
        for (_, _, track) in &out.layout.settings_tracks {
            // The track is drawn, and nothing boxes it: not the input's fill,
            // not a hover band.
            assert!(holds(&out, *track, out.skin.gauge_track), "no track at {track:?}");
            for rgba in [out.skin.input, out.skin.hot] {
                assert!(
                    !holds(&out, *track, rgba),
                    "a slider stands in a box: {rgba:?} behind {track:?}"
                );
            }
        }
        // Pointing at the track, or at the value beside it, changes not one
        // rectangle of the scene.
        let (index, side, _) = out.layout.settings_tracks[0];
        let resting: Vec<([f32; 4], [f32; 4])> = out
            .scene
            .rects
            .iter()
            .map(|rect| (rect.xywh(), rect.rgba()))
            .collect();
        for hot in [
            Hit::SettingsSlider(index, side),
            Hit::SettingsValue(index, side),
        ] {
            let lit = render_settings(&panel, 1400.0, 1000.0, Some(hot));
            let now: Vec<([f32; 4], [f32; 4])> = lit
                .scene
                .rects
                .iter()
                .map(|rect| (rect.xywh(), rect.rgba()))
                .collect();
            assert_eq!(now, resting, "{hot:?} changed the slider's look");
        }
    }

    /// The document draws where it lives: the global AGENTS.md on the SYSTEM
    /// PROMPT section, with its title, its path and its text.
    #[test]
    fn the_system_prompt_draws_the_document() {
        let mut panel = Settings::open(
            &Config::default(),
            None,
            an_agent_with_instructions(),
        );
        let doc = panel
            .section_names()
            .iter()
            .position(|name| *name == crate::settings::PROMPT)
            .expect("the system prompt section");
        panel.choose(doc);
        let out = render_settings(&panel, 1400.0, 1200.0, None);
        let text = text_of(&out.scene);

        // The file, by name and by path, with what is in it under the title.
        assert!(text.contains("AGENTS.md"), "{text}");
        assert!(text.contains("/home/hec/.config/noob/AGENTS.md"), "{text}");
        assert!(
            text.contains("Answer in as few words as carry the answer."),
            "the instructions are not drawn: {text}"
        );
        // Rendered as Markdown rather than printed with its marks in, the way
        // the column beside the skills list renders a SKILL.md.
        assert!(text.contains("Global instructions"), "{text}");
        assert!(!text.contains("# Global instructions"), "{text}");

        // The title is the theme's accent, the way every heading on this
        // panel is, so the block reads as a block rather than as more rows.
        let title = out
            .scene
            .texts
            .iter()
            .flat_map(|text| text.runs.iter())
            .find(|run| run.text == "AGENTS.md")
            .expect("the document's title");
        assert_eq!(title.color, Some(out.skin.heading));
    }

    /// A block of text is a card too, and it scrolls inside itself: its title
    /// stays in the header, its border stays where it was, and the rows around
    /// it do not move.
    ///
    /// Before it was a card the two documents were the one thing on a panel of
    /// boxes with nothing round them: a bare title, a bare path and twelve lines
    /// of prose reading as loose text between two cards.
    #[test]
    fn a_document_is_a_card_that_scrolls_inside_itself() {
        let body: Vec<String> = (0..crate::settings::PAPER_LINES * 3)
            .map(|at| format!("line {at} of the document"))
            .collect();
        let mut agent = an_agent();
        agent.instructions = crate::agent::Instructions {
            path: Some(std::path::PathBuf::from("/home/hec/.config/noob/AGENTS.md")),
            body: body.clone(),
            capped: false,
        };
        let mut panel = Settings::open(&Config::default(), None, agent);
        let section = panel
            .section_names()
            .iter()
            .position(|name| *name == crate::settings::PROMPT)
            .expect("the system prompt section");
        panel.choose(section);
        let out = render_settings(&panel, 1400.0, 1200.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let cols = out.layout.settings_entry_columns(PANE_TEXT.1);

        let at = panel
            .rows()
            .iter()
            .position(|row| {
                matches!(row, crate::settings::Row::Paper(paper) if !paper.body.is_empty())
            })
            .expect("the document block");
        let row = out
            .layout
            .settings_rows
            .iter()
            .find(|(index, ..)| *index == at)
            .map(|(_, _, row)| *row)
            .expect("the block is on screen");

        // It stands in the room the model counted, as a card with its title in
        // the header and a border round the whole of it.
        let counted = crate::settings::lines(panel.row(at).expect("the row"), cols);
        assert_eq!(
            counted,
            design::card_row_lines(crate::settings::paper_body_lines(), true),
            "an editable document is a card with a footer"
        );
        assert!((row.h - counted as f32 * line).abs() < 0.01, "{row:?}");
        let (box_, parts) = the_card(&out, row, true);
        assert!(
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                (x - box_.x).abs() < 0.01
                    && (y - box_.y).abs() < 0.01
                    && (w - box_.w).abs() < 0.01
                    && (h - box_.h).abs() < 0.01
                    && rect.extra()[3] >= 1.0
            }),
            "the block has no border round it"
        );
        let title = out
            .scene
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text == "AGENTS.md"))
            .expect("the title");
        assert!(
            (title.at.y - parts.title.y).abs() < 0.01,
            "the title is not in the header: {:?}",
            title.at
        );

        // Its text is inside the body, and the first line of it is the first
        // line of the document.
        let first = parts.body.y + line + design::tight(line);
        assert_eq!(line_of(&out, parts.body.x, first), body[0]);
        let last = parts.body.y + parts.body.h;
        assert!(
            first + crate::settings::PAPER_LINES as f32 * line <= last + 0.01,
            "the twelve lines it holds do not fit its body"
        );

        // Paged, the text moves and the card does not: the same box, the same
        // title, another twelve lines. A block that walked the list under it
        // would take the rows below it with it.
        panel.point_at(at, Side::Left);
        assert_eq!(panel.cursor(), at);
        assert!(panel.page(20, true), "the block did not scroll");
        let after = render_settings(&panel, 1400.0, 1200.0, None);
        let moved = after
            .layout
            .settings_rows
            .iter()
            .find(|(index, ..)| *index == at)
            .map(|(_, _, row)| *row)
            .expect("the block is still on screen");
        assert_eq!(moved, row, "the card moved while its text was read");
        assert_eq!(
            line_of(&after, parts.body.x, first),
            body[crate::settings::PAPER_LINES],
            "the text did not scroll inside the card"
        );
        assert!(
            text_of(&after.scene).contains("13-24 of 36"),
            "the block does not say how far down it is: {}",
            text_of(&after.scene)
        );
    }

    /// A file that is not there shows the shipped default under a
    /// note, with the checkbox that starts owning it. Never an empty box.
    #[test]
    fn a_missing_file_shows_the_built_in_text_on_its_own_block() {
        // `an_agent` has neither AGENTS.md nor TOOLS.md, so both blocks say
        // where the file would go and draw the text the agent runs with.
        let out = render_settings(
            &a_panel_on(&Config::default(), crate::settings::PROMPT),
            1400.0,
            1200.0,
            None,
        );
        let text = text_of(&out.scene);
        assert!(text.contains("not written yet"), "{text}");
        assert!(text.contains("/home/hec/.config/noob/AGENTS.md"), "{text}");
        assert!(text.contains("/home/hec/.config/noob/TOOLS.md"), "{text}");
        assert!(
            text.contains("You are noob, an agent working in the current directory."),
            "the built-in text is not drawn: {text}"
        );
        assert!(text.contains("enable edition"), "{text}");

        // The checkbox is pressed where it is drawn, and every action the
        // block has stands beside it whether or not edition is on: they are
        // drawn dim while it is off, rather than appearing out of a footer
        // that was empty a moment ago.
        let acts: Vec<Act> = out
            .layout
            .settings_acts
            .iter()
            .filter(|(index, ..)| *index == 0)
            .map(|(_, act, _)| *act)
            .collect();
        assert_eq!(
            acts,
            [Act::EditPrompt, Act::LoadPrompt, Act::RestorePrompt, Act::SavePrompt],
            "{acts:?}"
        );
        let (index, _, box_) = out
            .layout
            .settings_acts
            .iter()
            .find(|(index, act, _)| *index == 0 && *act == Act::EditPrompt)
            .expect("the checkbox has a box");
        let (x, y) = middle(*box_);
        assert_eq!(out.layout.hit(x, y), Some(Hit::SettingsAct(*index, Act::EditPrompt)));

        // Ticked, the save and the armed restore appear beside it, each
        // pressed where it is drawn.
        let mut panel = a_panel_on(&Config::default(), crate::settings::PROMPT);
        assert!(panel.toggle_edition(0, &Config::default()));
        let on = render_settings(&panel, 1400.0, 1200.0, None);
        let acts: Vec<Act> = on
            .layout
            .settings_acts
            .iter()
            .filter(|(index, ..)| *index == 0)
            .map(|(_, act, _)| *act)
            .collect();
        assert_eq!(
            acts,
            [Act::EditPrompt, Act::LoadPrompt, Act::RestorePrompt, Act::SavePrompt],
            "{acts:?}"
        );
        let text = text_of(&on.scene);
        for word in ["save", "restore", "load"] {
            assert!(text.contains(word), "{word} is not drawn: {text}");
        }
        for (index, act, box_) in on.layout.settings_acts.iter().filter(|(index, ..)| *index == 0)
        {
            let (x, y) = middle(*box_);
            assert_eq!(on.layout.hit(x, y), Some(Hit::SettingsAct(*index, *act)));
        }
    }

    /// The skills section is two columns: the entries down the left, and the
    /// `SKILL.md` of the one under the cursor beside them, rendered rather than
    /// printed with its marks in.
    #[test]
    fn the_skills_section_puts_the_skill_beside_the_list() {
        let mut panel = a_panel_on(&Config::default(), crate::settings::SKILLS);
        on_the_installed_skill(&mut panel);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let (list, doc) = (layout.settings_list, layout.settings_doc);
        assert!(doc.w >= 1.0, "there is no second column");
        assert!(
            doc.x + doc.w <= list.x + list.w + 0.01,
            "the document runs off the list: {doc:?} in {list:?}"
        );
        // The document stands beside the rows it belongs to and over nothing
        // else: the install form above the table keeps the whole width, and no
        // row runs into the document's column.
        let mut wide = 0;
        for (index, _, row) in &layout.settings_rows {
            let inside = row.y + row.h > doc.y && row.y < doc.y + doc.h;
            match inside {
                true => assert!(
                    row.x + row.w <= doc.x + 0.01,
                    "row {index} runs into the document: {row:?}"
                ),
                false => {
                    wide += usize::from(
                        (row.w - settings_list_rows(list).w).abs() < 1.01,
                    );
                }
            }
        }
        assert!(wide > 0, "no row above the table keeps the whole width");
        // The text of the document is its own region, because there is a
        // selection to begin there. This asserted `Hit::Settings` for as long as
        // the whole panel body was one swallowed press, and the region is what
        // changed rather than the geometry around it: the title line over the
        // box and the border around it are still panel.
        let (x, y) = middle(doc);
        assert_eq!(layout.hit(x, y), Some(Hit::SettingsDoc), "{doc:?}");
        let text = layout.settings_doc_text;
        assert!(text.w >= 1.0 && text.contains(x, y), "{text:?}");
        assert_eq!(
            layout.hit(doc.x + PAD, doc.y + 1.0),
            Some(Hit::Settings),
            "the title over the box is not text to select"
        );

        // What is drawn: the row on the left, the document on the right, and
        // no Markdown marks in it.
        let text = text_of(&out.scene);
        assert!(text.contains("coding"), "{text}");
        assert!(
            text.contains("https://github.com/someone/cod"),
            "the row does not say where the skill came from: {text}"
        );
        assert!(
            text.contains("Read the file before writing it."),
            "the skill's own document is not beside the list: {text}"
        );
        assert!(
            !text.contains("# Changing code"),
            "the document is printed rather than rendered: {text}"
        );

        // A section with no entries is one column, so the settings keep the
        // whole width they had.
        let plain = render_settings(
            &a_panel_on(&Config::default(), crate::settings::APPEARANCE),
            1400.0,
            900.0,
            None,
        );
        assert!(plain.layout.settings_doc.w < 1.0, "APPEARANCE grew a column");
        // The list itself is the same width on both: what narrows is the row
        // the document stands beside, not the column the rows are drawn in.
        assert!(
            (plain.layout.settings_list.w - list.w).abs() < 0.01,
            "the list changed width for a section with a document"
        );
        let widest = layout
            .settings_rows
            .iter()
            .map(|(_, _, row)| row.w)
            .fold(0.0_f32, f32::max);
        assert!(widest > doc.x - list.x, "no row is wider than the document's column");
    }

    /// A window too narrow to hold both columns is one column: the entries win,
    /// because a document forty characters wide is a column of broken words.
    #[test]
    fn a_narrow_panel_keeps_the_entries_and_drops_the_column() {
        let panel = a_panel_on(&Config::default(), crate::settings::SKILLS);
        let out = render_settings(&panel, 520.0, 400.0, None);
        assert!(out.layout.settings_doc.w < 1.0, "{:?}", out.layout.settings_doc);
        assert!(!out.layout.settings_rows.is_empty(), "and the list is still there");
    }

    /// The panel opened on the saved conversations, with three of them to draw.
    fn a_sessions_panel() -> Settings {
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        let saved = |id: &str, ago: u64, folder: &str, said: &str| crate::sessions::Saved {
            id: String::from(id),
            when: now - std::time::Duration::from_secs(ago),
            workspace: Some(std::path::PathBuf::from(folder)),
            gone: false,
            bytes: 12_000,
            context: None,
            opening: String::from(said),
        };
        let mut panel = Settings::open(
            &Config::default(),
            None,
            crate::agent::Agent {
                now,
                sessions: crate::sessions::Listing {
                    sessions: vec![
                        saved("aaa", 60, "/home/hec/workspace/noob-cli", "fix the panel"),
                        saved("bbb", 3_600, "/home/hec/workspace/anna", "read the map"),
                        saved("ccc", 86_400, "/home/hec/notes", "what did we say"),
                    ],
                    skipped: Vec::new(),
                },
                ..crate::agent::Agent::default()
            },
        );
        let at = panel
            .section_names()
            .iter()
            .position(|name| *name == crate::settings::SESSIONS)
            .expect("the sessions section");
        panel.choose(at);
        panel
    }

    /// Item H3: the saved conversations are a table inside a card. A row that
    /// read "17h ago  noob-cli  283 B  -  fix the panel" said five things with
    /// nothing naming any of them, so every cell sits in a column of its own
    /// under a name that says what that column is, the row the keys are on
    /// carries a band across the whole of it rather than differently coloured
    /// words, and the whole list stands in one card: a header saying how many
    /// there are, the table in the body, the buttons in the footer.
    #[test]
    fn the_sessions_section_is_a_table_in_a_card_with_its_buttons_in_the_footer() {
        let panel = a_sessions_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let column = 8.0;
        let line = Text::line_for(PANE_TEXT.0);
        let index = panel
            .rows()
            .iter()
            .position(|row| matches!(row, crate::settings::Row::Table(_)))
            .expect("the section carries a table");
        let table = panel.table(index).expect("a table");
        let row = layout
            .settings_rows
            .iter()
            .find(|(at, _, _)| *at == index)
            .map(|(_, _, at)| *at)
            .expect("the table is placed");
        let card = settings_card(row, line);
        let parts = settings_card_parts(
            card,
            line,
            PANE_TEXT.0,
            column,
            layout.settings_entry_columns(column),
            true,
        );
        let (names_at, boxes) = settings_table_parts(parts.body, line);

        // One region per conversation, all three of them inside the body of the
        // card, and each one carries which row of the table it is.
        let picks: Vec<(usize, Panel)> = layout
            .settings_picks
            .iter()
            .filter(|(at, _, _)| *at == index)
            .map(|(_, on, at)| (*on, *at))
            .collect();
        assert_eq!(picks.len(), 3, "every saved conversation is drawn");
        // The body was measured for at least the fixed row count, and the
        // last card of a section that fits stretches to the bottom of the
        // list, so the body may hold more boxes than the model counted.
        assert!(boxes.len() >= crate::settings::TABLE_ROWS, "{}", boxes.len());
        for (step, (_, at)) in picks.iter().enumerate() {
            assert!(
                (at.y - boxes[step].y).abs() < 0.01 && (at.h - boxes[step].h).abs() < 0.01,
                "row {step} is not on the band the body counted"
            );
        }
        for (on, at) in &picks {
            let (x, y) = middle(*at);
            assert_eq!(layout.hit(x, y), Some(Hit::SettingsPick(index, *on)));
            assert!(
                at.y >= parts.body.y - 0.01 && at.y + at.h <= parts.body.y + parts.body.h + 0.01,
                "row {on} is outside the card's body"
            );
        }

        // Every column has its name over it, and the name starts exactly where
        // the cells under it start.
        let names = settings_session_cells(names_at, column, table.columns);
        assert_eq!(names.len(), crate::settings::SESSION_COLUMNS.len());
        let text_at = |out: &Rendered, at: Panel| -> String {
            out.scene
                .texts
                .iter()
                .filter(|text| {
                    (text.at.y - at.y).abs() < 0.01 && (text.at.x - at.x).abs() < 0.01
                })
                .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
                .collect()
        };
        // Where a cell of this column is written: against the left edge of its
        // box for a word, against the right edge for a number. Asserting every
        // cell at its box's left edge is what left the size and the context
        // ragged, so it asserts the edge the column says it takes.
        // Cells stand a padding off the rules on both sides
        // (`settings_session_ink`), so the written x is asked from the same
        // helper the painter uses rather than recomputed here.
        let written_at = |at: Panel, step: usize, text: &str| -> f32 {
            settings_session_ink(at, step, text, column, table.columns).x
        };
        for (step, (name, _, _)) in crate::settings::SESSION_COLUMNS.iter().enumerate() {
            if name.is_empty() {
                continue;
            }
            let shown = clip(name, columns_in(names[step].w, column).saturating_sub(2));
            let x = written_at(names[step], step, &shown);
            let said = text_at(&out, Panel::new(x, names_at.y, 1.0, 1.0));
            assert!(said.starts_with(&shown), "column {step} is headed {said:?}");
        }

        // And every row writes its cells into those same columns, against the
        // same edge the name above it is written against.
        for (on, at) in &picks {
            let cells = table.rows[*on].cells.clone();
            let places = settings_session_cells(*at, column, table.columns);
            for (step, cell) in cells.iter().enumerate() {
                let step = step + table.of.first_cell();
                assert!(
                    (places[step].x - names[step].x).abs() < 0.01,
                    "row {on} column {step} is not under its header"
                );
                let shown = clip(cell, columns_in(places[step].w, column) - 1);
                let x = written_at(places[step], step, &shown);
                let said = text_at(&out, Panel::new(x, at.y, 1.0, 1.0));
                assert!(
                    said.starts_with(&shown),
                    "row {on} column {step} says {said:?}, not {cell:?}"
                );
            }
        }
        assert!(
            matches!(
                crate::settings::SESSION_COLUMNS[3].2,
                crate::settings::Align::Right
            ) && matches!(
                crate::settings::SESSION_COLUMNS[4].2,
                crate::settings::Align::Right
            ),
            "the size and the context are the numeric columns"
        );

        let filled = |out: &Rendered, want: Panel, rgba: [f32; 4]| {
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == rgba
                    && (x - want.x).abs() < 0.01
                    && (y - want.y).abs() < 0.01
                    && (w - want.w).abs() < 0.01
                    && (h - want.h).abs() < 0.01
            })
        };
        // The row the keys are on is a band across the whole row, in the solid
        // colour the folder picker's own session list uses, not a tint on the
        // words.
        let (_, banded) = picks
            .iter()
            .find(|(on, _)| *on == table.cursor)
            .copied()
            .expect("the keys are on a row of the table");
        assert!(filled(&out, banded, out.skin.picked), "no band on the row");
        for (on, at) in &picks {
            if *on != table.cursor {
                assert!(!filled(&out, *at, out.skin.picked), "row {on} is banded too");
            }
        }

        // And the names stand on a filled band of their own, across the body, in
        // the surface this window puts behind a block header. Rules between the
        // columns and a hairline under them were all they had, which read as one
        // more row of the list.
        assert!(
            filled(&out, names_at, out.skin.strip),
            "the header has no band: {names_at:?}"
        );

        // The card says what it holds, in the card title role, and the count is
        // the one thing it can say that the panel's own heading does not.
        let said = text_of(&out.scene);
        assert!(said.contains("3 SESSIONS"), "{said}");
        assert!(
            out.scene.texts.iter().any(|text| {
                (text.at.y - parts.title.y).abs() < 0.01
                    && text.size > PANE_TEXT.0
                    && text
                        .runs
                        .iter()
                        .any(|run| run.text.contains("3 SESSIONS"))
            }),
            "the card's title is not drawn in the card title role"
        );

        // Two heads over two columns: the gear and SETTINGS over the rail, and
        // the section's own title inside the body it titles, on the same top
        // line, starting where the list starts.
        let heading = out
            .scene
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text == " SETTINGS"))
            .expect("the rail says what the panel is");
        let title = out
            .scene
            .texts
            .iter()
            .find(|text| {
                text.runs
                    .iter()
                    .any(|run| run.text.contains(crate::settings::SESSION_TITLE))
                    && (text.at.y - heading.at.y).abs() < 0.01
            })
            .expect("the body is titled with what this section lists");
        assert!(
            title.at.x >= out.layout.settings_list.x - 0.01,
            "the section title is not inside the body: {:?}",
            title.at
        );

        // The three buttons stand in the card's footer, centred as one group,
        // and each one is pressed where it is drawn. They were a trash on the
        // end of every row, which is a delete per conversation and nothing that
        // could take several.
        let acts: Vec<(Act, Panel)> = layout
            .settings_acts
            .iter()
            .filter(|(at, _, _)| *at == index)
            .map(|(_, act, at)| (*act, *at))
            .collect();
        assert_eq!(acts.len(), 3, "select all, select none and the delete");
        for (act, at) in &acts {
            let (x, y) = middle(*at);
            assert_eq!(layout.hit(x, y), Some(Hit::SettingsAct(index, *act)));
            assert!(
                (at.y - parts.footer.y).abs() < 0.01,
                "{act:?} is not on the footer line"
            );
            assert!(card.contains(x, y), "{act:?} is outside its card");
        }
        let right = acts[2].1.x + acts[2].1.w;
        // One group hung on the footer's right end, where every card's own
        // action already sits: centred, the group read as belonging to
        // nothing.
        assert!(
            (parts.footer.x + parts.footer.w - right).abs() <= 0.01,
            "the buttons are not right-aligned: end {right} in {:?}",
            parts.footer
        );
        assert!(said.contains("select all") && said.contains("select none"), "{said}");
        assert!(said.contains("delete"), "{said}");

        // Nothing else on the panel grows any of the three, and the per row
        // trash is gone with them: one delete, under the list it deletes from.
        assert!(layout.settings_removes.is_empty(), "a session still has a trash");
        let plain = render_settings(
            &a_panel_on(&Config::default(), crate::settings::APPEARANCE),
            1400.0,
            900.0,
            None,
        );
        assert!(plain.layout.settings_acts.is_empty());
        assert!(plain.layout.settings_picks.is_empty());
        assert!(plain.layout.settings_marks.is_empty());

        // The card's own border and header are still the card: a press there
        // puts the panel's cursor on it rather than on a conversation.
        assert_eq!(
            layout.hit(row.x + 1.0, row.y + 1.0),
            Some(Hit::SettingsRow(index, crate::settings::Side::Left)),
            "the card's edge is not the card"
        );
    }

    /// The table is used at every width the window has, and at each of them its
    /// rows stay inside its card and under the names of their columns.
    ///
    /// A card is full width and stacks, so what a narrow window costs the table
    /// is the width of its last column and, below the width the three buttons
    /// need, the buttons themselves. Nothing is ever drawn outside the card it
    /// belongs to, which is what a press on a row of it depends on.
    #[test]
    fn the_table_holds_its_rows_inside_its_card_at_every_width() {
        let panel = a_sessions_panel();
        let index = panel
            .rows()
            .iter()
            .position(|row| matches!(row, crate::settings::Row::Table(_)))
            .expect("the section carries a table");
        for (w, h) in [(1400.0, 900.0), (900.0, 700.0), (700.0, 460.0), (520.0, 400.0)] {
            let out = render_settings(&panel, w, h, None);
            let line = Text::line_for(PANE_TEXT.0);
            let Some(row) = out
                .layout
                .settings_rows
                .iter()
                .find(|(at, _, _)| *at == index)
                .map(|(_, _, at)| *at)
            else {
                continue;
            };
            let card = settings_card(row, line);
            // The bottom bound is the visible row box rather than the card
            // arithmetic: a card cut by the bottom of the list shows its rows
            // down to the cut itself, and the layout hands over exactly the
            // visible part.
            for (_, on, at) in out
                .layout
                .settings_picks
                .iter()
                .filter(|(at, _, _)| *at == index)
            {
                assert!(
                    at.x >= card.x - 0.01
                        && at.x + at.w <= card.x + card.w + 0.01
                        && at.y >= card.y - 0.01
                        && at.y + at.h <= row.y + row.h + 0.01,
                    "{w}x{h}: row {on} is outside its card: {at:?} in {card:?}"
                );
                let (x, y) = middle(*at);
                assert_eq!(
                    out.layout.hit(x, y),
                    Some(Hit::SettingsPick(index, *on)),
                    "{w}x{h}: row {on} is not pressed where it is drawn"
                );
            }
            for (_, act, at) in out
                .layout
                .settings_acts
                .iter()
                .filter(|(at, _, _)| *at == index)
            {
                let (x, y) = middle(*at);
                assert!(card.contains(x, y), "{w}x{h}: {act:?} is outside its card");
                assert_eq!(out.layout.hit(x, y), Some(Hit::SettingsAct(index, *act)));
            }
        }
    }

    /// The mark in front of a conversation is its own control: pressing it marks
    /// the row without moving the keys, and the mark is drawn where it is
    /// pressed.
    #[test]
    fn a_conversation_is_marked_by_the_box_in_front_of_it() {
        let mut panel = a_sessions_panel();
        let index = panel
            .rows()
            .iter()
            .position(|row| matches!(row, crate::settings::Row::Table(_)))
            .expect("the section carries a table");
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let marks: Vec<(usize, Panel)> = out
            .layout
            .settings_marks
            .iter()
            .filter(|(at, _, _)| *at == index)
            .map(|(_, on, at)| (*on, *at))
            .collect();
        assert_eq!(marks.len(), 3, "every row can be marked");
        for (on, at) in &marks {
            let (x, y) = middle(*at);
            assert_eq!(out.layout.hit(x, y), Some(Hit::SettingsMark(index, *on)));
            // In the first column of the table, in front of the words.
            let row = out
                .layout
                .settings_picks
                .iter()
                .find(|(row, at, _)| *row == index && at == on)
                .map(|(_, _, at)| *at)
                .expect("the row it belongs to");
            assert!(
                (at.x - row.x).abs() < 0.01 && at.w < row.w * 0.5,
                "the mark is not the first column: {at:?} in {row:?}"
            );
        }

        // Unmarked, every one of them is an empty box. Marked, the row says so
        // where the box is: the whole of what multi selection is is seeing which
        // rows are in it without pressing anything.
        let boxes = |out: &Rendered, at: Panel, icon: char| -> bool {
            out.scene.texts.iter().any(|text| {
                (text.at.y - at.y).abs() < 2.0
                    && text.at.x >= at.x - 1.0
                    && text.at.x < at.x + at.w + 1.0
                    && text
                        .runs
                        .iter()
                        .any(|run| run.icon && run.text.contains(icon))
            })
        };
        for (_, at) in &marks {
            assert!(boxes(&out, *at, icons::UNCHECKED), "no empty box at {at:?}");
        }
        assert!(panel.mark(index, 1), "the second row could not be marked");
        let after = render_settings(&panel, 1400.0, 900.0, None);
        assert!(boxes(&after, marks[1].1, icons::CHECKED), "the mark is not drawn");
        assert!(boxes(&after, marks[0].1, icons::UNCHECKED), "it marked another row");
        // And the header counts it, which is what says how many a delete would
        // take before the delete is pressed.
        let said = text_of(&after.scene);
        assert!(said.contains("3 SESSIONS, 1 CHOSEN"), "{said}");
        assert!(said.contains("delete 1"), "the button does not say how many: {said}");
    }

    /// The delete asks before it acts: pressed once it says so on the button and
    /// on the footer, and the question names how many would go.
    #[test]
    fn the_delete_under_the_table_says_sure_before_it_takes_the_marked_rows() {
        let mut panel = a_sessions_panel();
        let index = panel
            .rows()
            .iter()
            .position(|row| matches!(row, crate::settings::Row::Table(_)))
            .expect("the section carries a table");
        assert!(panel.mark(index, 0));
        assert!(panel.mark(index, 2));

        let before = render_settings(&panel, 1400.0, 900.0, None);
        assert!(!text_of(&before.scene).contains("sure?"));

        assert_eq!(panel.uninstall(index), None, "the first press deleted them");
        let after = render_settings(&panel, 1400.0, 900.0, None);
        let text = text_of(&after.scene);
        assert!(text.contains("sure?"), "the button does not ask: {text}");
        assert!(
            text.contains("press delete again to remove 2 conversations"),
            "the footer does not say how many: {text}"
        );

        // The box says so with its edge as well, which is what makes it read as
        // armed rather than as a word that changed.
        let box_ = after
            .layout
            .settings_acts
            .iter()
            .find(|(at, act, _)| *at == index && *act == Act::Forget)
            .map(|(_, _, at)| *at)
            .expect("the delete is placed");
        assert!(
            after.scene.rects.iter().any(|rect| {
                let [x, y, ..] = rect.xywh();
                rect.rgba() == after.skin.close_hot
                    && (x - box_.x).abs() < 2.0
                    && (y - box_.y).abs() < 2.0
            }),
            "the armed delete looks exactly like the unarmed one"
        );
    }


    /// The toggle and the uninstall on an entry are pressed where they are
    /// drawn, and neither of them is the row: a press on the name still puts
    /// the cursor there rather than deleting a skill.
    #[test]
    fn an_entry_carries_a_toggle_and_an_uninstall_of_its_own() {
        let panel = a_wordy_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let entries: Vec<usize> = layout
            .settings_rows
            .iter()
            .filter(|(index, _, _)| {
                matches!(panel.row(*index), Some(crate::settings::Row::Entry(_)))
            })
            .map(|(index, _, _)| *index)
            .collect();
        assert_eq!(entries.len(), 2, "the servers are not rows of their own");
        let index = entries[0];
        let row = layout
            .settings_rows
            .iter()
            .find(|(at, _, _)| *at == index)
            .map(|(_, _, row)| *row)
            .expect("the row");

        let toggle = layout
            .settings_toggles
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, at)| *at)
            .expect("a toggle");
        let remove = layout
            .settings_removes
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, at)| *at)
            .expect("an uninstall");
        for (at, hit) in [
            (toggle, Hit::SettingsToggle(index)),
            (remove, Hit::SettingsRemove(index)),
        ] {
            let (x, y) = middle(at);
            assert_eq!(layout.hit(x, y), Some(hit), "{at:?}");
            assert!(row.contains(x, y), "{at:?} is outside its row {row:?}");
        }
        assert!(
            toggle.x + toggle.w <= remove.x,
            "the two controls overlap: {toggle:?} and {remove:?}"
        );
        // The row itself is still the row.
        assert_eq!(
            layout.hit(row.x + 2.0, row.y + 2.0),
            Some(Hit::SettingsRow(index, crate::settings::Side::Left))
        );
        let text = text_of(&out.scene);
        assert!(text.contains("uninstall"), "{text}");
        assert!(text.contains("on"), "{text}");

        // Nothing else on the panel grows one: a setting is not an entry.
        let plain = render_settings(
            &a_panel_on(&Config::default(), crate::settings::APPEARANCE),
            1400.0,
            900.0,
            None,
        );
        assert!(plain.layout.settings_toggles.is_empty());
        assert!(plain.layout.settings_removes.is_empty());
    }

    /// A description long enough that it used to be cut off where the buttons
    /// started, and a document line long enough that it used to be cut off at
    /// the edge of its column.
    const A_LONG_ABOUT: &str =
        "reads the file before it writes one, and says which file it read";
    const A_LONG_DOC_LINE: &str = "the whole point of a column beside the list is that a sentence written in it can be read all the way to the end of itself rather than stopping in three dots";

    /// An agent with the servers a section of entries is asserted on: one with
    /// a description long enough to wrap and a document beside it, and one that
    /// is turned off.
    ///
    /// The entries of the panel are the servers. The skills are a table, and
    /// every rule about an entry (its card, its two buttons, the column beside
    /// it) is asserted here, on the list that has them.
    fn an_agent_with_servers() -> crate::agent::Agent {
        let mut agent = an_agent();
        agent.mcp = crate::agent::Mcp {
            global: Some(std::path::PathBuf::from("/home/hec/.config/noob/mcp.json")),
            project: None,
            any_file: true,
            servers: vec![
                crate::agent::Server {
                    name: String::from("docs"),
                    how: String::from(A_LONG_ABOUT),
                    project: false,
                    on: true,
                    entry: String::from(A_LONG_DOC_LINE),
                },
                crate::agent::Server {
                    name: String::from("shell"),
                    how: String::from("http://localhost:9001/mcp"),
                    project: false,
                    on: false,
                    entry: String::from("{ \"url\": \"http://localhost:9001/mcp\" }"),
                },
            ],
            trouble: Vec::new(),
        };
        agent
    }

    fn a_wordy_servers_panel() -> Settings {
        let mut panel = Settings::open(
            &Config::default(),
            Some(std::path::Path::new("/home/hec/.config/noob/no0b.conf")),
            an_agent_with_servers(),
        );
        let at = panel
            .section_names()
            .iter()
            .position(|name| *name == crate::settings::MCP)
            .expect("MCP is a section");
        panel.choose(at);
        panel
    }


    /// The one entry row, and the entry itself, out of a rendered skills panel.
    fn the_entry_row(out: &Rendered, panel: &Settings) -> (usize, Panel) {
        let (index, _, row) = *out
            .layout
            .settings_rows
            .iter()
            .find(|(index, _, _)| {
                matches!(panel.row(*index), Some(crate::settings::Row::Entry(_)))
            })
            .expect("an entry row");
        (index, row)
    }

    /// The one card row of a rendered panel, and where it is.
    fn the_card_row(out: &Rendered, panel: &Settings) -> (usize, Panel) {
        let (index, _, row) = *out
            .layout
            .settings_rows
            .iter()
            .find(|(index, _, _)| matches!(panel.row(*index), Some(crate::settings::Row::Card(_))))
            .expect("a card row");
        (index, row)
    }

    /// Where the card drawn in a row has its title, its divider, its body and
    /// its footer, worked out through the same two functions the placement and
    /// the drawing go through.
    fn the_card(_out: &Rendered, row: Panel, footer: bool) -> (Panel, CardParts) {
        let line = Text::line_for(PANE_TEXT.0);
        // The row's own width, which is half the list for a card standing
        // beside another one, and the whole of it otherwise.
        let cols = settings_entry_cols(row.w, PANE_TEXT.1);
        let card = settings_card(row, line);
        let parts = settings_card_parts(card, line, PANE_TEXT.0, PANE_TEXT.1, cols, footer);
        (card, parts)
    }

    /// An entry is a card: its name in the header, what it is for in the body,
    /// where it came from under that, and its buttons in the footer.
    ///
    /// The name and the description used to share one line, with the
    /// description cut off wherever the buttons at the end of the row began.
    /// Then they were three bare lines with the buttons still beside the name.
    /// Now the three strings are three roles in three places and the buttons
    /// have a strip of their own.
    #[test]
    fn an_entry_is_a_card_with_its_name_its_words_and_its_path_in_it() {
        let panel = a_wordy_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let (index, row) = the_entry_row(&out, &panel);
        let (card, parts) = the_card(&out, row, true);
        assert!(
            card.h <= row.h - design::apart(line) + 0.01,
            "the card fills the space between itself and the next one"
        );

        // The name in the header, at the card title size rather than the size
        // everything under it is drawn at.
        let title = out
            .scene
            .texts
            .iter()
            .find(|text| (text.at.y - parts.title.y).abs() < 0.51)
            .expect("the title");
        assert_eq!(
            title.runs.iter().map(|run| run.text.as_str()).collect::<String>(),
            "docs"
        );
        assert_eq!(title.size, design::card_title_size(PANE_TEXT.0));
        assert!(title.size > PANE_TEXT.0, "the title is not bigger than a value");

        // What it is for, in the body, at the value size.
        assert_eq!(line_of(&out, parts.body.x, parts.body.y), A_LONG_ABOUT);
        // And where it came from under it, in the hint size: the quietest of
        // the three, and the one that used to look exactly like the other two.
        let wrapped = crate::settings::about_rows(
            A_LONG_ABOUT,
            design::card_cols(settings_entry_cols(row.w, PANE_TEXT.1)),
        );
        let under = out
            .scene
            .texts
            .iter()
            .find(|text| {
                (text.at.x - parts.body.x).abs() < 0.51
                    && (text.at.y
                        - (parts.body.y + wrapped as f32 * line + design::tight(line)))
                    .abs()
                        < 0.51
            })
            .expect("the path");
        assert_eq!(
            under.runs.iter().map(|run| run.text.as_str()).collect::<String>(),
            "global: /home/hec/.config/noob/mcp.json"
        );
        assert_eq!(under.size, design::hint_size(PANE_TEXT.0));
        assert!(under.size < PANE_TEXT.0, "the path is not quieter than the words");

        // The buttons are in the footer, at the bottom of the card, and the
        // description runs the full width of the body under them rather than
        // stopping where they begin.
        let toggle = out
            .layout
            .settings_toggles
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, at)| *at)
            .expect("a toggle");
        assert!(toggle.y >= parts.body.y + parts.body.h - 0.01, "{toggle:?}");
        let beside_the_name = ((toggle.x - 8.0 - parts.body.x) / 8.0).floor() as usize;
        assert!(
            A_LONG_ABOUT.chars().count() > beside_the_name,
            "the description would have fitted beside the buttons, so this proves nothing"
        );
        // And nothing an entry says is drawn over the document beside it.
        for text in out.scene.texts.iter().filter(|text| {
            (text.at.x - parts.body.x).abs() < 0.51 && text.at.y >= card.y && text.at.y <= card.y + card.h
        }) {
            assert!(
                text.at.x + text.at.w <= out.layout.settings_doc.x + 0.01,
                "a line of the entry runs into the document: {:?}",
                text.at
            );
        }
    }

    /// The footer's buttons are inside the card, at the bottom of it, and in the
    /// three kinds this window has and no others.
    ///
    /// "buttons, default bottom always, they are messy": they were pinned to the
    /// right of whichever line of a row they happened to belong to, so no two of
    /// them were at the same height and each one sat wherever its own row left
    /// space.
    #[test]
    fn the_buttons_of_a_card_stand_in_its_footer() {
        let panel = a_wordy_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let (index, row) = the_entry_row(&out, &panel);
        let (card, parts) = the_card(&out, row, true);
        let toggle = out
            .layout
            .settings_toggles
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, at)| *at)
            .expect("a toggle");
        let remove = out
            .layout
            .settings_removes
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, at)| *at)
            .expect("an uninstall");
        for (name, at) in [("the toggle", toggle), ("the uninstall", remove)] {
            assert!(
                at.y >= parts.footer.y - 0.01 && at.y + at.h <= card.y + card.h - 0.01,
                "{name} is not in the footer: {at:?} against {:?}",
                parts.footer
            );
            assert!(
                at.y + at.h >= parts.body.y + parts.body.h - 0.01,
                "{name} is not under the body: {at:?}"
            );
            assert!(
                at.x >= card.x && at.x + at.w <= card.x + card.w + 0.01,
                "{name} is outside the card: {at:?} in {card:?}"
            );
            // And pressed where it is drawn.
            let (x, y) = middle(at);
            assert!(card.contains(x, y), "{name} is drawn off its own card");
        }
        assert!(
            toggle.x + toggle.w <= remove.x,
            "the two buttons overlap: {toggle:?} and {remove:?}"
        );
        // Room under the last line of the body, so the buttons are not standing
        // on the words.
        assert!(
            parts.footer.y >= parts.body.y + parts.body.h + design::room(line) - 0.01,
            "the footer sits on the body: {:?} under {:?}",
            parts.footer,
            parts.body
        );

        // The primary is filled, the danger is outlined in the bad colour, and
        // neither of them is the other.
        let over = |at: Panel, rgba: [f32; 4]| {
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == rgba
                    && (x - at.x).abs() < 0.01
                    && (y - at.y).abs() < 0.01
                    && (w - at.w).abs() < 0.01
                    && (h - at.h).abs() < 0.01
            })
        };
        assert!(
            over(toggle, out.skin.button),
            "the skill is on, so its toggle is the filled button"
        );
        assert!(
            over(remove, out.skin.close_hot),
            "the uninstall is not outlined in the colour this window loses work in"
        );
        assert!(!over(remove, out.skin.button), "the uninstall is filled like a primary");
        assert!(text_of(&out.scene).contains("uninstall"));
    }

    /// A description several times the width of the entry column. Giving it a
    /// line of its own took the buttons out of its way; it still ended in an
    /// ellipsis at the edge of the column, which is what this one is long
    /// enough to prove.
    const A_WRAPPING_ABOUT: &str = "reads the file before it writes one, says which file it read, and stops at the first thing it does not recognise rather than guessing what the rest of the line was meant to say";

    /// The skills section with two skills, the first wordy enough that its
    /// description cannot be one row of the column and the second short enough
    /// that it is.
    fn a_wrapping_skills_panel() -> Settings {
        let mut agent = an_agent_with_servers();
        agent.mcp.servers[0].how = String::from(A_WRAPPING_ABOUT);
        let mut panel = a_wordy_servers_panel();
        panel.adopt_agent(agent, &Config::default());
        panel
    }

    /// Every entry row of a rendered skills panel, in the order they are drawn.
    fn the_entry_rows(out: &Rendered, panel: &Settings) -> Vec<(usize, Panel)> {
        out.layout
            .settings_rows
            .iter()
            .filter(|(index, _, _)| {
                matches!(panel.row(*index), Some(crate::settings::Row::Entry(_)))
            })
            .map(|(index, _, row)| (*index, *row))
            .collect()
    }

    /// A description too long for the card wraps onto as many rows as it needs
    /// and the card grows to hold them.
    ///
    /// Moving it off the name's line stopped the buttons cutting it; the column
    /// itself was still cutting it, so a skill whose description ran past the
    /// width of the list ended in three dots with the rest unreadable. It is
    /// broken by the rule the panes and the document column use, in the columns
    /// the model counted its height in, which are the card's own and not the
    /// list's.
    #[test]
    fn a_long_description_wraps_instead_of_ending_in_an_ellipsis() {
        let panel = a_wrapping_skills_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let (_, row) = the_entry_rows(&out, &panel)[0];
        let cols = design::card_cols(settings_entry_cols(row.w, PANE_TEXT.1));
        assert!(
            A_WRAPPING_ABOUT.chars().count() > cols,
            "the description fits in {cols} columns, so this proves nothing"
        );
        let (_, parts) = the_card(&out, row, true);
        let drawn = out
            .scene
            .texts
            .iter()
            .find(|text| {
                (text.at.x - parts.body.x).abs() < 0.51 && (text.at.y - parts.body.y).abs() < 0.51
            })
            .expect("the description");
        let said: String = drawn.runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(said, A_WRAPPING_ABOUT, "the description was cut");
        assert!(!said.contains('\u{2026}'), "it still ends in an ellipsis");
        assert_eq!(
            drawn.wrap_cols,
            Some(cols),
            "it wraps in the columns its height was counted in"
        );
        // As many rows as it wrapped to, and the card is that body, its header
        // and its footer.
        let wrapped = crate::settings::about_rows(A_WRAPPING_ABOUT, cols);
        assert!(wrapped > 1, "{wrapped} rows in {cols} columns");
        assert!(
            drawn.at.h >= wrapped as f32 * line - 0.01,
            "the box holds {} of {wrapped} rows: {:?}",
            drawn.at.h / line,
            drawn.at
        );
        assert!(
            parts.body.h >= (wrapped as f32 + 1.0) * line - 0.01,
            "the body did not grow: {:?} at {line}",
            parts.body
        );
        assert_eq!(
            row.h,
            crate::design::card_row_lines(
                crate::settings::entry_body_lines(
                    match panel.row(the_entry_rows(&out, &panel)[0].0) {
                        Some(crate::settings::Row::Entry(entry)) => entry,
                        other => panic!("{other:?}"),
                    },
                    settings_entry_cols(row.w, PANE_TEXT.1)
                ),
                true
            ) as f32
                * line,
            "the row is not the height the model counted"
        );
    }

    /// And the card under it starts below those rows rather than over them.
    #[test]
    fn the_entry_under_a_wrapped_one_is_drawn_below_its_rows() {
        let panel = a_wrapping_skills_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let rows = the_entry_rows(&out, &panel);
        let cols = design::card_cols(settings_entry_cols(rows[0].1.w, PANE_TEXT.1));
        let wrapped = crate::settings::about_rows(A_WRAPPING_ABOUT, cols);
        assert_eq!(rows.len(), 2, "two servers are two cards");
        let ((_, first), (_, second)) = (rows[0], rows[1]);
        assert!(
            second.y >= first.y + first.h - 0.01,
            "the rows overlap: {first:?} then {second:?}"
        );
        let (first_card, first_parts) = the_card(&out, first, true);
        let (_, second_parts) = the_card(&out, second, true);
        // The path sits under the last row of the description, not on top of it.
        assert_eq!(
            line_of(
                &out,
                first_parts.body.x,
                first_parts.body.y + wrapped as f32 * line + design::tight(line)
            ),
            "global: /home/hec/.config/noob/mcp.json"
        );
        // And the next name is under the whole of the first card, with the
        // space between two cards left between them.
        assert_eq!(
            line_of(&out, second_parts.title.x, second_parts.title.y),
            "shell"
        );
        assert!(
            second.y >= first_card.y + first_card.h + design::apart(line) - 1.01,
            "the second card is drawn over the first: {second:?} under {first_card:?}"
        );
    }

    /// A press on the entry after a wrapped one lands on that entry.
    ///
    /// The height of a row is read by the layout and by the scroll window
    /// alike, so a description counted at one row and drawn as four would put
    /// every press below it on its neighbour.
    #[test]
    fn a_press_below_a_wrapped_entry_lands_on_the_entry_under_it() {
        let panel = a_wrapping_skills_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let rows = the_entry_rows(&out, &panel);
        assert_eq!(rows.len(), 2, "two servers are two rows");
        let named = |index: usize| match panel.row(index) {
            Some(crate::settings::Row::Entry(entry)) => entry.name.clone(),
            other => panic!("row {index} is {other:?}"),
        };
        assert_eq!(named(rows[0].0), "docs");
        assert_eq!(named(rows[1].0), "shell");
        for (index, row) in rows {
            let (x, y) = middle(row);
            assert_eq!(
                out.layout.hit(x, y),
                Some(Hit::SettingsRow(index, crate::settings::Side::Left)),
                "the middle of {} answers for another row",
                named(index)
            );
            // Its own last line as well, which is the press a row measured
            // short hands to the entry below it.
            assert_eq!(
                out.layout.hit(x, row.y + row.h - 1.0),
                Some(Hit::SettingsRow(index, crate::settings::Side::Left)),
                "the last line of {} answers for another row",
                named(index)
            );
        }
    }

    /// The uninstall is a button with a word in it, and both of them fit: it
    /// ended exactly on the edge of the column, three pixels from the document's
    /// own border, in a box a column and a half wider than the word it holds, so
    /// the shaper's bounds took the last letter off it.
    #[test]
    fn the_uninstall_sits_inside_the_row_with_room_for_its_word() {
        let panel = a_wordy_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (index, row) = the_entry_row(&out, &panel);
        let remove = out
            .layout
            .settings_removes
            .iter()
            .find(|(at, _)| *at == index)
            .map(|(_, at)| *at)
            .expect("an uninstall");
        let (card, _) = the_card(&out, row, true);
        assert!(
            remove.x + remove.w
                <= card.x + card.w - design::room(Text::line_for(PANE_TEXT.0)) + 0.01,
            "the uninstall is against the edge of the card: {remove:?} in {card:?}"
        );
        assert!(
            remove.x + remove.w <= out.layout.settings_doc.x,
            "it reaches into the document: {remove:?}"
        );
        // The word inside it is written in the box the button leaves after its
        // own padding, and that box holds every letter of it.
        let word = "uninstall";
        let room = remove.w - INPUT_PAD * 2.0;
        assert!(
            room >= word.chars().count() as f32 * 8.0,
            "{word} needs {} pixels and has {room}",
            word.chars().count() as f32 * 8.0
        );
        assert!(text_of(&out.scene).contains(word));
    }

    /// The card the cursor is on carries the focus colour on its own border and
    /// the mark down its edge.
    ///
    /// It used to wear a solid band across the row. A row was one line tall then;
    /// a card is nine, and a filled block that tall is a highlight nobody can
    /// read through. The border says the same thing and leaves the words alone.
    #[test]
    fn the_entry_under_the_cursor_is_the_card_with_the_focus_border() {
        let mut panel = a_wordy_servers_panel();
        // The section opens on the install form, so the cursor is walked down
        // to the entry this test is about.
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (index, _) = the_entry_row(&out, &panel);
        panel.point_at(index, crate::settings::Side::Left);
        assert_eq!(panel.cursor(), index, "the entry cannot hold the cursor");
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (index, row) = the_entry_row(&out, &panel);
        let (card, _) = the_card(&out, row, true);
        assert_eq!(panel.cursor(), index, "the cursor is not on the entry");
        assert!(
            covered(&out, card, card.h, out.skin.edge_focus),
            "the card is not outlined in the focus colour: {card:?}"
        );
        assert!(
            covered(&out, Panel::new(card.x, card.y, MARK_W, card.h), card.h, out.skin.edge_focus),
            "no mark down the edge of the card the keys are on"
        );
        assert!(
            !covered(&out, row, row.h, out.skin.picked),
            "the band across the whole card is back"
        );
        // And the card that is not under the cursor keeps the quiet border, so
        // the two are told apart.
        let panel = a_wrapping_skills_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let rows = the_entry_rows(&out, &panel);
        let (quiet, _) = the_card(&out, rows[1].1, true);
        assert!(
            covered(&out, quiet, quiet.h, out.skin.edge),
            "the card the keys are not on is outlined in the focus colour too"
        );
    }

    /// A panel on MCP with one server configured, which the plain fixture has
    /// none of.
    fn a_servers_panel() -> Settings {
        let mut agent = an_agent();
        agent.mcp = crate::agent::Mcp {
            global: Some(std::path::PathBuf::from("/home/hec/.config/noob/mcp.json")),
            project: None,
            any_file: true,
            servers: vec![crate::agent::Server {
                name: String::from("deepwiki"),
                how: String::from("https://mcp.deepwiki.com/mcp"),
                project: false,
                on: true,
                entry: String::from("{ \"url\": \"https://mcp.deepwiki.com/mcp\" }"),
            }],
            trouble: Vec::new(),
        };
        let mut panel = Settings::open(
            &Config::default(),
            Some(std::path::Path::new("/home/hec/.config/noob/no0b.conf")),
            agent,
        );
        let at = panel
            .section_names()
            .iter()
            .position(|name| *name == crate::settings::MCP)
            .expect("MCP is a section");
        panel.choose(at);
        panel
    }

    /// A server's three strings are three roles too, at three sizes, the way a
    /// skill's are: the two lists are drawn by the same arm, and a server is
    /// the worse of the two to read when they all look alike, because its
    /// address and the file it came out of are both paths.
    #[test]
    fn a_server_says_its_name_its_address_and_its_file_in_three_roles() {
        let panel = a_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let (_, row) = the_entry_row(&out, &panel);
        let (_, parts) = the_card(&out, row, true);

        let title = out
            .scene
            .texts
            .iter()
            .find(|text| (text.at.y - parts.title.y).abs() < 0.51)
            .expect("the name");
        assert_eq!(
            title.runs.iter().map(|run| run.text.as_str()).collect::<String>(),
            "deepwiki"
        );
        assert_eq!(title.size, design::card_title_size(PANE_TEXT.0));
        // The address in the body at the value size, and the file it is
        // configured in under it at the hint size.
        assert_eq!(
            line_of(&out, parts.body.x, parts.body.y),
            "https://mcp.deepwiki.com/mcp"
        );
        let under = out
            .scene
            .texts
            .iter()
            .find(|text| {
                (text.at.x - parts.body.x).abs() < 0.51
                    && (text.at.y - (parts.body.y + line + design::tight(line))).abs() < 0.51
            })
            .expect("the file it came from");
        assert!(
            under
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                .contains("mcp.json"),
            "{:?}",
            under.runs
        );
        assert!(under.size < PANE_TEXT.0);
        assert!(title.size > under.size);
    }

    /// The install card's button: filled, in its own footer, at the bottom
    /// right of the card, and pressed where it is drawn.
    #[test]
    fn the_install_button_is_filled_and_stands_in_the_card_s_footer() {
        let panel = a_panel_on(&Config::default(), crate::settings::SKILLS);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (index, row) = the_card_row(&out, &panel);
        let (card, parts) = the_card(&out, row, true);
        let (_, act, box_) = *out
            .layout
            .settings_acts
            .iter()
            .find(|(at, ..)| *at == index)
            .expect("the install card has no button");
        assert_eq!(act, Act::Validate, "an unchecked source validates first");

        // In the footer, at the bottom right, and inside its own card.
        assert!(
            (box_.y - parts.footer.y).abs() < 0.01,
            "{box_:?} is not on the footer {:?}",
            parts.footer
        );
        assert!(box_.y >= parts.body.y + parts.body.h - 0.01, "over the body");
        assert!(box_.x + box_.w <= parts.footer.x + parts.footer.w + 0.01);
        assert!(box_.x > parts.footer.x + parts.footer.w * 0.5, "not at the right");
        assert!(box_.y + box_.h <= card.y + card.h + 0.01, "outside the card");

        // Filled, which is what a primary is, and holding the word.
        assert!(
            covered(&out, box_, box_.h, out.skin.button),
            "the install button is not filled: {box_:?}"
        );
        assert!(
            line_of(&out, box_.x + INPUT_PAD, box_.y).contains("validate"),
            "the button has no word in it"
        );

        // And a press inside it is that button and not the row under it.
        let (x, y) = (box_.x + box_.w * 0.5, box_.y + box_.h * 0.5);
        assert_eq!(
            out.layout.hit(x, y),
            Some(Hit::SettingsAct(index, Act::Validate))
        );
    }

    /// The column beside the list is a card of its own, and the text in it
    /// wraps. Every line of it used to be cut to the width of the column and
    /// ended in an ellipsis, which is the left edge of a document rather than a
    /// document.
    ///
    /// This asserted a bare line of text over a filled, outlined box. That is
    /// the shape everything on this panel was taken out of: the column is a
    /// card now, with the skill's name in its header at the card title size,
    /// one divider under it and the text in its body.
    #[test]
    fn the_document_is_a_card_whose_header_names_the_skill() {
        let panel = a_wordy_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let line = Text::line_for(13.0);
        let doc = layout.settings_doc;
        assert!(doc.w >= 1.0, "there is no second column");

        let box_ = settings_doc_box(doc, line);
        let parts = settings_doc_parts(box_, line, PANE_TEXT.0);
        assert!(box_.y >= doc.y - 0.01, "the card is not the whole column");
        assert!(box_.y + box_.h <= doc.y + doc.h + 0.01);

        // The name in the header, at the card title size, the way an entry's own
        // card carries its name.
        let title = out
            .scene
            .texts
            .iter()
            .find(|text| {
                (text.at.x - parts.title.x).abs() < 0.51
                    && (text.at.y - parts.title.y).abs() < 0.51
            })
            .expect("the header");
        assert_eq!(
            title.runs.iter().map(|run| run.text.as_str()).collect::<String>(),
            "docs"
        );
        assert_eq!(title.size, design::card_title_size(PANE_TEXT.0));

        // The border, stroked and cut like every other card, and the one
        // divider under the header.
        let border = out
            .scene
            .rects
            .iter()
            .find(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == out.skin.edge
                    && (x - box_.x).abs() < 0.01
                    && (y - box_.y).abs() < 0.01
                    && (w - box_.w).abs() < 0.01
                    && (h - box_.h).abs() < 0.01
            })
            .expect("the document card has no border");
        assert!(border.extra()[1] > 0.0, "the border is filled, not stroked");
        assert!(
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == out.skin.edge
                    && h <= 1.01
                    && (x - parts.rule.x).abs() < 0.01
                    && (w - parts.rule.w).abs() < 0.01
                    && (y - parts.rule.y).abs() < 1.01
            }),
            "no divider under the document's header"
        );

        // The text is in the body, off the border on every side.
        let inside = layout.settings_doc_text;
        assert_eq!((inside.x, inside.y), (parts.body.x, parts.body.y));
        assert!(inside.x >= box_.x + design::room(line) - 0.01);
        assert!(inside.y >= parts.rule.y + design::room(line) - 0.01);
        assert!(inside.x + inside.w <= box_.x + box_.w + 0.01);
        assert!(inside.y + inside.h <= box_.y + box_.h - design::room(line) + 0.01);
        assert!(inside.x > layout.settings_list.x);

        // The text wraps at the columns the box holds, by the same rule the
        // panes wrap at, and the long line is written whole.
        let cols = layout.settings_doc_columns(8.0);
        assert!(cols > 0);
        assert!(
            A_LONG_DOC_LINE.chars().count() > cols,
            "the line fits, so this proves nothing"
        );
        let text = out
            .scene
            .texts
            .iter()
            .find(|text| {
                (text.at.x - inside.x).abs() < 0.01 && (text.at.y - inside.y).abs() < 0.01
            })
            .expect("the document");
        assert_eq!(text.wrap_cols, Some(cols));
        assert_eq!(text.wrap_break, text_geometry::Break::Word);
        let written: String = text.runs.iter().map(|run| run.text.as_str()).collect();
        assert!(
            written.contains(A_LONG_DOC_LINE),
            "the line is not written whole: {written:?}"
        );
        assert!(!written.contains('\u{2026}'), "it is still clipped: {written:?}");
    }

    /// Both columns scroll, each in its own box: the document is drawn from
    /// wherever it was scrolled to, in rows of the box it is drawn in, and the
    /// list stays where it was.
    #[test]
    fn the_document_scrolls_inside_its_own_box() {
        let mut panel = a_wordy_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let cols = out.layout.settings_doc_columns(8.0);
        let rows = out.layout.settings_doc_rows(13.0);
        assert!(rows > 1, "the box holds more than a line");

        // A document longer than any window, scrolled past its first screenful.
        let mut agent = an_agent_with_servers();
        agent.mcp.servers[0].entry = (0..200)
            .map(|n| format!("line {n} of it"))
            .collect::<Vec<String>>()
            .join("\n");
        panel.adopt_agent(agent, &Config::default());
        let before = render_settings(&panel, 1400.0, 900.0, None);
        let first_row = before.layout.settings_rows[0];
        assert!(panel.scroll_doc(3, true, cols, rows), "the wheel moves it");
        let after = render_settings(&panel, 1400.0, 900.0, None);
        let inside = after.layout.settings_doc_text;
        let written: String = after
            .scene
            .texts
            .iter()
            .filter(|text| {
                (text.at.x - inside.x).abs() < 0.01 && (text.at.y - inside.y).abs() < 0.01
            })
            .flat_map(|text| text.runs.iter())
            .map(|run| run.text.as_str())
            .collect();
        assert!(written.contains("line 3 of it"), "{written:?}");
        assert!(!written.contains("line 0 of it"), "{written:?}");
        // The list did not move with it: the two columns are two scrolls.
        assert_eq!(after.layout.settings_rows[0], first_row);

        // And the bar that says how far down it is: inside the card, down its
        // right padding, and beside the body rather than beside the header,
        // which is not part of what scrolls.
        let line = Text::line_for(PANE_TEXT.0);
        let box_ = settings_doc_box(after.layout.settings_doc, line);
        let parts = settings_doc_parts(box_, line, PANE_TEXT.0);
        let track = after
            .scene
            .rects
            .iter()
            .find(|rect| rect.rgba() == after.skin.scroll_track)
            .map(|rect| rect.xywh())
            .expect("the document has no bar");
        assert!(track[0] > inside.x + inside.w, "the bar is over the text");
        assert!(track[0] + track[2] <= box_.x + box_.w, "outside the card");
        assert!(track[1] >= parts.body.y, "the bar reaches up past the header");
    }


    /// Whether two rectangles share a pixel.
    fn overlap(a: Panel, b: Panel) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }


    /// The SKILLS section with an install answer long enough that its block
    /// scrolls, over a list long enough that the panel scrolls too, wound to
    /// the end so the block is among the rows on screen.
    fn a_wordy_install_panel() -> Settings {
        let mut agent = an_agent();
        agent.skills[0].about = String::from(A_LONG_ABOUT);
        agent.skills[0].doc = vec![String::from(A_LONG_DOC_LINE)];
        for extra in 1..6 {
            let mut skill = agent.skills[0].clone();
            skill.name = format!("skill{extra}");
            skill.dir = format!("skill{extra}");
            agent.skills.push(skill);
        }
        let mut panel = Settings::open(
            &Config::default(),
            Some(std::path::Path::new("/home/hec/.config/noob/no0b.conf")),
            agent.clone(),
        );
        let at = panel
            .section_names()
            .iter()
            .position(|name| *name == crate::settings::SKILLS)
            .expect("SKILLS is a section");
        panel.choose(at);
        panel.begin_install(String::from("owner/skill"), &Config::default());
        panel.adopt_install(
            String::from("owner/skill"),
            Err((0..200)
                .map(|at| format!("clone line {at}"))
                .collect::<Vec<String>>()
                .join("\n")),
            agent,
            &Config::default(),
        );
        scrolled_to_the_end(&mut panel);
        panel
    }

    /// The saved conversations, more of them than the table's body holds.
    fn a_long_sessions_panel() -> Settings {
        let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(9_000_000);
        let sessions: Vec<crate::sessions::Saved> = (0..crate::settings::TABLE_ROWS * 3)
            .map(|at| crate::sessions::Saved {
                id: format!("id{at}"),
                when: now - std::time::Duration::from_secs(60 * (at as u64 + 1)),
                workspace: Some(std::path::PathBuf::from("/home/hec/workspace/noob-cli")),
                gone: false,
                bytes: 12_000,
                context: None,
                opening: format!("conversation {at}"),
            })
            .collect();
        let mut panel = Settings::open(
            &Config::default(),
            None,
            crate::agent::Agent {
                now,
                sessions: crate::sessions::Listing {
                    sessions,
                    skipped: Vec::new(),
                },
                ..crate::agent::Agent::default()
            },
        );
        let at = panel
            .section_names()
            .iter()
            .position(|name| *name == crate::settings::SESSIONS)
            .expect("the sessions section");
        panel.choose(at);
        panel
    }

    /// Item H6: one bar per region and no two of them in the same pixels.
    ///
    /// The list's bar was painted at the right edge of the list, which is the
    /// right edge of every card in it. It ran through the border of every card,
    /// through the trash button at the end of every saved conversation, and
    /// through the last glyph column of every wrapped description; a rectangle
    /// cannot cover a glyph on this layer, so the letters were drawn on top of
    /// the bar. Now the cards stop short of a gutter that belongs to the bar
    /// and to nothing else.
    #[test]
    fn no_two_scrollbars_on_the_settings_panel_share_a_pixel() {
        let panels = [
            ("INSTALL", a_wordy_install_panel()),
            ("SESSIONS", a_long_sessions_panel()),
            ("SKILLS", a_wordy_servers_panel()),
            (
                "APPEARANCE",
                a_panel_on(&Config::default(), crate::settings::APPEARANCE),
            ),
        ];
        for (name, panel) in &panels {
            for (w, h) in [(1400.0, 900.0), (980.0, 620.0), (760.0, 460.0)] {
                let out = render_settings(panel, w, h, None);
                let (tracks, thumbs) = bars_of(&out);
                for (at, one) in tracks.iter().enumerate() {
                    assert!(
                        within(*one, out.layout.settings),
                        "{name} at {w}x{h}: a bar outside the panel: {one:?}"
                    );
                    for other in tracks.iter().skip(at + 1) {
                        assert!(
                            !overlap(*one, *other),
                            "{name} at {w}x{h}: {one:?} and {other:?} share pixels"
                        );
                    }
                }
                // Every thumb stands in exactly one track: a thumb outside one
                // is a bar drawn somewhere its own track is not.
                for thumb in &thumbs {
                    let held = tracks.iter().filter(|track| within(*thumb, **track)).count();
                    assert_eq!(held, 1, "{name} at {w}x{h}: {thumb:?} is in {held} tracks");
                }
                // And no bar is drawn over a card. The gutter is the list's, and
                // a card's own bar stands in the card's padding, clear of the
                // body its text is written in.
                let line = Text::line_for(PANE_TEXT.0);
                let cols = out.layout.settings_entry_columns(PANE_TEXT.1);
                for (index, _, row) in &out.layout.settings_rows {
                    let card = settings_card(*row, line);
                    let footer = matches!(
                        panel.row(*index),
                        Some(SettingRow::Entry(_) | SettingRow::Table(_))
                    );
                    let parts =
                        settings_card_parts(card, line, PANE_TEXT.0, PANE_TEXT.1, cols, footer);
                    for track in &tracks {
                        assert!(
                            !overlap(*track, parts.body),
                            "{name} at {w}x{h}: {track:?} is over the body of row {index}"
                        );
                    }
                }
            }
        }
    }

    /// The list's own bar stands in the gutter beside the cards rather than in
    /// the last four pixels of them, and it is still the bar that counts the
    /// list: what it reports is the whole section, not one card of it.
    #[test]
    fn the_list_s_bar_stands_in_a_gutter_the_cards_are_kept_out_of() {
        let panel = a_wordy_install_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let list = out.layout.settings_list;
        let cards = settings_list_rows(list);
        let gutter = settings_list_bar(list);
        assert!(cards.w < list.w, "the list reserved no gutter");
        assert!(
            (cards.x + cards.w + GAP - gutter.x).abs() < 0.01,
            "the gutter does not start a gap after the cards: {cards:?} {gutter:?}"
        );
        for (index, _, row) in &out.layout.settings_rows {
            assert!(
                row.x + row.w <= gutter.x - GAP + 0.01,
                "row {index} runs into the gutter: {row:?}"
            );
        }

        // The list is longer than the panel, so it draws a bar, and that bar is
        // in the gutter. It used to start eight pixels below the first row, on a
        // chamfer the list does not have.
        let (tracks, _) = bars_of(&out);
        let track = tracks
            .iter()
            .find(|track| within(**track, gutter))
            .unwrap_or_else(|| panic!("nothing in the gutter: {tracks:?} in {gutter:?}"));
        assert!(
            track.y - list.y <= 3.01,
            "the bar starts {} pixels below the first row",
            track.y - list.y
        );
    }

    /// A block of text scrolls inside its own card, says so with a bar of its
    /// own, and a block that is already all on screen draws none.
    ///
    /// The block had no bar at all. The nearest one belonged to the list, was
    /// drawn immediately to its right, counted the block as the rows its card
    /// claims rather than the hundreds of lines in it, and did not move when the
    /// wheel over the block did. That is the "scroll overlaps the scroll" this
    /// item exists for.
    #[test]
    fn a_block_of_text_carries_its_own_bar_and_a_short_one_carries_none() {
        let mut panel = a_wordy_install_panel();
        // The block is the second row of the section now, and what is on
        // screen is wherever the cursor is: put it on the block this test is
        // about rather than rendering whatever the install left behind.
        while panel.scroll(4, false, 8, 80) {}
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let cols = out.layout.settings_entry_columns(PANE_TEXT.1);
        let (index, _, row) = *out
            .layout
            .settings_rows
            .iter()
            .find(|(index, _, _)| {
                matches!(panel.row(*index), Some(SettingRow::Paper(paper)) if paper.title.contains("INSTALL"))
            })
            .expect("the install block is on screen");
        let card = settings_card(row, line);
        let parts = settings_card_parts(card, line, PANE_TEXT.0, PANE_TEXT.1, cols, false);
        let text = settings_paper_text(&parts, line);
        assert_eq!(
            (text.h / line).round() as usize,
            crate::settings::PAPER_LINES,
            "the block shows fewer lines than the model counted it at"
        );

        // Its bar: inside the card, down the padding right of the body, and only
        // as tall as the text, so its head is not beside a header that does not
        // scroll.
        let (tracks, thumbs) = bars_of(&out);
        let track = *tracks
            .iter()
            .find(|track| within(**track, card))
            .unwrap_or_else(|| panic!("the block at row {index} drew no bar"));
        assert!(track.x >= parts.body.x + parts.body.w, "the bar is over the text");
        assert!((track.y - text.y).abs() < 3.01, "it reaches past the text");
        let thumb = *thumbs
            .iter()
            .find(|thumb| within(**thumb, track))
            .expect("the block's bar has no thumb");
        assert!(
            thumb.h < track.h * 0.5,
            "the thumb says most of a 200 line block is on screen: {thumb:?} in {track:?}"
        );

        // The wheel over the block moves the block and leaves the panel behind
        // it exactly where it was: two scrolls, two bars.
        let was = out.layout.settings_rows[0];
        let list_track = *tracks
            .iter()
            .find(|track| within(**track, settings_list_bar(out.layout.settings_list)))
            .expect("the list drew no bar");
        let list_thumb = *thumbs
            .iter()
            .find(|thumb| within(**thumb, list_track))
            .expect("the list's bar has no thumb");
        assert!(panel.scroll_paper(index, crate::settings::PAPER_LINES, true));
        let after = render_settings(&panel, 1400.0, 900.0, None);
        assert_eq!(after.layout.settings_rows[0], was, "the list moved with it");
        let (after_tracks, after_thumbs) = bars_of(&after);
        let moved = after_thumbs
            .iter()
            .find(|thumb| within(**thumb, track))
            .expect("the block's bar went away");
        assert!(moved.y > thumb.y, "the block's thumb did not move");
        let still = after_thumbs
            .iter()
            .find(|thumb| within(**thumb, list_track))
            .expect("the list's bar went away");
        assert_eq!(still.y, list_thumb.y, "the list's thumb moved with the block");
        assert_eq!(after_tracks.len(), tracks.len(), "a bar came or went");

        // A block that fits its box draws nothing, so a bar here means there is
        // more of it.
        panel.begin_install(String::from("owner/skill"), &Config::default());
        let short = render_settings(&panel, 1400.0, 900.0, None);
        let (tracks, _) = bars_of(&short);
        let (_, row) = *short
            .layout
            .settings_rows
            .iter()
            .find(|(index, _, _)| {
                matches!(panel.row(*index), Some(SettingRow::Paper(paper)) if paper.title.contains("INSTALL"))
            })
            .map(|(index, _, row)| (*index, *row))
            .as_ref()
            .expect("the block is still there");
        let card = settings_card(row, line);
        assert!(
            !tracks.iter().any(|track| within(*track, card)),
            "a block that fits its box drew a bar: {tracks:?}"
        );
    }

    /// The table of saved conversations scrolls inside its own body, so it
    /// carries its own bar too, and the bar stops at the rows: neither the
    /// header naming the columns nor the buttons under them scroll.
    #[test]
    fn the_table_of_conversations_carries_its_own_bar_over_its_rows_only() {
        let panel = a_long_sessions_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let cols = out.layout.settings_entry_columns(PANE_TEXT.1);
        let (index, _, row) = *out
            .layout
            .settings_rows
            .iter()
            .find(|(index, _, _)| matches!(panel.row(*index), Some(SettingRow::Table(_))))
            .expect("the section carries a table");
        let card = settings_card(row, line);
        let parts = settings_card_parts(card, line, PANE_TEXT.0, PANE_TEXT.1, cols, true);
        let (names, boxes) = settings_table_parts(parts.body, line);
        let (tracks, thumbs) = bars_of(&out);
        let track = *tracks
            .iter()
            .find(|track| within(**track, card))
            .unwrap_or_else(|| panic!("the table at row {index} drew no bar"));
        assert!(track.y >= names.y + names.h - 0.01, "the bar counts the header");
        assert!(
            track.y + track.h <= parts.footer.y + 0.01,
            "the bar runs into the footer"
        );
        assert!(track.x >= parts.body.x + parts.body.w, "the bar is over the rows");
        let thumb = *thumbs
            .iter()
            .find(|thumb| within(**thumb, track))
            .expect("the table's bar has no thumb");
        // A third of the list is on screen, so the thumb is about a third of the
        // track: the bar reports the real extent.
        let want = boxes.len() as f32 / (crate::settings::TABLE_ROWS * 3) as f32;
        assert!(
            (thumb.h / track.h - want).abs() < 0.06,
            "the thumb is {} of its track and {want} of the list is on screen",
            thumb.h / track.h
        );

        // Three conversations fit in a body that holds twelve, so that table
        // draws no bar at all.
        let few = render_settings(&a_sessions_panel(), 1400.0, 900.0, None);
        let (_, row) = *few
            .layout
            .settings_rows
            .iter()
            .find(|(index, _, _)| matches!(panel.row(*index), Some(SettingRow::Table(_))))
            .map(|(index, _, row)| (*index, *row))
            .as_ref()
            .expect("the short table is placed");
        let (tracks, _) = bars_of(&few);
        assert!(
            !tracks.iter().any(|track| within(*track, settings_card(row, line))),
            "a table that fits its body drew a bar: {tracks:?}"
        );
    }

    /// The first line is Markdown, so what is on screen is four characters
    /// shorter than what is in the file and a copy measured on the source would
    /// hand back marks that were nowhere.
    const A_MARKED_DOC_LINE: &str = "- **read** a file with `cat`";
    const A_DRAWN_DOC_LINE: &str = "• read a file with cat";

    /// A skill whose document is three short lines, one of them marked up.
    fn a_selectable_skills_panel() -> Settings {
        let mut agent = an_agent();
        agent.skills[0].doc = vec![
            String::from(A_MARKED_DOC_LINE),
            String::from("second line of the document"),
            String::from("third line of the document"),
        ];
        let mut panel = a_panel_on(&Config::default(), crate::settings::SKILLS);
        panel.adopt_agent(agent, &Config::default());
        on_the_installed_skill(&mut panel);
        panel
    }

    /// Put the keys on the first installed skill of the table, which is the row
    /// under the web search the CLI ships with.
    fn on_the_installed_skill(panel: &mut Settings) {
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, crate::settings::Row::Table(_)))
            .expect("the installed table");
        panel.point_at(at, crate::settings::Side::Left);
        panel.step(true);
    }

    /// The pixel in the middle of one drawn cell of the document.
    fn doc_cell(layout: &Layout, row: usize, at: usize) -> (f32, f32) {
        let inside = layout.settings_doc_text;
        let line = Text::line_for(13.0);
        (
            inside.x + at as f32 * 8.0,
            inside.y + row as f32 * line + line * 0.5,
        )
    }

    /// Where a press at that pixel lands in the document.
    fn doc_spot(layout: &Layout, panel: &Settings, row: usize, at: usize) -> crate::select::Spot {
        let (x, y) = doc_cell(layout, row, at);
        crate::spot_in_doc(layout, panel, x, y, 13.0, 8.0).expect("a character under the pointer")
    }

    fn doc_drag(
        layout: &Layout,
        panel: &Settings,
        from: (usize, usize),
        to: (usize, usize),
    ) -> crate::select::Selection {
        let mut selection = crate::select::Selection::new(
            crate::select::Where::SettingsDoc,
            doc_spot(layout, panel, from.0, from.1),
        );
        selection.extend(doc_spot(layout, panel, to.0, to.1));
        selection
    }

    /// The rectangles the band is painted with, in the document's own box.
    fn doc_bands(out: &Rendered) -> Vec<[f32; 4]> {
        let inside = out.layout.settings_doc_text;
        out.scene
            .rects
            .iter()
            .filter(|rect| rect.rgba() == out.skin.select)
            .map(|rect| rect.xywh())
            .filter(|[x, y, _, _]| *x >= inside.x - 0.01 && *y >= inside.y - 0.01)
            .collect()
    }

    /// A drag across the document selects the characters under the pointer, and
    /// what comes off it is what was highlighted: the glyphs, with the Markdown
    /// marks gone the same way they are gone from the screen.
    #[test]
    fn a_drag_across_the_document_selects_the_characters_under_it() {
        let panel = a_selectable_skills_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let cols = layout.settings_doc_columns(8.0);
        assert!(
            cols > A_DRAWN_DOC_LINE.chars().count(),
            "the line has to fit on one row for the columns to be the characters"
        );

        // Two columns into the drawn first line, and six: `read`.
        let word = doc_drag(layout, &panel, (0, 2), (0, 6));
        assert_eq!(word.range().0, crate::select::Spot::new(0, 2));
        assert_eq!(word.range().1, crate::select::Spot::new(0, 6));
        assert_eq!(word.text(&panel.doc_pane()), "read");

        // The whole of that line is the rendering, not the source: no stars, no
        // backticks, and every column of it is reachable.
        let whole = doc_drag(layout, &panel, (0, 0), (0, A_DRAWN_DOC_LINE.chars().count()));
        let copied = whole.text(&panel.doc_pane());
        assert_eq!(copied, A_DRAWN_DOC_LINE);
        assert!(
            !copied.contains('*') && !copied.contains('`'),
            "a marker that is nowhere on screen came back: {copied:?}"
        );

        // Down two rows: one break per line and not one more, and nothing of the
        // last line past where the drag stopped.
        let block = doc_drag(layout, &panel, (0, 0), (2, 5));
        let copied = block.text(&panel.doc_pane());
        assert_eq!(
            copied,
            format!("{A_DRAWN_DOC_LINE}\nsecond line of the document\nthird")
        );
        assert_eq!(copied.matches('\n').count(), 2, "a break was doubled: {copied:?}");

        // A press that never moved is not a selection, so it cannot swallow the
        // next copy.
        let click =
            crate::select::Selection::new(crate::select::Where::SettingsDoc, doc_spot(layout, &panel, 1, 3));
        assert!(click.is_empty());
        assert_eq!(click.text(&panel.doc_pane()), "");
    }

    /// The band covers what is selected and nothing else: it starts at the
    /// column the drag started on and is as wide as the run.
    #[test]
    fn the_band_covers_what_the_document_drag_selected() {
        let panel = a_selectable_skills_panel();
        let layout = render_settings(&panel, 1400.0, 900.0, None).layout;
        let word = doc_drag(&layout, &panel, (0, 2), (0, 6));
        let out = render_settings_selecting(&panel, 1400.0, 900.0, word);
        let inside = out.layout.settings_doc_text;
        let line = Text::line_for(13.0);

        let bands = doc_bands(&out);
        assert_eq!(bands.len(), 1, "one run is one rectangle: {bands:?}");
        let [x, y, w, h] = bands[0];
        assert!((x - (inside.x + 2.0 * 8.0)).abs() < 0.01, "{bands:?}");
        assert!((w - 4.0 * 8.0).abs() < 0.01, "the band is not four columns: {bands:?}");
        assert!((y - inside.y).abs() < 0.01, "{bands:?}");
        assert!((h - line).abs() < 0.01, "{bands:?}");

        // Nothing highlighted paints nothing at all.
        let none = render_settings(&panel, 1400.0, 900.0, None);
        assert!(doc_bands(&none).is_empty());

        // Three lines are three rectangles, one per line, because the first and
        // the last stop partway along.
        let block = doc_drag(&layout, &panel, (0, 4), (2, 5));
        let out = render_settings_selecting(&panel, 1400.0, 900.0, block);
        let bands = doc_bands(&out);
        assert_eq!(bands.len(), 3, "{bands:?}");
        assert!((bands[0][0] - (inside.x + 4.0 * 8.0)).abs() < 0.01, "{bands:?}");
        assert!((bands[2][0] - inside.x).abs() < 0.01, "the last line starts at the left");
        assert!((bands[2][2] - 5.0 * 8.0).abs() < 0.01, "{bands:?}");
        for (step, band) in bands.iter().enumerate() {
            assert!(
                (band[1] - (inside.y + step as f32 * line)).abs() < 0.01,
                "row {step} is not where it is drawn: {bands:?}"
            );
        }
    }

    /// A selection is made of line numbers, so scrolling the column moves the
    /// band with the text and copies the same characters.
    #[test]
    fn a_document_selection_survives_a_scroll_of_the_column() {
        let mut panel = a_selectable_skills_panel();
        let mut agent = an_agent();
        agent.skills[0].doc = (0..200).map(|n| format!("line {n} of it")).collect();
        panel.adopt_agent(agent, &Config::default());
        on_the_installed_skill(&mut panel);
        let layout = render_settings(&panel, 1400.0, 900.0, None).layout;
        let cols = layout.settings_doc_columns(8.0);
        let rows = layout.settings_doc_rows(13.0);
        assert!(rows > 6, "the box has to hold the rows this drags across");

        // `line 5` on the sixth row, which is where the sixth line is drawn.
        let selection = doc_drag(&layout, &panel, (5, 0), (5, 6));
        assert_eq!(selection.text(&panel.doc_pane()), "line 5");

        // Three rows up. The same characters come off it, and the band is three
        // rows higher up the box.
        let before = doc_bands(&render_settings_selecting(&panel, 1400.0, 900.0, selection));
        assert!(panel.scroll_doc(3, true, cols, rows), "the wheel moves it");
        let after_out = render_settings_selecting(&panel, 1400.0, 900.0, selection);
        assert_eq!(
            selection.text(&panel.doc_pane()),
            "line 5",
            "the selection came to mean another line"
        );
        let after = doc_bands(&after_out);
        assert_eq!(before.len(), 1, "{before:?}");
        assert_eq!(after.len(), 1, "{after:?}");
        assert!(
            (before[0][1] - after[0][1] - 3.0 * Text::line_for(13.0)).abs() < 0.01,
            "the band did not move with the text: {before:?} then {after:?}"
        );
        assert!((before[0][0] - after[0][0]).abs() < 0.01);
        assert!((before[0][2] - after[0][2]).abs() < 0.01);

        // And the pointer over that row now lands on the line that is drawn
        // there, which is three further down the document.
        assert_eq!(doc_spot(&layout, &panel, 5, 0), crate::select::Spot::new(8, 0));
    }

    /// The pane the selection is resolved in is the text the column draws: the
    /// same lines, wrapped into the same rows, or a band would be over glyphs
    /// the clipboard does not have.
    #[test]
    fn the_document_pane_holds_what_the_column_draws() {
        let panel = a_wordy_servers_panel();
        let layout = render_settings(&panel, 1400.0, 900.0, None).layout;
        let cols = layout.settings_doc_columns(8.0);
        let rows = layout.settings_doc_rows(13.0);
        let pane = panel.doc_pane_at(cols, rows);
        assert_eq!(pane.window(rows, cols), panel.doc_window(cols, rows));
        let heights = panel.doc_heights(cols);
        assert!(
            heights.iter().any(|tall| *tall > 1),
            "no line wraps, so this proves nothing"
        );
        for (line, tall) in heights.iter().enumerate() {
            assert_eq!(pane.rows_of_line(line, cols).len(), *tall, "line {line}");
        }
    }

    /// The document is the one thing on the panel a menu can act on, and the
    /// row it offers is greyed until something is highlighted.
    #[test]
    fn the_document_offers_a_copy_on_the_right_button() {
        let panel = a_selectable_skills_panel();
        let layout = render_settings(&panel, 1400.0, 900.0, None).layout;
        let (x, y) = doc_cell(&layout, 0, 3);
        assert_eq!(layout.hit(x, y), Some(Hit::SettingsDoc));

        let empty = crate::menu::Menu::for_settings_doc((x, y), false);
        assert_eq!(empty.target, crate::menu::Target::SettingsDoc);
        assert_eq!(empty.rows.len(), 1);
        assert_eq!(empty.rows[0].item, crate::menu::Item::CopySelection);
        assert!(!empty.rows[0].enabled, "it copies nothing and says so");

        let held = crate::menu::Menu::for_settings_doc((x, y), true);
        assert_eq!(held.rows.len(), empty.rows.len(), "the menu is the same shape");
        assert!(held.rows[0].enabled);
    }

    /// Every section is on the rail, is hit where its name is drawn, and picking
    /// one swaps what is beside it.
    #[test]
    fn every_section_is_on_the_rail_and_can_be_pressed() {
        let mut panel = a_settings_panel(&Config::default());
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        assert_eq!(
            layout.settings_rail.len(),
            panel.section_names().len(),
            "the rail is short of a section"
        );
        for (index, at) in &layout.settings_rail {
            let (x, y) = middle(*at);
            assert_eq!(layout.hit(x, y), Some(Hit::SettingsSection(*index)));
            assert!(
                at.x + at.w <= layout.settings_list.x,
                "the rail runs into the list: {at:?}"
            );
        }
        // Every name is written, and the chosen one is in the accent green while
        // the rest are not.
        let text = text_of(&out.scene);
        for name in panel.section_names() {
            assert!(text.contains(name), "{name} is not on the rail: {text}");
        }
        let tint_of = |out: &Rendered, name: &str| {
            out.scene
                .texts
                .iter()
                .flat_map(|text| text.runs.iter())
                .filter(|run| run.text.trim() == name)
                .filter_map(|run| run.color)
                .next_back()
                .unwrap_or_else(|| panic!("{name} is not drawn"))
        };
        assert_eq!(tint_of(&out, "AGENT"), out.skin.heading);
        assert_ne!(tint_of(&out, "APPEARANCE"), out.skin.heading);

        // Pressing one changes what the list shows, which is the whole point of
        // a rail: the same panel, a different screen.
        let looks = layout
            .settings_rail
            .iter()
            .find(|(index, _)| panel.section_names()[*index] == crate::settings::APPEARANCE)
            .map(|(index, _)| *index)
            .expect("the appearance is on the rail");
        panel.choose(looks);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let text = text_of(&out.scene);
        assert!(text.contains("BACKGROUND COLORS"), "{text}");
        assert!(!text.contains("api keys"), "the agent section is still up");
        assert_eq!(tint_of(&out, "APPEARANCE"), out.skin.heading);
    }

    /// "the line on each menu is too long, exceeding the size of the text."
    /// The chosen section's band hugs its name: the mark, the text and a
    /// breath of padding, never the whole cell. The press region is still the
    /// full cell, so nothing got harder to hit.
    #[test]
    fn the_rail_band_hugs_the_chosen_name() {
        // AGENT, the shortest name on the rail, so the gap between the name
        // and the cell is the widest there is to prove the band lets it go.
        let panel = a_panel_on(&Config::default(), crate::settings::AGENT);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (at, cell) = out
            .layout
            .settings_rail
            .iter()
            .find(|(index, _)| *index == panel.chosen())
            .map(|(index, cell)| (*index, *cell))
            .expect("the chosen section is on the rail");
        let name = panel.section_names()[at];
        let band = out
            .scene
            .rects
            .iter()
            .find(|rect| {
                let [x, y, ..] = rect.xywh();
                rect.rgba() == out.skin.strip
                    && (x - cell.x).abs() < 0.01
                    && (y - cell.y).abs() < 0.01
            })
            .unwrap_or_else(|| panic!("{name} has no band"));
        // Wide enough for the mark and the name, with a breath of padding, and
        // well short of the cell the old band filled.
        let column = 8.0; // what render_settings hands the frame
        let text = name.chars().count() as f32 * column;
        let [_, _, w, _] = band.xywh();
        assert!(w >= MARK_W + 3.0 + text, "the band does not cover {name}: {w}");
        assert!(
            w <= MARK_W + 3.0 + text + column * 2.0,
            "the band runs past the text: {w} for {text} of name"
        );
        assert!(w < cell.w - column, "the band still fills the cell: {w} of {}", cell.w);
        // The cell past the band is still the press (the cell's far edge
        // belongs to the rail divider's grab, which is not the band's doing).
        let past_band = cell.x + cell.w * 0.85;
        assert!(past_band > cell.x + w, "nowhere past the band to probe");
        assert_eq!(
            out.layout.hit(past_band, cell.y + cell.h * 0.5),
            Some(Hit::SettingsSection(at))
        );
    }

    /// Item F1: the rail hides no section, at the smallest window the window
    /// will open at and the largest text the settings file will carry.
    ///
    /// The names were one column that stopped at the last one that fitted, so
    /// 40 point in a 680 by 380 window drew three of the five: MCP and
    /// APPEARANCE had no box, answered no click, and APPEARANCE is the only
    /// place a font size raised that far can be lowered again. The column wraps
    /// now, so every section keeps a box it can be pressed on whatever the
    /// window and the font are doing.
    #[test]
    fn every_section_keeps_a_box_at_the_smallest_window_and_the_biggest_text() {
        let panel = a_settings_panel(&Config::default());
        let names = panel.section_names();
        let (w, h) = (crate::MIN_SIZE.width as f32, crate::MIN_SIZE.height as f32);
        for font in [PANE_TEXT, BIGGEST_TEXT] {
            let out = render_settings_at_font(&panel, w, h, font);
            let layout = &out.layout;
            assert_eq!(
                layout.settings_rail.len(),
                names.len(),
                "{font:?} lost a section"
            );
            let foot = layout.settings.y + layout.settings.h;
            for (index, at) in &layout.settings_rail {
                let (x, y) = middle(*at);
                assert_eq!(
                    layout.hit(x, y),
                    Some(Hit::SettingsSection(*index)),
                    "{} answers nothing at {font:?}",
                    names[*index]
                );
                assert!(
                    at.x + at.w <= layout.settings_list.x,
                    "{} runs into the list at {font:?}: {at:?}",
                    names[*index]
                );
                assert!(
                    at.y + at.h <= foot,
                    "{} is drawn past the bottom of the panel at {font:?}: {at:?}",
                    names[*index]
                );
            }
            // No two of them are the same box, and none of them is on top of
            // another: a box under another box is a name nothing can be aimed
            // at even though the layout carries it.
            for (index, at) in &layout.settings_rail {
                for (other, was) in &layout.settings_rail {
                    if index == other {
                        continue;
                    }
                    let apart = at.x + at.w <= was.x
                        || was.x + was.w <= at.x
                        || at.y + at.h <= was.y
                        || was.y + was.h <= at.y;
                    assert!(apart, "{at:?} and {was:?} are on top of each other");
                }
            }
            // And every one of them is written where its box is, in as much of
            // the name as the box holds. A narrow box clips, the way every name
            // in this window clips, so what is asserted is the front of the
            // name rather than the whole of it.
            let text = text_of(&out.scene);
            for name in &names {
                let front: String = name.chars().take(3).collect();
                assert!(
                    text.contains(&front),
                    "{name} is not written at {font:?}: {text}"
                );
            }
        }
    }

    /// The line between the rail and the settings is a divider like any other:
    /// it is grabbed by a band wider than the gap it stands in, and it takes
    /// nothing from either side that either side needs.
    #[test]
    fn the_settings_rail_is_grabbed_by_the_line_beside_it() {
        let panel = a_settings_panel(&Config::default());
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let divider = layout.settings_rail_divider;
        assert!(divider.live(), "there is nothing to drag");
        assert!(divider.band.w > GAP, "the band is no wider than the gap");

        let y = layout.settings_list.y + 30.0;
        // The hairline the eye reads as the line, and both ends of the band
        // around it. The list stands its own padding in from the line.
        let drawn = layout.settings_list.x - PAD - (GAP * 0.5).floor();
        for x in [
            drawn,
            divider.band.x + 0.5,
            divider.band.x + divider.band.w - 0.5,
        ] {
            assert_eq!(layout.hit(x, y), Some(Hit::SettingsRailDivider), "at {x}");
        }

        // A name is still pressed where it is drawn, and the left hand end of a
        // row is still that row: the band reaches into the rail, where there is
        // room for it, and stops at the list, where the labels start.
        let (rx, ry) = middle(layout.settings_rail[1].1);
        assert_eq!(layout.hit(rx, ry), Some(Hit::SettingsSection(1)));
        let (index, side, row) = layout.settings_rows[0];
        assert_eq!(
            layout.hit(row.x + 2.0, row.y + row.h * 0.5),
            Some(Hit::SettingsRow(index, side))
        );

        // And it is not there when the panel is not: a band left behind by a
        // shape change is a press that lands on something nobody can see.
        let dock = Dock::new();
        let plain = Layout::compute(1400.0, 900.0, &shape(&dock, &[]));
        assert!(!plain.settings_rail_divider.live());
        assert_ne!(plain.hit(drawn, y), Some(Hit::SettingsRailDivider));
    }

    /// Dragging it puts the line under the pointer and the settings beside it
    /// move with it: the rail ends where the list begins, and nothing is drawn
    /// across the two.
    #[test]
    fn dragging_the_settings_rail_moves_the_settings_with_it() {
        let panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let was = out.layout.settings_list.x;
        let mut seen = Vec::new();
        for x in [200.0, 420.0, 170.0] {
            let ratio = out.layout.settings_rail_ratio_at(x);
            let moved = render_settings_at_rail(&panel, 1400.0, 900.0, None, ratio);
            let layout = &moved.layout;
            let list = layout.settings_list;
            seen.push(list.x);
            // Under the pointer rather than near it, to the pixel the width was
            // floored to.
            let drawn = list.x - PAD - GAP * 0.5;
            assert!((drawn - x).abs() <= 1.5, "{x}: the line landed at {drawn}");

            // Every name ends where the gap begins, and every row starts a
            // padding past the far side of it.
            for (index, at) in &layout.settings_rail {
                assert!(
                    (at.x + at.w + GAP + PAD - list.x).abs() <= 0.01,
                    "name {index} at {at:?} against a list at {}",
                    list.x
                );
            }
            for (index, _, row) in &layout.settings_rows {
                assert!(row.x >= list.x - 0.01, "row {index} at {row:?}");
                assert!(row.x + row.w <= list.x + list.w + 0.01, "row {index}");
            }

            // And nothing straddles the line: a text box in the panel's body is
            // either a name in the rail or a setting in the list.
            for text in &moved.scene.texts {
                let at = text.at;
                if at.y + at.h <= list.y + 0.01 || at.y >= list.y + list.h - 0.01 {
                    continue;
                }
                let in_rail = at.x + at.w <= list.x - GAP + 0.01;
                let in_list = at.x >= list.x - 0.01;
                assert!(in_rail || in_list, "{at:?} is drawn across the line at {x}");
            }
        }
        assert!(
            seen.iter().any(|at| (at - was).abs() > 1.0),
            "the drag moved nothing: {seen:?} against {was}"
        );
    }

    /// Thrown past either end it stops where the names still fit, and so does
    /// the list beside them. Neither side is ever squeezed to nothing.
    #[test]
    fn the_settings_rail_dragged_past_the_end_stops_at_the_floor() {
        let panel = a_settings_panel(&Config::default());
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let floor = out.layout.settings_rail_divider.floor;
        assert!(floor > 0.0);
        for x in [-9000.0, -1.0, 0.0, 700.0, 1401.0, 9000.0] {
            let ratio = out.layout.settings_rail_ratio_at(x);
            let moved = render_settings_at_rail(&panel, 1400.0, 900.0, None, ratio);
            let layout = &moved.layout;
            // The cells stand PAD in from the rail's left edge, so the rail
            // itself is a padding wider than its first cell.
            let rail_w = layout.settings_rail[0].1.w + PAD;
            assert!(rail_w >= floor, "{x}: the rail is {rail_w}");
            assert!(
                layout.settings_list.w >= floor,
                "{x}: the settings are {}",
                layout.settings_list.w
            );
            assert!(!layout.settings_rows.is_empty(), "{x}: the list emptied");
        }
        // A fraction out of a settings file nobody clamped is held the same way.
        for ratio in [0.0, 1.0, -5.0, 12.0] {
            let moved = render_settings_at_rail(&panel, 1400.0, 900.0, None, ratio);
            assert!(
                moved.layout.settings_rail[0].1.w + PAD >= floor,
                "{ratio}"
            );
            assert!(moved.layout.settings_list.w >= floor, "{ratio}");
        }
    }

    /// The slider is a track a pointer can be anywhere along, and where it is
    /// along it is the value that would be written.
    #[test]
    fn the_slider_reads_a_pointer_as_a_value() {
        let panel = a_panel_on(&Config::parse("opacity = 0.50"), crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let (index, side, track) = layout
            .settings_tracks
            .first()
            .copied()
            .expect("a number is a slider");
        // The first track of the section is the first field of its first card,
        // which is the size of the text: the sliders are fields now.
        assert!(
            matches!(panel.cell(index, side), Some(crate::settings::Row::Setting { key, .. }) if *key == "font_size")
        );

        // Both ends and the middle, off the geometry the row is drawn with.
        assert_eq!(layout.slider_at(index, side, track.x), Some(0.0));
        assert_eq!(layout.slider_at(index, side, track.x + track.w), Some(1.0));
        let half = layout
            .slider_at(index, side, track.x + track.w * 0.5)
            .expect("the middle");
        assert!((half - 0.5).abs() < 0.01, "{half}");
        // A pointer that ran off the end holds the end rather than going dead.
        assert_eq!(layout.slider_at(index, side, track.x - 500.0), Some(0.0));
        assert_eq!(
            layout.slider_at(index, side, track.x + track.w + 500.0),
            Some(1.0)
        );
        // A row with no track has no position along one, and neither has the
        // side of a card that keeps no field there.
        assert_eq!(layout.slider_at(index + 500, side, track.x), None);
        let (alone, half, box_) = layout
            .settings_tracks
            .iter()
            .copied()
            .find(|(at, half, _)| {
                half.step(true)
                    .is_none_or(|next| panel.cell(*at, next).is_none())
            })
            .expect("a card of one field");
        let past = half.step(true).unwrap_or(crate::settings::Side::RightBelow);
        assert_eq!(layout.slider_at(alone, past, box_.x), None);

        // The track is drawn where it is pressed: an unlit bar the width of the
        // track and a lit one as far along it as the value.
        // Shorter than the line it stands in, or the card's own focus border
        // answers first: it is drawn in the accent too, and the accent is what
        // a lit track is.
        let on_the_track = |rgba: [f32; 4]| {
            out.scene
                .rects
                .iter()
                .filter(|rect| rect.rgba() == rgba)
                .map(|rect| rect.xywh())
                .find(|[x, y, _, h]| track.contains(*x + 0.5, *y + 0.5) && *h < track.h * 0.5)
        };
        let thumb = on_the_track(out.skin.gauge).expect("nothing is lit");
        assert!((thumb[0] - track.x).abs() < 0.01, "{thumb:?}");
        let full = on_the_track(out.skin.gauge_track).expect("there is no track");
        assert!((full[2] - track.w).abs() < 0.01, "{full:?}");
        // Where the value sits in its range, which for the text size the
        // window opens at is a fifth along.
        let at = (14.0 - 8.0) / (40.0 - 8.0);
        assert!(
            (thumb[2] / full[2] - at).abs() < 0.02,
            "the lit part is {thumb:?} of {full:?}"
        );
    }

    /// The mark that closes it is reachable, and clear of the corner the panel's
    /// own cut takes away.
    #[test]
    fn the_close_mark_clears_the_cut_corner() {
        let panel = a_settings_panel(&Config::default());
        for (w, h) in [(1400.0, 900.0), (700.0, 460.0), (2200.0, 1400.0)] {
            let out = render_settings(&panel, w, h, None);
            let layout = &out.layout;
            let (close, box_) = (layout.settings_close, layout.settings);
            let (x, y) = middle(close);
            assert_eq!(layout.hit(x, y), Some(Hit::SettingsClose), "{w}x{h}");
            assert!(
                close.x + close.w <= box_.x + box_.w - cut_of(box_),
                "the mark is drawn in the cut: {close:?} in {box_:?}"
            );
            // The mark stands on the panel, not on a block of its own: nothing
            // smaller than the panel is drawn behind it, whether or not the
            // pointer is on it.
            let mark = |hot: Option<Hit>| {
                let out = render_settings(&panel, w, h, hot);
                let close = out.layout.settings_close;
                let box_ = out.layout.settings;
                for rect in &out.scene.rects {
                    let [rx, ry, rw, rh] = rect.xywh();
                    let overlaps = rx < close.x + close.w
                        && rx + rw > close.x
                        && ry < close.y + close.h
                        && ry + rh > close.y;
                    // The panel's own surface and outline are the surface the
                    // mark is written on; anything smaller is a block.
                    let panel_itself = rw >= box_.w - 0.01 && rh >= box_.h - 0.01;
                    assert!(
                        !overlaps || panel_itself,
                        "{rect:?} is a block behind the close mark at {w}x{h}, hot {hot:?}"
                    );
                }
                // In the panel's own mark, not the window's: the title strip
                // draws the same glyph and is on screen behind a takeover.
                out.scene
                    .texts
                    .iter()
                    .filter(|text| close.contains(text.at.x + 1.0, text.at.y + 1.0))
                    .flat_map(|text| text.runs.iter())
                    .find(|run| run.icon && run.text == icons::CLOSE.to_string())
                    .and_then(|run| run.color)
                    .unwrap_or_else(|| panic!("no close mark at {w}x{h}"))
            };
            // What answers the pointer is the mark, in the colour this window
            // uses for losing something. A close with no answer at all would be
            // worse than the block it replaced.
            assert_eq!(mark(None), out.skin.bright, "{w}x{h}");
            assert_eq!(
                mark(Some(Hit::SettingsClose)),
                out.skin.bad,
                "the close mark does not answer the pointer at {w}x{h}"
            );
            assert_ne!(out.skin.bright, out.skin.bad);
        }
    }

    /// A card is drawn in exactly the room the model counted for it, and
    /// everything it draws is inside itself.
    ///
    /// This is the invariant the whole panel stands on: the model measures rows
    /// in whole lines and the layout draws them in pixels, so a card measured at
    /// one height and drawn at another puts every press below it on another
    /// card.
    #[test]
    fn a_card_measures_the_height_it_draws() {
        let panel = a_panel_on(&Config::default(), crate::settings::MCP);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let cols = out.layout.settings_entry_columns(PANE_TEXT.1);
        let (index, row) = the_card_row(&out, &panel);
        let card = match panel.row(index) {
            Some(crate::settings::Row::Card(card)) => card,
            other => panic!("row {index} is {other:?}"),
        };
        let counted = crate::settings::lines(
            panel.row(index).expect("the row"),
            cols,
        );
        assert_eq!(
            counted,
            design::card_row_lines(
                crate::settings::card_body_lines(card, cols),
                card.does.is_some()
            )
        );
        assert!(
            (row.h - counted as f32 * line).abs() < 0.01,
            "the row is {row:?} and the model counted {counted} lines of {line}"
        );

        // The card itself, and the space under it that keeps two cards apart.
        let (box_, parts) = the_card(&out, row, false);
        assert!(
            (row.h - box_.h - design::apart(line)).abs() < 0.01,
            "the space under the card is not APART: {box_:?} in {row:?}"
        );
        // Its body sits ROOM inside the border on every side, and its last
        // field and its hint are inside it.
        assert!(parts.body.x >= box_.x + design::room(line) - 0.01);
        assert!(parts.body.y >= parts.rule.y + design::room(line) - 0.01);
        assert!(parts.body.y + parts.body.h <= box_.y + box_.h - design::room(line) + 0.01);
        for text in out.scene.texts.iter().filter(|text| {
            text.at.y >= box_.y - 0.01 && text.at.y < box_.y + box_.h && text.at.x >= box_.x
        }) {
            assert!(
                text.at.y + text.at.h <= box_.y + box_.h + 0.01,
                "a line of the card is drawn out of the bottom of it: {:?} in {box_:?}",
                text.at
            );
            assert!(
                text.at.x + text.at.w <= box_.x + box_.w + 0.01,
                "a line of the card runs out of its right edge: {:?}",
                text.at
            );
        }
        // And the press at the very bottom of the row is still that row, which
        // is what a card measured short takes away.
        let (x, _) = middle(row);
        assert_eq!(
            out.layout.hit(x, row.y + row.h - 1.0),
            Some(Hit::SettingsRow(index, crate::settings::Side::Left))
        );
    }

    /// A field is a label with its value under it, never beside it, and the
    /// press inside a card lands on what is under the pointer rather than on
    /// whatever row happens to be nearest.
    ///
    /// "all text looks the same name, description, repo": a label and a value on
    /// one line read as one sentence, and every value on the panel looked like
    /// part of its own key.
    #[test]
    fn a_field_is_its_label_over_its_value_and_a_press_in_a_card_lands_in_it() {
        let panel = a_panel_on(&Config::default(), crate::settings::MCP);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let cols = out.layout.settings_entry_columns(PANE_TEXT.1);
        let (index, row) = the_card_row(&out, &panel);
        let card = match panel.row(index) {
            Some(crate::settings::Row::Card(card)) => card,
            other => panic!("row {index} is {other:?}"),
        };
        let (_, parts) = the_card(&out, row, false);
        let across = design::across(card.fields.len(), design::card_cols(cols));
        let slots = settings_card_slots(
            parts.body,
            line,
            &crate::settings::card_hints(card),
            across,
            card.group.as_ref().map(|group| group.at),
        );
        for (field, slot) in card.fields.iter().zip(&slots) {
            let (label_at, input_at) = settings_field_boxes(*slot, line);
            assert_eq!(
                line_of(&out, label_at.x, label_at.y),
                field.label,
                "the label is not on its own line"
            );
            assert!(
                input_at.y >= label_at.y + label_at.h - 0.01,
                "the value is beside its label rather than under it"
            );
            assert!(
                (input_at.y - label_at.y - line - design::tight(line)).abs() < 0.01,
                "the gap between a label and its value is not TIGHT"
            );
            assert_eq!(
                line_of(&out, input_at.x, input_at.y),
                field.value(),
                "the value is not under its own label"
            );
            // A reading has no border and no fill, which is the whole of what
            // says it cannot be typed into; a field that can be typed into
            // wears the input box.
            if field.editable() {
                continue;
            }
            for rgba in [out.skin.input, out.skin.edge] {
                assert!(
                    !covered(&out, input_at, input_at.h, rgba),
                    "a reading is drawn as a box that can be typed into"
                );
            }
            let (x, y) = middle(input_at);
            assert_eq!(
                out.layout.hit(x, y),
                Some(Hit::SettingsRow(index, crate::settings::Side::Left)),
                "a press inside the card answers for another row"
            );
        }
        // Only what can be typed into claims a press region: one per
        // editable field, none for a reading.
        let editable = card
            .fields
            .iter()
            .filter(|field| field.editable())
            .count();
        assert_eq!(
            out.layout
                .settings_values
                .iter()
                .filter(|(at, _, _)| *at == index)
                .count(),
            editable,
            "the card's press regions do not match its editable fields"
        );

        // And the field that can be typed into is the same shape with the box
        // round it, in the section that has one.
        let out = render_settings(
            &a_panel_on(&Config::default(), crate::settings::AGENT),
            1400.0,
            900.0,
            None,
        );
        let (index, side, at) = *out
            .layout
            .settings_values
            .first()
            .expect("a value that can be changed");
        let (x, y) = middle(at);
        assert_eq!(out.layout.hit(x, y), Some(Hit::SettingsValue(index, side)));
        assert!(covered(&out, at, at.h, out.skin.input), "no box round it");
    }

    /// The two fields of a card go side by side while the card is wide enough
    /// for both to keep their columns, and stack when it is not.
    ///
    /// "aware flexible design on resize as well". Cards stay full width and it
    /// is their contents that answer a narrow window, so nothing is ever laid
    /// out for one width.
    #[test]
    fn a_card_puts_two_fields_across_until_it_is_too_narrow_for_both() {
        let panel = a_panel_on(&Config::default(), crate::settings::MCP);
        let line = Text::line_for(PANE_TEXT.0);
        let flip = design::reflow_columns();

        let wide = render_settings(&panel, 1400.0, 900.0, None);
        let (index, wide_row) = the_card_row(&wide, &panel);
        let wide_cols = wide.layout.settings_entry_columns(PANE_TEXT.1);
        assert!(wide_cols >= flip, "{wide_cols} columns is not wide enough");

        let narrow = render_settings(&panel, 470.0, 900.0, None);
        let (_, narrow_row) = the_card_row(&narrow, &panel);
        let narrow_cols = narrow.layout.settings_entry_columns(PANE_TEXT.1);
        assert!(narrow_cols < flip, "{narrow_cols} columns is still wide");

        let card = match panel.row(index) {
            Some(crate::settings::Row::Card(card)) => card,
            other => panic!("row {index} is {other:?}"),
        };
        assert_eq!(card.fields.len(), 2, "the card has two fields to reflow");
        let places = |out: &Rendered, row: Panel, cols: usize| -> Vec<Panel> {
            let (_, parts) = the_card(out, row, false);
            let across = design::across(card.fields.len(), design::card_cols(cols));
            settings_card_slots(
                parts.body,
                line,
                &crate::settings::card_hints(card),
                across,
                card.group.as_ref().map(|group| group.at),
            )
        };
        let side_by_side = places(&wide, wide_row, wide_cols);
        assert_eq!(side_by_side[0].y, side_by_side[1].y, "not on one band");
        assert!(
            side_by_side[1].x >= side_by_side[0].x + side_by_side[0].w,
            "the two fields overlap: {side_by_side:?}"
        );
        let stacked = places(&narrow, narrow_row, narrow_cols);
        assert_eq!(stacked[0].x, stacked[1].x, "not in one column");
        assert!(
            stacked[1].y >= stacked[0].y + stacked[0].h,
            "the two fields overlap: {stacked:?}"
        );

        // Both labels are really drawn where the reflow put them, and the card
        // grew by the band it gained: the model counted the same flip the layout
        // drew, or every press under the card would be a row out.
        for (out, slots) in [(&wide, &side_by_side), (&narrow, &stacked)] {
            for (field, slot) in card.fields.iter().zip(slots) {
                assert_eq!(line_of(out, slot.x, slot.y), field.label);
            }
        }
        assert!(
            narrow_row.h > wide_row.h,
            "the stacked card is not taller: {narrow_row:?} against {wide_row:?}"
        );
    }

    /// The panel's own title is drawn in the panel title role and a card's title
    /// in the smaller one under it, so what you are looking at and what group it
    /// is in are two different sizes.
    ///
    /// "title totally unclear on each panel section increase font size on each".
    /// Everything on the panel was one size, so nothing on it was a heading.
    #[test]
    fn the_panel_title_is_larger_than_a_card_title_which_is_larger_than_a_value() {
        let panel = a_panel_on(&Config::default(), crate::settings::MCP);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let size = PANE_TEXT.0;
        let title = out
            .scene
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text.contains("SETTINGS")))
            .expect("the panel says what it is");
        assert_eq!(title.size, design::panel_title_size(size));

        let (_, row) = the_card_row(&out, &panel);
        let (_, parts) = the_card(&out, row, true);
        let card_title = out
            .scene
            .texts
            .iter()
            .find(|text| (text.at.y - parts.title.y).abs() < 0.51)
            .expect("the card says what it holds");
        assert_eq!(card_title.size, design::card_title_size(size));
        assert!(
            card_title
                .runs
                .iter()
                .any(|run| run.text.contains("ADD A SERVER")),
            "{:?}",
            card_title.runs
        );
        assert!(title.size > card_title.size, "the two headings are one size");
        assert!(card_title.size > size, "a card title is the size of its fields");

        // And the whole scale really is on screen: the hint under the fields is
        // smaller than the value over it.
        let hint = out
            .scene
            .texts
            .iter()
            .filter(|text| text.at.y > parts.body.y)
            .map(|text| text.size)
            .fold(f32::INFINITY, f32::min);
        assert_eq!(hint, design::hint_size(size));
        assert!(hint < size, "a hint is not quieter than a value");
    }

    /// Every button a card carries stays inside its own card, at every size the
    /// panel is used at.
    ///
    /// The pane text goes to forty points and the window goes down to nothing,
    /// and a card cut off by the bottom of the list has less room than it asked
    /// for. A button drawn above its own card would be a press that answers for
    /// the card over it.
    #[test]
    fn a_card_keeps_its_buttons_inside_itself_at_every_size() {
        for (w, h) in [(1400.0, 900.0), (700.0, 420.0), (420.0, 260.0), (300.0, 180.0)] {
            for font in [PANE_TEXT, BIGGEST_TEXT] {
                for section in [crate::settings::SKILLS, crate::settings::MCP] {
                    let mut panel = a_wrapping_skills_panel();
                    let at = panel
                        .section_names()
                        .iter()
                        .position(|name| *name == section)
                        .expect("a section");
                    panel.choose(at);
                    let out = render_settings_at_font(&panel, w, h, font);
                    let line = Text::line_for(font.0);
                    for (index, box_) in out
                        .layout
                        .settings_toggles
                        .iter()
                        .chain(out.layout.settings_removes.iter())
                    {
                        let row = out
                            .layout
                            .settings_rows
                            .iter()
                            .find(|(at, _, _)| at == index)
                            .map(|(_, _, row)| *row)
                            .expect("the row the button stands in");
                        let card = settings_card(row, line);
                        assert!(
                            box_.y >= card.y - 0.01
                                && box_.y + box_.h <= card.y + card.h + 0.01
                                && box_.x >= card.x - 0.01
                                && box_.x + box_.w <= card.x + card.w + 0.01,
                            "{section} at {w}x{h}, {font:?}: {box_:?} is outside {card:?}"
                        );
                        assert!(
                            out.layout.settings_list.contains(box_.x + 1.0, box_.y + 1.0),
                            "{section} at {w}x{h}, {font:?}: {box_:?} is outside the list"
                        );
                    }
                }
            }
        }
    }

    /// Every group's title is the same green the showing tab's line is, is
    /// drawn larger than the settings under it, and is given the room for that
    /// by the row it was laid out in.
    ///
    /// A list is unreadable if its groups do not separate from their contents.
    /// This was written about the bare headings the palette stood under: those
    /// are gone, and the same assertions are made about the card titles that
    /// replaced them, since a title measured at one height and drawn at another
    /// would put every click below it on the wrong row.
    #[test]
    fn the_settings_card_titles_are_the_heading_accent_and_the_size_they_were_measured_at() {
        let mut found = 0;
        let mut panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
        // A section of cards is taller than any window, so each title is looked
        // for on the screenful its own card is on rather than on the first one.
        // Tall on purpose: the last card here is ten swatches, and what this
        // test is about is the title's tint and the row it was measured in.
        let shape = render_settings(&panel, 1400.0, 1800.0, None);
        let rows = shape.layout.settings_capacity(13.0);
        let cols = shape.layout.settings_entry_columns(PANE_TEXT.1);
        for heading in [
            "DEFAULT THEMES",
            "THE WINDOW'S OWN TONES",
            "THE CODE COLOURS",
            "THE TOOL MARKS",
            "THE METERS",
        ] {
            let at = panel
                .rows()
                .iter()
                .position(|row| match row {
                    crate::settings::Row::Palette(palette) => palette.title == heading,
                    crate::settings::Row::Card(card) => card.title == heading,
                    _ => false,
                })
                .unwrap_or_else(|| panic!("{heading} is not a group of the section"));
            // From the top of the section every time, so where one heading was
            // found does not decide where the next one is looked for.
            while panel.scroll(4, false, rows, cols) {}
            while panel.first() < at && panel.scroll(1, true, rows, cols) {}
            let out = render_settings(&panel, 1400.0, 1800.0, None);
            let text = out
                .scene
                .texts
                .iter()
                .find(|text| text.runs.iter().any(|run| run.text.trim() == heading))
                .unwrap_or_else(|| panic!("{heading} is not on the panel"));
            let run = text
                .runs
                .iter()
                .find(|run| run.text.trim() == heading)
                .expect("the run that was just found");
            assert_eq!(run.color, Some(out.skin.heading), "{heading}");
            // Larger than the rows under it, and inside the row it was measured
            // into: 13pt is what everything else on the panel is drawn at.
            assert!(text.size > 13.0, "{heading} is drawn at {}", text.size);
            let row = out
                .layout
                .settings_rows
                .iter()
                .find(|(_, _, row)| row.contains(text.at.x + 1.0, text.at.y + 1.0))
                .map(|(index, _, row)| (*index, *row))
                .unwrap_or_else(|| panic!("{heading} is drawn on no row at all"));
            let named = match panel.row(row.0) {
                Some(crate::settings::Row::Palette(palette)) => palette.title == heading,
                Some(crate::settings::Row::Card(card)) => card.title == heading,
                _ => false,
            };
            assert!(
                named,
                "{heading} is drawn on row {}, which is {:?}",
                row.0,
                panel.row(row.0)
            );
            assert!(
                text.at.y + noob_draw::Text::line_for(text.size) <= row.1.y + row.1.h + 0.01,
                "{heading} is taller than the row it was laid out in: {:?} in {:?}",
                text.at,
                row.1
            );
            found += 1;
        }
        assert_eq!(found, 5);
        // Not the tint a field's value is written in, or a title is another
        // line of the card.
        let skin = shape.skin;
        assert_ne!(skin.heading, skin.body);
        assert_ne!(skin.heading, skin.title);
    }

    /// No row has a hairline under it any more, and a card carries a border
    /// and exactly one divider instead.
    ///
    /// There was a line under every row on the panel. A line between every two
    /// things on screen says nothing about which of them belong together, which
    /// is the whole of "lines everywhere, unclear what each thing is". Grouping
    /// and space say it now: a card is a bordered box, its fields have space
    /// between them, and two cards have more.
    #[test]
    fn no_row_is_ruled_off_and_a_card_is_bordered_instead() {
        let out = render_settings(
            &a_panel_on(&Config::default(), crate::settings::APPEARANCE),
            1400.0,
            900.0,
            None,
        );
        let rows = &out.layout.settings_rows;
        assert!(rows.len() > 3, "not enough rows to prove it: {}", rows.len());
        for (index, _, row) in rows {
            let ruled = out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == out.skin.edge
                    && h <= 1.01
                    && (x - row.x).abs() < 0.01
                    && (w - row.w).abs() < 0.01
                    && (y - (row.y + row.h - 1.0)).abs() < 0.01
            });
            assert!(!ruled, "row {index} still has a hairline under it");
        }

        // And the container that replaced it: the card's own border, cut on the
        // same corner every surface in this window is cut on, with the one
        // divider under its title and nothing else drawn between its fields.
        let panel = a_panel_on(&Config::default(), crate::settings::MCP);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (_, row) = the_card_row(&out, &panel);
        let (card, parts) = the_card(&out, row, true);
        // The cursor opens on this card, so its border may wear the focus
        // colour; either ink, the container is a border and not a rule.
        let border = out
            .scene
            .rects
            .iter()
            .find(|rect| {
                let [x, y, w, h] = rect.xywh();
                (rect.rgba() == out.skin.edge || rect.rgba() == out.skin.edge_focus)
                    && (x - card.x).abs() < 0.01
                    && (y - card.y).abs() < 0.01
                    && (w - card.w).abs() < 0.01
                    && (h - card.h).abs() < 0.01
            })
            .expect("the card has no border");
        assert!(border.extra()[1] > 0.0, "the border is filled, not stroked");
        assert_eq!(
            (border.extra()[2] as u32) & noob_draw::Rect::TOP_RIGHT,
            noob_draw::Rect::TOP_RIGHT,
            "the card is not cut on the window's own corner"
        );
        let hairlines: Vec<[f32; 4]> = out
            .scene
            .rects
            .iter()
            .filter(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == out.skin.edge
                    && h <= 1.01
                    && w > card.w * 0.5
                    && x >= card.x - 0.01
                    && y >= card.y
                    && y <= card.y + card.h
            })
            .map(|rect| rect.xywh())
            .collect();
        assert_eq!(
            hairlines.len(),
            1,
            "a card gets one divider and no more: {hairlines:?}"
        );
        assert!(
            (hairlines[0][1] - parts.rule.y).abs() < 1.01,
            "the divider is not under the title: {:?} against {:?}",
            hairlines[0],
            parts.rule
        );
        // Under the title and above the first field, which is what makes it a
        // header rather than a line through the card.
        assert!(parts.rule.y > parts.title.y + parts.title.h - 0.01);
        assert!(parts.rule.y < parts.body.y);
    }

    /// Anything that can be typed into or pressed to change is drawn as a box
    /// with an outline round it. Without one an editable row looked exactly like
    /// a reading, and the only way to tell one from the other was to press it.
    ///
    /// APPEARANCE was on this list for the theme, which was one box holding one
    /// word. It is every theme drawn as its own box now, which is asserted at
    /// the end: the section carries no plain value box left.
    #[test]
    fn an_editable_row_is_drawn_as_a_box_with_an_edge() {
        // The endpoint, which is the one row on the panel that is typed into.
        for section in [crate::settings::AGENT] {
            let panel = a_panel_on(&Config::default(), section);
            let out = render_settings(&panel, 1400.0, 900.0, None);
            let boxes = &out.layout.settings_values;
            assert!(!boxes.is_empty(), "{section} has no control on it");
            for (index, _, at) in boxes {
                let over = |rect: &noob_draw::Rect| {
                    let [x, y, w, h] = rect.xywh();
                    (x - at.x).abs() < 0.01
                        && (y - at.y).abs() < 0.01
                        && (w - at.w).abs() < 0.01
                        && (h - at.h).abs() < 0.01
                };
                assert!(
                    out.scene
                        .rects
                        .iter()
                        .any(|rect| over(rect) && rect.extra()[3] >= 1.0),
                    "the control on row {index} of {section} has no outline"
                );
                assert!(
                    out.scene
                        .rects
                        .iter()
                        .any(|rect| over(rect) && rect.rgba() == out.skin.input),
                    "the control on row {index} of {section} has no box under it"
                );
            }
        }

        // Every option of the theme, each one its own box: the one that is set
        // is filled the way a primary button is, and the rest carry the outline
        // a secondary does.
        let panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let options = &out.layout.settings_choices;
        // The three presets on the left column and custom on the right.
        assert_eq!(options.len(), crate::config::THEMES.len() + 1, "{options:?}");
        for (index, side, option, at) in options {
            let over = |rect: &&noob_draw::Rect| {
                let [x, y, w, h] = rect.xywh();
                (x - at.x).abs() < 0.01
                    && (y - at.y).abs() < 0.01
                    && (w - at.w).abs() < 0.01
                    && (h - at.h).abs() < 0.01
            };
            let name = option_name(*side, *option);
            let set = matches!(
                panel.cell(*index, *side),
                Some(crate::settings::Row::Setting { value, .. })
                    if value == name
            );
            let wanted = match set {
                true => out.skin.button,
                false => out.skin.input,
            };
            assert!(
                out.scene.rects.iter().filter(over).any(|rect| rect.rgba() == wanted),
                "option {option} is not drawn as the box it is"
            );
        }
        // And the value box the theme used to be is gone with it.
        assert!(
            out.layout.settings_values.is_empty(),
            "{:?} is still a one word control",
            out.layout.settings_values
        );

        // The endpoint's text sits inside its box rather than on the stroke, and
        // so does the caret while it is being typed into.
        let mut panel = a_panel_on(&Config::default(), crate::settings::AGENT);
        while !matches!(
            panel.at_cursor(),
            Some(crate::settings::Row::Field { .. })
        ) {
            assert!(panel.step(true), "the agent section has no field on it");
        }
        assert!(panel.edit());
        assert!(panel.type_text("http://localhost:9/v1"));
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let (_, _, at) = out
            .layout
            .settings_values
            .iter()
            .find(|(index, side, _)| *index == panel.cursor() && *side == panel.side())
            .expect("the endpoint's box");
        let caret = out
            .scene
            .rects
            .iter()
            .filter(|rect| rect.rgba() == out.skin.caret && rect.extra()[3] == 0.0)
            .find(|rect| {
                let [x, y, w, _] = rect.xywh();
                w <= 3.0 && (y - at.y).abs() < 0.01 && x >= at.x && x <= at.x + at.w
            })
            .unwrap_or_else(|| panic!("no caret inside {at:?} while typing"));
        let [x, _, w, _] = caret.xywh();
        assert!(
            x > at.x && x + w <= at.x + at.w,
            "the caret is on the border: {caret:?} in {at:?}"
        );
    }

    /// The cut corner belongs to the control, not to the pointer.
    ///
    /// The fill and the outline of every one of these boxes carried the
    /// diagonal and the fill drawn over them under the pointer did not, so the
    /// theme button squared its corner off the moment the pointer arrived and
    /// painted into the cut its own outline draws. Every box drawn that way had
    /// it: the theme and the flags, the endpoint field, a skill's on/off, its
    /// uninstall, and a session's delete.
    #[test]
    fn a_control_under_the_pointer_keeps_its_cut_corner() {
        // Every kind of control the panel has, in the section that carries it.
        let sections = [
            crate::settings::APPEARANCE,
            crate::settings::AGENT,
            crate::settings::MCP,
            crate::settings::SESSIONS,
        ];
        let (mut values, mut toggles, mut removes) = (0, 0, 0);
        for section in sections {
            let panel = a_panel_on(&Config::default(), section);
            let plain = render_settings(&panel, 1400.0, 900.0, None);
            let controls: Vec<(Panel, Hit)> = plain
                .layout
                .settings_values
                .iter()
                .map(|(index, side, at)| (*at, Hit::SettingsValue(*index, *side)))
                .chain(
                    plain
                        .layout
                        .settings_toggles
                        .iter()
                        .map(|(index, at)| (*at, Hit::SettingsToggle(*index))),
                )
                .chain(
                    plain
                        .layout
                        .settings_removes
                        .iter()
                        .map(|(index, at)| (*at, Hit::SettingsRemove(*index))),
                )
                .chain(
                    plain
                        .layout
                        .settings_acts
                        .iter()
                        .map(|(index, act, at)| (*at, Hit::SettingsAct(*index, *act))),
                )
                .chain(plain.layout.settings_choices.iter().map(
                    |(index, side, option, at)| {
                        (*at, Hit::SettingsChoice(*index, *side, *option))
                    },
                ))
                .collect();
            assert!(!controls.is_empty(), "{section} has no control on it");
            for (at, hit) in controls {
                let out = render_settings(&panel, 1400.0, 900.0, Some(hit));
                let over = |rect: &&noob_draw::Rect| {
                    let [x, y, w, h] = rect.xywh();
                    (x - at.x).abs() < 0.01
                        && (y - at.y).abs() < 0.01
                        && (w - at.w).abs() < 0.01
                        && (h - at.h).abs() < 0.01
                };
                // A control is a box of some kind: the input fill a field and
                // a value wear, the accent a primary button is filled with, or
                // the outline a secondary and a danger button carry.
                let boxed = [
                    out.skin.input,
                    out.skin.button,
                    out.skin.edge,
                    out.skin.close_hot,
                ];
                // Off the cold render: a primary button under the pointer is
                // drawn in the hot accent instead of its own, so its idle fill
                // is only there while nothing is over it.
                let base = plain
                    .scene
                    .rects
                    .iter()
                    .find(|rect| over(rect) && boxed.contains(&rect.rgba()))
                    .unwrap_or_else(|| panic!("{section}: {hit:?} has no box under it"));
                // And each kind lights with its own hot colour: a filled button
                // in the brighter accent, everything else in the window's own.
                let lit = out
                    .scene
                    .rects
                    .iter()
                    .find(|rect| {
                        over(rect)
                            && (rect.rgba() == out.skin.hot || rect.rgba() == out.skin.button_hot)
                    })
                    .unwrap_or_else(|| panic!("{section}: {hit:?} does not light up"));
                assert!(
                    lit.extra()[1] > 0.0,
                    "{section}: {hit:?} lights up as a square: {lit:?}"
                );
                assert_eq!(
                    lit.extra()[1..3],
                    base.extra()[1..3],
                    "{section}: {hit:?} is not the shape of the box under it"
                );
                assert_eq!(
                    (lit.extra()[2] as u32) & noob_draw::Rect::TOP_RIGHT,
                    noob_draw::Rect::TOP_RIGHT,
                    "{section}: {hit:?} cuts a corner other than the top right"
                );
                match hit {
                    Hit::SettingsValue(..) | Hit::SettingsChoice(..) => values += 1,
                    Hit::SettingsToggle(_) => toggles += 1,
                    _ => removes += 1,
                }
            }
        }
        // One of each kind at the least, or the pass proves it about one box.
        assert!(values > 0 && toggles > 0 && removes > 0, "{values} {toggles} {removes}");
    }

    /// Nothing the panel draws leaves it, at any size. A rectangle outside a
    /// takeover is a rectangle over the desktop.
    #[test]
    fn nothing_the_settings_panel_draws_escapes_it() {
        let panel = a_settings_panel(&Config::default());
        for (w, h) in [(1400.0, 900.0), (680.0, 380.0), (2200.0, 1400.0)] {
            let out = render_settings(&panel, w, h, Some(Hit::SettingsValue(7, Side::Left)));
            let box_ = out.layout.settings;
            let inside = |x: f32, y: f32, rw: f32, rh: f32| {
                x >= box_.x - 0.01
                    && y >= box_.y - 0.01
                    && x + rw <= box_.x + box_.w + 0.01
                    && y + rh <= box_.y + box_.h + 0.01
            };
            for rect in &out.scene.rects {
                let [x, y, rw, rh] = rect.xywh();
                // The backdrop and the title strip are the window's, not the
                // panel's; everything else here belongs to the panel.
                let backdrop = rw >= w - 0.01 && rh >= h - 0.01;
                assert!(
                    backdrop || y + rh <= TITLE_H + 0.01 || inside(x, y, rw, rh),
                    "{rect:?} escapes the panel at {w}x{h}"
                );
            }
            for text in &out.scene.texts {
                let at = text.at;
                assert!(
                    at.y + at.h <= TITLE_H + 0.01 || inside(at.x, at.y, at.w, at.h),
                    "{at:?} escapes the panel at {w}x{h}"
                );
            }
            assert!(out.scene.over_rects.is_empty(), "nothing floats over a takeover");
        }
    }

    /// What each section says: its own heading, the agent's file, the sessions
    /// and the skills read off the disk, the settings it can change, and the
    /// palette drawn as itself.
    #[test]
    fn the_panel_says_what_it_is_and_draws_the_palette() {
        let config = Config::parse("accent = #123456");
        let words = |section: &str| {
            text_of(&render_settings(&a_panel_on(&config, section), 1400.0, 1200.0, None).scene)
        };
        for (section, wanted) in [
            (crate::settings::AGENT, "NOOB_BASE_URL"),
            (crate::settings::AGENT, "http://localhost:8080/v1"),
            (crate::settings::SESSIONS, "rebuild the settings panel"),
            (crate::settings::SKILLS, "coding"),
            (crate::settings::MCP, "ADD A SERVER"),
            (crate::settings::APPEARANCE, "theme"),
            (crate::settings::APPEARANCE, "opacity"),
            (crate::settings::APPEARANCE, "pane_font_size"),
            // The palette is under APPEARANCE too now, and it is labelled with
            // what each colour colours rather than with its key.
            (crate::settings::APPEARANCE, "the accent"),
            (crate::settings::APPEARANCE, "the title bar"),
        ] {
            let text = words(section);
            assert!(
                text.contains(wanted),
                "{wanted:?} is not in {section}: {text}"
            );
        }
        // Where the agent's file is, on the last card of a section taller than
        // the window: a panel that only says it on a screenful nobody scrolls
        // to is a panel that does not say it.
        let mut agent = a_panel_on(&config, crate::settings::AGENT);
        scrolled_to_the_end(&mut agent);
        let text = text_of(&render_settings(&agent, 1400.0, 1200.0, None).scene);
        assert!(
            text.contains("/home/hec/.config/noob/.env"),
            "the agent's file is nowhere on its section: {text}"
        );
        // And what it does not say: the pane toggles and the divider ratios are
        // off the panel, so their names are nowhere in the text any section
        // draws. `show_files` was on this list until PANES was really removed.
        for section in crate::settings::SECTIONS {
            let text = words(section);
            for key in crate::settings::OFF_PANEL {
                assert!(!text.contains(key), "{section} still draws {key}: {text}");
            }
        }

        // The panel says what it is and which section it is showing.
        assert!(words(crate::settings::MCP).contains("SETTINGS"));

        // The colour is drawn as a block of itself, which is the only way a
        // palette can be read.
        let out = render_settings(
            &a_panel_on(&config, crate::settings::APPEARANCE),
            1400.0,
            1200.0,
            None,
        );
        let wanted = [0x12 as f32 / 255.0, 0x34 as f32 / 255.0, 0x56 as f32 / 255.0, 1.0];
        assert!(
            out.scene.rects.iter().any(|rect| rect.rgba() == wanted),
            "no swatch in the accent's own colour"
        );

        // The card the cursor is on carries the focus border and the mark down
        // its edge. Not a band: this section is cards now, and a filled strip
        // nine lines tall is a highlight nobody can read through.
        let panel = a_panel_on(&config, crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 1200.0, None);
        let row = out
            .layout
            .settings_rows
            .iter()
            .find(|(index, side, _)| *index == panel.cursor() && *side == panel.side())
            .map(|(_, _, row)| *row)
            .expect("the cursor's row is on screen");
        let card = [row.x, row.y, row.w, row.h - GAP];
        assert!(
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == out.skin.edge_focus
                    && rect.extra()[3] >= 1.0
                    && (x - card[0]).abs() < 0.01
                    && (y - card[1]).abs() < 0.01
                    && (w - card[2]).abs() < 0.01
                    && h > row.h * 0.5
            }),
            "the card the keys are on has no border"
        );
        assert!(
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == out.skin.edge_focus
                    && (x - row.x).abs() < 0.01
                    && (y - row.y).abs() < 0.01
                    && (w - MARK_W).abs() < 0.01
                    && h > row.h * 0.5
            }),
            "the card the keys are on has no mark down its edge"
        );
        assert!(
            !out.scene
                .rects
                .iter()
                .any(|rect| rect.rgba() == out.skin.strip
                    && rect.xywh() == [row.x, row.y, row.w, row.h]),
            "a card nine lines tall is banded"
        );
    }

    /// The agent's own numbers are tracks like every other number on the panel,
    /// and the number beside one is drawn whole.
    ///
    /// The context window is seven digits at the top of its range, which is two
    /// more than the value column beside a track used to hold: a slider reading
    /// `10485\u{2026}` at the end anybody would drag it to says nothing at all.
    #[test]
    fn the_agent_s_context_window_is_a_track_with_its_number_beside_it() {
        let agent = crate::agent::Agent {
            env: vec![
                (
                    String::from(crate::agent::ENDPOINT),
                    String::from("http://localhost:8080/v1"),
                ),
                (String::from(crate::agent::CTX), String::from("1048576")),
            ],
            ..an_agent()
        };
        let mut panel = Settings::open(
            &Config::default(),
            Some(std::path::Path::new("/home/hec/.config/noob/no0b.conf")),
            agent,
        );
        let at = panel
            .section_names()
            .iter()
            .position(|name| *name == crate::settings::AGENT)
            .expect("the agent section");
        panel.choose(at);

        // Tall enough for the whole section: every track has to be drawn to
        // be checked, and this is about what a number looks like rather than
        // about how a short window scrolls.
        let out = render_settings(&panel, 1400.0, 2000.0, None);
        assert!(
            text_of(&out.scene).contains("1048576"),
            "the context window is not drawn as the number it is: {}",
            text_of(&out.scene)
        );

        // And every number among them is a track, so the maximum concurrency
        // is a place to drop the pointer rather than a number to type. The
        // ones that are a name out of a list have no track to draw.
        for key in crate::agent::OWNED {
            let listed = crate::settings::AGENT_SETTINGS.iter().any(|(known, kind)| {
                *known == key && matches!(kind, crate::settings::Kind::Choice(_))
            });
            if listed {
                continue;
            }
            let (index, side) = panel
                .rows()
                .iter()
                .enumerate()
                .find_map(|(at, row)| {
                    Side::ALL.into_iter().find_map(|side| {
                        matches!(crate::settings::control(row, side), Some(crate::settings::Row::Setting { key: k, .. }) if *k == key)
                            .then_some((at, side))
                    })
                })
                .unwrap_or_else(|| panic!("{key} is not on the agent section"));
            assert!(
                out.layout
                    .settings_tracks
                    .iter()
                    .any(|(row, half, _)| *row == index && *half == side),
                "{key} is not drawn as a track"
            );
        }
    }

    /// The palette is a grid: more than one colour to a row, each one hit where
    /// its own block is drawn, and a press on the second column is that colour
    /// and not the first one on the row.
    ///
    /// It was one colour per row, so the row index was the colour. Nothing else
    /// on the panel is more than one control wide, which is why the cells carry
    /// their place along the row as well as the row.
    #[test]
    fn a_press_on_the_grid_lands_on_the_colour_under_it() {
        let config = Config::parse("accent = #123456");
        let mut panel = a_panel_on(&config, crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 1200.0, None);
        let layout = &out.layout;

        // The row the accent is on carries more than one colour, which is the
        // whole of what makes this a grid.
        let row = layout
            .settings_cells
            .iter()
            .find(|(row, cell, _)| {
                panel.swatch(*row, *cell).is_some_and(|it| it.key == "accent")
            })
            .map(|(row, ..)| *row)
            .expect("the accent is on the grid");
        let on_the_row: Vec<(usize, Panel)> = layout
            .settings_cells
            .iter()
            .filter(|(at, ..)| *at == row)
            .map(|(_, cell, panel)| (*cell, *panel))
            .collect();
        assert!(
            on_the_row.len() > 1,
            "one swatch to a row is the list again: {on_the_row:?}"
        );
        // Side by side along a line, left to right and never overlapping, and
        // the one after the end of a line starts again at the left edge of the
        // card one line lower.
        for pair in on_the_row.windows(2) {
            let (a, b) = (pair[0].1, pair[1].1);
            match (a.y - b.y).abs() < 0.01 {
                true => assert!(a.x + a.w <= b.x + 0.01, "{pair:?} share a column"),
                false => {
                    assert!(b.y > a.y, "{pair:?} run back up the card");
                    assert!(b.x <= a.x + 0.01, "{pair:?} did not start a line");
                }
            }
        }

        // The second column is the second colour, not the first: this is the
        // press a full width row per colour got wrong.
        let (first, second) = (on_the_row[0], on_the_row[1]);
        let (x, y) = middle(second.1);
        assert_eq!(layout.hit(x, y), Some(Hit::SettingsSwatch(row, second.0)));
        assert_ne!(
            layout.hit(x, y),
            Some(Hit::SettingsSwatch(row, first.0)),
            "the second column answers as the first"
        );
        assert!(panel.pick(row, second.0));
        let wanted = panel.swatch(row, second.0).expect("the second colour").key;
        let says = panel.says();
        assert!(says.contains(wanted), "the footer says {says}");
        let out = render_settings(&panel, 1400.0, 1200.0, None);
        assert!(
            text_of(&out.scene).contains(wanted),
            "the panel never says which key writes the pressed colour"
        );

        // Each cell draws a block of its own colour inside itself. Looked for
        // inside the cell rather than anywhere on screen: the panel's own
        // colour is a swatch as well as the fill of half the boxes on the
        // panel, so the first rect wearing it is not this one.
        for (cell, at) in &on_the_row {
            let colour = panel.swatch(row, *cell).expect("a colour");
            assert!(
                out.scene.rects.iter().any(|rect| {
                    let [x, y, ..] = rect.xywh();
                    rect.rgba() == swatch(colour.rgb) && at.contains(x + 0.5, y + 0.5)
                }),
                "{} is not drawn as itself inside {at:?}",
                colour.key
            );
        }
    }

    /// Item G2: the palette is drawn under the control that wrote it, and says
    /// which theme those colours belong to.
    ///
    /// "colors as theme groups i did not saw on the setup... i just sawe many
    /// colors... so i dont know". `theme` was drawn with the sizes, several rows
    /// and a heading above the grid, so the block was a wall of swatches with
    /// nothing on it saying where they came from. The control is the first thing
    /// over the colours now, with one line under it naming the theme that set
    /// them, and every block of colour is drawn with what it paints beside it.
    #[test]
    fn the_palette_is_drawn_under_the_theme_that_set_it() {
        for name in crate::config::THEMES {
            let config = Config::parse(&format!("theme = {name}"));
            let panel = a_panel_on(&config, crate::settings::APPEARANCE);
            let out = render_settings(&panel, 1400.0, 1200.0, None);
            let at = panel
                .rows()
                .iter()
                .position(|row| {
                    matches!(row, crate::settings::Row::Card(card)
                        if card.fields.iter().any(|field| matches!(
                            field.holds.as_ref(),
                            crate::settings::Row::Setting { key, .. } if *key == "theme"
                        )))
                })
                .expect("the theme card");
            let row = out
                .layout
                .settings_rows
                .iter()
                .find(|(index, _, _)| *index == at)
                .map(|(_, _, row)| *row)
                .expect("the theme row is on screen");
            // The grid under the card, which is every swatch below it: the
            // two backgrounds have a card of their own up beside the
            // transparencies, and those are not what this is about.
            let first = out
                .layout
                .settings_cells
                .iter()
                .map(|(_, _, cell)| cell.y)
                .filter(|y| *y > row.y)
                .fold(f32::MAX, f32::min);
            assert!(first < f32::MAX, "no swatch is drawn under the card at all");
            assert!(
                row.y + row.h <= first + 0.01,
                "the theme control is drawn at {row:?}, not above the first swatch at {first}"
            );
            // And the line under it names the theme that is really set, rather
            // than saying the colours came from somewhere in general.
            let text = text_of(&out.scene);
            assert!(
                text.contains(&format!("the {name} theme set these")),
                "the palette does not say whose colours it is showing: {text}"
            );
            // Every swatch on screen is drawn with what it paints beside it,
            // inside its own cell: a block of colour on its own is what he could
            // not read anything out of.
            for (grid, cell, box_) in &out.layout.settings_cells {
                let colour = panel.swatch(*grid, *cell).expect("a colour");
                let words = out
                    .scene
                    .texts
                    .iter()
                    .flat_map(|text| {
                        text.runs
                            .iter()
                            .map(move |run| (text.at, run.text.trim_end_matches('\u{2026}')))
                    })
                    .filter(|(place, _)| box_.contains(place.x + 1.0, place.y + 1.0))
                    .find(|(_, said)| !said.is_empty())
                    .map(|(_, said)| String::from(said))
                    .unwrap_or_else(|| panic!("{} is drawn with no label", colour.key));
                assert!(
                    colour.about.starts_with(&words),
                    "{} is labelled {words:?}, which is not {:?}",
                    colour.key,
                    colour.about
                );
                assert_ne!(words, colour.key, "{} is drawn as its own key", colour.key);
            }
        }
    }

    /// The palette reflows with the card it stands in, and the model counts it
    /// at the width it is really drawn in.
    ///
    /// It was three colours to a row whatever the window was, because the model
    /// was never handed a width. A grid measured at one height and drawn at
    /// another is every press below it on the wrong row, so the two read the
    /// same number here: the cells drawn are all the cells, they are inside the
    /// card, and a wider window puts more of them on a line.
    #[test]
    fn the_palette_reflows_with_the_window_and_is_counted_where_it_is_drawn() {
        let mut across = Vec::new();
        for (w, h) in [(2200.0, 1400.0), (1400.0, 1200.0), (700.0, 1200.0), (460.0, 900.0)] {
            let mut panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
            let shape = render_settings(&panel, w, h, None);
            let rows = shape.layout.settings_capacity(13.0);
            let cols = shape.layout.settings_entry_columns(PANE_TEXT.1);
            let at = panel
                .rows()
                .iter()
                .position(|row| {
                    matches!(row, crate::settings::Row::Palette(palette)
                        if palette.title == "THE TOOL MARKS")
                })
                .expect("the tools");
            // Scroll the card's own top line to the top of the window: the
            // scroll counts rows of text now, and the assertions below want
            // the card whole on screen.
            let top: usize = panel.heights(cols).iter().take(at).sum();
            while panel.first() < top && panel.scroll(1, true, rows, cols) {}
            let out = render_settings(&panel, w, h, None);
            let Some(crate::settings::Row::Palette(palette)) = panel.row(at) else {
                panic!("the tools are not a palette card");
            };
            let cells: Vec<Panel> = out
                .layout
                .settings_cells
                .iter()
                .filter(|(index, ..)| *index == at)
                .map(|(_, _, box_)| *box_)
                .collect();
            assert_eq!(
                cells.len(),
                palette.cells.len(),
                "{w}x{h}: the card drew {} of {} colours",
                cells.len(),
                palette.cells.len()
            );
            let row = out
                .layout
                .settings_rows
                .iter()
                .find(|(index, ..)| *index == at)
                .map(|(_, _, row)| *row)
                .expect("the card is on screen");
            let card = settings_card(row, Text::line_for(PANE_TEXT.0));
            for box_ in &cells {
                assert!(
                    box_.y >= card.y - 0.01 && box_.y + box_.h <= card.y + card.h + 0.01,
                    "{w}x{h}: {box_:?} is outside {card:?}"
                );
            }
            // How many share the first line, which is what the width decides.
            let first = cells[0].y;
            across.push(cells.iter().filter(|box_| (box_.y - first).abs() < 0.01).count());
            // And the row is as tall as the model said it was, whatever that
            // came to at this width.
            let lines = crate::settings::lines(panel.row(at).expect("the card"), cols);
            assert_eq!(
                row.h,
                lines as f32 * Text::line_for(PANE_TEXT.0),
                "{w}x{h}: the row is not the height the model counted"
            );
        }
        assert!(
            across.windows(2).all(|pair| pair[0] >= pair[1]),
            "the palette does not narrow with the window: {across:?}"
        );
        assert!(across[0] > across[3], "it never reflows at all: {across:?}");
    }

    /// Item H5: every theme is drawn by name and every one of them is a press.
    ///
    /// "i only see noob matrix the others i asked are absent". The control was
    /// one box holding one word, and the only way to find out there were three
    /// was to press it twice. All three stand side by side now, the one the
    /// window is wearing is filled, and each of them answers where it is drawn.
    #[test]
    fn every_theme_is_drawn_by_name_and_can_be_pressed() {
        for wearing in crate::config::THEMES {
            let config = Config::parse(&format!("theme = {wearing}"));
            let panel = a_panel_on(&config, crate::settings::APPEARANCE);
            let out = render_settings(&panel, 1400.0, 1200.0, None);
            let options = &out.layout.settings_choices;
            // The three presets, and custom in the column beside them.
            assert_eq!(options.len(), crate::config::THEMES.len() + 1);
            let words = text_of(&out.scene);
            for name in crate::config::THEMES {
                assert!(words.contains(name), "{name} is not drawn: {words}");
            }
            assert!(words.contains("custom"), "custom is not drawn: {words}");
            for (index, side, option, at) in options {
                // Each name inside its own box, so a name and the box that
                // writes it cannot come apart.
                let said = out
                    .scene
                    .texts
                    .iter()
                    .flat_map(|text| text.runs.iter().map(move |run| (text.at, run.text.clone())))
                    .find(|(place, said)| {
                        at.contains(place.x + 1.0, place.y + 1.0) && !said.is_empty()
                    })
                    .map(|(_, said)| said)
                    .unwrap_or_else(|| panic!("option {option} is drawn with no name"));
                assert_eq!(said.trim(), option_name(*side, *option));
                // And it is the press: the middle of the box answers as that
                // option and no other.
                let (x, y) = middle(*at);
                assert_eq!(
                    out.layout.hit(x, y),
                    Some(Hit::SettingsChoice(*index, *side, *option))
                );
                // Which one the window is wearing is said by the fill, since a
                // row of identical boxes says nothing about what is set.
                let filled = out.scene.rects.iter().any(|rect| {
                    let [x, y, w, _] = rect.xywh();
                    rect.rgba() == out.skin.button
                        && (x - at.x).abs() < 0.01
                        && (y - at.y).abs() < 0.01
                        && (w - at.w).abs() < 0.01
                });
                assert_eq!(
                    filled,
                    option_name(*side, *option) == wearing,
                    "{wearing}: option {option} is filled as {filled}"
                );
            }
        }
    }

    /// The way back to the defaults is a button in a card's footer, drawn in the
    /// colour this window keeps for anything that loses work, and it asks once
    /// before it acts.
    #[test]
    fn the_restore_is_a_danger_button_in_its_own_card_and_asks_first() {
        let mut panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
        let at = panel
            .rows()
            .iter()
            .position(|row| {
                matches!(row, crate::settings::Row::Card(card)
                    if card.does == Some(crate::settings::Doing::Restore))
            })
            .expect("the card that puts it back");
        // It is the last card of the section: it takes back everything above it.
        assert_eq!(at, panel.rows().len() - 1);
        let shape = render_settings(&panel, 1400.0, 1200.0, None);
        let rows = shape.layout.settings_capacity(13.0);
        let cols = shape.layout.settings_entry_columns(PANE_TEXT.1);
        // The scroll counts rows of text: walk the card's own top line up to
        // the window, which the clamp then holds whole on the last screenful.
        let top: usize = panel.heights(cols).iter().take(at).sum();
        while panel.first() < top && panel.scroll(1, true, rows, cols) {}
        let out = render_settings(&panel, 1400.0, 1200.0, None);
        let (act, box_) = out
            .layout
            .settings_acts
            .iter()
            .find(|(index, ..)| *index == at)
            .map(|(_, act, box_)| (*act, *box_))
            .expect("the button is on screen");
        assert_eq!(act, Act::Restore);
        // In the footer, at the bottom right of the card and inside it.
        let row = out
            .layout
            .settings_rows
            .iter()
            .find(|(index, ..)| *index == at)
            .map(|(_, _, row)| *row)
            .expect("the card is on screen");
        assert!(box_.y > row.y + row.h * 0.5, "{box_:?} is not in the footer");
        assert!(box_.x + box_.w <= row.x + row.w + 0.01);
        assert!(box_.y + box_.h <= row.y + row.h + 0.01);
        // Outlined in the danger colour and filled with nothing: a delete is
        // never the thing a card is filled for.
        let outlined = |out: &Rendered| {
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, _] = rect.xywh();
                rect.rgba() == out.skin.close_hot
                    && (x - box_.x).abs() < 0.01
                    && (y - box_.y).abs() < 0.01
                    && (w - box_.w).abs() < 0.01
            })
        };
        assert!(outlined(&out), "the restore is not drawn as a danger");
        assert!(
            !out.scene.rects.iter().any(|rect| {
                let [x, ..] = rect.xywh();
                rect.rgba() == out.skin.button && (x - box_.x).abs() < 0.01
            }),
            "the restore is filled like a primary"
        );
        let words = text_of(&out.scene);
        assert!(words.contains("restore"), "{words}");
        assert!(words.contains("comments out the sizes"), "{words}");

        // Armed, it says so on itself as well as on the footer: the second
        // press is the one that acts.
        assert!(panel.uninstall(at).is_none());
        let armed = render_settings(&panel, 1400.0, 1200.0, None);
        assert!(text_of(&armed.scene).contains("sure?"), "the button never asks");
        assert!(outlined(&armed), "the armed button changed kind");
    }

    /// Scrolling the panel moves the cells with the rows they are on. A hit
    /// region left where the row used to be is a press that changes the wrong
    /// colour, and the grid is the one place a row is more than one control.
    #[test]
    fn the_grid_cells_follow_the_list_when_it_scrolls() {
        let mut panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
        // A short window, so the palette is off the bottom until it is scrolled
        // to and the rows on screen change as it moves.
        let (w, h) = (1400.0, 520.0);
        let out = render_settings(&panel, w, h, None);
        let rows = out.layout.settings_capacity(13.0);
        let cols = out.layout.settings_entry_columns(PANE_TEXT.1);
        assert!(rows > 2, "the list is too short to scroll: {rows}");
        let mut seen = 0;
        for _ in 0..40 {
            let out = render_settings(&panel, w, h, None);
            let layout = &out.layout;
            for (row, cell, at) in &layout.settings_cells {
                // Where it is drawn: the block of colour is inside the cell.
                let colour = panel
                    .swatch(*row, *cell)
                    .unwrap_or_else(|| panic!("no colour at {row}/{cell}"));
                let block = out
                    .scene
                    .rects
                    .iter()
                    .find(|rect| {
                        rect.rgba() == swatch(colour.rgb)
                            && at.contains(rect.xywh()[0] + 0.5, rect.xywh()[1] + 0.5)
                    })
                    .unwrap_or_else(|| panic!("{} is not drawn in its cell", colour.key));
                assert!(
                    layout
                        .settings_list
                        .contains(block.xywh()[0], block.xywh()[1]),
                    "{} is drawn outside the list",
                    colour.key
                );
                // And where it is pressed: the same cell answers for it.
                let (x, y) = middle(*at);
                assert_eq!(
                    layout.hit(x, y),
                    Some(Hit::SettingsSwatch(*row, *cell)),
                    "the cell moved out from under its own colour"
                );
                seen += 1;
            }
            if !panel.scroll(1, true, rows, cols) {
                break;
            }
        }
        assert!(seen > 20, "the palette never came into view: {seen}");
    }

    /// The footer says what the keys will do to the row under the cursor, and
    /// says a refused write instead when there is one. A panel that writes a
    /// file has to say when the file said no.
    #[test]
    fn the_footer_carries_the_keys_and_then_the_trouble() {
        let config = Config::default();
        let mut panel = a_settings_panel(&config);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        assert!(text_of(&out.scene).contains(&panel.says()), "{}", panel.says());

        panel.say_trouble(String::from("cannot write it"));
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let text = text_of(&out.scene);
        assert!(text.contains("cannot write it"), "{text}");
        assert!(!text.contains(panel.hint()), "the trouble and the keys share a row");
        let said = out
            .scene
            .texts
            .iter()
            .flat_map(|t| t.runs.iter())
            .find(|run| run.text.contains("cannot write it"))
            .expect("the trouble is drawn");
        assert_eq!(said.color, Some(out.skin.bad), "trouble is not marked as trouble");
    }
}
