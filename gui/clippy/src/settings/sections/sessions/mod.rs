//! The SESSIONS section: the conversations the agent has already written, as
//! one table inside one card.
//!
//! One of the settings panel's nested section boxes. It owns the table's
//! vocabulary, the columns, the cell formatting and the section's title word;
//! the frame owns the cursor, the table's scroll machinery and the delete.

use std::path::Path;
use std::time::SystemTime;

use crate::agent::Agent;
use crate::settings::{Card, CardField, Kept, Row, Table};

/// Which edge of its column a cell is written against.
///
/// A table is read down its columns, and a number is read down its last digit:
/// `283 B` and `1.2 MB` written from the left put the digits that matter in a
/// different place on every row, so a column of them cannot be compared at a
/// glance. The numbers are written against the right edge of their column
/// instead, which is what makes them line up; words stay on the left, where a
/// word is read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// The columns of the saved-conversations table: what each one is called, how
/// many characters wide it is, and which edge its cells are written against.
///
/// The first one is the mark, which is why it has no name: a word over a column
/// of ticks is a word describing a control that says what it is by being one.
/// Zero is a column with no width of its own, and there is one: the first
/// message takes whatever is left of the row.
///
/// The names live here rather than in the drawing because they are what the
/// section says about itself. The widths and the alignment live here too so the
/// header and the cells under it cannot come apart: one list, read by the row
/// builder and by the layout.
pub const SESSION_COLUMNS: [(&str, usize, Align); 6] = [
    ("", 4, Align::Left),
    ("when", 10, Align::Left),
    ("folder", 20, Align::Left),
    ("size", 9, Align::Right),
    ("context", 9, Align::Right),
    ("first message", 0, Align::Left),
];

/// How many of [`SESSION_COLUMNS`] carry text. The first one is the mark.
pub const SESSION_CELLS: usize = SESSION_COLUMNS.len() - 1;


/// How many conversations are on screen inside the table at once.
///
/// A number rather than what fits, for the reason [`crate::settings::PAPER_LINES`]
/// is one: the height of a row cannot depend on the height of the window,
/// because [`crate::settings::lines`] is what the scroll window counts in and
/// what the layout places with. The table scrolls inside its own body instead,
/// and the card stays where it is.
pub const TABLE_ROWS: usize = 12;

/// How tall the table's body is, in lines: the row naming the columns, and the
/// [`TABLE_ROWS`] of conversations under it.
pub fn table_body_lines() -> f32 {
    crate::design::TEXT_LINES
        + crate::design::TIGHT
        + TABLE_ROWS as f32 * crate::design::TEXT_LINES
}

/// The name of the section's list, said once, in the panel's own heading.
///
/// "SETTINGS SESSIONS" over a rail that also says SESSIONS over a row labelled
/// "sessions" was the same word three times and never once said what the list
/// underneath was. The heading at the top of the panel takes this instead of
/// the rail's word ([`crate::settings::Settings::title`]), so the one line that
/// says where you are says what is listed; the section under it no longer
/// carries a title row of its own, because a title said twice on one screen is
/// the trouble the rail word was.
pub const SESSION_TITLE: &str = "SESSIONS";

/// The conversations the agent has already written, read with the same
/// reader the folder picker offers them with.
/// Two cards: where the transcripts are kept, and the table of them.
///
/// No title row: the body's own title says SESSIONS
/// ([`SESSION_TITLE`]), and a section that repeats its heading two lines
/// under it is the same noise the rail's word was.
pub fn rows(agent: &Agent) -> Vec<Row> {
    let empty = agent.sessions.sessions.is_empty();
    let mut rows = vec![Row::Card(Card {
        beside: false,
        group: None,
        does: None,
        title: String::from("WHERE SESSIONS ARE KEPT"),
        fields: vec![CardField::reading(
            "folder",
            match crate::sessions::dir() {
                Some(dir) => dir.display().to_string(),
                None => String::from("nowhere: no config directory"),
            },
        )],
        hint: Some(String::from(match empty {
            true => "none saved yet: the agent writes one transcript here per session",
            false => "one row of the table is one session the agent has already had",
        })),
    })];
    if !empty {
        rows.push(Row::Table(Table {
            of: crate::settings::TableOf::Sessions,
            columns: &SESSION_COLUMNS,
            rows: agent
                .sessions
                .sessions
                .iter()
                .map(|saved| Kept {
                    id: saved.id.clone(),
                    cells: session_cells(saved, agent.now).to_vec(),
                    marked: false,
                    on: None,
                    doc: Vec::new(),
                })
                .collect(),
            first: 0,
            cursor: 0,
        }));
    }
    for why in &agent.sessions.skipped {
        rows.push(Row::Note {
            text: why.clone(),
            bad: true,
        });
    }
    rows
}

/// What a session row says: when it was, which folder it belongs to, how big
/// the transcript is, how full its context window was, and the opening of the
/// first thing that was said in it.
///
/// The same five things in the same order the picker's session rows use, so the
/// list here and the list a session is resumed from are recognisably the same
/// sessions. The two middle ones are formatted by the picker's own helpers
/// rather than written again here, which is what keeps them saying the same
/// thing. Only the wording for a session with no folder differs, because the
/// picker's row says where pressing it would resume and this panel resumes
/// nothing.
fn session_cells(
    saved: &crate::sessions::Saved,
    now: SystemTime,
) -> [String; SESSION_CELLS] {
    let folder = match (&saved.workspace, saved.gone) {
        (Some(path), true) => format!("{} (gone)", short_folder(path)),
        (Some(path), false) => short_folder(path),
        // Written by the CLI rather than by this window, so nothing ever noted
        // where it was.
        (None, _) => String::from("no folder noted"),
    };
    let said = match saved.opening.is_empty() {
        true => String::from("nothing was said"),
        false => saved.opening.clone(),
    };
    [
        crate::sessions::ago(saved.when, now),
        folder,
        crate::picker::size_label(saved.bytes),
        crate::picker::context_label(saved.context),
        said,
    ]
}

/// The name of the folder and no more of the path, which is what the picker's
/// session rows say. The two lists have to read the same or they are two lists.
fn short_folder(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::settings::testing::*;
    use crate::settings::{lines, section_title, Settings, APPEARANCE, SECTIONS, SESSIONS};
    use std::path::PathBuf;
    use std::time::Duration;

    /// The table of saved conversations on the section that is showing.
    fn the_table(panel: &Settings) -> &Table {
        panel
            .rows()
            .iter()
            .find_map(|row| match row {
                Row::Table(table) => Some(table),
                _ => None,
            })
            .expect("the section carries a table")
    }

    /// Where that table stands on the section, which is what every press on it
    /// carries.
    fn the_table_row(panel: &Settings) -> usize {
        panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Table(_)))
            .expect("the section carries a table")
    }

    /// The sessions on the panel are the sessions the picker offers, read with
    /// the same reader and said the same way.
    #[test]
    fn the_sessions_section_matches_what_the_picker_shows() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let listing = crate::sessions::Listing {
            sessions: vec![
                a_session(
                    "aaa",
                    120,
                    Some("/home/hec/workspace/noob-cli"),
                    "fix the panel",
                ),
                a_session("bbb", 7200, None, ""),
            ],
            skipped: vec![String::from("ccc: no meta line")],
        };
        let agent = Agent {
            now,
            sessions: listing.clone(),
            ..Agent::default()
        };
        let mut panel = Settings::open(&Config::default(), None, agent);
        go_to(&mut panel, SESSIONS);
        let table = the_table(&panel);
        let listed: Vec<(String, Vec<String>)> = table
            .rows
            .iter()
            .map(|row| (row.id.clone(), row.cells.clone()))
            .collect();
        assert_eq!(
            listed.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(),
            vec!["aaa", "bbb"],
            "a row must carry the name a delete needs"
        );

        let mut picker = crate::picker::Picker::open(
            Box::new(crate::picker::Fixed(Vec::new())),
            PathBuf::from("/home/hec/workspace/noob-cli"),
            Vec::new(),
        );
        picker.show_sessions_at(listing, now);
        let offered: Vec<Vec<String>> = picker
            .rows()
            .iter()
            .filter_map(|row| match row {
                crate::picker::Row::Session(saved) => Some(picker.session_cells(saved).to_vec()),
                _ => None,
            })
            .collect();
        assert_eq!(offered.len(), 2, "the picker offers both");
        assert_eq!(listed.len(), offered.len(), "{listed:?} against {offered:?}");
        // Cell by cell rather than as one joined string, which is what both
        // lists were before either of them was a table.
        for ((_, mine), theirs) in listed.iter().zip(&offered) {
            assert_eq!(mine.len(), SESSION_CELLS);
            assert_eq!(theirs.len(), SESSION_CELLS);
            for (step, (mine, theirs)) in mine.iter().zip(theirs).enumerate() {
                // The picker says "this folder" for a session with no folder
                // noted, because that is where pressing it would resume; this
                // panel resumes nothing, so it says what it knows. Every other
                // cell is said the same way, in the same order.
                let theirs = theirs.replace("this folder", "no folder noted");
                assert_eq!(*mine, theirs, "column {step}");
            }
        }
        assert_eq!(listed[0].1[0], "2m ago", "{listed:?}");
        assert_eq!(listed[0].1[1], "noob-cli", "{listed:?}");
        assert_eq!(listed[0].1[4], "fix the panel", "{listed:?}");
        assert_eq!(listed[1].1[4], "nothing was said", "{listed:?}");

        // Every column has a name over it, and the header is one row of the
        // section rather than a word floated above the list.
        let header: Vec<&str> = table.columns.iter().map(|(name, ..)| *name).collect();
        assert_eq!(
            header,
            SESSION_COLUMNS.iter().map(|(name, ..)| *name).collect::<Vec<_>>()
        );
        assert_eq!(header.len(), SESSION_CELLS + 1, "the first column is the mark");
        // Every column that carries text is named. The first one is the mark,
        // and a word over a column of ticks describes a control that says what
        // it is by being one.
        assert!(header[0].is_empty(), "the mark is headed by a word");
        for name in &header[1..] {
            assert!(!name.is_empty(), "a column with no name");
        }

        // What the section is is said by the title inside its body, which the
        // painter draws from `panel.title()`; the section's own rows never
        // carry a bare title row of their own.
        assert_eq!(panel.title(), SESSION_TITLE);
        assert_eq!(section_title(SESSIONS), SESSION_TITLE);
        let text = said(&panel);
        assert!(text.contains("WHERE SESSIONS ARE KEPT"), "{text}");
        // The card over the table says how many there are, which is the one
        // thing a header here can say that the panel's own heading does not.
        assert_eq!(table.title(), "2 SESSIONS", "{text}");
        // Every other section is headed by the word the rail marks it with:
        // only this one lists something its rail word does not name.
        for name in SECTIONS {
            if name == SESSIONS {
                continue;
            }
            assert_eq!(section_title(name), name);
        }

        // A file that could not be read is said rather than quietly missing.
        assert!(
            panel
                .rows()
                .iter()
                .any(|row| matches!(row, Row::Note { text, bad } if *bad && text.contains("ccc"))),
            "{:?}",
            panel.rows()
        );
    }

    /// Item E3: up and down pick a saved conversation and leave the rail alone.
    ///
    /// They used to walk the rail whenever the keyboard was not inside a
    /// section, and in this section it never was: every row of it was a reading
    /// the cursor could not land on. So the two keys anybody reaches for in a
    /// list swapped the whole right hand side of the panel out instead of
    /// moving down it. The rail is pressed now and the keys are the rows.
    #[test]
    fn up_and_down_pick_a_session_and_leave_the_rail_where_it_is() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let mut panel = Settings::open(
            &Config::default(),
            None,
            Agent {
                now,
                sessions: crate::sessions::Listing {
                    sessions: vec![
                        a_session("aaa", 60, Some("/home/hec/one"), "first"),
                        a_session("bbb", 120, Some("/home/hec/two"), "second"),
                        a_session("ccc", 180, Some("/home/hec/three"), "third"),
                    ],
                    skipped: Vec::new(),
                },
                ..Agent::default()
            },
        );
        go_to(&mut panel, SESSIONS);
        let rail = panel.chosen();
        // Which conversation the keys are on. Inside the table now, because the
        // table is one row of the panel and a conversation is one row of the
        // table: the arrow keys walk the rows of the card the cursor is on.
        let at = |panel: &Settings| match panel.at_cursor() {
            Some(Row::Table(table)) => table
                .at_cursor()
                .map(|row| row.id.clone())
                .expect("the table has no row under the keys"),
            other => panic!("the cursor is not on the table: {other:?}"),
        };

        // It opens on the first conversation rather than on the title or the
        // path above it: a cursor on a row nothing can be done to is a dead
        // stop, and the band drawn on one would be a lie.
        assert_eq!(at(&panel), "aaa");
        assert!(panel.on_row(), "there is nothing for the band to sit on");

        assert!(panel.step(true));
        assert_eq!(at(&panel), "bbb");
        assert!(panel.step(true));
        assert_eq!(at(&panel), "ccc");
        assert!(!panel.step(true), "the end of the list is a stop");
        assert!(panel.step(false));
        assert_eq!(at(&panel), "bbb");
        assert_eq!(panel.chosen(), rail, "a key moved the rail");

        // Home and End and the page keys are the same list, not the rail.
        assert!(panel.jump(true));
        assert_eq!(at(&panel), "ccc");
        assert!(panel.jump(false));
        assert_eq!(at(&panel), "aaa");
        panel.page(4, true);
        assert_eq!(at(&panel), "ccc");
        assert_eq!(panel.chosen(), rail, "a key moved the rail");

        // The rail still answers a press, and each section keeps its own cursor
        // across the swap. Tab moves it too now, which is
        // `tab_walks_the_sections_and_the_arrows_walk_the_rows`; what matters
        // here is that no arrow key does.
        let looks = panel
            .section_names()
            .iter()
            .position(|name| *name == APPEARANCE)
            .expect("the appearance section");
        assert!(panel.choose(looks));
        assert_eq!(panel.title(), APPEARANCE);
        assert!(panel.choose(rail));
        assert_eq!(at(&panel), "ccc", "the section lost where the cursor was");
    }

    /// Item H3: the table is marked, scrolled inside itself, and the marks
    /// outlive everything except the conversations they are on.
    ///
    /// The marks are per row and the armed delete is per panel, and the two have
    /// deliberately different lifetimes: an armed delete is disarmed by any
    /// other input, and a set the arrow keys emptied could only ever hold one
    /// row, which is no multi selection at all.
    #[test]
    fn the_table_keeps_its_marks_while_the_keys_walk_it() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let many: Vec<crate::sessions::Saved> = (0..TABLE_ROWS + 3)
            .map(|at| {
                a_session(
                    &format!("s{at:02}"),
                    60 * (at as u64 + 1),
                    Some("/home/hec/one"),
                    "said something",
                )
            })
            .collect();
        let mut panel = Settings::open(
            &Config::default(),
            None,
            Agent {
                now,
                sessions: crate::sessions::Listing {
                    sessions: many.clone(),
                    skipped: Vec::new(),
                },
                ..Agent::default()
            },
        );
        go_to(&mut panel, SESSIONS);
        let index = the_table_row(&panel);
        assert_eq!(the_table(&panel).rows.len(), many.len());
        assert_eq!(the_table(&panel).title(), format!("{} SESSIONS", many.len()));

        // Two of them, marked, with the keys walked between the two: neither the
        // arrow keys nor the page keys take a mark off.
        assert!(panel.mark(index, 0));
        assert!(panel.step(true));
        assert!(panel.step(true));
        assert!(panel.mark(index, 2));
        assert!(panel.step(false));
        panel.page(4, true);
        let table = the_table(&panel);
        assert_eq!(table.chosen(), 2, "a key took a mark off");
        assert_eq!(table.taking(), vec![String::from("s00"), String::from("s02")]);
        assert_eq!(table.title(), format!("{} SESSIONS, 2 CHOSEN", many.len()));

        // Marked again is unmarked, and the delete then falls back to the row
        // the keys are on rather than taking nothing.
        assert!(panel.mark(index, 0));
        assert!(panel.mark(index, 2));
        let table = the_table(&panel);
        assert_eq!(table.chosen(), 0);
        assert_eq!(table.taking().len(), 1, "a delete with nothing marked takes one row");

        // The body holds a fixed number of rows and scrolls inside itself, so
        // the card is one row of the panel however long the list is. The keys
        // walked past the end of the body bring the body with them.
        assert_eq!(lines(panel.row(index).expect("the table"), 80), crate::design::card_row_lines(table_body_lines(), true));
        assert!(panel.jump(true));
        let table = the_table(&panel);
        assert_eq!(table.cursor, many.len() - 1);
        assert!(table.first > 0, "the body did not follow the keys");
        assert!(table.cursor < table.first + TABLE_ROWS, "the row is off the body");
        assert!(panel.jump(false));
        assert_eq!(the_table(&panel).first, 0, "the body did not come back");

        // Select all is every conversation on the list and not only the ones the
        // body is showing; select none is all of them again.
        assert!(panel.mark_all(index, true));
        assert_eq!(the_table(&panel).chosen(), many.len());
        assert_eq!(the_table(&panel).taking().len(), many.len());
        assert!(panel.mark_all(index, false));
        assert_eq!(the_table(&panel).chosen(), 0);

        // And the marks outlive a walk to another section and back, the way the
        // cursor does: they are the section's, not the keystroke's.
        assert!(panel.mark(index, 1));
        let looks = panel
            .section_names()
            .iter()
            .position(|name| *name == APPEARANCE)
            .expect("the appearance section");
        let rail = panel.chosen();
        assert!(panel.choose(looks));
        assert!(panel.choose(rail));
        assert_eq!(the_table(&panel).chosen(), 1, "the marks went with the rail");

        // An armed delete does not: anything else at all puts it back, marking
        // included, because the question on the footer names a set and this is
        // that set changing.
        assert_eq!(panel.uninstall(index), None);
        assert_eq!(panel.arming(), Some(index));
        assert!(panel.mark(index, 2));
        assert_eq!(panel.arming(), None, "the question outlived the answer");
    }
}
