//! Which view lives in which space, and moving them between spaces.
//!
//! Three spaces: the wide column on the left and two stacked on the right.
//! Every view is a tab in exactly one of them, and dragging its tab onto
//! another space moves it there. That is the whole model. It is deliberately
//! not a general splitter tree: a tree lets you make arrangements nobody wants
//! and costs a drag target for every edge of every node, and three spaces
//! covers reading a conversation beside two things at once, which is what this
//! window is for.
//!
//! The invariant everything else relies on: every view is in exactly one
//! space, always. A move takes it out of where it was before it puts it
//! anywhere, and a space that ends up empty gives its room to its neighbour
//! rather than becoming a hole.
//!
//! Tabs also have an order, and a drop can name a place in it: dropping a tab in
//! front of another puts it there ([`Dock::place_view`]), in the space it is
//! already in or in a different one.

/// One of the things that can occupy a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// What the model said: its prose and its reasoning, streamed.
    Output,
    Activity,
    Plan,
    Agents,
    /// GPU, CPU, memory: what the machine is doing.
    Hardware,
    /// What this run is holding: how full the context is and what has been
    /// asked of it.
    Context,
    /// What this run has cost: tokens in, tokens out, and how fast.
    Session,
    /// Tool calls that failed, and what was sent to them.
    Debug,
    /// The files the agent has touched, listed down the left of the pane with
    /// the open one's diff beside it.
    Files,
}

impl View {
    /// Every view there is, in the order the palette is indexed by. The window
    /// builds its arrangement from [`Dock::new`] rather than from this, but the
    /// skin reads a view's accent by position here, so the order is part of
    /// what a colour means.
    ///
    /// The order has not moved since the views were renamed: OUTPUT was TALK,
    /// CONTEXT was the pane called SESSION and SESSION was the one called
    /// OVERALL, each in the same slot it already had, so nobody's accents
    /// shifted along by one when the labels changed.
    pub const ALL: [View; 9] = [
        View::Output,
        View::Activity,
        View::Plan,
        View::Agents,
        View::Hardware,
        View::Context,
        View::Session,
        View::Debug,
        View::Files,
    ];

    pub fn label(self) -> &'static str {
        match self {
            View::Output => "OUTPUT",
            View::Activity => "ACTIVITY",
            View::Plan => "PLAN",
            View::Agents => "AGENTS",
            View::Hardware => "HARDWARE",
            View::Context => "CONTEXT",
            View::Session => "SESSION",
            View::Debug => "DEBUG",
            View::Files => "FILES",
        }
    }
}

/// Where a space is on screen. Only three, and they are named rather than
/// indexed so a caller cannot ask for the fourth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    Left,
    TopRight,
    BottomRight,
}

impl Space {
    pub const ALL: [Space; 3] = [Space::Left, Space::TopRight, Space::BottomRight];

    fn index(self) -> usize {
        match self {
            Space::Left => 0,
            Space::TopRight => 1,
            Space::BottomRight => 2,
        }
    }
}

/// The views in one space, and which of them is showing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Slot {
    pub views: Vec<View>,
    active: usize,
    /// Collapsed to its tab strip.
    pub folded: bool,
    /// Which of its tabs the strip starts at, when the strip is too narrow to
    /// hold them all. Kept here rather than in the layout because it outlives a
    /// frame, and per space because two strips overflow by different amounts.
    tab_first: usize,
}

impl Slot {
    pub fn active(&self) -> Option<View> {
        self.views.get(self.active).copied()
    }

    /// Which tab is showing, by position. The strip scrolls by index, and it has
    /// to keep the showing tab on screen, so it needs the number rather than the
    /// view.
    pub fn active_index(&self) -> Option<usize> {
        (self.active < self.views.len()).then_some(self.active)
    }

    /// Where its strip starts. A request: the layout clamps it again against the
    /// room the strip actually has, which is the only place that knows.
    pub fn tab_first(&self) -> usize {
        self.tab_first
    }

    /// Start the strip at this tab, and say whether that moved it.
    ///
    /// Clamped to the tabs there are. A space left scrolled past its last tab
    /// would show an empty strip, and the arrow that got it there would have
    /// nothing to walk back to.
    pub fn scroll_tabs(&mut self, to: usize) -> bool {
        let to = to.min(self.views.len().saturating_sub(1));
        let moved = to != self.tab_first;
        self.tab_first = to;
        moved
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
    }

    /// Show the tab at this position, and say whether that moved it.
    ///
    /// What the tab strip's arrows walk with: they count tabs rather than naming
    /// views, because what they walk is the strip.
    pub fn show_at(&mut self, at: usize) -> bool {
        if at >= self.views.len() || at == self.active {
            return false;
        }
        self.active = at;
        true
    }

    /// Show this view, if this space has it.
    pub fn show(&mut self, view: View) -> bool {
        match self.views.iter().position(|v| *v == view) {
            Some(at) => {
                self.active = at;
                true
            }
            None => false,
        }
    }

    /// The next tab along, wrapping. Returns whether there was one to move to.
    pub fn cycle(&mut self) -> bool {
        if self.views.len() < 2 {
            return false;
        }
        self.active = (self.active + 1) % self.views.len();
        true
    }

    fn remove(&mut self, view: View) -> bool {
        let Some(at) = self.views.iter().position(|v| *v == view) else {
            return false;
        };
        self.views.remove(at);
        // Keep showing something: the tab that slid into this one's place, or
        // the last one if this was the end.
        self.active = self.active.min(self.views.len().saturating_sub(1));
        // And keep the strip on a tab that exists. Closing or dragging away the
        // tabs at the end of a scrolled strip is the other way a space ends up
        // scrolled past everything it holds.
        self.tab_first = self.tab_first.min(self.views.len().saturating_sub(1));
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dock {
    slots: [Slot; 3],
    /// Views the settings turned off. Kept rather than merely absent, so a
    /// hidden view cannot be dragged back in and so the soundness check knows
    /// which views it is supposed to find.
    hidden: Vec<View>,
}

impl Default for Dock {
    fn default() -> Dock {
        Dock::new()
    }
}

impl Dock {
    /// The arrangement the window opens with: the conversation on the left,
    /// what the agent is doing above right, what it has touched below.
    pub fn new() -> Dock {
        Dock::hiding(&[])
    }

    /// The same arrangement without the views the settings turned off. A
    /// hidden view is not a folded one: it has no tab and nothing walks to it.
    pub fn hiding(hidden: &[View]) -> Dock {
        let mut dock = Dock::full();
        for view in hidden {
            for space in Space::ALL {
                dock.slot_mut(space).remove(*view);
            }
        }
        dock.hidden = hidden.to_vec();
        dock
    }

    fn full() -> Dock {
        Dock {
            hidden: Vec::new(),
            slots: [
                Slot {
                    views: vec![View::Output],
                    active: 0,
                    folded: false,
                    tab_first: 0,
                },
                Slot {
                    views: vec![
                        View::Activity,
                        View::Plan,
                        View::Agents,
                        View::Hardware,
                        View::Context,
                        View::Session,
                    ],
                    active: 0,
                    folded: false,
                    tab_first: 0,
                },
                // DEBUG opens down here rather than above, where seven tabs are
                // more than the strip can show at the width this window opens
                // at. A strip that overflows can be walked with its arrows now,
                // so nothing up there would be unreachable; six tabs and one
                // pane apiece is simply easier to read than seven tabs and a
                // strip you have to scroll to see them.
                Slot {
                    views: vec![View::Files, View::Debug],
                    active: 0,
                    folded: false,
                    tab_first: 0,
                },
            ],
        }
    }

    pub fn slot(&self, space: Space) -> &Slot {
        &self.slots[space.index()]
    }

    pub fn slot_mut(&mut self, space: Space) -> &mut Slot {
        &mut self.slots[space.index()]
    }

    /// Where a view currently lives.
    pub fn space_of(&self, view: View) -> Option<Space> {
        Space::ALL
            .into_iter()
            .find(|space| self.slot(*space).views.contains(&view))
    }

    /// Move a view into a space, at the end of its tabs, and show it there.
    ///
    /// Dropping a view back into the space it already lives in without naming a
    /// position is a no-op rather than a reorder: a drag that ends where it
    /// started should change nothing. [`Dock::place_view`] is the one that names
    /// a position, and it is what a drop onto a tab strip uses.
    pub fn move_view(&mut self, view: View, to: Space) -> bool {
        if self.space_of(view) == Some(to) {
            return false;
        }
        let end = self.slot(to).views.len();
        self.place_view(view, to, end)
    }

    /// Move a view into a space at a given place among its tabs, and show it
    /// there.
    ///
    /// `at` counts the tabs the target space holds now, before the move, so `0`
    /// is in front of its first tab and `views.len()` is after its last. Past
    /// the end is the end. This is what dropping a tab in front of another one
    /// does, and it works in the space the view is already in as well as in a
    /// different one.
    ///
    /// A drop that names the place the view is already in changes nothing, which
    /// is either half of its own tab: both `at == was` (in front of itself) and
    /// `at == was + 1` (behind itself) leave the strip exactly as it was,
    /// including which tab was showing.
    ///
    /// The view is taken out of wherever it was and put into `to` inside this one
    /// call, so no caller can observe it in two spaces or in none.
    pub fn place_view(&mut self, view: View, to: Space, at: usize) -> bool {
        if self.hidden.contains(&view) {
            return false;
        }
        if self.space_of(view) == Some(to) {
            let slot = self.slot_mut(to);
            let Some(was) = slot.views.iter().position(|v| *v == view) else {
                return false;
            };
            if at == was || at == was + 1 {
                return false;
            }
            slot.views.remove(was);
            // An insertion point beyond the tab that was just taken out has slid
            // one place along with every tab after it. Without this, dragging a
            // tab to the right always lands it one tab short of where it was
            // dropped.
            let at = (if at > was { at - 1 } else { at }).min(slot.views.len());
            slot.views.insert(at, view);
            slot.active = at;
            slot.folded = false;
            return true;
        }
        for space in Space::ALL {
            self.slot_mut(space).remove(view);
        }
        let slot = self.slot_mut(to);
        let at = at.min(slot.views.len());
        slot.views.insert(at, view);
        slot.active = at;
        slot.folded = false;
        true
    }

    /// Take a view out of the window: no tab, no space, nothing walks to it.
    ///
    /// The same state the settings produce at startup, reached at runtime by
    /// closing a widget or by dragging its tab off the window. It goes on the
    /// hidden list in the same move it leaves its space, so the invariant every
    /// view is in exactly one place or hidden holds between the two.
    pub fn hide(&mut self, view: View) -> bool {
        if self.hidden.contains(&view) {
            return false;
        }
        for space in Space::ALL {
            self.slot_mut(space).remove(view);
        }
        self.hidden.push(view);
        true
    }

    /// Put a hidden view back, in the space it opens in by default.
    ///
    /// Its old space is not remembered. A view that was hidden months of window
    /// time ago would come back into an arrangement that has since been dragged
    /// around it, and the default is the one place that is always still there.
    ///
    /// The caller is the settings panel: turning `show_activity` or `show_files`
    /// back on puts that view back while the window is running, rather than on
    /// the next launch. Closing a widget is still one way out, and the way back
    /// for the other seven is the orb launcher.
    pub fn unhide(&mut self, view: View) -> bool {
        let Some(at) = self.hidden.iter().position(|v| *v == view) else {
            return false;
        };
        self.hidden.remove(at);
        let slot = self.slot_mut(Dock::home_of(view));
        slot.views.push(view);
        slot.active = slot.views.len() - 1;
        slot.folded = false;
        true
    }

    /// Where a view lives before anything has been dragged. Read off the full
    /// arrangement rather than written out again, so the two cannot drift.
    ///
    /// Reached only through [`Dock::unhide`].
    fn home_of(view: View) -> Space {
        let full = Dock::full();
        Space::ALL
            .into_iter()
            .find(|space| full.slot(*space).views.contains(&view))
            .unwrap_or(Space::TopRight)
    }

    /// Whether a view is out of the window rather than merely not showing.
    pub fn is_hidden(&self, view: View) -> bool {
        self.hidden.contains(&view)
    }

    /// Show a view wherever it is, unfolding its space.
    pub fn reveal(&mut self, view: View) -> bool {
        let Some(space) = self.space_of(view) else {
            return false;
        };
        let slot = self.slot_mut(space);
        slot.show(view);
        slot.folded = false;
        true
    }

    /// Every view, in the order a keyboard walk should visit them: each space
    /// in turn, each of its tabs in order.
    pub fn walk(&self) -> Vec<View> {
        Space::ALL
            .into_iter()
            .flat_map(|space| self.slot(space).views.clone())
            .collect()
    }

    /// The view after this one in that walk, wrapping.
    pub fn after(&self, view: View) -> Option<View> {
        let order = self.walk();
        let at = order.iter().position(|v| *v == view)?;
        order.get((at + 1) % order.len()).copied()
    }

    /// Whether the arrangement still holds: every view in exactly one space,
    /// and every space showing one of its own.
    ///
    /// Reachable from the rest of the crate's tests, not only from this
    /// module's: anything that moves a view around has to leave the dock sound,
    /// and the one answer to whether it did belongs here rather than written out
    /// again beside each caller.
    #[cfg(test)]
    pub(crate) fn is_sound(&self) -> bool {
        View::ALL.into_iter().all(|view| {
            let want = usize::from(!self.hidden.contains(&view));
            Space::ALL
                .into_iter()
                .filter(|space| self.slot(*space).views.contains(&view))
                .count()
                == want
        }) && Space::ALL.into_iter().all(|space| {
            let slot = self.slot(space);
            slot.views.is_empty() || slot.active < slot.views.len()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_arrangement_holds_every_view_once() {
        let dock = Dock::new();
        assert!(dock.is_sound());
        assert_eq!(dock.slot(Space::Left).active(), Some(View::Output));
        assert_eq!(dock.slot(Space::TopRight).active(), Some(View::Activity));
        assert_eq!(dock.slot(Space::BottomRight).active(), Some(View::Files));
        assert_eq!(dock.walk().len(), View::ALL.len());
    }

    /// The decorative avatar was a view of its own and is not one any more, and
    /// the one LLM monitor is now three. Both are asserted by absence: the
    /// palette and the walk are indexed by `View::ALL`, so a leftover variant
    /// would keep a tab and an accent alive with nothing behind them.
    ///
    /// TALK and OVERALL are gone the same way. They were renamed rather than
    /// removed, so the count is the same and only the labels moved: a tab still
    /// reading either of them means a variant kept its old label.
    #[test]
    fn there_are_nine_views_and_no_avatar_and_no_single_llm_monitor() {
        assert_eq!(View::ALL.len(), 9);
        for view in View::ALL {
            for gone in ["CLIPPY", "LLM", "TALK", "OVERALL"] {
                assert_ne!(view.label(), gone, "{view:?}");
            }
        }
        for wanted in [View::Context, View::Session, View::Debug] {
            assert!(View::ALL.contains(&wanted), "{wanted:?}");
        }
        let dock = Dock::new();
        assert!(dock.is_sound());
        assert_eq!(
            dock.slot(Space::BottomRight).views,
            vec![View::Files, View::Debug]
        );
        assert_eq!(dock.slot(Space::BottomRight).active(), Some(View::Files));
    }

    /// The three labels the rename asked for, each in the slot it already had.
    /// The slot indexes the accent palette and the `view_*` colour keys, so a
    /// variant that shifted along would take another one's colour with it.
    #[test]
    fn the_renamed_views_keep_their_slots() {
        assert_eq!(View::ALL[0], View::Output);
        assert_eq!(View::ALL[5], View::Context);
        assert_eq!(View::ALL[6], View::Session);
        assert_eq!(View::Output.label(), "OUTPUT");
        assert_eq!(View::Context.label(), "CONTEXT");
        assert_eq!(View::Session.label(), "SESSION");
        // No two tabs may read the same, or the swap put one label on two views.
        let mut labels: Vec<&str> = View::ALL.iter().map(|view| view.label()).collect();
        let all = labels.len();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(labels.len(), all, "{labels:?}");
    }

    /// The invariant the rest of the window relies on: whatever gets dragged
    /// where, every view is in exactly one space.
    #[test]
    fn every_move_leaves_every_view_in_exactly_one_space() {
        let mut dock = Dock::new();
        let moves = [
            (View::Output, Space::BottomRight),
            (View::Files, Space::Left),
            (View::Activity, Space::Left),
            (View::Plan, Space::BottomRight),
            (View::Hardware, Space::Left),
            (View::Session, Space::BottomRight),
            (View::Agents, Space::TopRight),
            (View::Output, Space::Left),
        ];
        for (view, to) in moves {
            dock.move_view(view, to);
            assert!(dock.is_sound(), "after moving {view:?} to {to:?}: {dock:?}");
        }
        assert_eq!(dock.space_of(View::Output), Some(Space::Left));
    }

    /// A space can be emptied. It must stay sound and stay usable.
    #[test]
    fn a_space_can_be_emptied_and_refilled() {
        let mut dock = Dock::new();
        for view in [
            View::Activity,
            View::Plan,
            View::Agents,
            View::Hardware,
            View::Context,
            View::Session,
        ] {
            dock.move_view(view, Space::Left);
        }
        assert!(dock.slot(Space::TopRight).is_empty());
        assert_eq!(dock.slot(Space::TopRight).active(), None);
        assert!(dock.is_sound());
        dock.move_view(View::Plan, Space::TopRight);
        assert_eq!(dock.slot(Space::TopRight).active(), Some(View::Plan));
        assert!(dock.is_sound());
    }

    /// A drag that ends where it started changes nothing, including which tab
    /// was showing.
    #[test]
    fn dropping_a_view_where_it_already_is_changes_nothing() {
        let mut dock = Dock::new();
        dock.slot_mut(Space::TopRight).show(View::Agents);
        let before = dock.clone();
        assert!(!dock.move_view(View::Agents, Space::TopRight));
        assert_eq!(dock, before);
        // Naming its own place is the same answer: either half of its own tab is
        // where it already is, so nothing moves and nothing else starts showing.
        let was = dock
            .slot(Space::TopRight)
            .views
            .iter()
            .position(|v| *v == View::Agents)
            .unwrap();
        for at in [was, was + 1] {
            assert!(!dock.place_view(View::Agents, Space::TopRight, at), "{at}");
            assert_eq!(dock, before, "{at}");
        }
    }

    /// Item 23: a tab dropped in front of another takes that place in the strip,
    /// in either direction, and the tab that was dragged is the one showing.
    #[test]
    fn a_tab_dropped_in_front_of_another_takes_its_place() {
        let mut dock = Dock::new();
        let order = |dock: &Dock| dock.slot(Space::TopRight).views.clone();
        let start = order(&dock);
        assert_eq!(start[0], View::Activity);
        assert_eq!(start[4], View::Context);

        // Backwards: the fifth tab in front of the first.
        assert!(dock.place_view(View::Context, Space::TopRight, 0));
        assert!(dock.is_sound());
        assert_eq!(
            order(&dock),
            vec![
                View::Context,
                View::Activity,
                View::Plan,
                View::Agents,
                View::Hardware,
                View::Session,
            ]
        );
        assert_eq!(dock.slot(Space::TopRight).active(), Some(View::Context));

        // Forwards: the same tab back in behind the fourth. The insertion point
        // counts the tabs as they are before the move, so 4 means "in front of
        // whatever is fourth now", which is HARDWARE.
        assert!(dock.place_view(View::Context, Space::TopRight, 4));
        assert!(dock.is_sound());
        assert_eq!(
            order(&dock),
            vec![
                View::Activity,
                View::Plan,
                View::Agents,
                View::Context,
                View::Hardware,
                View::Session,
            ]
        );
        assert_eq!(dock.slot(Space::TopRight).active(), Some(View::Context));

        // And onto the end of its own strip.
        let end = order(&dock).len();
        assert!(dock.place_view(View::Activity, Space::TopRight, end));
        assert_eq!(order(&dock).last(), Some(&View::Activity));
        assert!(dock.is_sound());
    }

    /// The same in a different space: a tab dropped between two tabs of another
    /// space lands between them rather than at the end of them.
    #[test]
    fn a_tab_dropped_into_another_space_lands_where_it_was_dropped() {
        let mut dock = Dock::new();
        assert!(dock.place_view(View::Output, Space::TopRight, 2));
        assert!(dock.is_sound());
        assert_eq!(
            dock.slot(Space::TopRight).views,
            vec![
                View::Activity,
                View::Plan,
                View::Output,
                View::Agents,
                View::Hardware,
                View::Context,
                View::Session,
            ]
        );
        assert_eq!(dock.slot(Space::TopRight).active(), Some(View::Output));
        assert_eq!(dock.space_of(View::Output), Some(Space::TopRight));
        // Its old space is empty rather than still holding it.
        assert!(dock.slot(Space::Left).is_empty());

        // A place past the end is the end, not a panic and not a hole.
        assert!(dock.place_view(View::Output, Space::BottomRight, 99));
        assert!(dock.is_sound());
        assert_eq!(
            dock.slot(Space::BottomRight).views,
            vec![View::Files, View::Debug, View::Output]
        );
        // An empty space takes a drop at any place at all.
        assert!(dock.place_view(View::Output, Space::Left, 7));
        assert_eq!(dock.slot(Space::Left).views, vec![View::Output]);
        assert!(dock.is_sound());
    }

    /// The invariant again, driven by the reorder rather than by the plain move:
    /// whatever place a drop names, every view is still in exactly one space.
    #[test]
    fn every_reorder_leaves_every_view_in_exactly_one_space() {
        let mut dock = Dock::new();
        let drops = [
            (View::Session, Space::TopRight, 0),
            (View::Session, Space::TopRight, 6),
            (View::Files, Space::TopRight, 3),
            (View::Output, Space::BottomRight, 0),
            (View::Debug, Space::Left, 0),
            (View::Debug, Space::Left, 2),
            (View::Plan, Space::BottomRight, 99),
            (View::Activity, Space::TopRight, 1),
        ];
        for (view, to, at) in drops {
            dock.place_view(view, to, at);
            assert!(dock.is_sound(), "after {view:?} into {to:?} at {at}: {dock:?}");
            assert_eq!(dock.space_of(view), Some(to), "{view:?}");
            assert_eq!(dock.walk().len(), View::ALL.len());
        }
        // A view the settings turned off cannot be dropped in by naming a place
        // either.
        assert!(dock.hide(View::Plan));
        assert!(!dock.place_view(View::Plan, Space::Left, 0));
        assert_eq!(dock.space_of(View::Plan), None);
        assert!(dock.is_sound());
    }

    /// A reorder inside one space leaves the strip's own scroll offset pointing
    /// at a tab that exists: the tabs are the same tabs, so nothing may be
    /// dragged past the end of them.
    #[test]
    fn reordering_leaves_the_strip_on_a_tab_that_exists() {
        let mut dock = Dock::new();
        let tabs = dock.slot(Space::TopRight).views.len();
        dock.slot_mut(Space::TopRight).scroll_tabs(tabs - 1);
        assert!(dock.place_view(View::Session, Space::TopRight, 0));
        let slot = dock.slot(Space::TopRight);
        assert_eq!(slot.views.len(), tabs, "a reorder is not a removal");
        assert!(slot.tab_first() < slot.views.len(), "{slot:?}");
        assert!(dock.is_sound());
    }

    /// Taking the showing tab away must leave its neighbour showing, not an
    /// index past the end.
    #[test]
    fn removing_the_active_tab_leaves_a_valid_one_showing() {
        let mut dock = Dock::new();
        // Show the last tab, then move it out.
        dock.slot_mut(Space::TopRight).show(View::Session);
        dock.move_view(View::Session, Space::Left);
        let slot = dock.slot(Space::TopRight);
        assert!(slot.active().is_some());
        assert!(slot.views.contains(&slot.active().unwrap()));
        assert!(dock.is_sound());
    }

    #[test]
    fn cycling_walks_the_tabs_of_one_space_and_wraps() {
        let mut dock = Dock::new();
        let slot = dock.slot_mut(Space::TopRight);
        let first = slot.active().unwrap();
        for _ in 0..slot.views.len() {
            assert!(slot.cycle());
        }
        assert_eq!(slot.active(), Some(first), "it came back round");
        // A space with one tab has nowhere to cycle to.
        assert!(!dock.slot_mut(Space::Left).cycle());
    }

    /// The keyboard walk visits everything, wherever it has been dragged.
    #[test]
    fn the_walk_covers_every_view_and_wraps() {
        let mut dock = Dock::new();
        dock.move_view(View::Files, Space::Left);
        dock.move_view(View::Output, Space::BottomRight);
        let mut seen = vec![View::Output];
        let mut at = View::Output;
        for _ in 0..View::ALL.len() - 1 {
            at = dock.after(at).unwrap();
            assert!(!seen.contains(&at), "{at:?} twice in {seen:?}");
            seen.push(at);
        }
        assert_eq!(seen.len(), View::ALL.len());
        assert_eq!(dock.after(at), Some(View::Output), "and it wraps");
    }

    /// A view the settings turned off has no tab, nothing walks to it, and it
    /// cannot be dragged back in.
    #[test]
    fn a_hidden_view_is_gone_rather_than_folded() {
        // Both of the bottom space's tabs, so that space ends up empty: the
        // debug pane opens down there beside the files.
        let dock = Dock::hiding(&[View::Files, View::Debug, View::Activity]);
        assert!(dock.is_sound());
        assert_eq!(dock.space_of(View::Files), None);
        assert_eq!(dock.space_of(View::Activity), None);
        assert_eq!(dock.walk().len(), View::ALL.len() - 3);
        assert!(!dock.walk().contains(&View::Files));
        // The space those two were the only occupants of is empty, not broken.
        assert!(dock.slot(Space::BottomRight).is_empty());
        assert_eq!(dock.slot(Space::BottomRight).active(), None);

        let mut dock = dock;
        assert!(!dock.move_view(View::Files, Space::Left));
        assert_eq!(dock.space_of(View::Files), None);
        assert!(dock.is_sound());
        // The views that are on still walk, and still wrap.
        let mut at = View::Output;
        for _ in 0..dock.walk().len() {
            at = dock.after(at).unwrap();
        }
        assert_eq!(at, View::Output);
    }

    /// Closing a widget and reopening it, which is the same pair of states the
    /// settings produce at startup, reached one view at a time.
    #[test]
    fn hiding_a_view_takes_it_out_and_unhiding_puts_it_back() {
        let mut dock = Dock::new();
        assert!(dock.hide(View::Plan));
        assert!(dock.is_sound());
        assert!(dock.is_hidden(View::Plan));
        assert_eq!(dock.space_of(View::Plan), None);
        assert!(!dock.walk().contains(&View::Plan));
        assert_eq!(dock.walk().len(), View::ALL.len() - 1);
        // Hidden means out, so a drag cannot put it back either.
        assert!(!dock.move_view(View::Plan, Space::Left));
        assert_eq!(dock.space_of(View::Plan), None);
        // Hiding it twice is not two hidden entries.
        assert!(!dock.hide(View::Plan));
        assert!(dock.is_sound());

        assert!(dock.unhide(View::Plan));
        assert!(dock.is_sound());
        assert!(!dock.is_hidden(View::Plan));
        assert_eq!(dock.space_of(View::Plan), Some(Space::TopRight));
        assert_eq!(dock.slot(Space::TopRight).active(), Some(View::Plan));
        assert_eq!(dock.walk().len(), View::ALL.len());
        assert!(!dock.unhide(View::Plan), "it was not hidden any more");
    }

    /// It comes back where it opens by default, not where it happened to be
    /// dragged before it was closed.
    #[test]
    fn a_view_comes_back_in_the_space_it_opens_in() {
        let mut dock = Dock::new();
        dock.move_view(View::Files, Space::Left);
        assert!(dock.hide(View::Files));
        assert!(dock.unhide(View::Files));
        assert_eq!(dock.space_of(View::Files), Some(Space::BottomRight));
        assert!(dock.is_sound());
    }

    /// Closing the only tab in a space empties it. That is a space with no
    /// tabs, not a broken one: nothing shows, nothing is active, and the next
    /// thing dropped there lands.
    #[test]
    fn hiding_the_last_tab_in_a_space_leaves_it_empty_and_usable() {
        let mut dock = Dock::new();
        assert!(dock.hide(View::Files));
        assert!(dock.hide(View::Debug));
        let slot = dock.slot(Space::BottomRight);
        assert!(slot.is_empty());
        assert_eq!(slot.active(), None);
        assert!(dock.is_sound());
        dock.move_view(View::Plan, Space::BottomRight);
        assert_eq!(dock.slot(Space::BottomRight).active(), Some(View::Plan));
        assert!(dock.is_sound());
    }

    /// Every view closed is an empty window, which has to hold together: the
    /// walk is empty rather than wrong, and one unhide is enough to get back.
    #[test]
    fn hiding_everything_leaves_a_dock_that_still_holds() {
        let mut dock = Dock::new();
        for view in View::ALL {
            assert!(dock.hide(view), "{view:?}");
            assert!(dock.is_sound(), "{view:?}");
        }
        assert!(dock.walk().is_empty());
        assert_eq!(dock.after(View::Output), None);
        for space in Space::ALL {
            assert!(dock.slot(space).is_empty(), "{space:?}");
            assert_eq!(dock.slot(space).active(), None);
        }
        assert!(dock.unhide(View::Output));
        assert_eq!(dock.slot(Space::Left).active(), Some(View::Output));
        assert!(dock.is_sound());
    }

    /// A strip cannot be walked past its last tab. The layout clamps again
    /// against the room a strip actually has, but a number stored here that is
    /// past the end of the views is nonsense on its own terms.
    #[test]
    fn a_strip_cannot_be_scrolled_past_its_last_tab() {
        let mut dock = Dock::new();
        let slot = dock.slot_mut(Space::TopRight);
        let tabs = slot.views.len();
        assert_eq!(slot.tab_first(), 0, "it opens at the first tab");
        assert!(slot.scroll_tabs(2));
        assert_eq!(slot.tab_first(), 2);
        assert!(!slot.scroll_tabs(2), "it was already there");
        assert!(slot.scroll_tabs(99));
        assert_eq!(slot.tab_first(), tabs - 1);
        assert!(slot.scroll_tabs(0));
        assert_eq!(slot.tab_first(), 0);
        // A space with no tabs has nowhere to scroll to rather than an offset of
        // its own.
        let empty = dock.slot_mut(Space::Left);
        empty.views.clear();
        assert!(!empty.scroll_tabs(3));
        assert_eq!(empty.tab_first(), 0);
    }

    /// Closing or dragging away the tabs a strip was scrolled to pulls it back
    /// onto a tab that exists. Left where it was, the space would be scrolled
    /// past everything it holds.
    #[test]
    fn closing_a_tab_pulls_the_strip_back_onto_one_that_exists() {
        let mut dock = Dock::new();
        let tabs = dock.slot(Space::TopRight).views.len();
        dock.slot_mut(Space::TopRight).scroll_tabs(tabs - 1);
        // Closing one tab is one fewer place the strip can start.
        assert!(dock.hide(View::Session));
        assert_eq!(dock.slot(Space::TopRight).tab_first(), tabs - 2);
        // Dragging two more away is two more.
        assert!(dock.move_view(View::Context, Space::Left));
        assert!(dock.move_view(View::Hardware, Space::Left));
        let slot = dock.slot(Space::TopRight);
        assert!(slot.tab_first() < slot.views.len(), "{:?}", slot);
        assert_eq!(slot.tab_first(), slot.views.len() - 1);
        assert!(dock.is_sound());
        // And emptying it leaves no offset behind.
        for view in [View::Activity, View::Plan, View::Agents] {
            assert!(dock.move_view(view, Space::Left));
        }
        assert!(dock.slot(Space::TopRight).is_empty());
        assert_eq!(dock.slot(Space::TopRight).tab_first(), 0);
    }

    /// The strip's arrows count tabs rather than naming views, because what they
    /// walk is the strip.
    #[test]
    fn a_tab_can_be_shown_by_its_position() {
        let mut dock = Dock::new();
        let slot = dock.slot_mut(Space::TopRight);
        assert_eq!(slot.active_index(), Some(0));
        assert!(slot.show_at(3));
        assert_eq!(slot.active_index(), Some(3));
        assert_eq!(slot.active(), Some(slot.views[3]));
        assert!(!slot.show_at(3), "it was already showing");
        assert!(!slot.show_at(99), "there is no tab there");
        assert_eq!(slot.active_index(), Some(3));
        // A space with no tabs shows nothing, rather than showing the first of
        // none.
        let empty = dock.slot_mut(Space::Left);
        empty.views.clear();
        assert_eq!(empty.active_index(), None);
        assert!(!empty.show_at(0));
    }

    #[test]
    fn revealing_a_view_shows_it_and_unfolds_its_space() {
        let mut dock = Dock::new();
        dock.slot_mut(Space::TopRight).folded = true;
        assert!(dock.reveal(View::Session));
        assert_eq!(dock.slot(Space::TopRight).active(), Some(View::Session));
        assert!(!dock.slot(Space::TopRight).folded);
    }
}
