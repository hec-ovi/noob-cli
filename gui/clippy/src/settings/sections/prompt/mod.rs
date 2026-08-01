//! The SYSTEM PROMPT section: the agent's global `AGENTS.md` as one document.
//!
//! The CLI reads `<config dir>/AGENTS.md` and puts it at the top of every
//! prompt, in every folder, before the project's own. This section is that
//! file: where it is, what is in it, and the offer to write a starter one
//! when there is none.
//!
//! One of the settings panel's nested section boxes. It builds rows out of the
//! shared vocabulary in [`crate::settings`]; the frame owns the cursor, the
//! scroll and the write.

use crate::agent::{self, Agent};
use crate::settings::{Card, CardField, Paper, Row};

/// The file, then the document: a card naming where it is, and the text
/// itself as a block the page keys read.
pub fn rows(agent: &Agent) -> Vec<Row> {
    vec![Row::Card(file_card(agent)), Row::Paper(document(agent))]
}

/// Where the file is, as a reading: the one path this whole section is about.
fn file_card(agent: &Agent) -> Card {
    Card {
        does: None,
        title: String::from("THE FILE"),
        fields: vec![CardField::reading(
            "file",
            match agent.instructions.path.as_deref() {
                Some(path) => path.display().to_string(),
                None => String::from("nowhere: no config directory to keep one in"),
            },
        )
        // The `.env` is re-read on every request; this is not. The prompt is
        // assembled once when `serve` starts, so an edit here lands on the
        // next session rather than the next message, and a section that did
        // not say so would be telling somebody their change was live when it
        // is not.
        .saying("read when a session starts, so an edit lands on the next one")],
        hint: None,
    }
}

/// The document itself, under the file's own name.
///
/// Not called the prompt. It is one capped layer of one; the whole assembled
/// prompt is the AGENT section's block, out of `noob debug prompt`.
fn document(agent: &Agent) -> Paper {
    let it = &agent.instructions;
    let title = String::from("AGENTS.md");
    let Some(path) = it.path.as_deref() else {
        return Paper {
            title,
            under: String::from("nowhere: no config directory to keep one in"),
            body: Vec::new(),
            first: 0,
            offer: None,
            bad: true,
        };
    };
    // Empty and missing are one thing here because they are one thing to the
    // agent: it trims the file and a blank one contributes no heading at all.
    if it.body.is_empty() {
        return Paper {
            title,
            under: format!("nothing at {} yet", path.display()),
            body: vec![
                String::from("The agent reads this file first, in every folder, before the"),
                String::from("project's own AGENTS.md. There is none here yet."),
            ],
            first: 0,
            offer: Some(path.to_path_buf()),
            bad: false,
        };
    }
    let mut body = it.body.clone();
    if it.capped {
        body.push(String::new());
        body.push(format!(
            "[the CLI stops reading at {} KiB, so the rest of this file is not in the prompt]",
            agent::AGENTS_CAP / 1024
        ));
    }
    Paper {
        title,
        under: String::from(
            "the first layer of every prompt, before the project's own AGENTS.md",
        ),
        body,
        first: 0,
        offer: None,
        bad: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::settings::testing::*;
    use crate::settings::{
        Deed, Settings, AGENT, APPEARANCE, MCP, PROMPT, SECTIONS, SESSIONS, SKILLS,
    };

    /// The section is on the rail between AGENT and SESSIONS, and it shows the
    /// global AGENTS.md as a document: the file's path as a reading, the text
    /// itself as a block.
    #[test]
    fn the_section_names_the_file_and_shows_the_document() {
        assert_eq!(
            SECTIONS,
            [AGENT, PROMPT, SESSIONS, SKILLS, MCP, APPEARANCE],
            "the rail order is contract data"
        );
        let dir = scratch_dir("prompt-doc");
        let path = dir.join(agent::AGENTS_MD);
        std::fs::write(&path, "# Global instructions\n\nbe brief\n").expect("a file");
        let mut panel = Settings::open(
            &Config::default(),
            None,
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );
        go_to(&mut panel, PROMPT);
        let text = said(&panel);
        assert!(
            text.contains(&path.display().to_string()),
            "the section does not say where the file is: {text}"
        );
        assert!(text.contains("session starts"), "{text}");
        assert!(text.contains("be brief"), "{text}");

        // The card naming the file is read and never landed on; the section
        // opens on the document, which the page keys read.
        let at = panel.cursor();
        assert!(panel.on_row());
        let paper = panel.paper(at).expect("the cursor opens on the document");
        assert!(paper.title.contains("AGENTS.md"), "{}", paper.title);
        assert_eq!(paper.body, ["# Global instructions", "", "be brief"]);
        assert_eq!(paper.offer, None, "there is a file to show");
        assert!(panel.hint().contains("page"), "{}", panel.hint());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// With no file there the block says so and offers to write one, and the
    /// press writes it where the agent looks rather than anywhere this window
    /// decided.
    #[test]
    fn a_missing_file_is_offered_and_the_press_writes_the_starter() {
        let dir = scratch_dir("prompt-offer");
        let path = dir.join(agent::AGENTS_MD);
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&Config::default(), None, read());
        go_to(&mut panel, PROMPT);
        let at = panel.cursor();
        let paper = panel.paper(at).expect("the document").clone();
        assert_eq!(paper.offer.as_deref(), Some(path.as_path()));
        assert!(paper.under.contains("nothing at"), "{}", paper.under);
        assert!(!paper.body.is_empty(), "an empty box says nothing at all");
        assert!(panel.hint().contains("enter"), "{}", panel.hint());

        // The press asks for the file the block named, and once it is there
        // the offer is gone and the block shows what was written.
        assert_eq!(
            panel.make(at),
            Some(Deed::StartInstructions { path: path.clone() })
        );
        agent::start_instructions(&path).expect("the file is written");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, PROMPT);
        let paper = panel.paper(at).expect("the document");
        assert_eq!(paper.offer, None, "it still offers a file that is there");
        assert!(
            paper.body.iter().any(|line| line.contains("Global instructions")),
            "{:?}",
            paper.body
        );
        assert_eq!(panel.make(at), None);

        // A whitespace-only file is nothing at all to the agent, so it is
        // nothing at all here.
        std::fs::write(&path, "\n   \n").expect("a file");
        panel.adopt_agent(read(), &Config::default());
        go_to(&mut panel, PROMPT);
        assert!(panel.paper(at).expect("the document").offer.is_some());

        // And with no config directory there is nowhere to keep one, which is
        // said as trouble rather than offered.
        let mut nowhere = Settings::open(&Config::default(), None, Agent::default());
        go_to(&mut nowhere, PROMPT);
        let paper = nowhere.paper(nowhere.cursor()).expect("the document");
        assert!(paper.bad, "no config directory is not marked as trouble");
        assert_eq!(paper.offer, None, "it offers a file with nowhere to put it");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
