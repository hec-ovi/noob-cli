//! Drawing a menu: where its box and rows land, and what is painted in them.
//!
//! The menu is a floating surface with its own size, its own row height and its
//! own gutter, so its numbers and its painter live with the model rather than in
//! the window's shared vocabulary.

use noob_draw::{Panel, Rect, Run, Scene, Text};

use crate::menu::{Menu, MARKER_COLUMNS};
#[allow(clippy::wildcard_imports)]
use crate::view::*;

/// The size a menu's rows are written at, and the size its box is measured in
/// columns of.
///
/// Public because the box's width is a character count times the width of one
/// column at this size, and only the renderer can measure a column. It used to
/// be measured at the title bar's size while being drawn at this one, which at
/// the defaults left every row about 23 pixels short of its own box and put the
/// group chevron most of an inch past the end of its label. Two sizes, two
/// column widths, and the one that owns the geometry is the one the text is in.
pub const MENU_SIZE: f32 = crate::view::SMALL;
/// One row of a menu. Taller than a tab: a tab is read, a menu row is aimed at,
/// and 22 pixels is already tight for a pointer.
pub(crate) const MENU_ROW_H: f32 = 24.0;
/// The border hairline a menu's rows sit flush against, top and bottom: the
/// lit row's band runs to the frame, with no dark strip between them.
pub(crate) const MENU_EDGE: f32 = 1.0;
/// The margin around a menu's rows, top and bottom and on either side of a
/// label. Also what keeps the first row off the pointer that opened it.
pub(crate) const MENU_PAD: f32 = 5.0;
/// Columns every menu row leaves in front of its label for an icon, whether it
/// has one or not, so labels line up in a column instead of stepping in and out
/// with whichever rows happen to be marked.
pub(crate) const MENU_GUTTER: usize = 2;
/// The boxes an open menu puts on screen, and where every row on screen is
/// inside them: the menu's own column, and the widgets flyout while open.
pub(crate) struct MenuPlaces {
    pub(crate) box_: Panel,
    pub(crate) rows: Vec<(usize, Panel)>,
    pub(crate) fly: Panel,
    pub(crate) fly_rows: Vec<(usize, Panel)>,
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
pub(crate) fn place_menu(menu: &Menu, column: f32, width: f32, height: f32) -> MenuPlaces {
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
pub(crate) fn overlay(scene: &mut Scene, frame: &Frame) {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    use crate::config::Config;
    use crate::dock::{Dock, Space, View};
    use crate::design::icons;
    use crate::monitor::Monitor;
    use crate::state::State;
    use crate::style::skin::Skin;

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
}
