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

/// One of the things that can occupy a tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Talk,
    Activity,
    Plan,
    Agents,
    /// GPU, CPU, memory: what the machine is doing.
    Hardware,
    /// Context, tokens and rates: what the session is doing.
    Llm,
    /// The files the agent has touched, with its own inner tab per file.
    Files,
}

impl View {
    /// Every view there is, in the order the palette is indexed by. The window
    /// builds its arrangement from [`Dock::new`] rather than from this, but the
    /// skin reads a view's accent by position here, so the order is part of
    /// what a colour means.
    pub const ALL: [View; 7] = [
        View::Talk,
        View::Activity,
        View::Plan,
        View::Agents,
        View::Hardware,
        View::Llm,
        View::Files,
    ];

    pub fn label(self) -> &'static str {
        match self {
            View::Talk => "TALK",
            View::Activity => "ACTIVITY",
            View::Plan => "PLAN",
            View::Agents => "AGENTS",
            View::Hardware => "HARDWARE",
            View::Llm => "LLM",
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
}

impl Slot {
    pub fn active(&self) -> Option<View> {
        self.views.get(self.active).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.views.is_empty()
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
                    views: vec![View::Talk],
                    active: 0,
                    folded: false,
                },
                Slot {
                    views: vec![
                        View::Activity,
                        View::Plan,
                        View::Agents,
                        View::Hardware,
                        View::Llm,
                    ],
                    active: 0,
                    folded: false,
                },
                Slot {
                    views: vec![View::Files],
                    active: 0,
                    folded: false,
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
    /// Dropping a view back where it already is is a no-op rather than a
    /// reorder: a drag that ends where it started should change nothing.
    pub fn move_view(&mut self, view: View, to: Space) -> bool {
        if self.hidden.contains(&view) || self.space_of(view) == Some(to) {
            return false;
        }
        for space in Space::ALL {
            self.slot_mut(space).remove(view);
        }
        let slot = self.slot_mut(to);
        slot.views.push(view);
        slot.active = slot.views.len() - 1;
        slot.folded = false;
        true
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
    #[cfg(test)]
    fn is_sound(&self) -> bool {
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
        assert_eq!(dock.slot(Space::Left).active(), Some(View::Talk));
        assert_eq!(dock.slot(Space::TopRight).active(), Some(View::Activity));
        assert_eq!(dock.slot(Space::BottomRight).active(), Some(View::Files));
        assert_eq!(dock.walk().len(), View::ALL.len());
    }

    /// The decorative avatar was a view of its own and is not one any more.
    /// Its tab is the thing this asserts is gone: the palette and the walk are
    /// both indexed by `View::ALL`, so a leftover variant would keep a tab and
    /// an accent alive with nothing behind them.
    #[test]
    fn there_are_seven_views_and_none_of_them_is_the_avatar() {
        assert_eq!(View::ALL.len(), 7);
        for view in View::ALL {
            assert_ne!(view.label(), "CLIPPY", "{view:?}");
        }
        let dock = Dock::new();
        assert!(dock.is_sound());
        assert_eq!(dock.slot(Space::BottomRight).views, vec![View::Files]);
    }

    /// The invariant the rest of the window relies on: whatever gets dragged
    /// where, every view is in exactly one space.
    #[test]
    fn every_move_leaves_every_view_in_exactly_one_space() {
        let mut dock = Dock::new();
        let moves = [
            (View::Talk, Space::BottomRight),
            (View::Files, Space::Left),
            (View::Activity, Space::Left),
            (View::Plan, Space::BottomRight),
            (View::Hardware, Space::Left),
            (View::Llm, Space::BottomRight),
            (View::Agents, Space::TopRight),
            (View::Talk, Space::Left),
        ];
        for (view, to) in moves {
            dock.move_view(view, to);
            assert!(dock.is_sound(), "after moving {view:?} to {to:?}: {dock:?}");
        }
        assert_eq!(dock.space_of(View::Talk), Some(Space::Left));
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
            View::Llm,
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
    }

    /// Taking the showing tab away must leave its neighbour showing, not an
    /// index past the end.
    #[test]
    fn removing_the_active_tab_leaves_a_valid_one_showing() {
        let mut dock = Dock::new();
        // Show the last tab, then move it out.
        dock.slot_mut(Space::TopRight).show(View::Llm);
        dock.move_view(View::Llm, Space::Left);
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
        dock.move_view(View::Talk, Space::BottomRight);
        let mut seen = vec![View::Talk];
        let mut at = View::Talk;
        for _ in 0..View::ALL.len() - 1 {
            at = dock.after(at).unwrap();
            assert!(!seen.contains(&at), "{at:?} twice in {seen:?}");
            seen.push(at);
        }
        assert_eq!(seen.len(), View::ALL.len());
        assert_eq!(dock.after(at), Some(View::Talk), "and it wraps");
    }

    /// A view the settings turned off has no tab, nothing walks to it, and it
    /// cannot be dragged back in.
    #[test]
    fn a_hidden_view_is_gone_rather_than_folded() {
        let dock = Dock::hiding(&[View::Files, View::Activity]);
        assert!(dock.is_sound());
        assert_eq!(dock.space_of(View::Files), None);
        assert_eq!(dock.space_of(View::Activity), None);
        assert_eq!(dock.walk().len(), View::ALL.len() - 2);
        assert!(!dock.walk().contains(&View::Files));
        // The space the files were the only occupant of is empty, not broken.
        assert!(dock.slot(Space::BottomRight).is_empty());
        assert_eq!(dock.slot(Space::BottomRight).active(), None);

        let mut dock = dock;
        assert!(!dock.move_view(View::Files, Space::Left));
        assert_eq!(dock.space_of(View::Files), None);
        assert!(dock.is_sound());
        // The views that are on still walk, and still wrap.
        let mut at = View::Talk;
        for _ in 0..dock.walk().len() {
            at = dock.after(at).unwrap();
        }
        assert_eq!(at, View::Talk);
    }

    #[test]
    fn revealing_a_view_shows_it_and_unfolds_its_space() {
        let mut dock = Dock::new();
        dock.slot_mut(Space::TopRight).folded = true;
        assert!(dock.reveal(View::Llm));
        assert_eq!(dock.slot(Space::TopRight).active(), Some(View::Llm));
        assert!(!dock.slot(Space::TopRight).folded);
    }
}
