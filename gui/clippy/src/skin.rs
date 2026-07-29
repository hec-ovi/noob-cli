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
use crate::dock::View;
use crate::state::{Kind, Tone};

#[derive(Clone, Copy)]
pub struct Skin {
    /// The window body, behind every pane.
    pub backdrop: [f32; 4],
    /// The title and status bars, which stay green so the window reads as noob.
    pub bar: [f32; 4],
    /// A pane you read: dark, so green text sits on black.
    pub panel: [f32; 4],
    /// A tab strip, a shade darker than the panel under it.
    pub strip: [f32; 4],
    /// The tab that is showing. Lighter than the strip and less solid than it,
    /// so the block stands off the strip whatever is behind the window: over a
    /// dark desktop the colour is what shows it, over a bright one the alpha is.
    pub tab: [f32; 4],
    pub edge: [f32; 4],
    pub edge_focus: [f32; 4],
    pub input: [f32; 4],
    pub caret: [f32; 4],
    pub gauge: [f32; 4],
    pub gauge_track: [f32; 4],
    pub scroll_track: [f32; 4],
    pub scroll_thumb: [f32; 4],
    /// The band behind selected text. Drawn under the glyphs, so it has to be
    /// dark enough that green text still reads on top of it.
    pub select: [f32; 4],
    /// A window button under the pointer.
    pub hot: [f32; 4],
    pub close_hot: [f32; 4],

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

    /// One colour per view, in the order [`View`] declares them. A fill rather
    /// than a text tint: it is drawn as the accent line along the top of the
    /// tab that is showing.
    pub views: [[f32; 4]; 7],

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

/// A colour part of the way to another one. A surface that sits between two of
/// the palette's own colours has to be derived from them, or a theme moves the
/// window and leaves that one surface behind.
fn mix(from: [u8; 3], to: [u8; 3], amount: f32) -> [u8; 3] {
    let amount = amount.clamp(0.0, 1.0);
    let blend = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * amount).round() as u8;
    [
        blend(from[0], to[0]),
        blend(from[1], to[1]),
        blend(from[2], to[2]),
    ]
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
            // Part of the way to the bar, which is the one chrome colour the
            // window already has, so the block belongs to whatever theme set it.
            tab: rgba(mix(config.panel, config.bar, 0.6), o * 0.86),
            edge: rgba(config.dim, 0.65),
            edge_focus: rgba(config.accent, 1.0),
            input: rgba(config.panel, (o + 0.12).min(1.0)),
            caret: rgba(config.accent, 1.0),
            gauge: rgba(config.accent, 1.0),
            gauge_track: rgba(config.dim, 0.35),
            scroll_track: rgba(config.dim, 0.22),
            scroll_thumb: rgba(config.accent, 0.75),
            // Dim rather than accent: the band sits under the text, and a
            // bright one takes the reading surface away from what it selects.
            select: rgba(config.dim, 0.45),
            hot: rgba(config.accent, 0.30),
            close_hot: rgba(config.bad, 0.55),

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

            // Ordered by `View::ALL`, which is the order the config reads them
            // in, so the two cannot drift.
            views: config.views.map(|color| rgba(color, 1.0)),

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
            &mut self.input,
        ] {
            fill[3] = 1.0;
        }
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

    /// The accent a view is marked with. Indexed by position in [`View::ALL`],
    /// the way tools are, so the two tables are read the same way.
    pub fn view(&self, view: View) -> [f32; 4] {
        let at = View::ALL.iter().position(|v| *v == view).unwrap_or(0);
        self.views[at]
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
            skin.input,
        ] {
            assert_eq!(fill[3], 1.0, "{fill:?}");
        }
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

    /// The tab that is showing has to read as a block over any desktop. Lighter
    /// than the strip covers a dark one; less solid than it covers a bright one,
    /// where more of the desktop coming through is what makes it lighter.
    #[test]
    fn the_showing_tab_is_lighter_than_the_strip_it_sits_in() {
        let over_black = |c: [f32; 4]| (c[0] * 0.2 + c[1] * 0.7 + c[2] * 0.1) * c[3];
        for name in crate::config::THEMES {
            let skin = Skin::from(&crate::config::theme(name).expect(name));
            assert!(
                over_black(skin.tab) > over_black(skin.strip),
                "{name}: {:?} is not lighter than {:?}",
                skin.tab,
                skin.strip
            );
            assert!(skin.tab[3] < skin.strip[3], "{name}: and less solid");
        }
    }

    /// The table is indexed by position, so it has to have one entry for every
    /// view or a new view silently takes another one's accent.
    #[test]
    fn the_palette_has_one_entry_per_view() {
        let skin = Skin::default();
        assert_eq!(skin.views.len(), View::ALL.len());
        assert_eq!(skin.view(View::Talk), skin.views[0]);
        assert_eq!(skin.view(View::Files), skin.views[6]);
    }

    /// An accent nobody can tell from its neighbour's says nothing about which
    /// view is showing, which is the whole reason the tint exists.
    #[test]
    fn every_view_accent_is_visible_and_unlike_the_others() {
        let skin = Skin::default();
        let apart = |a: [f32; 4], b: [f32; 4]| {
            (0..3).map(|i| (a[i] - b[i]).abs() * 255.0).sum::<f32>()
        };
        for (i, a) in View::ALL.iter().enumerate() {
            let color = skin.view(*a);
            assert_eq!(color[3], 1.0, "{a:?}");
            assert!(
                color[0] + color[1] + color[2] > 1.5,
                "{a:?} is too dark to see against the strip"
            );
            for b in &View::ALL[i + 1..] {
                let distance = apart(color, skin.view(*b));
                assert!(distance > 60.0, "{a:?} and {b:?} are {distance} apart");
            }
        }
    }

    /// A view hue names the view, so a theme moves the window around it. The
    /// accents are the one part of the palette a preset leaves alone, along
    /// with the tool hues.
    #[test]
    fn a_theme_leaves_the_view_accents_where_they_are() {
        let noob = Skin::default();
        for name in crate::config::THEMES {
            let skin = Skin::from(&crate::config::theme(name).expect(name));
            for view in View::ALL {
                assert_eq!(skin.view(view), noob.view(view), "{name}: {view:?}");
            }
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

    #[test]
    fn plain_syntax_has_no_color_of_its_own() {
        assert!(Skin::default().token(crate::syntax::Token::Plain).is_none());
        assert!(Skin::default().token(crate::syntax::Token::Keyword).is_some());
    }
}
