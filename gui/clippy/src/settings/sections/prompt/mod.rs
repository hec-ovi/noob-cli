//! The SYSTEM PROMPT section: the agent's global `AGENTS.md` as one document,
//! edited in place.
//!
//! The CLI reads `<config dir>/AGENTS.md` and puts it at the top of every
//! prompt, in every folder, before the project's own. This section is that
//! file: where it is, what is in it, the offer to write a starter one when
//! there is none, and the editor that changes it.
//!
//! One of the settings panel's nested section boxes, and one of the ones that
//! carry state of their own: [`PromptSection`] owns the document's editor (the
//! lines being typed, the caret, the scroll that follows it) the way the MCP
//! box owns its add card's fields. The frame routes the keys here and the
//! save is a deed done in `main`, through the agent-files box, so nothing in
//! this file touches a disk. The frame's own editing line is single-line by
//! design (one buffer, caret at the end); a document needs lines and a caret
//! that walks them, which is why the editor lives here instead of growing
//! every single-line path in the frame.

use crate::agent::{self, Agent};
use crate::settings::{Card, CardField, Paper, Row, PAPER_LINES};

/// The section's own state: the document's editor, while it is open.
#[derive(Default)]
pub struct PromptSection {
    editing: Option<Editor>,
}

/// The document being retyped: its lines, the caret, and the first line the
/// block shows, kept so the caret is always on screen.
struct Editor {
    lines: Vec<String>,
    /// The caret, as the line it is on and the character it stands before.
    line: usize,
    col: usize,
    /// Which line of the buffer the block starts on.
    first: usize,
}

impl Editor {
    /// Keep the caret inside the block's own window, the way every cursor in
    /// this window is kept on screen.
    fn reveal(&mut self) {
        self.first = self.first.min(self.lines.len().saturating_sub(PAPER_LINES));
        if self.line < self.first {
            self.first = self.line;
        }
        if self.line >= self.first + PAPER_LINES {
            self.first = self.line + 1 - PAPER_LINES;
        }
    }

    /// How long the line the caret is on is, in characters.
    fn width(&self) -> usize {
        self.lines
            .get(self.line)
            .map_or(0, |line| line.chars().count())
    }

    /// Where the caret's character starts, as a byte of its line.
    fn at(&self) -> usize {
        self.lines
            .get(self.line)
            .map_or(0, |line| {
                line.char_indices()
                    .nth(self.col)
                    .map_or(line.len(), |(at, _)| at)
            })
    }
}

impl PromptSection {
    /// Whether the document is being edited.
    pub fn editing(&self) -> bool {
        self.editing.is_some()
    }

    /// Open the editor on the file's lines, with the caret at the top.
    pub(crate) fn begin(&mut self, body: &[String]) {
        let lines = match body.is_empty() {
            true => vec![String::new()],
            false => body.to_vec(),
        };
        self.editing = Some(Editor {
            lines,
            line: 0,
            col: 0,
            first: 0,
        });
    }

    /// Drop the editor and everything typed into it. True when there was one.
    pub(crate) fn cancel(&mut self) -> bool {
        self.editing.take().is_some()
    }

    /// The whole text as the save would write it, with the editor left open:
    /// it closes when the write lands ([`PromptSection::end`]), so a refusal
    /// loses nothing.
    pub(crate) fn take(&self) -> Option<String> {
        let editor = self.editing.as_ref()?;
        Some(editor.lines.join("\n"))
    }

    /// The save landed: the editor closes and the file speaks for itself.
    pub(crate) fn end(&mut self) {
        self.editing = None;
    }

    /// Type into the caret. Control characters are dropped; a newline is
    /// [`PromptSection::newline`], on its own key.
    pub(crate) fn insert(&mut self, text: &str) -> bool {
        let Some(editor) = self.editing.as_mut() else {
            return false;
        };
        let mut typed = false;
        for ch in text.chars().filter(|ch| !ch.is_control()) {
            let at = editor.at();
            let Some(line) = editor.lines.get_mut(editor.line) else {
                break;
            };
            line.insert(at, ch);
            editor.col += 1;
            typed = true;
        }
        editor.reveal();
        typed
    }

    /// Split the line at the caret, which is what Enter means in a document.
    pub(crate) fn newline(&mut self) -> bool {
        let Some(editor) = self.editing.as_mut() else {
            return false;
        };
        let at = editor.at();
        let Some(line) = editor.lines.get_mut(editor.line) else {
            return false;
        };
        let rest = line.split_off(at);
        editor.lines.insert(editor.line + 1, rest);
        editor.line += 1;
        editor.col = 0;
        editor.reveal();
        true
    }

    /// Take the character before the caret back off, joining two lines when
    /// the caret is at the head of one.
    pub(crate) fn backspace(&mut self) -> bool {
        let Some(editor) = self.editing.as_mut() else {
            return false;
        };
        if editor.col > 0 {
            editor.col -= 1;
            let at = editor.at();
            if let Some(line) = editor.lines.get_mut(editor.line) {
                line.remove(at);
            }
            editor.reveal();
            return true;
        }
        if editor.line == 0 {
            return false;
        }
        let taken = editor.lines.remove(editor.line);
        editor.line -= 1;
        editor.col = editor.width();
        if let Some(line) = editor.lines.get_mut(editor.line) {
            line.push_str(&taken);
        }
        editor.reveal();
        true
    }

    /// The caret one line up or down, holding its column to the line it lands
    /// on.
    pub(crate) fn step(&mut self, down: bool) -> bool {
        let Some(editor) = self.editing.as_mut() else {
            return false;
        };
        let next = match down {
            true => (editor.line + 1).min(editor.lines.len().saturating_sub(1)),
            false => editor.line.saturating_sub(1),
        };
        if next == editor.line {
            return false;
        }
        editor.line = next;
        editor.col = editor.col.min(editor.width());
        editor.reveal();
        true
    }

    /// The caret one character along, wrapping over a line's end onto the
    /// next, the way every text caret walks.
    pub(crate) fn cross(&mut self, right: bool) -> bool {
        let Some(editor) = self.editing.as_mut() else {
            return false;
        };
        match right {
            true if editor.col < editor.width() => editor.col += 1,
            true if editor.line + 1 < editor.lines.len() => {
                editor.line += 1;
                editor.col = 0;
            }
            false if editor.col > 0 => editor.col -= 1,
            false if editor.line > 0 => {
                editor.line -= 1;
                editor.col = editor.width();
            }
            _ => return false,
        }
        editor.reveal();
        true
    }

    /// Where the caret is, as the line of the block's body and the character
    /// along it, for whoever draws it.
    pub(crate) fn caret(&self) -> Option<(usize, usize)> {
        self.editing.as_ref().map(|editor| (editor.line, editor.col))
    }

    /// The file, then the document: a card naming where it is, and the text
    /// itself as a block. While the editor is open the block shows the buffer,
    /// scrolled to keep the caret on screen; the file itself has not changed.
    pub fn rows(&self, agent: &Agent) -> Vec<Row> {
        let paper = match self.editing.as_ref() {
            Some(editor) => Paper {
                title: String::from("AGENTS.md"),
                under: String::from(
                    "being edited \u{2022} nothing lands in the file until ctrl+s",
                ),
                body: editor.lines.clone(),
                first: editor.first,
                offer: None,
                bad: false,
            },
            None => document(agent),
        };
        vec![Row::Card(file_card(agent)), Row::Paper(paper)]
    }
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
/// Not called the prompt: it is one capped layer of one.
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
        Deed, Settings, AGENT, APPEARANCE, COMMANDS, MCP, PROMPT, SECTIONS, SESSIONS, SKILLS,
    };

    /// The section is on the rail between AGENT and SESSIONS, and it shows the
    /// global AGENTS.md as a document: the file's path as a reading, the text
    /// itself as a block.
    #[test]
    fn the_section_names_the_file_and_shows_the_document() {
        assert_eq!(
            SECTIONS,
            [AGENT, PROMPT, SESSIONS, SKILLS, MCP, COMMANDS, APPEARANCE],
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

    /// Enter opens the editor on the document, the keys change the buffer and
    /// walk the caret, and the save writes the whole file through the agent
    /// box: what the block shows next is the file read back.
    #[test]
    fn editing_changes_the_buffer_and_the_save_writes_the_file() {
        let config = Config::default();
        let dir = scratch_dir("prompt-edit");
        let path = dir.join(agent::AGENTS_MD);
        std::fs::write(&path, "# Mine\n\nbe brief\n").expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&config, None, read());
        go_to(&mut panel, PROMPT);
        let at = panel.cursor();

        // The legend says the way in, and Enter takes it: the block shows the
        // buffer with the caret at its head, and the footer says how to save
        // and how to leave.
        assert!(panel.hint().contains("enter edits it"), "{}", panel.hint());
        assert!(panel.edit_instructions(&config));
        assert!(panel.editing_instructions());
        assert!(!panel.edit_instructions(&config), "already editing");
        assert_eq!(panel.instructions_caret(at), Some((0, 0)));
        assert!(panel.hint().contains("ctrl+s writes the file"), "{}", panel.hint());
        assert!(panel.hint().contains("esc"), "{}", panel.hint());

        // Typing lands at the caret; the arrows walk it; Enter breaks a line;
        // backspace at a line's head joins it back.
        assert!(panel.type_instructions("!", &config));
        assert_eq!(panel.instructions_caret(at), Some((0, 1)));
        let body = &panel.paper(at).expect("the block").body;
        assert_eq!(body[0], "!# Mine", "{body:?}");
        assert!(panel.instructions_backspace(&config));
        assert!(panel.instructions_step(true, &config), "down a line");
        assert!(panel.instructions_step(true, &config));
        assert!(panel.instructions_cross(true, &config), "right a character");
        assert_eq!(panel.instructions_caret(at), Some((2, 1)));
        assert!(panel.instructions_newline(&config));
        assert_eq!(panel.instructions_caret(at), Some((3, 0)));
        assert_eq!(panel.paper(at).expect("the block").body[2..4], ["b", "e brief"]);
        assert!(panel.instructions_backspace(&config), "the join");
        assert_eq!(panel.instructions_caret(at), Some((2, 1)));
        assert!(panel.instructions_cross(false, &config));
        assert!(panel.instructions_cross(false, &config), "left over the line end");
        assert_eq!(panel.instructions_caret(at), Some((1, 0)));
        assert!(panel.type_instructions("says: ", &config));
        assert_eq!(
            panel.paper(at).expect("the block").body,
            ["# Mine", "says: ", "be brief"]
        );

        // Nothing has reached the disk: the buffer is the panel's until the
        // save, and the save is the deed that writes the whole file.
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file"),
            "# Mine\n\nbe brief\n",
            "the edit reached the file early"
        );
        let deed = panel.finish_instructions().expect("something to save");
        let Deed::SaveInstructions { path: to, text } = &deed else {
            panic!("{deed:?}");
        };
        assert_eq!(to, &path);
        assert_eq!(text, "# Mine\nsays: \nbe brief");
        agent::write_instructions(to, text).expect("the file takes it");
        panel.end_instructions_edit();
        panel.adopt_agent(read(), &config);
        go_to(&mut panel, PROMPT);
        assert!(!panel.editing_instructions());
        assert_eq!(
            panel.paper(at).expect("the block").body,
            ["# Mine", "says: ", "be brief"],
            "the block does not show what the file really says"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The block follows the caret while it is edited, so what is being typed
    /// is always on screen.
    #[test]
    fn the_block_follows_the_caret_past_its_own_edge() {
        let config = Config::default();
        let dir = scratch_dir("prompt-follow");
        let long: Vec<String> = (0..PAPER_LINES * 2).map(|at| format!("line {at}")).collect();
        std::fs::write(dir.join(agent::AGENTS_MD), long.join("\n")).expect("a file");
        let mut panel = Settings::open(
            &config,
            None,
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );
        go_to(&mut panel, PROMPT);
        let at = panel.cursor();
        assert!(panel.edit_instructions(&config));
        for _ in 0..PAPER_LINES + 2 {
            assert!(panel.instructions_step(true, &config));
        }
        assert_eq!(panel.instructions_caret(at), Some((PAPER_LINES + 2, 0)));
        assert_eq!(
            panel.paper(at).expect("the block").first,
            3,
            "the block did not scroll with the caret"
        );
        for _ in 0..PAPER_LINES + 2 {
            assert!(panel.instructions_step(false, &config));
        }
        assert!(!panel.instructions_step(false, &config), "above the first line");
        assert_eq!(panel.paper(at).expect("the block").first, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Escape abandons the edit: the buffer goes, the block reads the file
    /// again, and the file was never touched.
    #[test]
    fn escape_leaves_the_file_untouched() {
        let config = Config::default();
        let dir = scratch_dir("prompt-escape");
        let path = dir.join(agent::AGENTS_MD);
        std::fs::write(&path, "keep me\n").expect("a file");
        let mut panel = Settings::open(
            &config,
            None,
            Agent::read(Some(&dir), None, crate::sessions::Listing::default()),
        );
        go_to(&mut panel, PROMPT);
        let at = panel.cursor();
        assert!(panel.edit_instructions(&config));
        assert!(panel.type_instructions("gone ", &config));
        assert!(panel.cancel_instructions(&config));
        assert!(!panel.editing_instructions());
        assert!(!panel.cancel_instructions(&config), "nothing left to cancel");
        assert_eq!(panel.paper(at).expect("the block").body, ["keep me"]);
        assert_eq!(panel.instructions_caret(at), None, "a caret with no edit");
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file"),
            "keep me\n",
            "escape reached the file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A save that cannot land is said on the footer and loses nothing: the
    /// editor stays open with everything typed, and the file is as it was.
    /// A file longer than the CLI reads is refused at the way in, because
    /// saving the capped text would quietly lose the tail.
    #[test]
    fn a_failed_write_is_said_and_the_buffer_survives_it() {
        let config = Config::default();
        let dir = scratch_dir("prompt-refused");
        let path = dir.join(agent::AGENTS_MD);
        std::fs::write(&path, "mine\n").expect("a file");
        let read = || Agent::read(Some(&dir), None, crate::sessions::Listing::default());
        let mut panel = Settings::open(&config, None, read());
        go_to(&mut panel, PROMPT);
        assert!(panel.edit_instructions(&config));
        assert!(panel.type_instructions("more ", &config));

        // The write refuses (a directory stands where the file goes), which is
        // what `main` says on the footer; the editor still holds the text.
        let deed = panel.finish_instructions().expect("something to save");
        let Deed::SaveInstructions { text, .. } = &deed else {
            panic!("{deed:?}");
        };
        let blocked = dir.join("blocked").join(agent::AGENTS_MD);
        std::fs::create_dir_all(&blocked).expect("a directory in the way");
        let why = agent::write_instructions(&blocked, text).expect_err("it cannot land");
        panel.say_trouble(why);
        assert!(
            panel.trouble().is_some_and(|why| why.contains(&blocked.display().to_string())),
            "{:?}",
            panel.trouble()
        );
        assert!(panel.editing_instructions(), "the refusal closed the editor");
        assert_eq!(
            panel.finish_instructions(),
            Some(deed),
            "the refusal lost what was typed"
        );
        assert_eq!(std::fs::read_to_string(&path).expect("the file"), "mine\n");

        // Past the CLI's cap the editor refuses to open at all, with the
        // reason on the footer: a saved capped buffer would lose the tail.
        std::fs::write(&path, "x".repeat(agent::AGENTS_CAP as usize + 500)).expect("a file");
        let mut panel = Settings::open(&config, None, read());
        go_to(&mut panel, PROMPT);
        assert!(!panel.edit_instructions(&config));
        assert!(!panel.editing_instructions());
        assert!(
            panel.trouble().is_some_and(|why| why.contains("16 KiB")),
            "{:?}",
            panel.trouble()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
