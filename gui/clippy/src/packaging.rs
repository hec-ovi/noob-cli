//! What the desktop needs in order to show this window as an application.
//!
//! Almost all of it is checks that keep three files agreeing with each other.
//! They agree by convention rather than by construction, and the failure is
//! silent: the window opens, works perfectly, and wears a generic grey icon in
//! the dock forever, with nothing anywhere saying why.
//!
//! The chain is: the window announces a name, the desktop looks for a
//! `.desktop` file whose basename is that name, reads the `Icon=` key out of
//! it, and looks for an icon file of that name in the theme. Break any link
//! and the icon is gone.
//!
//! On Wayland this is the ONLY path. `winit` documents window icons as
//! unsupported there, so nothing the running program does can put a picture on
//! its own window; the installed files are the whole mechanism.
//!
//! The one piece of code is the other half of a launch: the desktop hands a
//! started application a token naming the click that started it, and expects
//! the token back on the window it opens.

#[cfg(all(unix, not(target_os = "macos")))]
use winit::event_loop::ActiveEventLoop;
#[cfg(all(unix, not(target_os = "macos")))]
use winit::window::ActivationToken;

/// Take the startup notification this process was launched with.
///
/// The desktop puts a token in the environment, starts a spinning cursor and a
/// placeholder in the dock, and waits for a window to arrive carrying that
/// token. Nothing here read it, so every launch from the dock spun for the ten
/// to fifteen seconds of the desktop's own timeout and showed no icon until it
/// expired, even though the window itself had painted immediately.
///
/// Taking it, rather than reading it, matters just as much: `noob serve` is
/// spawned from this process and inherits its environment, and a child holding
/// a token that has already been spent asks the compositor to act twice on one
/// click.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn take_activation_token(event_loop: &ActiveEventLoop) -> Option<ActivationToken> {
    use winit::platform::startup_notify::{EventLoopExtStartupNotify, reset_activation_token_env};

    let raw = event_loop.read_token_from_env().map(ActivationToken::into_raw);
    // Unconditionally, and even when this display server had nothing to read:
    // a session running both an X server and a Wayland compositor sets one
    // variable each, and only the child would ever see the other one.
    reset_activation_token_env();
    usable_token(raw)
}

/// The token worth passing on, which is a token with something in it.
///
/// A launcher that exports the variable without filling it in is not offering
/// a notification. Handing the empty string on is not free: it is a name no
/// click answers to, and the compositor is entitled to focus whatever it
/// decides that names.
#[cfg(all(unix, not(target_os = "macos")))]
fn usable_token(raw: Option<String>) -> Option<ActivationToken> {
    let raw = raw?;
    (!raw.trim().is_empty()).then(|| ActivationToken::from_raw(raw))
}

/// The desktop entry, checked against the code rather than trusted.
#[cfg(test)]
const DESKTOP: &str = include_str!("../../data/io.github.hec_ovi.NO0B.desktop");
#[cfg(test)]
const ICON: &str = include_str!("../../data/io.github.hec_ovi.NO0B.svg");
#[cfg(test)]
const SYMBOLIC: &str = include_str!("../../data/io.github.hec_ovi.NO0B-symbolic.svg");
#[cfg(test)]
const INSTALLER: &str = include_str!("../../data/install.sh");
#[cfg(test)]
const MAIN: &str = include_str!("main.rs");

/// The binary this crate builds, which is the name the entry's `Exec` says and
/// the name the installer places. Taken from cargo rather than typed out, so a
/// renamed binary fails a test here instead of leaving a launcher that starts
/// nothing.
#[cfg(test)]
const BINARY: &str = env!("CARGO_BIN_NAME");

/// What the window was called up to 0.6.0, kept only so the installer can be
/// checked for removing it. Two launchers in the menu, one of them dead, is the
/// failure a rename produces when nobody cleans up.
#[cfg(test)]
const OLD_APP_ID: &str = "io.github.hec_ovi.CLIppy";

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
        // And the entry runs the binary the installer put on PATH. Bare, with
        // no field code: the argument NO0B takes is a workspace directory, and
        // every field code the specification has stands for a file or a URL.
        // `%f` said "hand me a file", which nothing does anyway because the
        // entry declares no `MimeType`, and which would be the wrong thing if
        // something did.
        assert_eq!(key(DESKTOP, "Exec"), BINARY, "{DESKTOP}");
        assert!(!DESKTOP.contains("MimeType"), "a field code would be needed");
    }

    /// The launch handshake, which the entry opts into and the code has to
    /// finish. Without the code half the desktop starts a spinner on click and
    /// runs it for the full ten to fifteen seconds of its own timeout, showing
    /// no icon in the dock the whole time, while the window sits there painted.
    #[test]
    fn the_entry_asks_for_a_notification_the_code_answers() {
        assert_eq!(key(DESKTOP, "StartupNotify"), "true");
        assert!(MAIN.contains("take_activation_token"), "nothing reads the token");
    }

    #[test]
    fn an_empty_token_is_not_a_notification() {
        assert_eq!(usable_token(None), None);
        assert_eq!(usable_token(Some(String::new())), None);
        assert_eq!(usable_token(Some(String::from("  "))), None);
        assert_eq!(
            usable_token(Some(String::from("no0b-0_TIME42"))),
            Some(ActivationToken::from_raw(String::from("no0b-0_TIME42")))
        );
    }

    /// Every file the installer places has to exist to be placed.
    ///
    /// This is not hypothetical. The staging step used a glob that matched the
    /// icon and the launcher but not the icon's small variant, whose name has
    /// a suffix before the extension. The install died on a missing file with
    /// no clue which one, because `install` reports the error and not the
    /// path. The glob is gone; this keeps the list honest.
    #[test]
    fn the_installer_only_places_files_that_exist() {
        let placed: Vec<&str> = INSTALLER
            .lines()
            .filter(|line| line.contains("install -m") && line.contains("$HERE/"))
            .map(|line| {
                line.split("$HERE/")
                    .nth(1)
                    .and_then(|rest| rest.split('"').next())
                    .expect("an installed file is quoted")
            })
            .collect();
        assert!(placed.len() >= 3, "the installer places {placed:?}");
        for name in placed {
            let name = name.replace("$APP_ID", crate::APP_ID);
            // The binary is the one thing staged from the build rather than
            // from the repository.
            if name == BINARY {
                continue;
            }
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../data")
                .join(&name);
            assert!(path.exists(), "the installer places {name}, which is not in gui/data");
        }
    }

    /// The rename has to take the old launcher out, and it has to do it on an
    /// ordinary install rather than only on `--uninstall`.
    ///
    /// A desktop reads every entry it finds. Leaving CLIppy's behind means two
    /// icons in the menu, and the older of the two runs a binary the installer
    /// no longer writes, so it does nothing at all.
    #[test]
    fn the_installer_removes_what_the_old_name_left() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../data");
        let dir = std::env::temp_dir().join(format!("no0b-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (here, prefix) = (dir.join("stage"), dir.join("prefix"));
        std::fs::create_dir_all(&here).expect("a staging directory");
        for name in [
            String::from("install.sh"),
            format!("{}.desktop", crate::APP_ID),
            format!("{}.svg", crate::APP_ID),
            format!("{}-symbolic.svg", crate::APP_ID),
        ] {
            std::fs::copy(root.join(&name), here.join(&name)).expect("gui/data has it");
        }
        // Standing in for the release build, which a test does not have.
        std::fs::write(here.join(BINARY), "#!/bin/sh\n").expect("a stand-in binary");

        // A machine that installed CLIppy, exactly as its own installer left it.
        let apps = prefix.join("share/applications");
        let scalable = prefix.join("share/icons/hicolor/scalable/apps");
        let symbolic = prefix.join("share/icons/hicolor/symbolic/apps");
        for path in [&prefix.join("bin"), &apps, &scalable, &symbolic] {
            std::fs::create_dir_all(path).expect("a prefix");
        }
        let stale = [
            prefix.join("bin/clippy"),
            apps.join(format!("{OLD_APP_ID}.desktop")),
            scalable.join(format!("{OLD_APP_ID}.svg")),
            symbolic.join(format!("{OLD_APP_ID}-symbolic.svg")),
        ];
        for path in &stale {
            std::fs::write(path, "what the old name installed").expect("a stale file");
        }

        let run = || {
            std::process::Command::new("bash")
                .arg(here.join("install.sh"))
                .arg("--prefix")
                .arg(&prefix)
                .output()
                .expect("bash runs the installer")
        };
        let out = run();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

        // The new set is in place, and the entry points at the binary by its
        // full path rather than by name.
        let placed = prefix.join("bin").join(BINARY);
        assert!(placed.exists(), "{} is not there", placed.display());
        let entry = std::fs::read_to_string(apps.join(format!("{}.desktop", crate::APP_ID)))
            .expect("the entry was installed");
        assert!(entry.contains(&format!("Exec={}", placed.display())), "{entry}");
        assert!(scalable.join(format!("{}.svg", crate::APP_ID)).exists());
        assert!(symbolic.join(format!("{}-symbolic.svg", crate::APP_ID)).exists());
        // And the old set is gone, so the menu holds one launcher.
        for path in &stale {
            assert!(!path.exists(), "{} survived the install", path.display());
        }

        // Removing a file that is not there is not a failure: the second run
        // has nothing of the old name left to take out, and `set -e` would end
        // the script on a bare `rm`.
        let again = run();
        assert!(again.status.success(), "{}", String::from_utf8_lossy(&again.stderr));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A malformed entry is ignored in full, so the parts that make it valid
    /// are worth pinning.
    #[test]
    fn the_desktop_entry_is_a_valid_one() {
        assert!(DESKTOP.starts_with("[Desktop Entry]\n"), "{DESKTOP}");
        assert_eq!(key(DESKTOP, "Type"), "Application");
        assert_eq!(key(DESKTOP, "Name"), "NO0B");
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

    /// One path in one flat fill, and a console you can see through.
    ///
    /// Four subpaths rather than one, because the mark is a hollow wire now: the
    /// frame, the hole that opens it, the chevron and the bar. Under the nonzero
    /// fill rule the direction a subpath was drawn in is the only thing that
    /// says hole or solid, so a subpath written the wrong way round fills the
    /// console in and nothing anywhere reports it. That is what the signs are:
    /// the hole runs against the frame, and both glyphs run with it.
    ///
    /// The rest is what it always was. A gradient or a second colour is what
    /// stops a mark reading at 16 pixels, and a stroke is what puts its edges
    /// on half pixels.
    #[test]
    fn the_icon_is_one_path_and_the_console_is_hollow() {
        assert_eq!(ICON.matches("<path").count(), 1, "{ICON}");
        assert_eq!(ICON.matches("fill=").count(), 1);
        assert!(!ICON.contains("stroke"), "strokes do not scale cleanly");
        assert!(!ICON.contains("Gradient"), "a gradient carries no information here");
        assert!(ICON.contains(r#"viewBox="0 0 128 128""#), "{ICON}");

        let drawn = subpaths(ICON);
        assert_eq!(drawn.len(), 4, "frame, hole, chevron and bar: {}", path_of(ICON));
        let areas: Vec<f64> = drawn.iter().map(|points| twice_area(points)).collect();
        let winding: Vec<f64> = areas.iter().map(|a| a.signum()).collect();
        assert_eq!(winding, [1.0, -1.0, 1.0, 1.0], "the console is filled in: {areas:?}");
    }

    /// The glow is decoration, and the mark has to be whole without it.
    ///
    /// Not every consumer of an icon rasterizes with filters on, so the halo is
    /// one blur merged twice *under* the source graphic, inside one filter. The
    /// source graphic is the path itself, so a renderer that drops the filter
    /// draws the same four subpaths sharp and loses the halo and nothing else.
    /// The other way to get a glow is a second translucent copy of the path
    /// behind the first, and it fails exactly there: filters off, and the copy
    /// is a solid doubled mark instead of light. It would also put a second set
    /// of coordinates in the file for the first to drift away from.
    ///
    /// The radius is 4, by Hector's explicit choice over a radius 2 drawn beside
    /// it, and the halo is wider than the drawing can hold. Three radii is where
    /// a gaussian is spent, so this one wants 12 units of room and the mark
    /// keeps 8 of margin: the canvas edge cuts the last 4 units of the tails off
    /// down the left and the right. Rendered in librsvg at 128 the halo is at 7%
    /// alpha where the edge takes it, so what that costs is a straight seam at
    /// 7% down two sides. It is the chosen price of the brighter glow and not a
    /// defect, so there is deliberately no cap here to "restore". The radius was
    /// 2 for exactly one round and he picked the other one twice.
    ///
    /// What is worth holding instead is that the halo is only ever clipped by
    /// the canvas. The region has to be declared and has to be big enough for
    /// three radii, so a renderer's default region can never quietly become the
    /// thing cutting the glow, and raising the radius further cannot happen
    /// without widening the region in the same edit.
    #[test]
    fn the_glow_is_decoration_and_the_mark_survives_it_being_dropped() {
        assert_eq!(ICON.matches("<filter").count(), 1, "{ICON}");
        assert_eq!(ICON.matches("<feGaussianBlur").count(), 1, "{ICON}");
        let radius: f64 = attr(ICON, "<feGaussianBlur", "stdDeviation")
            .parse()
            .expect("the blur has a radius");
        assert!(radius > 0.0, "a radius of {radius} is no glow at all");

        // Declared, and declared in the drawing's own units. A filter with no
        // region of its own takes the renderer's default, which is a percentage
        // of the bounding box and so moves on its own the day the geometry does.
        assert!(ICON.contains(r#"filterUnits="userSpaceOnUse""#), "{ICON}");
        let region = (
            attr(ICON, "<filter", "x").parse::<f64>().expect("a region x"),
            attr(ICON, "<filter", "y").parse::<f64>().expect("a region y"),
            attr(ICON, "<filter", "width").parse::<f64>().expect("a region width"),
            attr(ICON, "<filter", "height").parse::<f64>().expect("a region height"),
        );
        // Room for the mark and three radii of halo around all of it. This is
        // the assertion that couples the two numbers: the region is what says
        // how far the glow is allowed to reach, so a bigger radius fails here
        // until whoever raised it widens the region on purpose.
        let (low_x, low_y, high_x, high_y) = box_of(&subpaths(ICON).concat());
        let reach = radius * 3.0;
        assert!(region.0 <= low_x - reach, "the region cuts the halo off on the left");
        assert!(region.1 <= low_y - reach, "the region cuts the halo off on top");
        assert!(region.0 + region.2 >= high_x + reach, "the region cuts it off on the right");
        assert!(region.1 + region.3 >= high_y + reach, "the region cuts it off underneath");

        // Two of halo and then the mark over them, in that order: the merge is
        // a stack, so the mark being last is what keeps its edge sharp.
        assert_eq!(ICON.matches("<feMergeNode").count(), 3, "{ICON}");
        let top = ICON.split("<feMergeNode").last().expect("a last merge node");
        assert!(top.contains("SourceGraphic"), "the halo is merged over the mark");
        // And nothing is faded. The halo gets its weight from being merged
        // twice, not from an opacity that would also take the mark down with it.
        assert!(!ICON.contains("opacity"), "{ICON}");

        // The proof rather than the argument: one reference to the filter, and
        // with it taken out the file still carries the whole drawing.
        assert_eq!(ICON.matches(r#"filter="url("#).count(), 1, "{ICON}");
        let id = attr(ICON, "<filter", "id");
        let plain = ICON.replace(&format!(" filter=\"url(#{id})\""), "");
        assert!(!plain.contains(r#"filter="url("#), "{plain}");
        assert_eq!(path_of(&plain), path_of(ICON), "stripping the filter moved the geometry");
        assert_eq!(subpaths(&plain).len(), 4, "{plain}");
        assert_eq!(plain.matches("fill=").count(), 1, "{plain}");
    }

    /// The prompt sits inside the console with a module of clearance.
    ///
    /// A glyph grown out to the wire welds itself to the frame, and the mark
    /// stops being a console with writing in it and becomes a filled box with a
    /// notch. It is the first thing an edit to the two glyphs breaks and it is
    /// invisible above 32 pixels.
    #[test]
    fn the_prompt_sits_inside_the_console() {
        let drawn = subpaths(ICON);
        let (hole_x, hole_y, hole_far_x, hole_far_y) = box_of(&drawn[1]);
        for glyph in &drawn[2..] {
            let (x, y, far_x, far_y) = box_of(glyph);
            assert!(x >= hole_x + 8.0 && far_x <= hole_far_x - 8.0, "{x} to {far_x} across");
            assert!(y >= hole_y + 8.0 && far_y <= hole_far_y - 8.0, "{y} to {far_y} down");
        }
    }

    /// Every coordinate is a whole pixel at 16, 32, 64 and 128, which is what
    /// a module of 8 on a 128 canvas buys. One coordinate off the module and
    /// that edge renders as two grey rows at the small sizes.
    ///
    /// The blur radius is not a coordinate and is not measured here. It is
    /// capped in [`tests::the_glow_is_decoration_and_the_mark_survives_it_being_dropped`]
    /// instead, against the margin rather than against the module, because what
    /// a halo has to fit inside is the canvas and not the pixel grid.
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

    /// The path split into the closed subpaths it is made of, each one the
    /// points it visits in the order it visits them. Only `M`, `L` and `Z`: the
    /// drawing has no curve in it, and a helper that pretended to handle one
    /// would be measuring a control point as if it were a corner.
    fn subpaths(svg: &str) -> Vec<Vec<(f64, f64)>> {
        let mut out: Vec<Vec<(f64, f64)>> = Vec::new();
        for (command, params) in commands(svg) {
            let points = params.chunks(2).map(|pair| (pair[0], pair[1]));
            match command {
                'M' => out.push(points.collect()),
                'L' => out.last_mut().expect("a line before any move").extend(points),
                'Z' => {}
                other => panic!("the icon is drawn with M, L and Z, not {other}"),
            }
        }
        out
    }

    /// Twice the signed area of a closed subpath, which is the shoelace sum.
    ///
    /// Only the sign is read: it is the direction the subpath was drawn in, and
    /// under the nonzero fill rule that is the whole difference between a hole
    /// and a solid.
    fn twice_area(points: &[(f64, f64)]) -> f64 {
        points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .map(|((x, y), (next_x, next_y))| x * next_y - next_x * y)
            .sum()
    }

    /// The box a set of points sits in, as near corner then far corner.
    fn box_of(points: &[(f64, f64)]) -> (f64, f64, f64, f64) {
        points.iter().fold(
            (f64::MAX, f64::MAX, f64::MIN, f64::MIN),
            |(low_x, low_y, high_x, high_y), (x, y)| {
                (low_x.min(*x), low_y.min(*y), high_x.max(*x), high_y.max(*y))
            },
        )
    }

    /// One attribute of the element that opens with `tag`.
    ///
    /// The leading space matters: `x` would otherwise match inside a longer
    /// attribute name, and `width` is on the `<svg>` element as well as on the
    /// filter, so the element has to be found before the attribute is.
    fn attr(svg: &str, tag: &str, name: &str) -> String {
        let element = svg
            .split(tag)
            .nth(1)
            .unwrap_or_else(|| panic!("there is no {tag} in {svg}"))
            .split('>')
            .next()
            .expect("an element ends somewhere");
        element
            .split(&format!(" {name}=\""))
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap_or_else(|| panic!("{tag} has no {name}: {element}"))
            .to_string()
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
