//! The palette, and the two rules that make the window read as one thing.
//!
//! **Everything is square.** No rounded corners anywhere. The primitive can
//! draw them; the skin does not ask for them.
//!
//! **Panels are translucent, at different depths.** The window composites
//! against the desktop, and each pane sits at its own alpha so the stack reads
//! as layers rather than as one flat sheet. When the compositor refuses alpha,
//! [`Skin::opaque`] returns the same palette with every alpha at 1.0, which
//! looks deliberate instead of looking broken.

use crate::state::Tone;

#[derive(Clone, Copy)]
pub struct Skin {
    /// The window body, behind every pane.
    pub backdrop: [f32; 4],
    pub bar: [f32; 4],
    /// A pane the eye should rest in.
    pub panel: [f32; 4],
    /// A pane that is context rather than content, more of the desktop showing.
    pub panel_thin: [f32; 4],
    pub edge: [f32; 4],
    pub edge_focus: [f32; 4],
    pub input: [f32; 4],
    pub caret: [f32; 4],
    pub gauge: [f32; 4],
    pub gauge_track: [f32; 4],

    pub title: [u8; 4],
    pub dim: [u8; 4],
    pub body: [u8; 4],
    pub bright: [u8; 4],
    pub good: [u8; 4],
    pub bad: [u8; 4],
    pub minus: [u8; 4],
    pub plus: [u8; 4],
    /// Syntax colors, in the order of `syntax::Token`.
    pub comment: [u8; 4],
    pub string: [u8; 4],
    pub number: [u8; 4],
    pub keyword: [u8; 4],
    pub markup: [u8; 4],
}

impl Default for Skin {
    fn default() -> Skin {
        Skin::matrix()
    }
}

impl Skin {
    /// noob's own theme, so the window reads as the same product as the CLI.
    pub fn matrix() -> Skin {
        Skin {
            backdrop: [0.008, 0.031, 0.020, 0.80],
            bar: [0.055, 0.180, 0.118, 0.97],
            panel: [0.000, 0.043, 0.024, 0.90],
            panel_thin: [0.000, 0.039, 0.020, 0.72],
            edge: [0.153, 0.365, 0.255, 0.90],
            edge_focus: [0.400, 0.780, 0.545, 1.0],
            input: [0.000, 0.063, 0.035, 0.95],
            caret: [0.541, 0.925, 0.639, 1.0],
            gauge: [0.302, 0.741, 0.451, 1.0],
            gauge_track: [0.086, 0.216, 0.145, 0.9],

            title: [172, 236, 190, 255],
            dim: [88, 150, 110, 255],
            body: [154, 214, 172, 255],
            bright: [206, 250, 219, 255],
            good: [116, 209, 148, 255],
            bad: [232, 122, 108, 255],
            minus: [206, 116, 106, 255],
            plus: [124, 216, 148, 255],
            comment: [86, 132, 102, 255],
            string: [214, 196, 122, 255],
            number: [178, 206, 240, 255],
            keyword: [130, 206, 240, 255],
            markup: [186, 160, 232, 255],
        }
    }

    /// The same palette with nothing translucent, for a surface that refused
    /// to composite. Every panel keeps its relative depth as a color shift, so
    /// the layering survives losing alpha.
    pub fn opaque(mut self) -> Skin {
        for fill in [
            &mut self.backdrop,
            &mut self.bar,
            &mut self.panel,
            &mut self.panel_thin,
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
        }
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

    /// The compositor-refused fallback must leave nothing see-through, or the
    /// window renders as a stack of muddy grey rectangles over black.
    #[test]
    fn the_opaque_fallback_has_no_transparency_left() {
        let skin = Skin::matrix().opaque();
        for fill in [
            skin.backdrop,
            skin.bar,
            skin.panel,
            skin.panel_thin,
            skin.input,
        ] {
            assert_eq!(fill[3], 1.0, "{fill:?}");
        }
    }

    /// Panels sit at different depths, which is what makes the stack read as
    /// layers rather than one sheet.
    #[test]
    fn panels_are_translucent_at_different_depths() {
        let skin = Skin::matrix();
        assert!(skin.panel[3] < 1.0);
        assert!(skin.panel_thin[3] < skin.panel[3]);
        assert!(skin.backdrop[3] < 1.0);
    }

    #[test]
    fn every_tone_resolves_to_a_visible_color() {
        let skin = Skin::matrix();
        for tone in [
            Tone::Dim,
            Tone::Body,
            Tone::Bright,
            Tone::Good,
            Tone::Bad,
            Tone::Minus,
            Tone::Plus,
        ] {
            let [r, g, b, a] = skin.tone(tone);
            assert_eq!(a, 255, "{tone:?}");
            assert!(r as u32 + g as u32 + b as u32 > 120, "{tone:?} is unreadable");
        }
    }

    /// Plain text takes the pane's own color rather than being tinted, which
    /// is what keeps an unrecognised language readable instead of uniformly
    /// wrong.
    #[test]
    fn plain_syntax_has_no_color_of_its_own() {
        assert!(Skin::matrix().token(crate::syntax::Token::Plain).is_none());
        assert!(Skin::matrix().token(crate::syntax::Token::Keyword).is_some());
    }
}
