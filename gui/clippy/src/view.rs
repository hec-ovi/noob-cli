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
use crate::icons;
use crate::menu::{MARKER_COLUMNS, Menu};
use crate::monitor::{Gauge, Monitor};
use crate::picker::{Picker, Row as PickerRow};
use crate::settings::{Row as SettingRow, Settings, Side};
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
/// phase, then the three counts that say what this run has asked for. They stay
/// put while the readings under them scroll.
const CONTEXT_HEAD: usize = 4;
/// The smallest a dot shrinks to, across or down, when a pane has more readings
/// than room. Below this the block stops reading as a block, so it is not drawn:
/// too tall for its rows and they scroll off, too narrow for its columns and the
/// pane draws numbers alone. A reading that scrolled off is honest and a number
/// with no block is honest; a smear is not.
const SMALL_DOT: f32 = 4.0;
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
/// The border the showing tab carries from where its accent stops: down the cut,
/// down the right edge and a short run along its foot. Half the accent, so weight
/// is still the whole difference between the tab that is showing and the rest,
/// and the border reads as the accent turning the corner rather than as a second
/// line shouting the same thing.
const TAB_EDGE_H: f32 = ACCENT_H * 0.5;
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
/// How far a flyout sits over the menu it came from: one pixel, so the two boxes
/// share the border line between them and read as attached to the row rather
/// than as a second menu that happens to be nearby.
///
/// One and no more. The floating layer paints every rectangle and then every
/// glyph, so a flyout box laid over a row of the menu would have that row's
/// writing show through it, and [`MENU_PAD`] of margin is all there is between
/// the menu's edge and its own labels.
const MENU_OVERLAP: f32 = 1.0;
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
/// How wide the folder picker gets, in pane columns.
///
/// One width for both of its lists, because the box must not move when the
/// button that swaps them is pressed. That makes this the width of the wider of
/// the two, which is the session table: five columns of content, four of them a
/// fixed size ([`SESSION_COLUMNS`]) and the last one holding what was first said
/// in the session. Sixty-four columns fitted the folder list alone and left the
/// opening line four words wide.
const PICKER_COLUMNS: usize = 96;
/// Where the dividers sit on a window nobody has dragged one in: a column takes
/// this much of the width, and a top space this much of the height.
///
/// One number each rather than one per half, because a window opened for the
/// first time has both halves of the grid breaking in the same place. What makes
/// them free to differ is that they are dragged apart afterwards.
///
/// Defaults, not constants. All of them are dragged, all of them are carried in
/// on [`Shape`], and the settings file remembers where they were left.
pub const LEFT_WIDTH: f32 = 0.54;
pub const TOP_HEIGHT: f32 = 0.46;
/// The same for the settings panel's rail: how much of the panel the column of
/// section names takes before anyone drags it.
///
/// A tenth, which is about fourteen columns of pane text on the window this is
/// usually opened in and is held up to the longest section name on a narrower
/// one.
pub const SETTINGS_RAIL: f32 = 0.10;
/// How far either side of the gap between two panes the pointer still counts as
/// being on the divider between them.
///
/// The gap is [`GAP`], six pixels, which is a line you can see and not a target
/// you can hit. This takes the target to fourteen without widening anything that
/// is drawn.
const GRAB: f32 = 4.0;
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

/// The rows the picker spends above its list on plain writing: the heading and
/// the folder it is listing. What has been typed sits under them in a bordered
/// field of its own, which is [`picker_field_h`] rather than one row.
const PICKER_HEAD_ROWS: f32 = 2.0;

/// How much taller the search field is than the line of text in it, on each
/// side.
///
/// The field carries the same cut corner and the same hairline every panel in
/// this window carries, and a box drawn tight around a line of text reads as a
/// line of text with a box round it rather than as something to type in.
const PICKER_FIELD_PAD: f32 = 4.0;

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

/// The session table's fixed columns: what each one is called at the top of the
/// list, and how many characters wide it is.
///
/// The last cell of a row, the opening line, is not here: it takes whatever is
/// left of the row, which is what makes the box's width worth having. Each cell
/// is written into one less column than it is given, so two full cells still
/// have a space between them and the table reads as columns rather than as one
/// run of words.
///
/// One row of a session list used to be a single string, `"5m ago  hec  first"`,
/// and item A7 is exactly the complaint that nobody could tell which part of it
/// was what.
const SESSION_COLUMNS: [(&str, usize); 4] = [
    ("when", 10),
    ("folder", 18),
    ("size", 10),
    ("context", 9),
];
/// What the last column is called. It has no width of its own.
const SESSION_OPENING: &str = "opening";

/// Where a session row's table starts and how many columns it has, for a row
/// panel `row` wide.
///
/// One answer for the header above the list and for every row in it, so the two
/// cannot come apart. In from the left by the same indent a folder row's mark
/// takes plus the row's own glyph and the space after it, because a session row
/// carries that glyph too and the table begins after it.
fn session_table(row: Panel, column: f32) -> (f32, usize) {
    let column = column.max(1.0);
    let x = row.x + PICKER_ROW_PAD + (PICKER_MARK_COLUMNS + ROW_ICON_COLUMNS + 1) as f32 * column;
    let room = ((row.x + row.w - x) / column).floor().max(0.0) as usize;
    (x, room)
}

/// One line of the session table: each cell written into its own columns, in
/// `room` columns altogether.
///
/// Space padded rather than drawn cell by cell. The list is monospace, so a
/// padded string is a table, and one shaped run per row is one run to tint: a
/// row of five texts on the cursor's green band would be five chances for one of
/// them to be tinted wrong.
fn session_line(cells: &[String], room: usize) -> String {
    let mut out = String::new();
    let mut left = room;
    for (step, cell) in cells.iter().enumerate() {
        if left == 0 {
            break;
        }
        let width = match SESSION_COLUMNS.get(step) {
            Some((_, wide)) => (*wide).min(left),
            // The last cell takes the rest of the row.
            None => left,
        };
        // Two columns short of its own, so a cell that fills its column still
        // has a space after it, and hard capped at the column either way: in a
        // window too narrow for a column the ellipsis would be the character
        // that ran over.
        let text = clip(cell, width.saturating_sub(2).max(1));
        let text: String = text.chars().take(width).collect();
        let written = text.chars().count();
        out.push_str(&text);
        // Not after the last cell: trailing spaces are columns nobody sees, and
        // they would push a clipped opening line past the end of the row.
        if step + 1 < cells.len() {
            for _ in written..width {
                out.push(' ');
            }
        }
        left -= width;
    }
    out
}

/// What the picker calls itself, in both of its lists.
///
/// One string rather than a heading that swapped between OPEN A FOLDER and OPEN
/// A SESSION. The title says what the box is for; which of the two lists is in
/// front of you is said by the pair of buttons under it, and a title that also
/// said it was a second thing to read for an answer already on screen.
const PICKER_TITLE: &str = "OPEN FOLDER OR CONTINUE SESSION";

/// What the picker says on the button that opens the row the cursor is on.
///
/// It used to spell out the folder that would be opened, which made the button
/// as wide as a path and made it change width every time the cursor moved. The
/// path is already written above the list. "selected" rather than the folder or
/// the session, because one button opens whichever of the two the cursor is on.
const PICKER_OPEN_LABEL: &str = "Open selected";

/// The two buttons that choose which list is showing.
///
/// Both are drawn in a box sized for the longer of the two, so the pair does not
/// shuffle sideways when the list swaps, and the one whose list is showing is
/// filled in the colour the chosen row is filled in.
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

/// How tall the search field is, for the same line height.
fn picker_field_h(line: f32) -> f32 {
    line + PICKER_FIELD_PAD * 2.0
}

/// What the picker keeps above its list: the row of buttons, a gap,
/// [`PICKER_HEAD_ROWS`] of writing, the search field, and a gap between that
/// field and the first row.
///
/// One answer, the way [`picker_foot`] is one answer for the bottom, and it does
/// not read the picker: the head is the same height on the folder list and on
/// the session list, so swapping between the two cannot move the box.
///
/// The buttons moved up here from the foot and the foot kept the line of keys,
/// so this is the same total as before and the list holds the same number of
/// rows it always did.
fn picker_head_h(line: f32) -> f32 {
    picker_open_h(line) + GAP + PICKER_HEAD_ROWS * line + picker_field_h(line) + GAP
}

/// What the picker keeps below its list: the line of keys. One answer, so the
/// box that is measured and the rows that are drawn into it cannot disagree
/// about where the bottom is.
fn picker_foot(line: f32) -> f32 {
    line
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
    /// The box of the activity popup. The whole of it: nothing inside it acts,
    /// so there is one region and it swallows the press, which is what lets a
    /// press anywhere else close the popup without also doing whatever it landed
    /// on. The same bargain the menu makes.
    CallPopup,
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
    /// One cell of the palette grid, as the row it is on and the column along
    /// it. A grid row is several controls wide, so the row on its own cannot say
    /// which colour the pointer is over.
    SettingsSwatch(usize, usize),
    /// The toggle on an entry row, by the row it is on. Pressing it turns that
    /// skill or that server on or off, which is a move on the disk: nothing in
    /// this window remembers a flag for either.
    SettingsToggle(usize),
    /// The uninstall beside that toggle, on the rows that have one. Its own
    /// region and tested before the row, the way the toggle is: one region for
    /// the row and the button would delete a skill or a server every time
    /// somebody pressed the row to read it.
    SettingsRemove(usize),
    /// The line between the rail of section names and the settings beside it.
    /// Dragging it decides how much of the panel each of the two takes.
    ///
    /// Its own hit rather than a [`Hit::ColumnDivider`]: the panel is a takeover,
    /// so while it is up there are no panes and no grid for a column divider to
    /// mean anything about.
    SettingsRailDivider,
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
    /// The two controls an entry row carries: the toggle that turns it on and
    /// off, and the uninstall on the rows that have one. Empty on every section
    /// that lists no entries, which is every section but the skills and the
    /// servers.
    pub settings_toggles: Vec<(usize, Panel)>,
    pub settings_removes: Vec<(usize, Panel)>,
    /// The column beside that list, where the entry under the cursor is shown:
    /// a skill's own `SKILL.md`, or a server's entry out of its file. Empty in
    /// every section that has no entries, which is what leaves those sections
    /// one column wide.
    pub settings_doc: Panel,
    pub settings_close: Panel,

    /// The floating layer. The open menu's box, and one panel per row on
    /// screen, both empty when no menu is open. Drawn last and hit tested
    /// first.
    ///
    /// Each row carries its place in the menu, the way the picker's and the
    /// settings panel's rows do, because the widget list scrolls: the third
    /// panel down is not always the third row.
    pub menu: Panel,
    pub menu_rows: Vec<(usize, Panel)>,
    /// The widget list's own box beside that menu, and its rows, both empty
    /// while the list is shut. Above the menu rather than beside it as far as
    /// hit testing goes: it is drawn over the menu, so it takes the click.
    pub menu_list: Panel,
    pub menu_list_rows: Vec<(usize, Panel)>,
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

fn nowhere() -> Panel {
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
            Some(menu) => place_menu(menu, shape.column, width, height),
            None => MenuPlaces {
                box_: nowhere(),
                rows: Vec::new(),
                list: nowhere(),
                list_rows: Vec::new(),
            },
        };
        let (menu, menu_rows) = (places.box_, places.rows);
        let (menu_list, menu_list_rows) = (places.list, places.list_rows);
        // Only in the shape that has panes. The three takeovers below collapse
        // it along with every other pane region.
        let call_popup = match shape.popup {
            Some(call) => place_popup(call, shape.pane_column, shape.pane_size, width, height),
            None => nowhere(),
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
                settings_toggles: Vec::new(),
                settings_removes: Vec::new(),
                settings_doc: nowhere(),
                settings_close: nowhere(),
                menu,
                menu_rows,
                menu_list,
                menu_list_rows,
                call_popup: nowhere(),
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
                settings_toggles: Vec::new(),
                settings_removes: Vec::new(),
                settings_doc: nowhere(),
                settings_close: nowhere(),
                menu,
                menu_rows,
                menu_list,
                menu_list_rows,
                call_popup: nowhere(),
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
                settings_toggles: places.toggles,
                settings_removes: places.removes,
                settings_doc: places.doc,
                settings_close: places.close,
                menu,
                menu_rows,
                menu_list,
                menu_list_rows,
                call_popup: nowhere(),
            };
        }

        let (body, input) = rest.split_bottom(shape.input_h.max(INPUT_H).min(rest.h));
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
            settings_toggles: Vec::new(),
            settings_removes: Vec::new(),
            settings_doc: nowhere(),
            settings_close: nowhere(),
            menu,
            menu_rows,
            menu_list,
            menu_list_rows,
            call_popup,
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
        // The flyout is above the menu inside that layer, so it is walked first
        // and its box is answered for before the menu's rows: it is drawn over
        // the menu it came from, and a row that is covered cannot be the row a
        // click lands on. The menu's own box is last, and swallows the press
        // that lands on its margin rather than letting it through to a pane.
        for (index, row) in &self.menu_list_rows {
            if row.contains(x, y) {
                return Some(Hit::MenuRow(*index));
            }
        }
        if self.menu_list.w >= 1.0 && self.menu_list.contains(x, y) {
            return Some(Hit::Menu);
        }
        for (index, row) in &self.menu_rows {
            if row.contains(x, y) {
                return Some(Hit::MenuRow(*index));
            }
        }
        if self.menu.w >= 1.0 && self.menu.contains(x, y) {
            return Some(Hit::Menu);
        }
        // Under the menu on the same layer, and above everything else. A menu
        // opened over the popup is the newer thing and takes the click; the
        // popup takes it from the panes it is drawn over.
        if self.call_popup.w >= 1.0 && self.call_popup.contains(x, y) {
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
        let room = (track.w - GAP).max(1.0);
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

    /// How many of the menu's widget list are on screen, for the wheel. Read
    /// off the rows the layout actually placed, so the scroll is bounded by
    /// what is drawn rather than by an arithmetic of its own.
    pub fn menu_capacity(&self) -> usize {
        self.menu_list_rows.len()
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

/// The two boxes an open menu can put on screen, and where every row on screen
/// is inside them.
struct MenuPlaces {
    box_: Panel,
    rows: Vec<(usize, Panel)>,
    /// The widget list's own box, and its rows. Both empty while the list is
    /// shut, which is most of the time.
    list: Panel,
    list_rows: Vec<(usize, Panel)>,
}

/// Where an open menu's box is, and where each of its rows on screen is inside
/// it.
///
/// Clamped into the window. A menu opened near the right edge or a row from the
/// bottom would otherwise hang off the surface, and the part that hangs off is
/// not merely invisible: no pointer can reach it, so the rows down there cannot
/// be picked at all.
///
/// The widget list is a box of its own beside the row that opened it, the way
/// every other desktop menu opens a submenu, and gets the same treatment carried
/// one step further. Out to the right by default; a menu opened near the right
/// edge has no room out there, so it flies out to the left instead rather than
/// off the surface where no pointer can reach it. Nine rows in a short window do
/// not fit either, so the box takes as many as there is room for and the list
/// scrolls through the rest.
fn place_menu(menu: &Menu, column: f32, width: f32, height: f32) -> MenuPlaces {
    let column = column.max(1.0);
    let w = (menu.width_chars() + MENU_GUTTER) as f32 * column + MENU_PAD * 2.0;
    let room = (((height - MENU_PAD * 2.0) / MENU_ROW_H).floor() as usize).max(1);
    let shown = menu.top.min(room);
    let h = shown as f32 * MENU_ROW_H + MENU_PAD * 2.0;
    let x = menu.at.0.min(width - w).max(0.0);
    let y = menu.at.1.min(height - h).max(0.0);
    let box_ = Panel::new(x, y, w, h);
    let rows = (0..shown)
        .map(|step| {
            (
                step,
                Panel::new(x, y + MENU_PAD + step as f32 * MENU_ROW_H, w, MENU_ROW_H),
            )
        })
        .collect();
    if menu.widgets() == 0 {
        return MenuPlaces {
            box_,
            rows,
            list: nowhere(),
            list_rows: Vec::new(),
        };
    }

    let lw = (menu.list_width_chars() + MENU_GUTTER) as f32 * column + MENU_PAD * 2.0;
    let seen = menu.widgets().min(room);
    // Where in the list the box starts. Clamped here rather than in the menu, so
    // a wheel that ran past the end does not leave the box half empty.
    let first = menu.first.min(menu.widgets() - seen);
    let lh = seen as f32 * MENU_ROW_H + MENU_PAD * 2.0;
    // Right of the menu, and left of it when the window has no room to the
    // right. Clamped after that for a window too narrow for either side, where
    // the flyout has to overlap the menu to be reachable at all.
    let out = x + w - MENU_OVERLAP;
    let lx = match out + lw <= width {
        true => out,
        false => (x + MENU_OVERLAP - lw).max(0.0).min(width - lw).max(0.0),
    };
    // Beside the row that opened it: its first row lines up with that row, and
    // the whole box is then clamped into the window like the menu's own.
    let opener = y + MENU_PAD + menu.top.saturating_sub(1) as f32 * MENU_ROW_H;
    let ly = (opener - MENU_PAD).min(height - lh).max(0.0);
    let list_rows = (0..seen)
        .map(|step| {
            (
                menu.top + first + step,
                Panel::new(lx, ly + MENU_PAD + step as f32 * MENU_ROW_H, lw, MENU_ROW_H),
            )
        })
        .collect();
    MenuPlaces {
        box_,
        rows,
        list: Panel::new(lx, ly, lw, lh),
        list_rows,
    }
}

/// How wide the activity popup is, in columns, and how much margin it keeps
/// inside its box.
///
/// Wide enough for a pretty-printed argument object and a stack trace without
/// wrapping every line of them, and capped again below against the window: a box
/// wider than what it is floating over is not a popup.
const POPUP_COLUMNS: usize = 88;
const POPUP_PAD: f32 = 10.0;

/// Where the activity popup sits and how big it is.
///
/// Centred, because it is about one row rather than opened at a point: a menu
/// belongs to the pixel it was opened on and this belongs to the call. As tall
/// as its contents up to nine tenths of the window, which is where it stops
/// growing and starts clipping. Nothing inside it scrolls, so the cells are
/// bounded at the source (`state::CELL_LINES`) rather than here.
fn place_popup(call: &crate::state::Call, column: f32, size: f32, width: f32, height: f32) -> Panel {
    let column = column.max(1.0);
    let line = Text::line_for(size);
    let room = (((width * 0.9 - POPUP_PAD * 2.0) / column).floor() as usize).max(8);
    let cols = POPUP_COLUMNS.min(room);
    let rows: usize = call
        .popup_lines()
        .iter()
        .map(|(text, _)| text_geometry::rows_in(text, cols, crate::state::PANE_WRAP).len())
        .sum();
    let w = (cols as f32 * column + POPUP_PAD * 2.0).min(width);
    let h = (rows as f32 * line + POPUP_PAD * 2.0)
        .min(height * 0.9)
        .max(line);
    Panel::new(
        ((width - w) * 0.5).max(0.0),
        ((height - h) * 0.5).max(0.0),
        w,
        h,
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

/// Where the picker's pieces are. A struct rather than a tuple, the way the
/// settings panel's are: five panels in a row is a call site nobody can read.
struct PickerPlaces {
    box_: Panel,
    list: Panel,
    rows: Vec<(usize, Panel)>,
    marks: Vec<(usize, Panel)>,
    open: Panel,
    filter: Panel,
    folders: Panel,
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

/// How much of the region that answers for the mark in front of a folder the
/// mark itself is drawn in, and the least it is ever drawn at.
///
/// Well under half of it. The mark used to be a filled glyph as tall as the row,
/// which made it the loudest thing on a row whose point is the folder's name,
/// and a solid block is a state rather than a control.
const PICKER_MARK_SIDE: f32 = 0.6;
const PICKER_MARK_MIN: f32 = 5.0;

/// The box that mark is drawn in, centred in the region that answers for
/// pressing it.
///
/// An odd side, so the plus inside it has a middle column and a middle row to
/// sit on. An even one puts the two bars off centre by half a pixel each and the
/// mark reads as a lower-case t.
fn picker_mark_box(mark: Panel) -> Panel {
    let side = (mark.w.min(mark.h) * PICKER_MARK_SIDE)
        .floor()
        .max(PICKER_MARK_MIN);
    let side = match side as i32 % 2 {
        0 => side + 1.0,
        _ => side,
    };
    Panel::new(
        mark.x + ((mark.w - side) * 0.5).floor(),
        mark.y + ((mark.h - side) * 0.5).floor(),
        side,
        side,
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
            filter: nowhere(),
            folders: nowhere(),
            sessions: nowhere(),
        };
    }
    let column = shape.pane_column.max(1.0);
    let line = Text::line_for(shape.pane_size);
    let head = picker_head_h(line);
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
    // The session list keeps one line above itself for the row that names its
    // columns. Taken out of the list rather than added to the head, so the box
    // is the same box in both lists and only the rows inside it move: a head
    // that changed height with the mode would move the whole dialog every time
    // the Sessions button was pressed.
    let header = match picker.on_sessions() {
        true => line,
        false => 0.0,
    };
    // Never past the room the box has for it: the head, the field and the
    // button all want a height of their own, and in a window too short for
    // them the list would otherwise start below the bottom of the box with the
    // field above it sized as if that room were there.
    let list = Panel::new(
        content.x,
        (content.y + head + header)
            .min(content.y + content.h - foot - GAP)
            .max(content.y),
        content.w,
        (content.h - head - header - foot - GAP).max(0.0),
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
    // The three buttons, all on the head's first row and all the same height.
    //
    // Open sits at the right limit of the box and the two that choose the list
    // sit at the left, so the button that acts on the row the cursor is on is
    // the furthest thing on the row from the two that only change what is being
    // listed. It used to be the other way round, with Open and the swap side by
    // side in the bottom left corner, which put a button that starts a session
    // one gap away from a button that does not.
    let button_h = picker_open_h(line).min(content.h);
    // Exactly as wide as what it says: the confirm glyph, the space after it,
    // [`PICKER_OPEN_LABEL`], a column of indent on the left and two on the right
    // so the cut corner never reaches the text.
    let open_w = ((ROW_ICON_COLUMNS + 1 + PICKER_OPEN_LABEL.chars().count() + 3) as f32 * column)
        .min(content.w);
    let open = Panel::new(
        content.x + content.w - open_w,
        content.y,
        open_w,
        button_h,
    );
    // The pair at the left, both sized for the longer of the two words, so
    // swapping the list does not move either of them, and both clipped to what
    // is left of the row once Open and a gap on either side of the pair have
    // been taken off: in a box too narrow for all three there are no mode
    // buttons rather than buttons sticking out of the picker.
    let mode_room = (content.w - open_w - GAP * 2.0).max(0.0);
    let mode_w = ((ROW_ICON_COLUMNS
        + 1
        + PICKER_SESSIONS_LABEL
            .chars()
            .count()
            .max(PICKER_FOLDERS_LABEL.chars().count())
        + 3) as f32
        * column)
        .min(((mode_room - GAP) * 0.5).max(0.0));
    let (folders, sessions) = match mode_w >= 1.0 {
        true => (
            Panel::new(content.x, content.y, mode_w, button_h),
            Panel::new(content.x + mode_w + GAP, content.y, mode_w, button_h),
        ),
        false => (nowhere(), nowhere()),
    };
    // The search field, under the buttons and the two lines of writing and above
    // the list. Its own panel rather than a rectangle worked out where the text
    // is drawn, so the border, the icon and what has been typed all come off one
    // shape.
    //
    // Its height is the room between the writing and the list, not the height a
    // field would like to be: in a window short enough that the head has no
    // room, a field taking its own height is drawn over the first rows of the
    // list. It ends up at nothing at that size, which is a picker with no field
    // rather than a field over the list. Its top is held inside the box for the
    // same reason: a field with no height still has a position, and a position
    // below the box is one the head of a short window would otherwise hand it.
    let field_top = (content.y + button_h + GAP + PICKER_HEAD_ROWS * line)
        .min(content.y + content.h)
        .max(content.y);
    let filter = Panel::new(
        content.x,
        field_top,
        content.w,
        picker_field_h(line).min((list.y - header - GAP - field_top).max(0.0)),
    );
    PickerPlaces {
        box_,
        list,
        rows,
        marks,
        open,
        filter,
        folders,
        sessions,
    }
}

/// Where the settings panel's pieces are. A struct rather than a tuple: six
/// panels in a row is a call site nobody can read, and four of them are lists.
///
/// The whole area rather than a centred box: this is a takeover, and a list in a
/// box the size of the picker's would be six rows of content and a lot of
/// margin. Two columns inside it: the rail of section names down the left, and
/// the chosen section beside it. The value column is a fixed number of columns in
/// from the right so every value lines up, which is what makes a screen of
/// settings scannable rather than a wall of words.
struct SettingsPlaces {
    box_: Panel,
    rail: Vec<(usize, Panel)>,
    divider: Divider,
    list: Panel,
    /// Every row, and every half of a form row: the box it is drawn in and
    /// clicked in. A row that is not a form is one entry on the left.
    rows: Vec<(usize, Side, Panel)>,
    values: Vec<(usize, Side, Panel)>,
    tracks: Vec<(usize, Side, Panel)>,
    cells: Vec<(usize, usize, Panel)>,
    toggles: Vec<(usize, Panel)>,
    removes: Vec<(usize, Panel)>,
    doc: Panel,
    close: Panel,
}

/// How much larger a group heading is than the rows under it.
///
/// A list whose groups are the same size as their contents is one list. The
/// heading takes two rows of the small text so the larger line has room and the
/// group has air above it, and [`crate::settings::lines`] says the same number
/// to the scroll window: a heading measured at one height and drawn at another
/// puts every click below it on the wrong row.
const SETTING_HEADING_SCALE: f32 = 1.35;

fn settings_heading_size(size: f32) -> f32 {
    (size * SETTING_HEADING_SCALE).round().max(size)
}

/// Where one cell of the palette grid sits inside its row.
///
/// Asked by the placement and by the drawing, the same way [`settings_control`]
/// is, so a swatch is drawn exactly where the press that names it is tested for.
fn settings_cell(row: Panel, cell: usize, cells: usize) -> Panel {
    let x = row.x + MARK_W + 3.0;
    let room = (row.x + row.w - x).max(1.0);
    let width = room / cells.max(1) as f32;
    Panel::new(x + cell as f32 * width, row.y, width, row.h)
}

/// How wide the trash at the end of a saved conversation is, in columns of pane
/// text.
///
/// Sized for the word the first press puts in it rather than for the mark, so
/// pressing it once does not change the width of the thing that was just
/// pressed and the second press lands on the same box.
const SETTING_SESSION_TRASH_COLUMNS: usize = 7;

/// Where every cell of the saved-conversations table sits inside one row.
///
/// One answer for the row that names the columns and for every row under it, so
/// a cell cannot drift out from under its own header. The widths come from the
/// model, [`crate::settings::SESSION_COLUMNS`]; the two written as zero there
/// are the first message, which takes whatever is left, and the trash, which is
/// a button pinned to the right end of the row.
///
/// Cell by cell rather than one space padded string, which is what the picker's
/// list does: this table has a column that is a button and a line between every
/// pair of columns, and both of those need each cell's own x.
fn settings_session_cells(row: Panel, column: f32) -> Vec<Panel> {
    let x = row.x + MARK_W + 3.0;
    let right = row.x + row.w;
    let trash =
        (SETTING_SESSION_TRASH_COLUMNS as f32 * column).min(((right - x) * 0.5).floor().max(0.0));
    let mut out = Vec::new();
    let mut at = x;
    for (_, wide) in crate::settings::SESSION_COLUMNS
        .iter()
        .take(crate::settings::SESSION_CELLS)
    {
        let room = (right - trash - at).max(0.0);
        let want = match wide {
            0 => room,
            wide => *wide as f32 * column,
        };
        let w = want.min(room);
        out.push(Panel::new(at, row.y, w, row.h));
        at += w;
    }
    out.push(Panel::new(right - trash, row.y, trash, row.h));
    out
}

/// The lines between the columns of the saved-conversations table.
///
/// One down the left edge of every column but the first, on the header and on
/// every row under it, so the table reads as a grid rather than as words that
/// happen to line up. The hairline along the bottom of a row is drawn by the
/// list itself, the same way it is for every other row on the panel.
fn settings_session_lines(scene: &mut Scene, cells: &[Panel], row: Panel, edge: [f32; 4]) {
    if row.h < 2.0 {
        return;
    }
    for at in cells.iter().skip(1) {
        if at.w < 1.0 || at.x - 1.0 < row.x {
            continue;
        }
        scene.rect(Panel::new(at.x - 1.0, row.y, 1.0, row.h).fill(edge));
    }
}

/// Where a row's control sits: one column in from the label, the width a value
/// needs and no more.
///
/// Beside the label rather than pinned to the right edge of the panel. The panel
/// is the whole window now, and a value at the far right of a 1400 pixel row is
/// a value nobody can read against the key it belongs to: the eye has to cross
/// the width of the screen. Everything lines up in the one column instead, which
/// is what makes a screen of settings scannable.
///
/// Asked by the placement and by the drawing, so a value is drawn exactly where
/// the click that changes it is tested for.
fn settings_control(row: Panel, label_w: f32, column: f32) -> Panel {
    let x = row.x + MARK_W + 3.0 + label_w;
    let room = (row.x + row.w - x).max(1.0);
    Panel::new(
        x,
        row.y,
        (SETTING_VALUE_COLUMNS as f32 * column).min(room),
        row.h,
    )
}

/// The least the rail of section names goes down to, and how wide the label
/// column of a row is, both in columns of pane text.
///
/// The rail holds the longest section name with room for its mark; the label
/// column holds the longest key in the settings file. The rail's number is a
/// floor rather than its width, because the rail is dragged: it is the room the
/// names need, and the settings beside them are held to the same floor, so
/// neither side of the drag can be squeezed away.
const SETTING_RAIL_COLUMNS: usize = 14;
const SETTING_LABEL_COLUMNS: usize = 24;

fn settings_rail_floor(column: f32) -> f32 {
    SETTING_RAIL_COLUMNS as f32 * column.max(1.0)
}

/// Where a row's value starts when it is a reading rather than a control: after
/// the label, and running to the end of the row.
///
/// A reading's value is usually a path, which is longer than the value column
/// and would be three dots in it. A control's value is short and lines up down
/// the right instead.
fn settings_label_w(list_w: f32, column: f32) -> f32 {
    (SETTING_LABEL_COLUMNS as f32 * column).min((list_w * 0.5).floor())
}

/// Where the two halves of a form row sit.
///
/// Half each, floored to a whole pixel so the line between them does not land on
/// a half one, and the same split is asked for by the placement, the hit test
/// and the drawing.
fn settings_halves(row: Panel) -> [(Side, Panel); 2] {
    let half = (row.w * 0.5).floor().max(1.0);
    [
        (Side::Left, Panel::new(row.x, row.y, half, row.h)),
        (
            Side::Right,
            Panel::new(row.x + half, row.y, (row.w - half).max(1.0), row.h),
        ),
    ]
}

/// How much of a slider's row the number beside the track takes. The track gets
/// the rest.
///
/// Eight, because the widest number on any track is the agent's context window
/// and that is seven digits at the top of its range. Six fitted every setting of
/// the window's own and clipped `1048576` to `10485\u{2026}`, which is a slider
/// whose number cannot be read at the end anybody would drag it to.
const SETTING_TRACK_VALUE_COLUMNS: usize = 8;

/// What the two controls on an entry row take, in columns of pane text: the
/// toggle that turns it on and off, and the uninstall beside it.
///
/// Both are boxes with a word in them, sized for the longer of the two words
/// they can hold, so pressing one does not change the width of the thing that
/// was just pressed.
const SETTING_TOGGLE_COLUMNS: usize = 5;
const SETTING_REMOVE_COLUMNS: usize = 11;

/// The least the column beside the entry list goes down to, in columns of pane
/// text, and the least the list itself keeps.
///
/// Below either of them the split is not made at all and the section is one
/// column: a document forty characters wide is a column of hyphenated words,
/// and a list of skills squeezed to nothing is a list nobody can read.
const SETTING_DOC_MIN_COLUMNS: usize = 40;
const SETTING_ENTRY_MIN_COLUMNS: usize = 34;

/// How much of the two-column split the list takes. The document gets the rest.
const SETTING_ENTRY_SHARE: f32 = 0.45;

fn place_settings(area: Panel, shape: &Shape, panel: &Settings) -> SettingsPlaces {
    if area.w < 1.0 || area.h < 1.0 {
        return SettingsPlaces {
            box_: nowhere(),
            rail: Vec::new(),
            divider: Divider::none(),
            list: nowhere(),
            rows: Vec::new(),
            values: Vec::new(),
            tracks: Vec::new(),
            cells: Vec::new(),
            toggles: Vec::new(),
            removes: Vec::new(),
            doc: nowhere(),
            close: nowhere(),
        };
    }
    let column = shape.pane_column.max(1.0);
    let line = Text::line_for(shape.pane_size);
    let content = area.inset(PAD);
    // The heading, and the footer that says what the keys do.
    let head = line;
    let foot = line;
    let body = Panel::new(
        content.x,
        content.y + head + GAP,
        content.w,
        (content.h - head - foot - GAP * 2.0).max(0.0),
    );
    // The rail takes its share of the body the way a column of panes takes its
    // share of the grid: a fraction off the settings file, held so neither the
    // names nor the settings beside them are squeezed below the room the names
    // need. Floored to a whole pixel so the line does not sit on a half one.
    let room = (body.w - GAP).max(1.0);
    let rail_floor = settings_rail_floor(column);
    let rail_w = (room * held(shape.settings_rail, room, rail_floor)).floor();
    let mut rail = Vec::new();
    for (index, _) in panel.section_names().iter().enumerate() {
        let y = body.y + index as f32 * line;
        // A rail taller than the window keeps the sections that fit rather than
        // drawing over the footer. Eight names in a window that cannot hold
        // eight rows is a window nothing is readable in anyway.
        if y + line > body.y + body.h {
            break;
        }
        rail.push((index, Panel::new(body.x, y, rail_w, line)));
    }
    let list = Panel::new(
        body.x + rail_w + GAP,
        body.y,
        (body.w - rail_w - GAP).max(0.0),
        body.h,
    );
    // What the rail is dragged by. The gap between the two, plus [`GRAB`] on the
    // rail's side of it and a single pixel on the list's: the list's rows start
    // at its own left edge, and a band that reached into them would take the
    // press that puts the cursor on a row.
    let divider = Divider {
        band: Panel::new(list.x - GAP - GRAB, body.y, GAP + GRAB + 1.0, body.h),
        track: body,
        floor: rail_floor,
    };
    // A section that lists skills or servers is two columns: the entries on the
    // left and the one under the cursor beside them. Split off the list rather
    // than off the body, so the rail keeps the width it was dragged to and the
    // document takes its share of what is left. Not split at all when either
    // half would be too narrow to read, which is what a small window gets: the
    // entries win, since the document is what the entry is for.
    let (list, doc) = match panel.showing() {
        Some(_) => {
            let total = columns_in(list.w, column);
            match total.checked_sub(SETTING_DOC_MIN_COLUMNS + SETTING_ENTRY_MIN_COLUMNS) {
                Some(_) => {
                    let want = (list.w * SETTING_ENTRY_SHARE).floor();
                    let least = SETTING_ENTRY_MIN_COLUMNS as f32 * column;
                    let most = list.w - SETTING_DOC_MIN_COLUMNS as f32 * column;
                    list.split_left(want.clamp(least, most.max(least)))
                }
                None => (list, nowhere()),
            }
        }
        None => (list, nowhere()),
    };
    // The entries themselves stop a gap short of the document, so the two
    // columns read as two columns and a long name does not run into the text.
    let list = match doc.w >= 1.0 {
        true => Panel::new(list.x, list.y, (list.w - GAP).max(1.0), list.h),
        false => list,
    };
    let rows_fit = Text::rows_for(shape.pane_size, list.h);
    let (first, count) = panel.window(rows_fit);
    let mut rows = Vec::new();
    let mut values = Vec::new();
    let mut tracks = Vec::new();
    let mut cells = Vec::new();
    let mut toggles = Vec::new();
    let mut removes = Vec::new();
    // A running height rather than the row number times a line: a heading is two
    // rows of text tall and everything under it moves down by that much. The
    // heights come from the model, which is what the scroll window counts in.
    let mut y = list.y;
    for step in 0..count {
        let index = first + step;
        let Some(entry) = panel.row(index) else {
            break;
        };
        // Cut off at the bottom of the list rather than drawn over it: a list
        // one row tall that opens on a heading has room for one row of it.
        let tall = (crate::settings::lines(entry) as f32 * line).min(list.y + list.h - y);
        if tall < 1.0 {
            break;
        }
        let row = Panel::new(list.x, y, list.w, tall);
        y += tall;
        // A form row is two boxes side by side and everything else is one. Each
        // half is placed exactly the way a whole row is, with its own label
        // column measured off its own width, so a control in one is drawn where
        // the press that changes it is tested for.
        let halves: Vec<(Side, Panel)> = match entry {
            SettingRow::Pair(_, _) => settings_halves(row).into(),
            _ => vec![(Side::Left, row)],
        };
        for (side, at) in &halves {
            rows.push((index, *side, *at));
            let cell = crate::settings::cell(entry, *side);
            let label_w = settings_label_w(at.w, column);
            let value_at = settings_control(*at, label_w, column);
            // Only a row that carries a control gets one, and a control is
            // either a value or a track. A heading or a reading with a click
            // region over its value would answer a press with nothing.
            match cell {
                SettingRow::Setting { kind, .. } if kind.fraction(0.0).is_some() => {
                    let number =
                        (SETTING_TRACK_VALUE_COLUMNS as f32 * column).min(value_at.w * 0.5);
                    tracks.push((
                        index,
                        *side,
                        Panel::new(value_at.x, value_at.y, value_at.w - number, value_at.h),
                    ));
                }
                SettingRow::Setting { .. } | SettingRow::Field { .. } => {
                    values.push((index, *side, value_at));
                }
                _ => {}
            }
        }
        match entry {
            // The whole row is controls: one cell per colour on it.
            SettingRow::Swatches(swatches) => {
                for cell in 0..swatches.len() {
                    cells.push((index, cell, settings_cell(row, cell, swatches.len())));
                }
            }
            // The toggle and, where there is one, the uninstall: both at the
            // right of the row's first line, so they line up down the list the
            // way every value on the panel does. Nothing at all in a column too
            // narrow to hold them beside a name, since a button drawn over the
            // name it belongs to is a press nobody can aim.
            // The trash at the end of a saved conversation, in the last column
            // of the table. Placed from the same function that places the cells
            // beside it, so the button is under its own header. Nothing at all
            // in a row too narrow to hold it, since a button drawn over the
            // words it belongs to is a press nobody can aim.
            SettingRow::Session { .. } => {
                if let Some(trash) = settings_session_cells(row, column).last()
                    && trash.w >= column * 3.0
                {
                    removes.push((index, Panel::new(trash.x, row.y, trash.w, line)));
                }
            }
            SettingRow::Entry(entry) => {
                let remove_w = match entry.removable {
                    true => SETTING_REMOVE_COLUMNS as f32 * column,
                    false => 0.0,
                };
                let toggle_w = SETTING_TOGGLE_COLUMNS as f32 * column;
                let gap = match entry.removable {
                    true => column,
                    false => 0.0,
                };
                let x = row.x + row.w - remove_w - gap - toggle_w;
                if x > row.x + MARK_W + 3.0 + column * 8.0 {
                    toggles.push((index, Panel::new(x, row.y, toggle_w, line)));
                    if entry.removable {
                        removes.push((
                            index,
                            Panel::new(x + toggle_w + gap, row.y, remove_w, line),
                        ));
                    }
                }
            }
            _ => {}
        }
    }
    // Top right, one cut's reach in from the corner the cut takes away, so the
    // mark is not drawn in the triangle that is not there.
    let close = Panel::new(content.x + content.w - CUT - line, content.y, line, line);
    SettingsPlaces {
        box_: area,
        rail,
        divider,
        list,
        rows,
        values,
        tracks,
        cells,
        toggles,
        removes,
        doc,
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
    // nothing to type at. The menu still goes on top: a right click on a session
    // row opens one, and the layout has always placed and hit tested it here, so
    // returning before the overlay left a menu that answered presses and was
    // nowhere on screen.
    if layout.picking {
        folder_picker(&mut scene, frame);
        overlay(&mut scene, frame);
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
    call_popup(scene, frame);
    let Some(menu) = frame.menu else {
        return;
    };
    let (skin, layout) = (frame.skin, frame.layout);
    if layout.menu.w < 1.0 {
        return;
    }
    scene.over_rect(panel_fill(layout.menu, skin.menu));
    scene.over_rect(panel_edge(layout.menu, skin.edge_focus));
    // The flyout is a box of its own with a border of its own, over the menu it
    // came from rather than a continuation of it, so it is filled after it.
    if layout.menu_list.w >= 1.0 {
        scene.over_rect(panel_fill(layout.menu_list, skin.menu));
        scene.over_rect(panel_edge(layout.menu_list, skin.edge_focus));
    }
    for (rows, chars) in [
        (&layout.menu_rows, menu.width_chars()),
        (&layout.menu_list_rows, menu.list_width_chars()),
    ] {
        for (index, panel) in rows {
            let Some(row) = menu.rows.get(*index) else {
                continue;
            };
            menu_row(scene, frame, *row, *index, *panel, chars);
        }
    }
}

/// One activity row opened out: what was invoked, when, what the model
/// generated, what came back and the detail.
///
/// What it says is [`crate::state::Call::popup_lines`]; this only puts it on the
/// screen. On the floating layer with `over_rect`/`over_text` for the reason the
/// menu is: a box pushed onto the base layer is painted before every glyph in
/// the window and comes out underneath the pane text it is covering.
fn call_popup(scene: &mut Scene, frame: &Frame) {
    let Some(call) = frame.state.popped() else {
        return;
    };
    let (skin, box_) = (frame.skin, frame.layout.call_popup);
    if box_.w < 1.0 || box_.h < 1.0 {
        return;
    }
    scene.over_rect(panel_fill(box_, skin.menu));
    scene.over_rect(panel_edge(box_, skin.edge_focus));
    let text = box_.inset(POPUP_PAD);
    let cols = cols_of(text, frame.pane_column);
    let mut runs = Vec::new();
    for (line, tone) in call.popup_lines() {
        runs.push(Run::tinted(line, skin.tone(tone)));
        runs.push(Run::plain("\n"));
    }
    scene.over_text(Text::rich(runs, text, frame.pane_size, skin.body).wrap_at(cols));
}

/// One row of a menu or of its flyout: the mark in the gutter, the label, and
/// the flyout marker at the far end for a row that has one.
///
/// `chars` is how many columns the labels in this box are laid out across, which
/// is what puts the marker at the end of the row rather than after the label.
/// Padded out in the run itself rather than drawn in a box of its own, so the
/// mark and the label are one shaped line and cannot come apart from each other.
fn menu_row(
    scene: &mut Scene,
    frame: &Frame,
    row: crate::menu::Row,
    index: usize,
    panel: Panel,
    chars: usize,
) {
    let skin = frame.skin;
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
    runs.push(Run::tinted(format!(" {}", row.item.label()), tint));
    let text = Panel::new(
        panel.x + MENU_PAD,
        panel.y,
        (panel.w - MENU_PAD * 2.0).max(1.0),
        panel.h,
    );
    let line = Text::line_for(SMALL);
    scene.over_text(Text::rich(runs, text.row(0.0, line), SMALL, tint));

    let Some(mark) = row.item.marker() else {
        return;
    };
    // The last columns of the row, in a box of their own rather than spaces
    // written after the label. Padding a label out to the edge puts the mark
    // hard against the wrap width, where a column of drift between the symbol
    // font and the monospace one carries it onto a second line the row is not
    // tall enough to show, and a mark that is not drawn at all is the one
    // failure this window has already had once.
    let room = text.w / (chars + MENU_GUTTER) as f32 * MARKER_COLUMNS as f32;
    let at = Panel::new(text.x + text.w - room, text.y, room, text.h);
    scene.over_text(Text::rich(
        vec![Run::icon(mark.to_string(), tint)],
        at.row(0.0, line),
        SMALL,
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

/// A line along the cut itself, so the corner is a drawn edge rather than only a
/// missing one.
///
/// [`Rect::stroke`] follows the whole shape and cannot be asked for one side, so
/// a box that wants its diagonal without its other three sides has to draw the
/// diagonal itself. A stair of `weight` sized squares stepping one pixel down and
/// one pixel left, which at the hairline weight every other edge is drawn at is
/// one pixel per step: the fragment stage measures a square by its narrow axis,
/// so each step lands at the same coverage as the straight hairlines it meets.
///
/// It runs from where the top edge would stop, `(right - cut, top)`, to where the
/// right edge starts, `(right - weight, top + cut - weight)`, so the two ends
/// meet whatever else the caller draws.
fn cut_line(scene: &mut Scene, panel: Panel, rgba: [f32; 4], weight: f32) {
    let cut = cut_of(panel);
    if cut < weight {
        // A box squeezed smaller than the line is meant to be thick lost its
        // corner to the cap in `cut_of`; there is no diagonal left to draw.
        return;
    }
    let right = panel.x + panel.w;
    let steps = (cut - weight) as usize;
    for step in 0..=steps {
        let at = step as f32;
        scene.rect(Panel::new(right - cut + at, panel.y + at, weight, weight).fill(rgba));
    }
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
/// Thin rectangles rather than the one stroked rect [`panel_edge`] draws, because
/// a stroke follows the whole shape and cannot leave a side out. The three that
/// are left are straight lines, and the cut is on the top right, which is the
/// corner the top edge had.
///
/// The cut is bordered too ([`cut_line`]). Every other chromed box in the window
/// is stroked all the way round, so a line runs down their diagonal and the pane
/// was the one box where the corner was a hole in the border instead of a corner.
/// In the same colour as the other three sides: a pane is one material, and a
/// corner in a second colour reads as a second thing stuck on it.
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
    cut_line(scene, panel, rgba, 1.0);
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
    // And picked up again there, at half the weight, so the accent turns the
    // corner instead of stopping in mid air: down the cut, on down the right
    // edge, then a short run back along the foot. Half rather than the same
    // weight because the top of the tab is the reading and the rest of it is
    // the outline of the reading; at full weight the tab is a boxed label
    // again, which is what the fills were taken away for.
    let weight = TAB_EDGE_H.min(tab.h);
    cut_line(scene, tab, skin.tab_accent, weight);
    scene.rect(
        Panel::new(
            tab.x + tab.w - weight,
            tab.y + cut,
            weight,
            (tab.h - cut).max(0.0),
        )
        .fill(skin.tab_accent),
    );
    // The last pixels inside the tab, not the first row of the pane: the tab and
    // its pane are one surface, and a line drawn at the pane's top edge is the
    // rule under the strip that item 12 took away. As long as the cut reaches,
    // so the foot and the diagonal are the same length and the border reads as
    // one turn rather than as two stubs.
    let foot = cut.min(tab.w);
    scene.rect(
        Panel::new(
            tab.x + tab.w - foot,
            tab.y + tab.h - weight,
            foot,
            weight.min(tab.h),
        )
        .fill(skin.tab_accent),
    );
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
    // The columns the text is in and the columns in front of it, from the one
    // place that says so: the file view keeps four for its line numbers, and a
    // band measured in the full width of the box was four columns wide of the
    // glyphs on every row of a file.
    let (cols, chrome) = text_columns(view, panel, column);
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
    //
    // The box names its column count, so the renderer breaks the rows with the
    // same `text-geometry` call the pane was measured with rather than wrapping
    // them itself. Left to the shaper the columns drift by one per blank it
    // swallows at a break, and the selection lands on the wrong glyphs.
    scene.text(
        Text::rich(runs, panel.inset(PAD), frame.body_size, frame.skin.body)
            .scrolled(state.output.window(rows, cols).skip as f32)
            .wrap_at(cols),
    );
    scrollbar(scene, skin, panel, state.output.thumb(rows, cols));
}

fn activity(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let rows = frame.layout.rows(panel, frame.pane_size);
    let cols = cols_of(panel, frame.pane_column);
    let mut runs = Vec::new();
    for line in state.activity.visible(rows, cols) {
        // The clock column in front of a row is the subordinate part of it and
        // is drawn in the dim tone: what the eye is looking down the list for
        // is the tag and the subject, and a time in the call's own color would
        // be competing with them for the row.
        let (clock, rest) = line.split_gutter();
        if !clock.is_empty() {
            runs.push(Run::tinted(clock, skin.dim));
        }
        runs.push(Run::tinted(rest, skin.tone(line.tone)));
        runs.push(Run::plain("\n"));
    }
    scene.text(
        Text::rich(runs, panel.inset(PAD), frame.pane_size, frame.skin.body)
            .scrolled(state.activity.window(rows, cols).skip as f32)
            .wrap_at(cols),
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

/// One logical line of a list pane: the runs that draw it, and the text they
/// draw.
///
/// The text is taken off the runs rather than passed in beside them, and the
/// height is measured from it by the same call the renderer breaks the rows
/// with. A row counted one way and drawn another is a pane that scrolls by a
/// different number of rows than it has, and a row of prose with blanks in it
/// wraps at a different place from a row of the same length without them.
struct ListRow {
    runs: Vec<Run>,
    text: String,
}

impl ListRow {
    fn new(runs: Vec<Run>) -> ListRow {
        let text = runs.iter().map(|run| run.text.as_str()).collect();
        ListRow { runs, text }
    }

    /// How many rows this takes in a box `cols` wide.
    fn rows(&self, cols: usize) -> usize {
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
fn list_pane(scene: &mut Scene, frame: &Frame, panel: Panel, view: View, rows: Vec<ListRow>) {
    let size = frame.pane_size;
    let fit = frame.layout.rows(panel, size);
    let cols = cols_of(panel, frame.pane_column);
    let heights: Vec<usize> = rows.iter().map(|row| row.rows(cols)).collect();
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
        View::Plan => lines(plan_rows(frame.state, frame.skin)),
        View::Agents => lines(agent_rows(frame.state, frame.skin)),
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

/// The CONTEXT pane: what phase this run is in, what it has asked for, and how
/// full it is.
///
/// The header is four rows with labels beside them, in the separation the phase
/// row has had since the title strip was cut back. The three under the phase are
/// counts that were readings in the list below until they were the three most
/// worth reading first, so they came up here: a labelled row is easier to find
/// than a dot block. The model and the workspace were up here too and are not
/// any more, because the strip says the path again and the model is on the
/// settings panel. The readings under the header are [`Monitor::context`], named
/// for this pane.
fn context(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let content = panel.inset(PAD);
    let line = Text::line_for(frame.pane_size);
    // The failed count rides with the total rather than getting a row of its
    // own: the two are one reading, and a pane that failed nothing still says
    // so. It is where the DEBUG pane's count went when that pane was removed.
    let calls = match state.failed_calls {
        0 => crate::state::thousands(state.tool_calls as u64),
        failed => format!(
            "{} ({} failed)",
            crate::state::thousands(state.tool_calls as u64),
            crate::state::thousands(failed as u64)
        ),
    };
    let rows: [(&str, String, [u8; 4]); CONTEXT_HEAD] = [
        (
            "PHASE",
            match state.resumed {
                true => format!("{} (resumed)", state.phase.word()),
                false => state.phase.word().to_string(),
            },
            // The bad colour while a turn is running, which is the one tint in
            // the palette that pulls the eye off whatever it was reading. It is
            // not a fault: it is the reading that says the machine has the turn
            // and anything you type is queued behind it.
            if state.phase.busy() {
                skin.bad
            } else {
                skin.body
            },
        ),
        (
            "TOTAL REQUESTS",
            crate::state::thousands(state.requests as u64),
            skin.body,
        ),
        (
            "TOTAL TOOL CALLS",
            calls,
            // In the fault colour once something has failed, which is how the
            // pane it came from read its own count. A run with nothing wrong
            // with it reads the same as every other row.
            match state.failed_calls {
                0 => skin.body,
                _ => skin.bad,
            },
        ),
        (
            "LAST PREFILL",
            crate::state::thousands(state.last_prefill),
            skin.body,
        ),
    ];
    // As wide as the longest label, the way the readings below size theirs.
    // Fixed at ten columns, "TOTAL TOOL CALLS" ran into its own number.
    let label_cols = rows
        .iter()
        .map(|(label, _, _)| label.chars().count())
        .max()
        .unwrap_or(LABEL_COLUMNS)
        .max(LABEL_COLUMNS)
        + 1;
    let label_w = label_cols as f32 * frame.pane_column;
    for (index, (label, value, tint)) in rows.iter().enumerate() {
        let y = content.y + index as f32 * line;
        scene.text(Text::rich(
            vec![Run::tinted(*label, skin.dim)],
            Panel::new(content.x, y, label_w.max(1.0), line),
            frame.pane_size,
            skin.dim,
        ));
        // Clipped, not wrapped: the rows are at fixed heights, so a long value
        // that wrapped would have its second row cut off by its own box.
        let room = cols_of(panel, frame.pane_column).saturating_sub(label_cols + 1);
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
/// header itself does not scroll: it is four rows saying what this run is
/// doing and what it has asked for, and a monitor whose first rows scrolled
/// away would be a monitor with no summary.
fn gauge_area(panel: Panel, size: f32) -> Option<Panel> {
    let line = Text::line_for(size);
    let used = CONTEXT_HEAD as f32 * line + line * 0.5;
    if panel.h - used < line {
        return None;
    }
    Some(Panel::new(panel.x, panel.y + used, panel.w, panel.h - used))
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
    let (cols, chrome) = text_columns(View::Files, body, frame.pane_column);
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
        // The gutter, so a diff line says where in the file it landed. Exactly
        // `chrome` columns of it, on this row and on every row this line
        // continues onto.
        match entry.number {
            Some(number) => runs.push(Run::tinted(file_number(number, chrome), skin.comment)),
            None if !entry.text.is_empty() => runs.push(Run::plain(" ".repeat(chrome))),
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
    // Broken into rows by the same call the pane counts them with, in the
    // columns that are left once the gutter has been paid for, and the rows a
    // line continues onto are indented past that gutter. Wrapping the gutter
    // along with the text is what put every continuation row four columns out
    // from the band, the caret and the clipboard.
    scene.text(
        Text::rich(runs, content, frame.pane_size, skin.body)
            .wrap_at(cols)
            .hanging(chrome),
    );
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
            scene.rect(Panel::new(row.x, row.y, MARK_W, row.h).fill(skin.tab_accent));
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

    // The three buttons, on the row above the writing: the two that choose the
    // list at the left, the one that opens the row the cursor is on at the right
    // limit of the box.
    //
    // The pair carries a third face on top of the idle and the hot one every
    // button here has, and the one whose list is showing wears it: the band the
    // chosen row is drawn in, written over in the same dark ink. Two buttons
    // drawn identically are two buttons that do not say which list is in front
    // of you, and that answer used to be carried by the heading alone.
    let on_sessions = picker.on_sessions();
    for (panel, hit, icon, label) in [
        (
            layout.picker_folders,
            Hit::PickerFolders,
            icons::FOLDER,
            PICKER_FOLDERS_LABEL,
        ),
        (
            layout.picker_sessions,
            Hit::PickerSessions,
            icons::RECENT,
            PICKER_SESSIONS_LABEL,
        ),
        (
            layout.picker_open,
            Hit::PickerOpen,
            icons::CONFIRM,
            PICKER_OPEN_LABEL,
        ),
    ] {
        if panel.w < 1.0 || panel.h < 1.0 {
            continue;
        }
        let showing = match hit {
            Hit::PickerFolders => !on_sessions,
            Hit::PickerSessions => on_sessions,
            _ => false,
        };
        // The showing mode keeps its band under the pointer. Pressing it does
        // nothing, so lighting it would promise a change that never comes.
        let (face, ink) = match (showing, frame.hot == Some(hit)) {
            (true, _) => (skin.picked, skin.picked_ink),
            (false, true) => (skin.button_hot, skin.bright),
            (false, false) => (skin.button, skin.bright),
        };
        scene.rect(panel_fill(panel, face));
        scene.rect(panel_edge(panel, skin.edge_focus));
        say(
            scene,
            vec![
                Run::icon(icon.to_string(), ink),
                Run::tinted(format!(" {label}"), ink),
            ],
            Panel::new(
                panel.x + frame.pane_column,
                panel.y + PICKER_OPEN_PAD,
                (panel.w - frame.pane_column).max(1.0),
                line,
            ),
            ink,
        );
    }
    // The title, one string in both lists, and what the session list says about
    // itself beside it: how many there are, and how many files in the directory
    // could not be described.
    let writing = layout.picker_open.y + layout.picker_open.h + GAP;
    let mut head = vec![Run::tinted(PICKER_TITLE, skin.bright)];
    if let Some(note) = picker.note() {
        let room = cols.saturating_sub(PICKER_TITLE.chars().count() + 2);
        head.push(Run::tinted(format!("  {}", clip(note, room)), skin.dim));
    }
    say(
        scene,
        head,
        Panel::new(content.x, writing, content.w, line),
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
        Panel::new(content.x, writing + line, content.w, line),
        skin.body,
    );
    // What has been typed, why the list is empty when it is empty for a reason,
    // or why the last press did nothing. A folder with no permission looks
    // exactly like an empty folder otherwise, and a button that silently does
    // not work looks exactly like a button that is broken.
    //
    // In a box of its own, with the surface the prompt is drawn on, the hairline
    // every panel here carries and the same cut corner. It was a line of text
    // with a funnel in front of it, which said the list had been narrowed and
    // never said that this is the thing you type into.
    let field = layout.picker_filter;
    let mut runs = vec![Run::icon(icons::SEARCH.to_string(), skin.dim), Run::plain(" ")];
    let room = cols.saturating_sub(ROW_ICON_COLUMNS + 2);
    let tint = match (picker.refused().or(picker.trouble()), picker.filter()) {
        (Some(why), _) => {
            runs.push(Run::tinted(clip(why, room), skin.bad));
            skin.bad
        }
        (None, "") => {
            runs.push(Run::tinted("type to narrow the list", skin.dim));
            skin.dim
        }
        (None, typed) => {
            runs.push(Run::tinted(clip(typed, room), skin.bright));
            skin.bright
        }
    };
    if field.w >= 1.0 && field.h >= 1.0 {
        scene.rect(panel_fill(field, skin.input));
        scene.rect(panel_edge(field, skin.edge_focus));
        say(
            scene,
            runs,
            Panel::new(
                field.x + PAD,
                field.y + PICKER_FIELD_PAD,
                (field.w - 2.0 * PAD).max(1.0),
                line,
            ),
            tint,
        );
    }

    // The row that names the columns, on the line the layout kept above the
    // list. Only the sessions are a table: a folder list is one column of names
    // and a header over it would be a word explaining the obvious.
    if picker.on_sessions() {
        let head_row = Panel::new(
            layout.picker_list.x,
            layout.picker_list.y - line,
            layout.picker_list.w,
            line,
        );
        let (at, room) = session_table(head_row, frame.pane_column);
        let names: Vec<String> = SESSION_COLUMNS
            .iter()
            .map(|(name, _)| String::from(*name))
            .chain(std::iter::once(String::from(SESSION_OPENING)))
            .collect();
        say(
            scene,
            vec![Run::tinted(session_line(&names, room), skin.dim)],
            Panel::new(at, head_row.y, (head_row.w - (at - head_row.x)).max(1.0), line),
            skin.dim,
        );
    }

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
        // The mark that opens and shuts the folder, drawn inside the region that
        // answers for pressing it: a hairline box with a plus in it, or with the
        // plus's upright taken away once the folder is open.
        //
        // Rectangles rather than a glyph. It was Font Awesome's filled
        // plus-square at the size of the row's text, which put a solid block at
        // the front of every folder in the list: the biggest, heaviest thing on
        // a row whose point is the folder's name. Nothing is filled here and the
        // box is well under the height of the row.
        let (indent, wide) = picker_indent(entry.depth(), frame.pane_column, list_cols);
        if let Some(open) = entry.open() {
            let hot = frame.hot == Some(Hit::PickerMark(*index));
            // Green, and the panel colour instead on the row the cursor is on,
            // where the band behind it is already that green. Under the pointer
            // it keeps its colour and doubles its weight: a second colour for a
            // hover is a mark that means two things.
            let edge = match on {
                true => skin.mark_on_band,
                false => skin.mark_edge,
            };
            let weight = match hot {
                true => 2.0,
                false => 1.0,
            };
            let square = picker_mark_box(Panel::new(row.x + indent, row.y, wide, line));
            scene.rect(square.outline(edge, weight));
            // The bars sit two pixels inside the box on every side, so the plus
            // never touches the edge round it, and both are one pixel: the box
            // is nine across and a thicker bar closes the gap up.
            let middle = ((square.w - 1.0) * 0.5).floor();
            let arm = (square.w - 4.0).max(1.0);
            scene.rect(Panel::new(square.x + 2.0, square.y + middle, arm, 1.0).fill(edge));
            if !open {
                scene.rect(Panel::new(square.x + middle, square.y + 2.0, 1.0, arm).fill(edge));
            }
        }
        let start = indent + wide;
        // A session is a table row: the glyph in the gutter, then the cells, at
        // the one x the header above the list is written at. Its own text rather
        // than one run after the glyph, because the header has no glyph and the
        // two have to line up to the pixel.
        if let PickerRow::Session(saved) = entry {
            let (at, room) = session_table(*row, frame.pane_column);
            say(
                scene,
                vec![Run::icon(icon.to_string(), tint)],
                Panel::new(row.x + start, row.y, (row.w - start).max(1.0), line),
                tint,
            );
            say(
                scene,
                vec![Run::tinted(
                    session_line(&picker.session_cells(saved), room),
                    tint,
                )],
                Panel::new(at, row.y, (row.x + row.w - at).max(1.0), line),
                tint,
            );
            continue;
        }
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

    // The keys, spelled out, on the last line of the box. Nothing else in this
    // window needs them written down, but this is the first thing a new install
    // shows and it is the one place where there is no pane to experiment in.
    //
    // It is the whole of the foot now that the buttons are in the head, so it is
    // placed off the bottom of the box, which is where [`picker_foot`] says the
    // one line it keeps down there is.
    say(
        scene,
        vec![Run::tinted(
            clip(
                "enter opens \u{2022} right walks in \u{2022} left goes out \u{2022} esc quits",
                cols,
            ),
            skin.dim,
        )],
        Panel::new(
            content.x,
            (content.y + content.h - line).max(content.y),
            content.w,
            line,
        ),
        skin.dim,
    );
}

/// The settings panel: the whole surface under the title strip while it is up.
///
/// A rail of section names down the left and the chosen section beside it. Each
/// row is two columns: the label says what a setting is called in the file, so
/// the panel doubles as the documentation for editing that file by hand, and the
/// value sits down the right where it can be scanned. Only one thing here is a
/// widget in the usual sense, the slider on a setting with a range; everything
/// else is text, and what makes it a control is that the arrow keys and a click
/// on it change it.
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

    // The heading, and which section it is showing. The rail says that as well,
    // in the column below; this is the one line at the top that reads as a
    // sentence, so a window photographed mid-thought says where it was.
    let here = panel
        .section_names()
        .get(panel.chosen())
        .copied()
        .unwrap_or_default();
    say(
        scene,
        vec![
            Run::icon(icons::SETTINGS.to_string(), skin.bright),
            Run::tinted(" SETTINGS ", skin.bright),
            Run::tinted(clip(here, cols.saturating_sub(11)), skin.good),
        ],
        Panel::new(content.x, content.y, content.w, line),
        skin.bright,
    );

    // The mark and nothing under it. It stood on a filled block while the
    // pointer was on it, which is a button, and the panel is a takeover with no
    // other button on it: the mark is on the panel's own surface the way the
    // heading beside it is. What answers the pointer is the mark itself, in the
    // bad colour, because closing is the one thing here that throws work away
    // and it is the same colour the window uses for that everywhere else.
    let close = layout.settings_close;
    if close.w >= 1.0 {
        let ink = match frame.hot == Some(Hit::SettingsClose) {
            true => skin.bad,
            false => skin.bright,
        };
        say(
            scene,
            vec![Run::icon(icons::CLOSE.to_string(), ink)],
            close,
            ink,
        );
    }

    // The rail. The chosen section carries the band and the mark every list in
    // this window marks its current row with. The mark is always the focus
    // colour now: the rail is pressed rather than walked, so there is no second
    // state for it to be in.
    let list = layout.settings_list;
    let names = panel.section_names();
    for (index, at) in &layout.settings_rail {
        let Some(name) = names.get(*index) else {
            continue;
        };
        let chosen = *index == panel.chosen();
        if chosen {
            scene.rect(at.fill(skin.strip));
            scene.rect(Panel::new(at.x, at.y, MARK_W, at.h).fill(skin.edge_focus));
        }
        let tint = match (chosen, frame.hot == Some(Hit::SettingsSection(*index))) {
            (true, _) => skin.good,
            (false, true) => skin.bright,
            (false, false) => skin.dim,
        };
        let text_at = Panel::new(
            at.x + MARK_W + 3.0,
            at.y,
            (at.w - MARK_W - 3.0).max(1.0),
            line,
        );
        say(
            scene,
            vec![Run::tinted(
                clip(name, columns_in(text_at.w, column)),
                tint,
            )],
            text_at,
            tint,
        );
    }
    // The hairline between the rail and what it chose, so the two columns read
    // as two columns rather than as a list with a gap in it.
    if list.w >= 1.0 && list.h >= 1.0 {
        scene.rect(Panel::new(list.x - (GAP * 0.5).floor(), list.y, 1.0, list.h).fill(skin.edge));
    }

    let last_drawn = layout.settings_rows.last().map(|(index, _, _)| *index);
    for (index, side, row) in &layout.settings_rows {
        let Some(whole_row) = panel.row(*index) else {
            continue;
        };
        // A form row is drawn one half at a time, in the half's own box: what a
        // cell is and where it goes come from the same two functions the
        // placement and the hit test used.
        let paired = matches!(whole_row, SettingRow::Pair(_, _));
        let entry = crate::settings::cell(whole_row, *side);
        let label_w = settings_label_w(row.w, column);
        let label_cols = columns_in(label_w, column).saturating_sub(1);
        let on = *index == panel.cursor() && panel.on_row() && (!paired || panel.side() == *side);
        // The band is what says which half of a form the keyboard is in, so it
        // is drawn on the half rather than across the row. Not on a block of
        // text: a filled strip fourteen lines tall over a page of prose is a
        // highlight nobody can read through, and the mark down its edge says the
        // same thing.
        if on {
            if !matches!(entry, SettingRow::Paper(_)) {
                // A list being picked from carries the solid band the folder
                // picker's own session list carries; a form being read carries
                // the quiet strip, which is enough to say which row the keys
                // are on without shouting over the values beside it.
                let band = match entry {
                    SettingRow::Session { .. } => skin.picked,
                    _ => skin.strip,
                };
                scene.rect(row.fill(band));
            }
            scene.rect(Panel::new(row.x, row.y, MARK_W, row.h).fill(skin.edge_focus));
        }
        // The line between the two columns of a form, so the row reads as two
        // things rather than as one sentence that happens to have a gap in it.
        if paired && *side == Side::Right && row.h >= 2.0 {
            scene.rect(Panel::new(row.x, row.y + 2.0, 1.0, (row.h - 4.0).max(1.0)).fill(skin.edge));
        }
        // A hairline under every row but the last one on screen. A settings
        // panel is a form, and a form with nothing between its rows is one block
        // of text where a key belongs to whichever value it happens to be beside.
        // Not under the last: a line along the bottom of the list reads as the
        // edge of a box that is not there. Not under a heading either: the line
        // above a heading is what separates one group from the last, and a
        // second one under it would cut the heading off from its own group.
        let ruled = !matches!(entry, SettingRow::Heading(_));
        if ruled && last_drawn != Some(*index) && row.h >= 2.0 {
            scene.rect(Panel::new(row.x, row.y + row.h - 1.0, row.w, 1.0).fill(skin.edge));
        }
        let text_x = row.x + MARK_W + 3.0;
        let whole = Panel::new(text_x, row.y, (row.w - MARK_W - 3.0).max(1.0), line);
        // One column short of what the box holds, so the mark saying a line was
        // cut fits inside it: a clipped line whose ellipsis does not fit wraps
        // out of a one line box and reads as a sentence that simply stops.
        let whole_cols = columns_in(whole.w, column).saturating_sub(1);
        let label_room = Panel::new(text_x, row.y, label_w.max(1.0), line);
        let value_at = settings_control(*row, label_w, column);
        match entry {
            // A heading is the only thing on its row, and it gets the whole
            // width: `THE HIGHLIGHTER` is longer than a label column.
            //
            // In the same green the showing tab's line is drawn in. It was the
            // ordinary text tint, which is what the settings under it are
            // written in, so the groups of a long list did not separate from
            // their contents at a glance.
            //
            // Larger than the rows under it, and given the room for it by the
            // model: `settings::lines` says a heading is two rows of the small
            // text, which is what `place_settings` laid it out with and what the
            // scroll window counted. Its own column width, or a bigger font
            // clipped by the small text's columns would lose the end of a
            // heading that fits.
            SettingRow::Heading(name) => {
                let big = settings_heading_size(size);
                let big_line = Text::line_for(big);
                let big_column = column * big / size.max(1.0);
                let room = Panel::new(
                    text_x,
                    row.y + ((row.h - big_line) * 0.5).floor().max(0.0),
                    whole.w,
                    big_line.min(row.h),
                );
                scene.text(Text::rich(
                    vec![Run::tinted(
                        clip(name, columns_in(room.w, big_column).saturating_sub(1)),
                        skin.good,
                    )],
                    room,
                    big,
                    skin.good,
                ));
            }
            // Prose, and the one row that can be trouble. The whole width: a
            // sentence in a label column is two words and three dots.
            SettingRow::Note { text, bad } => {
                let tint = match bad {
                    true => skin.bad,
                    false => skin.dim,
                };
                say(
                    scene,
                    vec![Run::tinted(clip(text, whole_cols), tint)],
                    whole,
                    tint,
                );
            }
            // The row that names the columns of the table under it, written in
            // the same boxes the cells are written in: one function places both,
            // so a name cannot end up over the wrong column.
            SettingRow::Columns(names) => {
                let cells = settings_session_cells(*row, column);
                for (at, name) in cells.iter().zip(names) {
                    if at.w < column {
                        continue;
                    }
                    say(
                        scene,
                        vec![Run::tinted(
                            clip(name, columns_in(at.w, column).saturating_sub(1)),
                            skin.dim,
                        )],
                        Panel::new(at.x, row.y, at.w, line),
                        skin.dim,
                    );
                }
                settings_session_lines(scene, &cells, *row, skin.edge);
            }
            // One saved conversation, cell by cell under the header. The band
            // under the cursor is the solid one the session list in the folder
            // picker uses rather than the quiet strip the rest of the panel
            // uses: this is a list being picked from, not a form being read.
            SettingRow::Session { cells, .. } => {
                let ink = match on {
                    true => skin.picked_ink,
                    false => skin.body,
                };
                let boxes = settings_session_cells(*row, column);
                for (at, text) in boxes.iter().zip(cells) {
                    if at.w < column {
                        continue;
                    }
                    say(
                        scene,
                        vec![Run::tinted(
                            clip(text, columns_in(at.w, column).saturating_sub(1)),
                            ink,
                        )],
                        Panel::new(at.x, row.y, at.w, line),
                        ink,
                    );
                }
                settings_session_lines(scene, &boxes, *row, skin.edge);
                // The trash. A mark rather than a word, in the colour this
                // window uses for everything that throws work away; pressed
                // once it says so and waits for the second press, and the
                // footer says which conversation would go with it.
                for (at, box_) in &layout.settings_removes {
                    if at != index {
                        continue;
                    }
                    let armed = panel.arming() == Some(*index);
                    scene.rect(panel_fill(*box_, skin.input));
                    if frame.hot == Some(Hit::SettingsRemove(*index)) {
                        scene.rect(box_.fill(skin.hot));
                    }
                    scene.rect(panel_edge(
                        *box_,
                        match armed {
                            true => skin.close_hot,
                            false => skin.edge,
                        },
                    ));
                    let room = Panel::new(
                        box_.x + INPUT_PAD,
                        box_.y,
                        (box_.w - INPUT_PAD * 2.0).max(1.0),
                        box_.h,
                    );
                    let run = match armed {
                        true => Run::tinted("sure?", skin.bad),
                        false => Run::icon(icons::TRASH.to_string(), skin.bad),
                    };
                    say(scene, vec![run], room, skin.bad);
                }
            }
            // A reading's value starts in the same column a control does and
            // runs to the end of the row rather than stopping where a value
            // would: most of them are paths, and a path in a value column is
            // three dots.
            SettingRow::Reading { label, value } => {
                say(
                    scene,
                    vec![Run::tinted(clip(label, label_cols), skin.dim)],
                    label_room,
                    skin.dim,
                );
                let value_room = Panel::new(
                    value_at.x,
                    row.y,
                    (row.x + row.w - value_at.x).max(1.0),
                    line,
                );
                say(
                    scene,
                    vec![Run::tinted(
                        clip(value, columns_in(value_room.w, column)),
                        skin.body,
                    )],
                    value_room,
                    skin.body,
                );
            }
            // The one field: text, and while it is being typed into, what has
            // been typed with a caret after it.
            SettingRow::Field { key, value } => {
                let tint = if on { skin.bright } else { skin.body };
                say(
                    scene,
                    vec![Run::tinted(clip(key, label_cols), tint)],
                    label_room,
                    tint,
                );
                let typing = on.then(|| panel.editing()).flatten();
                let (shown, ink) = match (typing, value.is_empty()) {
                    (Some(typed), _) => (typed.to_string(), skin.bright),
                    (None, true) => (String::from(crate::settings::UNSET), skin.dim),
                    (None, false) => (value.clone(), skin.bright),
                };
                // The box a field is typed into, drawn as a box: the prompt's
                // own fill and an outline round it. Without one an editable row
                // looked exactly like a reading, and the only way to find out
                // which was which was to press one.
                scene.rect(panel_fill(value_at, skin.input));
                if frame.hot == Some(Hit::SettingsValue(*index, *side)) {
                    scene.rect(value_at.fill(skin.hot));
                }
                scene.rect(panel_edge(
                    value_at,
                    match typing.is_some() || on {
                        true => skin.edge_focus,
                        false => skin.edge,
                    },
                ));
                // The end of it rather than the start: what changes in an
                // endpoint is the port and the path, and a URL clipped from the
                // left keeps the half being typed on screen. Inside the box
                // rather than on its stroke, and two columns short of it, so the
                // caret after the text is inside the box as well.
                let inside = Panel::new(
                    value_at.x + INPUT_PAD,
                    value_at.y,
                    (value_at.w - INPUT_PAD * 2.0).max(1.0),
                    value_at.h,
                );
                let shown = tail(&shown, SETTING_VALUE_COLUMNS.saturating_sub(2));
                say(scene, vec![Run::tinted(shown.clone(), ink)], inside, ink);
                // The same caret the prompt draws, in the same colour: a block
                // character would be a glyph that can be missing, and a missing
                // glyph draws as nothing at all.
                if typing.is_some() {
                    scene.rect(
                        Panel::new(
                            inside.x + shown.chars().count() as f32 * column,
                            inside.y,
                            2.0,
                            line,
                        )
                        .fill(skin.caret),
                    );
                }
            }
            SettingRow::Setting { key, value, .. } => {
                let tint = if on { skin.bright } else { skin.body };
                say(
                    scene,
                    vec![Run::tinted(clip(key, label_cols), tint)],
                    label_room,
                    tint,
                );
                let track = layout
                    .settings_tracks
                    .iter()
                    .find(|(at, half, _)| at == index && half == side)
                    .map(|(_, _, track)| *track);
                let value = panel.preview(*index, *side).unwrap_or(value);
                match track {
                    // A setting with a range is a position on a track, with the
                    // number it is at beside it. Nineteen presses of an arrow
                    // key from one end of opacity to the other is not a control.
                    Some(track) if track.w >= 1.0 => {
                        if frame.hot == Some(Hit::SettingsSlider(*index, *side)) {
                            scene.rect(track.fill(skin.hot));
                        }
                        let thick = (line * 0.3).floor().max(2.0);
                        let up = ((line - thick) * 0.5).floor();
                        let at = panel.fraction(*index, *side).unwrap_or(0.0);
                        scene.rect(
                            Panel::new(track.x, track.y + up, track.w, thick)
                                .fill(skin.gauge_track),
                        );
                        scene.rect(
                            Panel::new(track.x, track.y + up, (track.w * at).floor(), thick)
                                .fill(skin.gauge),
                        );
                        // The grip: a bar at the position, tall enough to press.
                        let grip = CARET_W;
                        scene.rect(
                            Panel::new(
                                track.x + ((track.w - grip) * at).floor(),
                                track.y + 1.0,
                                grip,
                                (line - 2.0).max(1.0),
                            )
                            .fill(skin.edge_focus),
                        );
                        let number = Panel::new(
                            track.x + track.w + column,
                            row.y,
                            (value_at.w - track.w - column).max(1.0),
                            line,
                        );
                        say(
                            scene,
                            vec![Run::tinted(
                                clip(value, SETTING_TRACK_VALUE_COLUMNS.saturating_sub(1)),
                                skin.bright,
                            )],
                            number,
                            skin.bright,
                        );
                    }
                    // The value of a setting that can change is drawn as the
                    // control it is: a box with an outline round it, accent
                    // tinted, and lit under the pointer the way a window button
                    // is. The same box the field gets, because they answer the
                    // same press: anything with an outline here can be changed.
                    _ => {
                        scene.rect(panel_fill(value_at, skin.input));
                        if frame.hot == Some(Hit::SettingsValue(*index, *side)) {
                            scene.rect(value_at.fill(skin.hot));
                        }
                        scene.rect(panel_edge(
                            value_at,
                            match on {
                                true => skin.edge_focus,
                                false => skin.edge,
                            },
                        ));
                        say(
                            scene,
                            vec![Run::tinted(
                                clip(value, SETTING_VALUE_COLUMNS.saturating_sub(2)),
                                skin.bright,
                            )],
                            Panel::new(
                                value_at.x + INPUT_PAD,
                                value_at.y,
                                (value_at.w - INPUT_PAD * 2.0).max(1.0),
                                value_at.h,
                            ),
                            skin.bright,
                        );
                    }
                }
            }
            // One row of the palette grid: a block of each colour with what it
            // colours written beside it, and a hairline between the cells so the
            // row reads as three things and not as one sentence.
            SettingRow::Swatches(cells) => {
                for (cell, colour) in cells.iter().enumerate() {
                    let at = settings_cell(*row, cell, cells.len());
                    let held = panel.picked() == Some((*index, cell));
                    if held {
                        scene.rect(at.fill(skin.strip));
                    }
                    if frame.hot == Some(Hit::SettingsSwatch(*index, cell)) {
                        scene.rect(at.fill(skin.hot));
                    }
                    if cell > 0 {
                        scene.rect(
                            Panel::new(at.x, at.y + 2.0, 1.0, (at.h - 4.0).max(1.0))
                                .fill(skin.edge),
                        );
                    }
                    let side = (line * 0.6).floor().max(2.0);
                    let up = ((at.h - side) * 0.5).floor().max(0.0);
                    let block = Panel::new(at.x + INPUT_PAD, at.y + up, side, side);
                    scene.rect(block.fill(swatch(colour.rgb)));
                    // An outline round the block as well: a swatch of the
                    // panel's own colour on the panel would have no edges at all.
                    scene.rect(block.fill(skin.edge).stroke(1.0));
                    let words = Panel::new(
                        block.x + side + column,
                        at.y,
                        (at.x + at.w - block.x - side - column).max(1.0),
                        line.min(at.h),
                    );
                    let ink = match held {
                        true => skin.bright,
                        false => skin.body,
                    };
                    say(
                        scene,
                        vec![Run::tinted(
                            clip(colour.about, columns_in(words.w, column).saturating_sub(1)),
                            ink,
                        )],
                        words,
                        ink,
                    );
                }
            }
            // One skill or one server: what it is called, what is under it, and
            // the two controls at the right. A row that is off is drawn in the
            // quiet tint the whole way across, so a list of skills says which
            // ones the agent will actually load without anything having to be
            // read.
            SettingRow::Entry(entry) => {
                let tint = match (entry.on, on) {
                    (true, true) => skin.bright,
                    (true, false) => skin.body,
                    (false, _) => skin.dim,
                };
                // The name stops a column short of the controls: a name drawn
                // under the toggle is a name whose end nobody can read.
                let controls = layout
                    .settings_toggles
                    .iter()
                    .chain(layout.settings_removes.iter())
                    .filter(|(at, _)| at == index)
                    .map(|(_, at)| at.x)
                    .fold(f32::INFINITY, f32::min);
                let name_w = match controls.is_finite() {
                    true => (controls - column - text_x).max(1.0),
                    false => whole.w,
                };
                say(
                    scene,
                    vec![Run::tinted(
                        clip(&entry.name, columns_in(name_w, column).saturating_sub(1)),
                        tint,
                    )],
                    Panel::new(text_x, row.y, name_w, line),
                    tint,
                );
                if row.h >= line * 2.0 {
                    say(
                        scene,
                        vec![Run::tinted(clip(&entry.under, whole_cols), skin.dim)],
                        Panel::new(text_x, row.y + line, whole.w, line),
                        skin.dim,
                    );
                }
                for (at, box_) in &layout.settings_toggles {
                    if at != index {
                        continue;
                    }
                    let ink = match entry.on {
                        true => skin.good,
                        false => skin.dim,
                    };
                    scene.rect(panel_fill(*box_, skin.input));
                    if frame.hot == Some(Hit::SettingsToggle(*index)) {
                        scene.rect(box_.fill(skin.hot));
                    }
                    scene.rect(panel_edge(
                        *box_,
                        match on {
                            true => skin.edge_focus,
                            false => skin.edge,
                        },
                    ));
                    say(
                        scene,
                        vec![Run::tinted(
                            match entry.on {
                                true => "on",
                                false => "off",
                            },
                            ink,
                        )],
                        Panel::new(
                            box_.x + INPUT_PAD,
                            box_.y,
                            (box_.w - INPUT_PAD * 2.0).max(1.0),
                            box_.h,
                        ),
                        ink,
                    );
                }
                // In the colour this window uses for everything that throws
                // work away, which is what a delete is.
                for (at, box_) in &layout.settings_removes {
                    if at != index {
                        continue;
                    }
                    // Pressed once, it says so and waits for the second press.
                    // The footer says what would go with it.
                    let armed = panel.arming() == Some(*index);
                    scene.rect(panel_fill(*box_, skin.input));
                    if frame.hot == Some(Hit::SettingsRemove(*index)) {
                        scene.rect(box_.fill(skin.hot));
                    }
                    scene.rect(panel_edge(
                        *box_,
                        match armed {
                            true => skin.close_hot,
                            false => skin.edge,
                        },
                    ));
                    say(
                        scene,
                        vec![Run::tinted(
                            match armed {
                                true => "sure?",
                                false => "uninstall",
                            },
                            skin.bad,
                        )],
                        Panel::new(
                            box_.x + INPUT_PAD,
                            box_.y,
                            (box_.w - INPUT_PAD * 2.0).max(1.0),
                            box_.h,
                        ),
                        skin.bad,
                    );
                }
            }
            // A block of text under a title of its own: what the agent is really
            // told, and where that came from. Rendered as Markdown, the way the
            // column beside the skills list renders a `SKILL.md`, because both of
            // these are Markdown and printing the marks would be showing the
            // file rather than the instructions.
            SettingRow::Paper(paper) => {
                let held = crate::settings::PAPER_LINES;
                say(
                    scene,
                    vec![Run::tinted(clip(&paper.title, whole_cols), skin.good)],
                    whole,
                    skin.good,
                );
                // How far down a block that is longer than its box has been
                // read, on the title's own line: a page of prose with no way of
                // telling how much of it is left is a box that reads as the whole
                // thing.
                if paper.body.len() > held {
                    let last = (paper.first + held).min(paper.body.len());
                    let counter = format!("{}-{} of {}", paper.first + 1, last, paper.body.len());
                    let wide = (counter.chars().count() as f32 + 1.0) * column;
                    if wide < whole.w * 0.5 {
                        say(
                            scene,
                            vec![Run::tinted(counter, skin.dim)],
                            Panel::new(whole.x + whole.w - wide, whole.y, wide, line),
                            skin.dim,
                        );
                    }
                }
                let under = match paper.bad {
                    true => skin.bad,
                    false => skin.dim,
                };
                if row.h >= line * 2.0 {
                    say(
                        scene,
                        vec![Run::tinted(clip(&paper.under, whole_cols), under)],
                        Panel::new(text_x, row.y + line, whole.w, line),
                        under,
                    );
                }
                // Where the fences stand after everything scrolled off, so a
                // block that starts inside a code block is drawn as code.
                let mut fence = crate::markdown::fence_after(
                    paper.body.iter().take(paper.first).map(String::as_str),
                );
                for (step, text) in paper.body.iter().skip(paper.first).take(held).enumerate() {
                    let at = Panel::new(text_x, row.y + (step as f32 + 2.0) * line, whole.w, line);
                    // Cut off at the bottom of the row rather than drawn under
                    // it: the last block on a short list is clipped by the list.
                    if at.y + line > row.y + row.h {
                        break;
                    }
                    let mut runs = Vec::new();
                    crate::markdown::line(&clip(text, whole_cols), &mut fence, skin, &mut runs);
                    scene.text(Text::rich(runs, at, size, skin.body));
                }
            }
            // Never drawn: a half of a form is what is drawn, and `cell` hands
            // one back rather than the pair it came out of. A pair inside a pair
            // is the only way here, and nothing builds one.
            SettingRow::Pair(_, _) => {}
        }
    }
    // The column beside that list: the entry under the cursor, rendered the way
    // the transcript renders what the model writes, because a `SKILL.md` is
    // Markdown and showing it with its marks in would be showing the file
    // rather than the skill.
    let doc = layout.settings_doc;
    if doc.w >= 1.0 && doc.h >= 1.0 {
        scene.rect(Panel::new(doc.x - (GAP * 0.5).floor(), doc.y, 1.0, doc.h).fill(skin.edge));
        let inside = Panel::new(doc.x + PAD, doc.y, (doc.w - PAD).max(1.0), doc.h);
        let doc_cols = columns_in(inside.w, column).saturating_sub(1);
        let doc_rows = Text::rows_for(size, inside.h);
        if let Some(entry) = panel.showing() {
            let first = panel.doc_first(doc_rows);
            // Where the fences stand after everything scrolled off, so a column
            // that starts inside a code block is drawn as code.
            let mut fence =
                crate::markdown::fence_after(entry.doc.iter().take(first).map(String::as_str));
            for (step, text) in entry.doc.iter().skip(first).take(doc_rows).enumerate() {
                let mut runs = Vec::new();
                crate::markdown::line(&clip(text, doc_cols), &mut fence, skin, &mut runs);
                scene.text(Text::rich(
                    runs,
                    Panel::new(inside.x, inside.y + step as f32 * line, inside.w, line),
                    size,
                    skin.body,
                ));
            }
            if entry.doc.is_empty() {
                say(
                    scene,
                    vec![Run::tinted(
                        clip("nothing to show: this one has no SKILL.md", doc_cols),
                        skin.dim,
                    )],
                    Panel::new(inside.x, inside.y, inside.w, line),
                    skin.dim,
                );
            }
        }
    }
    scrollbar(
        scene,
        skin,
        layout.settings,
        panel.thumb(layout.settings_capacity(size)),
    );

    // What the keys do to the row under the cursor, which swatch was pressed, or
    // why the last change did not land. A panel that writes a file has to say
    // when the file refused.
    let (foot, tint) = match panel.trouble() {
        Some(why) => (clip(why, cols), skin.bad),
        None => (clip(&panel.says(), cols), skin.dim),
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
    scene.text(
        Text::rich(
            vec![
                Run::tinted(marker.to_string(), skin.dim),
                Run::tinted(frame.prompt.text(), skin.bright),
            ],
            box_,
            frame.body_size,
            skin.bright,
        )
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
fn rows_in(box_: Panel, line: f32) -> usize {
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
/// One line number, in exactly `chrome` columns.
///
/// Fixed width because the wrap is: the text of a file line starts `chrome`
/// columns in on its first row and on every row it continues onto, so a number
/// that took one column more would push the first row one character out from
/// the rows under it. Three digits and a blank is the usual answer; a file long
/// enough spends the blank, and one longer still says it was cut rather than
/// quietly showing a different line's number.
fn file_number(number: u32, chrome: usize) -> String {
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

/// The same, from the other end: the last `chars` characters, with a mark where
/// the front was cut off.
///
/// For a value whose end is the part that changes. A URL clipped from the left
/// keeps the port and the path, which is what somebody typing one is looking at;
/// clipped from the right it says `http://localho…` on every endpoint there is.
fn tail(text: &str, chars: usize) -> String {
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
            left_width: [LEFT_WIDTH; 2],
            top_height: [TOP_HEIGHT; 2],
            settings_rail: SETTINGS_RAIL,
            popup: None,
        }
    }

    /// The same with both halves of the grid breaking in the same place, which
    /// is the window nobody has dragged one half away from the other.
    fn split_shape(dock: &Dock, left_width: f32, top_height: f32) -> Shape<'_> {
        halves_shape(dock, [left_width; 2], [top_height; 2])
    }

    /// The same with each half put where the test wants it.
    fn halves_shape(dock: &Dock, left_width: [f32; 2], top_height: [f32; 2]) -> Shape<'_> {
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

    /// Every dot of the orb in a scene.
    ///
    /// A rectangle a few pixels across inside the title strip is one. It used to
    /// be "a rectangle in the strip with a corner radius", which stopped finding
    /// the resting orb the day its dots became squares; size is what both states
    /// share. Nothing else drawn up there is small: the strip's own fill and the
    /// context gauge run the width of the window, and a window button is thirty
    /// pixels wide.
    fn discs_of(scene: &Scene) -> Vec<&Rect> {
        scene
            .rects
            .iter()
            .filter(|rect| {
                let [_, y, w, h] = rect.xywh();
                w <= 4.0 && y + h <= TITLE_H
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

    /// Every cell that has tabs in it, which is not every cell of the grid: the
    /// window opens with the one under the conversation empty, and an empty cell
    /// has no strip and no body because its room went to its neighbour.
    fn occupied(dock: &Dock) -> Vec<Space> {
        Space::ALL
            .into_iter()
            .filter(|space| !dock.slot(*space).is_empty())
            .collect()
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
            // And it is not the focus edge, which is the other coloured line a
            // pane can carry and is the theme's accent rather than the green.
            assert_ne!(out.skin.tab_accent, out.skin.edge_focus);
        }
    }

    /// The accent turns the corner instead of stopping in mid air: on down the
    /// cut, down the right edge, and back along the foot, at half its own
    /// weight. Half is the whole point, so the tab that is showing is still told
    /// by weight and not by having a border at all.
    ///
    /// Every box is read off the tab and the accent rather than off the
    /// constants, so a layout change moves the assertion with the drawing.
    #[test]
    fn the_showing_tab_s_border_follows_the_cut_at_half_the_accent() {
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
            // Half the line it continues, whatever that line is thick.
            let weight = accent.xywh()[3] * 0.5;
            assert!(
                (weight - TAB_EDGE_H).abs() < 0.01,
                "{space:?}: the border is {weight} against an accent of {:?}",
                accent.xywh()[3]
            );
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
            for step in 0..=((cut - weight) as usize) {
                let at = step as f32;
                assert!(
                    drawn(Panel::new(right - cut + at, tab.y + at, weight, weight)),
                    "{space:?}: the cut has no line at step {step}"
                );
            }
            // On down the right edge, from where the cut leaves off to the foot.
            assert!(
                drawn(Panel::new(
                    right - weight,
                    tab.y + cut,
                    weight,
                    tab.h - cut
                )),
                "{space:?}: the border does not run down the right edge"
            );
            // And back along the foot, as far as the cut reached, on the last
            // row inside the tab. A row lower is the pane's own top edge, which
            // is the rule under the strip that item 12 took away.
            assert!(
                drawn(Panel::new(
                    right - cut,
                    tab.y + tab.h - weight,
                    cut,
                    weight
                )),
                "{space:?}: the border does not turn along the foot"
            );
            assert!(
                (tab.y + tab.h - placed.body.y).abs() < 0.01,
                "{space:?}: the tab does not sit on the pane, so its foot is not the seam"
            );
            checked += 1;
        }
        assert_eq!(checked, 3, "only {checked} spaces had a showing tab");
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
        let state = busy_state();
        let mut seen = Vec::new();
        for view in View::ALL {
            let mut dock = Dock::new();
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
    /// inside the corner rather than in it: a hairline drawn on the line clears by
    /// `cut - 1`, one pixel short of the empty triangle, which is what a border
    /// following an edge means. Anything thicker than a hairline, and anything
    /// further in than one pixel, is still in the corner and still fails.
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
                    clear >= cut - 1.01,
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
    /// rather than looked up, so a corner in a colour of its own is caught.
    #[test]
    fn a_pane_s_cut_corner_is_bordered_like_its_other_sides() {
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
                .find(|r| r.xywh() == [panel.x, panel.y, 1.0, panel.h])
                .unwrap_or_else(|| panic!("{space:?}: no left edge to take the colour from"));
            let hairline = side.xywh()[2];
            let at = |box_: [f32; 4]| {
                out.scene
                    .rects
                    .iter()
                    .any(|r| r.xywh() == box_ && r.rgba() == side.rgba())
            };
            // From where a top edge would have stopped down to where the right
            // edge starts, one hairline square per pixel.
            for step in 0..=((cut - hairline) as usize) {
                let a = step as f32;
                assert!(
                    at([right - cut + a, panel.y + a, hairline, hairline]),
                    "{space:?}: the cut has no line at step {step}"
                );
            }
            // Which is the row the right edge already starts on, so the two are
            // one border and not two.
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
            let mark = topped(&out, *row, row.h, out.skin.tab_accent);
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
            out.layout.content(Space::TopLeft),
            out.layout.placed(Space::TopLeft).body
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

    /// A file, open, holding lines long enough that the pane has to wrap them,
    /// each with the line number a file's rows carry.
    fn a_wrapped_file(lines: &[(u32, &str)]) -> (State, Vec<String>) {
        let paths = ["src/main.rs"];
        let mut state = touched(&paths);
        for (number, text) in lines {
            state.files[0]
                .pane
                .push(crate::state::Line::new(*text, Tone::Body).at(*number));
        }
        (state, labels(&paths))
    }

    /// The rows a file line wraps onto start under its text, not under its line
    /// number.
    ///
    /// The gutter is four columns the text never gets, and it is written once,
    /// on the first row of the line. The rows under it used to start at the
    /// left edge of the box, so they held four characters more than the
    /// arithmetic that counts the rows, places the caret and draws the band
    /// budgets for, and everything below the first row of a wrapped file line
    /// was four columns out. Every row is the same width now: the gutter, then
    /// exactly the characters the pane says that row is showing.
    #[test]
    fn a_wrapped_file_line_continues_under_its_own_text() {
        let long = "let total = numbers.iter().filter(|n| **n > 0).map(|n| n * 2).sum::<i64>(); \
                    // and a comment on the end of it with plenty of blanks to break at";
        let (state, names) = a_wrapped_file(&[(7, long), (8, "fn main() {}"), (9, "")]);
        let out = render(
            &state,
            1400.0,
            900.0,
            &Dock::new(),
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let body = out.layout.file_diff;
        let (cols, chrome) = text_columns(View::Files, body, 8.0);
        let pane = &state.files[0].pane;
        let rows = out.layout.rows(body, 13.0);
        let window = pane.window(rows, cols);
        assert_eq!(window.skip, 0, "the file is meant to fit in the pane");

        let text = out
            .scene
            .texts
            .iter()
            .find(|text| text.at == body.inset(PAD) && text.wrap_cols.is_some())
            .expect("the file draws its text");
        assert_eq!(text.wrap_cols, Some(cols), "the box wraps in the columns the pane counts");
        assert_eq!(text.wrap_indent, chrome, "the box does not keep the gutter clear");

        // The rows the renderer will lay out, which is what the reader sees.
        let laid: String =
            noob_draw::Run::wrapped_under(&text.runs, cols, text.wrap_break, text.wrap_indent)
                .iter()
                .map(|run| run.text.as_str())
                .collect();
        let drawn: Vec<Vec<char>> = laid.split('\n').map(|row| row.chars().collect()).collect();

        let mut wrapped = 0;
        let mut previous = None;
        for (row, on_screen) in drawn.iter().enumerate().take(rows) {
            let Some((line, start)) = pane.spot_in(rows, cols, row, 0) else {
                break;
            };
            let (same, end) = pane
                .spot_in(rows, cols, row, cols + 9)
                .expect("the row a moment ago is still a row");
            assert_eq!(same, line, "row {row} lands on two different lines");
            let source = pane.line(line).expect("a row of a line the pane holds");
            let shown: Vec<char> = source.text.chars().take(end).skip(start).collect();

            let (gutter, after) = on_screen.split_at(chrome.min(on_screen.len()));
            assert_eq!(after, shown, "screen row {row} is not the characters the pane says");
            assert!(
                on_screen.len() <= chrome + cols,
                "screen row {row} is wider than the box"
            );
            match previous == Some(line) {
                // A row that continues a line is blank where the number was.
                true => {
                    assert!(
                        gutter.iter().all(|ch| *ch == ' ') && gutter.len() == chrome,
                        "row {row} continues line {line} under {gutter:?} instead of the text"
                    );
                    wrapped += 1;
                }
                // The first row of a line carries the number, once. A line the
                // file did not number is blank there, and an empty line with
                // no number has nothing on it at all.
                false => {
                    let head: String = gutter.iter().collect();
                    let want = match (source.number, source.text.is_empty()) {
                        (Some(number), _) => file_number(number, chrome),
                        (None, false) => " ".repeat(chrome),
                        (None, true) => String::new(),
                    };
                    assert_eq!(head, want, "row {row} of line {line} has the wrong gutter");
                }
            }
            previous = Some(line);
        }
        assert!(wrapped >= 2, "only {wrapped} rows continued a wrapped line");
    }

    /// A number wider than the gutter is still exactly the gutter, because the
    /// width of the gutter is what every row of the line is indented by.
    #[test]
    fn a_line_number_is_written_in_exactly_the_columns_the_gutter_has() {
        assert_eq!(file_number(7, GUTTER), "007 ");
        assert_eq!(file_number(120, GUTTER), "120 ");
        // Past three digits the blank goes, and past four the number says it
        // was cut rather than reading as another line's number.
        assert_eq!(file_number(1204, GUTTER), "1204");
        assert_eq!(file_number(12040, GUTTER), "120\u{2026}");
        for number in [1u32, 9, 10, 999, 1000, 9999, 10_000, 999_999] {
            assert_eq!(
                file_number(number, GUTTER).chars().count(),
                GUTTER,
                "line {number} does not fill the gutter"
            );
        }
    }

    /// The band over a wrapped file line covers the glyphs on every row of it,
    /// including the rows that continue it.
    ///
    /// The band was measured in the full width of the box while the text was
    /// laid out in the width the gutter leaves, so it started four columns left
    /// of the first character it was highlighting and, on a continuation row,
    /// covered the indent instead of the text.
    #[test]
    fn the_band_over_a_wrapped_file_line_covers_the_glyphs() {
        let long = "let total = numbers.iter().filter(|n| **n > 0).map(|n| n * 2).sum::<i64>(); \
                    // and a comment on the end of it with plenty of blanks to break at";
        let (mut state, names) = a_wrapped_file(&[(7, long)]);
        let files = names.iter().map(String::as_str).collect::<Vec<_>>();
        let line = state.files[0].pane.last() - 1;
        let chars = long.chars().count();
        let selection = {
            let mut selection =
                crate::select::Selection::new(View::Files, crate::select::Spot::new(line, 0));
            selection.extend(crate::select::Spot::new(line, chars));
            selection
        };
        state.selection = Some(selection);

        let dock = Dock::new();
        let shape = shape(&dock, &files);
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

        let body = layout.file_diff;
        let content = body.inset(PAD);
        let (cols, chrome) = text_columns(View::Files, body, 8.0);
        let pane = &state.files[0].pane;
        let rows = layout.rows(body, 13.0);
        let (top, height) = pane.band_of(rows, cols, line).expect("the line is on screen");
        let spans = pane.rows_of_line(line, cols);
        assert!(height > 1, "the line under test does not wrap");

        let mut bands: Vec<[f32; 4]> = scene
            .rects
            .iter()
            .filter(|rect| rect.rgba() == skin.select)
            .map(|rect| rect.xywh())
            .collect();
        bands.sort_by(|a, b| a[1].partial_cmp(&b[1]).unwrap());
        assert_eq!(bands.len(), height, "one band per row of the line: {bands:?}");

        let line_h = Text::line_for(13.0);
        for (i, band) in bands.iter().enumerate() {
            let span = spans[i];
            // Past the gutter on the first row and past the indent under it on
            // the rest, and exactly as wide as the characters drawn there.
            assert!(
                (band[0] - (content.x + chrome as f32 * 8.0)).abs() < 0.01,
                "row {i} of the band starts at {} and the text at {}",
                band[0],
                content.x + chrome as f32 * 8.0
            );
            assert!(
                (band[2] - span.len() as f32 * 8.0).abs() < 0.01,
                "row {i} of the band is {} wide over {} characters",
                band[2],
                span.len()
            );
            assert!(
                (band[1] - (content.y + (top + i) as f32 * line_h)).abs() < 0.01,
                "row {i} of the band is on the wrong row"
            );
        }
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
        dock.move_view(View::Output, Space::TopRight);
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        assert_eq!(out.layout.placed(Space::TopLeft).strip.w, 0.0);
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
        let layout = Layout::compute(1400.0, 900.0, &split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT));
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
                &halves_shape(&dock, [0.54; 2], [TOP_HEIGHT, ratio]),
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
        let layout = Layout::compute(1400.0, 900.0, &split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT));
        let column_floor = layout.column_divider[0].floor;
        let row_floor = layout.row_divider[1].floor;
        assert!(column_floor > 0.0 && row_floor > 0.0);

        for x in [-4000.0, -1.0, 700.0, 1401.0, 9000.0] {
            let ratio = layout.column_ratio_at(0, x);
            let moved = Layout::compute(1400.0, 900.0, &split_shape(&dock, ratio, TOP_HEIGHT));
            let (left_w, _) = box_of(&moved, Space::TopLeft);
            let (right_w, _) = box_of(&moved, Space::TopRight);
            assert!(left_w >= column_floor, "{x}: the left column is {left_w}");
            assert!(right_w >= column_floor, "{x}: the right column is {right_w}");
        }
        for y in [-4000.0, -1.0, 500.0, 901.0, 9000.0] {
            let ratio = layout.row_ratio_at(1, y);
            let moved = Layout::compute(1400.0, 900.0, &split_shape(&dock, LEFT_WIDTH, ratio));
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
        let band = out.layout.column_divider[0].band;
        assert!(band.w > GAP, "the band is no wider than the gap");
        let y = band.y + TAB_H + 20.0;
        for x in [band.x + 0.5, band.x + band.w * 0.5, band.x + band.w - 0.5] {
            assert_eq!(out.layout.hit(x, y), Some(Hit::ColumnDivider(0)), "at {x}");
        }
        assert_eq!(out.layout.hit(band.x - 1.0, y), Some(Hit::Body(Space::TopLeft)));
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
        dock.move_view(View::Output, Space::TopRight);
        let out = render(&busy_state(), 1200.0, 800.0, &dock, &[]);
        assert!(!out.layout.column_divider[0].live());
        assert!(out.layout.row_divider[1].live(), "the two right spaces are still there");
        assert!(box_of(&out.layout, Space::TopRight).0 > 1000.0, "the width was handed over");

        // Nothing in the bottom right: one space in that column, so no divider
        // across it, and the vertical one is still there.
        let mut dock = Dock::new();
        dock.move_view(View::Files, Space::TopRight);
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
        dock.move_view(View::Output, Space::TopRight);
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
        let open = Layout::compute(1200.0, 800.0, &split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT));
        assert!(open.column_divider[0].live() && open.row_divider[1].live());
        let band = open.column_divider[0].band;
        let (x, y) = (band.x + band.w * 0.5, band.y + TAB_H + 20.0);

        let picker = a_picker(&["src", "docs"], &[]);
        let panel = a_settings_panel(&Config::default());
        for (what, shape) in [
            ("shaded", Shape { shaded: true, ..split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT) }),
            ("picking", Shape { picker: Some(&picker), ..split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT) }),
            ("settings", Shape { settings: Some(&panel), ..split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT) }),
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
            for (left_width, top_height) in [(0.3, 0.3), (LEFT_WIDTH, TOP_HEIGHT), (0.7, 0.72)] {
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
            Layout::compute(1400.0, 900.0, &halves_shape(&dock, [LEFT_WIDTH; 2], [0.3, 0.7]));
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
        let start = Layout::compute(1400.0, 900.0, &split_shape(&dock, LEFT_WIDTH, TOP_HEIGHT));
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
        let moved = Layout::compute(1400.0, 900.0, &halves_shape(&dock, [LEFT_WIDTH; 2], ratios));
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
            &halves_shape(&dock, [LEFT_WIDTH; 2], [TOP_HEIGHT, ratios[1]]),
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
            Layout::compute(1400.0, 900.0, &halves_shape(&dock, [LEFT_WIDTH; 2], [0.3, 0.7]));
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
            Layout::compute(1400.0, 900.0, &halves_shape(&dock, [0.3, 0.7], [TOP_HEIGHT; 2]));
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
            &halves_shape(&dock, [LEFT_WIDTH; 2], halves),
        );
        dock.slot_mut(Space::TopLeft).folded = true;
        let folded = Layout::compute(
            1200.0,
            800.0,
            &halves_shape(&dock, [LEFT_WIDTH; 2], halves),
        );
        // While it is folded the pane is its strip and the one under it has the
        // rest, whatever the ratio says.
        assert_eq!(folded.placed(Space::TopLeft).body.h, 0.0);
        assert!(!folded.row_divider[0].live());

        dock.slot_mut(Space::TopLeft).folded = false;
        let opened = Layout::compute(
            1200.0,
            800.0,
            &halves_shape(&dock, [LEFT_WIDTH; 2], halves),
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
        // the way, which is any cell standing empty: the left column is one
        // pane over both of its cells, so the whole band there is a pair.
        let x = cells[0].x + cells[0].w * 0.5;
        assert_eq!(
            layout.landing(x, line_y + SPAN_BAND),
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
        // both, so the box says both. The conversation dropped back into the
        // cell it is already in leaves the cell under it empty, and it keeps the
        // whole column.
        let (rect, grid) = boxed_view(View::Output, Landing::In(Space::TopLeft, None));
        assert_eq!(
            rect,
            want(&[Space::TopLeft, Space::BottomLeft], grid),
            "the pane still spans the column it is alone in"
        );
        // And the same drop one cell down moves it there, where it still has
        // the column to itself.
        let (rect, grid) = boxed_view(View::Output, Landing::In(Space::BottomLeft, None));
        assert_eq!(
            rect,
            want(&[Space::TopLeft, Space::BottomLeft], grid),
            "nothing is left above it, so it still spans"
        );
        // A second pane in that column is what takes the span apart, and then
        // the box is one cell.
        let (rect, grid) = boxed(Landing::In(Space::BottomLeft, None));
        assert_eq!(rect, want(&[Space::BottomLeft], grid), "beside the conversation");
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

    /// A pane is drawn in exactly the columns its selection is counted in.
    ///
    /// Everything from the pointer to the clipboard runs over the stored text
    /// by character index, and that is only true if the row on screen holds the
    /// characters the pane says it does. No shaper wraps the way anything else
    /// counts, so the box names its column count and the rows are broken before
    /// shaping, by the same `text-geometry` call that measured them. Left to
    /// the shaper, the blank at each break was dropped from the screen, so
    /// every row below one began a character further along than the arithmetic
    /// said and a selection made there picked up spaces that were nowhere on
    /// screen. It only showed up once the window was narrow enough to wrap,
    /// which is why resizing looked like the trigger.
    ///
    /// The first version of this fix made both sides break on the column, which
    /// held the property and broke prose in the middle of words. Both sides
    /// break at the blank now, so the assertion is no longer `row * cols`: it
    /// is that the row a hit test lands on starts where the drawn row starts,
    /// that the drawn row runs to where the next one begins, and that the only
    /// thing between two rows is the single blank the break was spent on.
    ///
    /// Prose with blanks, deliberately, plus a word wider than the pane, a run
    /// of blanks and an empty line: every wrapped-pane test in this repo used
    /// `"x".repeat(n)`, which has no blank to break at and so wraps the same
    /// way whatever the rule is. That is why the whole corpus stayed green over
    /// the bug.
    #[test]
    fn a_pane_is_drawn_in_the_columns_its_selection_is_counted_in() {
        let mut state = busy_state();
        let dock = Dock::new();
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
            "a second line of prose with a good few blanks in it to break on, \
             long enough that it takes three rows of this pane to show and so \
             crosses two wrap points on the way down"
                .to_string(),
            String::new(),
            // A word with nowhere to break in it, wider than the pane whatever
            // the pane turns out to be, and a run of blanks either side.
            format!("a word   with   runs   of   blanks   {}   and no room to break it", "z".repeat(cols + 5)),
        ] {
            state.activity.say(text, Tone::Body);
        }
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

        // And the characters the hit test believes each row is showing. Both
        // ends of the row come out of the hit test itself: column zero is where
        // a drag from the left edge of the row starts, and a column past the
        // right edge is where a drag off the end of the row stops.
        let rows = layout.rows(panel, 13.0);
        let skip = state.activity.window(rows, cols).skip;
        let mut checked = 0;
        let mut wrapped = 0;
        let mut on_the_column = 0;
        let mut previous: Option<(usize, usize)> = None;
        for row in 0..rows {
            let Some((line, start)) = state.activity.spot_in(rows, cols, row, 0) else {
                break;
            };
            let (same, end) = state
                .activity
                .spot_in(rows, cols, row, cols + 9)
                .expect("the row a moment ago is still a row");
            assert_eq!(same, line, "row {row} lands on two different lines");
            let text = &state
                .activity
                .line(line)
                .expect("a row of a line the pane still holds")
                .text;
            let source: Vec<char> = text.chars().take(end).skip(start).collect();
            assert!(end - start <= cols, "row {row} is wider than the pane");
            assert_eq!(
                drawn[row + skip], source,
                "screen row {row} holds something other than what a selection there would copy"
            );
            // Nothing falls between two rows of one line but the single blank
            // the break was spent on, and nothing is on both.
            if let Some((before, was)) = previous.filter(|(_, was)| *was == line) {
                match start - before {
                    0 => {}
                    1 => assert_eq!(
                        text.chars().nth(before),
                        Some(' '),
                        "row {row} of line {was} skipped a character that is not a blank"
                    ),
                    gap => panic!("row {row} of line {was} starts {gap} characters past the row above"),
                }
                wrapped += 1;
            }
            if end - start == cols && !source.contains(&' ') {
                on_the_column += 1;
            }
            previous = Some((end, line));
            checked += 1;
        }
        assert!(checked > 3, "only {checked} rows were on screen");
        assert!(wrapped > 2, "only {wrapped} rows continued a wrapped line");
        assert!(
            on_the_column > 0,
            "no row broke on the column, so the word wider than the pane was never drawn"
        );
        // The words really are whole: no row of prose ends mid-word with the
        // next one carrying on from it, unless the word had nowhere to break.
        let broken: usize = (0..rows)
            .filter_map(|row| {
                let (line, start) = state.activity.spot_in(rows, cols, row, 0)?;
                let (_, end) = state.activity.spot_in(rows, cols, row, cols + 9)?;
                let text = &state.activity.line(line)?.text;
                let after = text.chars().nth(end);
                Some(usize::from(
                    end > start && after.is_some_and(|ch| ch != ' ') && end - start == cols,
                ))
            })
            .sum();
        assert_eq!(
            broken, on_the_column,
            "a row broke inside a word that had a blank to break at"
        );
    }

    /// The time on an activity row is drawn in the dim tone, and the tag and
    /// the subject keep the call's own color.
    ///
    /// Item 42 put the clock on the row. Drawn in the call's color it would be
    /// eight characters shouting as loudly as the thing the row is about, which
    /// is what "does not fight the subject" rules out. The two runs together
    /// are still exactly the stored line, so nothing the pane measures moved.
    #[test]
    fn the_clock_on_an_activity_row_is_drawn_dim_and_the_subject_is_not() {
        let mut state = busy_state();
        state.day_zero = Some(14 * 3600 + 30 * 60);
        state.apply_at(
            noob_proto::Event::ToolStart {
                call_id: "c9".into(),
                name: "bash".into(),
                brief: "cargo build".into(),
                args: serde_json::json!({"cmd": "cargo build --release"}),
            },
            Some(9.0),
        );
        let out = render(&state, 1400.0, 900.0, &Dock::new(), &[]);
        let text = out
            .scene
            .texts
            .iter()
            .find(|text| text.runs.iter().any(|run| run.text.starts_with("14:30:09")))
            .expect("the activity pane draws the row's clock");
        let at = text
            .runs
            .iter()
            .position(|run| run.text.starts_with("14:30:09"))
            .expect("the reading is its own run");
        let (clock, rest) = (&text.runs[at], &text.runs[at + 1]);
        assert_eq!(clock.text, "14:30:09  ");
        assert_eq!(clock.color, Some(out.skin.dim));
        assert!(rest.text.contains("bash"), "{:?}", rest.text);
        assert!(rest.text.contains("cargo build --release"), "{:?}", rest.text);
        assert_eq!(
            rest.color,
            Some(out.skin.tone(Tone::Call(crate::state::Kind::Bash))),
            "the subject is drawn in the call's own color"
        );
        assert_ne!(rest.color, clock.color);
        // The row on screen is the row the pane holds, character for character:
        // the split is two runs of one line, not a line with something added.
        let held = state
            .activity
            .line(state.activity.last() - 1)
            .expect("the row is still there");
        assert_eq!(format!("{}{}", clock.text, rest.text), held.text);
    }

    /// The transcript is drawn as Markdown, and it is counted in the Markdown
    /// it drew.
    ///
    /// `markdown::line` eats the stars, the backticks and the hashes and turns
    /// `- ` into `• `, so the drawn line is shorter than the line the model
    /// wrote. While the pane measured the source, the tail of every marked-up
    /// line had no glyph to point at, and a wrapped one was out by however many
    /// marks were consumed above the pointer. The pane renders the line as it
    /// pushes it now, and its rows, its bands and its clipboard are all counted
    /// in that, so this asserts the whole of it at once: what the renderer lays
    /// out on a row is what a drag across that row would copy.
    #[test]
    fn the_transcript_is_counted_in_the_markdown_it_draws() {
        let mut state = busy_state();
        for text in [
            "## Notable Features",
            "- **read** a file, then `write` it back out again with __every__ \
             mark it came with, which is a good few marks and a good few blanks \
             and more than one row of any pane",
            "plain prose with no marks in it at all, long enough to wrap over a \
             row boundary and land some of itself on a second row",
            "",
            "1. a numbered item with `code` in it and a **bold** run near the end",
        ] {
            state.output.say(text, Tone::Body);
        }

        let dock = Dock::new();
        let shape = shape(&dock, &["a.rs"]);
        let layout = Layout::compute(1400.0, 900.0, &shape);
        let panel = layout.placed(Space::TopLeft).body;
        let cols = cols_of(panel, 8.0);
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
            selection: None,
            menu: None,
            picker: None,
            settings: None,
        });

        let text = scene
            .texts
            .iter()
            .find(|text| text.at == panel.inset(PAD))
            .expect("the output pane draws its text");
        assert_eq!(text.wrap_cols, Some(cols));
        let laid: String = noob_draw::Run::wrapped(&text.runs, cols, text.wrap_break)
            .iter()
            .map(|run| run.text.as_str())
            .collect();
        let drawn: Vec<Vec<char>> = laid.split('\n').map(|row| row.chars().collect()).collect();

        let rows = layout.rows(panel, 14.0);
        let skip = state.output.window(rows, cols).skip;
        let mut checked = 0;
        let mut marked = 0;
        for row in 0..rows {
            let Some((line, start)) = state.output.spot_in(rows, cols, row, 0) else {
                break;
            };
            let (same, end) = state
                .output
                .spot_in(rows, cols, row, cols + 9)
                .expect("the row a moment ago is still a row");
            assert_eq!(same, line, "row {row} lands on two different lines");
            let held = state.output.line(line).expect("a row of a line still held");
            let source: Vec<char> = held.shown().chars().take(end).skip(start).collect();
            assert_eq!(
                drawn[row + skip], source,
                "screen row {row} holds something other than what a selection there would copy"
            );
            if held.shown() != held.text {
                marked += 1;
            }
            checked += 1;
        }
        assert!(checked > 6, "only {checked} rows were on screen");
        assert!(marked > 2, "only {marked} rows came off a line with marks in it");

        // And the row count of a marked-up line is the number of rows it is
        // actually drawn as, which is what keeps every row below it in step.
        let bullet = state.output.last() - 4;
        let held = state.output.line(bullet).expect("the bullet is held");
        assert!(held.shown() != held.text, "the bullet line had no marks");
        let counted = state.output.rows_of_line(bullet, cols);
        assert!(counted.len() > 1, "the bullet has to wrap");
        let on_screen: Vec<&Vec<char>> = drawn
            .iter()
            .skip(
                (0..rows)
                    .find(|row| {
                        state.output.spot_in(rows, cols, *row, 0).map(|(line, _)| line)
                            == Some(bullet)
                    })
                    .expect("the bullet is on screen")
                    + skip,
            )
            .take(counted.len())
            .collect();
        let rejoined: String = on_screen
            .iter()
            .map(|row| row.iter().collect::<String>())
            .collect::<Vec<String>>()
            .join(" ");
        assert_eq!(
            rejoined,
            held.shown(),
            "the rows drawn do not add back up to the line"
        );
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
                left_width: [LEFT_WIDTH; 2],
                top_height: [TOP_HEIGHT; 2],
                settings_rail: SETTINGS_RAIL,
                popup: None,
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
            left_width: [LEFT_WIDTH; 2],
            top_height: [TOP_HEIGHT; 2],
            settings_rail: SETTINGS_RAIL,
            popup: None,
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

    /// The header of the context pane is four labelled rows: the phase, and the
    /// three counts that say what this run has asked for. They are separated the
    /// way the phase row always was, a label in the dim tint and the reading
    /// beside it, which is what the counts never had as gauges.
    ///
    /// This asserted MODEL and PATH, which were the other two rows. The strip
    /// says where the agent is working now, so a PATH row here would be the same
    /// answer twice, and the model is on the settings panel.
    #[test]
    fn the_context_header_is_the_phase_and_the_three_counts() {
        let state = busy_state();
        let mut monitor = Monitor::new();
        monitor.sample(&state);
        let mut dock = Dock::new();
        dock.reveal(View::Context);
        let out = render_with(&state, 1400.0, 900.0, &dock, &[], &monitor, None);
        let text = text_of(&out.scene);
        let body = out.layout.placed(Space::TopRight).body;
        for wanted in [
            "PHASE",
            "TOTAL REQUESTS",
            "TOTAL TOOL CALLS",
            "LAST PREFILL",
            "CONTEXT",
        ] {
            assert!(text.contains(wanted), "{wanted:?} is not in the pane: {text}");
        }
        for gone in ["MODEL", "PATH", state.model.as_str()] {
            assert!(!text.contains(gone), "{gone:?} is still in the pane: {text}");
        }
        // Every label is drawn in its own box in the dim tint, with its reading
        // in a box of its own beside it. That separation is the whole of what
        // moving these three off the gauge list bought.
        for (label, reading) in [
            ("TOTAL REQUESTS", crate::state::thousands(state.requests as u64)),
            ("TOTAL TOOL CALLS", crate::state::thousands(state.tool_calls as u64)),
            ("LAST PREFILL", crate::state::thousands(state.last_prefill)),
        ] {
            let row: Vec<&noob_draw::Text> = out
                .scene
                .texts
                .iter()
                .filter(|t| body.contains(t.at.x, t.at.y))
                .filter(|t| {
                    t.runs.iter().any(|r| r.text == label || r.text == reading)
                })
                .collect();
            assert_eq!(row.len(), 2, "{label} is not a label and a reading");
            let (name, value) = (row[0], row[1]);
            assert_eq!(name.runs[0].text, label);
            assert_eq!(value.runs[0].text, reading, "{label}");
            assert_eq!(name.runs[0].color, Some(out.skin.dim), "{label} is not dim");
            assert_eq!(value.runs[0].color, Some(out.skin.body), "{label}'s reading");
            assert!(value.at.x > name.at.x, "{label} is not beside its reading");
            assert!((value.at.y - name.at.y).abs() < 0.01, "{label} is not on one row");
        }
        // And the reading above the header is a row of its own, not a line of
        // the title strip: the phase word is drawn in the pane, not up there.
        assert!(
            out.scene.texts.iter().any(|t| {
                body.contains(t.at.x, t.at.y)
                    && t.runs.iter().any(|r| r.text.contains(state.phase.word()))
            }),
            "the phase is not drawn in the context pane"
        );
    }

    /// The calls that failed are counted beside the calls that were made.
    ///
    /// That count was the whole of the DEBUG pane's first row and the pane is
    /// gone. It rides with the total rather than taking a row of its own,
    /// because the two are one reading, and it takes the fault colour once
    /// there is anything to say.
    #[test]
    fn the_failed_calls_are_counted_beside_the_total() {
        let mut state = busy_state();
        let mut dock = Dock::new();
        dock.reveal(View::Context);
        let reading = |state: &State| {
            let out = render_with(state, 1400.0, 900.0, &dock, &[], &Monitor::new(), None);
            let body = out.layout.placed(Space::TopRight).body;
            let run = out
                .scene
                .texts
                .iter()
                .filter(|t| body.contains(t.at.x, t.at.y))
                .flat_map(|t| t.runs.iter())
                .find(|r| r.text == crate::state::thousands(state.tool_calls as u64)
                    || r.text.starts_with(&format!(
                        "{} (",
                        crate::state::thousands(state.tool_calls as u64)
                    )))
                .expect("the tool call total is drawn")
                .clone();
            (run.text.clone(), run.color, out.skin)
        };

        // Nothing has failed: the total is the whole reading, in the ordinary
        // tint. A "(0 failed)" on every window is noise.
        assert_eq!(state.failed_calls, 0);
        let (text, tint, skin) = reading(&state);
        assert_eq!(text, crate::state::thousands(state.tool_calls as u64));
        assert_eq!(tint, Some(skin.body));

        state.apply(noob_proto::Event::ToolStart {
            call_id: "boom".into(),
            name: "bash".into(),
            brief: "no".into(),
            args: serde_json::json!({"cmd": "no"}),
        });
        state.apply(noob_proto::Event::ToolEnd {
            call_id: "boom".into(),
            summary: "refused".into(),
            elapsed_ms: 1,
            error: Some(noob_proto::ToolError {
                kind: "denied".into(),
                code: None,
                message: "outside the workspace".into(),
                detail: None,
                remedy: None,
            }),
        });
        let (text, tint, skin) = reading(&state);
        assert_eq!(
            text,
            format!("{} (1 failed)", crate::state::thousands(state.tool_calls as u64))
        );
        assert_eq!(tint, Some(skin.bad), "a failure does not read as a number");
        assert_ne!(skin.bad, skin.body);
    }

    /// While a turn is running the phase reads INFERRING in the bad colour, and
    /// at rest it is READY in the ordinary body tint.
    ///
    /// The colour is the point as much as the word: it is the one reading in the
    /// window that has to be answerable from the corner of the eye, because it
    /// is what says whether anything typed now is going anywhere.
    #[test]
    fn the_phase_reads_infering_in_the_bad_colour_while_a_turn_runs() {
        let mut dock = Dock::new();
        dock.reveal(View::Context);
        let phase_run = |state: &State| {
            let out = render_with(
                state,
                1400.0,
                900.0,
                &dock,
                &[],
                &Monitor::new(),
                None,
            );
            let body = out.layout.placed(Space::TopRight).body;
            let run = out
                .scene
                .texts
                .iter()
                .filter(|text| body.contains(text.at.x, text.at.y))
                .flat_map(|text| text.runs.iter())
                .find(|run| run.text.contains("READY") || run.text.contains("INFERRING"))
                .expect("the phase is drawn in the pane")
                .clone();
            (run.text.clone(), run.color, out.skin)
        };

        let (word, tint, skin) = phase_run(&busy_state());
        assert_eq!(word, "INFERRING");
        assert_eq!(tint, Some(skin.bad), "the busy word is not the bad colour");

        let mut ready = State::new();
        ready.apply(noob_proto::Event::SessionStart {
            id: "s1".into(),
            workspace: "/tmp".into(),
            model: "laguna-s21".into(),
            resumed: false,
        });
        let (word, tint, skin) = phase_run(&ready);
        assert_eq!(word, "READY");
        assert_eq!(tint, Some(skin.body));
        assert_ne!(skin.body, skin.bad);
    }

    /// A window whose every list is longer than any pane can hold: forty todos,
    /// twelve children with news each, and thirty calls that failed.
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
            (View::Agents, 1400.0, 900.0, "news 11"),
            // The monitor pane is five readings in a box that holds fewer.
            (View::Session, 900.0, 330.0, "DECODE"),
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
    /// layout, and the scene, at the default 14pt body size. Idle, with the
    /// clock at zero.
    fn render_prompt(
        prompt: &crate::prompt::Prompt,
        max_rows: usize,
    ) -> (Panel, Layout, Scene) {
        render_prompt_at(prompt, max_rows, &State::new(), 0.0)
    }

    /// The same with the window's state and the moment on its clock given, which
    /// is what the marker slot is drawn from.
    fn render_prompt_at(
        prompt: &crate::prompt::Prompt,
        max_rows: usize,
        state: &State,
        clock: f32,
    ) -> (Panel, Layout, Scene) {
        let dock = Dock::new();
        let skin = Skin::from(&Config::default());
        let mut shape = shape(&dock, &[]);
        shape.input_h = input_height(1200.0, 8.0, prompt.len(), Text::line_for(14.0), max_rows);
        let layout = Layout::compute(1200.0, 800.0, &shape);
        let scene = build(&Frame {
            state,
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
                &crate::icons::MINIMIZE.to_string(),
                &crate::icons::MAXIMIZE.to_string(),
                &crate::icons::CLOSE.to_string(),
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
        let mut dock = Dock::new();
        // Every view but one in the left space, which is more than its strip can
        // hold. The one left behind keeps the space split, so the strip is the
        // width it usually is rather than the whole window.
        for view in View::ALL.into_iter().filter(|v| *v != View::Files) {
            dock.move_view(view, Space::TopLeft);
        }
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
        // there, in the flyout as well as in the menu itself.
        let mut open = Menu::for_widget(at, View::Plan, Space::TopRight, false);
        open.toggle_widgets(&dock);
        assert_eq!(
            picked(&open),
            vec![
                Some(Item::Settings),
                None,
                Some(Item::Close),
                Some(Item::Widgets(true)),
            ],
            "the menu keeps its four rows and grows none"
        );
        let layout = with_menu(&dock, &open, 1400.0, 900.0);
        assert_eq!(
            layout
                .menu_list_rows
                .iter()
                .map(|(_, row)| {
                    let (x, y) = middle(*row);
                    match layout.hit(x, y) {
                        Some(Hit::MenuRow(index)) => open.pick(index),
                        other => panic!("{other:?} is not a row"),
                    }
                })
                .collect::<Vec<_>>(),
            View::ALL
                .into_iter()
                .map(|view| Some(Item::Widget(view, false)))
                .collect::<Vec<_>>()
        );
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

    /// The widget list has the same problem the menu has, one box further on:
    /// nine rows opened near the bottom of a short window would hang off the
    /// surface, and a row down there cannot be picked at all.
    #[test]
    fn the_widget_list_is_clamped_into_the_window_and_scrolls_when_it_cannot_fit() {
        let dock = Dock::new();
        let (w, h) = (900.0, 600.0);
        let mut menu = Menu::for_widget((w - 2.0, h - 2.0), View::Plan, Space::TopLeft, false);
        menu.toggle_widgets(&dock);
        let layout = with_menu(&dock, &menu, w, h);
        let box_ = layout.menu_list;
        assert!(box_.y >= 0.0 && box_.y + box_.h <= h + 0.01, "{box_:?}");
        assert!(box_.x >= 0.0 && box_.x + box_.w <= w + 0.01, "{box_:?}");
        assert_eq!(
            layout.menu_rows.len(),
            menu.top,
            "the menu keeps its own rows and no more"
        );
        assert_eq!(
            layout.menu_list_rows.len(),
            View::ALL.len(),
            "the whole list fits in a window this tall, so all of it is placed"
        );
        for (index, row) in layout.menu_rows.iter().chain(&layout.menu_list_rows) {
            let (x, y) = middle(*row);
            assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(*index)));
        }

        // A window too short for nine rows gives the list what room there is,
        // which then has to move to reach the rest.
        let short = 150.0;
        let layout = with_menu(&dock, &menu, w, short);
        let placed: Vec<usize> = layout.menu_list_rows.iter().map(|(i, _)| *i).collect();
        assert!(
            placed.len() < View::ALL.len(),
            "this window is not short enough to prove anything"
        );
        assert_eq!(
            layout.menu_rows.len(),
            menu.top,
            "the menu itself still fits and keeps every row"
        );
        assert!(layout.menu_list.y >= 0.0);
        assert!(layout.menu_list.y + layout.menu_list.h <= short + 0.01);
        let capacity = layout.menu_capacity();
        assert_eq!(capacity, placed.len());

        // Scrolled to the end, the last widget is on screen and the first is
        // not, and no row has left the window.
        menu.scroll(View::ALL.len(), true, capacity);
        let layout = with_menu(&dock, &menu, w, short);
        let placed: Vec<usize> = layout.menu_list_rows.iter().map(|(i, _)| *i).collect();
        assert_eq!(
            placed.last().copied(),
            Some(menu.top + View::ALL.len() - 1),
            "the last widget is reachable"
        );
        for (index, row) in layout.menu_rows.iter().chain(&layout.menu_list_rows) {
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

    /// A submenu, which is what the row said it was: the list flies out to the
    /// SIDE of the row that opened it rather than pushing the menu's own rows
    /// around underneath it, in a box of its own.
    ///
    /// It shipped as an accordion, growing downwards into the same column, which
    /// is the correction this asserts.
    #[test]
    fn the_widget_list_flies_out_beside_the_row_that_opened_it() {
        let dock = Dock::new();
        let (w, h) = (1400.0, 900.0);
        let at = (400.0, 300.0);
        let shut = Menu::for_widget(at, View::Plan, Space::TopLeft, false);
        let closed = with_menu(&dock, &shut, w, h);
        assert_eq!(closed.menu_list.w, 0.0, "shut, there is no second box");
        assert!(closed.menu_list_rows.is_empty());

        let mut menu = shut.clone();
        menu.toggle_widgets(&dock);
        let layout = with_menu(&dock, &menu, w, h);
        assert_eq!(
            (layout.menu.x, layout.menu.y, layout.menu.w),
            (closed.menu.x, closed.menu.y, closed.menu.w),
            "the menu itself did not move or grow when the list opened"
        );

        // Out to the side, and no further into the menu than the border line the
        // two boxes share.
        let (box_, list) = (layout.menu, layout.menu_list);
        assert!(list.w >= 1.0, "the flyout has no box");
        assert!(
            list.x >= box_.x + box_.w - MENU_OVERLAP,
            "the flyout is not beside the menu: {list:?} against {box_:?}"
        );

        // Beside the row that opened it: the row it flew out of and its first
        // row are the same band of the window, not one under the other.
        let opener = layout
            .menu_rows
            .iter()
            .find(|(index, _)| menu.pick(*index) == Some(crate::menu::Item::Widgets(true)))
            .map(|(_, panel)| *panel)
            .expect("the Widgets row is on screen");
        let first = layout.menu_list_rows[0].1;
        assert!((first.y - opener.y).abs() < 0.01, "{first:?} {opener:?}");
        for (_, row) in &layout.menu_list_rows {
            assert!(
                row.x >= opener.x + opener.w - MENU_OVERLAP,
                "{row:?} is under the menu rather than beside it"
            );
        }
    }

    /// A menu opened near the right edge has no room out to the right, so the
    /// list flies out to the left instead. Rows hanging off the surface are not
    /// merely invisible: no pointer can reach them.
    #[test]
    fn the_widget_list_flips_to_the_left_at_the_right_edge() {
        let dock = Dock::new();
        let (w, h) = (1400.0, 900.0);
        let mut menu = Menu::for_widget((w - 4.0, 200.0), View::Plan, Space::TopLeft, false);
        menu.toggle_widgets(&dock);
        let layout = with_menu(&dock, &menu, w, h);
        let (box_, list) = (layout.menu, layout.menu_list);
        assert!(
            box_.x + box_.w > w - 1.0,
            "the menu is not against the right edge, so this proves nothing"
        );
        assert!(
            list.x + list.w <= box_.x + MENU_OVERLAP,
            "the flyout did not flip: {list:?} against {box_:?}"
        );
        assert!(list.x >= 0.0 && list.x + list.w <= w + 0.01, "{list:?}");
        for (index, row) in &layout.menu_list_rows {
            let (x, y) = middle(*row);
            assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(*index)));
        }
    }

    /// The flyout is drawn over the menu, so it takes the click before it, and
    /// a click that lands on the menu beside it is still swallowed rather than
    /// falling through to the pane underneath.
    #[test]
    fn the_flyout_is_hit_tested_before_the_menu_it_came_from() {
        let dock = Dock::new();
        let (w, h) = (1400.0, 900.0);
        let at = (500.0, 400.0);
        let plain = Layout::compute(w, h, &shape(&dock, &[]));
        assert!(
            matches!(plain.hit(at.0, at.1), Some(Hit::Body(_))),
            "the menu is not over a pane, so this proves nothing"
        );

        let mut menu = Menu::for_widget(at, View::Plan, Space::TopLeft, false);
        menu.toggle_widgets(&dock);
        let layout = with_menu(&dock, &menu, w, h);

        // Every point of the flyout answers as the flyout, box and rows alike.
        for (index, row) in &layout.menu_list_rows {
            let (x, y) = middle(*row);
            assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(*index)));
        }
        let list = layout.menu_list;
        assert_eq!(
            layout.hit(list.x + list.w * 0.5, list.y + 1.0),
            Some(Hit::Menu),
            "the flyout's own margin lets a click through"
        );

        // And the menu beside it, over the pane the menu was opened on, still
        // swallows one.
        for (index, row) in &layout.menu_rows {
            let (x, y) = middle(*row);
            assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(*index)));
        }
        let box_ = layout.menu;
        assert_eq!(layout.hit(box_.x + 1.0, box_.y + 1.0), Some(Hit::Menu));
        assert_eq!(
            layout.hit(box_.x + 1.0, box_.y + box_.h - 1.0),
            Some(Hit::Menu),
            "a click on the menu fell through to a pane"
        );
    }

    /// The list floats with the menu it came from: it is painted on the floating
    /// layer, above the pane text it covers, and inside its own box. In the base
    /// layer it would be eight tab names drawn under that box.
    #[test]
    fn the_widget_list_is_drawn_on_the_floating_layer() {
        let dock = Dock::hiding(&[View::Hardware]);
        let mut menu = Menu::for_widget((400.0, 200.0), View::Plan, Space::TopLeft, false);
        menu.toggle_widgets(&dock);
        let out = render_menu(&busy_state(), 1400.0, 900.0, &dock, &menu, None);
        let (box_, list) = (out.layout.menu, out.layout.menu_list);
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
        // Switches, marked in the gutter: a ticked box for the eight in the
        // window, an empty one for the widget that is out.
        let empty = runs
            .iter()
            .filter(|text| *text == &icons::UNCHECKED.to_string());
        assert_eq!(empty.count(), 1, "only DEBUG is closed");
        assert_eq!(
            runs.iter()
                .filter(|text| *text == &icons::CHECKED.to_string())
                .count(),
            View::ALL.len() - 1
        );
        // The list is written in its own box and the menu's rows in the menu's,
        // and both boxes have a surface under them on the overlay.
        for text in &out.scene.over_texts {
            let holder = match text.at.x >= list.x - 0.01 {
                true => list,
                false => box_,
            };
            assert!(
                text.at.y >= holder.y - 0.01
                    && text.at.y + text.at.h <= holder.y + holder.h + 0.01
                    && text.at.x + text.at.w <= holder.x + holder.w + 0.01,
                "{:?} is outside {holder:?}",
                text.at
            );
        }
        let surface = |panel: Panel| {
            out.scene
                .over_rects
                .iter()
                .any(|r| r.xywh() == [panel.x, panel.y, panel.w, panel.h] && r.extra()[3] == 0.0)
        };
        assert!(surface(box_), "the menu has no surface");
        assert!(surface(list), "the flyout is not a box of its own");
    }

    /// The row that opens the list is marked twice: the grid of frames in the
    /// gutter in front, saying what the row is, and the submenu chevron at its
    /// END, saying the list is out to the side.
    ///
    /// It shipped with a plus at the front, which is what a row that folds a
    /// list out underneath itself says, and this test then asserted the gutter
    /// was empty. Empty is not what the gutter is for: it is spent on every row
    /// either way, and this was the only row in the menu spending it on nothing.
    #[test]
    fn the_row_that_flies_out_is_marked_in_its_gutter_and_at_its_end() {
        use crate::menu::Item;
        let dock = Dock::new();
        for open in [false, true] {
            let mut menu = Menu::for_widget((400.0, 300.0), View::Plan, Space::TopLeft, false);
            if open {
                menu.toggle_widgets(&dock);
            }
            let out = render_menu(&busy_state(), 1400.0, 900.0, &dock, &menu, None);
            let row = out
                .layout
                .menu_rows
                .iter()
                .find(|(index, _)| matches!(menu.pick(*index), Some(Item::Widgets(_))))
                .map(|(_, panel)| *panel)
                .expect("the Widgets row is on screen");
            let marks: Vec<&Text> = out
                .scene
                .over_texts
                .iter()
                .filter(|text| {
                    text.runs
                        .iter()
                        .any(|run| run.text == icons::SUBMENU.to_string())
                })
                .collect();
            assert_eq!(marks.len(), 1, "one row flies out, so there is one mark");
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

    /// Every row of the menu is drawn with a mark in its gutter, and it is the
    /// mark the model names. Four of them shipped blank: copy selection, close
    /// this widget, Widgets and paste each spent the gutter on a space, which
    /// reads as a row whose icon failed to draw rather than a row without one.
    #[test]
    fn every_menu_row_is_drawn_with_its_own_mark_in_the_gutter() {
        use crate::menu::Item;
        let dock = Dock::hiding(&[View::Hardware]);
        let mut widget = Menu::for_widget((400.0, 300.0), View::Plan, Space::TopLeft, true);
        widget.toggle_widgets(&dock);
        for menu in [widget, Menu::for_input((400.0, 300.0), true)] {
            let out = render_menu(&busy_state(), 1400.0, 900.0, &dock, &menu, None);
            let rows = out
                .layout
                .menu_rows
                .iter()
                .chain(out.layout.menu_list_rows.iter());
            let mut seen = 0;
            for (index, panel) in rows {
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
            assert!(seen >= menu.top, "not every row was placed");
        }
        // The four the requirement named, on the rows the requirement named.
        assert_eq!(Item::CopySelection.icon(), Some(icons::COPY));
        assert_eq!(Item::Close.icon(), Some(icons::CLOSE_WIDGET));
        assert_eq!(Item::Widgets(false).icon(), Some(icons::WIDGETS));
        assert_eq!(Item::Paste.icon(), Some(icons::PASTE));
    }

    /// The same window with one activity row opened out.
    fn render_popup(state: &State, w: f32, h: f32, dock: &Dock) -> Rendered {
        let shape = Shape {
            popup: state.popped(),
            ..shape(dock, &[])
        };
        let layout = Layout::compute(w, h, &shape);
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

    /// The popup is on the floating layer, it carries the four things it
    /// promises, and shutting it takes the whole thing off the screen.
    ///
    /// On the overlay for the reason the menu is: the renderer paints a layer's
    /// rectangles in one pass and its glyphs in a later one, so a box pushed onto
    /// the base layer lands under the pane text it is covering, however late it
    /// was pushed.
    #[test]
    fn the_activity_popup_is_painted_over_the_window_and_closes_off_it() {
        let mut state = busy_state();
        state.apply(noob_proto::Event::ToolEnd {
            call_id: "c1".into(),
            summary: "bash cargo test (2.0s, exit 101)".into(),
            elapsed_ms: 2000,
            error: Some(noob_proto::ToolError {
                kind: "exit_status".into(),
                code: Some(101),
                message: "1 test failed".into(),
                detail: Some("thread 'a' panicked at src/lib.rs:9".into()),
                remedy: Some("run it again with --nocapture".into()),
            }),
        });
        let dock = Dock::new();

        // Shut, there is no box and nothing on the overlay.
        let shut = render_popup(&state, 1400.0, 900.0, &dock);
        assert!(shut.layout.call_popup.w < 1.0);
        assert!(shut.scene.over_rects.is_empty(), "something is floating already");

        state.open_call = state.call_at_line(0);
        assert!(state.open_call.is_some(), "the first row is the bash call");
        let out = render_popup(&state, 1400.0, 900.0, &dock);
        let box_ = out.layout.call_popup;
        assert!(box_.w >= 1.0 && box_.h >= 1.0);

        // The condition that makes the overlay the point: there is pane text
        // under it.
        assert!(
            text_over(&out.scene.texts, box_),
            "nothing is written under the popup, so this proves nothing"
        );
        let surface = |rects: &[Rect]| {
            rects
                .iter()
                .any(|r| r.xywh() == [box_.x, box_.y, box_.w, box_.h] && r.extra()[3] == 0.0)
        };
        assert!(surface(&out.scene.over_rects), "the popup has no surface");
        assert!(
            !surface(&out.scene.rects),
            "the popup's surface is in the base layer, under every glyph"
        );

        // Everything on the overlay is the popup's, and every cell it promised
        // is written.
        let floating: String = out
            .scene
            .over_texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
            .collect();
        for want in ["INVOKED", "GENERATED", "RETURNED", "DETAIL", "WHEN"] {
            assert!(floating.contains(want), "no {want} cell: {floating}");
        }
        assert!(floating.contains("cargo test --workspace"), "{floating}");
        assert!(floating.contains("exit_status 101"), "{floating}");
        assert!(floating.contains("panicked at src/lib.rs:9"), "{floating}");
        assert!(floating.contains("run it again with --nocapture"), "{floating}");

        for text in &out.scene.over_texts {
            assert!(
                text.at.x >= box_.x - 0.01 && text.at.y >= box_.y - 0.01,
                "{:?} is on the overlay but is not the popup",
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
        assert!(dock.hide(View::Files));
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

    fn inside(rect: Rect, box_: Panel) -> bool {
        let [x, y, w, h] = rect.xywh();
        x >= box_.x - 0.01
            && y >= box_.y - 0.01
            && x + w <= box_.x + box_.w + 0.01
            && y + h <= box_.y + box_.h + 0.01
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

        // The row the cursor is on is a filled green band with the dark ink
        // written over it. Item 4: the quiet band the file explorer marks its
        // open row with said almost nothing here.
        let (index, cursor_row) = layout.picker_rows[0];
        assert_eq!(index, picker.cursor());
        assert!(
            covered(&out, cursor_row, cursor_row.h, out.skin.picked),
            "the cursor's row has no band"
        );
        // And no other row is banded, or every row would read as the one. Only
        // the full width of a row counts: `skin.mark_edge` is the same green,
        // and the hairline box in front of every folder is not a band.
        let banded = out
            .scene
            .rects
            .iter()
            .filter(|rect| rect.rgba() == out.skin.picked && rect.xywh()[2] >= cursor_row.w - 0.01)
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
        assert!(!covered(&cold, sessions, sessions.h, cold.skin.picked));
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
        assert_eq!(box_.rgba(), out.skin.mark_edge, "the box is not green");
        assert!(
            out.skin.mark_edge[1] > out.skin.mark_edge[0]
                && out.skin.mark_edge[1] > out.skin.mark_edge[2],
            "{:?} is not green",
            out.skin.mark_edge
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
            crate::menu::Item::DeleteSession.label(),
        ] {
            assert!(rows.contains(label), "{label:?} is not on screen: {rows:?}");
        }

        // And it takes the press before the row it covers, which it always did.
        let (x, y) = middle(out.layout.menu_rows[1].1);
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
        render_settings_at_rail(panel, w, h, hot, SETTINGS_RAIL)
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
        let dock = Dock::new();
        let state = busy_state();
        let mut shape = shape(&dock, &["a.rs"]);
        shape.settings = Some(panel);
        shape.settings_rail = rail;
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

    /// An agent with one of everything, so every section has rows of its own to
    /// draw rather than a note saying it is empty.
    fn an_agent() -> crate::agent::Agent {
        crate::agent::Agent {
            env_path: Some(std::path::PathBuf::from("/home/hec/.config/noob/.env")),
            env_exists: true,
            env: vec![
                (
                    String::from(crate::agent::ENDPOINT),
                    String::from("http://localhost:8080/v1"),
                ),
                (String::from("NOOB_CTX"), String::from("262144")),
            ],
            skills_at: Some(std::path::PathBuf::from("/home/hec/.config/noob/skills")),
            skills: vec![crate::agent::Skill {
                dir: String::from("coding"),
                name: String::from("coding"),
                about: String::from("Changing code that already exists."),
                repo: Some(String::from("https://github.com/someone/coding")),
                path: std::path::PathBuf::from("/home/hec/.config/noob/skills/coding"),
                on: true,
                doc: vec![
                    String::from("# Changing code"),
                    String::new(),
                    String::from("Read the file before writing it."),
                ],
            }],
            // Where a global AGENTS.md would go, with nothing in it: the
            // machine this fixture stands for has never written one.
            instructions: crate::agent::Instructions {
                path: Some(std::path::PathBuf::from("/home/hec/.config/noob/AGENTS.md")),
                body: Vec::new(),
                capped: false,
            },
            sessions: crate::sessions::Listing {
                sessions: vec![crate::sessions::Saved {
                    id: String::from("abc"),
                    when: std::time::SystemTime::UNIX_EPOCH,
                    workspace: Some(std::path::PathBuf::from("/home/hec/workspace/noob-cli")),
                    gone: false,
                    bytes: 4_096,
                    context: None,
                    opening: String::from("rebuild the settings panel"),
                }],
                skipped: Vec::new(),
            },
            ..crate::agent::Agent::default()
        }
    }

    fn a_settings_panel(config: &Config) -> Settings {
        Settings::open(
            config,
            Some(std::path::Path::new("/home/hec/.config/noob/no0b.conf")),
            an_agent(),
        )
    }

    /// The panel with one section chosen, which is what a press on the rail
    /// leaves behind. The keyboard is on the rows of it either way: the rail is
    /// pressed, never walked.
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

    /// Every row of every section is hit where it is drawn, the control at the
    /// end of a row that can change is its own region, and a row that cannot
    /// change has none.
    #[test]
    fn every_settings_row_lands_where_it_is_drawn() {
        for section in crate::settings::SECTIONS {
            let panel = a_panel_on(&Config::default(), section);
            let out = render_settings(&panel, 1400.0, 900.0, None);
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
                // A row of the palette grid is all controls: one cell per colour
                // on it, each one hit where its block is drawn.
                if let Some(crate::settings::Row::Swatches(swatches)) = panel.row(*index) {
                    let cells: Vec<(usize, Panel)> = layout
                        .settings_cells
                        .iter()
                        .filter(|(at, ..)| at == index)
                        .map(|(_, cell, panel)| (*cell, *panel))
                        .collect();
                    assert_eq!(cells.len(), swatches.len(), "row {index} of {section}");
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
                    // A saved conversation carries its own delete in the last
                    // column of the table, so its right hand end is a button
                    // rather than more of the row.
                    (None, None)
                        if matches!(
                            panel.row(*index),
                            Some(crate::settings::Row::Session { .. })
                        ) =>
                    {
                        assert_eq!(
                            layout.hit(row.x + row.w - 2.0, row.y + row.h * 0.5),
                            Some(Hit::SettingsRemove(*index)),
                            "{section}"
                        );
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
            for want in [crate::settings::Side::Left, crate::settings::Side::Right] {
                let lefts: Vec<f32> = layout
                    .settings_values
                    .iter()
                    .chain(layout.settings_tracks.iter())
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

    /// An agent with instructions of its own and a prompt already read, for the
    /// two blocks at the bottom of the AGENT section.
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

    /// The AGENT section is a form: two rows of two, each half drawn in its own
    /// column and pressed where it is drawn.
    ///
    /// "put it like forms": the endpoint and the file the agent reads down the
    /// left, the two numbers that decide what it gets down the right, with
    /// nothing between them. Every half is its own box, so a press in one column
    /// cannot land on the other.
    #[test]
    fn the_agent_form_draws_two_columns_that_are_pressed_apart() {
        let mut panel = a_panel_on(&Config::default(), crate::settings::AGENT);
        panel.adopt_prompt(
            String::from("/home/hec/workspace/noob-cli"),
            Ok(vec![String::from("You are noob.")]),
            &Config::default(),
        );
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let list = layout.settings_list;
        let form: Vec<(usize, Side, Panel)> = layout
            .settings_rows
            .iter()
            .filter(|(index, _, _)| *index < 2)
            .copied()
            .collect();
        assert_eq!(form.len(), 4, "the form is not two rows of two: {form:?}");
        for pair in form.chunks(2) {
            let [(index, left_side, left), (_, right_side, right)] = pair else {
                panic!("{pair:?}");
            };
            assert_eq!((*left_side, *right_side), (Side::Left, Side::Right));
            // Side by side, on the same lines, filling the row between them.
            assert_eq!(left.y, right.y, "row {index} is not one row");
            assert_eq!(left.h, right.h);
            assert!((left.x - list.x).abs() < 0.01, "{left:?}");
            assert!((left.x + left.w - right.x).abs() < 0.01, "{left:?} {right:?}");
            assert!(
                (right.x + right.w - (list.x + list.w)).abs() < 0.01,
                "{right:?} against {list:?}"
            );
            // And each half answers for itself: a press in one column is that
            // column's row, never the other's.
            for (side, at) in [(Side::Left, left), (Side::Right, right)] {
                let (x, y) = middle(*at);
                assert!(
                    matches!(layout.hit(x, y), Some(Hit::SettingsRow(row, half) | Hit::SettingsValue(row, half) | Hit::SettingsSlider(row, half)) if row == *index && half == side),
                    "the {side:?} half of row {index} answers with {:?}",
                    layout.hit(x, y)
                );
            }
        }
        // The controls are in the halves they belong to: the endpoint is typed
        // into on the left, the context window is dragged on the right.
        let endpoint = layout
            .settings_values
            .iter()
            .find(|(index, side, _)| *index == 0 && *side == Side::Left)
            .map(|(_, _, at)| *at)
            .expect("the endpoint's box");
        let ctx = layout
            .settings_tracks
            .iter()
            .find(|(index, side, _)| *index == 0 && *side == Side::Right)
            .map(|(_, _, at)| *at)
            .expect("the context window's track");
        assert!(endpoint.x + endpoint.w <= ctx.x, "{endpoint:?} {ctx:?}");
        assert!(form[0].2.contains(endpoint.x + 1.0, endpoint.y + 1.0));
        assert!(form[1].2.contains(ctx.x + 1.0, ctx.y + 1.0));

        // Both are drawn, both are labelled with the key that writes them, and
        // the file the agent reads is on the row under the endpoint.
        let text = text_of(&out.scene);
        for wanted in [
            crate::agent::ENDPOINT,
            crate::agent::CTX,
            crate::agent::TASK_CONCURRENCY,
            "main file",
        ] {
            assert!(text.contains(wanted), "{wanted} is not drawn: {text}");
        }
    }

    /// The two blocks under the form: the file the agent really reads, and the
    /// whole prompt it is one layer of. Both draw their title and their text.
    #[test]
    fn the_agent_section_draws_the_instructions_and_the_prompt() {
        let mut panel = Settings::open(
            &Config::default(),
            None,
            an_agent_with_instructions(),
        );
        let at = panel
            .section_names()
            .iter()
            .position(|name| *name == crate::settings::AGENT)
            .expect("the agent section");
        panel.choose(at);
        panel.adopt_prompt(
            String::from("/home/hec/workspace/noob-cli"),
            Ok(vec![
                String::from("You are noob, a coding agent."),
                String::new(),
                String::from("# Global instructions (AGENTS.md)"),
            ]),
            &Config::default(),
        );
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

        // And the prompt, under a title of its own that says where it came
        // from. The file is never called the prompt: it is one layer of it.
        assert!(text.contains("THE PROMPT THE AGENT GETS"), "{text}");
        assert!(text.contains("noob debug prompt"), "{text}");
        assert!(text.contains("You are noob, a coding agent."), "{text}");

        // The titles are the accent green, the way every heading on this panel
        // is, so a block reads as a block rather than as more rows.
        let title = out
            .scene
            .texts
            .iter()
            .flat_map(|text| text.runs.iter())
            .find(|run| run.text.contains("THE PROMPT"))
            .expect("the prompt's title");
        assert_eq!(title.color, Some(out.skin.good));
    }

    /// A file that is not there is an offer to write one, and a command that
    /// failed is the reason it failed. Neither is an empty box.
    #[test]
    fn a_missing_file_and_a_failed_command_are_said_on_their_own_block() {
        let mut panel = a_panel_on(&Config::default(), crate::settings::AGENT);
        panel.adopt_prompt(
            String::from("/home/hec/workspace/noob-cli"),
            Err(String::from("noob debug prompt failed: no such subcommand")),
            &Config::default(),
        );
        let out = render_settings(&panel, 1400.0, 1200.0, None);
        let text = text_of(&out.scene);
        // `an_agent` has no AGENTS.md at all, so the block says where one would
        // go and what the key would do.
        assert!(text.contains("nothing at"), "{text}");
        assert!(text.contains("/home/hec/.config/noob/AGENTS.md"), "{text}");
        assert!(
            text.contains("The agent reads this file first"),
            "the block is empty: {text}"
        );
        // And the prompt block carries the reason, in the colour this window
        // uses for something that went wrong.
        assert!(text.contains("no such subcommand"), "{text}");
        let why = out
            .scene
            .texts
            .iter()
            .flat_map(|text| text.runs.iter())
            .find(|run| run.text.contains("no such subcommand"))
            .expect("the reason");
        assert_eq!(why.color, Some(out.skin.bad));
    }

    /// The skills section is two columns: the entries down the left, and the
    /// `SKILL.md` of the one under the cursor beside them, rendered rather than
    /// printed with its marks in.
    #[test]
    fn the_skills_section_puts_the_skill_beside_the_list() {
        let panel = a_panel_on(&Config::default(), crate::settings::SKILLS);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let (list, doc) = (layout.settings_list, layout.settings_doc);
        assert!(doc.w >= 1.0, "there is no second column");
        assert!(
            list.x + list.w <= doc.x,
            "the two columns overlap: {list:?} and {doc:?}"
        );
        assert!(
            doc.x + doc.w <= layout.settings.x + layout.settings.w,
            "the document runs off the panel"
        );
        // Every row of the list is in the left column, so a press in the
        // document cannot land on a skill.
        for (index, _, row) in &layout.settings_rows {
            assert!(
                row.x + row.w <= doc.x,
                "row {index} runs into the document: {row:?}"
            );
        }
        let (x, y) = middle(doc);
        assert_eq!(layout.hit(x, y), Some(Hit::Settings), "{doc:?}");

        // What is drawn: the name and the repository on the left, the document
        // on the right, and no Markdown marks in it.
        let text = text_of(&out.scene);
        assert!(text.contains("coding"), "{text}");
        assert!(
            text.contains("https://github.com/someone/coding"),
            "the repository is not under the name: {text}"
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
        assert!(plain.layout.settings_list.w > list.w, "the list did not split");
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

    /// Item E3: the saved conversations are a table. A row that read
    /// "17h ago  noob-cli  283 B  -  fix the panel" said five things with
    /// nothing naming any of them, so every cell sits in a column of its own
    /// under a row that says what that column is, the row the cursor is on
    /// carries a band across the whole of it rather than differently coloured
    /// words, and the last column is a trash of its own.
    #[test]
    fn the_sessions_section_is_a_table_under_a_row_naming_its_columns() {
        let panel = a_sessions_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let layout = &out.layout;
        let column = 8.0;
        let find = |want: fn(&crate::settings::Row) -> bool| -> Vec<(usize, Panel)> {
            layout
                .settings_rows
                .iter()
                .filter(|(index, _, _)| panel.row(*index).is_some_and(want))
                .map(|(index, _, at)| (*index, *at))
                .collect()
        };
        let header = find(|row| matches!(row, crate::settings::Row::Columns(_)));
        assert_eq!(header.len(), 1, "one row names the columns");
        let (_, header_at) = header[0];
        let rows = find(|row| matches!(row, crate::settings::Row::Session { .. }));
        assert_eq!(rows.len(), 3, "every saved conversation is drawn");

        // Every column has its name over it, and the name starts exactly where
        // the cells under it start.
        let names = settings_session_cells(header_at, column);
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
        for (step, (name, _)) in crate::settings::SESSION_COLUMNS.iter().enumerate() {
            let said = text_at(&out, Panel::new(names[step].x, header_at.y, 1.0, 1.0));
            assert!(said.starts_with(name), "column {step} is headed {said:?}");
        }

        // And every row writes its cells into those same columns, at the same x
        // the header is drawn at.
        for (index, row) in &rows {
            let cells = match panel.row(*index) {
                Some(crate::settings::Row::Session { cells, .. }) => cells.clone(),
                other => panic!("not a session: {other:?}"),
            };
            let boxes = settings_session_cells(*row, column);
            for (step, cell) in cells.iter().enumerate() {
                assert!(
                    (boxes[step].x - names[step].x).abs() < 0.01,
                    "row {index} column {step} is not under its header"
                );
                let said = text_at(&out, Panel::new(boxes[step].x, row.y, 1.0, 1.0));
                assert!(
                    said.starts_with(&clip(cell, columns_in(boxes[step].w, column) - 1)),
                    "row {index} column {step} says {said:?}, not {cell:?}"
                );
            }
        }

        // The row the cursor is on is a band across the whole row, in the solid
        // colour the folder picker's own session list uses, not a tint on the
        // words.
        let (cursor, band) = rows
            .iter()
            .find(|(index, _)| *index == panel.cursor())
            .copied()
            .expect("the cursor is on a session");
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
        assert!(filled(&out, band, out.skin.picked), "no band on row {cursor}");
        for (index, at) in &rows {
            if index != &cursor {
                assert!(!filled(&out, *at, out.skin.picked), "row {index} is banded too");
            }
        }

        // The last column is a trash of its own: pressed where it is drawn, in
        // the colour this window uses for everything that throws work away, and
        // it is not the row, so a press on the words still moves the cursor.
        let trash: Vec<(usize, Panel)> = layout
            .settings_removes
            .iter()
            .filter(|(index, _)| rows.iter().any(|(row, _)| row == index))
            .map(|(index, at)| (*index, *at))
            .collect();
        assert_eq!(trash.len(), 3, "every row can be deleted");
        for (index, box_) in &trash {
            let (x, y) = middle(*box_);
            assert_eq!(layout.hit(x, y), Some(Hit::SettingsRemove(*index)));
            let marks: Vec<&noob_draw::Run> = out
                .scene
                .texts
                .iter()
                .filter(|text| text.at.x >= box_.x && text.at.x < box_.x + box_.w)
                .flat_map(|text| text.runs.iter())
                .collect();
            assert!(
                marks
                    .iter()
                    .any(|run| {
                        run.text.contains(icons::TRASH)
                            && run.icon
                            && run.color == Some(out.skin.bad)
                    }),
                "row {index} has no trash mark in the bad colour"
            );
            // In the last column of the table, under the name of it.
            let row = rows
                .iter()
                .find(|(at, _)| at == index)
                .map(|(_, at)| *at)
                .expect("the row it belongs to");
            let last = *settings_session_cells(row, column)
                .last()
                .expect("a last column");
            assert!((box_.x - last.x).abs() < 0.01, "the trash is not the last column");
            assert!(
                (box_.x - names[names.len() - 1].x).abs() < 0.01,
                "the trash is not under the name of its column"
            );
            assert!(row.contains(box_.x + 1.0, box_.y + 1.0), "the trash is outside its row");
        }
        let (_, first) = rows[0];
        assert_eq!(
            layout.hit(first.x + 2.0, first.y + first.h * 0.5),
            Some(Hit::SettingsRow(rows[0].0, crate::settings::Side::Left)),
            "the words of a row still put the cursor there"
        );
    }

    /// The trash asks before it acts: pressed once it says so on the button and
    /// on the footer, and nothing is drawn differently anywhere else.
    #[test]
    fn the_trash_on_a_session_says_sure_before_it_deletes() {
        let mut panel = a_sessions_panel();
        let row = panel
            .rows()
            .iter()
            .position(|row| matches!(row, crate::settings::Row::Session { .. }))
            .expect("a session row");

        let before = render_settings(&panel, 1400.0, 900.0, None);
        assert!(!text_of(&before.scene).contains("sure?"));

        assert_eq!(panel.uninstall(row), None, "the first press deleted it");
        let after = render_settings(&panel, 1400.0, 900.0, None);
        let text = text_of(&after.scene);
        assert!(text.contains("sure?"), "the button does not ask: {text}");
        assert!(text.contains("press delete again"), "the footer does not ask: {text}");

        // The box says so with its edge as well, which is what makes it read as
        // armed rather than as a word that changed.
        let box_ = after
            .layout
            .settings_removes
            .iter()
            .find(|(index, _)| *index == row)
            .map(|(_, at)| *at)
            .expect("the trash is placed");
        assert!(
            after.scene.rects.iter().any(|rect| {
                let [x, y, ..] = rect.xywh();
                rect.rgba() == after.skin.close_hot
                    && (x - box_.x).abs() < 2.0
                    && (y - box_.y).abs() < 2.0
            }),
            "the armed trash looks exactly like the unarmed one"
        );
    }

    /// The toggle and the uninstall on an entry are pressed where they are
    /// drawn, and neither of them is the row: a press on the name still puts
    /// the cursor there rather than deleting a skill.
    #[test]
    fn an_entry_carries_a_toggle_and_an_uninstall_of_its_own() {
        let panel = a_panel_on(&Config::default(), crate::settings::SKILLS);
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
        assert_eq!(entries.len(), 1, "the one skill is not a row of its own");
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
        assert_eq!(tint_of(&out, "AGENT"), out.skin.good);
        assert_ne!(tint_of(&out, "APPEARANCE"), out.skin.good);

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
        assert!(text.contains("THE WINDOW"), "{text}");
        assert!(!text.contains("api keys"), "the agent section is still up");
        assert_eq!(tint_of(&out, "APPEARANCE"), out.skin.good);
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
        // around it.
        let drawn = layout.settings_list.x - (GAP * 0.5).floor();
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
        for x in [200.0, 420.0, 150.0] {
            let ratio = out.layout.settings_rail_ratio_at(x);
            let moved = render_settings_at_rail(&panel, 1400.0, 900.0, None, ratio);
            let layout = &moved.layout;
            let list = layout.settings_list;
            seen.push(list.x);
            // Under the pointer rather than near it, to the pixel the width was
            // floored to.
            let drawn = list.x - GAP * 0.5;
            assert!((drawn - x).abs() <= 1.5, "{x}: the line landed at {drawn}");

            // Every name ends where the gap begins, and every row starts on the
            // far side of it.
            for (index, at) in &layout.settings_rail {
                assert!(
                    (at.x + at.w + GAP - list.x).abs() <= 0.01,
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
            let rail_w = layout.settings_rail[0].1.w;
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
            assert!(moved.layout.settings_rail[0].1.w >= floor, "{ratio}");
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
        assert!(
            matches!(panel.row(index), Some(crate::settings::Row::Setting { key, .. }) if *key == "opacity")
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
        // other half of a row that has one.
        assert_eq!(layout.slider_at(index + 500, side, track.x), None);
        assert_eq!(layout.slider_at(index, side.other(), track.x), None);

        // The track is drawn where it is pressed: an unlit bar the width of the
        // track and a lit one as far along it as the value.
        let on_the_track = |rgba: [f32; 4]| {
            out.scene
                .rects
                .iter()
                .filter(|rect| rect.rgba() == rgba)
                .map(|rect| rect.xywh())
                .find(|[x, y, ..]| track.contains(*x + 0.5, *y + 0.5))
        };
        let thumb = on_the_track(out.skin.gauge).expect("nothing is lit");
        assert!((thumb[0] - track.x).abs() < 0.01, "{thumb:?}");
        let full = on_the_track(out.skin.gauge_track).expect("there is no track");
        assert!((full[2] - track.w).abs() < 0.01, "{full:?}");
        // Half way through its range, so the lit part is about half the track.
        let at = (0.5 - 0.05) / (1.0 - 0.05);
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

    /// Every group heading inside a section is the same green the showing tab's
    /// line is, is drawn larger than the settings under it, and is given the room
    /// for that by the row it was laid out in.
    ///
    /// A list is unreadable if its groups do not separate from their contents.
    /// The heading was in the ordinary text tint, and then it was the accent
    /// green at the same size as everything else; a heading measured at one
    /// height and drawn at another would put every click below it on the wrong
    /// row, which is why the size and the row's height are asserted together.
    #[test]
    fn the_settings_headings_are_the_accent_green_and_the_size_they_were_measured_at() {
        let mut found = 0;
        let panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 1200.0, None);
        for heading in [
            "THE WINDOW",
            "THE HIGHLIGHTER",
            "ONE PER TOOL",
            "ONE PER GAUGE",
        ] {
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
            assert_eq!(run.color, Some(out.skin.good), "{heading}");
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
            assert!(
                matches!(panel.row(row.0), Some(crate::settings::Row::Heading(name)) if *name == heading),
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
        assert_eq!(found, 4);
        // The same green the tab wears, and not the tint a setting's key is
        // written in, or the heading is another row of the list.
        assert_eq!(
            [
                out.skin.good[0] as f32 / 255.0,
                out.skin.good[1] as f32 / 255.0,
                out.skin.good[2] as f32 / 255.0,
                1.0
            ],
            out.skin.tab_accent
        );
        assert_ne!(out.skin.good, out.skin.body);
        assert_ne!(out.skin.good, out.skin.title);
    }

    /// Every row but the last one on screen has a hairline under it, so a row
    /// reads as its own thing rather than as one more line of a block of text.
    #[test]
    fn a_settings_row_is_ruled_off_from_the_one_under_it() {
        let panel = a_panel_on(&Config::default(), crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let rows = &out.layout.settings_rows;
        assert!(rows.len() > 3, "not enough rows to rule off: {}", rows.len());
        let rule = |row: &Panel| {
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, h] = rect.xywh();
                rect.rgba() == out.skin.edge
                    && h <= 1.01
                    && (x - row.x).abs() < 0.01
                    && (w - row.w).abs() < 0.01
                    && (y - (row.y + row.h - 1.0)).abs() < 0.01
            })
        };
        let mut ruled = 0;
        for (index, _, row) in &rows[..rows.len() - 1] {
            // A heading keeps the group under it: the line above it is what
            // separates that group from the one before.
            if matches!(panel.row(*index), Some(crate::settings::Row::Heading(_))) {
                assert!(!rule(row), "the heading on row {index} is cut off from its group");
                continue;
            }
            assert!(rule(row), "row {index} runs into the one under it");
            ruled += 1;
        }
        assert!(ruled > 3, "hardly anything is ruled off: {ruled}");
        // Not under the last one: a line along the bottom of the list reads as
        // the edge of a box that is not there.
        let (_, _, last) = rows.last().expect("a last row");
        assert!(!rule(last), "the list is closed off at the bottom");
    }

    /// Anything that can be typed into or pressed to change is drawn as a box
    /// with an outline round it. Without one an editable row looked exactly like
    /// a reading, and the only way to tell one from the other was to press it.
    #[test]
    fn an_editable_row_is_drawn_as_a_box_with_an_edge() {
        // The endpoint, which is the one row on the panel that is typed into,
        // and the presets and flags, which are pressed to change.
        for section in [crate::settings::AGENT, crate::settings::APPEARANCE] {
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
            (crate::settings::AGENT, "/home/hec/.config/noob/.env"),
            (crate::settings::SESSIONS, "rebuild the settings panel"),
            (crate::settings::SKILLS, "coding"),
            (crate::settings::MCP, "none configured"),
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

        // The row the cursor is on carries the band and the mark every list in
        // this window marks its current row with.
        let panel = a_panel_on(&config, crate::settings::APPEARANCE);
        let out = render_settings(&panel, 1400.0, 1200.0, None);
        let row = out
            .layout
            .settings_rows
            .iter()
            .find(|(index, side, _)| *index == panel.cursor() && *side == panel.side())
            .map(|(_, _, row)| *row)
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

        let out = render_settings(&panel, 1400.0, 1200.0, None);
        assert!(
            text_of(&out.scene).contains("1048576"),
            "the context window is not drawn as the number it is: {}",
            text_of(&out.scene)
        );

        // And both of them are tracks, so the maximum concurrency is a place to
        // drop the pointer rather than a number to type. Both of them are in
        // the right hand column of the form, which is where the two numbers
        // that decide what the agent gets live.
        for key in crate::agent::OWNED {
            let (index, side) = panel
                .rows()
                .iter()
                .enumerate()
                .find_map(|(at, row)| {
                    [Side::Left, Side::Right].into_iter().find_map(|side| {
                        matches!(crate::settings::cell(row, side), crate::settings::Row::Setting { key: k, .. } if *k == key)
                            .then_some((at, side))
                    })
                })
                .unwrap_or_else(|| panic!("{key} is not on the agent section"));
            assert_eq!(side, Side::Right, "{key} is not in the form's right column");
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
        // Side by side, left to right, and none of them overlapping.
        for pair in on_the_row.windows(2) {
            assert!(
                pair[0].1.x + pair[0].1.w <= pair[1].1.x + 0.01,
                "{pair:?} share a column"
            );
            assert!((pair[0].1.y - pair[1].1.y).abs() < 0.01, "{pair:?}");
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

        // Each cell draws a block of its own colour inside itself.
        for (cell, at) in &on_the_row {
            let colour = panel.swatch(row, *cell).expect("a colour");
            let block = out
                .scene
                .rects
                .iter()
                .find(|rect| rect.rgba() == swatch(colour.rgb))
                .unwrap_or_else(|| panic!("{} is not drawn as itself", colour.key));
            let [x, y, ..] = block.xywh();
            assert!(at.contains(x + 0.5, y + 0.5), "{block:?} is outside {at:?}");
        }
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
        let rows = render_settings(&panel, w, h, None)
            .layout
            .settings_capacity(13.0);
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
            if !panel.scroll(1, true, rows) {
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
