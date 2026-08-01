//! The COMMANDS section: the slash commands as a read-only list, one row per
//! command with the doc column showing the one under the cursor.
//!
//! One of the settings panel's nested section boxes, and the only one with no
//! state at all: the rows come off the command registry
//! ([`crate::commands::ALL`]) and nothing here is settable, turnable or
//! removable. The registry is the single source: this section, `/help` and
//! the dispatcher all read the same entries, so the list can never say a
//! command the prompt would refuse.

use crate::commands;
use crate::settings::{Entry, Row, Which};

/// The registry as rows: a note saying where these are typed, then one fixed
/// entry per command, in registry order. The entry's document is the
/// command's manual, which is also what `/help <name>` prints.
pub fn rows() -> Vec<Row> {
    let mut rows = vec![Row::Note {
        text: String::from(
            "Typed into the prompt, a /command does what this panel does. \
             /help lists them, /help <name> tells one's whole story, and the \
             column beside this list shows the command under the cursor.",
        ),
        bad: false,
    }];
    for command in &commands::ALL {
        rows.push(Row::Entry(Entry {
            name: format!("/{}", command.name),
            about: command.about.to_string(),
            under: commands::spelled(command),
            on: true,
            what: Which::Fixed,
            removable: false,
            doc: commands::manual(command),
        }));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::settings::testing::*;
    use crate::settings::COMMANDS;

    /// The section is the registry, in order: one row per command carrying
    /// its name, its line, its usage and its manual, under the opening note.
    #[test]
    fn the_section_lists_the_registry() {
        let mut panel = over(&Config::default());
        go_to(&mut panel, COMMANDS);
        let entries: Vec<&Entry> = panel
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Entry(entry) => Some(entry),
                _ => None,
            })
            .collect();
        assert_eq!(entries.len(), commands::ALL.len());
        for (entry, command) in entries.iter().zip(commands::ALL.iter()) {
            assert_eq!(entry.name, format!("/{}", command.name));
            assert_eq!(entry.about, command.about);
            assert_eq!(entry.under, commands::spelled(command));
            assert_eq!(entry.doc, commands::manual(command));
            assert_eq!(entry.what, Which::Fixed);
            assert!(entry.on && !entry.removable);
        }
        assert!(
            matches!(panel.rows().first(), Some(Row::Note { text, bad: false }) if text.contains("/help")),
            "the note over the list says where these are typed"
        );
    }

    /// Read-only means read-only: the cursor lands on a command's row and
    /// shows its manual in the doc column, but no press produces a deed.
    #[test]
    fn the_rows_are_only_read() {
        let mut panel = over(&Config::default());
        go_to(&mut panel, COMMANDS);
        let at = panel.cursor();
        assert!(
            matches!(panel.rows().get(at), Some(Row::Entry(_))),
            "the cursor opens on the first command"
        );
        let doc = panel.doc_pane();
        let text: Vec<String> = (0..doc.last())
            .filter_map(|at| doc.line(at).map(|line| line.text.clone()))
            .collect();
        assert_eq!(text, commands::manual(&commands::ALL[0]));
        assert_eq!(panel.toggle(at), None, "nothing to turn");
        assert_eq!(panel.uninstall(at), None, "nothing to remove");
        assert_eq!(panel.uninstall(at), None, "and a second press arms nothing");
    }
}
