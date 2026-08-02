//! The SKILLS section: what is installed under the agent's `skills/`, and the
//! card one more is installed with.
//!
//! One of the settings panel's nested section boxes, and one of the two that
//! carry state of their own: [`SkillsSection`] owns the install field, the
//! validate verdict and the install cycle. The frame owns the cursor, the
//! editing buffer and the process that actually clones; what arrives back
//! lands here and the rows are rebuilt from it.

use crate::agent::Agent;
use crate::settings::{Align, Card, CardField, Doing, Kept, Paper, Row, Table, TableOf};

/// The columns of the installed list, in order: the name, the width in columns
/// and how the cell is aligned. Zero is the column that takes whatever is left
/// of the row.
///
/// The names live here because they are what the section says about itself, and
/// the widths live with them so the header and the cells under it cannot come
/// apart.
pub const SKILL_COLUMNS: [(&str, usize, Align); 4] = [
    ("skill", 22, Align::Left),
    ("on", 5, Align::Left),
    ("where it is", 34, Align::Left),
    ("what it is for", 0, Align::Left),
];

/// One row per installed skill, with the web search the CLI ships with at the
/// top of them.
///
/// The shipped one is read off the machine rather than off the skills
/// directory: it is a tool of the CLI's own, so the row says whether the agent
/// has it and where it came from, and nothing on it can be turned off from
/// here.
fn skill_rows(agent: &Agent) -> Vec<Kept> {
    let mut rows = vec![Kept {
        id: String::new(),
        cells: vec![
            String::from("web search"),
            String::from(match crate::agent::websearch_on() {
                true => "yes",
                false => "no",
            }),
            String::from("shipped with noob"),
            String::from(
                "searches the web and reads pages, through the websearch program on PATH",
            ),
        ],
        marked: false,
        on: None,
        doc: websearch_doc(),
    }];
    rows.extend(agent.skills.iter().map(|skill| Kept {
        id: skill.dir.clone(),
        cells: vec![
            skill.name.clone(),
            String::from(match skill.on {
                true => "yes",
                false => "no",
            }),
            match &skill.repo {
                Some(repo) => repo.clone(),
                // Nothing on disk records the repository of an installed skill,
                // so where it is is the truthful cell.
                None => skill.path.display().to_string(),
            },
            skill.about.clone(),
        ],
        marked: false,
        on: Some(skill.on),
        doc: skill.doc.clone(),
    }));
    rows
}

/// What the column beside the table says about the shipped web search: what it
/// is, and what to do when the row says the agent does not have it.
fn websearch_doc() -> Vec<String> {
    let mut out = vec![
        String::from("# web search"),
        String::new(),
        String::from(
            "Ships with noob rather than being installed here: the CLI registers its \
             `websearch` tool when the `websearch` program is on PATH, and a sub-agent \
             given `web` tools has this and nothing else.",
        ),
        String::new(),
    ];
    match crate::agent::websearch_on() {
        true => out.push(String::from(
            "It is on. `websearch searxng up` starts the instance the searches run through.",
        )),
        false => out.extend([
            String::from("It is off: the program is not on PATH."),
            String::new(),
            String::from("    uv tool install websearch-skill"),
            String::new(),
            String::from(
                "`NOOB_WEBSEARCH` names another program to run instead, or turns the tool off.",
            ),
        ]),
    }
    out
}

/// The key the install field carries.
///
/// Not a key of either file, deliberately: every other [`Row::Field`] on this
/// panel is a line of the agent's `.env` and Enter on one writes that line, so
/// a second field added without a key of its own would write whatever was typed
/// into it under whatever key it was given. `main` branches on this before the
/// write and starts an install instead.
pub const SKILL_SOURCE: &str = "skill source";

/// What the install of a skill is doing.
///
/// Three states, because a clone is given two minutes, so the panel has to be
/// able to say it is running, and a failure has to be readable rather than a
/// window that did nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installing {
    /// It has been started and has not answered yet.
    Running { source: String },
    /// It finished. `said` is what it answered, a line at a time, and `bad` is
    /// whether that is a failure.
    Done {
        source: String,
        said: Vec<String>,
        bad: bool,
    },
}

/// The section's own state: the install field, the validate verdict, and the
/// install that is running or last ran.
pub struct SkillsSection {
    /// What has been typed into the install field, kept across a rebuild so an
    /// install that failed leaves the address on screen to be corrected rather
    /// than making it be typed again.
    source: String,
    /// The last source the validate button checked, and its verdict. The
    /// install button only exists while this matches what the field holds and
    /// the verdict is good; typing anything else voids it.
    checked: Option<(String, Result<String, String>)>,
    /// The install that is running, or the last one that ran. Nothing until one
    /// is asked for, which is what keeps the block off the section.
    install: Option<Installing>,
}

impl SkillsSection {
    /// Fresh state over what is installed. The field starts empty: the web
    /// search a fresh config wants is the CLI's own tool, and it is a row of
    /// the list rather than something to install here.
    pub fn new() -> SkillsSection {
        SkillsSection {
            source: String::new(),
            checked: None,
            install: None,
        }
    }

    /// What is in the install field right now. Only the tests want it on its
    /// own: the window reads it off the field the row builder put it in.
    #[cfg(test)]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Take what the install field holds. `typed` is a running edit the frame
    /// ended for this, when there was one: a button that read the row instead
    /// would install the last thing that was saved rather than the address on
    /// screen.
    pub fn take_source(&mut self, typed: Option<String>) -> String {
        if let Some(typed) = typed {
            self.source = typed;
        }
        self.source = self.source.trim().to_string();
        self.source.clone()
    }

    /// The validate button's answer, shown under the card and voided the
    /// moment the field says something else.
    pub fn note_check(&mut self, source: String, verdict: Result<String, String>) {
        self.checked = Some((source, verdict));
    }

    /// Whether the field's current source is the one the validate button
    /// approved. What turns the card's button from validate into install.
    pub fn checked_ok(&self) -> bool {
        matches!(&self.checked, Some((source, Ok(_))) if *source == self.source && !source.is_empty())
    }

    /// Say that an install has started, so the section says so while it runs.
    pub fn begin_install(&mut self, source: String) {
        self.source = source.clone();
        self.install = Some(Installing::Running { source });
    }

    /// Take what the install answered.
    ///
    /// The message goes on the block above the list, whichever way it went,
    /// because a git failure is several lines and a footer holds one. What was
    /// typed is kept on a failure and cleared on the one that worked: an
    /// address that failed is one to correct, and one that landed is a field
    /// ready for the next skill.
    pub fn end_install(&mut self, source: String, answer: Result<String, String>) {
        let (said, bad) = match answer {
            Ok(name) => (
                vec![
                    format!("installed {name}"),
                    String::new(),
                    format!("it is in the list below, turned on. The agent picks it up on its next session."),
                ],
                false,
            ),
            Err(why) => (why.lines().map(str::to_string).collect(), true),
        };
        if !bad {
            self.source = String::new();
        }
        self.install = Some(Installing::Done { source, said, bad });
    }

    /// The block over the list saying what the last install did, or nothing at
    /// all until one has been asked for.
    fn install_paper(&self) -> Option<Paper> {
        let title = String::from("THE LAST INSTALL");
        Some(match self.install.as_ref()? {
            Installing::Running { source } => Paper {
                title,
                under: format!("installing {source}\u{2026}"),
                body: vec![String::from(
                    "fetching it, reading its SKILL.md and putting it in place.",
                )],
                first: 0,
                does: false,
                bad: false,
            },
            Installing::Done { source, said, bad } => Paper {
                title,
                under: match bad {
                    true => format!("could not install {source}"),
                    false => format!("from {source}"),
                },
                body: said.clone(),
                first: 0,
                does: false,
                bad: *bad,
            },
        })
    }

    /// What is installed under the agent's `skills/`, what has been turned off
    /// into the sibling beside it, and the field one more is installed with.
    ///
    /// Two columns: these rows are the left one, and the skill under the cursor
    /// carries its own `SKILL.md` for the right one. Each row is the skill's
    /// name with the repository it records underneath, or, since nothing the
    /// CLI writes records where a skill came from, the directory it was found
    /// in instead.
    ///
    /// The card at the top is the one thing this section could not do: it could
    /// list, turn off and delete, and installing one meant a terminal.
    pub fn rows(&self, agent: &Agent) -> Vec<Row> {
        // The form first, the list under it. It was the other way round, and
        // a card at the foot of a list of skills read as a note about the
        // last one rather than as the way to add another: "must be extremely
        // clear a install a skill, IS A FORM. AND UNDER A LIST".
        let mut rows = Vec::new();
        rows.push(Row::Card(Card {
            beside: false,
            group: None,
            // The act the card exists for: validate what was typed, and once
            // the source checks out, install it. One button, two steps.
            does: Some(match self.checked_ok() {
                true => Doing::Install,
                false => Doing::Validate,
            }),
            title: String::from("INSTALL A SKILL"),
            fields: vec![
                CardField::link("repository or folder", SKILL_SOURCE, self.source.clone())
                    .saying("a git address, an owner/name, or a folder with a SKILL.md in it"),
                CardField::reading(
                    "installed in",
                    match &agent.skills_at {
                        Some(path) => path.display().to_string(),
                        None => String::from("nowhere: no config directory"),
                    },
                )
                .saying("the agent reads this folder in every project, not just this one"),
            ],
            hint: Some(String::from(
                "a skill is a directory in there with a SKILL.md in it; turning one off moves it \
                 beside that folder, where the agent does not look, and uninstall deletes it",
            )),
        }));
        // The validate button's verdict, under the card it answers, for the
        // source the field still holds; a verdict about something no longer
        // typed says nothing and is not shown.
        if let Some((source, verdict)) = &self.checked
            && *source == self.source
        {
            rows.push(Row::Note {
                text: match verdict {
                    Ok(what) => format!("valid: {what}"),
                    Err(why) => why.clone(),
                },
                bad: verdict.is_err(),
            });
        }
        if let Some(paper) = self.install_paper() {
            rows.push(Row::Paper(paper));
        }
        // Then what is installed, as a table in a card of its own: one row per
        // skill, the columns naming themselves, and the row under the cursor
        // written out in the column beside it. One card headed with the count,
        // the way the conversations are listed.
        rows.push(Row::Table(Table {
            of: TableOf::Skills,
            columns: &SKILL_COLUMNS,
            rows: skill_rows(agent),
            first: 0,
            cursor: 0,
        }));
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent;
    use crate::config::Config;
    use crate::settings::testing::*;
    use crate::settings::{card_body_lines, card_is_reachable, lines, Deed, Settings, SKILLS};
    use std::path::PathBuf;

    /// An agent whose skills directory holds the skills named.
    fn a_skills_agent(names: &[&str]) -> Agent {
        Agent {
            skills_at: Some(PathBuf::from("/home/hec/.config/noob/skills")),
            skills: names
                .iter()
                .map(|name| agent::Skill {
                    dir: String::from(*name),
                    name: String::from(*name),
                    about: format!("What {name} is for."),
                    repo: None,
                    path: PathBuf::from("/home/hec/.config/noob/skills").join(name),
                    on: true,
                    doc: vec![format!("# {name}")],
                })
                .collect(),
            ..Agent::default()
        }
    }

    /// The one block of text in the skills section, which is there only once an
    /// install has been asked for.
    fn the_install_block(panel: &Settings) -> &Paper {
        panel
            .rows()
            .iter()
            .find_map(|row| match row {
                Row::Paper(paper) => Some(paper),
                _ => None,
            })
            .expect("the install block")
    }

    /// Every name on the list, in order.
    /// The installed skills the table lists, without the web search the CLI
    /// ships with, which is the one row that is not a directory here.
    fn the_skills(panel: &Settings) -> Vec<String> {
        the_table(panel)
            .rows
            .iter()
            .filter(|row| row.on.is_some())
            .map(|row| row.cells[0].clone())
            .collect()
    }

    /// Put the keys on the row of the table that lists one skill.
    fn on_the_skill(panel: &mut Settings, name: &str) {
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Table(_)))
            .expect("the installed table");
        panel.point_at(at, crate::settings::Side::Left);
        while the_table(panel).at_cursor().map(|row| row.cells[0].as_str()) != Some(name) {
            assert!(panel.step(true), "{name} is not a row of the table");
        }
    }

    /// The section's one table.
    fn the_table(panel: &Settings) -> &Table {
        panel
            .rows()
            .iter()
            .find_map(|row| match row {
                Row::Table(table) => Some(table),
                _ => None,
            })
            .expect("the installed table")
    }

    /// The skills section lists what is on the disk: a row per skill, the
    /// repository it records or the directory it was found in underneath, and
    /// its own `SKILL.md` in the column beside the list.
    ///
    /// This used to assert two `Row::Item` strings, which is the read-only list
    /// it was. The rows carry an identity now because they can be turned off
    /// and uninstalled, and the section is two columns.
    #[test]
    fn the_skills_section_lists_what_is_on_the_disk() {
        let dir = scratch_dir("skills-section");
        let skills = dir.join("skills");
        std::fs::create_dir_all(skills.join("coding")).expect("a directory");
        std::fs::write(
            skills.join("coding").join("SKILL.md"),
            "---\nname: coding\ndescription: Changing code that already exists.\n---\n\n# Changing code\n\nRead it first.\n",
        )
        .expect("a file");
        std::fs::create_dir_all(skills.join("web-search")).expect("a directory");
        std::fs::write(
            skills.join("web-search").join("SKILL.md"),
            "---\nname: web-search\ndescription: Search the web.\nrepo: https://github.com/someone/web-search\n---\n\n# Searching\n",
        )
        .expect("a file");
        // Installed and turned off, which is a move rather than a delete: it is
        // still on the list and still says what it is.
        std::fs::create_dir_all(agent::skills_off(&skills).join("noisy")).expect("a directory");
        std::fs::write(
            agent::skills_off(&skills).join("noisy").join("SKILL.md"),
            "---\nname: noisy\ndescription: Talks too much.\n---\n",
        )
        .expect("a file");

        let agent = Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, agent);
        go_to(&mut panel, SKILLS);

        // One table, headed by the count, with the web search the CLI ships
        // with first and every installed directory under it. The name, whether
        // the agent loads it, where it is and what it is for are four columns,
        // which is what the list of three-line cards was.
        let table = the_table(&panel);
        assert_eq!(
            table.columns.iter().map(|(name, ..)| *name).collect::<Vec<_>>(),
            ["skill", "on", "where it is", "what it is for"]
        );
        assert!(table.title().contains("SKILLS INSTALLED"), "{}", table.title());
        let listed: Vec<(&str, &str, &str)> = table
            .rows
            .iter()
            .skip(1)
            .map(|row| {
                (
                    row.cells[0].as_str(),
                    row.cells[1].as_str(),
                    row.cells[2].as_str(),
                )
            })
            .collect();
        // The shipped row first, whose on-cell is read off the machine this
        // runs on rather than off the skills directory.
        assert_eq!(table.rows[0].cells[0], "web search");
        assert_eq!(table.rows[0].cells[2], "shipped with noob");
        assert_eq!(
            listed,
            vec![
                ("coding", "yes", skills.join("coding").display().to_string().leak() as &str),
                (
                    "noisy",
                    "no",
                    agent::skills_off(&skills)
                        .join("noisy")
                        .display()
                        .to_string()
                        .leak() as &str
                ),
                ("web-search", "yes", "https://github.com/someone/web-search"),
            ],
            "the table does not list what is on the disk"
        );
        // The shipped row is not a directory: nothing turns it off or deletes
        // it, and every installed one is both.
        assert_eq!(
            table.rows.iter().map(|row| row.on).collect::<Vec<_>>(),
            vec![None, Some(true), Some(false), Some(true)]
        );
        assert!(
            said(&panel).contains(&skills.display().to_string()),
            "the panel does not say where they live"
        );

        // The section opens on the form: installing one is what somebody
        // comes here to do, and the list of what is already installed reads
        // under it.
        assert!(
            matches!(panel.at_cursor(), Some(Row::Field { key, .. }) if *key == SKILL_SOURCE),
            "the section does not open on the install form: {:?}",
            panel.at_cursor()
        );
        // The column beside the table is the row the keys are on, and the first
        // row while the cursor is still on the form: a column that went blank
        // until somebody walked into the list would be a column nobody knew was
        // a document.
        assert_eq!(panel.showing().expect("something to show").name, "web search");
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Table(_)))
            .expect("the skills are listed");
        assert!(panel.point_at(at, crate::settings::Side::Left));
        assert!(panel.step(true), "the keys do not walk the table");
        let showing = panel.showing().expect("something to show");
        assert_eq!(showing.name, "coding");
        assert_eq!(
            showing.doc,
            vec![
                String::from("# Changing code"),
                String::new(),
                String::from("Read it first."),
            ],
            "the front matter is not the document"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The install card stands under the list, its address field validated
    /// before its button will install: the button reads validate until the
    /// source checks out, then install.
    ///
    /// "on skills allow to install more by command as well". The list could
    /// show, turn off and delete, and the only way to add one was a terminal.
    #[test]
    fn a_skill_is_installed_from_a_field_at_the_top_of_the_section() {
        let mut panel = a_panel_showing(vec![String::from("# coding")]);
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Card(card) if card.title == "INSTALL A SKILL"))
            .expect("the install card is on the section");
        assert_eq!(at, 0, "the form does not stand over the list");
        let Some(Row::Card(card)) = panel.row(at) else {
            panic!("no card at {at}");
        };
        assert_eq!(
            card.does,
            Some(Doing::Validate),
            "an unchecked source must be validated before it installs"
        );
        assert_eq!(card.does.expect("an action").word(), "validate");
        // The field it is typed into, and the folder it lands in, which is the
        // one the agent reads in every project.
        assert!(card.fields[0].editable(), "the source cannot be typed into");
        assert!(!card.fields[1].editable(), "the folder is not a reading");
        assert!(
            card.fields[1]
                .value()
                .contains("/home/hec/.config/noob/skills")
        );
        assert!(card_is_reachable(card), "a field nothing can reach");
        // And the row claims the extra line its footer takes, or the button is
        // drawn under the card it belongs to.
        assert!(
            lines(&Row::Card(card.clone()), 90)
                > crate::design::card_row_lines(card_body_lines(card, 90), false),
            "the footer costs the row nothing"
        );

        // What is typed is what is installed, whether the edit was ended with
        // Enter or left running when the button was pressed. The field is the
        // top of the section, which is where the cursor opens.
        panel.point_at(at, crate::settings::Side::Left);
        assert!(
            matches!(panel.at_cursor(), Some(Row::Field { key, .. }) if *key == SKILL_SOURCE),
            "the install field is not the card's first: {:?}",
            panel.at_cursor()
        );
        assert!(panel.edit());
        // The field opens holding the standard suggestion; typing another
        // source starts by taking it out, the way a person would.
        while panel.editing().is_some_and(|typed| !typed.is_empty()) {
            assert!(panel.backspace());
        }
        assert!(panel.type_text("someone/writing"));
        assert!(
            panel.hint().contains("installs it"),
            "the footer does not say what enter does: {}",
            panel.hint()
        );
        assert_eq!(panel.take_source(), "someone/writing");
        assert!(panel.editing().is_none(), "the edit is still running");
        // Whitespace around it is not part of an address.
        assert_eq!(panel.take_source(), "someone/writing");
    }

    /// What the section says while an install runs, when it fails, and when it
    /// lands.
    ///
    /// Every one of the things that can go wrong is a message rather than a
    /// button that answered a press with nothing: a git failure is several
    /// lines, so it goes in a block over the list where all of it can be read.
    #[test]
    fn a_failed_install_says_why_and_a_good_one_brings_the_list_back_off_the_disk() {
        let config = Config::default();
        let mut panel = a_panel_showing(vec![String::from("# coding")]);
        assert!(
            !panel.rows().iter().any(|row| matches!(row, Row::Paper(_))),
            "the block is on the section before anything has been installed"
        );

        panel.begin_install(String::from("someone/writing"), &config);
        let block = the_install_block(&panel);
        assert!(block.under.contains("installing someone/writing"), "{block:?}");
        assert!(!block.bad);
        assert_eq!(panel.source(), "someone/writing", "the field lost what was typed");

        // A failure: every line of what it said, in the bad tint, with the
        // address left on screen to be corrected.
        panel.adopt_install(
            String::from("someone/writing"),
            Err(String::from(
                "git clone failed: repository 'https://github.com/someone/writing.git' not found",
            )),
            a_skills_agent(&["coding"]),
            &config,
        );
        let block = the_install_block(&panel);
        assert!(block.bad, "a failed install is not marked as one");
        assert!(block.under.contains("could not install"), "{block:?}");
        assert_eq!(
            block.body,
            vec![String::from(
                "git clone failed: repository 'https://github.com/someone/writing.git' not found"
            )]
        );
        assert_eq!(panel.source(), "someone/writing");
        assert_eq!(the_skills(&panel), ["coding"], "a failure changed the list");

        // A clone that says several lines says all of them, since git writes
        // its reason on one line and its advice on the next.
        panel.adopt_install(
            String::from("someone/writing"),
            Err(String::from("could not read Username\nfatal: could not read")),
            a_skills_agent(&["coding"]),
            &config,
        );
        assert_eq!(the_install_block(&panel).body.len(), 2);

        // And one that landed. The list comes off the reading handed in with it
        // and not out of what the install said, and the field is empty for the
        // next one.
        panel.adopt_install(
            String::from("someone/writing"),
            Ok(String::from("writing")),
            a_skills_agent(&["coding", "writing"]),
            &config,
        );
        let block = the_install_block(&panel);
        assert!(!block.bad);
        assert!(block.body[0].contains("installed writing"), "{block:?}");
        assert_eq!(the_skills(&panel), ["coding", "writing"]);
        assert_eq!(panel.source(), "", "the field still holds the last address");
    }

    /// The toggle on a skill's row is a move on the disk and back, and what the
    /// row says next comes from reading the disk again rather than from
    /// remembering what was pressed.
    #[test]
    fn turning_a_skill_off_moves_it_and_the_row_reads_the_disk_again() {
        let dir = scratch_dir("skill-toggle");
        let skills = dir.join("skills");
        std::fs::create_dir_all(skills.join("coding")).expect("a directory");
        std::fs::write(
            skills.join("coding").join("SKILL.md"),
            "---\nname: coding\ndescription: Change code.\n---\n\n# Changing code\n",
        )
        .expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        go_to(&mut panel, SKILLS);
        // The section opens on the form; the table is under it, and the keys
        // walk its rows once the cursor is on it. The first row is the web
        // search the CLI ships with, and the installed skill is under it.
        let at = panel
            .rows()
            .iter()
            .position(|row| matches!(row, Row::Table(_)))
            .expect("the skill is listed");
        on_the_skill(&mut panel, "coding");

        let deed = panel.turn_row(at).expect("the row turns off");
        assert_eq!(
            deed,
            Deed::TurnSkill {
                dir: String::from("coding"),
                on: false,
            }
        );
        // Nothing turns off the shipped row: it is a tool of the CLI's own, and
        // the buttons say so by doing nothing on it.
        let table = the_table(&panel);
        assert_eq!(table.rows[0].on, None, "the shipped row can be turned off");

        // What `main` does with it, and then what the panel does with the disk.
        agent::set_skill(panel.skills_at().expect("a skills directory"), "coding", false)
            .expect("it moves");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, SKILLS);
        on_the_skill(&mut panel, "coding");
        assert_eq!(
            the_table(&panel).at_cursor().expect("a row").on,
            Some(false),
            "the row still says it is on"
        );
        assert!(!skills.join("coding").exists());

        let back = panel.turn_row(at).expect("and back");
        assert_eq!(
            back,
            Deed::TurnSkill {
                dir: String::from("coding"),
                on: true,
            }
        );
        agent::set_skill(panel.skills_at().expect("a skills directory"), "coding", true)
            .expect("it comes back");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, SKILLS);
        on_the_skill(&mut panel, "coding");
        assert_eq!(the_table(&panel).at_cursor().expect("a row").on, Some(true));
        assert!(skills.join("coding/SKILL.md").is_file());

        // And the uninstall beside it takes two presses and names the same
        // directory: the only thing on this panel that cannot be undone.
        assert_eq!(panel.uninstall(at), None, "one press deleted a skill");
        assert_eq!(panel.arming(), Some(at));
        assert!(
            panel.says().contains("press uninstall again"),
            "the panel does not say what is about to go: {}",
            panel.says()
        );
        assert_eq!(
            panel.uninstall(at),
            Some(Deed::RemoveSkill {
                dir: String::from("coding"),
                on: true,
            })
        );
        assert_eq!(panel.arming(), None, "it stayed armed after it fired");

        // And anything else at all disarms it, so a button pressed once and
        // walked away from cannot be finished off by the next press that lands.
        assert_eq!(panel.uninstall(at), None);
        assert!(panel.step(true) || panel.arming().is_none());
        assert_eq!(panel.arming(), None, "a key left it armed");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
