//! What the desktop needs in order to show this window as an application.
//!
//! There is no code here, only the checks that keep three files agreeing with
//! each other. They agree by convention rather than by construction, and the
//! failure is silent: the window opens, works perfectly, and wears a generic
//! grey icon in the dock forever, with nothing anywhere saying why.
//!
//! The chain is: the window announces a name, the desktop looks for a
//! `.desktop` file whose basename is that name, reads the `Icon=` key out of
//! it, and looks for an icon file of that name in the theme. Break any link
//! and the icon is gone.
//!
//! On Wayland this is the ONLY path. `winit` documents window icons as
//! unsupported there, so nothing the running program does can put a picture on
//! its own window; the installed files are the whole mechanism.

/// The desktop entry, checked against the code rather than trusted.
#[cfg(test)]
const DESKTOP: &str = include_str!("../../data/io.github.hec_ovi.CLIppy.desktop");
#[cfg(test)]
const ICON: &str = include_str!("../../data/io.github.hec_ovi.CLIppy.svg");
#[cfg(test)]
const SYMBOLIC: &str = include_str!("../../data/io.github.hec_ovi.CLIppy-symbolic.svg");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::APP_ID;

    fn key<'a>(text: &'a str, name: &str) -> &'a str {
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .unwrap_or_else(|| panic!("the desktop entry has no {name} key"))
            .trim()
    }

    /// The whole chain, asserted end to end. Every one of these is a link that
    /// breaks the icon silently if it stops matching.
    #[test]
    fn the_window_the_entry_and_the_icon_all_answer_to_one_name() {
        // The desktop matches a window to an entry by this name, and the
        // entry's own filename is that name too.
        assert_eq!(key(DESKTOP, "StartupWMClass"), APP_ID);
        // The entry names its icon, and the installer writes a file called
        // exactly that into the theme.
        assert_eq!(key(DESKTOP, "Icon"), APP_ID);
        // And the entry runs the binary the installer put on PATH.
        assert!(key(DESKTOP, "Exec").starts_with("clippy"), "{DESKTOP}");
    }

    /// A malformed entry is ignored in full, so the parts that make it valid
    /// are worth pinning.
    #[test]
    fn the_desktop_entry_is_a_valid_one() {
        assert!(DESKTOP.starts_with("[Desktop Entry]\n"), "{DESKTOP}");
        assert_eq!(key(DESKTOP, "Type"), "Application");
        assert_eq!(key(DESKTOP, "Name"), "CLIppy");
        assert_eq!(key(DESKTOP, "Terminal"), "false");
        // A category the menus actually have. An invented one files the app
        // under nothing at all.
        assert!(key(DESKTOP, "Categories").starts_with("Development;"));
        // Keys are one per line with no spaces around the equals sign.
        for line in DESKTOP.lines().filter(|l| l.contains('=')) {
            assert!(!line.contains(" ="), "{line:?}");
            assert!(!line.contains("= "), "{line:?}");
        }
    }

    /// One flat fill and one path. Not a style rule: a gradient or a second
    /// colour is what stops a mark reading at 16 pixels, and a stroke is what
    /// puts its edges on half pixels.
    #[test]
    fn the_icon_is_one_filled_path() {
        assert_eq!(ICON.matches("<path").count(), 1, "{ICON}");
        assert_eq!(ICON.matches("fill=").count(), 1);
        assert!(!ICON.contains("stroke"), "strokes do not scale cleanly");
        assert!(!ICON.contains("Gradient"), "a gradient carries no information here");
        assert!(ICON.contains(r#"viewBox="0 0 128 128""#), "{ICON}");
    }

    /// Every coordinate is a whole pixel at 16, 32, 64 and 128, which is what
    /// a module of 8 on a 128 canvas buys. One coordinate off the module and
    /// that edge renders as two grey rows at the small sizes.
    #[test]
    fn every_coordinate_lands_on_the_module() {
        for value in path_numbers(ICON) {
            assert_eq!(
                value % 8.0,
                0.0,
                "{value} is off the 8 module in {}",
                path_of(ICON)
            );
        }
    }

    /// The small icon is redrawn on its own grid rather than scaled onto it.
    /// Scaling the 128 drawing to 16 puts every edge on a half pixel; drawing
    /// it again at 16 puts them all on whole ones.
    #[test]
    fn the_small_icon_is_drawn_at_its_own_size() {
        assert!(SYMBOLIC.contains(r#"viewBox="0 0 16 16""#), "{SYMBOLIC}");
        assert_eq!(SYMBOLIC.matches("<path").count(), 1);
        for value in path_numbers(SYMBOLIC) {
            assert_eq!(value.fract(), 0.0, "{value} is not a whole pixel at 16");
            assert!((0.0..=16.0).contains(&value), "{value} is outside the canvas");
        }
        // Not the same path with the numbers divided, which would be a scale.
        assert_ne!(path_of(SYMBOLIC), path_of(ICON));
    }

    /// The mark keeps a margin, because a silhouette that touches the canvas
    /// edge is cropped by some desktops and looks oversized next to every
    /// other icon in the dock.
    #[test]
    fn the_mark_does_not_touch_the_edges() {
        let values = path_points(ICON);
        let low = values.iter().cloned().fold(f64::MAX, f64::min);
        let high = values.iter().cloned().fold(f64::MIN, f64::max);
        assert!(low >= 8.0, "starts at {low}");
        assert!(high <= 120.0, "reaches {high}");
    }

    /// The one that is easy to get wrong by taste. The interface accent is
    /// tuned for black panels and disappears on white, so the icon uses a
    /// darker green that clears 3:1 on both grounds and needs no plate behind
    /// it. If somebody "corrects" the icon back to the interface green, this
    /// fails and says why.
    #[test]
    fn the_icon_colour_survives_a_light_dock_and_a_dark_one() {
        let fill = ICON
            .split("fill=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("the icon has a fill");
        let on_white = contrast(fill, "#ffffff");
        let on_black = contrast(fill, "#000000");
        assert!(on_white >= 3.0, "{fill} is {on_white:.2}:1 on white");
        assert!(on_black >= 3.0, "{fill} is {on_black:.2}:1 on black");
    }

    fn path_of(svg: &str) -> String {
        svg.split(" d=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("the icon has a path")
            .to_string()
    }

    /// The path split into commands and their parameters.
    fn commands(svg: &str) -> Vec<(char, Vec<f64>)> {
        let data = path_of(svg);
        let mut out: Vec<(char, Vec<f64>)> = Vec::new();
        for token in data.split([' ', ',']).filter(|t| !t.is_empty()) {
            let head = token.chars().next().expect("a token has a character");
            if head.is_ascii_alphabetic() {
                out.push((head, Vec::new()));
                if token.len() > 1 {
                    let rest = token[1..].parse().expect("path data is numbers");
                    out.last_mut().expect("just pushed").1.push(rest);
                }
                continue;
            }
            let value = token.parse().expect("path data is numbers");
            out.last_mut().expect("a number before any command").1.push(value);
        }
        out
    }

    /// Every geometric number: coordinates and radii, never the arc flags.
    ///
    /// An arc is `rx ry rotation large-arc sweep x y`, so three of its seven
    /// parameters are not lengths at all. Measuring them would flag every
    /// well-drawn icon and pass every badly drawn one.
    fn path_numbers(svg: &str) -> Vec<f64> {
        let mut out = Vec::new();
        for (command, params) in commands(svg) {
            match command {
                'A' => {
                    for chunk in params.chunks(7) {
                        out.extend([chunk[0], chunk[1], chunk[5], chunk[6]]);
                    }
                }
                'Z' => {}
                _ => out.extend(params),
            }
        }
        out
    }

    /// Only the points the path passes through, for measuring the silhouette.
    fn path_points(svg: &str) -> Vec<f64> {
        let mut out = Vec::new();
        for (command, params) in commands(svg) {
            match command {
                'A' => {
                    for chunk in params.chunks(7) {
                        out.extend([chunk[5], chunk[6]]);
                    }
                }
                'Z' => {}
                _ => out.extend(params),
            }
        }
        out
    }

    fn contrast(a: &str, b: &str) -> f64 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// WCAG relative luminance, which is the number both the desktop
    /// guidelines and the accessibility rules are written against.
    fn luminance(hex: &str) -> f64 {
        let hex = hex.trim_start_matches('#');
        let channel = |at: usize| {
            let raw = u8::from_str_radix(&hex[at..at + 2], 16).expect("a hex colour");
            let v = f64::from(raw) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4)
    }
}
