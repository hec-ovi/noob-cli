//! Layout, hit regions, and turning state into a scene.
//!
//! One surface carved into three spaces, never several OS windows. The window
//! has no system chrome, so the title bar, its three buttons, the tab strips,
//! the scrollbars and the resize edges are all rectangles here and hit regions
//! in [`Layout`]. Drawing and hit testing take the same numbers from the same
//! place, which is the only way they can never disagree.
//!
//! Every view is a tab in one of the three spaces and can be dragged into
//! another; [`crate::dock`] owns that arrangement, and this module only asks it
//! where things are.
//!
//! The window has three shapes. Open, it is three spaces. Shaded, it is one
//! strip carrying [`State::headline`] and nothing else, the way Winamp collapsed
//! to its title; double-click the bar to go between those two. Before a folder
//! has been chosen it is the picker and nothing else, because there is no agent
//! to arrange panes around yet.

use noob_draw::{Panel, Rect, Run, Scene, Text};

use crate::dock::{Dock, Space, View};
use crate::icons;
use crate::menu::Menu;
use crate::monitor::{Gauge, Monitor};
use crate::picker::{Picker, Row as PickerRow};
use crate::settings::{Kind, Row as SettingRow, Settings};
use crate::skin::Skin;
use crate::state::{State, TodoState, Tone};

pub const TITLE_H: f32 = 30.0;
const INPUT_H: f32 = 36.0;
const TAB_H: f32 = 22.0;
const RESIZE_EDGE: f32 = 6.0;
const GAP: f32 = 6.0;
const PAD: f32 = 9.0;
/// Columns the file view spends on its line-number gutter, on every row.
const GUTTER: usize = 4;
const SMALL: f32 = 12.0;
const SCROLL_W: f32 = 4.0;
const BUTTON_W: f32 = 26.0;
/// The square at the left end of the title strip that the orb is drawn in.
///
/// The strip's text starts after this, so the orb sits in a slot of its own
/// instead of over the name, and the strip reads
/// `[orb] NO0B \u{25b8} version` left to right. The orb sizes itself to whatever
/// square it is handed, so this is the only number that decides how big it is.
pub const ORB_W: f32 = TITLE_H;
const LABEL_COLUMNS: usize = 9;
/// A gauge is a block of dots: twenty across and four down is 0 to 100 percent,
/// so one row is 25 percent and one dot is 1.25.
///
/// Wide and short, which is the shape the panes were asked for. Eight by five
/// was the shape before and it stood the hardware pane on end: six readings each
/// five rows tall is a column of stacks, tall and narrow, in a pane that has
/// width to spare and no height. Twenty dots to a row also puts a usable
/// resolution on one row, a dot being a percent and a quarter, so a reading
/// climbing under load moves dot by dot instead of in fifths of a row.
const DOT_COLUMNS: usize = 20;
const DOT_ROWS: usize = 4;
/// How much larger the number beside a block is than the label. One: the same
/// size as every other glyph in the window.
///
/// It was one and a half, and at that size the readings were the loudest thing on
/// screen, which is not what a monitor is for. The metric's own tint is what says
/// a number is the thing being read. The arithmetic that caps it against the room
/// beside the block is kept, so raising this again cannot put a reading over the
/// edge of a narrow pane.
const BIG_READING: f32 = 1.0;
/// Rows the CONTEXT pane spends on its header before its readings start: the
/// phase, the model and the workspace. They stay put while the readings under
/// them scroll.
const CONTEXT_HEAD: usize = 3;
/// The smallest a dot shrinks to, across or down, when a pane has more readings
/// than room. Below this the block stops reading as a block, so it is not drawn:
/// too tall for its rows and they scroll off, too narrow for its columns and the
/// pane draws numbers alone. A reading that scrolled off is honest and a number
/// with no block is honest; a smear is not.
const SMALL_DOT: f32 = 4.0;
const PROMPT_COLUMNS: usize = 2;
const INPUT_PAD: f32 = 6.0;
/// How far the 45 degree cut reaches along each edge of a panel's top-right
/// corner. One corner, so the shape reads as a mark rather than as a rounded
/// box, and always the same corner so two panels side by side still line up.
const CUT: f32 = 10.0;
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
/// How far a scrollbar sits in from the right edge of the pane it belongs to.
const SCROLL_GAP: f32 = 2.0;
/// One row of a menu. Taller than a tab: a tab is read, a menu row is aimed at,
/// and 22 pixels is already tight for a pointer.
const MENU_ROW_H: f32 = 20.0;
/// The margin around a menu's rows, top and bottom and on either side of a
/// label. Also what keeps the first row off the pointer that opened it.
const MENU_PAD: f32 = 5.0;
/// Columns every menu row leaves in front of its label for an icon, whether it
/// has one or not, so labels line up in a column instead of stepping in and out
/// with whichever rows happen to be marked.
const MENU_GUTTER: usize = 2;
/// Columns a row of the file explorer spends before its name: the type icon and
/// the space after it.
const ROW_ICON_COLUMNS: usize = 2;
/// Columns a row spends on the changed mark, when it carries one.
const ROW_MARK_COLUMNS: usize = 2;
/// The widest the explorer column gets, however long the names in it are. Past
/// this the list is spending the pane on directory prefixes nobody is reading.
const LIST_MAX_COLUMNS: usize = 20;
/// The narrowest it gets: an icon and enough characters to tell two names apart.
const LIST_MIN_COLUMNS: usize = 9;
/// What the file keeps whatever the list wants: the line-number gutter and
/// enough code beside it to read a line. The file view usually lives in the
/// right-hand column, which is about 35 columns wide in a window at its minimum
/// size, so a list sized to its own content alone would leave the thing being
/// looked at unreadable. At that size this floor wins and the list goes below
/// [`LIST_MIN_COLUMNS`], because the file is what is being read.
const DIFF_MIN_COLUMNS: usize = GUTTER + 20;
/// How wide the folder picker gets, in pane columns. Wide enough for a deep
/// path and no wider: folder names in a 200 column box are one word per row with
/// the rest of it empty.
const PICKER_COLUMNS: usize = 64;
/// Where the two dividers sit on a window nobody has dragged one in: the left
/// column takes this much of the width, and the top right space this much of the
/// right column's height.
///
/// Defaults, not constants. Both are dragged, both are carried in on [`Shape`],
/// and the settings file remembers where they were left.
pub const LEFT_WIDTH: f32 = 0.54;
pub const TOP_HEIGHT: f32 = 0.46;
/// How far either side of the gap between two panes the pointer still counts as
/// being on the divider between them.
///
/// The gap is [`GAP`], six pixels, which is a line you can see and not a target
/// you can hit. This takes the target to fourteen without widening anything that
/// is drawn.
const GRAB: f32 = 4.0;
/// The least height a space can be dragged down to: its tab strip ([`TAB_H`]),
/// the [`PAD`] above and below its content, and the shortest gauge block that
/// still reads as a block ([`DOT_ROWS`] rows of [`SMALL_DOT`]). Fifty-six pixels.
const MIN_SPACE_H: f32 = TAB_H + PAD * 2.0 + DOT_ROWS as f32 * SMALL_DOT;

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
fn held(ratio: f32, room: f32, floor: f32) -> f32 {
    if room <= floor * 2.0 {
        return 0.5;
    }
    let edge = floor / room;
    ratio.clamp(edge, 1.0 - edge)
}

/// Columns the settings panel keeps at the right of a row for its value.
///
/// Wide enough for the longest value on the panel, which is a rate reading like
/// `312 mean, 340 median tok/s`, and no wider: past that the label and the value
/// it belongs to are at opposite ends of the window with nothing between them.
/// A path is longer than this and is clipped, which is what the panel is for
/// rather than what it says.
const SETTING_VALUE_COLUMNS: usize = 28;

/// The rows the picker spends above its list: the heading, the folder it is
/// listing, and what has been typed.
const PICKER_HEAD_ROWS: f32 = 3.0;

/// How far in from the left edge of a picker row its first mark sits.
///
/// A pane's row runs to its own edge because the band behind it is the width of
/// the pane. The picker's band is green and solid, so it needs an edge of its
/// own rather than starting under the glyph.
const PICKER_ROW_PAD: f32 = 5.0;

/// Columns a picker row spends on the mark that opens and shuts it, the mark
/// included and a space after it, and columns a step further into the tree costs.
///
/// Every row reserves the mark's column whether it has a mark or not, so the
/// folder glyphs line up in one column down the list instead of the ones with a
/// plus in front of them standing out of the ones without.
const PICKER_MARK_COLUMNS: usize = 2;
const PICKER_INDENT_COLUMNS: usize = 2;
/// The columns a row keeps for what it says, however deep it sits. Past this the
/// indent stops growing: a name at depth twelve pushed off the right of the box
/// is a row that says nothing.
const PICKER_LABEL_COLUMNS: usize = 12;

/// What the picker says on its button, and the whole of what it says.
///
/// It used to spell out the folder that would be opened, which made the button
/// as wide as a path and made it change width every time the cursor moved. The
/// path is already written above the list.
const PICKER_OPEN_LABEL: &str = "Open";

/// What the button beside it says, in each of the two lists it swaps between.
///
/// Both are drawn in a box sized for the longer of the two, so pressing it does
/// not change the width of the thing that was just pressed.
const PICKER_SESSIONS_LABEL: &str = "Sessions";
const PICKER_FOLDERS_LABEL: &str = "Folders";

/// How much taller that button is than the line of text in it, on each side.
///
/// A button reads as a button because there is room around what it says. The
/// same string with a hairline drawn around it reads as a label with a box.
const PICKER_OPEN_PAD: f32 = 5.0;

/// How tall the picker's list is allowed to get, in rows, and how short.
///
/// Bounds on the window, not on the folder. The box takes as many rows as there
/// is room for between these two and then holds that height whatever it is
/// listing. It used to take as many rows as the folder had entries, so walking
/// from a folder with three subfolders into one with forty resized the dialog
/// and recentred it under the pointer, moving every row while the pointer was
/// still on one of them. A short folder now gets empty rows under its list,
/// which is the cheaper of the two: a box that does not move is worth more than
/// a box with no whitespace in it.
const PICKER_MIN_ROWS: usize = 6;
const PICKER_MAX_ROWS: usize = 24;

/// How tall the Open button is for text of this line height.
fn picker_open_h(line: f32) -> f32 {
    line + PICKER_OPEN_PAD * 2.0
}

/// What the picker keeps below its list: the line of keys, a gap, and the
/// button. One answer, so the box that is measured and the rows that are drawn
/// into it cannot disagree about where the bottom is.
fn picker_foot(line: f32) -> f32 {
    line + GAP + picker_open_h(line)
}

/// How wide the mark down the left of the selected row is. A tab's accent runs
/// along its top edge because a strip is read left to right; a row is entered
/// from the left, so its accent runs down that edge instead.
const MARK_W: f32 = 2.0;

/// How wide the caret standing in the gap a dragged tab would drop into is.
///
/// Three, one more than every hairline in the window, because it is the one mark
/// that has to be read while something else is moving.
const CARET_W: f32 = 3.0;

/// The version this build was cut from, and the version the title strip reads.
///
/// It comes from the crate rather than from a string typed into the strip, so
/// the window cannot claim a release the package does not carry. The two cargo
/// workspaces, the CLI and this window, set the same number and ship as one
/// release.
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The commit the build came from, as ` abc1234`, with a `+` when the tree had
/// uncommitted changes, and nothing at all when there was no repository to ask.
///
/// `build.rs` stamps [`VERSION`] and the commit into one string. The strip takes
/// the version from the crate, so what is left to take from the stamp is the
/// part after it. A version alone cannot tell two test builds of the same
/// release apart, which is what the commit is for.
fn build_commit() -> &'static str {
    env!("NO0B_BUILD").strip_prefix(VERSION).unwrap_or("")
}

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
    /// A row of the folder picker, by position in its list.
    PickerRow(usize),
    /// The mark in front of a folder on that row, which puts what is inside it
    /// into the list under it. Its own region inside the row and tested before
    /// it: pressing the mark opens the folder, pressing the row selects it, and
    /// one region for both would make every press do both things.
    PickerMark(usize),
    /// The button that confirms the row the cursor is on, which is how the
    /// mouse chooses a folder without a keyboard.
    PickerOpen,
    /// The button beside it, which swaps the list between the folders and the
    /// sessions the agent has already written. The only way in to a past
    /// conversation from a window that has just opened.
    PickerSessions,
    /// The picker's box, away from any row. Swallowed, so a press on its margin
    /// does not read as a press on the window behind it.
    Picker,
    /// A row of the settings panel, by position in its list. Puts the cursor
    /// there and nothing else: a click anywhere on a row that also changed the
    /// setting would change one every time the pointer missed the value.
    SettingsRow(usize),
    /// The value at the end of that row, which is the control. Clicking it is
    /// the same nudge the right arrow is.
    SettingsValue(usize),
    /// The mark that closes the panel, for a pointer with no Escape key handy.
    SettingsClose,
    /// The panel's box, away from any row. Swallowed, like the picker's.
    Settings,
    /// The band between the left column and the right one. Dragging it decides
    /// how much of the width each column gets.
    ColumnDivider,
    /// The band between the two right hand spaces, which decides how the right
    /// column's height is shared between them.
    RowDivider,
}

impl Hit {
    /// The space a drop here would move a view into.
    pub fn space(self) -> Option<Space> {
        match self {
            Hit::Tab(_, space)
            | Hit::TabsLeft(space)
            | Hit::TabsRight(space)
            | Hit::Body(space)
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
    /// Off the window entirely, which takes the view out of it.
    Out,
    /// Somewhere in the window that is not a space: the title strip, the
    /// prompt, the margin between two panes. Nothing happens.
    Nowhere,
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
    fn none() -> Divider {
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

    /// One per [`Space`], in `Space::ALL` order.
    pub spaces: [Placed; 3],
    /// The two dividers, both empty in every shape that has no panes and beside
    /// any space that is standing empty or folded away.
    pub column_divider: Divider,
    pub row_divider: Divider,
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
    /// The button beside it, which swaps the list between folders and saved
    /// sessions. Empty when there is no room for it beside Open, which is the
    /// only reason a button in this box ever goes away.
    pub picker_sessions: Panel,

    /// True while the settings panel is up, which is the third shape of its own:
    /// the panes and the prompt are still there behind it, and the panel covers
    /// the lot, because it is a takeover rather than a window over a window.
    pub in_settings: bool,
    /// Its box, its list, one panel per visible row, the value at the end of
    /// each of those rows, and the mark that closes it. All empty when it is
    /// not up.
    pub settings: Panel,
    pub settings_list: Panel,
    pub settings_rows: Vec<(usize, Panel)>,
    pub settings_values: Vec<(usize, Panel)>,
    pub settings_close: Panel,

    /// The floating layer. The open menu's box, and one panel per row on
    /// screen, both empty when no menu is open. Drawn last and hit tested
    /// first.
    ///
    /// Each row carries its place in the menu, the way the picker's and the
    /// settings panel's rows do, because the widget list at the foot of the
    /// menu scrolls: the third panel down is not always the third row.
    pub menu: Panel,
    pub menu_rows: Vec<(usize, Panel)>,
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
    pub column: f32,
    /// The size and column width the panes are drawn at. The explorer's rows are
    /// pane text, so their height and the width of the column they sit in are
    /// the pane's, not the title bar's.
    pub pane_size: f32,
    pub pane_column: f32,
    /// How tall the prompt is. It grows with what has been typed, so it is an
    /// input to the layout rather than a constant.
    pub input_h: f32,
    /// How much of the body's width the left column takes.
    pub left_width: f32,
    /// How much of the right column's height the top space takes.
    ///
    /// Both are inputs for the same reason `input_h` is: they are dragged while
    /// the window is up and read back out of the settings file at the next
    /// launch. Held here rather than trusted: [`held`] keeps whatever arrives
    /// inside what the window can actually draw, so neither a file with a silly
    /// number in it nor a drag thrown past the edge can collapse a space.
    pub top_height: f32,
}

fn nowhere() -> Panel {
    Panel::new(0.0, 0.0, 0.0, 0.0)
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
        let buttons = [
            Panel::new(width - BUTTON_W * 3.0, 0.0, BUTTON_W, TITLE_H),
            Panel::new(width - BUTTON_W * 2.0, 0.0, BUTTON_W, TITLE_H),
            Panel::new(width - BUTTON_W, 0.0, BUTTON_W, TITLE_H),
        ];
        // Placed before the shape is decided, because the overlay is above the
        // window in both shapes: a menu that survived a double click on the
        // title bar would still be hit tested and would have nothing drawn.
        let (menu, menu_rows) = match shape.menu {
            Some(menu) => place_menu(menu, shape.column, width, height),
            None => (nowhere(), Vec::new()),
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
                spaces: [empty_placed(), empty_placed(), empty_placed()],
                column_divider: Divider::none(),
                row_divider: Divider::none(),
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
                picker_sessions: nowhere(),
                in_settings: false,
                settings: nowhere(),
                settings_list: nowhere(),
                settings_rows: Vec::new(),
                settings_values: Vec::new(),
                settings_close: nowhere(),
                menu,
                menu_rows,
            };
        }

        // The picker is the whole window while it is up, and every other region
        // collapses to nothing the way it does when the window is shaded: a
        // stale hit region left behind here would let a click reach a pane that
        // has no agent behind it.
        if let Some(picker) = shape.picker {
            let places = place_picker(rest.inset(GAP), shape, picker);
            return Layout {
                width,
                height,
                shaded: false,
                title,
                minimize: buttons[0],
                maximize: buttons[1],
                close: buttons[2],
                spaces: [empty_placed(), empty_placed(), empty_placed()],
                column_divider: Divider::none(),
                row_divider: Divider::none(),
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
                picker_sessions: places.sessions,
                in_settings: false,
                settings: nowhere(),
                settings_list: nowhere(),
                settings_rows: Vec::new(),
                settings_values: Vec::new(),
                settings_close: nowhere(),
                menu,
                menu_rows,
            };
        }

        // The settings panel takes the whole surface under the title strip, and
        // every pane region collapses the way it does for the picker. A takeover
        // rather than a box over the panes: half a window of live panes behind a
        // list of settings is two scroll regions over each other, and a click
        // that missed the panel would land in a transcript nobody can see.
        if let Some(panel) = shape.settings {
            let places = place_settings(rest.inset(GAP), shape, panel);
            return Layout {
                width,
                height,
                shaded: false,
                title,
                minimize: buttons[0],
                maximize: buttons[1],
                close: buttons[2],
                spaces: [empty_placed(), empty_placed(), empty_placed()],
                column_divider: Divider::none(),
                row_divider: Divider::none(),
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
                picker_sessions: nowhere(),
                in_settings: true,
                settings: places.box_,
                settings_list: places.list,
                settings_rows: places.rows,
                settings_values: places.values,
                settings_close: places.close,
                menu,
                menu_rows,
            };
        }

        let (body, input) = rest.split_bottom(shape.input_h.max(INPUT_H).min(rest.h));
        let body = body.inset(GAP);

        // An empty space gives its room away rather than leaving a hole, and a
        // divider with an empty space on one side of it has nothing left to
        // divide, so it goes with the room.
        let has = |space: Space| !shape.dock.slot(space).is_empty();
        let column_floor = min_column_w(shape.pane_column);
        let (left, right, column_divider) =
            if has(Space::Left) && (has(Space::TopRight) || has(Space::BottomRight)) {
                let room = (body.w - GAP).max(0.0);
                let taken = (room * held(shape.left_width, room, column_floor)).floor();
                let (left, rest) = body.split_left(taken);
                let band = Panel::new(left.x + left.w - GRAB, body.y, GAP + GRAB * 2.0, body.h);
                (
                    left,
                    Panel::new(rest.x + GAP, rest.y, (rest.w - GAP).max(1.0), rest.h),
                    Divider {
                        band,
                        track: body,
                        floor: column_floor,
                    },
                )
            } else if has(Space::Left) {
                (body, nowhere(), Divider::none())
            } else {
                (nowhere(), body, Divider::none())
            };

        let folded = |space: Space| shape.dock.slot(space).folded;
        let (top, bottom, row_divider) = match (has(Space::TopRight), has(Space::BottomRight)) {
            (false, false) => (nowhere(), nowhere(), Divider::none()),
            (true, false) => (right, nowhere(), Divider::none()),
            (false, true) => (nowhere(), right, Divider::none()),
            (true, true) => {
                let room = (right.h - GAP).max(0.0);
                // A folded space is already as short as it goes and its
                // neighbour has taken the rest, so there is nothing between the
                // two to move until it is opened again.
                let (top_h, movable) = match (folded(Space::TopRight), folded(Space::BottomRight)) {
                    (true, _) => (TAB_H, false),
                    (false, true) => ((room - TAB_H).max(TAB_H), false),
                    (false, false) => (
                        (room * held(shape.top_height, room, MIN_SPACE_H))
                            .floor()
                            .max(TAB_H),
                        true,
                    ),
                };
                let top_h = top_h.min(right.h);
                let (top, lower) = right.split_top(top_h);
                let divider = match movable {
                    true => Divider {
                        band: Panel::new(right.x, right.y + top_h - GRAB, right.w, GAP + GRAB * 2.0),
                        track: right,
                        floor: MIN_SPACE_H,
                    },
                    false => Divider::none(),
                };
                (
                    top,
                    Panel::new(lower.x, lower.y + GAP, lower.w, (lower.h - GAP).max(0.0)),
                    divider,
                )
            }
        };

        let place = |space: Space, area: Panel| -> Placed {
            if area.w < 1.0 || area.h < 1.0 {
                return empty_placed();
            }
            let (strip, rest) = area.split_top(TAB_H.min(area.h));
            let slot = shape.dock.slot(space);
            let widths: Vec<usize> = slot
                .views
                .iter()
                .map(|v| v.label().chars().count())
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
            place(Space::Left, left),
            place(Space::TopRight, top),
            place(Space::BottomRight, bottom),
        ];

        // The file view's explorer runs down the left of whichever space is
        // showing it, with the open file in the room that is left.
        let files_in = shape.dock.space_of(View::Files).filter(|space| {
            shape.dock.slot(*space).active() == Some(View::Files)
                && !shape.dock.slot(*space).folded
        });
        let (file_list, file_diff, file_rows) = match files_in {
            Some(space) => place_files(
                spaces[Space::ALL.iter().position(|s| *s == space).unwrap()].body,
                shape,
            ),
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
            picker_sessions: nowhere(),
            in_settings: false,
            settings: nowhere(),
            settings_list: nowhere(),
            settings_rows: Vec::new(),
            settings_values: Vec::new(),
            settings_close: nowhere(),
            menu,
            menu_rows,
        }
    }

    pub fn placed(&self, space: Space) -> &Placed {
        &self.spaces[Space::ALL.iter().position(|s| *s == space).unwrap()]
    }

    /// What is under a point. One place, so a click and the thing it appears to
    /// land on can never come apart.
    pub fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        // The floating layer first. A menu is above the window, so it takes the
        // click even when a window button, a tab or a pane is under it; without
        // this the menu would be drawn over things it could be clicked through
        // onto, which is worse than having no menu.
        for (index, row) in &self.menu_rows {
            if row.contains(x, y) {
                return Some(Hit::MenuRow(*index));
            }
        }
        if self.menu.w >= 1.0 && self.menu.contains(x, y) {
            return Some(Hit::Menu);
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
            if self.picker_open.w >= 1.0 && self.picker_open.contains(x, y) {
                return Some(Hit::PickerOpen);
            }
            if self.picker_sessions.w >= 1.0 && self.picker_sessions.contains(x, y) {
                return Some(Hit::PickerSessions);
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
            // The value before the row it sits in, because it sits inside it.
            for (index, panel) in &self.settings_values {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsValue(*index));
                }
            }
            for (index, panel) in &self.settings_rows {
                if panel.contains(x, y) {
                    return Some(Hit::SettingsRow(*index));
                }
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
        // Before the bodies, because the band is wider than the gap it stands
        // in and so it reaches a little way into the pane on either side. After
        // the tabs and the file rows, which are smaller targets inside those
        // panes: the more particular thing under the pointer wins.
        for (divider, hit) in [
            (self.column_divider, Hit::ColumnDivider),
            (self.row_divider, Hit::RowDivider),
        ] {
            if divider.live() && divider.band.contains(x, y) {
                return Some(hit);
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
    pub fn landing(&self, x: f32, y: f32) -> Landing {
        if x < 0.0 || y < 0.0 || x > self.width || y > self.height {
            return Landing::Out;
        }
        let Some(space) = self.hit(x, y).and_then(Hit::space) else {
            return Landing::Nowhere;
        };
        // On the strip, the drop names a place among the tabs. In the pane below
        // it, it names the space only: a tab dragged into the middle of its own
        // pane and let go has not been put in front of anything, so the order it
        // was in is the order it keeps.
        let at = self
            .placed(space)
            .strip
            .contains(x, y)
            .then(|| self.insertion(space, x));
        Landing::In(space, at)
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

    /// Where a pointer at `x` puts the divider between the columns, as the
    /// fraction [`Shape::left_width`] is.
    ///
    /// The inverse of the arithmetic [`Layout::compute`] lays the columns out
    /// with, off the same box, so the divider lands under the pointer rather
    /// than near it. Held by the same rule as well, so a drag thrown past either
    /// end of the window stops at the floor instead of collapsing a column.
    pub fn column_ratio_at(&self, x: f32) -> f32 {
        let track = self.column_divider.track;
        let room = (track.w - GAP).max(1.0);
        held(
            (x - track.x - GAP * 0.5) / room,
            room,
            self.column_divider.floor,
        )
    }

    /// The same for the divider between the two right hand spaces.
    pub fn row_ratio_at(&self, y: f32) -> f32 {
        let track = self.row_divider.track;
        let room = (track.h - GAP).max(1.0);
        held((y - track.y - GAP * 0.5) / room, room, self.row_divider.floor)
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

    /// How many of the menu's widget list are on screen, for the wheel. Read
    /// off the rows the layout actually placed, so the scroll is bounded by
    /// what is drawn rather than by an arithmetic of its own.
    pub fn menu_capacity(&self, menu: &Menu) -> usize {
        self.menu_rows
            .iter()
            .filter(|(index, _)| *index >= menu.top)
            .count()
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

    /// Where a click in the prompt puts the caret, as a character offset into
    /// the typed text.
    ///
    /// The inverse of the arithmetic [`input_row`] draws the caret with, off
    /// the same box, so the caret lands under the pointer instead of near it.
    /// A click past the end of the text lands at the end, which is why `chars`
    /// is passed in.
    pub fn input_caret(&self, x: f32, y: f32, size: f32, column: f32, chars: usize) -> usize {
        if column <= 0.0 {
            return chars;
        }
        let line = Text::line_for(size);
        let box_ = input_box(self.input, line);
        let columns = columns_in(box_.w, column);
        let row = ((y - box_.y) / line).floor().max(0.0) as usize;
        // Rounded, not floored, so pressing on the right half of a character
        // puts the caret after it, the way a text cursor behaves everywhere.
        let at = ((((x - box_.x) / column).round().max(0.0)) as usize).min(columns);
        // The marker in front of the text owns the first columns of the first
        // row, so a click on it means the start of the text.
        (row * columns + at).saturating_sub(PROMPT_COLUMNS).min(chars)
    }
}

/// Where an open menu's box is, and where each of its rows on screen is inside
/// it.
///
/// Clamped into the window. A menu opened near the right edge or a row from the
/// bottom would otherwise hang off the surface, and the part that hangs off is
/// not merely invisible: no pointer can reach it, so the rows down there cannot
/// be picked at all.
///
/// The widget list gets the same treatment carried one step further. Clamping
/// is enough while the menu is shorter than the window, and a menu with nine
/// widget rows open in a short window is not, so the box takes as many rows as
/// there is room for and the list scrolls through the rest. The top level rows
/// are kept whatever happens: they are the ones the menu was opened for.
fn place_menu(menu: &Menu, column: f32, width: f32, height: f32) -> (Panel, Vec<(usize, Panel)>) {
    let column = column.max(1.0);
    let w = (menu.width_chars() + MENU_GUTTER) as f32 * column + MENU_PAD * 2.0;
    let room = (((height - MENU_PAD * 2.0) / MENU_ROW_H).floor() as usize).max(1);
    let shown = menu.rows.len().min(room);
    // What is left for the list once the top level has had its rows, and where
    // in the list that window starts. Clamped here rather than in the menu, so
    // a wheel that ran past the end does not leave the box half empty.
    let visible = shown.saturating_sub(menu.top);
    let first = menu.first.min(menu.widgets().saturating_sub(visible));
    let h = shown as f32 * MENU_ROW_H + MENU_PAD * 2.0;
    let x = menu.at.0.min(width - w).max(0.0);
    let y = menu.at.1.min(height - h).max(0.0);
    let rows = (0..shown)
        .map(|step| {
            let index = match step < menu.top {
                true => step,
                false => menu.top + first + (step - menu.top),
            };
            (
                index,
                Panel::new(x, y + MENU_PAD + step as f32 * MENU_ROW_H, w, MENU_ROW_H),
            )
        })
        .collect();
    (Panel::new(x, y, w, h), rows)
}

/// One row per file, as heights the scroll window can be taken from.
///
/// The explorer clips a name that does not fit rather than wrapping it, so a row
/// is always exactly one row. That is what keeps a click from resolving to a
/// different file than the one under the pointer, the same rule the debug pane
/// follows. Written as heights, and read through
/// [`text_geometry`], so the window and the clamp come from the one place that
/// owns them rather than from arithmetic at two call sites.
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

/// Where the picker's pieces are. A struct rather than a tuple, the way the
/// settings panel's are: five panels in a row is a call site nobody can read.
struct PickerPlaces {
    box_: Panel,
    list: Panel,
    rows: Vec<(usize, Panel)>,
    marks: Vec<(usize, Panel)>,
    open: Panel,
    sessions: Panel,
}

/// How far in from the left of a picker row its mark sits, and how wide that
/// mark is, for a row at this depth in the tree.
///
/// One answer, so the region a press is tested against and the glyph that is
/// drawn cannot end up in two places. The indent stops growing once the label
/// is down to [`PICKER_LABEL_COLUMNS`]: a deep tree in a narrow box would
/// otherwise push its names off the right of the list.
fn picker_indent(depth: usize, column: f32, cols: usize) -> (f32, f32) {
    let column = column.max(1.0);
    let room = cols
        .saturating_sub(PICKER_LABEL_COLUMNS + PICKER_MARK_COLUMNS + ROW_ICON_COLUMNS + 1)
        / PICKER_INDENT_COLUMNS.max(1);
    let steps = depth.min(room);
    (
        PICKER_ROW_PAD + (steps * PICKER_INDENT_COLUMNS) as f32 * column,
        PICKER_MARK_COLUMNS as f32 * column,
    )
}

/// The folder picker's box, its list, the rows on screen, the mark that opens
/// and shuts each of them, and its button.
///
/// Centred in `area` and no wider than [`PICKER_COLUMNS`], because the thing
/// being read is a column of folder names: stretched across a 2200 pixel window
/// the eye has to travel the whole width to get from a name to the button under
/// it.
///
/// One shape, and it is not the folder's shape. The height is chosen from the
/// room the window has, between [`PICKER_MIN_ROWS`] and [`PICKER_MAX_ROWS`], and
/// then held: `picker` says what goes in the box and never how big it is. That
/// is why nothing here reads `picker.rows().len()`. Walking into a folder with a
/// different number of entries used to resize and recentre the whole dialog
/// under the pointer, so every row moved out from under the click that was about
/// to happen.
fn place_picker(area: Panel, shape: &Shape, picker: &Picker) -> PickerPlaces {
    if area.w < 1.0 || area.h < 1.0 {
        return PickerPlaces {
            box_: nowhere(),
            list: nowhere(),
            rows: Vec::new(),
            marks: Vec::new(),
            open: nowhere(),
            sessions: nowhere(),
        };
    }
    let column = shape.pane_column.max(1.0);
    let line = Text::line_for(shape.pane_size);
    let head = PICKER_HEAD_ROWS * line;
    let foot = picker_foot(line);
    // Everything the box spends on something other than its list.
    let chrome = PAD * 2.0 + head + GAP + foot;
    let fits = ((area.h - chrome) / line).floor().max(0.0) as usize;
    let want = fits.clamp(PICKER_MIN_ROWS, PICKER_MAX_ROWS);
    let w = (PICKER_COLUMNS as f32 * column + PAD * 2.0).min(area.w);
    let h = (chrome + want as f32 * line).min(area.h);
    let box_ = Panel::new(
        area.x + ((area.w - w) * 0.5).floor(),
        area.y + ((area.h - h) * 0.5).floor(),
        w,
        h,
    );
    let content = box_.inset(PAD);
    let list = Panel::new(
        content.x,
        content.y + head,
        content.w,
        (content.h - head - foot - GAP).max(0.0),
    );
    let rows_fit = Text::rows_for(shape.pane_size, list.h);
    let heights = picker.heights();
    let back = text_geometry::scrollback_for(&heights, rows_fit, picker.first());
    let window = text_geometry::window(&heights, rows_fit, back);
    let rows: Vec<(usize, Panel)> = (0..window.count)
        .map(|step| {
            // The full width of the list, so the whole row answers the click the
            // way a row of a file manager does, not just the characters of the
            // name.
            let index = window.first + step;
            (
                index,
                Panel::new(list.x, list.y + step as f32 * line, list.w, line),
            )
        })
        .collect();
    // A mark only where there is a folder to open: the folder being listed, the
    // way out of it, a folder remembered from an earlier session and the message
    // under a folder that could not be read are not branches of the tree.
    let cols = cols_of(list, column);
    let marks = rows
        .iter()
        .filter_map(|(index, row)| {
            let entry = picker.row(*index)?;
            entry.open()?;
            let (indent, wide) = picker_indent(entry.depth(), column, cols);
            Some((*index, Panel::new(row.x + indent, row.y, wide, row.h)))
        })
        .collect();
    // Exactly as wide as what it says, and what it says is one fixed word: the
    // confirm glyph, the space after it, [`PICKER_OPEN_LABEL`], a column of
    // indent on the left and two on the right so the cut corner never reaches
    // the text.
    let open_w = ((ROW_ICON_COLUMNS + 1 + PICKER_OPEN_LABEL.chars().count() + 3) as f32 * column)
        .min(content.w);
    let open_h = picker_open_h(line).min(content.h);
    let open = Panel::new(
        content.x,
        content.y + content.h - open_h,
        open_w,
        open_h,
    );
    // The list swap, beside it. Sized for the longer of the two words it can
    // say, so pressing it does not change the width of what was pressed, and
    // clipped to what is left of the row rather than allowed out of the box: in
    // a window too narrow for both there is no button rather than a button
    // sticking out of the picker.
    let toggle_w = ((ROW_ICON_COLUMNS
        + 1
        + PICKER_SESSIONS_LABEL
            .chars()
            .count()
            .max(PICKER_FOLDERS_LABEL.chars().count())
        + 3) as f32
        * column)
        .min((content.w - open.w - GAP).max(0.0));
    let sessions = Panel::new(open.x + open.w + GAP, open.y, toggle_w, open_h);
    PickerPlaces {
        box_,
        list,
        rows,
        marks,
        open,
        sessions,
    }
}

/// The settings panel's box, its list, the rows on screen, the value at the end
/// of each of those rows, and the mark that closes it.
///
/// The whole area rather than a centred box: this is a takeover, and a list of
/// sixty rows in a box the size of the picker's would be six rows of content and
/// a lot of margin. The value column is a fixed number of columns in from the
/// right so every value lines up in one column, which is what makes a screen of
/// settings scannable rather than a wall of words.
/// Where the settings panel's pieces are. A struct rather than a tuple: five
/// panels in a row is a call site nobody can read, and two of them are lists.
struct SettingsPlaces {
    box_: Panel,
    list: Panel,
    rows: Vec<(usize, Panel)>,
    values: Vec<(usize, Panel)>,
    close: Panel,
}

/// What the value column takes, capped so a narrow window keeps a label: past
/// half the row the values are what gets clipped, since a row whose label is gone
/// says nothing at all.
///
/// Asked by the placement and by the drawing, so a value is drawn exactly where
/// the click that changes it is tested for.
fn settings_value_w(list_w: f32, column: f32) -> f32 {
    (SETTING_VALUE_COLUMNS as f32 * column).min((list_w * 0.5).floor())
}

fn place_settings(area: Panel, shape: &Shape, panel: &Settings) -> SettingsPlaces {
    if area.w < 1.0 || area.h < 1.0 {
        return SettingsPlaces {
            box_: nowhere(),
            list: nowhere(),
            rows: Vec::new(),
            values: Vec::new(),
            close: nowhere(),
        };
    }
    let column = shape.pane_column.max(1.0);
    let line = Text::line_for(shape.pane_size);
    let content = area.inset(PAD);
    // The heading, and the footer that says what the keys do.
    let head = line;
    let foot = line;
    let list = Panel::new(
        content.x,
        content.y + head + GAP,
        content.w,
        (content.h - head - foot - GAP * 2.0).max(0.0),
    );
    let rows_fit = Text::rows_for(shape.pane_size, list.h);
    let heights = panel.heights();
    let back = text_geometry::scrollback_for(&heights, rows_fit, panel.first());
    let window = text_geometry::window(&heights, rows_fit, back);
    let value_w = settings_value_w(list.w, column);
    let mut rows = Vec::new();
    let mut values = Vec::new();
    for step in 0..window.count {
        let index = window.first + step;
        let row = Panel::new(list.x, list.y + step as f32 * line, list.w, line);
        rows.push((index, row));
        // Only a row that carries a control gets one. A heading or a reading
        // with a click region over its value would answer a press with nothing.
        if matches!(panel.row(index), Some(SettingRow::Setting { kind, .. }) if kind.changes()) {
            values.push((
                index,
                Panel::new(row.x + row.w - value_w, row.y, value_w, row.h),
            ));
        }
    }
    // Top right, one cut's reach in from the corner the cut takes away, so the
    // mark is not drawn in the triangle that is not there.
    let close = Panel::new(content.x + content.w - CUT - line, content.y, line, line);
    SettingsPlaces {
        box_: area,
        list,
        rows,
        values,
        close,
    }
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
    // nothing to type at.
    if layout.picking {
        folder_picker(&mut scene, frame);
        return scene;
    }

    // The settings panel covers the panes and the prompt, so nothing under it is
    // drawn: a pane painted behind a panel that fills the surface is a pane
    // nobody sees, redrawn on every keystroke.
    if layout.in_settings {
        settings_panel(&mut scene, frame);
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

/// What a drop would do, drawn over the space it would do it to: a translucent
/// green box over the whole space, and a caret in the gap between the two tabs
/// the tab would land between.
///
/// On the floating layer, so it covers the pane rather than being painted under
/// the pane's own text the way a base-layer rectangle is (see [`overlay`]). A
/// wash under the glyphs is exactly the feedback item 17 said it could not see.
///
/// The box is the strip and the body together, because the space is what the drop
/// lands in and its tabs are part of it. Folded, the body is zero tall and the
/// box is the strip, which is all there is of that space to point at.
///
/// The caret is only drawn for a drop that names a place, which is a drop on a
/// tab strip. In the body of a pane there is no gap being aimed at: the tab goes
/// to the end of the space, and a caret standing between two tabs would promise
/// a position the drop does not name.
fn drop_target(scene: &mut Scene, frame: &Frame) {
    let Some(drag) = frame.drag else {
        return;
    };
    let Landing::In(space, at) = drag.landing else {
        return;
    };
    let (skin, layout) = (frame.skin, frame.layout);
    let placed = layout.placed(space);
    if placed.strip.w < 1.0 {
        return;
    }
    let box_ = Panel::new(
        placed.strip.x,
        placed.strip.y,
        placed.strip.w,
        placed.strip.h + placed.body.h,
    );
    // The same cut corner every panel in the window has, so the box lies on the
    // pane instead of squaring off its top right corner.
    scene.over_rect(box_.fill(skin.drop_target).chamfer(cut_of(box_), Rect::TOP_RIGHT));
    let Some(at) = at else {
        return;
    };
    let x = layout
        .insertion_gap(space, at)
        .min(placed.strip.x + placed.strip.w - CARET_W);
    scene.over_rect(Panel::new(x, placed.strip.y, CARET_W, placed.strip.h).fill(skin.drop_mark));
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
    let Some(menu) = frame.menu else {
        return;
    };
    let (skin, layout) = (frame.skin, frame.layout);
    if layout.menu.w < 1.0 {
        return;
    }
    scene.over_rect(panel_fill(layout.menu, skin.menu));
    scene.over_rect(panel_edge(layout.menu, skin.edge_focus));
    let line = Text::line_for(SMALL);
    for (index, panel) in &layout.menu_rows {
        let Some(row) = menu.rows.get(*index) else {
            continue;
        };
        let (index, panel) = (*index, *panel);
        // Only a row that can act lights up. Highlighting a greyed one promises
        // something will happen when the button comes down and it will not.
        if row.enabled && frame.hot == Some(Hit::MenuRow(index)) {
            scene.over_rect(panel.fill(skin.hot));
        }
        // A row that cannot act says so by weight, the way a tab that is not
        // showing does, rather than by being missing.
        let tint = if row.enabled { skin.bright } else { skin.dim };
        let mut runs = Vec::new();
        match row.item.icon() {
            Some(icon) => runs.push(Run::icon(icon.to_string(), tint)),
            // The gutter is spent either way, so the labels line up.
            None => runs.push(Run::tinted(" ", tint)),
        }
        // The space after the mark, plus whatever the row steps in by: a widget
        // row sits under the row that listed it rather than beside it.
        let lead = " ".repeat(1 + row.item.indent());
        runs.push(Run::tinted(
            format!("{lead}{}", row.item.label()),
            tint,
        ));
        let text = Panel::new(
            panel.x + MENU_PAD,
            panel.y,
            (panel.w - MENU_PAD * 2.0).max(1.0),
            panel.h,
        );
        scene.over_text(Text::rich(runs, text.row(0.0, line), SMALL, tint));
    }
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
    // turn to animate, one frozen dimmer frame otherwise. The base layer is
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
    for disc in crate::orb::discs(block, frame.clock, state.phase.busy(), skin) {
        scene.rect(disc);
    }

    // These were three hand-drawn rectangles, because the Unicode glyphs the
    // first version asked for were not on this machine and a missing glyph
    // draws as nothing. The symbol font ships in the binary now, so they are
    // the same marks every other window on the desktop uses.
    let line = Text::line_for(SMALL);
    for (panel, hit, tint, glyph, quiet) in [
        (layout.minimize, Hit::Minimize, skin.hot, crate::icons::MINIMIZE, true),
        (layout.maximize, Hit::Maximize, skin.hot, crate::icons::MAXIMIZE, true),
        (layout.close, Hit::Close, skin.close_hot, crate::icons::CLOSE, false),
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

    // The name, then the marker, and nothing else at full strength. It read
    // "NO0B \u{25b8} CLIppy" while the window had two names; it has one.
    let room = (layout.width - BUTTON_W * 3.0 - ORB_W - 12.0).max(1.0);
    let mut runs = vec![
        Run::tinted("NO0B \u{25b8}", skin.bright),
        // Which build this is: the version the crate carries, then the commit
        // build.rs stamped after it, because a version cannot tell two test
        // builds of the same release apart. At the text tint, not the dim one:
        // dim is the faintest thing the palette has and two builds side by side
        // could not be told apart, which is the one job this reading has.
        Run::tinted(format!(" {VERSION}{}", build_commit()), skin.title),
    ];
    // Open, the strip says which build this is and nothing more. The phase, the
    // model, the workspace and the token budget were readings squeezed into a
    // title with no room to label them; they belong in the monitors, which have
    // both. Trouble stays because it is the one thing that makes the rest of
    // the window meaningless.
    if let Some(trouble) = frame.trouble {
        runs.push(Run::tinted(format!("   {trouble}"), skin.bad));
    } else if layout.shaded {
        // Shaded, this strip is the whole window, so it carries the one thing
        // worth knowing while there is nowhere else to read it.
        runs.push(Run::tinted(format!("   {}", state.headline()), skin.good));
    }
    scene.text(Text::rich(
        runs,
        Panel::new(ORB_W, 0.0, room, TITLE_H).row(0.0, Text::line_for(SMALL)),
        SMALL,
        skin.title,
    ));
}

/// The body of a panel: the fill, cut corner and all.
///
/// The cut lives on the fill as well as on the outline because they are the
/// same shape twice. A square fill under a cut outline shows a triangle of the
/// wrong colour poking out of the corner.
fn panel_fill(panel: Panel, rgba: [f32; 4]) -> Rect {
    panel.fill(rgba).chamfer(CUT, Rect::TOP_RIGHT)
}

/// How far the cut actually reaches on a box this size.
///
/// The shader caps the reach at half the shorter side, so a short box loses a
/// smaller corner than [`CUT`]. Anything that has to stop where the cut starts
/// has to cap it the same way, or it stops short of a corner nothing took.
fn cut_of(panel: Panel) -> f32 {
    CUT.min(panel.w * 0.5).min(panel.h * 0.5).max(0.0)
}

/// Its hairline border, as one rectangle. Four of them could not follow the
/// cut.
///
/// For a box that wants all four sides: the prompt, the picker, the menu. A
/// pane's body uses [`pane_edges`] instead, which leaves the top one out.
fn panel_edge(panel: Panel, rgba: [f32; 4]) -> Rect {
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
/// Three thin rectangles rather than the one stroked rect [`panel_edge`] draws,
/// because a stroke follows the whole shape and cannot leave a side out. The
/// three that are left are straight lines: the cut is on the top right, which is
/// the corner the top edge had, and what the remaining right edge has to do about
/// it is start where the cut stops rather than in a corner that is not there.
fn pane_edges(scene: &mut Scene, panel: Panel, rgba: [f32; 4]) {
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
}

/// One tab of a strip, before its label goes on.
///
/// A tab is not a button. Both states carry the pane's own surface and the same
/// cut corner the pane has, so the tab reads as the top of the pane; what says
/// which one is showing is weight. The showing tab is that surface at full
/// strength with an accent line in the colour of what it holds, the rest are
/// the same colour at a lower alpha. A filled block over a filled strip is what
/// made these look like a row of buttons.
///
/// `Skin::tab` is exactly `Skin::panel`, and the showing tab sits flush on the
/// pane, so the two composite to one surface with nothing between them. That is
/// the other half of losing the line under the strip ([`pane_edges`]): a step in
/// colour where the line was is the same complaint as the line.
fn tab_block(scene: &mut Scene, skin: &Skin, tab: Panel, active: bool, accent: [f32; 4]) {
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
    scene.rect(Panel::new(tab.x, tab.y, (tab.w - cut).max(1.0), ACCENT_H.min(tab.h)).fill(accent));
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
        tab_block(scene, skin, *panel, active, skin.view(*view));
        // Not showing reads as not showing. This was the title tint, as strong
        // as the showing tab's, which left the fill to carry the whole
        // difference and is why the fill had to be so heavy.
        let color = if active && !lifted {
            skin.bright
        } else {
            skin.dim
        };
        scene.text(Text::rich(
            vec![Run::tinted(view.label(), color)],
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
        Some(View::Output) => output(scene, frame, panel),
        Some(View::Activity) => activity(scene, frame, panel),
        Some(View::Plan) => plan(scene, frame, panel),
        Some(View::Agents) => agents(scene, frame, panel),
        Some(View::Hardware) => {
            gauges(scene, frame, panel, View::Hardware, frame.monitor.hardware())
        }
        // The monitor's lists are named for the panes they feed, so a reading in
        // the wrong pane is a rename away from being obvious rather than two
        // files away.
        Some(View::Context) => context(scene, frame, panel),
        Some(View::Session) => gauges(scene, frame, panel, View::Session, frame.monitor.session()),
        Some(View::Debug) => debug(scene, frame, panel),
        Some(View::Files) => files(scene, frame, panel),
    }
}

/// The band behind selected text, drawn before the glyphs go over it.
///
/// One rectangle per visible line of the selection rather than one for the
/// whole block, because the first and last lines start and stop mid-line and a
/// single rectangle would cover text that is not selected.
fn selection_band(scene: &mut Scene, frame: &Frame, panel: Panel, showing: Option<View>) {
    let (Some(selection), Some(view)) = (frame.selection, showing) else {
        return;
    };
    if selection.view != view || selection.is_empty() {
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
    let rows = frame.layout.rows(panel, size);
    let cols = cols_of(panel, column);
    let line_h = Text::line_for(size);
    let window = pane.window(rows, cols);
    let first = pane.showing_from(rows, cols);
    for step in 0..window.count {
        let number = first + step;
        let Some(line) = pane.line(number) else {
            continue;
        };
        let chars = line.text.chars().count();
        let Some((from, to)) = selection.columns_on(number, chars) else {
            continue;
        };
        let Some((top, height)) = pane.band_of(rows, cols, number) else {
            continue;
        };
        // A wrapped line needs one rectangle per visual row, each covering only
        // the part of the selection that lands on that row. The first line in
        // the window may start partway down, which is what `skip` records.
        let from_row = if step == 0 { window.skip } else { 0 };
        for i in 0..height {
            let wrapped = from_row + i;
            let row_start = wrapped * cols;
            let row_end = (row_start + cols).min(chars.max(row_start));
            let a = from.max(row_start);
            let b = to.min(row_end);
            if a >= b {
                continue;
            }
            let x = content.x + (a - row_start) as f32 * column;
            let width = ((b - a) as f32 * column).min(content.x + content.w - x);
            let y = content.y + (top + i) as f32 * line_h;
            if width <= 0.0 || y + line_h > content.y + content.h {
                continue;
            }
            scene.rect(Panel::new(x, y, width, line_h).fill(frame.skin.select));
        }
    }
}

fn text_box(scene: &mut Scene, frame: &Frame, panel: Panel, size: f32, runs: Vec<Run>) {
    scene.text(Text::rich(runs, panel.inset(PAD), size, frame.skin.body));
}

/// The OUTPUT pane: what the model said, as Markdown.
fn output(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let rows = frame.layout.rows(panel, frame.body_size);
    let cols = cols_of(panel, frame.column);
    let mut runs = Vec::new();
    // A window that starts inside a fenced block has to know it is looking at
    // code, so the state is carried in from the lines above it.
    let mut fence = state.output.fence_before(rows, cols);
    for line in state.output.visible(rows, cols) {
        match line.tone {
            // Only the model's prose is Markdown. What the human typed and
            // what the harness noted are shown as written.
            Tone::Body => crate::markdown::line(&line.text, &mut fence, skin, &mut runs),
            tone => runs.push(Run::tinted(&line.text, skin.tone(tone))),
        }
        runs.push(Run::plain("\n"));
    }
    // The window may start partway down a wrapped line rather than dropping
    // it, so the shaped buffer is scrolled by the rows that sit above.
    scene.text(
        Text::rich(runs, panel.inset(PAD), frame.body_size, frame.skin.body)
            .scrolled(state.output.window(rows, cols).skip as f32),
    );
    scrollbar(scene, skin, panel, state.output.thumb(rows, cols));
}

fn activity(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let rows = frame.layout.rows(panel, frame.pane_size);
    let cols = cols_of(panel, frame.pane_column);
    let mut runs = Vec::new();
    for line in state.activity.visible(rows, cols) {
        runs.push(Run::tinted(&line.text, skin.tone(line.tone)));
        runs.push(Run::plain("\n"));
    }
    scene.text(
        Text::rich(runs, panel.inset(PAD), frame.pane_size, frame.skin.body)
            .scrolled(state.activity.window(rows, cols).skip as f32),
    );
    scrollbar(scene, skin, panel, state.activity.thumb(rows, cols));
}

fn plan(scene: &mut Scene, frame: &Frame, panel: Panel) {
    list_pane(scene, frame, panel, View::Plan, plan_rows(frame.state, frame.skin));
}

/// One row per todo, wrapped in whatever width the pane has.
fn plan_rows(state: &State, skin: &Skin) -> Vec<ListRow> {
    if state.plan.is_empty() {
        return vec![ListRow::new(vec![Run::tinted("no plan yet", skin.dim)])];
    }
    state
        .plan
        .iter()
        .map(|todo| {
            let (mark, color) = match todo.state {
                TodoState::Done => ("[x] ", skin.good),
                TodoState::Active => ("[>] ", skin.bright),
                TodoState::Pending => ("[ ] ", skin.dim),
            };
            ListRow::new(vec![
                Run::tinted(mark, color),
                Run::tinted(&todo.text, color),
            ])
        })
        .collect()
}

/// The fleet: one child per row, and under each the last thing it said.
fn agents(scene: &mut Scene, frame: &Frame, panel: Panel) {
    list_pane(
        scene,
        frame,
        panel,
        View::Agents,
        agent_rows(frame.state, frame.skin),
    );
}

/// Two rows per child, and the second is where the news is.
///
/// A row alone is a name and a word, which for eight children at once tells you
/// nothing about any of them: while a child runs the second row is that child's
/// own output, and once it ends it is the reason it ended. Two rows each is also
/// why this pane needs a scroll more than any other, a fleet of eight being
/// sixteen rows.
fn agent_rows(state: &State, skin: &Skin) -> Vec<ListRow> {
    if state.agents.is_empty() {
        return vec![ListRow::new(vec![Run::tinted(
            "no sub-agents this session",
            skin.dim,
        )])];
    }
    let mut rows = Vec::new();
    for agent in &state.agents {
        let mut runs = vec![
            Run::tinted(format!("{:<9}", agent.label), skin.dim),
            Run::tinted(format!("{:<10}", agent.state), skin.tone(agent.tone)),
        ];
        // The tool set says whether this child can change anything, which is
        // the one thing about a detached child worth knowing at a glance.
        if !agent.tools.is_empty() {
            runs.push(Run::tinted(format!("{:<10}", agent.tools), skin.dim));
        }
        runs.push(Run::tinted(clip(&agent.brief, 300), skin.body));
        rows.push(ListRow::new(runs));
        if !agent.last.is_empty() {
            rows.push(ListRow::new(vec![Run::tinted(
                format!("           {}", clip(&agent.last, 300)),
                skin.dim,
            )]));
        }
    }
    rows
}

/// One logical line of a list pane: the runs that draw it, and how long it is in
/// characters.
///
/// The length is counted off the runs rather than passed in beside them. It is
/// what the scroll window is measured from, and a length that disagreed with what
/// was drawn is a pane that scrolls by a different number of rows than it has.
struct ListRow {
    runs: Vec<Run>,
    chars: usize,
}

impl ListRow {
    fn new(runs: Vec<Run>) -> ListRow {
        let chars = runs.iter().map(|run| run.text.chars().count()).sum();
        ListRow { runs, chars }
    }
}

/// A pane that is a list of lines, scrolled inside its own box.
///
/// PLAN, AGENTS and DEBUG. All three drew every row they had, with no window and
/// no bar: the first two into one text box that ran off the bottom of the pane,
/// and the third by taking as many rows as fitted and dropping the rest. What was
/// past the edge could not be reached at all, which is what item 14 reported.
///
/// The window, the clamp and the thumb come from `text_geometry` through
/// [`crate::scroll::Scrolls`], the same numbers the transcript is drawn from, so a row of
/// a list and a row of a transcript mean the same thing. A line partly scrolled
/// off the top is drawn in full and offset by `skip` rather than dropped, which is
/// what lets a wrapped todo scroll a row at a time.
fn list_pane(scene: &mut Scene, frame: &Frame, panel: Panel, view: View, rows: Vec<ListRow>) {
    let size = frame.pane_size;
    let fit = frame.layout.rows(panel, size);
    let cols = cols_of(panel, frame.pane_column);
    let heights = text_geometry::heights(rows.iter().map(|row| row.chars), cols);
    let scrolls = &frame.state.scrolls;
    let window = scrolls.window(view, &heights, fit);
    let mut runs = Vec::new();
    for row in rows.into_iter().skip(window.first).take(window.count) {
        runs.extend(row.runs);
        runs.push(Run::plain("\n"));
    }
    if !runs.is_empty() {
        scene.text(
            Text::rich(runs, panel.inset(PAD), size, frame.skin.body)
                .scrolled(window.skip as f32),
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
/// wheel, the page keys, the click in the debug pane and the per-frame clamp all
/// ask here. Anything that worked it out for itself would eventually scroll a pane
/// by a different number of rows than the pane drew, which is the class of bug
/// `text_geometry` exists to end.
pub fn scroll_extent(frame: &Frame, view: View, panel: Panel) -> Option<(Vec<usize>, usize)> {
    let fit = frame.layout.rows(panel, frame.pane_size);
    let cols = cols_of(panel, frame.pane_column);
    let lines = |rows: Vec<ListRow>| {
        Some((
            text_geometry::heights(rows.iter().map(|row| row.chars), cols),
            fit,
        ))
    };
    match view {
        View::Plan => lines(plan_rows(frame.state, frame.skin)),
        View::Agents => lines(agent_rows(frame.state, frame.skin)),
        View::Debug => lines(debug_list(frame.state, cols, frame.skin)),
        View::Hardware => gauge_extent(frame, panel, frame.monitor.hardware()),
        View::Session => gauge_extent(frame, panel, frame.monitor.session()),
        // The readings sit under the header, in a box of their own, and it is that
        // box they scroll in.
        View::Context => gauge_extent(
            frame,
            gauge_area(panel, frame.pane_size)?,
            frame.monitor.context(),
        ),
        View::Output | View::Activity | View::Files => None,
    }
}

/// A monitor pane's content: one row per reading, in rows of the pane's own
/// pitch rather than of one line. See [`gauges`].
fn gauge_extent(frame: &Frame, panel: Panel, gauges: Vec<Gauge>) -> Option<(Vec<usize>, usize)> {
    if gauges.is_empty() {
        return None;
    }
    let grid = gauge_grid(
        &gauges,
        panel.inset(PAD),
        frame.pane_size,
        frame.pane_column,
    );
    Some((flat_heights(gauges.len()), grid.rows))
}

/// A label column, a block of dots, and the reading, laid out as three boxes
/// rather than as one padded string.
///
/// One string with the bar's room spelled as spaces was the first attempt, and
/// the readings landed on top of the bars: the spaces are the pane's column
/// width and the bar was drawn in the transcript's, which is a different
/// number. Three boxes at computed positions cannot drift apart.
///
/// The block is [`DOT_COLUMNS`] by [`DOT_ROWS`] dots in the metric's own colour,
/// filling row by row from the bottom, so a row is 25% and a dot is 1.25%. Wide
/// and short on purpose: see the constants. An unbounded reading draws no block
/// at all, where it used to draw an empty track, so most of a pane was empty
/// rectangles and the two rows that were filled read as noise. An unbounded row
/// keeps the line pitch, because a tall empty row would push the rows that do
/// have blocks off the bottom of the pane.
///
/// Twenty columns is a lot of width to ask a pane for, so the number is served
/// first and the block takes what is left. What is left can be nothing: a pane
/// dragged narrow enough that a dot would be under [`SMALL_DOT`] across draws no
/// blocks at all and every row becomes a label and a number, which is a row this
/// function already draws. That is the whole of the narrow case, and it is why
/// the readings themselves are never clipped or shrunk: a block is only ever
/// drawn in room the reading did not need.
///
/// The pane scrolls, so a reading past the bottom is reachable rather than
/// dropped. It used to stop drawing at the last row that fitted, which for the
/// hardware pane on a machine with two GPUs meant readings nothing could reach.
/// Every row is the same height ([`Grid::pitch`]) for that reason: the scroll
/// window is measured in rows of one height, and a pane whose rows differed could
/// not say how many of itself were on screen. The cost is that an unbounded row in
/// a pane that has blocks is as tall as a block row instead of one line, which is
/// a pane of evenly pitched rows rather than a pane of two pitches.
fn gauges(scene: &mut Scene, frame: &Frame, panel: Panel, view: View, gauges: Vec<Gauge>) {
    let skin = frame.skin;
    let content = panel.inset(PAD);

    if gauges.is_empty() {
        text_box(
            scene,
            frame,
            panel,
            frame.pane_size,
            vec![Run::tinted("sampling\u{2026}", skin.dim)],
        );
        return;
    }

    let grid = gauge_grid(&gauges, content, frame.pane_size, frame.pane_column);
    let heights = flat_heights(gauges.len());
    let scrolls = &frame.state.scrolls;
    let window = scrolls.window(view, &heights, grid.rows);
    let (label_w, gap, dot) = (grid.label_w, grid.gap, grid.dot);
    let (block_h, pitch) = (grid.block_h, grid.pitch);
    let cell = dot + gap;
    let line = Text::line_for(frame.pane_size);

    let mut y = content.y;
    for gauge in gauges.iter().skip(window.first).take(window.count) {
        // No block in a pane with no room for one, so the row is the label and
        // the number, exactly as an unbounded reading is drawn.
        let fraction = gauge.fraction().filter(|_| grid.blocked);
        let row_h = pitch;
        let (lit, unlit, ink) = skin.gauge_slot(gauge.hue);
        scene.text(Text::rich(
            vec![Run::tinted(gauge.label, skin.dim)],
            Panel::new(
                content.x,
                y + ((row_h - line) * 0.5).floor(),
                label_w.max(1.0),
                line,
            ),
            frame.pane_size,
            skin.dim,
        ));
        // The metric's own colour, so the number and its block are one reading.
        // Nearly full is the one thing worth overriding it for: a block cannot
        // warn, because a metric whose hue is already red has nowhere to go.
        let tint = if fraction.is_some_and(|f| f > 0.85) {
            skin.bad
        } else {
            ink
        };
        let (size, at_x) = match fraction {
            Some(_) => (grid.reading, grid.read_x),
            None => (frame.pane_size, content.x + label_w),
        };
        let read_line = Text::line_for(size);
        scene.text(Text::rich(
            vec![Run::tinted(gauge.reading(), tint)],
            Panel::new(
                at_x,
                y + ((row_h - read_line) * 0.5).floor(),
                (content.x + content.w - at_x).max(1.0),
                read_line,
            ),
            size,
            tint,
        ));

        if let Some(fraction) = fraction {
            let filled = (fraction * (DOT_COLUMNS * DOT_ROWS) as f32).round() as usize;
            let top = y + ((row_h - block_h) * 0.5).floor();
            for index in 0..DOT_COLUMNS * DOT_ROWS {
                let (row, col) = (index / DOT_COLUMNS, index % DOT_COLUMNS);
                // Rows fill from the bottom, so the block reads as a level
                // rising rather than as a staircase. Every dot is drawn, lit or
                // not, which is what makes the block read as a block at 2%.
                scene.rect(
                    Panel::new(
                        content.x + label_w + col as f32 * cell,
                        top + block_h - (row + 1) as f32 * dot - row as f32 * gap,
                        dot,
                        dot,
                    )
                    .fill(if index < filled { lit } else { unlit })
                    .radius(0.5 * dot),
                );
            }
        }
        y += row_h;
    }
    scrollbar(scene, skin, panel, scrolls.thumb(view, &heights, grid.rows));
}

/// How a monitor pane's rows are sized, worked out once for the pane rather than
/// per row.
///
/// The wheel and the per-frame clamp need [`Grid::rows`] as much as the drawing
/// does, and a second copy of this arithmetic at the call site is how a pane comes
/// to scroll by a different number of rows than it drew.
struct Grid {
    /// The label column, as wide as the longest label in this pane.
    label_w: f32,
    dot: f32,
    gap: f32,
    /// Whether a block is drawn at all in this pane.
    blocked: bool,
    /// How tall the block is, or zero when it is not drawn. Its width is spent
    /// rather than carried: what a caller needs is where the reading starts,
    /// which is [`Grid::read_x`].
    block_h: f32,
    /// What every row of this pane is tall, block row or not.
    pitch: f32,
    /// The size a reading is drawn at, and where a bounded one starts.
    reading: f32,
    read_x: f32,
    /// How many rows of this pane are on screen.
    rows: usize,
}

fn gauge_grid(gauges: &[Gauge], content: Panel, size: f32, column: f32) -> Grid {
    let line = Text::line_for(size);
    // As wide as the longest label in this pane, so TOTAL TOOL CALLS is not
    // clipped and a pane of short labels does not pay for one that has none.
    let label_cols = gauges
        .iter()
        .map(|gauge| gauge.label.chars().count())
        .max()
        .unwrap_or(LABEL_COLUMNS)
        .max(LABEL_COLUMNS)
        + 1;
    let label_w = label_cols as f32 * column;
    let gap = (line * 0.12).round().max(1.0);
    // The number is served first: it gets the room its longest reading needs at
    // the pane's own size, and the block takes what is left, never more than half
    // of it and never less than a legible dot. A block that pushed the number off
    // the pane would be hiding the reading it exists to describe.
    let widest = gauges
        .iter()
        .filter(|gauge| gauge.fraction().is_some())
        .map(|gauge| gauge.reading().chars().count())
        .max()
        .unwrap_or(1)
        .max(1);
    let needed = widest as f32 * column;
    let free = (content.w - label_w - column).max(1.0);
    let room = (free - needed).max(0.0).min(free * 0.5);
    // As chunky as this pane can afford. A dot big enough to read as a block is
    // the point of the shape, but a pane of thirteen readings cannot spend the
    // same height per block as one of five. Past the floor the pane scrolls
    // instead of shrinking further, which is what item 14 asked for.
    //
    // Not clamped up to anything: a pane with no room for a legible dot is meant
    // to come out of here under [`SMALL_DOT`], which is what says no block.
    let mut dot = (line * 0.34)
        .round()
        .min((room / DOT_COLUMNS as f32 - gap).floor());
    let bounded = gauges.iter().any(|gauge| gauge.fraction().is_some());
    let tall = |dot: f32| {
        let block = dot * DOT_ROWS as f32 + gap * (DOT_ROWS - 1) as f32;
        gauges.len() as f32 * (block + 2.0 * gap).max(line)
    };
    while dot > SMALL_DOT && tall(dot) > content.h {
        dot -= 1.0;
    }
    // Whether this pane draws blocks at all. Either the dot is legible or the
    // pane is too narrow (or too short, since the loop above stops at the same
    // floor) to draw one, and then every reading is a number beside its label. A
    // pane with nothing bounded in it has no block to draw either way, and must
    // not pay a block's row height for the readings it does have.
    let blocked = bounded && dot >= SMALL_DOT;
    let (block_w, block_h) = match blocked {
        true => (
            (dot + gap) * DOT_COLUMNS as f32,
            dot * DOT_ROWS as f32 + gap * (DOT_ROWS - 1) as f32,
        ),
        false => (0.0, 0.0),
    };
    // The size of the number beside a block, which at [`BIG_READING`] of one is
    // the pane's own size and nothing else in this arithmetic bites. It is kept
    // because it is what makes a larger reading safe: capped at the room left
    // beside the block, so `1,048,576 / 2,097,152` in a pane dragged narrow comes
    // out smaller rather than clipped halfway through, which reads as a different
    // number. Floored, not rounded, because rounding up is what puts the last
    // character over the edge.
    let beside = (content.w - label_w - block_w - column).max(1.0);
    let reading = (size * BIG_READING)
        .min(size * beside / needed)
        .floor()
        .max(size);
    let pitch = (block_h + 2.0 * gap).max(Text::line_for(reading));
    Grid {
        label_w,
        dot,
        gap,
        blocked,
        block_h,
        pitch,
        reading,
        read_x: content.x + label_w + block_w + column,
        rows: (content.h / pitch).floor().max(0.0) as usize,
    }
}

/// The CONTEXT pane: what the agent is, where it is working, and how full it is.
///
/// The first three rows are what came off the title strip when that was cut
/// back to the build stamp. They are readings with labels, which is what they
/// never were up there: the phase, the model and the workspace sat unlabelled
/// on one line with the token budget, and nothing said which was which. The
/// readings under them are [`Monitor::context`], named for this pane.
fn context(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let content = panel.inset(PAD);
    let line = Text::line_for(frame.pane_size);
    let label_w = (LABEL_COLUMNS + 1) as f32 * frame.pane_column;
    let rows: [(&str, String, [u8; 4]); CONTEXT_HEAD] = [
        (
            "PHASE",
            match state.resumed {
                true => format!("{} (resumed)", state.phase.word()),
                false => state.phase.word().to_string(),
            },
            if state.phase.busy() {
                skin.bright
            } else {
                skin.body
            },
        ),
        ("MODEL", state.model.clone(), skin.body),
        ("PATH", short_path(&state.workspace), skin.body),
    ];
    for (index, (label, value, tint)) in rows.iter().enumerate() {
        let y = content.y + index as f32 * line;
        scene.text(Text::rich(
            vec![Run::tinted(*label, skin.dim)],
            Panel::new(content.x, y, label_w.max(1.0), line),
            frame.pane_size,
            skin.dim,
        ));
        // Clipped, not wrapped: the rows are at fixed heights, so a long model
        // name that wrapped would have its second row cut off by its own box.
        let room = cols_of(panel, frame.pane_column).saturating_sub(LABEL_COLUMNS + 2);
        let text = match value.is_empty() {
            true => String::from("\u{2014}"),
            false => clip(value, room.max(1)),
        };
        scene.text(Text::rich(
            vec![Run::tinted(text, *tint)],
            Panel::new(
                content.x + label_w,
                y,
                (content.w - label_w).max(1.0),
                line,
            ),
            frame.pane_size,
            *tint,
        ));
    }
    // The readings start under the header, in the room that is left.
    let Some(below) = gauge_area(panel, frame.pane_size) else {
        return;
    };
    gauges(scene, frame, below, View::Context, frame.monitor.context());
}

/// The room the CONTEXT pane's readings get, under its header.
///
/// `None` when the pane is too short to hold even one reading under it. The
/// header itself does not scroll: it is three rows saying which agent this is,
/// and a monitor whose first rows scrolled away would be a monitor of an
/// unnamed session.
fn gauge_area(panel: Panel, size: f32) -> Option<Panel> {
    let line = Text::line_for(size);
    let used = CONTEXT_HEAD as f32 * line + line * 0.5;
    if panel.h - used < line {
        return None;
    }
    Some(Panel::new(panel.x, panel.y + used, panel.w, panel.h - used))
}

/// Calls that failed, and what was sent to the one that is open.
///
/// One row per line, clipped rather than wrapped. A click is turned into a row
/// by dividing by the line height, so a row that wrapped onto two would expand a
/// different failure than the one under the pointer. The rows themselves come
/// from [`State::debug_rows`], which is also what resolves the click, and the
/// window they are drawn from is what the click has added back to it: the row
/// under the pointer is the row on screen, not the row in the list.
fn debug(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let cols = cols_of(panel, frame.pane_column);
    list_pane(
        scene,
        frame,
        panel,
        View::Debug,
        debug_list(frame.state, cols, frame.skin),
    );
}

/// The debug pane's rows, each clipped to one row of a pane `cols` wide.
fn debug_list(state: &State, cols: usize, skin: &Skin) -> Vec<ListRow> {
    // One column short of the pane, because `clip` spends one on the ellipsis it
    // adds: a row exactly as wide as the pane would come back one wider and
    // wrap, which is the one thing this pane cannot allow.
    let room = cols.saturating_sub(1).max(1);
    state
        .debug_rows()
        .into_iter()
        .map(|row| {
            ListRow::new(vec![Run::tinted(
                clip(&row.text, room),
                skin.tone(row.tone),
            )])
        })
        .collect()
}

/// The file view: the explorer column, and the open file beside it.
fn files(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    if state.files.is_empty() {
        scene.text(Text::rich(
            vec![Run::tinted("no files touched yet", skin.dim)],
            panel.inset(PAD),
            frame.pane_size,
            skin.dim,
        ));
        return;
    }
    if layout.file_list.w >= 1.0 {
        explorer(scene, frame, layout.file_list);
    }

    let body = layout.file_diff;
    if body.w < 1.0 || body.h < Text::line_for(frame.pane_size) + 2.0 * PAD {
        return;
    }
    let rows = layout.rows(body, frame.pane_size);
    let Some(file) = state.files.get(state.open_file) else {
        return;
    };

    // A band behind every block header, drawn before the text. Without it a
    // `write lines 17-17` reads as a line of the file rather than as the mark
    // between two of them.
    let content = body.inset(PAD);
    let line = Text::line_for(frame.pane_size);
    // Every row carries a four column gutter, so the text wraps in what is
    // left rather than in the full width of the box.
    let (cols, _) = text_columns(View::Files, body, frame.pane_column);
    let first = file.pane.showing_from(rows, cols);
    let shown = file.pane.visible(rows, cols);
    for (step, entry) in shown.iter().enumerate() {
        if !matches!(entry.tone, Tone::Call(_)) {
            continue;
        }
        // A header that wraps gets a band as tall as it actually is, taken
        // from the same arithmetic the text is laid out with.
        let Some((top, height)) = file.pane.band_of(rows, cols, first + step) else {
            continue;
        };
        let y = content.y + top as f32 * line;
        let tall = height as f32 * line;
        if y + tall > content.y + content.h {
            break;
        }
        scene.rect(Panel::new(body.x + 1.0, y, (body.w - 2.0).max(1.0), tall).fill(skin.strip));
    }

    let syntax = crate::syntax::for_path(&file.path);
    let mut runs = Vec::new();
    for entry in &shown {
        let base = skin.tone(entry.tone);
        // The gutter, so a diff line says where in the file it landed.
        match entry.number {
            Some(number) => runs.push(Run::tinted(format!("{number:03} "), skin.comment)),
            None if !entry.text.is_empty() => runs.push(Run::plain("    ")),
            None => {}
        }
        // A removed line reads as removed first, so only what is there now is
        // tokenized.
        if matches!(entry.tone, Tone::Plus | Tone::Body) {
            let (marker, rest) = entry.text.split_at(entry.text.len().min(2));
            runs.push(Run::tinted(marker, base));
            for (text, token) in crate::syntax::scan(rest, syntax) {
                runs.push(Run::tinted(text, skin.token(token).unwrap_or(base)));
            }
        } else {
            runs.push(Run::tinted(&entry.text, base));
        }
        runs.push(Run::plain("\n"));
    }
    scene.text(Text::rich(runs, content, frame.pane_size, skin.body));
    scrollbar(scene, skin, body, file.pane.thumb(rows, cols));
}

/// The file list down the left of the pane, one row per file the agent has
/// touched, the way an editor's explorer reads.
///
/// Flat, because the set behind it is flat: these are the files the agent has
/// opened, not a filesystem. Nothing here groups by directory or expands, and a
/// row is a file.
fn explorer(scene: &mut Scene, frame: &Frame, list: Panel) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    // The one thing between the list and the file. The pane has a single surface
    // and a single outline, so without this line the two columns read as one.
    scene.rect(list.right_edge(skin.edge));
    let line = Text::line_for(frame.pane_size);
    let cols = cols_of(list, frame.pane_column);
    for (index, row) in &layout.file_rows {
        let Some(file) = state.files.get(*index) else {
            continue;
        };
        let open = *index == state.open_file;
        if open {
            // A band across the row and a mark down its left edge, not a block
            // in a colour of its own: the pane is already a surface, and a block
            // standing on it is what made the old tabs read as buttons.
            scene.rect(row.fill(skin.strip));
            scene.rect(Panel::new(row.x, row.y, MARK_W, row.h).fill(skin.view(View::Files)));
        }
        // A file compaction dropped is still worth reading; it is just no longer
        // what the agent is holding, and the row says which.
        let tint = match (open, file.closed) {
            (_, true) => skin.dim,
            (true, false) => skin.bright,
            (false, false) => skin.body,
        };
        let room = cols
            .saturating_sub(ROW_ICON_COLUMNS + if file.changed { ROW_MARK_COLUMNS } else { 0 })
            .max(1);
        let mut runs = vec![
            // The type mark, so a row is recognisable before it is read.
            Run::icon(crate::icons::for_path(&file.path).to_string(), tint),
            Run::tinted(format!(" {}", fit_name(&file.path, room)), tint),
        ];
        if file.changed {
            runs.push(Run::tinted(" \u{2022}", skin.plus));
        }
        scene.text(Text::rich(
            runs,
            Panel::new(row.x + PAD, row.y, (row.w - 2.0 * PAD).max(1.0), line),
            frame.pane_size,
            tint,
        ));
    }
    // The list is a scroll window like any other pane, so it says how much of
    // itself is on screen the same way.
    let rows = layout.rows(list, frame.pane_size);
    scrollbar(scene, skin, list, state.files_thumb(rows));
}

/// The folder picker: the whole window until a folder is chosen.
///
/// One box in the middle of the surface, drawn with the same rectangles and the
/// same text as everything else here. No native dialog: a file chooser from the
/// desktop's toolkit would pull in dozens of crates and a portal at runtime, for
/// a window whose whole point is that it is one GPU surface.
fn folder_picker(scene: &mut Scene, frame: &Frame) {
    let Some(picker) = frame.picker else {
        return;
    };
    let (skin, layout) = (frame.skin, frame.layout);
    let box_ = layout.picker;
    if box_.w < 1.0 || box_.h < 1.0 {
        return;
    }
    scene.rect(panel_fill(box_, skin.panel));
    scene.rect(panel_edge(box_, skin.edge_focus));

    let size = frame.pane_size;
    let line = Text::line_for(size);
    let content = box_.inset(PAD);
    let cols = cols_of(content, frame.pane_column);
    let say = |scene: &mut Scene, runs: Vec<Run>, at: Panel, tint: [u8; 4]| {
        scene.text(Text::rich(runs, at, size, tint));
    };

    let heading = match picker.on_sessions() {
        true => "OPEN A SESSION",
        false => "OPEN A FOLDER",
    };
    // The heading, and what the session list says about itself: how many there
    // are, and how many files in the directory could not be described.
    let mut head = vec![Run::tinted(heading, skin.bright)];
    if let Some(note) = picker.note() {
        let room = cols.saturating_sub(heading.chars().count() + 2);
        head.push(Run::tinted(format!("  {}", clip(note, room)), skin.dim));
    }
    say(
        scene,
        head,
        Panel::new(content.x, content.y, content.w, line),
        skin.bright,
    );
    // The folder being listed, in full. The rows under it are names, so this is
    // the only thing on screen saying where in the tree they are, and with the
    // sessions showing it is the folder a session that never noted one would be
    // resumed in.
    say(
        scene,
        vec![Run::tinted(
            clip(&picker.at().display().to_string(), cols),
            skin.body,
        )],
        Panel::new(content.x, content.y + line, content.w, line),
        skin.body,
    );
    // What has been typed, why the list is empty when it is empty for a reason,
    // or why the last press did nothing. A folder with no permission looks
    // exactly like an empty folder otherwise, and a button that silently does
    // not work looks exactly like a button that is broken.
    let mut runs = vec![Run::icon(icons::FILTER.to_string(), skin.dim), Run::plain(" ")];
    let tint = match (picker.refused().or(picker.trouble()), picker.filter()) {
        (Some(why), _) => {
            runs.push(Run::tinted(clip(why, cols), skin.bad));
            skin.bad
        }
        (None, "") => {
            runs.push(Run::tinted("type to narrow the list", skin.dim));
            skin.dim
        }
        (None, typed) => {
            runs.push(Run::tinted(typed, skin.bright));
            skin.bright
        }
    };
    say(
        scene,
        runs,
        Panel::new(content.x, content.y + 2.0 * line, content.w, line),
        tint,
    );

    let list_cols = cols_of(layout.picker_list, frame.pane_column);
    for (index, row) in &layout.picker_rows {
        let Some(entry) = picker.row(*index) else {
            continue;
        };
        let on = *index == picker.cursor();
        if on {
            // Filled solid in the good colour, and written over in the darkest
            // ink the palette has. The quiet band the file explorer marks its
            // open row with was not enough here: the picker is a list of forty
            // folders where the only question is which one Enter opens.
            scene.rect(row.fill(skin.picked));
        }
        // Typing dims what it did not match instead of taking it away, so the
        // list you were reading is still the list in front of you. The answer
        // comes from the model, which is the same answer the arrow keys walk by:
        // a row cannot be dim here and bright to the keyboard.
        // A session whose folder has been deleted is drawn the way an
        // unreadable folder is, because it is the same thing: a row that is
        // there to be seen and cannot be opened.
        let dead = matches!(entry, PickerRow::Session(saved) if saved.gone);
        let tint = match (on, picker.matched(entry), entry) {
            (true, _, _) => skin.picked_ink,
            (false, false, _) => skin.dim,
            (false, true, PickerRow::Locked { .. }) => skin.bad,
            (false, true, _) if dead => skin.bad,
            (false, true, _) => skin.body,
        };
        let icon = match entry {
            PickerRow::Here => icons::FOLDER_OPEN,
            PickerRow::Up => icons::UP,
            PickerRow::Recent(_) => icons::RECENT,
            PickerRow::Folder { .. } => icons::FOLDER,
            PickerRow::Locked { .. } => icons::LOCKED,
            // The clock the remembered folders carry, since a saved session is
            // the same idea: something from before. The lock when it cannot be
            // opened, which is what that glyph says everywhere else here.
            PickerRow::Session(saved) => match saved.gone {
                true => icons::LOCKED,
                false => icons::RECENT,
            },
        };
        // The mark that opens and shuts the folder, where the layout put it, so
        // the glyph is inside the region that answers for pressing it.
        let (indent, wide) = picker_indent(entry.depth(), frame.pane_column, list_cols);
        if let Some(open) = entry.open() {
            let mark = match open {
                true => icons::COLLAPSE,
                false => icons::EXPAND,
            };
            let hot = frame.hot == Some(Hit::PickerMark(*index));
            let ink = match (on, hot) {
                (true, _) => skin.picked_ink,
                (false, true) => skin.bright,
                (false, false) => tint,
            };
            say(
                scene,
                vec![Run::icon(mark.to_string(), ink)],
                Panel::new(row.x + indent, row.y, wide, line),
                ink,
            );
        }
        let start = indent + wide;
        let room = cols
            .saturating_sub(ROW_ICON_COLUMNS + 1 + (start / frame.pane_column.max(1.0)) as usize)
            .max(1);
        say(
            scene,
            vec![
                Run::icon(icon.to_string(), tint),
                Run::tinted(format!(" {}", clip(&picker.label(entry), room)), tint),
            ],
            Panel::new(row.x + start, row.y, (row.w - start).max(1.0), line),
            tint,
        );
    }
    scrollbar(
        scene,
        skin,
        layout.picker,
        picker.thumb(layout.picker_capacity(size)),
    );

    // The keys, spelled out. Nothing else in this window needs them written
    // down, but this is the first thing a new install shows and it is the one
    // place where there is no pane to experiment in.
    //
    // Placed off the button rather than off the bottom of the box, so the two
    // cannot end up on top of each other when the button's height changes.
    let open = layout.picker_open;
    say(
        scene,
        vec![Run::tinted(
            clip(
                "enter opens \u{2022} right walks in \u{2022} left goes out \u{2022} esc quits",
                cols,
            ),
            skin.dim,
        )],
        Panel::new(content.x, open.y - GAP - line, content.w, line),
        skin.dim,
    );
    // A surface of its own, a cut corner and an accent edge, so the two things
    // here that are buttons read as buttons. They used to be `tab_idle` with a
    // hairline, which is the quietest surface in the palette.
    //
    // The second one swaps the list. Its word says what pressing it gets you,
    // not what is on screen, which is the only reading of a button that does
    // not need a caption to go with it.
    let toggle = match picker.on_sessions() {
        true => PICKER_FOLDERS_LABEL,
        false => PICKER_SESSIONS_LABEL,
    };
    for (panel, hit, icon, label) in [
        (open, Hit::PickerOpen, icons::CONFIRM, PICKER_OPEN_LABEL),
        (
            layout.picker_sessions,
            Hit::PickerSessions,
            icons::RECENT,
            toggle,
        ),
    ] {
        if panel.w < 1.0 || panel.h < 1.0 {
            continue;
        }
        let face = match frame.hot == Some(hit) {
            true => skin.button_hot,
            false => skin.button,
        };
        scene.rect(panel_fill(panel, face));
        scene.rect(panel_edge(panel, skin.edge_focus));
        say(
            scene,
            vec![
                Run::icon(icon.to_string(), skin.bright),
                Run::tinted(format!(" {label}"), skin.bright),
            ],
            Panel::new(
                panel.x + frame.pane_column,
                panel.y + PICKER_OPEN_PAD,
                (panel.w - frame.pane_column).max(1.0),
                line,
            ),
            skin.bright,
        );
    }
}

/// The settings panel: the whole surface under the title strip while it is up.
///
/// Two columns. The label on the left says what a setting is called in the file,
/// so the panel doubles as the documentation for editing that file by hand, and
/// the value sits in one column down the right where it can be scanned. Nothing
/// here is a form widget: a value is text, and what makes it a control is that
/// the arrow keys and a click on it change it.
fn settings_panel(scene: &mut Scene, frame: &Frame) {
    let Some(panel) = frame.settings else {
        return;
    };
    let (skin, layout) = (frame.skin, frame.layout);
    let box_ = layout.settings;
    if box_.w < 1.0 || box_.h < 1.0 {
        return;
    }
    scene.rect(panel_fill(box_, skin.panel));
    scene.rect(panel_edge(box_, skin.edge_focus));

    let size = frame.pane_size;
    let line = Text::line_for(size);
    let column = frame.pane_column.max(1.0);
    let content = box_.inset(PAD);
    let cols = cols_of(content, column);
    let say = |scene: &mut Scene, runs: Vec<Run>, at: Panel, tint: [u8; 4]| {
        scene.text(Text::rich(runs, at, size, tint));
    };

    // The heading, and the file it is a view of. The path is a reading further
    // down as well, where it can be read in full; this is the one line at the
    // top saying that the panel and the file are the same thing.
    say(
        scene,
        vec![
            Run::icon(icons::SETTINGS.to_string(), skin.bright),
            Run::tinted(" SETTINGS", skin.bright),
        ],
        Panel::new(content.x, content.y, content.w, line),
        skin.bright,
    );

    let close = layout.settings_close;
    if close.w >= 1.0 {
        if frame.hot == Some(Hit::SettingsClose) {
            scene.rect(close.fill(skin.close_hot));
        }
        say(
            scene,
            vec![Run::icon(icons::CLOSE.to_string(), skin.bright)],
            close,
            skin.bright,
        );
    }

    let list = layout.settings_list;
    let value_w = settings_value_w(list.w, column);
    let label_cols = cols_of(list, column).saturating_sub(SETTING_VALUE_COLUMNS + 1);
    for (index, row) in &layout.settings_rows {
        let Some(entry) = panel.row(*index) else {
            continue;
        };
        let on = *index == panel.cursor();
        if on {
            // The band and the mark every list in this window marks its current
            // row with, rather than a colour of its own.
            scene.rect(row.fill(skin.strip));
            scene.rect(Panel::new(row.x, row.y, MARK_W, row.h).fill(skin.edge_focus));
        }
        let text_x = row.x + MARK_W + 3.0;
        let label_room = Panel::new(text_x, row.y, (row.w - value_w - MARK_W - 3.0).max(1.0), line);
        let value_at = Panel::new(row.x + row.w - value_w, row.y, value_w, line);
        match entry {
            // A heading is the only thing on its row, and it gets the whole
            // width: `WHICH PANES OPEN` is longer than a label column.
            SettingRow::Heading(name) => say(
                scene,
                vec![Run::tinted(clip(name, cols), skin.title)],
                Panel::new(text_x, row.y, (row.w - MARK_W - 3.0).max(1.0), line),
                skin.title,
            ),
            SettingRow::Reading { label, value } => {
                say(
                    scene,
                    vec![Run::tinted(clip(label, label_cols), skin.dim)],
                    label_room,
                    skin.dim,
                );
                say(
                    scene,
                    vec![Run::tinted(
                        clip(value, SETTING_VALUE_COLUMNS),
                        skin.body,
                    )],
                    value_at,
                    skin.body,
                );
            }
            SettingRow::Setting { key, value, kind } => {
                let tint = if on { skin.bright } else { skin.body };
                say(
                    scene,
                    vec![Run::tinted(clip(key, label_cols), tint)],
                    label_room,
                    tint,
                );
                match kind {
                    // A colour is drawn as itself. A hex string is not a colour
                    // to anyone reading a palette, and this is the panel where
                    // the palette is read.
                    Kind::Colour(rgb) => {
                        let side = (line * 0.6).floor().max(2.0);
                        let up = ((line - side) * 0.5).floor();
                        scene.rect(
                            Panel::new(value_at.x, value_at.y + up, side, side).fill(swatch(*rgb)),
                        );
                        say(
                            scene,
                            vec![Run::tinted(clip(value, SETTING_VALUE_COLUMNS), skin.dim)],
                            Panel::new(
                                value_at.x + side + column,
                                value_at.y,
                                (value_at.w - side - column).max(1.0),
                                line,
                            ),
                            skin.dim,
                        );
                    }
                    // The value of a setting that can change is drawn as the
                    // control it is: accent tinted, and lit under the pointer
                    // the way a window button is.
                    _ => {
                        if frame.hot == Some(Hit::SettingsValue(*index)) {
                            scene.rect(value_at.fill(skin.hot));
                        }
                        say(
                            scene,
                            vec![Run::tinted(
                                clip(value, SETTING_VALUE_COLUMNS),
                                skin.bright,
                            )],
                            value_at,
                            skin.bright,
                        );
                    }
                }
            }
        }
    }
    scrollbar(
        scene,
        skin,
        layout.settings,
        panel.thumb(layout.settings_capacity(size)),
    );

    // What the keys do to the row under the cursor, or why the last change did
    // not land. A panel that writes a file has to say when the file refused.
    let (foot, tint) = match panel.trouble() {
        Some(why) => (clip(why, cols), skin.bad),
        None => (clip(panel.hint(), cols), skin.dim),
    };
    say(
        scene,
        vec![Run::tinted(foot, tint)],
        Panel::new(content.x, content.y + content.h - line, content.w, line),
        tint,
    );
}

/// A colour from the settings file as the renderer wants it. Fully opaque: the
/// swatch is the colour itself, and the panel's own fill is what carries the
/// window's transparency.
fn swatch(rgb: [u8; 3]) -> [f32; 4] {
    [
        rgb[0] as f32 / 255.0,
        rgb[1] as f32 / 255.0,
        rgb[2] as f32 / 255.0,
        1.0,
    ]
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
    let label = drag.view.label();
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
fn scrollbar(scene: &mut Scene, skin: &Skin, panel: Panel, thumb: Option<(f32, f32)>) {
    let Some((top, size)) = thumb else {
        return;
    };
    // The track runs down the right edge, which is the edge the cut takes a
    // triangle out of. Starting it three pixels down put its head inside that
    // triangle, hanging in the air outside the pane, so it starts below the cut
    // instead: the cut reaches `cut` in from the corner along both edges, and
    // the track is already `SCROLL_GAP` in from the right.
    let head = (cut_of(panel) - SCROLL_GAP).max(3.0);
    let track = Panel::new(
        panel.x + panel.w - SCROLL_W - SCROLL_GAP,
        panel.y + head,
        SCROLL_W,
        (panel.h - head - 3.0).max(1.0),
    );
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
                box_.y + row as f32 * line,
                (stop - at) as f32 * frame.column,
                line,
            );
            if band.y + band.h <= box_.y + box_.h + 0.5 {
                scene.rect(band.fill(skin.select));
            }
            at = stop;
        }
    }
    let marker = if state.phase.busy() { "\u{2026}" } else { "\u{203a}" };
    scene.text(
        Text::rich(
            vec![
                Run::tinted(format!("{marker} "), skin.dim),
                Run::tinted(frame.prompt.text(), skin.bright),
            ],
            box_,
            frame.body_size,
            skin.bright,
        )
        // Wrap by glyph, so counting columns lands the caret where the glyph
        // actually is. Word wrap would put it a word away on every long line.
        .wrap_anywhere(),
    );
    let at = frame.prompt.caret() + PROMPT_COLUMNS;
    let (row, column) = (at / columns, at % columns);
    let caret = Panel::new(
        box_.x + column as f32 * frame.column,
        box_.y + row as f32 * line,
        2.0,
        line,
    );
    if caret.y + caret.h <= box_.y + box_.h + 0.5 {
        scene.rect(caret.fill(skin.caret));
    }
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
fn columns_in(width: f32, column: f32) -> usize {
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
fn cols_of(panel: Panel, column: f32) -> usize {
    columns_in(panel.inset(PAD).w, column)
}

/// How tall the prompt has to be to hold `chars` characters.
///
/// Grows a line at a time up to `max_rows`, then scrolls inside itself. A
/// prompt that grows without limit eventually eats the conversation it is
/// about, and how much of the window that is worth is a matter of taste, which
/// is why the ceiling is a setting.
pub fn input_height(width: f32, column: f32, chars: usize, line: f32, max_rows: usize) -> f32 {
    let inner = (width - 2.0 * GAP - 2.0 * PAD).max(column);
    let columns = columns_in(inner, column);
    let rows = (chars + PROMPT_COLUMNS + 1)
        .div_ceil(columns)
        .clamp(1, max_rows.max(1));
    // The strip, not the box inside it: the layout insets this by `GAP` before
    // the prompt gets it, and forgetting that cost the last row of a full one.
    (rows as f32 * line + 2.0 * INPUT_PAD + 2.0 * GAP).max(INPUT_H)
}

fn clip(text: &str, chars: usize) -> String {
    let mut out: String = text.chars().take(chars).collect();
    if text.chars().count() > chars {
        out.push('\u{2026}');
    }
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
fn fit_name(path: &str, cols: usize) -> String {
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
mod tests {
    use super::*;
    use crate::config::Config;

    fn shape<'a>(dock: &'a Dock, files: &[&str]) -> Shape<'a> {
        scrolled_shape(dock, files, 0)
    }

    /// The same, with the explorer list scrolled `first` rows down.
    fn scrolled_shape<'a>(dock: &'a Dock, files: &[&str], first: usize) -> Shape<'a> {
        Shape {
            shaded: false,
            dock,
            menu: None,
            picker: None,
            settings: None,
            file_labels: files.iter().map(|f| f.to_string()).collect(),
            file_first: first,
            column: 8.0,
            pane_size: 13.0,
            pane_column: 8.0,
            input_h: INPUT_H,
            left_width: LEFT_WIDTH,
            top_height: TOP_HEIGHT,
        }
    }

    /// The same with the two dividers put where the test wants them.
    fn split_shape(dock: &Dock, left_width: f32, top_height: f32) -> Shape<'_> {
        Shape {
            left_width,
            top_height,
            ..shape(dock, &[])
        }
    }

    /// A prompt holding `text` with the caret at `at`.
    fn typed_prompt(text: &str, at: usize) -> crate::prompt::Prompt {
        let mut prompt = crate::prompt::Prompt::default();
        prompt.insert(text);
        prompt.place(at);
        prompt
    }

    fn busy_state() -> State {
        let mut state = State::new();
        state.apply(noob_proto::Event::SessionStart {
            id: "s1".into(),
            workspace: "/home/hec/workspace/noob-cli".into(),
            model: "laguna-s21".into(),
            resumed: false,
        });
        state.apply(noob_proto::Event::TurnStart { turn: 1 });
        state.apply(noob_proto::Event::TextDelta {
            d: "looking at it now".into(),
        });
        state.apply(noob_proto::Event::ToolStart {
            call_id: "c1".into(),
            name: "bash".into(),
            brief: "cargo test".into(),
            args: serde_json::json!({"cmd": "cargo test --workspace"}),
        });
        state.apply(noob_proto::Event::ToolStart {
            call_id: "c2".into(),
            name: "plan".into(),
            brief: "2 items".into(),
            args: serde_json::json!({"todos": [
                {"content": "read it", "status": "completed"},
                {"content": "fix it", "status": "in_progress"},
            ]}),
        });
        state.apply(noob_proto::Event::ToolStart {
            call_id: "c3".into(),
            name: "subagent".into(),
            brief: "research".into(),
            args: serde_json::json!({"prompt": "search the web"}),
        });
        // The admission above is the parent asking; the child's own frames are
        // what the fleet is drawn from.
        state.apply(noob_proto::Event::AgentSpawn {
            agent_id: "agent-1".into(),
            prompt: "search the web".into(),
            tools: "web".into(),
        });
        state.apply(noob_proto::Event::AgentStateChanged {
            agent_id: "agent-1".into(),
            state: noob_proto::AgentState::Running,
            detail: None,
        });
        state.apply(noob_proto::Event::AgentOutput {
            agent_id: "agent-1".into(),
            line: "* websearch search".into(),
        });
        state.apply(noob_proto::Event::FileEdit {
            path: "src/calc.py".into(),
            span: noob_proto::Span {
                start: 2,
                end: 2,
                kind: None,
                name: None,
            },
            before: "    return a - b".into(),
            after: "    return a + b".into(),
            call_id: Some("c4".into()),
        });
        state.apply(noob_proto::Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 1816,
                cached_prompt: 1200,
                completion: 42,
                context_total: 65536,
            },
        });
        state
    }

    struct Rendered {
        scene: Scene,
        layout: Layout,
        skin: Skin,
    }

    fn render(state: &State, w: f32, h: f32, dock: &Dock, files: &[&str]) -> Rendered {
        render_with(state, w, h, dock, files, &Monitor::new(), None)
    }

    /// The window has to say which build it is, or a tester cannot tell two of
    /// them apart. The crate version alone cannot: it does not move between
    /// commits, so `build.rs` stamps the commit into it.
    #[test]
    fn the_title_bar_names_the_build() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &[]);
        let text = text_of(&out.scene);
        // The window is NO0B and has one name. It used to draw the product name
        // twice, "NO0B \u{25b8} CLIppy", and the second one is gone.
        assert!(text.contains("NO0B"), "{text}");
        assert!(!text.contains("CLIppy"), "the old name is still drawn: {text}");
        assert!(
            text.contains(env!("NO0B_BUILD")),
            "the build stamp {:?} is not on screen: {text}",
            env!("NO0B_BUILD")
        );
        assert!(
            env!("NO0B_BUILD").starts_with(env!("CARGO_PKG_VERSION")),
            "the stamp has to start with the version, got {:?}",
            env!("NO0B_BUILD")
        );
    }

    /// What item 21 asked for, read left to right: the orb, the name, the
    /// marker, the version.
    ///
    /// Both halves of that. The orb is drawn, as discs inside the leftmost
    /// [`ORB_W`] of the strip, and no text starts inside that square, so the two
    /// share the strip instead of overlapping.
    #[test]
    fn the_title_strip_reads_orb_then_name_then_marker_then_version() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &[]);
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
        assert!(
            line.starts_with(&format!("NO0B \u{25b8} {VERSION}")),
            "the strip reads {line:?}"
        );
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

    /// Every disc of the orb in a scene.
    ///
    /// A rectangle in the title strip with a corner radius is one: nothing else
    /// up there is round, and the only other rounded rectangles in the window are
    /// the gauge dots, which are in panes below the strip.
    fn discs_of(scene: &Scene) -> Vec<&Rect> {
        scene
            .rects
            .iter()
            .filter(|rect| {
                let [_, y, _, h] = rect.xywh();
                rect.extra()[0] > 0.0 && y + h <= TITLE_H
            })
            .collect()
    }

    /// One frame at a given moment on the orb's clock.
    ///
    /// Its own helper rather than another argument to [`render_with`], which is
    /// already at the argument count clippy allows.
    fn render_at(state: &State, clock: f32) -> Rendered {
        let dock = Dock::new();
        let shape = shape(&dock, &[]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock,
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        });
        Rendered {
            scene,
            layout,
            skin,
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
            drag: None,
            hot: None,
            trouble: None,
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
        // The commit follows the version rather than repeating it.
        assert!(!build_commit().contains(VERSION), "{:?}", build_commit());
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

    /// The strip carries the name and the build stamp, and nothing else.
    ///
    /// It used to carry the phase, the model, the workspace, a resumed marker
    /// and the whole token budget on one unlabelled line. Those are readings
    /// and they are moving to the monitors, so this asserts they are gone from
    /// here rather than that they are here, which is what it asserted before.
    #[test]
    fn the_title_strip_carries_only_the_name_and_the_build() {
        let state = busy_state();
        let out = render(&state, 1400.0, 900.0, &Dock::new(), &[]);
        let title = out
            .scene
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text.contains("NO0B")))
            .expect("the title strip names the window");
        let line: String = title.runs.iter().map(|run| run.text.as_str()).collect();
        assert!(line.contains(env!("NO0B_BUILD")), "{line}");
        // The budget was a whole line of readings up here. It is a set of
        // monitor rows now, so what is asserted is that none of its words are.
        for evicted in [
            state.phase.word().to_lowercase(),
            state.model.clone(),
            short_path(&state.workspace),
            String::from("prefilled"),
            String::from("requests"),
        ] {
            assert!(
                !line.contains(&evicted),
                "{evicted:?} is still in the title strip: {line}"
            );
        }
        // And the stamp is readable. It was in the dim tint, the faintest the
        // palette has, and two builds could not be told apart by it.
        let stamp = title
            .runs
            .iter()
            .find(|run| run.text.contains(env!("NO0B_BUILD")))
            .expect("the build stamp is a run of its own");
        assert_eq!(stamp.color, Some(out.skin.title));
        assert_ne!(stamp.color, Some(out.skin.dim));
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

    fn render_with(
        state: &State,
        w: f32,
        h: f32,
        dock: &Dock,
        files: &[&str],
        monitor: &Monitor,
        drag: Option<Drag>,
    ) -> Rendered {
        let shape = shape(dock, files);
        let layout = Layout::compute(w, h, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state,
            monitor,
            dock,
            skin: &skin,
            layout: &layout,
            prompt: &typed_prompt("type here", 4),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            drag,
            hot: None,
            trouble: None,
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        });
        Rendered {
            scene,
            layout,
            skin,
        }
    }

    fn text_of(scene: &Scene) -> String {
        scene
            .texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
            .collect()
    }

    #[test]
    fn the_default_arrangement_puts_every_space_on_screen() {
        let dock = Dock::new();
        for (w, h) in [(1200.0, 800.0), (700.0, 460.0), (2200.0, 1400.0)] {
            let out = render(&busy_state(), w, h, &dock, &["a.rs"]);
            for space in Space::ALL {
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

    /// Whether a rectangle of this colour is drawn exactly over `box_`, at
    /// `height` from its top.
    fn covered(out: &Rendered, box_: Panel, height: f32, want: [f32; 4]) -> bool {
        out.scene.rects.iter().any(|rect| {
            let [x, y, w, h] = rect.xywh();
            (x - box_.x).abs() < 0.01
                && (y - box_.y).abs() < 0.01
                && (w - box_.w).abs() < 0.01
                && (h - height).abs() < 0.01
                && rect.rgba() == want
        })
    }

    /// The rectangle of this colour drawn at the top-left of `box_`, whatever
    /// its width. What an accent line stopping short of the cut needs, since
    /// [`covered`] insists on the full width.
    fn topped(out: &Rendered, box_: Panel, height: f32, want: [f32; 4]) -> Option<Rect> {
        out.scene
            .rects
            .iter()
            .find(|rect| {
                let [x, y, _, h] = rect.xywh();
                (x - box_.x).abs() < 0.01
                    && (y - box_.y).abs() < 0.01
                    && (h - height).abs() < 0.01
                    && rect.rgba() == want
            })
            .copied()
    }

    /// The showing tab is the pane's own surface with the view's accent on top,
    /// and it takes the pane's cut corner. It used to be a block in a colour of
    /// its own, standing on a filled strip, which read as a button.
    #[test]
    fn the_showing_tab_wears_the_pane_s_surface_and_its_view_s_accent() {
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
            let accent = topped(&out, *tab, ACCENT_H, out.skin.view(active))
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
            // And the accent is the view's own, not one colour for every strip.
            assert_ne!(out.skin.view(active), out.skin.edge_focus);
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
                    clear >= cut - 0.01,
                    "{space:?}: {:?} is {clear}px into a {cut}px cut",
                    rect.xywh()
                );
            }
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
                    topped(&out, *tab, ACCENT_H, out.skin.view(*view)).is_none(),
                    "{view:?} has an accent line and is not showing"
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

    /// A state that has touched every named file, in order, with the last one
    /// open. The paths are what the agent would have sent, so `short_name` and
    /// the type icons are exercised rather than bypassed.
    fn touched(paths: &[&str]) -> State {
        let mut state = State::new();
        for path in paths {
            state.apply(noob_proto::Event::FileEdit {
                path: (*path).into(),
                span: noob_proto::Span {
                    start: 1,
                    end: 1,
                    kind: None,
                    name: None,
                },
                before: "was".into(),
                after: "is".into(),
                call_id: None,
            });
        }
        state
    }

    fn labels(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| short_name(p)).collect()
    }

    /// The list runs down the pane, one row per file, not across it. This was a
    /// horizontal strip of tabs and the direct instruction was "vertical, like
    /// in visual studio code".
    #[test]
    fn the_file_list_is_a_column_with_one_row_per_file() {
        let paths = ["src/calc.py", "README.md", "src/main.rs"];
        let state = touched(&paths);
        let names = labels(&paths);
        let out = render(
            &state,
            1400.0,
            900.0,
            &Dock::new(),
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        assert_eq!(out.layout.file_rows.len(), 3);
        let line = Text::line_for(13.0);
        let mut last: Option<Panel> = None;
        for (step, (index, row)) in out.layout.file_rows.iter().enumerate() {
            assert_eq!(*index, step, "the rows are the files, in order");
            assert!((row.h - line).abs() < 0.01, "a row is one line tall: {row:?}");
            if let Some(above) = last {
                assert!((row.x - above.x).abs() < 0.01, "the rows are a column");
                assert!(
                    (row.y - (above.y + line)).abs() < 0.01,
                    "row {step} does not sit under the one before it"
                );
            }
            last = Some(*row);
        }
        // The list is on the left and the file is beside it, not under it.
        let (list, diff) = (out.layout.file_list, out.layout.file_diff);
        assert!(list.w > 1.0 && diff.w > 1.0);
        assert!((list.x + list.w - diff.x).abs() < 0.01, "{list:?} {diff:?}");
        assert!((list.y - diff.y).abs() < 0.01, "the two columns start level");
        // Every name is there, and the type icon in front of it.
        let text = text_of(&out.scene);
        for name in &names {
            assert!(text.contains(name.as_str()), "{name} is not in the list: {text}");
        }
        for path in paths {
            let icon = crate::icons::for_path(path).to_string();
            assert!(text.contains(&icon), "{path} has no type icon");
        }
    }

    /// One row is marked, and it is the open file's. A band and an accent down
    /// the left edge rather than a block in another colour: a block standing on
    /// the pane's own surface is what made the old tabs read as buttons.
    #[test]
    fn the_open_file_s_row_is_the_marked_one() {
        let paths = ["src/calc.py", "src/main.rs"];
        let mut state = touched(&paths);
        state.open_file = 0;
        let names = labels(&paths);
        let out = render(
            &state,
            1400.0,
            900.0,
            &Dock::new(),
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        for (index, row) in &out.layout.file_rows {
            let open = *index == state.open_file;
            assert_eq!(
                covered(&out, *row, row.h, out.skin.strip),
                open,
                "row {index} and its band disagree about being open"
            );
            let mark = topped(&out, *row, row.h, out.skin.view(View::Files));
            assert_eq!(
                mark.is_some(),
                open,
                "row {index} and its mark disagree about being open"
            );
            if let Some(mark) = mark {
                assert!((mark.xywh()[2] - MARK_W).abs() < 0.01, "{:?}", mark.xywh());
            }
            let label = out
                .scene
                .texts
                .iter()
                .find(|text| row.contains(text.at.x + 1.0, text.at.y + 1.0))
                .unwrap_or_else(|| panic!("row {index} has no label"));
            assert_eq!(
                label.color,
                if open { out.skin.bright } else { out.skin.body },
                "row {index} is not tinted for being open or not"
            );
        }
    }

    /// A list longer than the pane shows a screenful, scrolls to the rest, and
    /// says so with a thumb. The window comes from text-geometry, so the rows
    /// drawn and the rows a scroll position names cannot disagree.
    #[test]
    fn a_list_longer_than_the_pane_scrolls_instead_of_dropping_files() {
        let paths: Vec<String> = (0..40).map(|n| format!("src/file{n}.rs")).collect();
        let borrowed: Vec<&str> = paths.iter().map(String::as_str).collect();
        let mut state = touched(&borrowed);
        state.open_file = 0;
        let names = labels(&borrowed);
        let short: Vec<&str> = names.iter().map(String::as_str).collect();

        let dock = Dock::new();
        let out = render(&state, 1400.0, 900.0, &dock, &short);
        let shown = out.layout.file_rows.len();
        assert!(shown > 4, "only {shown} rows fit");
        assert!(shown < paths.len(), "all {shown} rows fit, nothing to scroll");
        assert_eq!(out.layout.file_rows[0].0, 0, "the top of the list");

        // Scrolled down, the same rows carry later files, and none of them are
        // drawn outside the column.
        let scrolled = Layout::compute(1400.0, 900.0, &scrolled_shape(&dock, &short, 5));
        assert_eq!(scrolled.file_rows.len(), shown);
        assert_eq!(scrolled.file_rows[0].0, 5);
        for (_, row) in &scrolled.file_rows {
            assert!(
                row.y >= scrolled.file_list.y - 0.01
                    && row.y + row.h <= scrolled.file_list.y + scrolled.file_list.h + 0.01,
                "{row:?} is outside {:?}",
                scrolled.file_list
            );
        }
        // Past the end clamps to the last screenful rather than to nothing.
        let far = Layout::compute(1400.0, 900.0, &scrolled_shape(&dock, &short, 999));
        assert_eq!(far.file_rows.len(), shown);
        assert_eq!(far.file_rows.last().unwrap().0, paths.len() - 1);

        // And the list carries a thumb, because it does not all fit.
        let rows = out.layout.rows(out.layout.file_list, 13.0);
        assert!(state.files_thumb(rows).is_some(), "no thumb on a long list");
        assert!(
            State::new().files_thumb(rows).is_none(),
            "a thumb with nothing to scroll"
        );
    }

    /// The file is the thing being looked at, so it keeps its floor whatever the
    /// list wants. The pane is narrow: at the smallest window the layout allows,
    /// the file view lives in the right-hand column.
    #[test]
    fn the_file_keeps_room_to_be_read_beside_the_list() {
        let paths = ["src/averyverylongfilename.rs", "src/other.rs"];
        let state = touched(&paths);
        let names = labels(&paths);
        let short: Vec<&str> = names.iter().map(String::as_str).collect();
        for (w, h) in [(680.0, 380.0), (900.0, 700.0), (2200.0, 1400.0)] {
            let out = render(&state, w, h, &Dock::new(), &short);
            let (list, diff) = (out.layout.file_list, out.layout.file_diff);
            assert!(list.w > 1.0, "no list at {w}x{h}");
            assert!(
                cols_of(diff, 8.0) >= DIFF_MIN_COLUMNS,
                "the file has {} columns at {w}x{h}, under the {DIFF_MIN_COLUMNS} floor",
                cols_of(diff, 8.0)
            );
            assert!(
                cols_of(list, 8.0) <= LIST_MAX_COLUMNS,
                "the list is {} columns wide at {w}x{h}",
                cols_of(list, 8.0)
            );
        }
    }

    /// Where a character is, in the file view, is measured from the file's own
    /// box and not from the pane. The list is not text to be selected: a drag
    /// starting on a row is a drag on the list, and hit testing the whole pane
    /// would put every click in the file a list's width away from the glyph
    /// under it.
    #[test]
    fn the_file_s_text_is_measured_from_its_own_column() {
        let paths = ["src/calc.py", "src/main.rs"];
        let state = touched(&paths);
        let names = labels(&paths);
        let out = render(
            &state,
            1400.0,
            900.0,
            &Dock::new(),
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let space = Space::BottomRight;
        assert_eq!(out.layout.content(space), out.layout.file_diff);
        // The other spaces are unchanged: their content is their whole body.
        assert_eq!(
            out.layout.content(Space::Left),
            out.layout.placed(Space::Left).body
        );

        let diff = out.layout.file_diff.inset(PAD);
        let (row, at) = (3usize, 6usize);
        let line = Text::line_for(13.0);
        let cell = out.layout.cell(
            diff.x + at as f32 * 8.0,
            diff.y + row as f32 * line + 2.0,
            13.0,
            8.0,
        );
        assert_eq!(cell, Some((space, row, at)), "measured from the wrong box");

        // And a point on a row of the list is no cell at all.
        let (_, first) = out.layout.file_rows[0];
        assert_eq!(
            out.layout
                .cell(first.x + 4.0, first.y + first.h * 0.5, 13.0, 8.0),
            None
        );
    }

    /// The gutter in front of a file's text is chrome, not text. One place says
    /// how many columns it takes, so the wrapping the file is drawn with and the
    /// column a click resolves to cannot drift apart, which is what put file
    /// selection four characters along.
    #[test]
    fn a_file_s_line_numbers_are_not_part_of_its_line() {
        let box_ = Panel::new(0.0, 0.0, 8.0 * 40.0 + 2.0 * PAD, 100.0);
        assert_eq!(text_columns(View::Files, box_, 8.0), (40 - GUTTER, GUTTER));
        for view in View::ALL.into_iter().filter(|v| *v != View::Files) {
            assert_eq!(text_columns(view, box_, 8.0), (40, 0), "{view:?}");
        }
        // A box narrower than the gutter still wraps in at least one column.
        let sliver = Panel::new(0.0, 0.0, 8.0 + 2.0 * PAD, 100.0);
        assert_eq!(text_columns(View::Files, sliver, 8.0).0, 1);
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

    /// A drop lands somewhere. Every point inside a space's body or its strip
    /// names that space, or a drag can be released over nothing.
    #[test]
    fn every_point_in_a_space_names_that_space_for_a_drop() {
        let dock = Dock::new();
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        // How far in from an edge a point has to be to be in the pane rather
        // than on a divider: the band a divider is grabbed by is [`GRAB`] wider
        // than the gap it stands in on each side, so the outermost few pixels of
        // a pane beside one belong to the divider. A release there is
        // `Landing::Nowhere`, which is what the margin between two panes has
        // always been.
        let in_ = GRAB + 4.0;
        for space in Space::ALL {
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
        dock.move_view(View::Session, Space::Left);
        let out = render(&busy_state(), 1400.0, 900.0, &dock, &["a.rs"]);
        let left: Vec<View> = out
            .layout
            .placed(Space::Left)
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
        dock.move_view(View::Output, Space::TopRight);
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        assert_eq!(out.layout.placed(Space::Left).strip.w, 0.0);
        let top = out.layout.placed(Space::TopRight);
        assert!(top.body.w > 1000.0, "{:?}", top.body);
    }

    /// How wide and how tall a space is, strip and body together.
    fn box_of(layout: &Layout, space: Space) -> (f32, f32) {
        let placed = layout.placed(space);
        (
            placed.strip.w,
            placed.body.y + placed.body.h - placed.strip.y,
        )
    }

    /// Item 16: the two ratios decide where the dividers fall, and the spaces
    /// still cover the body between them with one gap in each direction.
    #[test]
    fn the_two_ratios_decide_where_the_dividers_fall() {
        let dock = Dock::new();
        for (left_width, top_height) in [(0.3, 0.3), (LEFT_WIDTH, TOP_HEIGHT), (0.7, 0.7)] {
            let layout = Layout::compute(1400.0, 900.0, &split_shape(&dock, left_width, top_height));
            let body = layout.column_divider.track;
            let room = body.w - GAP;
            let (left_w, _) = box_of(&layout, Space::Left);
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

            let right = layout.row_divider.track;
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
            assert_eq!(layout.column_divider.band.w, GAP + GRAB * 2.0);
            assert!((layout.column_divider.band.x + GRAB - left_w - body.x).abs() <= 0.01);
            assert_eq!(layout.row_divider.band.h, GAP + GRAB * 2.0);
            assert!((layout.row_divider.band.y + GRAB - right.y - top_h).abs() <= 0.01);
        }
    }

    /// The pointer puts a divider where the pointer is: the ratio a drag reads
    /// off a position, laid out again, puts the gap back under that position.
    #[test]
    fn a_dragged_divider_lands_under_the_pointer() {
        let dock = Dock::new();
        let layout = Layout::compute(1400.0, 900.0, &split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT));
        let body = layout.column_divider.track;
        let right = layout.row_divider.track;

        for x in [body.x + 400.0, body.x + 700.0, body.x + 1000.0] {
            let moved =
                Layout::compute(1400.0, 900.0, &split_shape(&dock, layout.column_ratio_at(x), 0.46));
            let gap = moved.placed(Space::Left).strip.x + moved.placed(Space::Left).strip.w;
            assert!((gap + GAP * 0.5 - x).abs() <= 1.0, "{x} put the gap at {gap}");
        }
        for y in [right.y + 200.0, right.y + 400.0, right.y + 600.0] {
            let moved =
                Layout::compute(1400.0, 900.0, &split_shape(&dock, 0.54, layout.row_ratio_at(y)));
            let top = moved.placed(Space::TopRight);
            let gap = top.body.y + top.body.h;
            assert!((gap + GAP * 0.5 - y).abs() <= 1.0, "{y} put the gap at {gap}");
        }
    }

    /// A drag thrown past either end of the window stops at the floor. Nothing
    /// collapses: the smallest a space goes is a tab strip and enough pane to
    /// read under it.
    #[test]
    fn a_divider_dragged_past_the_end_stops_at_the_floor() {
        let dock = Dock::new();
        let layout = Layout::compute(1400.0, 900.0, &split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT));
        let column_floor = layout.column_divider.floor;
        let row_floor = layout.row_divider.floor;
        assert!(column_floor > 0.0 && row_floor > 0.0);

        for x in [-4000.0, -1.0, 700.0, 1401.0, 9000.0] {
            let ratio = layout.column_ratio_at(x);
            let moved = Layout::compute(1400.0, 900.0, &split_shape(&dock, ratio, TOP_HEIGHT));
            let (left_w, _) = box_of(&moved, Space::Left);
            let (right_w, _) = box_of(&moved, Space::TopRight);
            assert!(left_w >= column_floor, "{x}: the left column is {left_w}");
            assert!(right_w >= column_floor, "{x}: the right column is {right_w}");
        }
        for y in [-4000.0, -1.0, 500.0, 901.0, 9000.0] {
            let ratio = layout.row_ratio_at(y);
            let moved = Layout::compute(1400.0, 900.0, &split_shape(&dock, LEFT_WIDTH, ratio));
            let (_, top_h) = box_of(&moved, Space::TopRight);
            let (_, bottom_h) = box_of(&moved, Space::BottomRight);
            assert!(top_h >= row_floor, "{y}: the top space is {top_h}");
            assert!(bottom_h >= row_floor, "{y}: the bottom space is {bottom_h}");
        }

        // A ratio out of a settings file nobody clamped is held the same way.
        for ratio in [0.0, 1.0, -5.0, 12.0] {
            let moved = Layout::compute(1400.0, 900.0, &split_shape(&dock, ratio, ratio));
            assert!(box_of(&moved, Space::Left).0 >= column_floor, "{ratio}");
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
        let (left_w, _) = box_of(&layout, Space::Left);
        let (right_w, _) = box_of(&layout, Space::TopRight);
        assert!(left_w < layout.column_divider.floor, "the floor still fits");
        assert!((left_w - right_w).abs() <= 1.0, "{left_w} against {right_w}");
        assert!(left_w > 1.0 && right_w > 1.0, "a column collapsed");

        // The same downward: a right column with no room for two floors.
        let short = Layout::compute(1400.0, 180.0, &split_shape(&dock, LEFT_WIDTH, 0.9));
        let (_, top_h) = box_of(&short, Space::TopRight);
        let (_, bottom_h) = box_of(&short, Space::BottomRight);
        assert!((top_h - bottom_h).abs() <= 1.0, "{top_h} against {bottom_h}");
        assert!(top_h > 1.0 && bottom_h > 1.0, "a space collapsed");
    }

    /// The band is wider than the gap it stands in, or a six pixel line is a
    /// target nobody can hit. It wins against the panes it reaches into, and the
    /// pane wins back one pixel outside it.
    #[test]
    fn a_divider_is_grabbed_by_a_band_wider_than_the_gap() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &[]);
        let band = out.layout.column_divider.band;
        assert!(band.w > GAP, "the band is no wider than the gap");
        let y = band.y + TAB_H + 20.0;
        for x in [band.x + 0.5, band.x + band.w * 0.5, band.x + band.w - 0.5] {
            assert_eq!(out.layout.hit(x, y), Some(Hit::ColumnDivider), "at {x}");
        }
        assert_eq!(out.layout.hit(band.x - 1.0, y), Some(Hit::Body(Space::Left)));
        assert_eq!(
            out.layout.hit(band.x + band.w + 1.0, y),
            Some(Hit::Body(Space::TopRight))
        );

        let band = out.layout.row_divider.band;
        assert!(band.h > GAP);
        let x = band.x + band.w * 0.5;
        for y in [band.y + 0.5, band.y + band.h * 0.5, band.y + band.h - 0.5] {
            assert_eq!(out.layout.hit(x, y), Some(Hit::RowDivider), "at {y}");
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
        // And a divider is not a drop target: a tab let go on one goes back
        // where it was, the way it does on the margin between two panes.
        assert_eq!(out.layout.landing(x, band.y + 1.0), Landing::Nowhere);
    }

    /// A divider beside an empty space has nothing to divide, so it is not
    /// there. The space that gave its room away is what makes it so, and the
    /// remaining spaces still fill the window.
    #[test]
    fn a_divider_beside_an_empty_or_folded_space_is_not_there() {
        // Nothing on the left: one column, so no divider between two of them.
        let mut dock = Dock::new();
        dock.move_view(View::Output, Space::TopRight);
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        assert!(!out.layout.column_divider.live());
        assert!(out.layout.row_divider.live(), "the two right spaces are still there");
        assert!(box_of(&out.layout, Space::TopRight).0 > 1000.0, "the width was handed over");

        // Nothing in the bottom right: one space in that column, so no divider
        // across it, and the vertical one is still there.
        let mut dock = Dock::new();
        for view in [View::Files, View::Debug] {
            dock.move_view(view, Space::TopRight);
        }
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        assert!(out.layout.column_divider.live());
        assert!(!out.layout.row_divider.live());
        let (_, top_h) = box_of(&out.layout, Space::TopRight);
        let body = out.layout.column_divider.track;
        assert!((top_h - body.h).abs() <= 1.0, "the height was handed over");

        // A folded space is already as short as it goes: the fold owns the
        // height until it is opened, so there is nothing to drag.
        let mut dock = Dock::new();
        dock.slot_mut(Space::TopRight).folded = true;
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        assert!(!out.layout.row_divider.live());
        assert!(out.layout.column_divider.live(), "the columns can still be moved");

        // And with no divider under it, the point that would have been on one
        // belongs to the pane again.
        let mut dock = Dock::new();
        dock.move_view(View::Output, Space::TopRight);
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        let full = render(&busy_state(), 1200.0, 800.0, &Dock::new(), &[]);
        let band = full.layout.column_divider.band;
        let (x, y) = (band.x + band.w * 0.5, band.y + TAB_H + 20.0);
        assert_eq!(full.layout.hit(x, y), Some(Hit::ColumnDivider));
        assert_eq!(out.layout.hit(x, y), Some(Hit::Body(Space::TopRight)));
    }

    /// The three shapes with no panes in them have no dividers either. A band
    /// left behind by a shape change is a press that lands on something nobody
    /// can see.
    #[test]
    fn no_divider_survives_a_shape_change() {
        let dock = Dock::new();
        let open = Layout::compute(1200.0, 800.0, &split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT));
        assert!(open.column_divider.live() && open.row_divider.live());
        let band = open.column_divider.band;
        let (x, y) = (band.x + band.w * 0.5, band.y + TAB_H + 20.0);

        let picker = a_picker(&["src", "docs"], &[]);
        let panel = a_settings_panel(&Config::default());
        for (what, shape) in [
            ("shaded", Shape { shaded: true, ..split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT) }),
            ("picking", Shape { picker: Some(&picker), ..split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT) }),
            ("settings", Shape { settings: Some(&panel), ..split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT) }),
        ] {
            let layout = Layout::compute(1200.0, 800.0, &shape);
            assert!(!layout.column_divider.live(), "{what}");
            assert!(!layout.row_divider.live(), "{what}");
            assert!(
                !matches!(layout.hit(x, y), Some(Hit::ColumnDivider | Hit::RowDivider)),
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
                landing: Landing::In(Space::Left, None),
            }),
        );
        // The box covers the whole space: its tab strip and its pane.
        let placed = dragging.layout.placed(Space::Left);
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
        let in_ = ghost_of(Landing::In(Space::Left, None), (400.0, 500.0));
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
            crate::select::Selection::new(View::Output, crate::select::Spot::new(last - 2, 6));
        selection.extend(crate::select::Spot::new(last, 5));
        state.selection = Some(selection);

        let dock = Dock::new();
        let shape = shape(&dock, &["a.rs"]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
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
            drag: None,
            hot: None,
            trouble: None,
            selection: Some(selection),
            menu: None,
            picker: None,
            settings: None,
        });

        let body = layout.placed(Space::Left).body.inset(PAD);
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

    /// A selection in a pane that is not on screen must not paint anything.
    #[test]
    fn a_selection_in_a_hidden_pane_draws_nothing() {
        let mut state = busy_state();
        state.activity.say("something to select", Tone::Body);
        let last = state.activity.last() - 1;
        let mut selection =
            crate::select::Selection::new(View::Activity, crate::select::Spot::new(last, 0));
        selection.extend(crate::select::Spot::new(last, 9));
        state.selection = Some(selection);

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
            drag: None,
            hot: None,
            trouble: None,
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
        // is, and what it spent getting there. Plus what failed.
        let hardware = seen(View::Hardware);
        assert!(hardware.contains("CPU") || hardware.contains("RAM"), "{hardware}");
        let context = seen(View::Context);
        assert!(context.contains("TOTAL TOOL CALLS"), "{context}");
        assert!(context.contains("LAST PREFILL"), "{context}");
        assert!(context.contains("laguna-s21"), "the model belongs here: {context}");
        assert!(!context.contains("CPU"), "hardware leaked into CONTEXT: {context}");
        let session = seen(View::Session);
        for wanted in ["PREFILLED", "GENERATED", "CACHED", "PREFILL", "DECODE"] {
            assert!(session.contains(wanted), "{wanted} is not in {session}");
        }
        assert!(!session.contains("TOOL CALLS"), "the other pane leaked: {session}");
        assert!(!session.contains("MEAN"), "the all-time readings are gone: {session}");
        assert!(!hardware.contains("DECODE"), "the reverse: {hardware}");
        let debug = seen(View::Debug);
        assert!(debug.contains("failed calls"), "{debug}");
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
        assert_eq!(on_strip.len(), View::ALL.len(), "{on_strip:?}");
    }

    /// The conversation and what has been typed are on screen whichever tab is
    /// up, because they are in a different space.
    ///
    /// This also asserted the token budget, which the title strip used to
    /// carry. It does not any more, and the budget is a monitor reading now.
    #[test]
    fn the_conversation_stays_visible_whatever_the_other_space_shows() {
        let state = busy_state();
        for view in [View::Activity, View::Plan, View::Agents] {
            let mut dock = Dock::new();
            dock.reveal(view);
            let text = text_of(&render(&state, 1400.0, 900.0, &dock, &["calc.py"]).scene);
            assert!(text.contains("looking at it now"), "{view:?}");
            assert!(text.contains("type here"), "{view:?}");
        }
    }

    #[test]
    fn a_changed_file_is_marked_in_its_tab() {
        let text = text_of(&render(&busy_state(), 1400.0, 900.0, &Dock::new(), &["calc.py"]).scene);
        assert!(text.contains("calc.py \u{2022}"), "{text}");
    }

    #[test]
    fn the_file_strip_says_so_when_there_are_no_files() {
        let text = text_of(&render(&State::new(), 1200.0, 800.0, &Dock::new(), &[]).scene);
        assert!(text.contains("no files touched yet"), "{text}");
    }

    #[test]
    fn the_files_view_is_syntax_colored() {
        let mut state = State::new();
        state.apply(noob_proto::Event::FileEdit {
            path: "calc.py".into(),
            span: noob_proto::Span {
                start: 1,
                end: 1,
                kind: None,
                name: None,
            },
            before: String::new(),
            after: "x = \"hello\"  # a note".into(),
            call_id: None,
        });
        let out = render(&state, 1400.0, 900.0, &Dock::new(), &["calc.py"]);
        let colors: Vec<Option<[u8; 4]>> = out
            .scene
            .texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.color))
            .collect();
        assert!(colors.contains(&Some(out.skin.string)), "the string is tinted");
        assert!(colors.contains(&Some(out.skin.comment)), "the comment is tinted");
    }

    /// The bug this replaced: the bar's room was spelled as spaces in the
    /// pane's font while the bar itself was drawn in the transcript's column
    /// width, so the readings landed on top of the bars.
    ///
    /// The bar is a block of dots now, so the thing the reading has to clear is
    /// every dot of it. Found by fill rather than by size: a dot is a few pixels
    /// square, which no size filter can tell from a hairline.
    #[test]
    fn a_monitor_reading_never_lands_on_its_block() {
        let mut state = State::new();
        state.apply(noob_proto::Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 5869,
                cached_prompt: 5348,
                completion: 40,
                context_total: 65536,
            },
        });
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        monitor.sample(&state);

        let mut dock = Dock::new();
        dock.reveal(View::Context);
        // Deliberately mismatched: the transcript's columns are wider than the
        // pane's, which is the situation that produced the overlap.
        for (column, pane_column) in [(8.4, 7.8), (7.8, 8.4), (8.0, 8.0)] {
            let shape = Shape {
                shaded: false,
                dock: &dock,
                menu: None,
                picker: None,
            settings: None,
                file_labels: vec![],
                file_first: 0,
                column,
                pane_size: 13.0,
                pane_column,
                input_h: INPUT_H,
                left_width: LEFT_WIDTH,
                top_height: TOP_HEIGHT,
            };
            let layout = Layout::compute(1400.0, 900.0, &shape);
            let skin = Skin::from(&Config::default());
            let scene = build(&Frame {
                state: &state,
                monitor: &monitor,
                dock: &dock,
                skin: &skin,
                layout: &layout,
                prompt: &crate::prompt::Prompt::default(),
                column,
                pane_column,
                body_size: 14.0,
                pane_size: 13.0,
                clock: 0.0,
                drag: None,
                hot: None,
                trouble: None,
                selection: None,
                menu: None,
                picker: None,
            settings: None,
            });
            let body = layout.placed(Space::TopRight).body;
            let hues: Vec<[f32; 4]> = skin
                .gauges
                .iter()
                .chain(skin.gauges_unlit.iter())
                .copied()
                .collect();
            let dots: Vec<[f32; 4]> = scene
                .rects
                .iter()
                .filter(|r| hues.contains(&r.rgba()) && body.contains(r.xywh()[0], r.xywh()[1]))
                .map(|r| r.xywh())
                .collect();
            assert!(!dots.is_empty(), "no dots were drawn");
            for [_, _, w, h] in &dots {
                assert_eq!(w, h, "a dot is square so its radius rounds it off");
            }
            let block_right = dots.iter().map(|[x, _, w, _]| x + w).fold(0.0f32, f32::max);
            let reading = scene
                .texts
                .iter()
                .find(|t| {
                    body.contains(t.at.x, t.at.y)
                        && t.runs.iter().any(|r| r.text.contains('/'))
                })
                .expect("the bounded reading is on screen");
            assert!(
                reading.at.x >= block_right,
                "reading at {} overlaps a block ending at {block_right} ({column}/{pane_column})",
                reading.at.x
            );
        }
    }

    /// Twenty dots across and four down, so a row is 25% and a dot is 1.25%. 525
    /// of 1000 tokens is 52.5%, which is two whole rows and two dots of a third,
    /// filling from the bottom the way a level meter does. Every dot is drawn
    /// either way, so the block reads as a block rather than as a scatter.
    ///
    /// This asserted eight across and five down, and before that ten columns of
    /// four in one shared gauge colour. The shape is the width and height of the
    /// block: five rows of eight stood the panes on end, which is what item 13
    /// reported, and the same forty dots wide and short is the same reading in a
    /// shape a pane has room for.
    #[test]
    fn a_gauge_is_a_block_of_dots_in_the_metric_s_own_colour() {
        let mut state = State::new();
        state.apply(noob_proto::Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 525,
                cached_prompt: 0,
                completion: 0,
                context_total: 1000,
            },
        });
        let mut monitor = Monitor::new();
        monitor.sample(&state);

        let mut dock = Dock::new();
        dock.reveal(View::Context);
        let shape = Shape {
            shaded: false,
            dock: &dock,
            menu: None,
            picker: None,
            settings: None,
            file_labels: vec![],
            file_first: 0,
            column: 8.0,
            pane_size: 13.0,
            pane_column: 8.0,
            input_h: INPUT_H,
            left_width: LEFT_WIDTH,
            top_height: TOP_HEIGHT,
        };
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
            monitor: &monitor,
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        });

        // CONTEXT is the only bounded reading in this pane with anything in it,
        // and its hue is nobody else's, so filtering by that colour isolates the
        // one block under test.
        let context = monitor
            .context()
            .into_iter()
            .find(|gauge| gauge.key == "context")
            .expect("the context reading");
        let (lit, unlit, ink) = skin.gauge_slot(context.hue);
        let body = layout.placed(Space::TopRight).body;
        let of = |color: [f32; 4]| -> Vec<[f32; 4]> {
            scene
                .rects
                .iter()
                .filter(|r| r.rgba() == color && body.contains(r.xywh()[0], r.xywh()[1]))
                .map(|r| r.xywh())
                .collect()
        };
        let dots = of(lit);
        assert_eq!((DOT_COLUMNS, DOT_ROWS), (20, 4), "the shape item 13 asked for");
        assert_eq!(dots.len(), 42, "52.5% of 80 dots");
        assert_eq!(
            of(unlit).len(),
            DOT_COLUMNS * DOT_ROWS - 42,
            "the rest of the block is still drawn, faintly"
        );

        // Rows, not columns: 42 dots is two full rows of twenty and two of a
        // third, and the part-filled row is the top one.
        let mut rows: Vec<f32> = dots.iter().map(|[_, y, _, _]| *y).collect();
        rows.sort_by(f32::total_cmp);
        rows.dedup();
        assert_eq!(rows.len(), 3);
        let across = |y: f32| dots.iter().filter(|[_, dy, _, _]| *dy == y).count();
        assert_eq!(
            rows.iter().map(|y| across(*y)).collect::<Vec<_>>(),
            vec![2, DOT_COLUMNS, DOT_COLUMNS],
            "the part-filled row is at the top"
        );
        // Evenly pitched, or the block reads as a random scatter.
        let pitch = rows[1] - rows[0];
        for pair in rows.windows(2) {
            assert!((pair[1] - pair[0] - pitch).abs() < 0.01, "{rows:?}");
        }

        // Wider than it is tall, which is the whole of the shape complaint: the
        // block used to be a stack.
        let left = dots.iter().map(|[x, _, _, _]| *x).fold(f32::MAX, f32::min);
        let right = dots.iter().map(|[x, _, w, _]| x + w).fold(0.0f32, f32::max);
        let top = rows[0];
        let foot = of(lit)
            .iter()
            .chain(of(unlit).iter())
            .map(|[_, y, _, h]| y + h)
            .fold(0.0f32, f32::max);
        assert!(
            right - left > 2.0 * (foot - top),
            "the block is {} by {}",
            right - left,
            foot - top
        );

        // The number is the metric's colour and the pane's own size. It was one
        // and a half times the pane size, which read as the loudest thing in the
        // window; the tint is what says it is the reading.
        let reading = scene
            .texts
            .iter()
            .find(|t| t.runs.iter().any(|r| r.text.contains("525 / 1,000")))
            .expect("the context reading is written out");
        assert_eq!(reading.runs[0].color, Some(ink));
        assert_eq!(reading.size, 13.0, "the reading is not the pane size");

        // And an unbounded reading draws no block at all: no track, no dots, and
        // the number where the block would have started.
        let calls = scene
            .texts
            .iter()
            .find(|t| t.runs.iter().any(|r| r.text == "TOTAL TOOL CALLS"))
            .expect("an unbounded row");
        let row = Panel::new(body.x, calls.at.y, body.w, calls.at.h);
        assert!(
            !scene
                .rects
                .iter()
                .any(|r| row.contains(r.xywh()[0], r.xywh()[1] + 0.5 * r.xywh()[3])),
            "something was drawn on the row of an unbounded reading"
        );
    }

    /// The reading is never squeezed: it is the pane's own size at every width,
    /// and it always fits the box it was given. What gives instead is the block,
    /// which is drawn in the room the reading did not need.
    ///
    /// This asserted that a narrow pane drew the number smaller, which was the
    /// answer while the number was one and a half times the pane size and could
    /// afford to lose the difference. At the pane size there is nothing to give
    /// back, so twenty columns of dots go instead.
    #[test]
    fn the_reading_keeps_its_size_and_the_block_gives_way() {
        let mut state = State::new();
        state.apply(noob_proto::Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 1_048_576,
                cached_prompt: 0,
                completion: 0,
                context_total: 2_097_152,
            },
        });
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        let mut dock = Dock::new();
        dock.reveal(View::Context);

        let mut blocks = Vec::new();
        for width in [1600.0, 760.0] {
            let out = render_with(&state, width, 900.0, &dock, &[], &monitor, None);
            let reading = out
                .scene
                .texts
                .iter()
                .find(|t| t.runs.iter().any(|r| r.text.contains("1,048,576 /")))
                .expect("the context reading is on screen");
            // The box it was given has to hold it: a monospace column at this
            // size is the pane's column scaled by the size it is drawn at.
            let chars = reading
                .runs
                .iter()
                .map(|r| r.text.chars().count())
                .sum::<usize>() as f32;
            let column = 8.0 * reading.size / 13.0;
            assert!(
                chars * column <= reading.at.w + 0.01,
                "{width}: {chars} columns of {column} do not fit {}",
                reading.at.w
            );
            assert_eq!(reading.size, 13.0, "{width}: not the pane size");
            blocks.push(dots_in(&out, Space::TopRight).len());
        }
        assert_eq!(blocks[0], DOT_COLUMNS * DOT_ROWS, "a whole block fits at 1600");
        assert_eq!(
            blocks[1], 0,
            "the narrow pane drew {} dots rather than none",
            blocks[1]
        );
    }

    /// Every dot drawn in one space, lit or unlit, found by fill: a dot is a few
    /// pixels square, which no size filter can tell from a hairline.
    fn dots_in(out: &Rendered, space: Space) -> Vec<[f32; 4]> {
        let body = out.layout.placed(space).body;
        let hues: Vec<[f32; 4]> = out
            .skin
            .gauges
            .iter()
            .chain(out.skin.gauges_unlit.iter())
            .copied()
            .collect();
        out.scene
            .rects
            .iter()
            .filter(|r| hues.contains(&r.rgba()) && body.contains(r.xywh()[0], r.xywh()[1]))
            .map(|r| r.xywh())
            .collect()
    }

    /// A pane with no room for a legible block draws none of it, and says so by
    /// drawing the reading where an unbounded one goes. The alternative is twenty
    /// dots two pixels wide, which is a texture rather than a level, in the room
    /// the number needed to be read at all.
    ///
    /// Every reading is still on the pane either way. Losing the block is not
    /// losing the number.
    #[test]
    fn a_pane_too_narrow_for_a_block_draws_no_block_and_keeps_the_numbers() {
        let state = busy_state();
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        monitor.sample(&state);
        let mut dock = Dock::new();
        dock.reveal(View::Context);

        // Wide enough for a block, and narrow enough that a dot would be under
        // SMALL_DOT across. 680 is the window's own minimum size.
        let wide = render_with(&state, 1400.0, 900.0, &dock, &[], &monitor, None);
        assert_eq!(
            dots_in(&wide, Space::TopRight).len(),
            DOT_COLUMNS * DOT_ROWS,
            "one whole block at 1400"
        );
        for [_, _, w, _] in dots_in(&wide, Space::TopRight) {
            assert!(w >= SMALL_DOT, "a {w} pixel dot is a smear");
        }

        let narrow = render_with(&state, 680.0, 500.0, &dock, &[], &monitor, None);
        assert!(
            dots_in(&narrow, Space::TopRight).is_empty(),
            "a block was drawn in a pane with no room for one"
        );
        let text = text_of(&narrow.scene);
        for label in ["CONTEXT", "TOTAL REQUESTS", "LAST PREFILL"] {
            assert!(text.contains(label), "{label} left the narrow pane: {text}");
        }
        assert!(text.contains("1,816 / 65,536"), "the fill still reads: {text}");
    }

    /// The session monitor carries what the title strip lost: which phase, which
    /// model, which workspace. Labelled, which they never were up there.
    #[test]
    fn the_session_monitor_carries_what_the_title_strip_lost() {
        let state = busy_state();
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        let mut dock = Dock::new();
        dock.reveal(View::Context);
        let out = render_with(&state, 1400.0, 900.0, &dock, &[], &monitor, None);
        let text = text_of(&out.scene);
        for wanted in [
            "PHASE",
            "MODEL",
            "PATH",
            state.model.as_str(),
            &short_path(&state.workspace),
            "CONTEXT",
        ] {
            assert!(text.contains(wanted), "{wanted:?} is not in the pane: {text}");
        }
        // And the reading above the header is a row of its own, not a line of
        // the title strip: the phase word is drawn in the pane, not up there.
        let body = out.layout.placed(Space::TopRight).body;
        assert!(
            out.scene.texts.iter().any(|t| {
                body.contains(t.at.x, t.at.y)
                    && t.runs.iter().any(|r| r.text.contains(state.phase.word()))
            }),
            "the phase is not drawn in the session pane"
        );
    }

    /// One row per line and one line per row, because a click in this pane is
    /// turned into a row by dividing by the line height. A row that wrapped
    /// would open a different failure than the one under the pointer.
    ///
    /// The rows are one text box now that the pane scrolls, rather than one box
    /// each, so what this reads is the lines of that box: every one of them is at
    /// most as wide as the pane, and the box steps a line at a time, which is what
    /// the click arithmetic divides by.
    #[test]
    fn every_row_of_the_debug_pane_is_one_line_of_it() {
        let mut state = busy_state();
        state.apply(noob_proto::Event::ToolStart {
            call_id: "z".into(),
            name: "write".into(),
            brief: "write it".into(),
            args: serde_json::json!({"path": "x".repeat(400), "content": "y"}),
        });
        state.apply(noob_proto::Event::ToolEnd {
            call_id: "z".into(),
            summary: "refused".into(),
            elapsed_ms: 4,
            error: Some(noob_proto::ToolError {
                kind: "denied".into(),
                code: None,
                message: "outside the workspace".into(),
                detail: None,
                remedy: None,
            }),
        });
        state.open_failure = Some(0);

        let mut dock = Dock::new();
        dock.reveal(View::Debug);
        let out = render_with(&state, 1400.0, 900.0, &dock, &[], &Monitor::new(), None);
        let body = out.layout.placed(Space::BottomRight).body;
        let line = Text::line_for(13.0);
        let cols = cols_of(body, 8.0);
        let box_ = out
            .scene
            .texts
            .iter()
            .find(|t| body.contains(t.at.x, t.at.y))
            .expect("the pane wrote its rows");
        assert_eq!(box_.line_height, line, "the rows do not step by one line");
        let written: String = box_.runs.iter().map(|r| r.text.as_str()).collect();
        let rows: Vec<&str> = written.lines().collect();
        assert_eq!(rows.len(), state.debug_rows().len(), "{rows:?}");
        for (index, row) in rows.iter().enumerate() {
            assert!(
                row.chars().count() <= cols,
                "row {index} is {} columns wide in a pane {cols} wide",
                row.chars().count()
            );
        }
        // The long argument was cut rather than wrapped, and it says so.
        assert!(written.contains("outside the workspace"), "{written}");
        assert!(written.contains('\u{2026}'), "the long argument was not clipped");
    }

    /// A window whose every list is longer than any pane can hold: forty todos,
    /// twelve children with news each, and thirty failed calls.
    fn crowded_state() -> State {
        let mut state = busy_state();
        let todos: Vec<serde_json::Value> = (0..40)
            .map(|i| serde_json::json!({"content": format!("step {i:02}"), "status": "pending"}))
            .collect();
        state.apply(noob_proto::Event::ToolStart {
            call_id: "plan-2".into(),
            name: "plan".into(),
            brief: "40 items".into(),
            args: serde_json::json!({"todos": todos}),
        });
        for i in 0..12 {
            state.apply(noob_proto::Event::AgentSpawn {
                agent_id: format!("kid-{i:02}"),
                prompt: format!("child {i:02} is reading"),
                tools: "read".into(),
            });
            state.apply(noob_proto::Event::AgentOutput {
                agent_id: format!("kid-{i:02}"),
                line: format!("news {i:02}"),
            });
        }
        for i in 0..30 {
            let id = format!("bad-{i:02}");
            state.apply(noob_proto::Event::ToolStart {
                call_id: id.clone(),
                name: "bash".into(),
                brief: format!("call {i:02}"),
                args: serde_json::json!({"cmd": "no"}),
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
        state
    }

    /// A monitor that has read this state twice, which is what the two token
    /// panes need before they report a rate.
    fn sampled(state: &State) -> Monitor {
        let mut monitor = Monitor::new();
        monitor.sample(state);
        monitor.sample(state);
        monitor
    }

    /// Where a view is showing and how tall its content is there, taken from the
    /// pane's own extent so a test drives the arithmetic the wheel drives.
    fn measured(
        state: &State,
        w: f32,
        h: f32,
        dock: &Dock,
        monitor: &Monitor,
        view: View,
    ) -> (Space, Vec<usize>, usize) {
        let space = Space::ALL
            .into_iter()
            .find(|space| dock.slot(*space).active() == Some(view))
            .expect("the view is showing somewhere");
        let shape = shape(dock, &[]);
        let layout = Layout::compute(w, h, &shape);
        let skin = Skin::from(&Config::default());
        let frame = Frame {
            state,
            monitor,
            dock,
            skin: &skin,
            layout: &layout,
            prompt: &crate::prompt::Prompt::default(),
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        };
        let (heights, rows) = scroll_extent(&frame, view, layout.placed(space).body)
            .expect("the view reports an extent");
        (space, heights, rows)
    }

    /// The scrollbar drawn in one space: its track and its thumb, or nothing when
    /// the pane's content fits and it drew no bar.
    fn bar_in(out: &Rendered, space: Space) -> Option<([f32; 4], [f32; 4])> {
        let body = out.layout.placed(space).body;
        let of = |want: [f32; 4]| {
            out.scene
                .rects
                .iter()
                .find(|r| r.rgba() == want && body.contains(r.xywh()[0], r.xywh()[1]))
                .map(|r| r.xywh())
        };
        Some((of(out.skin.scroll_track)?, of(out.skin.scroll_thumb)?))
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
    /// itself. One mechanism for four panes, so this drives all four.
    ///
    /// Every one of them used to lose what did not fit: PLAN and AGENTS drew past
    /// the bottom edge of the pane, DEBUG stopped at the last row that fitted, and
    /// a monitor stopped at the last reading that fitted. None of them had a bar,
    /// so nothing on screen even said there was more.
    #[test]
    fn a_pane_with_more_rows_than_its_box_scrolls_to_the_end() {
        for (view, w, h, last) in [
            (View::Plan, 1400.0, 900.0, "step 39"),
            (View::Agents, 1400.0, 900.0, "news 11"),
            (View::Debug, 1400.0, 900.0, "boom 29"),
            // The monitor pane is five readings in a box that holds three.
            (View::Context, 900.0, 520.0, "LAST GENERATED"),
        ] {
            let mut state = crowded_state();
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
                state.scrolls.scroll(view, 9_999, true, &heights, rows),
                "{view:?} would not scroll"
            );
            assert!(
                !state.scrolls.scroll(view, 1, true, &heights, rows),
                "{view:?} scrolled past its own end"
            );
            let end = render_with(&state, w, h, &dock, &[], &monitor, None);
            let written = written_in(&end, space);
            assert!(written.contains(last), "{view:?} cannot reach {last}: {written}");
            let (track, thumb) = bar_in(&end, space).expect("still a bar");
            assert!(
                (thumb[1] + thumb[3] - track[1] - track[3]).abs() < 1.5,
                "{view:?}: the thumb is not at the foot of its track: {thumb:?} in {track:?}"
            );

            // And back to the top, where it started.
            assert!(state.scrolls.scroll(view, 9_999, false, &heights, rows));
            assert_eq!(state.scrolls.first(view), 0, "{view:?}");
        }
    }

    /// A pane holding less than it can show draws no bar. A bar that is always
    /// there and always full says nothing, which is why `scrollbar` takes an
    /// option.
    #[test]
    fn a_pane_whose_content_fits_draws_no_bar() {
        let state = busy_state();
        let monitor = sampled(&state);
        for view in [View::Plan, View::Agents, View::Debug, View::Session] {
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
        let monitor = sampled(&state);
        let mut dock = Dock::new();
        dock.reveal(View::Plan);
        let (space, heights, rows) = measured(&state, 1400.0, 900.0, &dock, &monitor, View::Plan);
        state.scrolls.scroll(View::Plan, 9_999, true, &heights, rows);
        let scrolled = state.scrolls.first(View::Plan);
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
        let out = render_with(&state, 1400.0, 900.0, &dock, &[], &monitor, None);
        let written = written_in(&out, space);
        for wanted in ["late 00", "late 01", "late 02"] {
            assert!(
                written.contains(wanted),
                "the pane is blank at row {scrolled} of three: {written:?}"
            );
        }

        let (_, short, rows) = measured(&state, 1400.0, 900.0, &dock, &monitor, View::Plan);
        assert!(
            state.scrolls.settle(View::Plan, &short, rows),
            "the offset was left past the end"
        );
        assert_eq!(state.scrolls.first(View::Plan), 0);
        let after = render_with(&state, 1400.0, 900.0, &dock, &[], &monitor, None);
        assert!(
            bar_in(&after, space).is_none(),
            "three todos in eighteen rows still drew a bar"
        );
    }

    /// A frame that is nothing but a prompt: the strip it landed in, its
    /// layout, and the scene, at the default 14pt body size.
    fn render_prompt(
        prompt: &crate::prompt::Prompt,
        max_rows: usize,
    ) -> (Panel, Layout, Scene) {
        let state = State::new();
        let dock = Dock::new();
        let skin = Skin::from(&Config::default());
        let mut shape = shape(&dock, &[]);
        shape.input_h = input_height(1200.0, 8.0, prompt.len(), Text::line_for(14.0), max_rows);
        let layout = Layout::compute(1200.0, 800.0, &shape);
        let scene = build(&Frame {
            state: &state,
            monitor: &Monitor::new(),
            dock: &dock,
            skin: &skin,
            layout: &layout,
            prompt,
            column: 8.0,
            pane_column: 8.0,
            body_size: 14.0,
            pane_size: 13.0,
            clock: 0.0,
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        });
        (layout.input, layout, scene)
    }

    /// The prompt grows with what has been typed, and the caret follows the
    /// wrap rather than running off the end of the first line.
    #[test]
    fn the_prompt_grows_and_the_caret_stays_inside_it() {
        let line = Text::line_for(14.0);
        let short = typed_prompt("short", 5);
        let long = typed_prompt(&"x".repeat(600), 600);
        let (one, ..) = render_prompt(&short, 8);
        let (many, _, scene) = render_prompt(&long, 8);
        assert!(many.h > one.h, "the prompt grew: {} then {}", one.h, many.h);
        assert!(many.h <= 8.0 * line + 30.0, "and stopped growing: {}", many.h);
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

    /// How tall it is allowed to get is a setting, not a constant. Two rows
    /// and twenty rows are both a window somebody wants.
    #[test]
    fn the_prompt_stops_growing_at_the_configured_row_count() {
        let line = Text::line_for(14.0);
        // More than twenty rows of it, so the ceiling is what stops it.
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
        // A ceiling nobody typed up to still leaves the prompt one row.
        let (empty, ..) = render_prompt(&crate::prompt::Prompt::default(), 20);
        assert!((empty.h - (line + 2.0 * INPUT_PAD)).abs() < 0.01, "{}", empty.h);
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
                layout.input_caret(x, y, 14.0, 8.0, prompt.len()),
                at,
                "row {row} column {column}"
            );
        }
        // Past the end of the text, and past the end of a row.
        let below = box_.y + box_.h - 1.0;
        assert_eq!(
            layout.input_caret(box_.x + box_.w - 1.0, below, 14.0, 8.0, prompt.len()),
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
            layout.input_caret(placed[0] + 1.0, placed[1] + 1.0, 14.0, 8.0, moved.len()),
            columns + 3
        );
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

    /// Two dark panels side by side over a busy desktop read as one region
    /// with a gap in it. The border is what tells them apart.
    ///
    /// Item 12: a pane gets three sides of that border and no top. This asserted
    /// one stroked rectangle around the whole pane, which is what drew the line
    /// under every tab strip. The prompt and the picker still take the stroke, so
    /// `panel_edge` is still the way a box that wants four sides is drawn.
    #[test]
    fn a_pane_is_bordered_on_three_sides_and_open_at_the_top() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &["a.rs"]);
        for space in Space::ALL {
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
        assert_eq!(checked, 3, "only {checked} spaces had a showing tab");
    }

    /// The cut corner, on the fill and on the border alike. A square fill under
    /// a cut border leaves a triangle of panel colour outside its own edge.
    ///
    /// A pane is a fill on its own since item 12 took its top edge away, and the
    /// prompt is still a fill plus a stroke, so the count is per box rather than
    /// two everywhere.
    #[test]
    fn a_panel_is_cut_on_its_top_right_corner_only() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &["a.rs"]);
        let boxes: Vec<(Panel, usize)> = Space::ALL
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
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        })
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
        assert!(text.contains("WORKING") || text.contains("THINKING"), "{text}");
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
        let mut dock = Dock::new();
        // Every view but one in the left space, which is more than its strip can
        // hold. The one left behind keeps the space split, so the strip is the
        // width it usually is rather than the whole window.
        for view in View::ALL.into_iter().filter(|v| *v != View::Files) {
            dock.move_view(view, Space::Left);
        }
        let out = render(&busy_state(), 900.0, 700.0, &dock, &["calc.py"]);
        let placed = out.layout.placed(Space::Left);
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
        let mut dock = Dock::new();
        let all = dock.slot(Space::TopRight).views.len();
        let roomy = Layout::compute(1400.0, 900.0, &shape(&dock, &[]));
        let placed = roomy.placed(Space::TopRight);
        assert_eq!(placed.tabs.len(), all, "six tabs fit in a wide window");
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
        for view in [View::Files, View::Debug] {
            assert!(empty.hide(view));
        }
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
        let dock = Dock::new();
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
        let mut dock = Dock::new();
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
        let layout = with_menu(dock, menu, w, h);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state,
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
            drag: None,
            hot,
            trouble: None,
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

    fn middle(panel: Panel) -> (f32, f32) {
        (panel.x + panel.w * 0.5, panel.y + panel.h * 0.5)
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

    /// The row the pointer is over is the row that acts, and a greyed one acts
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
        // The copy row is the greyed one now that the settings panel exists, and
        // it keeps its place: the rows either side of it act as before.
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
        // And with the list open, every row of it resolves to the widget drawn
        // there, top level rows included.
        let mut open = Menu::for_widget(at, View::Plan, Space::TopRight, false);
        open.toggle_widgets(&dock);
        let mut want = vec![
            Some(Item::Settings),
            None,
            Some(Item::Close),
            Some(Item::Widgets(true)),
        ];
        want.extend(
            View::ALL
                .into_iter()
                .map(|view| Some(Item::Widget(view, false))),
        );
        assert_eq!(picked(&open), want);
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

    /// The widget list has the same problem the menu has, one step further on:
    /// nine more rows opened near the bottom of a short window would hang off
    /// the surface, and a row down there cannot be picked at all.
    #[test]
    fn the_widget_list_is_clamped_into_the_window_and_scrolls_when_it_cannot_fit() {
        let dock = Dock::new();
        let (w, h) = (900.0, 600.0);
        let mut menu = Menu::for_widget((w - 2.0, h - 2.0), View::Plan, Space::Left, false);
        menu.toggle_widgets(&dock);
        let layout = with_menu(&dock, &menu, w, h);
        let box_ = layout.menu;
        assert!(box_.y >= 0.0 && box_.y + box_.h <= h + 0.01, "{box_:?}");
        assert_eq!(
            layout.menu_rows.len(),
            menu.rows.len(),
            "the whole list fits in a window this tall, so all of it is placed"
        );
        for (index, row) in &layout.menu_rows {
            let (x, y) = middle(*row);
            assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(*index)));
        }

        // A window too short for thirteen rows keeps the top level and gives
        // what is left to the list, which then has to move to reach the rest.
        let short = 220.0;
        let layout = with_menu(&dock, &menu, w, short);
        let placed: Vec<usize> = layout.menu_rows.iter().map(|(index, _)| *index).collect();
        assert!(
            placed.len() < menu.rows.len(),
            "this window is not short enough to prove anything"
        );
        assert_eq!(&placed[..menu.top], &[0, 1, 2, 3], "the top level is kept");
        assert!(layout.menu.y >= 0.0);
        assert!(layout.menu.y + layout.menu.h <= short + 0.01);
        let capacity = layout.menu_capacity(&menu);
        assert_eq!(capacity, placed.len() - menu.top);

        // Scrolled to the end, the last widget is on screen and the first is
        // not, and no row has left the window.
        menu.scroll(View::ALL.len(), true, capacity);
        let layout = with_menu(&dock, &menu, w, short);
        let placed: Vec<usize> = layout.menu_rows.iter().map(|(index, _)| *index).collect();
        assert_eq!(&placed[..menu.top], &[0, 1, 2, 3]);
        assert_eq!(
            placed.last().copied(),
            Some(menu.top + View::ALL.len() - 1),
            "the last widget is reachable"
        );
        for (index, row) in &layout.menu_rows {
            let (x, y) = middle(*row);
            assert!(row.y >= 0.0 && row.y + row.h <= short + 0.01, "{row:?}");
            assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(*index)));
            if *index >= menu.top {
                assert!(
                    matches!(menu.pick(*index), Some(crate::menu::Item::Widget(..))),
                    "row {index} is on screen and does nothing"
                );
            }
        }
    }

    /// The list is part of the menu, so it is painted where the menu is: on the
    /// floating layer, above the pane text it covers. In the base layer it
    /// would be nine tab names drawn under the menu's own box.
    #[test]
    fn the_widget_list_is_drawn_on_the_floating_layer() {
        let dock = Dock::hiding(&[View::Debug]);
        let mut menu = Menu::for_widget((400.0, 200.0), View::Plan, Space::Left, false);
        menu.toggle_widgets(&dock);
        let out = render_menu(&busy_state(), 1400.0, 900.0, &dock, &menu, None);
        let box_ = out.layout.menu;
        let runs: Vec<String> = out
            .scene
            .over_texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.text.clone()))
            .collect();
        for view in View::ALL {
            assert!(
                runs.iter().any(|text| text.contains(view.label())),
                "{} is not on the overlay: {runs:?}",
                view.label()
            );
        }
        // Marked in the gutter: a check for the eight in the window, the close
        // mark for the one that is not.
        let marks = runs.iter().filter(|text| *text == &icons::CLOSE.to_string());
        assert_eq!(marks.count(), 1, "only DEBUG is closed");
        assert_eq!(
            runs.iter()
                .filter(|text| *text == &icons::CONFIRM.to_string())
                .count(),
            View::ALL.len() - 1
        );
        // And all of it inside the menu's own box, like every other row.
        for text in &out.scene.over_texts {
            assert!(
                text.at.y >= box_.y - 0.01 && text.at.y + text.at.h <= box_.y + box_.h + 0.01,
                "{:?} is outside the menu",
                text.at
            );
        }
    }

    /// Whether any text in this list has a glyph box overlapping the panel.
    fn text_over(texts: &[Text], panel: Panel) -> bool {
        texts.iter().any(|text| {
            text.at.x < panel.x + panel.w
                && panel.x < text.at.x + text.at.w
                && text.at.y < panel.y + panel.h
                && panel.y < text.at.y + text.at.h
        })
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

        // The rows are legible, and a row that cannot act says so by weight.
        // Read off the overlay: a label still in the base layer would be drawn
        // under the menu's own box.
        let runs: Vec<(&str, Option<[u8; 4]>)> = out
            .scene
            .over_texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| (r.text.as_str(), r.color)))
            .collect();
        let base = text_of(&out.scene);
        for (label, tint) in [
            ("Settings", out.skin.bright),
            ("Copy selection", out.skin.dim),
            ("Close this widget", out.skin.bright),
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
            drag: None,
            hot: None,
            trouble: None,
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
        // The body of a pane names the space and no place in its strip.
        let (x, y) = middle(layout.placed(Space::Left).body);
        assert_eq!(layout.landing(x, y), Landing::In(Space::Left, None));
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
        // Both tabs of the bottom space: the debug pane opens beside the files.
        assert!(dock.hide(View::Files));
        assert!(dock.hide(View::Debug));
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
            out.layout.placed(Space::Left).body,
            full.placed(Space::Left).body
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
            drag: None,
            hot,
            trouble: None,
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
            "OPEN A FOLDER",
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

        // The row the cursor is on is a filled green band with the dark ink
        // written over it. Item 4: the quiet band the file explorer marks its
        // open row with said almost nothing here.
        let (index, cursor_row) = layout.picker_rows[0];
        assert_eq!(index, picker.cursor());
        assert!(
            covered(&out, cursor_row, cursor_row.h, out.skin.picked),
            "the cursor's row has no band"
        );
        // And no other row is banded, or every row would read as the one.
        let banded = out
            .scene
            .rects
            .iter()
            .filter(|rect| rect.rgba() == out.skin.picked)
            .count();
        assert_eq!(banded, 1, "more than one row is banded");
        // Everything written on that band is the dark ink. Green text on a
        // green band is the one thing the whole palette is built to avoid.
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

    /// The button says one word, carries the cut corner every panel in this
    /// window carries, sits on a surface of its own and lights up under the
    /// pointer. It is the only thing in the picker a mouse can press that is not
    /// a row, and before this round it was drawn in the quietest fill in the
    /// palette with a hairline round it, which read as a label.
    #[test]
    fn the_open_button_says_one_word_and_reads_as_a_button() {
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

        // It says "Open" and nothing else. The folder it would open is written
        // above the list, and spelling it out here made the button as wide as a
        // path and a different width every time the cursor moved.
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

    /// Item 2: a window that has just opened has to offer the sessions that came
    /// before it, not only a fresh one. A second button beside Open swaps the
    /// list, and everything else about the box stays where it was.
    #[test]
    fn the_button_beside_open_swaps_the_list_for_the_saved_sessions() {
        let mut picker = a_picker(&["gui"], &[]);
        let cold = render_picker(&picker, 1205.0, 791.0, None);
        let warm = render_picker(&picker, 1205.0, 791.0, Some(Hit::PickerSessions));
        let (open, button) = (cold.layout.picker_open, cold.layout.picker_sessions);

        // Beside Open, on the same line, inside the box, and its own target.
        assert!(button.x >= open.x + open.w, "{open:?} then {button:?}");
        assert!((button.y - open.y).abs() < 0.01 && (button.h - open.h).abs() < 0.01);
        assert!(button.x + button.w <= cold.layout.picker.x + cold.layout.picker.w + 0.01);
        let (x, y) = middle(button);
        assert_eq!(cold.layout.hit(x, y), Some(Hit::PickerSessions));
        assert_eq!(
            cold.layout.hit(middle(open).0, middle(open).1),
            Some(Hit::PickerOpen),
            "and the two do not overlap"
        );

        // The same surface Open sits on, and it lights up under the pointer.
        assert!(covered(&cold, button, button.h, cold.skin.button));
        assert!(covered(&warm, button, button.h, warm.skin.button_hot));
        assert!(
            covered(&warm, open, open.h, warm.skin.button),
            "the pointer on one button must not light the other"
        );

        let text = text_of(&cold.scene);
        assert!(text.contains("OPEN A FOLDER"));
        assert!(text.contains(PICKER_SESSIONS_LABEL), "{text}");

        // Pressed, the same box lists the sessions instead: same rectangle,
        // same rows, same button, and the word on it now says the way back.
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
            (after.layout.picker, after.layout.picker_open, after.layout.picker_sessions),
            (cold.layout.picker, open, button),
            "swapping the list moved the box"
        );
        assert_eq!(after.layout.picker_rows.len(), 2);
        let text = text_of(&after.scene);
        for wanted in [
            "OPEN A SESSION",
            "2 saved sessions",
            "10m ago",
            "carry this on",
            "deleted (gone)",
            // Still written above the list, because it is the folder a session
            // that never noted one would be resumed in.
            "/home/hec",
            PICKER_FOLDERS_LABEL,
        ] {
            assert!(text.contains(wanted), "{wanted:?} is not on screen: {text}");
        }
        assert!(
            !text.contains(PICKER_SESSIONS_LABEL),
            "the button still offers the list that is already showing"
        );

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

        // And it goes with the picker. A button left behind by a shape change
        // is a press that lands on something nobody can see.
        let dock = Dock::new();
        let panel = a_settings_panel(&Config::default());
        for (what, shape) in [
            ("shaded", Shape { shaded: true, ..shape(&dock, &[]) }),
            ("settings", Shape { settings: Some(&panel), ..shape(&dock, &[]) }),
        ] {
            let layout = Layout::compute(1205.0, 791.0, &shape);
            assert_eq!(layout.picker_sessions.w, 0.0, "{what}");
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
        let (index, mark) = out.layout.picker_marks[0];
        let warm = render_picker(&picker, 1205.0, 791.0, Some(Hit::PickerMark(index)));
        let lit = |out: &Rendered, at: Panel| -> Vec<Option<[u8; 4]>> {
            out.scene
                .texts
                .iter()
                .filter(|text| (text.at.x - at.x).abs() < 0.01 && (text.at.y - at.y).abs() < 0.01)
                .flat_map(|text| text.runs.iter().map(|run| run.color))
                .collect()
        };
        assert_eq!(lit(&warm, mark), vec![Some(warm.skin.bright)]);
        assert_eq!(lit(&out, mark), vec![Some(out.skin.body)]);

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
        let glyph = |out: &Rendered, at: Panel| -> String {
            out.scene
                .texts
                .iter()
                .filter(|text| (text.at.x - at.x).abs() < 0.01 && (text.at.y - at.y).abs() < 0.01)
                .flat_map(|text| text.runs.iter().map(|run| run.text.as_str()))
                .collect()
        };
        assert_eq!(glyph(&out, mark), icons::EXPAND.to_string());
        let (_, reopened) = after
            .layout
            .picker_marks
            .iter()
            .find(|(at, _)| *at == index)
            .copied()
            .expect("the folder that was opened lost its mark");
        assert_eq!(glyph(&after, reopened), icons::COLLAPSE.to_string());

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
        let dock = Dock::new();
        let state = busy_state();
        let mut shape = shape(&dock, &["a.rs"]);
        shape.settings = Some(panel);
        let layout = Layout::compute(w, h, &shape);
        let skin = Skin::from(&Config::default());
        let scene = build(&Frame {
            state: &state,
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
            drag: None,
            hot,
            trouble: None,
            selection: None,
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
            &crate::totals::Totals::default(),
            Some(std::path::Path::new("/home/hec/.config/noob/no0b.conf")),
        )
    }

    /// The panel is a takeover: while it is up there are no panes, no tabs and
    /// no prompt, and it answers for every point under the title strip. The
    /// strip itself still works, so the window can be moved and closed from it.
    #[test]
    fn the_settings_panel_takes_the_whole_window() {
        let panel = a_settings_panel(&Config::default());
        let out = render_settings(&panel, 1205.0, 791.0, None);
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
        assert!(box_.y + box_.h <= 791.0 && box_.x + box_.w <= 1205.0, "{box_:?}");
        assert!(box_.w >= 1205.0 - 4.0 * GAP, "not a takeover: {box_:?}");
        assert!(box_.h >= 791.0 - TITLE_H - 4.0 * GAP, "not a takeover: {box_:?}");

        assert_eq!(
            layout.hit(box_.x + 1.0, box_.y + box_.h - 1.0),
            Some(Hit::Settings),
            "its own margin swallows a press rather than passing it on"
        );
        assert_eq!(layout.hit(400.0, 8.0), Some(Hit::TitleBar));
        let (x, y) = middle(layout.close);
        assert_eq!(layout.hit(x, y), Some(Hit::Close));
    }

    /// Every row is hit where it is drawn, the value at the end of a row that
    /// can change is its own region, and a row that cannot change has none.
    #[test]
    fn every_settings_row_lands_where_it_is_drawn() {
        let panel = a_settings_panel(&Config::default());
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        assert!(!layout.settings_rows.is_empty());
        for (index, row) in &layout.settings_rows {
            assert!(
                layout.settings_list.contains(row.x + 1.0, row.y + 1.0),
                "row {index} is outside the list: {row:?}"
            );
            // The left of the row, which is the label, puts the cursor there.
            assert_eq!(
                layout.hit(row.x + 2.0, row.y + row.h * 0.5),
                Some(Hit::SettingsRow(*index))
            );
            let control = matches!(
                panel.row(*index),
                Some(crate::settings::Row::Setting { kind, .. }) if kind.changes()
            );
            let value = layout
                .settings_values
                .iter()
                .find(|(at, _)| at == index)
                .map(|(_, panel)| *panel);
            match (control, value) {
                (true, Some(value)) => {
                    let (x, y) = middle(value);
                    assert_eq!(layout.hit(x, y), Some(Hit::SettingsValue(*index)));
                    assert!(row.contains(x, y), "the value is outside its row");
                }
                // A heading, a reading or a colour: the whole row is the row,
                // and a press on its right hand end changes nothing.
                (false, None) => assert_eq!(
                    layout.hit(row.x + row.w - 2.0, row.y + row.h * 0.5),
                    Some(Hit::SettingsRow(*index))
                ),
                other => panic!("row {index} carries {other:?}"),
            }
        }
        // The values are one column, which is what makes a screen of settings
        // scannable rather than a wall of words.
        let lefts: Vec<f32> = layout.settings_values.iter().map(|(_, p)| p.x).collect();
        assert!(
            lefts.windows(2).all(|pair| (pair[0] - pair[1]).abs() < 0.01),
            "{lefts:?}"
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
            // And it lights up under the pointer, so it reads as something to
            // press rather than as a decoration.
            let lit = render_settings(&panel, w, h, Some(Hit::SettingsClose));
            let hot = lit
                .scene
                .rects
                .iter()
                .any(|r| r.rgba() == lit.skin.close_hot && close.contains(r.xywh()[0] + 1.0, r.xywh()[1] + 1.0));
            assert!(hot, "the close mark does not light up at {w}x{h}");
        }
    }

    /// Nothing the panel draws leaves it, at any size. A rectangle outside a
    /// takeover is a rectangle over the desktop.
    #[test]
    fn nothing_the_settings_panel_draws_escapes_it() {
        let panel = a_settings_panel(&Config::default());
        for (w, h) in [(1400.0, 900.0), (680.0, 380.0), (2200.0, 1400.0)] {
            let out = render_settings(&panel, w, h, Some(Hit::SettingsValue(7)));
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

    /// What the panel says: its own heading, the all-time block, the keys of the
    /// settings it can change, and the palette drawn as itself.
    #[test]
    fn the_panel_says_what_it_is_and_draws_the_palette() {
        let config = Config::parse("accent = #123456");
        let panel = a_settings_panel(&config);
        let out = render_settings(&panel, 1400.0, 1200.0, None);
        let text = text_of(&out.scene);
        for wanted in [
            "SETTINGS",
            "ALL TIME",
            "prefilled",
            "theme",
            "opacity",
            "show_files",
            "COLOURS",
            "accent",
            "#123456",
        ] {
            assert!(text.contains(wanted), "{wanted:?} is not on the panel: {text}");
        }

        // The colour is drawn as a swatch of itself, not only as a hex string.
        let wanted = [0x12 as f32 / 255.0, 0x34 as f32 / 255.0, 0x56 as f32 / 255.0, 1.0];
        assert!(
            out.scene.rects.iter().any(|rect| rect.rgba() == wanted),
            "no swatch in the accent's own colour"
        );

        // The row the cursor is on carries the band and the mark every list in
        // this window marks its current row with.
        let row = out
            .layout
            .settings_rows
            .iter()
            .find(|(index, _)| *index == panel.cursor())
            .map(|(_, row)| *row)
            .expect("the cursor's row is on screen");
        assert!(
            out.scene
                .rects
                .iter()
                .any(|rect| rect.rgba() == out.skin.strip && rect.xywh() == [row.x, row.y, row.w, row.h]),
            "the cursor's row has no band"
        );
        assert!(
            out.scene
                .rects
                .iter()
                .any(|rect| rect.rgba() == out.skin.edge_focus
                    && rect.xywh() == [row.x, row.y, MARK_W, row.h]),
            "the cursor's row has no mark"
        );
    }

    /// The footer says what the keys will do to the row under the cursor, and
    /// says a refused write instead when there is one. A panel that writes a
    /// file has to say when the file said no.
    #[test]
    fn the_footer_carries_the_keys_and_then_the_trouble() {
        let config = Config::default();
        let mut panel = a_settings_panel(&config);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        assert!(text_of(&out.scene).contains(panel.hint()), "{}", panel.hint());

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
