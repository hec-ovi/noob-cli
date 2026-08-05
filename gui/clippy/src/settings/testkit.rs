//! The settings panel's own scene rig: open a panel on a section, render it,
//! and read the boxes back.
//!
//! A child of `settings`, so the section boxes can render a panel without
//! borrowing the panel's own test file to do it. `#[cfg(test)]` at the
//! declaration: none of this ships.

#[allow(clippy::wildcard_imports)]
use super::*;
#[allow(clippy::wildcard_imports)]
use crate::settings::places::*;
#[allow(clippy::wildcard_imports)]
use crate::view::testkit::*;
#[allow(clippy::wildcard_imports)]
use crate::view::*;
use crate::config::Config;
use crate::dock::Dock;
use crate::monitor::Monitor;
use crate::style::skin::Skin;
use noob_draw::{Panel, Text};

/// The pane text every other settings test is laid out and drawn in: the
/// size and the advance of one character at it.
pub(crate) const PANE_TEXT: (f32, f32) = (13.0, 8.0);
/// The biggest the settings file will carry, and what a character of a
/// monospace face costs at that size. `font_size` and `pane_font_size` are
/// both clamped to 40 by `Config::apply`, and `column_width` measures the
/// real face and falls back to six tenths of the size, which is what a
/// monospace advance is within a pixel either way.
pub(crate) const BIGGEST_TEXT: (f32, f32) = (40.0, 24.0);
/// A description long enough that it used to be cut off where the buttons
/// started, and a document line long enough that it used to be cut off at
/// the edge of its column.
pub(crate) const A_LONG_ABOUT: &str =
    "reads the file before it writes one, and says which file it read";
pub(crate) const A_LONG_DOC_LINE: &str = "the whole point of a column beside the list is that a sentence written in it can be read all the way to the end of itself rather than stopping in three dots";
/// A description several times the width of the entry column. Giving it a
/// line of its own took the buttons out of its way; it still ended in an
/// ellipsis at the edge of the column, which is what this one is long
/// enough to prove.
pub(crate) const A_WRAPPING_ABOUT: &str = "reads the file before it writes one, says which file it read, and stops at the first thing it does not recognise rather than guessing what the rest of the line was meant to say";
/// The first line is Markdown, so what is on screen is four characters
/// shorter than what is in the file and a copy measured on the source would
/// hand back marks that were nowhere.
pub(crate) const A_MARKED_DOC_LINE: &str = "- **read** a file with `cat`";
pub(crate) const A_DRAWN_DOC_LINE: &str = "• read a file with cat";
/// The window with the settings panel up, laid out and drawn off one shape,
/// which is what makes a row land where it is drawn.
pub(crate) fn render_settings(panel: &Settings, w: f32, h: f32, hot: Option<Hit>) -> Rendered {
    render_settings_at_rail(panel, w, h, hot, crate::config::SETTINGS_RAIL)
}
/// And the same with a drag over the document, which is what puts a band
/// under the glyphs.
pub(crate) fn render_settings_selecting(
    panel: &Settings,
    w: f32,
    h: f32,
    selection: crate::select::Selection,
) -> Rendered {
    render_settings_with(panel, w, h, None, crate::config::SETTINGS_RAIL, Some(selection), PANE_TEXT)
}
pub(crate) fn render_settings_with(
    panel: &Settings,
    w: f32,
    h: f32,
    hot: Option<Hit>,
    rail: f32,
    selection: Option<crate::select::Selection>,
    font: (f32, f32),
) -> Rendered {
    let dock = Dock::new();
    let state = busy_state();
    let mut shape = shape(&dock, &["a.rs"]);
    shape.settings = Some(panel);
    shape.settings_rail = rail;
    shape.pane_size = font.0;
    shape.pane_column = font.1;
    let layout = Layout::compute(w, h, &shape);
    let skin = Skin::from(&Config::default());
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
        pane_column: font.1,
        body_size: 14.0,
        pane_size: font.0,
        clock: 0.0,
        orb_morph: None,
        drag: None,
        hot,
        trouble: None,
        esc_armed: false,
        popup_scroll: [0, 0],
        cursor: (-100.0, -100.0),
        selection,
        menu: None,
        picker: None,
        settings: Some(panel),
    });
    Rendered {
        scene,
        layout,
        skin,
    }
}
/// The same with the rail dragged to `rail` of the panel's width, which is
/// the only thing a drag of the line beside it changes.
pub(crate) fn render_settings_at_rail(
    panel: &Settings,
    w: f32,
    h: f32,
    hot: Option<Hit>,
    rail: f32,
) -> Rendered {
    render_settings_with(panel, w, h, hot, rail, None, PANE_TEXT)
}
/// The same panel at one font size, since the rail's layout is a question
/// about how many lines of that size fit in the window.
pub(crate) fn render_settings_at_font(panel: &Settings, w: f32, h: f32, font: (f32, f32)) -> Rendered {
    render_settings_with(panel, w, h, None, crate::config::SETTINGS_RAIL, None, font)
}
pub(crate) fn a_panel_on(config: &Config, section: &str) -> Settings {
    let mut panel = a_settings_panel(config);
    let at = panel
        .section_names()
        .iter()
        .position(|name| *name == section)
        .unwrap_or_else(|| panic!("{section} is not a section"));
    panel.choose(at);
    panel
}
/// The panel opened on the saved conversations, with three of them to draw.
pub(crate) fn a_sessions_panel() -> Settings {
    let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
    let saved = |id: &str, ago: u64, folder: &str, said: &str| crate::sessions::Saved {
        id: String::from(id),
        when: now - std::time::Duration::from_secs(ago),
        workspace: Some(std::path::PathBuf::from(folder)),
        gone: false,
        bytes: 12_000,
        context: None,
        opening: String::from(said),
    };
    let mut panel = Settings::open(
        &Config::default(),
        None,
        crate::agent::Agent {
            now,
            sessions: crate::sessions::Listing {
                sessions: vec![
                    saved("aaa", 60, "/home/hec/workspace/noob-cli", "fix the panel"),
                    saved("bbb", 3_600, "/home/hec/workspace/anna", "read the map"),
                    saved("ccc", 86_400, "/home/hec/notes", "what did we say"),
                ],
                skipped: Vec::new(),
            },
            ..crate::agent::Agent::default()
        },
    );
    let at = panel
        .section_names()
        .iter()
        .position(|name| *name == crate::settings::SESSIONS)
        .expect("the sessions section");
    panel.choose(at);
    panel
}
/// The saved conversations, more of them than the table's body holds.
pub(crate) fn a_long_sessions_panel() -> Settings {
    let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(9_000_000);
    let sessions: Vec<crate::sessions::Saved> = (0..crate::settings::TABLE_ROWS * 3)
        .map(|at| crate::sessions::Saved {
            id: format!("id{at}"),
            when: now - std::time::Duration::from_secs(60 * (at as u64 + 1)),
            workspace: Some(std::path::PathBuf::from("/home/hec/workspace/noob-cli")),
            gone: false,
            bytes: 12_000,
            context: None,
            opening: format!("conversation {at}"),
        })
        .collect();
    let mut panel = Settings::open(
        &Config::default(),
        None,
        crate::agent::Agent {
            now,
            sessions: crate::sessions::Listing {
                sessions,
                skipped: Vec::new(),
            },
            ..crate::agent::Agent::default()
        },
    );
    let at = panel
        .section_names()
        .iter()
        .position(|name| *name == crate::settings::SESSIONS)
        .expect("the sessions section");
    panel.choose(at);
    panel
}
/// The SKILLS section with an install answer long enough that its block
/// scrolls, over a list long enough that the panel scrolls too, wound to
/// the end so the block is among the rows on screen.
pub(crate) fn a_wordy_install_panel() -> Settings {
    let mut agent = an_agent();
    agent.skills[0].about = String::from(A_LONG_ABOUT);
    agent.skills[0].doc = vec![String::from(A_LONG_DOC_LINE)];
    for extra in 1..6 {
        let mut skill = agent.skills[0].clone();
        skill.name = format!("skill{extra}");
        skill.dir = format!("skill{extra}");
        agent.skills.push(skill);
    }
    let mut panel = Settings::open(
        &Config::default(),
        Some(std::path::Path::new("/home/hec/.config/noob/no0b.conf")),
        agent.clone(),
    );
    let at = panel
        .section_names()
        .iter()
        .position(|name| *name == crate::settings::SKILLS)
        .expect("SKILLS is a section");
    panel.choose(at);
    panel.begin_install(String::from("owner/skill"), &Config::default());
    panel.adopt_install(
        String::from("owner/skill"),
        Err((0..200)
            .map(|at| format!("clone line {at}"))
            .collect::<Vec<String>>()
            .join("\n")),
        agent,
        &Config::default(),
    );
    scrolled_to_the_end(&mut panel);
    panel
}
pub(crate) fn a_wordy_servers_panel() -> Settings {
    let mut panel = Settings::open(
        &Config::default(),
        Some(std::path::Path::new("/home/hec/.config/noob/no0b.conf")),
        an_agent_with_servers(),
    );
    let at = panel
        .section_names()
        .iter()
        .position(|name| *name == crate::settings::MCP)
        .expect("MCP is a section");
    panel.choose(at);
    panel
}
/// An agent with the servers a section of entries is asserted on: one with
/// a description long enough to wrap and a document beside it, and one that
/// is turned off.
///
/// The entries of the panel are the servers. The skills are a table, and
/// every rule about an entry (its card, its two buttons, the column beside
/// it) is asserted here, on the list that has them.
pub(crate) fn an_agent_with_servers() -> crate::agent::Agent {
    let mut agent = an_agent();
    agent.mcp = crate::agent::Mcp {
        global: Some(std::path::PathBuf::from("/home/hec/.config/noob/mcp.json")),
        project: None,
        any_file: true,
        servers: vec![
            crate::agent::Server {
                name: String::from("docs"),
                how: String::from(A_LONG_ABOUT),
                project: false,
                on: true,
                entry: String::from(A_LONG_DOC_LINE),
            },
            crate::agent::Server {
                name: String::from("shell"),
                how: String::from("http://localhost:9001/mcp"),
                project: false,
                on: false,
                entry: String::from("{ \"url\": \"http://localhost:9001/mcp\" }"),
            },
        ],
        trouble: Vec::new(),
    };
    agent
}
/// The skills section with two skills, the first wordy enough that its
/// description cannot be one row of the column and the second short enough
/// that it is.
pub(crate) fn a_wrapping_skills_panel() -> Settings {
    let mut agent = an_agent_with_servers();
    agent.mcp.servers[0].how = String::from(A_WRAPPING_ABOUT);
    let mut panel = a_wordy_servers_panel();
    panel.adopt_agent(agent, &Config::default());
    panel
}
/// A skill whose document is three short lines, one of them marked up.
pub(crate) fn a_selectable_skills_panel() -> Settings {
    let mut agent = an_agent();
    agent.skills[0].doc = vec![
        String::from(A_MARKED_DOC_LINE),
        String::from("second line of the document"),
        String::from("third line of the document"),
    ];
    let mut panel = a_panel_on(&Config::default(), crate::settings::SKILLS);
    panel.adopt_agent(agent, &Config::default());
    on_the_installed_skill(&mut panel);
    panel
}
/// Put the keys on the first installed skill of the table, which is the row
/// under the web search the CLI ships with.
pub(crate) fn on_the_installed_skill(panel: &mut Settings) {
    let at = panel
        .rows()
        .iter()
        .position(|row| matches!(row, crate::settings::Row::Table(_)))
        .expect("the installed table");
    panel.point_at(at, crate::settings::Side::Left);
    panel.step(true);
}
/// Where the card drawn in a row has its title, its divider, its body and
/// its footer, worked out through the same two functions the placement and
/// the drawing go through.
pub(crate) fn the_card(_out: &Rendered, row: Panel, footer: bool) -> (Panel, CardParts) {
    let line = Text::line_for(PANE_TEXT.0);
    // The row's own width, which is half the list for a card standing
    // beside another one, and the whole of it otherwise.
    let cols = settings_entry_cols(row.w, PANE_TEXT.1);
    let card = settings_card(row, line);
    let parts = settings_card_parts(card, line, PANE_TEXT.0, PANE_TEXT.1, cols, footer);
    (card, parts)
}
/// The one card row of a rendered panel, and where it is.
pub(crate) fn the_card_row(out: &Rendered, panel: &Settings) -> (usize, Panel) {
    let (index, _, row) = *out
        .layout
        .settings_rows
        .iter()
        .find(|(index, _, _)| matches!(panel.row(*index), Some(crate::settings::Row::Card(_))))
        .expect("a card row");
    (index, row)
}
/// The one entry row, and the entry itself, out of a rendered skills panel.
pub(crate) fn the_entry_row(out: &Rendered, panel: &Settings) -> (usize, Panel) {
    let (index, _, row) = *out
        .layout
        .settings_rows
        .iter()
        .find(|(index, _, _)| {
            matches!(panel.row(*index), Some(crate::settings::Row::Entry(_)))
        })
        .expect("an entry row");
    (index, row)
}
/// Every entry row of a rendered skills panel, in the order they are drawn.
pub(crate) fn the_entry_rows(out: &Rendered, panel: &Settings) -> Vec<(usize, Panel)> {
    out.layout
        .settings_rows
        .iter()
        .filter(|(index, _, _)| {
            matches!(panel.row(*index), Some(crate::settings::Row::Entry(_)))
        })
        .map(|(index, _, row)| (*index, *row))
        .collect()
}
/// The panel with one section chosen, which is what a press on the rail or
/// a Tab leaves behind. The keyboard is on the rows of it either way: no
/// arrow key touches the rail.
/// The arrangement the window opens with, with one view's tab brought to
/// the front of the space it lives in: FILES and ACTIVITY are tabs of the
/// conversation's own space now, so a test about either has to show it the
/// way a press on its tab would.
/// Every view in one space, for the tests about a strip with more tabs
/// than it can draw. The arrangement the window opens with gives each
/// The pixel in the middle of one drawn cell of the document.
pub(crate) fn doc_cell(layout: &Layout, row: usize, at: usize) -> (f32, f32) {
    let inside = layout.settings_doc_text;
    let line = Text::line_for(13.0);
    (
        inside.x + at as f32 * 8.0,
        inside.y + row as f32 * line + line * 0.5,
    )
}
pub(crate) fn doc_drag(
    layout: &Layout,
    panel: &Settings,
    from: (usize, usize),
    to: (usize, usize),
) -> crate::select::Selection {
    let mut selection = crate::select::Selection::new(
        crate::select::Where::SettingsDoc,
        doc_spot(layout, panel, from.0, from.1),
    );
    selection.extend(doc_spot(layout, panel, to.0, to.1));
    selection
}
/// The rectangles the band is painted with, in the document's own box.
pub(crate) fn doc_bands(out: &Rendered) -> Vec<[f32; 4]> {
    let inside = out.layout.settings_doc_text;
    out.scene
        .rects
        .iter()
        .filter(|rect| rect.rgba() == out.skin.select)
        .map(|rect| rect.xywh())
        .filter(|[x, y, _, _]| *x >= inside.x - 0.01 && *y >= inside.y - 0.01)
        .collect()
}
/// Where a press at that pixel lands in the document.
pub(crate) fn doc_spot(layout: &Layout, panel: &Settings, row: usize, at: usize) -> crate::select::Spot {
    let (x, y) = doc_cell(layout, row, at);
    crate::spot_in_doc(layout, panel, x, y, 13.0, 8.0).expect("a character under the pointer")
}
/// The end of the section that is showing, for a test about a row near the
/// bottom of a list longer than a window.
///
/// A section of cards is taller than the panel: the AGENT section is its
/// cards with the prompt block as the last row of it. The window clamps
/// whatever this asks for to the last screenful it can start on, which is
/// exactly what the wheel does.
pub(crate) fn scrolled_to_the_end(panel: &mut Settings) {
    let rows = 8;
    while panel.scroll(4, true, rows, 80) {}
}
/// Whether two rectangles share a pixel.
pub(crate) fn overlap(a: Panel, b: Panel) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// A panel on MCP with one server configured, which the plain fixture has
/// none of.
pub(crate) fn a_servers_panel() -> Settings {
    let mut agent = an_agent();
    agent.mcp = crate::agent::Mcp {
        global: Some(std::path::PathBuf::from("/home/hec/.config/noob/mcp.json")),
        project: None,
        any_file: true,
        servers: vec![crate::agent::Server {
            name: String::from("deepwiki"),
            how: String::from("https://mcp.deepwiki.com/mcp"),
            project: false,
            on: true,
            entry: String::from("{ \"url\": \"https://mcp.deepwiki.com/mcp\" }"),
        }],
        trouble: Vec::new(),
    };
    let mut panel = Settings::open(
        &Config::default(),
        Some(std::path::Path::new("/home/hec/.config/noob/no0b.conf")),
        agent,
    );
    let at = panel
        .section_names()
        .iter()
        .position(|name| *name == crate::settings::MCP)
        .expect("MCP is a section");
    panel.choose(at);
    panel
}
/// An agent with instructions of its own, for the system prompt section's
/// document.
pub(crate) fn an_agent_with_instructions() -> crate::agent::Agent {
    crate::agent::Agent {
        instructions: crate::agent::Instructions {
            path: Some(std::path::PathBuf::from("/home/hec/.config/noob/AGENTS.md")),
            body: vec![
                String::from("# Global instructions"),
                String::new(),
                String::from("Answer in as few words as carry the answer."),
            ],
            capped: false,
        },
        ..an_agent()
    }
}
