//! The glyphs the window draws, by name.
//!
//! Every one is in the symbol font `noob-draw` embeds, and a test over there
//! asserts that each resolves to a real glyph rather than to `.notdef`. That
//! test is the point of naming them in one place: a missing glyph draws as
//! nothing at all, which is how the window buttons ended up being three
//! hand-drawn rectangles in the first place.
//!
//! Codepoints are from the Nerd Fonts sets: `cod` is Codicon, `seti` and `dev`
//! are the file-type marks, `fa` is Font Awesome.

/// Window controls, matching what every other application on the desktop uses.
pub const CLOSE: char = '\u{eab8}';
pub const MAXIMIZE: char = '\u{eab9}';
pub const MINIMIZE: char = '\u{eaba}';

/// The gear on a menu's settings row.
pub const SETTINGS: char = '\u{eb51}';

/// The rest of what a menu row can be: two sheets for taking a copy, a
/// clipboard for putting one back, a grid of frames for the row that lists every
/// widget, and a plain cross for the row that takes one out of the window.
///
/// The cross is its own codepoint rather than [`CLOSE`] above: that one is the
/// window button that kills the application, and a menu row wearing the same
/// mark reads as the same act.
///
/// The clipboard is the nearest the symbol font has. There is no paste glyph in
/// the Codicon set the rest of these come from, and a clipboard is what every
/// other menu on the desktop puts on that row anyway.
pub const COPY: char = '\u{ebcc}';
pub const PASTE: char = '\u{eac0}';
pub const WIDGETS: char = '\u{eb23}';
pub const CLOSE_WIDGET: char = '\u{ea76}';

/// The two arrows a tab strip grows when it holds more tabs than it has room
/// for, one step along the strip each. Chevrons rather than triangles: they are
/// the same weight as the window controls above them, and a filled triangle at
/// this size reads as a fold marker.
pub const TABS_LEFT: char = '\u{eab5}';
pub const TABS_RIGHT: char = '\u{eab6}';

/// At the right end of a menu row that opens a group of rows underneath it.
///
/// Two, because the rows appear in the same column as the header rather than
/// out to the side: the chevron points right while the group is shut and turns
/// down while it is open, which is what every tree on the desktop says and the
/// only thing on the row that says what pressing it will do. The shut one is
/// the same chevron the tab strip walks with, on purpose: one mark in the
/// window means there is more of this that way.
pub const SUBMENU: char = '\u{eab6}';
pub const SUBMENU_OPEN: char = '\u{eab4}';

/// The two states of a row that is a switch rather than a destination: the
/// widget is in the window, or it is out. Boxed rather than a bare tick, so the
/// row reads as something that can be turned off as well as on.
pub const CHECKED: char = '\u{f14a}';
pub const UNCHECKED: char = '\u{f096}';

/// A file whose type has no mark of its own.
const FILE: char = '\u{ea7b}';
/// A folder in the picker's list.
pub const FOLDER: char = '\u{e5ff}';
/// The mark in front of a folder in the picker's tree used to be two glyphs
/// here, Font Awesome's filled plus-square and minus-square. It is drawn out of
/// rectangles now, in `view.rs`, because the filled boxes were the biggest thing
/// on a row and read as blocks rather than as a control: see `picker_mark`.
///
/// The folder the picker is listing, which is also the one it would open.
pub const FOLDER_OPEN: char = '\u{eaf7}';
/// The way out of it.
pub const UP: char = '\u{eaa1}';
/// On the row saying why a folder in the tree could not be read.
pub const LOCKED: char = '\u{f023}';
/// A folder opened in an earlier session.
pub const RECENT: char = '\u{ea82}';
/// In front of what has been typed to narrow a list. A magnifier rather than
/// the funnel that was here before: the field it sits in is typed into, and a
/// funnel says the list has been filtered rather than saying type here.
pub const SEARCH: char = '\u{ea6d}';
/// On the button that confirms a choice, and on the menu row that opens the
/// session under the pointer: the same act, so the same mark.
pub const CONFIRM: char = '\u{eab2}';
/// On the menu row that deletes a session.
///
/// A bin, which is what every other list on the desktop puts on that row. The
/// one row in this window that destroys a file, so it is worth being the one
/// glyph nobody has to read a label to recognise.
pub const TRASH: char = '\u{ea81}';

/// The mark for a file, chosen by extension.
///
/// Falls back to the plain file glyph rather than to nothing, so an unknown
/// extension still lines up with the rows above and below it.
pub fn for_path(path: &str) -> char {
    let extension = path
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "rs" => '\u{e7a8}',
        "py" => '\u{e606}',
        "js" | "mjs" | "cjs" => '\u{e60c}',
        "ts" | "tsx" => '\u{e628}',
        "md" | "markdown" => '\u{e609}',
        "json" => '\u{e60b}',
        "toml" | "ini" | "cfg" | "conf" => '\u{e615}',
        "yml" | "yaml" => '\u{e615}',
        "sh" | "bash" | "zsh" => '\u{ea85}',
        "html" | "htm" => '\u{e60e}',
        "css" => '\u{e614}',
        "c" | "h" => '\u{e61e}',
        "cpp" | "cc" | "hpp" => '\u{e61d}',
        "go" => '\u{e627}',
        "lock" => '\u{f023}',
        "txt" | "log" => '\u{f0f6}',
        _ => FILE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_extension_gets_its_own_mark_and_the_rest_get_the_plain_one() {
        assert_eq!(for_path("src/main.rs"), '\u{e7a8}');
        assert_eq!(for_path("README.md"), '\u{e609}');
        assert_eq!(for_path("a/b/setup.PY"), '\u{e606}', "case does not matter");
        assert_eq!(for_path("Cargo.lock"), '\u{f023}');
        assert_eq!(for_path("mystery.qqq"), FILE);
    }

    /// A name with no dot in it must not read the whole path as an extension.
    #[test]
    fn a_file_without_an_extension_still_gets_a_mark() {
        assert_eq!(for_path("Makefile"), FILE);
        assert_eq!(for_path(""), FILE);
        assert_eq!(for_path("dir.d/plain"), FILE);
    }

    /// Every codepoint named in this module has to exist in the embedded font.
    /// A missing one draws as nothing at all, which is indistinguishable from
    /// having forgotten to draw it, and is how the window buttons came to be
    /// three hand-drawn rectangles.
    #[test]
    fn every_named_icon_exists_in_the_embedded_font() {
        let extensions = [
            "rs", "py", "js", "mjs", "cjs", "ts", "tsx", "md", "markdown", "json", "toml", "ini",
            "cfg", "conf", "yml", "yaml", "sh", "bash", "zsh", "html", "htm", "css", "c", "h",
            "cpp", "cc", "hpp", "go", "lock", "txt", "log", "nothing-in-particular",
        ];
        let named = [
            CLOSE,
            MAXIMIZE,
            MINIMIZE,
            SETTINGS,
            COPY,
            PASTE,
            WIDGETS,
            CLOSE_WIDGET,
            TABS_LEFT,
            TABS_RIGHT,
            SUBMENU,
            SUBMENU_OPEN,
            CHECKED,
            UNCHECKED,
            FILE,
            FOLDER,
            FOLDER_OPEN,
            UP,
            LOCKED,
            RECENT,
            SEARCH,
            CONFIRM,
            TRASH,
        ]
            .into_iter()
            .chain(extensions.iter().map(|e| for_path(&format!("a.{e}"))));
        for ch in named {
            assert!(
                noob_draw::has_glyph(ch),
                "U+{:04X} is not in the symbol font, so it would draw as nothing",
                ch as u32
            );
        }
    }
}

