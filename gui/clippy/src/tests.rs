//! The window shell's own tests: what a press, a key and a deed do to the App.
//!
//! Its own file rather than three thousand lines under the event loop it
//! proves. A child of the crate root, so it sees the shell's privates.

#[allow(clippy::wildcard_imports)]
use crate::*;
use winit::window::CursorIcon;

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
    assert_eq!(soonest([Some(sample), Some(orb), None, None]), Some(orb));
    assert_eq!(soonest([Some(orb), Some(sample), None, None]), Some(orb));
    assert_eq!(soonest([None, Some(sample), None, None]), Some(sample));
    assert_eq!(soonest([Some(orb), None, None, None]), Some(orb));
    assert_eq!(
        soonest([None, None, None, None]),
        None,
        "an idle window blocks"
    );
    // The wait on an unanswered prompt is a clock like the others: it has
    // to survive an orb that is turning the whole time it runs.
    let answer = now + ANSWER_WAIT;
    assert_eq!(soonest([None, Some(orb), None, Some(answer)]), Some(orb));
    assert_eq!(soonest([None, None, None, Some(answer)]), Some(answer));
}

/// The line a prompt that started nothing leaves behind: what to run next,
/// with nothing in it about versions.
#[test]
fn a_prompt_that_started_no_turn_points_at_the_endpoint() {
    let said = unanswered();
    assert!(said.contains("noob doctor"), "{said}");
    assert!(!said.contains("protocol"), "{said}");
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
        agent_tab: None,
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
    for view in [View::Activity, View::Files, View::Plan, View::Agents] {
        dock.move_view(view, Space::TopRight);
    }
    dock.slot_mut(Space::TopRight).show_at(0);
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
    let mut dock = Dock::new();
    for view in [View::Output, View::Activity, View::Files, View::Plan] {
        dock.move_view(view, Space::TopRight);
    }
    dock.move_view(View::Agents, Space::TopLeft);
    dock.slot_mut(Space::TopRight).show_at(0);
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
fn a_right_click_on_the_settings_table_offers_the_same_two_acts() {
    // A conversation on the settings SESSIONS table gets the picker row's
    // menu: open it, or delete it, while a window is connected.
    let dock = Dock::new();
    let menu = menu_for(
        Some(Hit::SettingsPick(3, 1)),
        (500.0, 400.0),
        &dock,
        false,
        None,
        None,
    )
    .expect("a table row has a menu");
    assert_eq!(menu.target, Target::Kept(3, 1));
    assert_eq!(menu.pick(0), Some(Item::OpenSession));
    assert_eq!(menu.pick(1), Some(Item::DeleteSession(false)));
    // The mark in front of it answers the same way: both regions are the row.
    let marked = menu_for(
        Some(Hit::SettingsMark(3, 1)),
        (500.0, 400.0),
        &dock,
        false,
        None,
        None,
    )
    .expect("the mark is the row too");
    assert_eq!(marked.target, Target::Kept(3, 1));
}

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
    // And the key every editor saves with, which the system prompt's
    // document editor reads and nothing else on the panel does.
    assert_eq!(control_in_settings(Key::Character("s")), Control::Save);
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
    menu.fold(4, &dock);

    // In the window, and out: no tab, no space, nothing walks to it. Dragged
    // somewhere else first, so where it comes back to says something.
    assert!(dock.move_view(View::Files, Space::TopRight));
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
    assert_ne!(home, Space::TopRight, "which is not where it was dragged to");
    assert!(dock.is_sound(), "{dock:?}");

    // And the marks follow, so the row says which way it will go next.
    assert_eq!(
        menu.pick(5 + 7),
        Some(Item::Widget(View::Files, false)),
        "FILES is the eighth widget and it is back in the window"
    );
    toggle_view(&mut dock, &mut menu, View::Files);
    assert_eq!(
        menu.pick(5 + 7),
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
    menu.fold(4, &dock);

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
    menu.fold(4, &dock);
    // Every switchable view: the agent-output one has no switch, opening
    // and closing with the agent it is on instead.
    let switchable: Vec<View> = View::ALL
        .into_iter()
        .filter(|view| *view != View::Agent)
        .collect();
    for view in switchable.iter().copied() {
        assert!(toggle_view(&mut dock, &mut menu, view).hidden);
        assert!(dock.is_sound(), "after {view:?} went out: {dock:?}");
        assert!(dock.is_hidden(view));
    }
    assert!(dock.walk().is_empty(), "the window is empty");
    for space in Space::ALL {
        assert!(dock.slot(space).views.is_empty());
    }
    for view in switchable.iter().copied() {
        assert!(!toggle_view(&mut dock, &mut menu, view).hidden);
        assert!(dock.is_sound(), "after {view:?} came back: {dock:?}");
    }
    assert_eq!(dock.walk().len(), switchable.len());
    // Every row of the list says the widget is in the window again.
    for (step, view) in switchable.iter().copied().enumerate() {
        assert_eq!(
            menu.pick(5 + step),
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
    let on = Config::parse("show_activity = on\nshow_files = on");
    assert!(on.show_activity && on.show_files);
    assert!(!Config::default().show_activity, "both open closed");

    // A change to something else moves neither.
    let bigger = Config::parse("show_activity = on\nshow_files = on\nfont_size = 20");
    assert_eq!(pane_changes(&on, &bigger), Vec::new());

    // And one that does moves only its own.
    let off = Config::parse("show_activity = off\nshow_files = on");
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
    assert!(land(&mut dock, View::Files, Landing::In(Space::TopRight, None)));
    assert_eq!(dock.space_of(View::Files), Some(Space::TopRight));

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
        "show_activity = off\nshow_files = on\nleft_width = 0.30\nleft_width_bottom = 0.70\ntop_height = 0.35\ntop_height_right = 0.65\nsettings_rail = 0.40\n",
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
    menu.fold(4, &dock);
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
        agent_tab: None,
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
    assert_eq!(order(&dock)[0], View::Hardware);

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

    let mut dock = Dock::new();
    dock.reveal(View::Activity);
    let space = dock.space_of(View::Activity).expect("the activity list is in the window");
    let layout = laid_out(&dock, None);
    let size = Config::default().pane_font_size;
    let inner = layout.content(space).inset(9.0);
    let line = noob_draw::Text::line_for(size);
    let row = |n: usize| (inner.x + 2.0, inner.y + (n as f32 + 0.5) * line);
    let call_at = |n: usize| {
        let (x, y) = row(n);
        let spot = spot_in_pane(&layout, space, View::Activity, &state.activity, x, y, size, COLUMN, 0)
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
    let spot = spot_in_pane(&layout, space, View::Activity, &state.activity, x, y, size, COLUMN, 0)
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
    // Full panel: the whole surface under the title strip, a margin in
    // from every edge, rather than a note sized to its lines.
    assert!(box_.w >= W * 0.9, "{box_:?}");
    assert!(box_.y + box_.h >= H * 0.9, "{box_:?}");
    // Its close mark answers for itself, the same close settings has,
    // and its scroll track takes the press the way every track does.
    let close = layout.call_popup_close;
    assert!(close.w >= 1.0, "{close:?}");
    let (x, y) = middle(close);
    assert_eq!(layout.hit(x, y), Some(Hit::CallPopupClose));
    let track = view::scroll_track(box_);
    let (x, y) = middle(track);
    assert_eq!(layout.hit(x, y), Some(Hit::CallPopupScrollbar));

    // A press outside it is not the popup's, so `App::click` closes the
    // popup and stops there.
    let outside = layout.hit(box_.x - 2.0, box_.y + box_.h * 0.5);
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
        agent_tab: None,
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
        spot_in_pane(&layout, space, View::Output, &pane, x, y, SIZE, COLUMN, 0)
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
        spot_in_pane(&layout, space, View::Output, &pane, x, y, SIZE, COLUMN, 0)
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
        spot_in_pane(&layout, space, View::Output, &pane, x, y, SIZE, COLUMN, 0)
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
    let mut dock = Dock::new();
    dock.reveal(View::Files);
    let layout = laid_out(&dock, None);
    let space = dock.space_of(View::Files).expect("the file view is in the window");
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
            let spot = spot_in_pane(&layout, space, View::Files, &pane, x, y, size, COLUMN, 0)
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
    let spot = spot_in_pane(&layout, space, View::Files, &pane, inner.x, y, size, COLUMN, 0)
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
    state.apply(noob_proto::Event::AgentOutput {
        agent_id: "kid".into(),
        line: "looking".into(),
    });
    // Pointed at the child, so the agent-output pane is a scrollback too.
    assert!(state.show_agent(1));
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
        esc_armed: false,
        popup_scroll: [0, 0],
        cursor: (-100.0, -100.0),
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
        // scroll, and that is the one exception.
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

/// A typed /mcp_add lands in the file through the same deed the panel's
/// add card writes: the dispatcher resolves the line and [`deed_on_disk`]
/// couriers it, which is the exact pair the window wires together in
/// [`App::run_command`]. And a deed with nowhere to land answers with a
/// reason rather than nothing.
#[test]
fn a_typed_mcp_add_writes_through_the_panels_own_deed() {
    let dir = std::env::temp_dir().join(format!("no0b-command-add-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let global = dir.join("mcp.json");

    let answer = commands::dispatch(
        "/mcp_add docs npx docs-server --port 9000",
        &agent::Agent::default(),
    );
    let commands::Answer::Do(commands::Act::Deed { deed, .. }) = answer else {
        panic!("an add resolves to a deed");
    };
    deed_on_disk(&deed, None, Some(&global), None).expect("the add lands");
    let written = std::fs::read_to_string(&global).expect("the file is there");
    assert!(written.contains("\"docs\""), "{written}");
    // The writer splits a command line into command and args, the same
    // shape the CLI reads; the whole call is in there, word by word.
    for word in ["npx", "docs-server", "--port", "9000"] {
        assert!(written.contains(&format!("\"{word}\"")), "{written}");
    }

    let refused = deed_on_disk(&deed, None, None, None);
    assert!(refused.is_err(), "nowhere to write is a reason, not silence");
    let _ = std::fs::remove_dir_all(&dir);
}
