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
use crate::skin::Skin;
use crate::state::{State, TodoState, Tone};

pub const TITLE_H: f32 = 30.0;
pub const INPUT_H: f32 = 36.0;
pub const TAB_H: f32 = 22.0;
pub const RESIZE_EDGE: f32 = 6.0;
const GAP: f32 = 6.0;
const PAD: f32 = 9.0;
/// Columns the file view spends on its line-number gutter, on every row.
const GUTTER: usize = 4;
const SMALL: f32 = 12.0;
const SCROLL_W: f32 = 4.0;
const BUTTON_W: f32 = 26.0;
/// The square kept clear at the left end of the title strip, where the orb goes.
///
/// The orb is the one animated thing in the window and it is not built yet. The
/// strip's text starts after this, so drawing the orb later adds it to an empty
/// slot rather than pushing the name along, and the strip reads
/// `[orb] NO0B \u{25b8} version` from the first frame it ever draws.
pub const ORB_W: f32 = TITLE_H;
const LABEL_COLUMNS: usize = 9;
/// A gauge is a block of dots: eight across and five down is 0 to 100 percent,
/// so one row is 20 percent and one dot is 2.5. Squarer than the ten by four bar
/// it replaced, because the reference asked for a block that reads as a block,
/// and a row ten dots wide reads as a bar with rows in it.
const DOT_COLUMNS: usize = 8;
const DOT_ROWS: usize = 5;
/// How much larger the number beside a block is than the label. The reference
/// puts the value in large text next to the block, and it is the thing being
/// read; the label only says which reading it is.
const BIG_READING: f32 = 1.5;
/// The smallest a dot shrinks to when a pane has more readings than room. Below
/// this the block stops reading as a block, so the rows go instead: a reading
/// that scrolled off is honest, a smear is not.
const SMALL_DOT: f32 = 4.0;
const PROMPT_COLUMNS: usize = 2;
const INPUT_PAD: f32 = 6.0;
/// How far the 45 degree cut reaches along each edge of a panel's top-right
/// corner. One corner, so the shape reads as a mark rather than as a rounded
/// box, and always the same corner so two panels side by side still line up.
const CUT: f32 = 10.0;
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

/// The rows the picker spends above its list: the heading, the folder it is
/// listing, and what has been typed.
const PICKER_HEAD_ROWS: f32 = 3.0;

/// And below it: the keys, then the button that confirms.
const PICKER_FOOT_ROWS: f32 = 2.0;

/// How tall its list is allowed to get, in rows, and how short. Short enough
/// that a folder with two entries is not a mostly empty box, tall enough that a
/// deep source tree is not read four rows at a time.
const PICKER_MIN_ROWS: usize = 6;
const PICKER_MAX_ROWS: usize = 24;

/// How wide the mark down the left of the selected row is. A tab's accent runs
/// along its top edge because a strip is read left to right; a row is entered
/// from the left, so its accent runs down that edge instead.
const MARK_W: f32 = 2.0;

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
    /// The button that confirms the row the cursor is on, which is how the
    /// mouse chooses a folder without a keyboard.
    PickerOpen,
    /// The picker's box, away from any row. Swallowed, so a press on its margin
    /// does not read as a press on the window behind it.
    Picker,
}

impl Hit {
    /// The space a drop here would move a view into.
    pub fn space(self) -> Option<Space> {
        match self {
            Hit::Tab(_, space)
            | Hit::Body(space)
            | Hit::File(_, space) => Some(space),
            _ => None,
        }
    }
}

/// Where a dragged tab ends up when the button comes up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Landing {
    /// Into a space, which moves the view there.
    In(Space),
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
    pub tabs: Vec<(View, Panel)>,
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
    /// Its box, its list, one panel per visible row of that list, and the button
    /// that confirms. All empty when it is not up.
    pub picker: Panel,
    pub picker_list: Panel,
    pub picker_rows: Vec<(usize, Panel)>,
    pub picker_open: Panel,

    /// The floating layer. The open menu's box, and one panel per row, both
    /// empty when no menu is open. Drawn last and hit tested first.
    pub menu: Panel,
    pub menu_rows: Vec<Panel>,
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
}

fn nowhere() -> Panel {
    Panel::new(0.0, 0.0, 0.0, 0.0)
}

fn empty_placed() -> Placed {
    Placed {
        strip: nowhere(),
        body: nowhere(),
        tabs: Vec::new(),
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
                file_list: nowhere(),
                file_diff: nowhere(),
                file_rows: Vec::new(),
                files_in: None,
                input: nowhere(),
                picking: false,
                picker: nowhere(),
                picker_list: nowhere(),
                picker_rows: Vec::new(),
                picker_open: nowhere(),
                menu,
                menu_rows,
            };
        }

        // The picker is the whole window while it is up, and every other region
        // collapses to nothing the way it does when the window is shaded: a
        // stale hit region left behind here would let a click reach a pane that
        // has no agent behind it.
        if let Some(picker) = shape.picker {
            let (box_, list, rows, open) = place_picker(rest.inset(GAP), shape, picker);
            return Layout {
                width,
                height,
                shaded: false,
                title,
                minimize: buttons[0],
                maximize: buttons[1],
                close: buttons[2],
                spaces: [empty_placed(), empty_placed(), empty_placed()],
                file_list: nowhere(),
                file_diff: nowhere(),
                file_rows: Vec::new(),
                files_in: None,
                input: nowhere(),
                picking: true,
                picker: box_,
                picker_list: list,
                picker_rows: rows,
                picker_open: open,
                menu,
                menu_rows,
            };
        }

        let (body, input) = rest.split_bottom(shape.input_h.max(INPUT_H).min(rest.h));
        let body = body.inset(GAP);

        // An empty space gives its room away rather than leaving a hole.
        let has = |space: Space| !shape.dock.slot(space).is_empty();
        let (left, right) = if has(Space::Left) && (has(Space::TopRight) || has(Space::BottomRight))
        {
            let split = (body.w * 0.54).floor() - GAP * 0.5;
            let (left, right) = body.split_left(split);
            (
                left,
                Panel::new(right.x + GAP, right.y, (right.w - GAP).max(1.0), right.h),
            )
        } else if has(Space::Left) {
            (body, nowhere())
        } else {
            (nowhere(), body)
        };

        let folded = |space: Space| shape.dock.slot(space).folded;
        let (top, bottom) = match (has(Space::TopRight), has(Space::BottomRight)) {
            (false, false) => (nowhere(), nowhere()),
            (true, false) => (right, nowhere()),
            (false, true) => (nowhere(), right),
            (true, true) => {
                let top_h = match (folded(Space::TopRight), folded(Space::BottomRight)) {
                    (true, _) => TAB_H,
                    (false, true) => (right.h - TAB_H - GAP).max(TAB_H),
                    (false, false) => ((right.h - GAP) * 0.46).max(TAB_H).floor(),
                };
                let (top, lower) = right.split_top(top_h.min(right.h));
                (
                    top,
                    Panel::new(lower.x, lower.y + GAP, lower.w, (lower.h - GAP).max(0.0)),
                )
            }
        };

        let place = |space: Space, area: Panel| -> Placed {
            if area.w < 1.0 || area.h < 1.0 {
                return empty_placed();
            }
            let (strip, rest) = area.split_top(TAB_H.min(area.h));
            let room = strip;
            let slot = shape.dock.slot(space);
            let tabs = strip_tabs(
                room,
                slot.views.iter().map(|v| v.label().chars().count()),
                shape.column,
            )
            .into_iter()
            .enumerate()
            .map(|(i, panel)| (slot.views[i], panel))
            .collect();
            Placed {
                strip,
                body: if slot.folded {
                    Panel::new(rest.x, rest.y, rest.w, 0.0)
                } else {
                    rest
                },
                tabs,
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
            file_list,
            file_diff,
            file_rows,
            files_in,
            input: input.inset(GAP),
            picking: false,
            picker: nowhere(),
            picker_list: nowhere(),
            picker_rows: Vec::new(),
            picker_open: nowhere(),
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
        for (index, row) in self.menu_rows.iter().enumerate() {
            if row.contains(x, y) {
                return Some(Hit::MenuRow(index));
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
            for (index, panel) in &self.picker_rows {
                if panel.contains(x, y) {
                    return Some(Hit::PickerRow(*index));
                }
            }
            if self.picker_open.w >= 1.0 && self.picker_open.contains(x, y) {
                return Some(Hit::PickerOpen);
            }
            if self.picker.w >= 1.0 && self.picker.contains(x, y) {
                return Some(Hit::Picker);
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
        }
        if let Some(space) = self.files_in {
            for (index, panel) in &self.file_rows {
                if panel.contains(x, y) {
                    return Some(Hit::File(*index, space));
                }
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
        match self.hit(x, y).and_then(Hit::space) {
            Some(space) => Landing::In(space),
            None => Landing::Nowhere,
        }
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

/// Where an open menu's box is, and where each of its rows is inside it.
///
/// Clamped into the window. A menu opened near the right edge or a row from the
/// bottom would otherwise hang off the surface, and the part that hangs off is
/// not merely invisible: no pointer can reach it, so the rows down there cannot
/// be picked at all.
fn place_menu(menu: &Menu, column: f32, width: f32, height: f32) -> (Panel, Vec<Panel>) {
    let column = column.max(1.0);
    let w = (menu.width_chars() + MENU_GUTTER) as f32 * column + MENU_PAD * 2.0;
    let h = menu.rows.len() as f32 * MENU_ROW_H + MENU_PAD * 2.0;
    let x = menu.at.0.min(width - w).max(0.0);
    let y = menu.at.1.min(height - h).max(0.0);
    let rows = (0..menu.rows.len())
        .map(|i| Panel::new(x, y + MENU_PAD + i as f32 * MENU_ROW_H, w, MENU_ROW_H))
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
    text_geometry::heights((0..count).map(|_| 0), 1)
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

/// The folder picker's box, its list, the rows on screen and its button.
///
/// Centred in `area` and no wider than [`PICKER_COLUMNS`], because the thing
/// being read is a column of folder names: stretched across a 2200 pixel window
/// the eye has to travel the whole width to get from a name to the button under
/// it. The list is as tall as it has entries, between two bounds, so a folder
/// with three subfolders is not a mostly empty box.
fn place_picker(area: Panel, shape: &Shape, picker: &Picker) -> (Panel, Panel, Vec<(usize, Panel)>, Panel) {
    if area.w < 1.0 || area.h < 1.0 {
        return (nowhere(), nowhere(), Vec::new(), nowhere());
    }
    let column = shape.pane_column.max(1.0);
    let line = Text::line_for(shape.pane_size);
    let head = PICKER_HEAD_ROWS * line;
    let foot = PICKER_FOOT_ROWS * line;
    let want = picker.rows().len().clamp(PICKER_MIN_ROWS, PICKER_MAX_ROWS);
    let w = (PICKER_COLUMNS as f32 * column + PAD * 2.0).min(area.w);
    let h = (PAD * 2.0 + head + want as f32 * line + GAP + foot).min(area.h);
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
    let rows = (0..window.count)
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
    // Exactly as wide as what it says, and the caption comes from the picker, so
    // the button cannot be a size the text does not fill or say a thing it does
    // not do.
    let caption = ((picker.caption().chars().count() + ROW_ICON_COLUMNS + 2) as f32 * column)
        .min(content.w);
    let open = Panel::new(content.x, content.y + content.h - line, caption, line);
    (box_, list, rows, open)
}

/// Lay tabs left to right at the width their labels need, dropping any that do
/// not fit rather than squeezing them into unreadable slivers.
fn strip_tabs(bar: Panel, widths: impl Iterator<Item = usize>, column: f32) -> Vec<Panel> {
    let mut out = Vec::new();
    let mut x = bar.x;
    for chars in widths {
        let w = (chars as f32 + 3.0) * column;
        if x + w > bar.x + bar.w {
            break;
        }
        out.push(Panel::new(x, bar.y, w, bar.h));
        x += w;
    }
    out
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
    pub onto: Option<Space>,
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

    // Shaded, the strip is the whole window and nothing else is painted, not
    // even the backdrop. A compositor is free to hand back a surface taller
    // than the strip was asked for, and a full-window backdrop under a 30
    // pixel strip is what drew the black bar below it.
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

    for space in Space::ALL {
        space_pane(&mut scene, frame, space);
    }
    input_row(&mut scene, frame);
    dragging(&mut scene, frame);
    overlay(&mut scene, frame);
    scene
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
    for (index, (row, panel)) in menu.rows.iter().zip(&layout.menu_rows).enumerate() {
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
        scene.over_text(Text::rich(runs, text.row(0.0, line), SMALL, tint));
    }
}

fn title_bar(scene: &mut Scene, frame: &Frame) {
    let (skin, layout, state) = (frame.skin, frame.layout, frame.state);
    scene.rect(layout.title.fill(skin.bar));

    // How full the context is, as a hairline along the bottom of the strip.
    // It was a bar of its own at the foot of the window; two pixels at the top
    // of the window says the same thing and costs no rows.
    let gauge = Panel::new(0.0, layout.title.y + layout.title.h - 2.0, layout.width, 2.0);
    scene.rect(gauge.fill(skin.gauge_track));
    let used = state.context_fraction();
    if used > 0.0 {
        scene.rect(Panel::new(0.0, gauge.y, layout.width * used, 2.0).fill(skin.gauge));
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
fn panel_edge(panel: Panel, rgba: [f32; 4]) -> Rect {
    panel_fill(panel, rgba).stroke(1.0)
}

/// One tab of a strip, before its label goes on.
///
/// A tab is not a button. Both states carry the pane's own surface and the same
/// cut corner the pane has, so the tab reads as the top of the pane; what says
/// which one is showing is weight. The showing tab is that surface at full
/// strength with an accent line in the colour of what it holds, the rest are
/// the same colour at a lower alpha. A filled block over a filled strip is what
/// made these look like a row of buttons.
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

fn space_pane(scene: &mut Scene, frame: &Frame, space: Space) {
    let skin = frame.skin;
    let placed = frame.layout.placed(space);
    let slot = frame.dock.slot(space);
    if placed.strip.w < 1.0 {
        return;
    }

    // A space being dragged onto is lit along its whole edge, so a drop target
    // is a place rather than a guess.
    let target = frame.drag.is_some_and(|drag| drag.onto == Some(space));
    // The strip itself is not drawn. It is the window, not a toolbar, and the
    // tabs standing in it are the only thing up here. Its fill and the hairline
    // along its foot were both square, so they ran past the cut corner of the
    // pane below and left a stray stroke there; the pane's own outline is the
    // line now, and it follows the cut.
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
    if slot.folded || placed.body.h < 2.0 {
        return;
    }
    let panel = placed.body;
    scene.rect(panel_fill(panel, skin.panel));
    scene.rect(panel_edge(
        panel,
        if target { skin.edge_focus } else { skin.edge },
    ));

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
        Some(View::Hardware) => gauges(scene, frame, panel, frame.monitor.hardware()),
        // The monitor's own three lists are still called hardware, session and
        // overall. Which readings belong in which pane is a separate change from
        // what the panes are called, so the labels moved here first and the
        // lists behind them keep the names they had.
        Some(View::Context) => context(scene, frame, panel),
        Some(View::Session) => gauges(scene, frame, panel, frame.monitor.overall()),
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
    let (skin, state) = (frame.skin, frame.state);
    let mut runs = Vec::new();
    if state.plan.is_empty() {
        runs.push(Run::tinted("no plan yet", skin.dim));
    }
    for todo in &state.plan {
        let (mark, color) = match todo.state {
            TodoState::Done => ("[x] ", skin.good),
            TodoState::Active => ("[>] ", skin.bright),
            TodoState::Pending => ("[ ] ", skin.dim),
        };
        runs.push(Run::tinted(mark, color));
        runs.push(Run::tinted(&todo.text, color));
        runs.push(Run::plain("\n"));
    }
    text_box(scene, frame, panel, frame.pane_size, runs);
}

/// The fleet: one child per row, and under each the last thing it said.
///
/// A row alone is a name and a word, which for eight children at once tells
/// you nothing about any of them. The second line is where the news is: while
/// a child runs it is that child's own output, and once it ends it is the
/// reason it ended.
fn agents(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let mut runs = Vec::new();
    if state.agents.is_empty() {
        runs.push(Run::tinted("no sub-agents this session", skin.dim));
    }
    for agent in &state.agents {
        runs.push(Run::tinted(format!("{:<9}", agent.label), skin.dim));
        runs.push(Run::tinted(
            format!("{:<10}", agent.state),
            skin.tone(agent.tone),
        ));
        // The tool set says whether this child can change anything, which is
        // the one thing about a detached child worth knowing at a glance.
        if !agent.tools.is_empty() {
            runs.push(Run::tinted(format!("{:<10}", agent.tools), skin.dim));
        }
        runs.push(Run::tinted(clip(&agent.brief, 300), skin.body));
        runs.push(Run::plain("\n"));
        if !agent.last.is_empty() {
            runs.push(Run::tinted(format!("           {}", clip(&agent.last, 300)), skin.dim));
            runs.push(Run::plain("\n"));
        }
    }
    text_box(scene, frame, panel, frame.pane_size, runs);
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
/// filling row by row from the bottom, so a row is 20% and a dot is 2.5%. It
/// replaced ten columns of four small dots in one shared gauge colour, which
/// read as a smear rather than as a level, and an unbounded reading now draws no
/// block at all: it used to draw an empty track, so most of a pane was empty
/// rectangles and the two rows that were filled read as noise. An unbounded row
/// keeps the line pitch, because a tall empty row would push the rows that do
/// have blocks off the bottom of the pane.
fn gauges(scene: &mut Scene, frame: &Frame, panel: Panel, gauges: Vec<Gauge>) {
    let skin = frame.skin;
    let content = panel.inset(PAD);
    let column = frame.pane_column;
    let line = Text::line_for(frame.pane_size);

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

    // As wide as the longest label in this pane, so PREFILL MEAN is not clipped
    // and a pane of short labels does not pay for one that has none.
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
    // the pane's own size, and the block takes what is left, down to a dot two
    // pixels across and never more than half. A block that pushed the number off
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
    // same height per block as one of five, and a block that pushed the last
    // rows off the bottom would be hiding readings to look better.
    let mut dot = (line * 0.34)
        .round()
        .min((room / DOT_COLUMNS as f32 - gap).floor())
        .max(2.0);
    let blocks = gauges
        .iter()
        .filter(|gauge| gauge.fraction().is_some())
        .count() as f32;
    let plain = gauges.len() as f32 - blocks;
    let tall = |dot: f32| {
        let block = dot * DOT_ROWS as f32 + gap * (DOT_ROWS - 1) as f32;
        blocks * (block + 2.0 * gap) + plain * line
    };
    while dot > SMALL_DOT && tall(dot) > content.h {
        dot -= 1.0;
    }
    let cell = dot + gap;
    let block_w = cell * DOT_COLUMNS as f32;
    let block_h = dot * DOT_ROWS as f32 + gap * (DOT_ROWS - 1) as f32;
    // The number beside a block is the thing being read from across the desk, so
    // it is drawn larger than the label. Only beside a block: a pane of numbers
    // all at this size would fit four of them.
    //
    // Never wider than the room left beside the block, though. `1,048,576 /
    // 2,097,152` at one and a half times the pane size does not fit a pane
    // dragged narrow, and a reading clipped halfway through is worse than a
    // smaller one: it reads as a different number. Floored, not rounded, because
    // rounding up is what puts the last character over the edge.
    let beside = (content.w - label_w - block_w - column).max(1.0);
    let big = (frame.pane_size * BIG_READING)
        .min(frame.pane_size * beside / needed)
        .floor()
        .max(frame.pane_size);
    let big_line = Text::line_for(big);
    let pitch = (block_h + 2.0 * gap).max(big_line);
    let read_x = content.x + label_w + block_w + column;

    let mut y = content.y;
    for gauge in &gauges {
        let fraction = gauge.fraction();
        let row_h = if fraction.is_some() { pitch } else { line };
        if y + row_h > content.y + content.h {
            break;
        }
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
            Some(_) => (big, read_x),
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
}

/// This run: what the agent is, where it is working, and what it has spent.
///
/// The first three rows are what came off the title strip when that was cut
/// back to the build stamp. They are readings with labels, which is what they
/// never were up there: the phase, the model and the workspace sat unlabelled
/// on one line with the token budget, and nothing said which was which.
/// The CONTEXT pane: which phase, model and workspace, then its readings.
fn context(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let content = panel.inset(PAD);
    let line = Text::line_for(frame.pane_size);
    let label_w = (LABEL_COLUMNS + 1) as f32 * frame.pane_column;
    let rows: [(&str, String, [u8; 4]); 3] = [
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
    let used = rows.len() as f32 * line + line * 0.5;
    if panel.h - used < line {
        return;
    }
    let below = Panel::new(panel.x, panel.y + used, panel.w, panel.h - used);
    gauges(scene, frame, below, frame.monitor.session());
}

/// Calls that failed, and what was sent to the one that is open.
///
/// One row per line, clipped rather than wrapped. A click is turned into a row
/// by dividing by the line height, so a row that wrapped onto two would expand a
/// different failure than the one under the pointer. The rows themselves come
/// from [`State::debug_rows`], which is also what resolves the click.
fn debug(scene: &mut Scene, frame: &Frame, panel: Panel) {
    let (skin, state) = (frame.skin, frame.state);
    let content = panel.inset(PAD);
    let line = Text::line_for(frame.pane_size);
    let cols = cols_of(panel, frame.pane_column);
    let room = frame.layout.rows(panel, frame.pane_size);
    // One column short of the pane, because `clip` spends one on the ellipsis it
    // adds: a row exactly as wide as the pane would come back one wider and
    // wrap, which is the one thing this pane cannot allow.
    let room_for = cols.saturating_sub(1).max(1);
    for (index, row) in state.debug_rows().iter().take(room).enumerate() {
        let tint = skin.tone(row.tone);
        scene.text(Text::rich(
            vec![Run::tinted(clip(&row.text, room_for), tint)],
            Panel::new(
                content.x,
                content.y + index as f32 * line,
                content.w,
                line,
            ),
            frame.pane_size,
            tint,
        ));
    }
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

    say(
        scene,
        vec![Run::tinted("OPEN A FOLDER", skin.bright)],
        Panel::new(content.x, content.y, content.w, line),
        skin.bright,
    );
    // The folder being listed, in full. The rows under it are names, so this is
    // the only thing on screen saying where in the tree they are.
    say(
        scene,
        vec![Run::tinted(
            clip(&picker.at().display().to_string(), cols),
            skin.body,
        )],
        Panel::new(content.x, content.y + line, content.w, line),
        skin.body,
    );
    // What has been typed, or why the list is empty when it is empty for a
    // reason. A folder with no permission looks exactly like an empty folder
    // otherwise.
    let mut runs = vec![Run::icon(icons::FILTER.to_string(), skin.dim), Run::plain(" ")];
    let tint = match (picker.trouble(), picker.filter()) {
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

    for (index, row) in &layout.picker_rows {
        let Some(entry) = picker.row(*index) else {
            continue;
        };
        let on = *index == picker.cursor();
        if on {
            // The band and the mark the file explorer marks its open row with,
            // rather than a block in a colour of its own: one language for "this
            // is the row you are on" in the whole window.
            scene.rect(row.fill(skin.strip));
            scene.rect(Panel::new(row.x, row.y, MARK_W, row.h).fill(skin.edge_focus));
        }
        let tint = if on { skin.bright } else { skin.body };
        let icon = match entry {
            PickerRow::Here => icons::FOLDER_OPEN,
            PickerRow::Up => icons::UP,
            PickerRow::Recent(_) => icons::RECENT,
            PickerRow::Folder(_) => icons::FOLDER,
        };
        let room = cols.saturating_sub(ROW_ICON_COLUMNS + 1).max(1);
        say(
            scene,
            vec![
                Run::icon(icon.to_string(), tint),
                Run::tinted(format!(" {}", clip(&picker.label(entry), room)), tint),
            ],
            Panel::new(
                row.x + MARK_W + 3.0,
                row.y,
                (row.w - MARK_W - 3.0).max(1.0),
                line,
            ),
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
            content.y + content.h - 2.0 * line,
            content.w,
            line,
        ),
        skin.dim,
    );
    let open = layout.picker_open;
    scene.rect(open.fill(if frame.hot == Some(Hit::PickerOpen) {
        skin.hot
    } else {
        skin.tab_idle
    }));
    scene.rect(open.outline(skin.edge, 1.0));
    say(
        scene,
        vec![
            Run::icon(icons::CONFIRM.to_string(), skin.bright),
            Run::tinted(
                format!(" {}", clip(&picker.caption(), cols)),
                skin.bright,
            ),
        ],
        open.row(3.0, line),
        skin.bright,
    );
}

/// The tab under the pointer while it is being dragged, so the drag has
/// something following it and the drop has somewhere to be aimed.
fn dragging(scene: &mut Scene, frame: &Frame) {
    let Some(drag) = frame.drag else {
        return;
    };
    let skin = frame.skin;
    let label = drag.view.label();
    let w = (label.chars().count() as f32 + 3.0) * frame.column;
    let ghost = Panel::new(drag.at.0 - w * 0.5, drag.at.1 - TAB_H * 0.5, w, TAB_H);
    scene.rect(ghost.fill(skin.bar));
    scene.rect(ghost.outline(skin.edge_focus, 1.0));
    scene.text(Text::rich(
        vec![Run::tinted(label, skin.bright)],
        ghost.row(SMALL * 0.6, Text::line_for(SMALL)),
        SMALL,
        skin.bright,
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
pub fn cols_of(panel: Panel, column: f32) -> usize {
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
pub fn fit_name(path: &str, cols: usize) -> String {
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
            file_labels: files.iter().map(|f| f.to_string()).collect(),
            file_first: first,
            column: 8.0,
            pane_size: 13.0,
            pane_column: 8.0,
            input_h: INPUT_H,
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

    /// The running totals a monitor is sampled against: enough in them that the
    /// pane reading them has something to write, and the same every time so a
    /// test does not depend on the machine's own file.
    fn sample_totals() -> crate::totals::Totals {
        crate::totals::Totals {
            prefilled: 4_200_000,
            generated: 90_000,
            cached: 3_100_000,
            prefill_tokens: 4_000_000,
            prefill_seconds: 1_600.0,
            decode_tokens: 90_000,
            decode_seconds: 3_000.0,
            prefill_rates: vec![2400.0, 2600.0],
            decode_rates: vec![29.0, 31.0, 30.0],
        }
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

    /// What item 21 asked for, read left to right: the orb's slot, the name, the
    /// marker, the version.
    ///
    /// The orb itself belongs to the animation work and is not here yet, so what
    /// is asserted about it is that its room is empty: no text starts inside the
    /// leftmost [`ORB_W`] of the strip, which is what makes adding it later a
    /// draw rather than a relayout.
    #[test]
    fn the_title_strip_reads_orb_then_name_then_marker_then_version() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &[]);
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
            drag,
            hot: None,
            trouble: None,
            selection: None,
            menu: None,
            picker: None,
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
        for space in Space::ALL {
            let placed = out.layout.placed(space);
            for point in [
                (placed.body.x + 4.0, placed.body.y + 4.0),
                (
                    placed.body.x + placed.body.w - 4.0,
                    placed.body.y + placed.body.h - 4.0,
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

    /// A drag has to be visible, and its target has to be named, or a drop is
    /// a guess.
    #[test]
    fn a_dragged_tab_follows_the_pointer_and_lights_its_target() {
        let dock = Dock::new();
        let plain = render(&busy_state(), 1200.0, 800.0, &dock, &["a.rs"]);
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
                onto: Some(Space::Left),
            }),
        );
        assert!(
            dragging.scene.rects.len() > plain.scene.rects.len(),
            "the ghost is drawn"
        );
        // The ghost is where the pointer is.
        let ghost = dragging
            .scene
            .rects
            .iter()
            .map(|r| r.xywh())
            .find(|[x, y, w, h]| {
                *x < 400.0 && *x + *w > 400.0 && *y < 500.0 && *y + *h > 500.0 && *h <= TAB_H + 1.0
            });
        assert!(ghost.is_some(), "no ghost under the pointer");
        // The target space is outlined in the focus colour.
        let target = dragging.layout.placed(Space::Left).body;
        let lit = dragging.scene.rects.iter().any(|r| {
            let [x, y, w, h] = r.xywh();
            (w <= 1.5 || h <= 1.5) && x >= target.x - 0.5 && y >= target.y - 0.5
        });
        assert!(lit, "the target is not outlined");
        let _ = dragging.skin;
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
            drag: None,
            hot: None,
            trouble: None,
            selection: Some(selection),
            menu: None,
            picker: None,
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
            drag: None,
            hot: None,
            trouble: None,
            selection: Some(selection),
            menu: None,
            picker: None,
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
        let totals = sample_totals();
        monitor.sample(&state, &totals);
        monitor.sample(&state, &totals);
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

    #[test]
    fn every_rectangle_is_inside_the_surface() {
        let state = busy_state();
        let dock = Dock::new();
        for (w, h) in [(1400.0, 900.0), (320.0, 240.0)] {
            let out = render(&state, w, h, &dock, &["a.rs"]);
            assert!(!out.scene.rects.is_empty());
            for rect in &out.scene.rects {
                let [x, y, rw, rh] = rect.xywh();
                assert!(x >= 0.0 && y >= 0.0, "{rect:?} at {w}x{h}");
                assert!(
                    x + rw <= w + 0.01 && y + rh <= h + 0.01,
                    "{rect:?} at {w}x{h}"
                );
            }
        }
    }

    /// Each view shows its own thing and not another's.
    #[test]
    fn each_view_shows_its_own_content() {
        let state = busy_state();
        let mut monitor = Monitor::new();
        let totals = sample_totals();
        monitor.sample(&state, &totals);
        monitor.sample(&state, &totals);
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
        // The monitors are four different lists now: the machine, this run,
        // every run, and what failed.
        let hardware = seen(View::Hardware);
        assert!(hardware.contains("CPU") || hardware.contains("RAM"), "{hardware}");
        let context = seen(View::Context);
        assert!(context.contains("TOOL CALLS"), "{context}");
        assert!(context.contains("laguna-s21"), "the model belongs here: {context}");
        assert!(!context.contains("CPU"), "hardware leaked into CONTEXT: {context}");
        let session = seen(View::Session);
        assert!(session.contains("DECODE MID"), "{session}");
        assert!(!session.contains("TOOL CALLS"), "the other pane leaked: {session}");
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
        let totals = sample_totals();
        monitor.sample(&state, &totals);
        monitor.sample(&state, &totals);

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
                file_labels: vec![],
                file_first: 0,
                column,
                pane_size: 13.0,
                pane_column,
                input_h: INPUT_H,
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
                drag: None,
                hot: None,
                trouble: None,
                selection: None,
                menu: None,
                picker: None,
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

    /// Eight dots across and five down, so a row is 20% and a dot is 2.5%. 525
    /// of 1000 tokens is 52.5%, which is two whole rows and five dots of a third,
    /// filling from the bottom the way a level meter does. Every dot is drawn
    /// either way, so the block reads as a block rather than as a scatter.
    ///
    /// This asserted ten columns of four dots in one shared gauge colour, which
    /// is the look that was rejected: it read as a smear, and a pane of them with
    /// an empty track on every unbounded row read as noise.
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
        monitor.sample(&state, &sample_totals());

        let mut dock = Dock::new();
        dock.reveal(View::Context);
        let shape = Shape {
            shaded: false,
            dock: &dock,
            menu: None,
            picker: None,
            file_labels: vec![],
            file_first: 0,
            column: 8.0,
            pane_size: 13.0,
            pane_column: 8.0,
            input_h: INPUT_H,
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
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            menu: None,
            picker: None,
        });

        // CONTEXT is the only bounded reading in this pane with anything in it,
        // and its hue is nobody else's, so filtering by that colour isolates the
        // one block under test.
        let context = monitor
            .session()
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
        assert_eq!(dots.len(), 21, "52.5% of 40 dots");
        assert_eq!(
            of(unlit).len(),
            DOT_COLUMNS * DOT_ROWS - 21,
            "the rest of the block is still drawn, faintly"
        );

        // Rows, not columns: 21 dots is two full rows of eight and five of a
        // third, and the part-filled row is the top one.
        let mut rows: Vec<f32> = dots.iter().map(|[_, y, _, _]| *y).collect();
        rows.sort_by(f32::total_cmp);
        rows.dedup();
        assert_eq!(rows.len(), 3);
        let across = |y: f32| dots.iter().filter(|[_, dy, _, _]| *dy == y).count();
        assert_eq!(
            rows.iter().map(|y| across(*y)).collect::<Vec<_>>(),
            vec![5, DOT_COLUMNS, DOT_COLUMNS],
            "the part-filled row is at the top"
        );
        // Evenly pitched, or the block reads as a random scatter.
        let pitch = rows[1] - rows[0];
        for pair in rows.windows(2) {
            assert!((pair[1] - pair[0] - pitch).abs() < 0.01, "{rows:?}");
        }

        // The number is the metric's colour and bigger than the label beside it.
        let reading = scene
            .texts
            .iter()
            .find(|t| t.runs.iter().any(|r| r.text.contains("525 / 1,000")))
            .expect("the context reading is written out");
        assert_eq!(reading.runs[0].color, Some(ink));
        assert!(reading.size > 13.0, "{} is not large text", reading.size);

        // And an unbounded reading draws no block at all: no track, no dots, and
        // the number where the block would have started.
        let calls = scene
            .texts
            .iter()
            .find(|t| t.runs.iter().any(|r| r.text == "TOOL CALLS"))
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

    /// A pane dragged narrow shrinks the number rather than running it off the
    /// edge. A reading clipped halfway through is worse than a smaller one: it
    /// reads as a different number.
    #[test]
    fn a_reading_shrinks_rather_than_running_off_a_narrow_pane() {
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
        monitor.sample(&state, &sample_totals());
        let mut dock = Dock::new();
        dock.reveal(View::Context);

        let mut sizes = Vec::new();
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
            assert!(reading.size >= 13.0, "{width}: smaller than the label");
            sizes.push(reading.size);
        }
        assert!(
            sizes[1] < sizes[0],
            "the narrow pane drew it just as large: {sizes:?}"
        );
    }

    /// The session monitor carries what the title strip lost: which phase, which
    /// model, which workspace. Labelled, which they never were up there.
    #[test]
    fn the_session_monitor_carries_what_the_title_strip_lost() {
        let state = busy_state();
        let mut monitor = Monitor::new();
        monitor.sample(&state, &sample_totals());
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
        let rows: Vec<&Text> = out
            .scene
            .texts
            .iter()
            .filter(|t| body.contains(t.at.x, t.at.y))
            .collect();
        assert_eq!(rows.len(), state.debug_rows().len());
        for (index, text) in rows.iter().enumerate() {
            let written: String = text.runs.iter().map(|r| r.text.as_str()).collect();
            assert!(
                written.chars().count() <= cols,
                "row {index} is {} columns wide in a pane {cols} wide",
                written.chars().count()
            );
            assert_eq!(text.at.h, line, "row {index} is not one line tall");
        }
        // The long argument was cut rather than wrapped, and it says so.
        let shown = text_of(&out.scene);
        assert!(shown.contains("outside the workspace"), "{shown}");
        assert!(shown.contains('\u{2026}'), "the long argument was not clipped");
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
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            menu: None,
            picker: None,
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
    /// One stroked rectangle covering the whole panel, not four 1px edges
    /// around it: a square outline cannot follow the cut corner, and it used to
    /// cost five rectangles per pane instead of two.
    #[test]
    fn every_space_is_drawn_with_a_border() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &["a.rs"]);
        for space in Space::ALL {
            let panel = out.layout.placed(space).body;
            let over_panel: Vec<_> = out
                .scene
                .rects
                .iter()
                .filter(|r| r.xywh() == [panel.x, panel.y, panel.w, panel.h])
                .collect();
            let strokes: Vec<_> = over_panel
                .iter()
                .filter(|r| r.extra()[3] > 0.0)
                .collect();
            assert_eq!(strokes.len(), 1, "{space:?} is not bordered by one rect");
            assert_eq!(strokes[0].extra()[3], 1.0, "a hairline, not a slab");
            // The fill under it is a second rectangle, and no more than that.
            assert_eq!(over_panel.len(), 2, "{space:?} costs more than fill plus edge");
        }
    }

    /// The cut corner, on the fill and on the border alike. A square fill under
    /// a cut border leaves a triangle of panel colour outside its own edge.
    #[test]
    fn a_panel_is_cut_on_its_top_right_corner_only() {
        let out = render(&busy_state(), 1400.0, 900.0, &Dock::new(), &["a.rs"]);
        let boxes: Vec<Panel> = Space::ALL
            .iter()
            .map(|space| out.layout.placed(*space).body)
            .chain(std::iter::once(out.layout.input))
            .collect();
        for panel in boxes {
            let shaped: Vec<_> = out
                .scene
                .rects
                .iter()
                .filter(|r| r.xywh() == [panel.x, panel.y, panel.w, panel.h])
                .collect();
            assert_eq!(shaped.len(), 2, "{panel:?} is not a fill plus an edge");
            for rect in shaped {
                let [_, chamfer, corners, _] = rect.extra();
                assert_eq!(chamfer, CUT, "{rect:?} is not cut");
                assert_eq!(corners, Rect::TOP_RIGHT as f32, "{rect:?} cuts elsewhere");
            }
        }
    }

    /// Shaded, the window is one strip. Every other region has to be gone, or
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
        }
        assert_eq!(layout.hit(600.0, 400.0), None);
        assert_eq!(layout.hit(600.0, 10.0), Some(Hit::TitleBar));

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
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            menu: None,
            picker: None,
        });
        let text = text_of(&scene);
        assert!(text.contains("WORKING") || text.contains("THINKING"), "{text}");
        assert!(!text.contains("looking at it now"), "no pane content");

        // Nothing is painted below the strip. The layout is computed at the
        // size the compositor gave back, which can be the full window when it
        // refuses to shrink, and a backdrop over that height was a black bar
        // with the gauge hairline stranded in it.
        for rect in &scene.rects {
            let [_, y, _, h] = rect.xywh();
            assert!(
                y + h <= TITLE_H + 0.01,
                "{rect:?} reaches {} past the strip",
                y + h - TITLE_H
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
    /// slivers. This used to be asserted on the file strip, which no longer
    /// exists: a list too long for its pane scrolls now, and
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
        let tabs = &out.layout.placed(Space::Left).tabs;
        assert!(tabs.len() < View::ALL.len(), "every tab fitted");
        for (_, panel) in tabs {
            assert!(panel.w > 20.0, "no slivers: {panel:?}");
        }
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
            drag: None,
            hot,
            trouble: None,
            selection: None,
            menu: Some(menu),
            picker: None,
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
        for (index, row) in layout.menu_rows.iter().enumerate() {
            let (x, y) = middle(*row);
            assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(index)));
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
        let menu = Menu::for_widget((600.0, 400.0), View::Plan, Space::TopRight, true);
        let layout = with_menu(&dock, &menu, 1400.0, 900.0);
        let picked: Vec<Option<Item>> = layout
            .menu_rows
            .iter()
            .map(|row| {
                let (x, y) = middle(*row);
                match layout.hit(x, y) {
                    Some(Hit::MenuRow(index)) => menu.pick(index),
                    other => panic!("{other:?} is not a row"),
                }
            })
            .collect();
        assert_eq!(
            picked,
            vec![None, Some(Item::CopySelection), Some(Item::Close)],
            "settings is drawn and refuses to act"
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
            for (index, row) in layout.menu_rows.iter().enumerate() {
                let (x, y) = middle(*row);
                assert_eq!(layout.hit(x, y), Some(Hit::MenuRow(index)), "{at:?}");
            }
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
            ("Settings", out.skin.dim),
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
            drag: None,
            hot: None,
            trouble: None,
            selection: None,
            menu: Some(&menu),
            picker: None,
        });
        assert!(!scene.over_rects.is_empty(), "the menu box is not drawn");
        let rows: String = scene
            .over_texts
            .iter()
            .flat_map(|t| t.runs.iter().map(|r| r.text.as_str()))
            .collect();
        assert!(rows.contains("Close this widget"), "{rows}");
        // And the strip itself is still the only thing in the base layer: the
        // menu hangs below it, on the overlay, over the desktop.
        for rect in &scene.rects {
            let [_, y, _, h] = rect.xywh();
            assert!(y + h <= TITLE_H + 0.01, "{rect:?} reaches past the strip");
        }
    }

    /// Only a row that can act lights up. Highlighting a greyed one promises
    /// something will happen when the button comes down and it will not.
    #[test]
    fn a_greyed_row_does_not_light_up_under_the_pointer() {
        let dock = Dock::new();
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
        assert_eq!(lit(Some(Hit::MenuRow(0))), 0, "settings is disabled");
        assert_eq!(lit(Some(Hit::MenuRow(2))), 1, "close is not");
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
        let (x, y) = middle(layout.placed(Space::Left).body);
        assert_eq!(layout.landing(x, y), Landing::In(Space::Left));
        let (x, y) = middle(layout.placed(Space::TopRight).tabs[0].1);
        assert_eq!(layout.landing(x, y), Landing::In(Space::TopRight));
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
            drag: None,
            hot,
            trouble: None,
            selection: None,
            menu: None,
            picker: Some(picker),
        });
        Rendered {
            scene,
            layout,
            skin,
        }
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
        // folder, the names inside, and what confirming would do.
        let text = text_of(&out.scene);
        for wanted in [
            "OPEN A FOLDER",
            "/home/hec",
            "/home/hec/workspace/noob-cli",
            "gui",
            "crates",
            "..",
            "OPEN /home/hec/workspace/noob-cli",
            "enter opens",
        ] {
            assert!(text.contains(wanted), "{wanted:?} is not on screen: {text}");
        }

        // The row the cursor is on is banded and marked, the same way the file
        // explorer marks the file it is showing.
        let (index, cursor_row) = layout.picker_rows[0];
        assert_eq!(index, picker.cursor());
        assert!(
            covered(&out, cursor_row, cursor_row.h, out.skin.strip),
            "the cursor's row has no band"
        );
        assert!(
            out.scene.rects.iter().any(|rect| {
                let [x, y, w, _] = rect.xywh();
                (x - cursor_row.x).abs() < 0.01
                    && (y - cursor_row.y).abs() < 0.01
                    && (w - MARK_W).abs() < 0.01
                    && rect.rgba() == out.skin.edge_focus
            }),
            "the cursor's row has no mark down its edge"
        );
        // And no other row is banded, or every row would read as the one.
        let banded = out
            .scene
            .rects
            .iter()
            .filter(|rect| rect.rgba() == out.skin.strip)
            .count();
        assert_eq!(banded, 1, "more than one row is banded");

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

    /// The button lights up under the pointer, because it is the only thing in
    /// the picker a mouse can press that is not a row.
    #[test]
    fn the_confirm_button_lights_up_under_the_pointer() {
        let picker = a_picker(&["gui"], &[]);
        let cold = render_picker(&picker, 1205.0, 791.0, None);
        let warm = render_picker(&picker, 1205.0, 791.0, Some(Hit::PickerOpen));
        let button = cold.layout.picker_open;
        assert!(!covered(&cold, button, button.h, cold.skin.hot));
        assert!(covered(&warm, button, button.h, warm.skin.hot));
        // The caption is drawn inside the button it names, or the two would say
        // different things about what Enter does.
        let inside = warm.scene.texts.iter().any(|text| {
            let line: String = text.runs.iter().map(|run| run.text.as_str()).collect();
            line.contains(&picker.caption())
                && text.at.x >= button.x - 0.01
                && text.at.x + text.at.w <= button.x + button.w + 0.01
        });
        assert!(inside, "the caption is not in the button");
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
}

