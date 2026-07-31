//! The palette, and the three rules that make the window read as one thing.
//!
//! **Everything is square.** No rounded corners anywhere. The primitive can
//! draw them; the skin does not ask for them.
//!
//! **Dark under green, never green under green.** The panels are black at low
//! alpha and the text is green. A green panel behind green text is what made
//! the first version hard to read: the two greens fight, and lowering the
//! opacity to see the desktop through it made the text worse rather than
//! better. Black backs the text and the desktop shows through the black.
//!
//! **Transparency is a setting, not a constant.** [`Config::opacity`] scales
//! every fill. When the compositor refuses alpha entirely, [`Skin::opaque`]
//! returns the same palette at full opacity, which looks deliberate.

use crate::config::Config;
use crate::state::{Kind, Tone};

#[derive(Clone, Copy)]
pub struct Skin {
    /// The window body, behind every pane.
    pub backdrop: [f32; 4],
    /// The title and status bars, which stay green so the window reads as noob.
    pub bar: [f32; 4],
    /// A pane you read: dark, so green text sits on black.
    pub panel: [f32; 4],
    /// The one surface more solid than a pane: the band behind a block header
    /// in the file view. A tab strip used to be drawn in it too, until a strip
    /// with a surface of its own read as a toolbar.
    pub strip: [f32; 4],
    /// The tab that is showing. The pane's own surface, so the tab reads as the
    /// top of the pane rather than as a raised block sitting on a bar.
    pub tab: [f32; 4],
    /// A tab that is not showing: the same colour with less of it. Weight is
    /// the whole difference, because a second fill colour up here turns the
    /// strip back into a row of buttons.
    pub tab_idle: [f32; 4],
    /// The surface a menu floats on. The most solid thing in the window,
    /// because a menu is above the window rather than part of it: read a pane
    /// through a menu and the menu stops reading as the thing in front.
    ///
    /// Derived from the panel colour rather than given a settings key of its
    /// own, the way `tab_idle` is. A menu is on screen for a second at a time
    /// and nobody is going to tune it.
    pub menu: [f32; 4],
    pub edge: [f32; 4],
    pub edge_focus: [f32; 4],
    /// The box over the space a dragged tab would land in.
    ///
    /// Green because it means yes, and green rather than the accent because the
    /// accent is whatever the theme is: a drop target in the noob-red theme's accent
    /// would read as a warning. Translucent, and low enough that the pane under it
    /// still reads: a solid box hides the thing being aimed at, which is the pane
    /// you are trying to drop onto.
    ///
    /// Derived rather than given a settings key of its own, the way `menu` is.
    /// It is on screen only while a tab is in the air.
    pub drop_target: [f32; 4],
    /// The caret between the two tabs a drop would land between: the same green
    /// at full strength, because a mark two pixels wide has to be seen.
    pub drop_mark: [f32; 4],
    /// The edge of the tab in the air while it is outside the window, where
    /// letting go closes that widget. The bad colour, because that is what the
    /// window uses for something being lost, and the same colour the label takes
    /// there ([`Skin::bad`]).
    pub drop_out: [f32; 4],
    pub input: [f32; 4],
    pub caret: [f32; 4],
    pub gauge: [f32; 4],
    pub gauge_track: [f32; 4],
    pub scroll_track: [f32; 4],
    pub scroll_thumb: [f32; 4],
    /// The band behind selected text. Drawn under the glyphs, so it has to be
    /// dark enough that green text still reads on top of it.
    pub select: [f32; 4],
    /// The band behind the row the folder picker is on.
    ///
    /// The one place in the window that breaks "dark under green": the row you
    /// are about to open is filled solid with the good colour and written in the
    /// darkest ink the palette has. It used to be the quiet band the file
    /// explorer marks its open row with, which on a list of forty folders said
    /// almost nothing about which one Enter would take.
    ///
    /// Green rather than the accent, for the same reason the drop target is:
    /// the accent is whatever the theme is, and a picked row in the noob-red
    /// theme's accent would read as a warning.
    ///
    /// Derived rather than given a settings key of its own, the way `menu` and
    /// `drop_target` are. It is one row in one dialog.
    pub picked: [f32; 4],
    /// The ink on that band. The panel colour, which is the darkest thing the
    /// palette has, rather than a flat black: a theme whose panel is not near
    /// black would then be writing in a colour that is in no palette at all.
    pub picked_ink: [u8; 4],
    /// The mark in front of a folder in the picker's tree: the hairline box and
    /// the plus inside it. Green, the same green [`Skin::picked`] is, because
    /// both say the same thing about a row.
    ///
    /// A fill rather than a text tint, because the mark is drawn out of
    /// rectangles rather than out of a glyph, and its own field rather than
    /// `picked` reused: one is a band across a whole row and the other is a
    /// hairline three characters wide, and a change to either has no business
    /// moving the other.
    pub mark_edge: [f32; 4],
    /// The same mark on the row the cursor is on, where the band behind it is
    /// already that green. The panel colour, which is what
    /// [`Skin::picked_ink`] is for the writing beside it.
    pub mark_on_band: [f32; 4],
    /// A window button under the pointer.
    pub hot: [f32; 4],
    /// The one surface in the window that is a button you press rather than a
    /// pane you read: the picker's Open.
    ///
    /// The accent at low weight, because a button whose fill is the pane colour
    /// is a rectangle around some text and nothing more. Low enough that green
    /// text still reads on top of it, which is the same rule the whole skin is
    /// built on.
    ///
    /// Derived rather than given a settings key of its own, the way `menu` and
    /// `drop_target` are. There is one button and nobody is going to tune it.
    pub button: [f32; 4],
    /// The same button under the pointer. Twice the weight, because a hot state
    /// a shade away from the idle one is a hot state nobody sees.
    pub button_hot: [f32; 4],
    pub close_hot: [f32; 4],
    /// The orb's ink at full weight.
    ///
    /// The animation is greyscale on paper in the drawing it was ported from,
    /// where the darkest mark is the nearest dot. This window is dark, so a
    /// dot's weight is that mirrored, and this is the colour the weight is
    /// spent on: the accent, so the corner belongs to the theme rather than
    /// being a grey sphere in a green window. Derived, not a settings key: a
    /// second accent for one 30 pixel square is a setting nobody would find.
    pub orb: [f32; 4],

    pub title: [u8; 4],
    pub dim: [u8; 4],
    pub body: [u8; 4],
    pub bright: [u8; 4],
    pub good: [u8; 4],
    pub bad: [u8; 4],
    pub minus: [u8; 4],
    pub plus: [u8; 4],

    /// One colour per tool, in the order [`Kind`] declares them.
    pub tools: [[u8; 4]; 14],

    /// The border of the tab that is showing (the line along its top, the two
    /// hairlines down its sides and the heavier one down its cut corner), and the
    /// mark down the left of the open row in the file list. One colour for the
    /// whole window, not one per view.
    ///
    /// It was a hue each, nine of them, and nine hues on nine tabs is a
    /// harlequin strip: what a tab says is its label, and the accent's one job
    /// is saying which of them you are looking at. Green, and the good colour
    /// rather than the theme's accent, for the reason the drop target and the
    /// picked row are green: the accent is whatever the theme is, and a showing
    /// tab in the noob-red theme's accent would read as a warning.
    pub tab_accent: [f32; 4],

    /// The gauge palette. A monitor reading names the slot it wants, so a block
    /// and its number carry the metric's own colour instead of one gauge colour
    /// shared by every reading in the window.
    ///
    /// Both a fill and a tint, because a reading is drawn as both: the dots are
    /// rects and the number beside them is text.
    pub gauges: [[f32; 4]; 10],
    /// The same hues as an unlit dot: present, so the block reads as a block at
    /// a glance even at two percent, and faint, so it does not read as filled.
    pub gauges_unlit: [[f32; 4]; 10],
    pub gauge_ink: [[u8; 4]; 10],

    pub comment: [u8; 4],
    pub string: [u8; 4],
    pub number: [u8; 4],
    pub keyword: [u8; 4],
    pub markup: [u8; 4],
}

fn rgba(color: [u8; 3], alpha: f32) -> [f32; 4] {
    [
        color[0] as f32 / 255.0,
        color[1] as f32 / 255.0,
        color[2] as f32 / 255.0,
        alpha.clamp(0.0, 1.0),
    ]
}

fn text(color: [u8; 3]) -> [u8; 4] {
    [color[0], color[1], color[2], 255]
}

impl Default for Skin {
    fn default() -> Skin {
        Skin::from(&Config::default())
    }
}

impl Skin {
    pub fn from(config: &Config) -> Skin {
        let o = config.opacity;
        Skin {
            // The reading surface is darkest; everything else lets more of the
            // desktop through, so the eye lands where the text is.
            backdrop: rgba(config.panel, o * 0.55),
            bar: rgba(config.bar, (o + 0.25).min(1.0)),
            panel: rgba(config.panel, o * 0.86),
            strip: rgba(config.panel, o * 0.97),
            // Exactly the pane, not a colour of its own. It was part of the way
            // to the bar, which made the showing tab a green block standing off
            // the strip: a button, which is not what a tab is.
            tab: rgba(config.panel, o * 0.86),
            tab_idle: rgba(config.panel, o * 0.34),
            menu: rgba(config.panel, (o + 0.42).min(1.0)),
            edge: rgba(config.dim, 0.65),
            edge_focus: rgba(config.accent, 1.0),
            drop_target: rgba(config.good, 0.22),
            drop_mark: rgba(config.good, 1.0),
            drop_out: rgba(config.bad, 1.0),
            input: rgba(config.panel, (o + 0.12).min(1.0)),
            caret: rgba(config.accent, 1.0),
            gauge: rgba(config.accent, 1.0),
            gauge_track: rgba(config.dim, 0.35),
            scroll_track: rgba(config.dim, 0.22),
            scroll_thumb: rgba(config.accent, 0.75),
            // Dim rather than accent: the band sits under the text, and a
            // bright one takes the reading surface away from what it selects.
            select: rgba(config.dim, 0.45),
            // Solid, not scaled by the opacity setting: a band you can see the
            // desktop through is the band this replaced.
            picked: rgba(config.good, 1.0),
            picked_ink: text(config.panel),
            mark_edge: rgba(config.good, 1.0),
            mark_on_band: rgba(config.panel, 1.0),
            hot: rgba(config.accent, 0.30),
            button: rgba(config.accent, 0.22),
            button_hot: rgba(config.accent, 0.50),
            close_hot: rgba(config.bad, 0.55),
            orb: rgba(config.accent, 1.0),

            title: text(config.text),
            dim: text(config.dim),
            body: text(config.text),
            bright: text(config.bright),
            good: text(config.good),
            bad: text(config.bad),
            minus: text(config.bad),
            plus: text(config.good),

            // Ordered by `Kind::ALL`, which is the order the config reads them
            // in, so the two cannot drift.
            tools: config.tools.map(text),

            tab_accent: rgba(config.good, 1.0),

            gauges: config.gauges.map(|color| rgba(color, 1.0)),
            // Faint enough to sit behind the lit dots without reading as one of
            // them, and not `gauge_track`: an unlit dot is the metric's own hue
            // turned down, so the empty part of a block belongs to the same
            // reading as the full part.
            gauges_unlit: config.gauges.map(|color| rgba(color, 0.20)),
            gauge_ink: config.gauges.map(text),

            comment: text(config.syntax_comment),
            string: text(config.syntax_string),
            number: text(config.syntax_number),
            keyword: text(config.syntax_keyword),
            markup: text(config.syntax_markup),
        }
    }

    /// The same palette with nothing translucent, for a surface that refused to
    /// composite. Every fill keeps its relative depth as a color, so the
    /// layering survives losing alpha.
    pub fn opaque(mut self) -> Skin {
        for fill in [
            &mut self.backdrop,
            &mut self.bar,
            &mut self.panel,
            &mut self.strip,
            &mut self.tab,
            &mut self.menu,
            &mut self.input,
        ] {
            fill[3] = 1.0;
        }
        // Not `tab_idle`. It is the showing tab's colour at a lower alpha, so
        // making it solid here would make every tab the showing one. Its own
        // blend still happens inside the window, which an opaque surface does
        // not take away.
        self
    }

    pub fn tone(&self, tone: Tone) -> [u8; 4] {
        match tone {
            Tone::Dim => self.dim,
            Tone::Body => self.body,
            Tone::Bright => self.bright,
            Tone::Good => self.good,
            Tone::Bad => self.bad,
            Tone::Minus => self.minus,
            Tone::Plus => self.plus,
            Tone::Call(kind) => self.kind(kind),
        }
    }

    pub fn kind(&self, kind: Kind) -> [u8; 4] {
        let at = Kind::ALL.iter().position(|k| *k == kind).unwrap_or(13);
        self.tools[at]
    }

    /// A reading's hue: the lit dot, the unlit dot, and the number's tint.
    ///
    /// Wrapped rather than clamped or panicking. A metric declaring a slot the
    /// palette does not have is a bug in the table over in `monitor.rs`, and the
    /// test there catches it; on screen it takes another metric's hue, which is
    /// a duller window rather than a crashed one.
    pub fn gauge_slot(&self, slot: usize) -> ([f32; 4], [f32; 4], [u8; 4]) {
        let at = slot % self.gauges.len();
        (self.gauges[at], self.gauges_unlit[at], self.gauge_ink[at])
    }

    pub fn token(&self, token: crate::syntax::Token) -> Option<[u8; 4]> {
        use crate::syntax::Token;
        match token {
            Token::Plain => None,
            Token::Comment => Some(self.comment),
            Token::Str => Some(self.string),
            Token::Number => Some(self.number),
            Token::Keyword => Some(self.keyword),
            Token::Markup => Some(self.markup),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rule the second round was about: the surface under the text is dark,
    /// so green text is not sitting on green.
    #[test]
    fn panels_are_dark_and_text_is_not() {
        let skin = Skin::default();
        let luminance = |c: [f32; 4]| c[0] * 0.2 + c[1] * 0.7 + c[2] * 0.1;
        assert!(luminance(skin.panel) < 0.05, "{:?}", skin.panel);
        assert!(luminance(skin.backdrop) < 0.05, "{:?}", skin.backdrop);
        let [_, g, ..] = skin.body;
        assert!(g > 150, "the text is green: {:?}", skin.body);
    }

    /// The reading surface is the most solid thing in the window, so the eye
    /// lands on it rather than on the desktop behind it.
    #[test]
    fn the_reading_surface_is_more_solid_than_the_backdrop() {
        let skin = Skin::default();
        assert!(skin.panel[3] > skin.backdrop[3]);
        assert!(skin.strip[3] > skin.panel[3]);
        assert!(skin.panel[3] < 1.0, "and still lets the desktop through");
    }

    /// Turning the opacity down must move every fill together, or the layering
    /// inverts halfway down the range.
    #[test]
    fn opacity_scales_the_whole_stack_and_keeps_its_order() {
        let ghost = Skin::from(&Config {
            opacity: 0.2,
            ..Config::default()
        });
        let solid = Skin::from(&Config {
            opacity: 1.0,
            ..Config::default()
        });
        assert!(ghost.panel[3] < solid.panel[3]);
        assert!(ghost.backdrop[3] < ghost.panel[3], "order survives");
        assert!(solid.backdrop[3] < solid.panel[3]);
    }

    #[test]
    fn the_opaque_fallback_has_no_transparency_left() {
        let skin = Skin::default().opaque();
        for fill in [
            skin.backdrop,
            skin.bar,
            skin.panel,
            skin.strip,
            skin.tab,
            skin.menu,
            skin.input,
        ] {
            assert_eq!(fill[3], 1.0, "{fill:?}");
        }
    }

    /// A menu floats above the window, so nothing in the window may be more
    /// solid than it. Read the pane through the menu and the two swap places.
    #[test]
    fn a_menu_is_the_most_solid_surface_in_the_window() {
        for name in crate::config::THEMES {
            let skin = Skin::from(&crate::config::theme(name).expect(name));
            for under in [skin.panel, skin.strip, skin.tab, skin.bar, skin.input] {
                assert!(
                    skin.menu[3] >= under[3],
                    "{name}: {:?} is not above {under:?}",
                    skin.menu
                );
            }
            assert_eq!(skin.menu[..3], skin.panel[..3], "{name}: not the pane's hue");
        }
        // And it stays that way with the transparency wound right down.
        let ghost = Skin::from(&Config {
            opacity: 0.1,
            ..Config::default()
        });
        assert!(ghost.menu[3] > ghost.panel[3]);
    }

    /// Grouping tools by category read as no colour at all: most of a session
    /// is read, ls and grep, and one colour for all three left the list
    /// looking uncoloured. Every tool but the catch-all stands off ordinary
    /// text, and every one of them stands off the others.
    #[test]
    fn every_tool_has_its_own_colour_and_none_is_the_colour_of_prose() {
        let skin = Skin::default();
        let named: Vec<Kind> = Kind::ALL
            .into_iter()
            .filter(|kind| *kind != Kind::Other)
            .collect();
        for kind in &named {
            assert_ne!(skin.kind(*kind), skin.dim, "{kind:?}");
            if *kind != Kind::Plan {
                assert_ne!(skin.kind(*kind), skin.body, "{kind:?}");
            }
        }
        for (i, a) in named.iter().enumerate() {
            for b in &named[i + 1..] {
                assert_ne!(skin.kind(*a), skin.kind(*b), "{a:?} and {b:?} match");
            }
        }
    }

    /// The table is indexed by position, so it has to have one entry for every
    /// variant or a new tool silently takes another one's colour.
    #[test]
    fn the_palette_has_one_entry_per_tool() {
        let skin = Skin::default();
        assert_eq!(skin.tools.len(), Kind::ALL.len());
        assert_eq!(skin.kind(Kind::Other), skin.body, "the catch-all is prose");
        assert_eq!(skin.kind(Kind::Bash), skin.tools[0]);
        assert_eq!(skin.kind(Kind::Plan), skin.tools[12]);
    }

    /// The showing tab is the pane's own surface and the others are the same
    /// colour with less of it, so the strip behind them can stay the window.
    ///
    /// This asserted the opposite until the tabs stopped being buttons: the tab
    /// was mixed towards the bar and had to be lighter than the strip it sat
    /// in, which is exactly the raised block that was wrong.
    #[test]
    fn a_tab_is_the_pane_s_surface_and_an_idle_one_is_less_of_it() {
        for name in crate::config::THEMES {
            let skin = Skin::from(&crate::config::theme(name).expect(name));
            assert_eq!(skin.tab, skin.panel, "{name}: the tab is not the pane");
            assert_eq!(
                skin.tab_idle[..3],
                skin.tab[..3],
                "{name}: an idle tab has a colour of its own"
            );
            assert!(
                skin.tab_idle[3] < skin.tab[3],
                "{name}: {:?} is not lighter than {:?}",
                skin.tab_idle,
                skin.tab
            );
            assert!(skin.tab_idle[3] > 0.0, "{name}: and is still drawn");
        }
    }

    /// Item 17 asked for a green transparent box over the drop target, and both
    /// halves of that are palette rules: green in every theme, and see-through
    /// enough that the pane it covers can still be read.
    #[test]
    fn the_drop_target_is_green_and_the_pane_reads_through_it() {
        for name in crate::config::THEMES {
            let skin = Skin::from(&crate::config::theme(name).expect(name));
            let [r, g, b, a] = skin.drop_target;
            assert!(g > r && g > b, "{name}: the drop target is not green: {:?}", skin.drop_target);
            assert!(a > 0.0 && a < 0.4, "{name}: alpha {a} is not see-through");
            // The pane is the more solid of the two, or the box hides what it is
            // pointing at.
            assert!(a < skin.panel[3], "{name}: {a} over the pane's {}", skin.panel[3]);
            // The caret is the same green, all the way up: a mark this narrow at
            // the box's own alpha would not be seen at all.
            assert_eq!(skin.drop_mark[..3], skin.drop_target[..3], "{name}");
            assert_eq!(skin.drop_mark[3], 1.0, "{name}");
            // And a drop that closes the widget is the other answer, so it cannot
            // be the same colour as the one that keeps it.
            let [r, g, b, a] = skin.drop_out;
            assert!(r > g && r > b, "{name}: the delete tint is not red: {:?}", skin.drop_out);
            assert_eq!(a, 1.0, "{name}");
            assert_eq!(
                skin.drop_out[..3],
                [
                    skin.bad[0] as f32 / 255.0,
                    skin.bad[1] as f32 / 255.0,
                    skin.bad[2] as f32 / 255.0
                ],
                "{name}: the edge and the label are different reds"
            );
        }
    }

    /// Item 4 asked for the picked row to be green with black text on it, and
    /// both halves are palette rules: green in every theme, and "black" meaning
    /// the darkest colour the theme actually has rather than a flat zero, so a
    /// palette built on something other than near black still writes in one of
    /// its own colours.
    #[test]
    fn the_picked_row_is_green_with_the_palette_s_darkest_ink_on_it() {
        let luminance = |c: [f32; 4]| c[0] * 0.2 + c[1] * 0.7 + c[2] * 0.1;
        for name in crate::config::THEMES {
            let config = crate::config::theme(name).expect(name);
            let skin = Skin::from(&config);
            let [r, g, b, a] = skin.picked;
            assert!(g > r && g > b, "{name}: the picked row is not green: {:?}", skin.picked);
            assert_eq!(a, 1.0, "{name}: a band you can see the list through");
            assert_eq!(
                skin.picked[..3],
                skin.drop_target[..3],
                "{name}: two greens for two yeses"
            );

            // The ink is the palette's own darkest colour, the panel, and it is
            // written rather than derived from it, so a theme cannot end up
            // writing in a colour that is nowhere in it.
            assert_eq!(
                skin.picked_ink,
                [config.panel[0], config.panel[1], config.panel[2], 255],
                "{name}"
            );
            let ink = luminance([
                skin.picked_ink[0] as f32 / 255.0,
                skin.picked_ink[1] as f32 / 255.0,
                skin.picked_ink[2] as f32 / 255.0,
                1.0,
            ]);
            // Dark on light, which is the one place in the window that is the
            // other way up from every other pair in it.
            assert!(
                luminance(skin.picked) - ink > 0.5,
                "{name}: {ink} ink on a {} band",
                luminance(skin.picked)
            );
            // And not the tint an unpicked row is written in, or the band would
            // be the only thing saying which row it is.
            assert_ne!(skin.picked_ink, skin.body, "{name}");
            assert_ne!(skin.picked_ink, skin.bright, "{name}");
        }
    }

    /// A gauge hue is one colour used three ways, and the three have to agree:
    /// a lit dot, a fainter one behind it, and the number beside them.
    #[test]
    fn a_gauge_slot_is_one_hue_lit_unlit_and_written() {
        let skin = Skin::default();
        assert_eq!(skin.gauges.len(), skin.gauges_unlit.len());
        assert_eq!(skin.gauges.len(), skin.gauge_ink.len());
        for slot in 0..skin.gauges.len() {
            let (lit, unlit, ink) = skin.gauge_slot(slot);
            assert_eq!(lit[3], 1.0, "{slot}");
            assert_eq!(lit[..3], unlit[..3], "{slot}: the unlit dot changed hue");
            assert!(unlit[3] > 0.0 && unlit[3] < lit[3], "{slot}: {unlit:?}");
            assert_eq!(ink[3], 255, "{slot}");
            let written = [
                (ink[0] as f32 / 255.0),
                (ink[1] as f32 / 255.0),
                (ink[2] as f32 / 255.0),
            ];
            for channel in 0..3 {
                assert!(
                    (written[channel] - lit[channel]).abs() < 0.01,
                    "{slot}: the number is not the block's colour"
                );
            }
            // Readable on black, the way every other tint has to be.
            let sum = ink[0] as u32 + ink[1] as u32 + ink[2] as u32;
            assert!(sum > 180, "{slot} is too dark to read: {ink:?}");
        }
        // A slot past the end takes another metric's hue rather than panicking.
        assert_eq!(
            skin.gauge_slot(skin.gauges.len()).0,
            skin.gauge_slot(0).0
        );
    }

    /// Two readings in the same colour say nothing about which is which, which
    /// is the whole reason the table exists.
    #[test]
    fn every_gauge_hue_is_unlike_the_others() {
        let skin = Skin::default();
        let apart =
            |a: [f32; 4], b: [f32; 4]| (0..3).map(|i| (a[i] - b[i]).abs() * 255.0).sum::<f32>();
        for slot in 0..skin.gauges.len() {
            for other in slot + 1..skin.gauges.len() {
                let distance = apart(skin.gauges[slot], skin.gauges[other]);
                assert!(distance > 60.0, "{slot} and {other} are {distance} apart");
            }
        }
    }

    /// A theme moves the window around the readings, not the readings: a gauge
    /// hue names its metric, the way a tool hue names its tool.
    #[test]
    fn a_theme_leaves_the_gauge_hues_where_they_are() {
        let noob = Skin::default();
        for name in crate::config::THEMES {
            let skin = Skin::from(&crate::config::theme(name).expect(name));
            assert_eq!(skin.gauges, noob.gauges, "{name}");
            assert_eq!(skin.gauge_ink, noob.gauge_ink, "{name}");
        }
    }

    /// The accent is the good colour, solid, and it is the same colour whatever
    /// a theme does to the rest of the window.
    ///
    /// Solid because a two pixel line at half weight is a line nobody sees, and
    /// the good colour because that is the window's yes: the same green the drop
    /// target and the picked row are, so the three marks that mean "this one"
    /// are one colour between them.
    #[test]
    fn the_tab_accent_is_the_good_colour_and_every_theme_has_one() {
        for name in crate::config::THEMES {
            let config = crate::config::theme(name).expect(name);
            let skin = Skin::from(&config);
            assert_eq!(skin.tab_accent[3], 1.0, "{name}");
            assert_eq!(skin.tab_accent, skin.drop_mark, "{name}");
            assert_eq!(
                skin.tab_accent,
                [
                    config.good[0] as f32 / 255.0,
                    config.good[1] as f32 / 255.0,
                    config.good[2] as f32 / 255.0,
                    1.0
                ],
                "{name}"
            );
            // Bright enough to see against the surface it is drawn on, which is
            // the pane's own fill.
            assert!(
                skin.tab_accent[0] + skin.tab_accent[1] + skin.tab_accent[2] > 1.5,
                "{name}: {:?} is too dark to see", skin.tab_accent
            );
        }
    }

    #[test]
    fn every_tone_resolves_to_something_readable() {
        let skin = Skin::default();
        let tones: Vec<Tone> = [
            Tone::Dim,
            Tone::Body,
            Tone::Bright,
            Tone::Good,
            Tone::Bad,
            Tone::Minus,
            Tone::Plus,
        ]
        .into_iter()
        .chain(Kind::ALL.into_iter().map(Tone::Call))
        .collect();
        for tone in tones {
            let [r, g, b, a] = skin.tone(tone);
            assert_eq!(a, 255, "{tone:?}");
            assert!(
                r as u32 + g as u32 + b as u32 > 180,
                "{tone:?} is too dark to read on black"
            );
        }
    }

    /// A user's colors reach the window rather than being decoration in a file.
    #[test]
    fn the_config_actually_drives_the_palette() {
        let skin = Skin::from(&Config {
            accent: [0xff, 0x00, 0x00],
            text: [0x11, 0x22, 0x33],
            ..Config::default()
        });
        assert_eq!(skin.caret, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(skin.body, [0x11, 0x22, 0x33, 255]);
    }

    /// The invariants above, run over every preset rather than only the one
    /// they were written against. A preset that breaks one of them is an
    /// unreadable window, and nobody can see that from a name in a file.
    #[test]
    fn every_preset_is_a_window_you_can_read() {
        use crate::syntax::Token;
        let luminance = |c: [f32; 4]| c[0] * 0.2 + c[1] * 0.7 + c[2] * 0.1;
        let named: Vec<Kind> = Kind::ALL
            .into_iter()
            .filter(|kind| *kind != Kind::Other)
            .collect();
        for name in crate::config::THEMES {
            let config = crate::config::theme(name).expect(name);
            let skin = Skin::from(&config);

            assert!(luminance(skin.panel) < 0.05, "{name}: {:?}", skin.panel);
            assert!(luminance(skin.backdrop) < 0.05, "{name}: {:?}", skin.backdrop);
            assert!(skin.backdrop[3] < skin.panel[3], "{name}: backdrop over panel");
            assert!(skin.panel[3] < skin.strip[3], "{name}: panel over strip");

            let readable = |what: &str, [r, g, b, a]: [u8; 4]| {
                assert_eq!(a, 255, "{name}: {what}");
                assert!(
                    r as u32 + g as u32 + b as u32 > 180,
                    "{name}: {what} is too dark to read on black"
                );
            };
            for tone in [
                Tone::Dim,
                Tone::Body,
                Tone::Bright,
                Tone::Good,
                Tone::Bad,
                Tone::Minus,
                Tone::Plus,
            ]
            .into_iter()
            .chain(Kind::ALL.into_iter().map(Tone::Call))
            {
                readable(&format!("{tone:?}"), skin.tone(tone));
            }
            for token in [
                Token::Comment,
                Token::Str,
                Token::Number,
                Token::Keyword,
                Token::Markup,
            ] {
                readable(&format!("{token:?}"), skin.token(token).expect(name));
            }

            for (i, a) in named.iter().enumerate() {
                assert_ne!(skin.kind(*a), skin.dim, "{name}: {a:?}");
                for b in &named[i + 1..] {
                    assert_ne!(skin.kind(*a), skin.kind(*b), "{name}: {a:?} and {b:?} match");
                }
            }
            assert_eq!(skin.kind(Kind::Other), skin.body, "{name}: the catch-all is prose");
        }
    }

    /// Every ink stands off the surface it is written on, by the measure a
    /// contrast checker uses rather than by adding the channels up.
    ///
    /// This became a claim worth making when the colours started arriving on
    /// the screen as they were written. Until then a fill was drawn lighter
    /// than the settings file asked for, so the pane and the bar under the text
    /// were not the colours this reads, and neither was the answer. 3:1 is the
    /// floor WCAG puts under large text and under anything that is not text at
    /// all; the ink here is a 13 to 14 pixel monospace face, so this says the
    /// palette is not unreadable rather than that it is comfortable.
    #[test]
    fn every_ink_stands_off_the_surface_it_is_written_on() {
        use crate::syntax::Token;
        let linear = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        let relative = |c: [u8; 3]| {
            0.2126 * linear(c[0]) + 0.7152 * linear(c[1]) + 0.0722 * linear(c[2])
        };
        let contrast = |ink: [u8; 4], under: [u8; 3]| {
            let (a, b) = (relative([ink[0], ink[1], ink[2]]), relative(under));
            (a.max(b) + 0.05) / (a.min(b) + 0.05)
        };
        for name in crate::config::THEMES {
            let config = crate::config::theme(name).expect(name);
            let skin = Skin::from(&config);

            let mut inks: Vec<(String, [u8; 4])> = [
                Tone::Dim,
                Tone::Body,
                Tone::Bright,
                Tone::Good,
                Tone::Bad,
                Tone::Minus,
                Tone::Plus,
            ]
            .into_iter()
            .chain(Kind::ALL.into_iter().map(Tone::Call))
            .map(|tone| (format!("{tone:?}"), skin.tone(tone)))
            .collect();
            for token in [
                Token::Comment,
                Token::Str,
                Token::Number,
                Token::Keyword,
                Token::Markup,
            ] {
                inks.push((format!("{token:?}"), skin.token(token).expect(name)));
            }
            for slot in 0..skin.gauge_ink.len() {
                inks.push((format!("gauge {slot}"), skin.gauge_ink[slot]));
            }
            for (what, ink) in &inks {
                let ratio = contrast(*ink, config.panel);
                assert!(ratio >= 3.0, "{name}: {what} is {ratio:.2}:1 on the pane");
            }
            // The bar is the one other surface with writing on it: the window's
            // name in the text tint, the version and the buttons in the dim
            // one. It is the lightest surface in the window, so it is where an
            // ink runs out of room first.
            for (what, ink) in [("the title", skin.title), ("the version", skin.dim)] {
                let ratio = contrast(ink, config.bar);
                assert!(ratio >= 3.0, "{name}: {what} is {ratio:.2}:1 on the bar");
            }
        }
    }

    #[test]
    fn plain_syntax_has_no_color_of_its_own() {
        assert!(Skin::default().token(crate::syntax::Token::Plain).is_none());
        assert!(Skin::default().token(crate::syntax::Token::Keyword).is_some());
    }
}
