//! The MCP section: the configured servers out of the two files the CLI
//! merges, and the ADD A SERVER card under them.
//!
//! One of the settings panel's nested section boxes, and one of the two that
//! carry state of their own: [`McpSection`] owns the add card's two fields
//! until its button writes them. The frame owns the cursor, the editing
//! buffer and the write; the deed built here is what `main` does.

use std::path::Path;

use crate::agent::{self, Agent};
use crate::settings::{Card, CardField, Deed, Doing, Entry, Row, Which};

/// The add-a-server card's two fields, held on the panel the way the skill
/// source is: keys of the model, never written into any file themselves.
pub const SERVER_NAME: &str = "server name";
pub const SERVER_HOW: &str = "server command";

/// The section's own state: the add card's two fields, until its button
/// writes them.
#[derive(Default)]
pub struct McpSection {
    server_name: String,
    server_how: String,
}

impl McpSection {
    /// Keep a running edit on the add card's two fields: the text goes into
    /// the panel, never into any file. `key` is which of the two the frame
    /// found under the cursor.
    pub fn keep_edit(&mut self, key: &str, typed: String) {
        match key == SERVER_NAME {
            true => self.server_name = typed.trim().to_string(),
            false => self.server_how = typed.trim().to_string(),
        }
    }

    /// The add card's press, as the deed that writes the global file, or the
    /// refusal the frame says on the panel.
    pub fn add_deed(&self) -> Result<Deed, String> {
        let (name, how) = (self.server_name.clone(), self.server_how.clone());
        if name.is_empty() || how.is_empty() {
            return Err(String::from(
                "a server needs its name and its command or url",
            ));
        }
        Ok(Deed::AddServer { name, how })
    }

    /// After the deed landed: the card goes back to empty, ready for the next
    /// one. On a refusal the fields keep what was typed.
    pub fn clear(&mut self) {
        self.server_name.clear();
        self.server_how.clear();
    }

    /// The MCP servers, out of the two files the CLI merges.
    ///
    /// The same two columns the skills are: a row per server with what it is
    /// underneath, and that server's entry out of its own file beside them.
    pub fn rows(&self, agent: &Agent) -> Vec<Row> {
        let mcp = &agent.mcp;
        // The two files, as one card of two fields. They go side by side while
        // the panel is wide enough for both to keep their columns and stack
        // when it is not, which is the whole of what the panel does about a
        // narrow window: cards are full width, their contents reflow.
        //
        // Said in full rather than shown as an empty list, which reads as a
        // panel that failed to load one.
        let hint = match (mcp.any_file, mcp.servers.is_empty() && mcp.trouble.is_empty()) {
            (false, _) | (true, true) => "none configured yet: the next session loads what is added",
            (true, false) => "a project .noob/mcp.json wins for a server named in both files",
        };
        // The configured list first, the add card under it: what is running
        // is the section's subject, and adding one is the act at the bottom.
        let mut rows = Vec::new();
        for server in &mcp.servers {
            rows.push(Row::Entry(Entry {
                name: server.name.clone(),
                about: server.how.clone(),
                under: server_under(server, file(agent, server.project)),
                on: server.on,
                what: Which::Server {
                    project: server.project,
                },
                // A server is a few lines somebody wrote in a file, and taking
                // those lines out is the one thing this list could not do. It
                // is the same two-press uninstall a skill carries, and it goes
                // through the same rename, so a press that fails leaves the
                // file exactly as it was.
                removable: true,
                doc: server_doc(server),
            }));
        }
        rows.push(Row::Card(Card {
            beside: false,
            group: None,
            does: Some(Doing::AddServer),
            title: String::from("ADD A SERVER"),
            fields: vec![
                CardField::text("name", SERVER_NAME, self.server_name.clone())
                    .saying("what the agent's tools call it"),
                CardField::text("command or url", SERVER_HOW, self.server_how.clone())
                    .saying("a command line to start it, or an http(s) address it already answers on"),
            ],
            hint: Some(format!(
                "written into {}, which the agent reads in every project{}",
                match &mcp.global {
                    Some(path) => path.display().to_string(),
                    None => String::from("nowhere: no config directory"),
                },
                match hint.is_empty() {
                    true => String::new(),
                    false => format!("; {hint}"),
                }
            )),
        }));
        for why in &mcp.trouble {
            rows.push(Row::Note {
                text: why.clone(),
                bad: true,
            });
        }
        rows
    }
}

/// Which `mcp.json` a server belongs to, which is the only file a toggle on
/// its row writes.
pub fn file(agent: &Agent, project: bool) -> Option<&Path> {
    match project {
        true => agent.mcp.project.as_deref(),
        false => agent.mcp.global.as_deref(),
    }
}

/// The third line of a server's row: which of the two files its entry is in,
/// and where that file is. The same place a skill's row says where it was found.
fn server_under(server: &agent::Server, file: Option<&Path>) -> String {
    let where_ = match server.project {
        true => "project",
        false => "global",
    };
    match file {
        Some(path) => format!("{where_}: {}", path.display()),
        None => format!("{where_} file"),
    }
}

/// What the column beside the list shows for a server: its entry out of the
/// file, exactly as the file carries it.
///
/// Fenced as JSON so the highlighter reads it the way it reads any other code
/// block: the whole column is Markdown, and a skill's own document is the thing
/// it was built for. No heading of its own: the column is titled with the name
/// now, and a document that opened with the same name said it twice.
fn server_doc(server: &agent::Server) -> Vec<String> {
    let mut out = vec![String::from("```json")];
    out.extend(server.entry.lines().map(str::to_string));
    out.push(String::from("```"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(clippy::wildcard_imports)]
    use crate::settings::testkit::*;
    #[allow(clippy::wildcard_imports)]
    use crate::view::testkit::*;
    
    
    use crate::design;
    use noob_draw::Text;
    use crate::config::Config;
    use crate::settings::testing::*;
    use crate::settings::{Settings, MCP};

    /// A malformed mcp.json is a line on the panel, not a panel that will not
    /// open. The servers from the file that did parse are still listed.
    #[test]
    fn a_broken_mcp_file_is_a_line_on_the_panel() {
        let dir = scratch_dir("broken");
        std::fs::create_dir_all(dir.join("work/.noob")).expect("a scratch directory");
        std::fs::write(dir.join("mcp.json"), "{\"servers\": {").expect("a file");
        std::fs::write(
            dir.join("work/.noob/mcp.json"),
            r#"{"servers": {"docs": {"url": "http://localhost:9000/mcp"}}}"#,
        )
        .expect("a file");
        let agent = Agent::read(
            Some(&dir),
            Some(&dir.join("work")),
            crate::sessions::Listing::default(),
        );
        let mut panel = Settings::open(&Config::default(), None, agent);
        go_to(&mut panel, MCP);
        let rows = panel.rows();
        assert!(
            rows.iter().any(|row| matches!(row, Row::Entry(entry)
                if entry.name == "docs"
                    // The name, what it is and where it came from, each on a
                    // line of its own rather than one run of text.
                    && entry.about == "http://localhost:9000/mcp"
                    && entry.under.contains("project")
                    && entry.under.contains("mcp.json")
                    && entry.on)),
            "{rows:?}"
        );
        assert!(
            rows.iter().any(|row| matches!(row, Row::Note { text, bad }
                if *bad && text.contains("not valid JSON"))),
            "the broken file is not reported: {rows:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server's row toggles the same way, in its own file, and a server that
    /// is off is still a row rather than a server that disappeared.
    #[test]
    fn turning_a_server_off_moves_its_entry_in_its_own_file() {
        let dir = scratch_dir("server-toggle");
        std::fs::write(
            dir.join("mcp.json"),
            r#"{"servers": {"docs": {"url": "http://localhost:9000/mcp"}}}"#,
        )
        .expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        go_to(&mut panel, MCP);
        let at = panel.cursor();
        assert!(
            matches!(panel.row(at), Some(Row::Entry(entry)) if entry.on && entry.removable),
            "{:?}",
            panel.row(at)
        );
        // The entry itself is what the column beside the list shows.
        let showing = panel.showing().expect("something to show");
        assert!(showing.doc.iter().any(|line| line.contains("localhost:9000")), "{:?}", showing.doc);

        let deed = panel.toggle(at).expect("an entry toggles");
        assert_eq!(
            deed,
            Deed::TurnServer {
                name: String::from("docs"),
                project: false,
                on: false,
            }
        );
        let file = panel.mcp_file(false).expect("a global file").to_path_buf();
        assert_eq!(file, dir.join("mcp.json"));
        agent::set_server(&file, "docs", false).expect("it moves");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, MCP);
        assert!(
            matches!(panel.row(panel.cursor()), Some(Row::Entry(entry)) if !entry.on),
            "{:?}",
            panel.row(panel.cursor())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The uninstall a skill has is on a server's row too, takes two presses,
    /// names the file rather than a directory, and the row is gone once the deed
    /// has been done.
    ///
    /// This used to assert the opposite: that a server had no uninstall and
    /// `uninstall` answered nothing on its row. It is written the other way
    /// round now because the button is there, which is the whole of this change.
    #[test]
    fn a_server_is_uninstalled_out_of_its_file_and_leaves_the_list() {
        let dir = scratch_dir("server-remove");
        std::fs::write(
            dir.join("mcp.json"),
            r#"{"servers": {"docs": {"url": "http://localhost:9000/mcp"}, "shell": {"command": "mcp-shell"}}, "timeout_s": 30}"#,
        )
        .expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        go_to(&mut panel, MCP);
        // The rows are sorted by name, so the first entry is "docs".
        let at = panel.cursor();
        assert!(
            matches!(panel.row(at), Some(Row::Entry(entry)) if entry.name == "docs" && entry.removable),
            "{:?}",
            panel.row(at)
        );

        // One press arms it and says what would go, in the words of what would
        // actually happen: nothing about a directory.
        assert_eq!(panel.uninstall(at), None, "one press removed a server");
        assert_eq!(panel.arming(), Some(at));
        let said = panel.says();
        assert!(said.contains("press uninstall again"), "{said}");
        assert!(said.contains("docs") && said.contains("mcp.json"), "{said}");
        assert!(!said.contains("directory"), "a server has no directory: {said}");

        let deed = panel.uninstall(at).expect("the second press is the deed");
        assert_eq!(
            deed,
            Deed::RemoveServer {
                name: String::from("docs"),
                project: false,
            }
        );
        assert_eq!(panel.arming(), None, "it stayed armed after it fired");

        // What `main` does with it, and then what the panel does with the disk.
        let file = panel.mcp_file(false).expect("a global file").to_path_buf();
        agent::remove_server(&file, "docs").expect("it goes");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, MCP);
        let names: Vec<String> = panel
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Entry(entry) => Some(entry.name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec![String::from("shell")], "the row is still listed");
        // And nothing else in the file went with it.
        let text = std::fs::read_to_string(&file).expect("the file");
        let root: serde_json::Value = serde_json::from_str(&text).expect("still JSON");
        assert_eq!(root["timeout_s"], 30, "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server that was turned off first still uninstalls, out of the key it
    /// was moved to, and the file it was in keeps everything else.
    #[test]
    fn a_server_that_is_off_uninstalls_too() {
        let dir = scratch_dir("server-remove-off");
        std::fs::write(
            dir.join("mcp.json"),
            r#"{"servers": {"docs": {"url": "http://localhost:9000/mcp"}, "shell": {"command": "mcp-shell"}}}"#,
        )
        .expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        let file = panel.mcp_file(false).expect("a global file").to_path_buf();
        agent::set_server(&file, "docs", false).expect("it moves out of the way");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, MCP);
        let at = panel.cursor();
        assert!(
            matches!(panel.row(at), Some(Row::Entry(entry)) if entry.name == "docs" && !entry.on && entry.removable),
            "{:?}",
            panel.row(at)
        );

        assert_eq!(panel.uninstall(at), None);
        assert_eq!(
            panel.uninstall(at),
            Some(Deed::RemoveServer {
                name: String::from("docs"),
                project: false,
            }),
            "a server that is off asks for a different deed"
        );
        agent::remove_server(&file, "docs").expect("an off server goes too");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, MCP);
        assert!(
            !panel
                .rows()
                .iter()
                .any(|row| matches!(row, Row::Entry(entry) if entry.name == "docs")),
            "the row came back"
        );
        let root: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&file).expect("the file"))
                .expect("still JSON");
        assert!(root["servers"]["shell"]["command"].is_string());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A removal that fails leaves the file and the row exactly as they were,
    /// and says why on the footer.
    #[test]
    fn a_failed_removal_leaves_the_file_and_the_row_where_they_were() {
        let dir = scratch_dir("server-remove-fails");
        let path = dir.join("mcp.json");
        let whole = "{\"servers\": {\"docs\": {\"url\": \"http://localhost:9000/mcp\"}}}";
        std::fs::write(&path, whole).expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        go_to(&mut panel, MCP);
        let at = panel.cursor();
        assert_eq!(panel.uninstall(at), None);
        let deed = panel.uninstall(at).expect("the deed");

        // Somebody edited the file to something that will not parse in between
        // the press and the write, which is the shape every failure here has:
        // the removal refuses and the file it refused on is untouched.
        std::fs::write(&path, "{\"servers\": {\"docs\":").expect("a file");
        let Deed::RemoveServer { name, project } = &deed else {
            panic!("{deed:?}");
        };
        let file = panel.mcp_file(*project).expect("a global file").to_path_buf();
        let why = agent::remove_server(&file, name).expect_err("it cannot be done");
        assert!(why.contains("not valid JSON"), "{why}");
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file"),
            "{\"servers\": {\"docs\":",
            "the refusal rewrote the file"
        );

        // The panel says why and the row it was on is still the row it was on.
        panel.say_trouble(why);
        assert!(
            panel.trouble().is_some_and(|why| why.contains("not valid JSON")),
            "{:?}",
            panel.trouble()
        );
        assert!(
            matches!(panel.row(at), Some(Row::Entry(entry)) if entry.name == "docs"),
            "{:?}",
            panel.row(at)
        );
        assert_eq!(panel.arming(), None, "it stayed armed after it fired");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A server's three strings are three roles too, at three sizes, the way a
    /// skill's are: the two lists are drawn by the same arm, and a server is
    /// the worse of the two to read when they all look alike, because its
    /// address and the file it came out of are both paths.
    #[test]
    fn a_server_says_its_name_its_address_and_its_file_in_three_roles() {
        let panel = a_servers_panel();
        let out = render_settings(&panel, 1400.0, 900.0, None);
        let line = Text::line_for(PANE_TEXT.0);
        let (_, row) = the_entry_row(&out, &panel);
        let (_, parts) = the_card(&out, row, true);

        let title = out
            .scene
            .texts
            .iter()
            .find(|text| (text.at.y - parts.title.y).abs() < 0.51)
            .expect("the name");
        assert_eq!(
            title.runs.iter().map(|run| run.text.as_str()).collect::<String>(),
            "deepwiki"
        );
        assert_eq!(title.size, design::card_title_size(PANE_TEXT.0));
        // The address in the body at the value size, and the file it is
        // configured in under it at the hint size.
        assert_eq!(
            line_of(&out, parts.body.x, parts.body.y),
            "https://mcp.deepwiki.com/mcp"
        );
        let under = out
            .scene
            .texts
            .iter()
            .find(|text| {
                (text.at.x - parts.body.x).abs() < 0.51
                    && (text.at.y - (parts.body.y + line + design::tight(line))).abs() < 0.51
            })
            .expect("the file it came from");
        assert!(
            under
                .runs
                .iter()
                .map(|run| run.text.as_str())
                .collect::<String>()
                .contains("mcp.json"),
            "{:?}",
            under.runs
        );
        assert!(under.size < PANE_TEXT.0);
        assert!(title.size > under.size);
    }
}
