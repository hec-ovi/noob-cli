//! The SYSTEM PROMPT section: the prompt the agent gets, as the three layers
//! it is built from, stacked in the order the CLI assembles them.
//!
//! The CLI reads `<config dir>/AGENTS.md`, appends `<config dir>/TOOLS.md`,
//! and then appends an environment block it computes for the request. The
//! first two are user-owned files with shipped defaults behind them: when a
//! file is absent the agent runs with the built-in text, so that is what the
//! block shows, said honestly. The third is data about the machine and the
//! moment, so its block is read and never edited; `noob debug env` prints it
//! and the frame hands what it answered in here ([`EnvBlock`]).
//!
//! One of the settings panel's nested section boxes, and one that carries
//! state of its own: [`PromptSection`] owns the document editor (the lines
//! being typed, the caret, the scroll that follows it) for whichever of the
//! two files has edition enabled. One editor at a time, because there is one
//! keyboard: enabling edition on one file drops an edit running on the other.
//! The frame routes the keys here and every save is a deed done in `main`
//! through the agent-files box, so nothing in this file touches a disk.

use crate::agent::{self, Agent, Instructions};
use crate::settings::{EnvBlock, Paper, PaperActs, Row, PAPER_LINES};

/// Which of the two user-owned prompt files a row or an editor is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptFile {
    Agents,
    Tools,
}

impl PromptFile {
    /// The file's name, which is also its block's title.
    pub(crate) fn name(self) -> &'static str {
        match self {
            PromptFile::Agents => agent::AGENTS_MD,
            PromptFile::Tools => agent::TOOLS_MD,
        }
    }

    /// The text the CLI runs with when the file is absent.
    pub(crate) fn default_text(self) -> &'static str {
        match self {
            PromptFile::Agents => agent::AGENTS_DEFAULT,
            PromptFile::Tools => agent::TOOLS_DEFAULT,
        }
    }

    /// The file's reading off the agent snapshot.
    pub(crate) fn of(self, agent: &Agent) -> &Instructions {
        match self {
            PromptFile::Agents => &agent.instructions,
            PromptFile::Tools => &agent.tools,
        }
    }

    /// Which row of the section the file's block is.
    pub(crate) fn row(self) -> usize {
        match self {
            PromptFile::Agents => 0,
            PromptFile::Tools => 1,
        }
    }
}

/// The section's own state: the document editor, while edition is enabled on
/// one of the two files.
#[derive(Default)]
pub struct PromptSection {
    editing: Option<(PromptFile, Editor)>,
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
    /// The file a row of the section is, or nothing for the rows that are not
    /// one of the two.
    pub(crate) fn file_at(index: usize) -> Option<PromptFile> {
        match index {
            0 => Some(PromptFile::Agents),
            1 => Some(PromptFile::Tools),
            _ => None,
        }
    }

    /// Whether edition is enabled anywhere in the section.
    pub fn editing(&self) -> bool {
        self.editing.is_some()
    }

    /// Which file the open editor is on, when one is open.
    pub(crate) fn editing_file(&self) -> Option<PromptFile> {
        self.editing.as_ref().map(|(file, _)| *file)
    }

    /// Which row the open editor is on, for whoever keeps scroll positions.
    pub(crate) fn editing_row(&self) -> Option<usize> {
        self.editing_file().map(PromptFile::row)
    }

    /// Enable edition on one file: the editor opens on the file's own text,
    /// or on the shipped default when there is no file, which is what "edit
    /// and save to own it" means.
    pub(crate) fn begin(&mut self, file: PromptFile, agent: &Agent) {
        let it = file.of(agent);
        let lines = match it.body.is_empty() {
            true => file.default_text().lines().map(str::to_string).collect(),
            false => it.body.clone(),
        };
        self.begin_with(file, lines);
    }

    /// The same, on lines somebody handed in: the load action's way in.
    pub(crate) fn begin_with(&mut self, file: PromptFile, lines: Vec<String>) {
        let lines = match lines.is_empty() {
            true => vec![String::new()],
            false => lines,
        };
        self.editing = Some((
            file,
            Editor {
                lines,
                line: 0,
                col: 0,
                first: 0,
            },
        ));
    }

    /// Drop the editor and everything typed into it. True when there was one.
    pub(crate) fn cancel(&mut self) -> bool {
        self.editing.take().is_some()
    }

    /// The whole text as the save would write it, with the editor left open:
    /// it closes when the write lands ([`PromptSection::end`]), so a refusal
    /// loses nothing.
    pub(crate) fn take(&self) -> Option<String> {
        let (_, editor) = self.editing.as_ref()?;
        Some(editor.lines.join("\n"))
    }

    /// The save landed: the editor closes and the file speaks for itself.
    pub(crate) fn end(&mut self) {
        self.editing = None;
    }

    /// Type into the caret. Control characters are dropped; a newline is
    /// [`PromptSection::newline`], on its own key.
    pub(crate) fn insert(&mut self, text: &str) -> bool {
        let Some((_, editor)) = self.editing.as_mut() else {
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
        let Some((_, editor)) = self.editing.as_mut() else {
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
        let Some((_, editor)) = self.editing.as_mut() else {
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
        let Some((_, editor)) = self.editing.as_mut() else {
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
        let Some((_, editor)) = self.editing.as_mut() else {
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
        self.editing
            .as_ref()
            .map(|(_, editor)| (editor.line, editor.col))
    }

    /// The section's rows: the two files in the order the CLI reads them, the
    /// environment block, and the line naming that order.
    pub fn rows(&self, agent: &Agent, env: &EnvBlock) -> Vec<Row> {
        vec![
            Row::Paper(self.file_paper(PromptFile::Agents, agent)),
            Row::Paper(self.file_paper(PromptFile::Tools, agent)),
            Row::Paper(env_paper(env)),
            Row::Note {
                text: String::from(
                    "the CLI assembles the prompt in this order: AGENTS.md, then TOOLS.md, then the environment block",
                ),
                bad: false,
            },
        ]
    }

    /// One file's block: its buffer while edition is enabled on it, the file's
    /// own text while there is one, and the shipped default, said honestly,
    /// while there is not.
    fn file_paper(&self, file: PromptFile, agent: &Agent) -> Paper {
        let title = String::from(file.name());
        let acts = Some(PaperActs {
            load: file == PromptFile::Agents,
        });
        if let Some((editing, editor)) = self.editing.as_ref()
            && *editing == file
        {
            return Paper {
                title,
                under: String::from(
                    "being edited \u{2022} nothing lands in the file until ctrl+s",
                ),
                body: editor.lines.clone(),
                first: editor.first,
                does: acts,
                bad: false,
            };
        }
        let it = file.of(agent);
        let Some(path) = it.path.as_deref() else {
            return Paper {
                title,
                under: String::from("nowhere: no config directory to keep one in"),
                body: Vec::new(),
                first: 0,
                does: None,
                bad: true,
            };
        };
        // Empty and missing are one thing because they are one thing to the
        // agent: the CLI trims the file and falls back to the shipped text,
        // so the shipped text is what the block shows.
        if it.body.is_empty() {
            return Paper {
                title,
                under: format!(
                    "{}: not written yet; this is the built-in text, enable edition and save to own it",
                    path.display()
                ),
                body: file.default_text().lines().map(str::to_string).collect(),
                first: 0,
                does: acts,
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
            under: path.display().to_string(),
            body,
            first: 0,
            does: acts,
            bad: false,
        }
    }
}

/// The block after the two files: what `noob debug env` printed, which is the
/// tail the CLI computes for every request. Read and never edited, because it
/// is data about the machine and the moment: frozen in a file it would make
/// the prompt lie as soon as noob ran in another folder or on another day.
fn env_paper(env: &EnvBlock) -> Paper {
    let title = String::from("THE ENVIRONMENT BLOCK");
    match env {
        EnvBlock::Waiting => Paper {
            title,
            under: String::from("running noob debug env\u{2026}"),
            body: Vec::new(),
            first: 0,
            does: None,
            bad: false,
        },
        EnvBlock::Got { at, body } => Paper {
            title,
            under: format!(
                "noob debug env, run in {at}; computed for each request, so there is nothing here to edit"
            ),
            body: body.clone(),
            first: 0,
            does: None,
            bad: false,
        },
        EnvBlock::Failed { at, why } => Paper {
            title,
            under: format!("{why} (run in {at})"),
            body: Vec::new(),
            first: 0,
            does: None,
            bad: true,
        },
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

    fn read(dir: &std::path::Path) -> Agent {
        Agent::read(Some(dir), None, crate::sessions::Listing::default())
    }

    /// The section is on the rail between AGENT and SESSIONS and it stacks the
    /// three layers in assembly order: AGENTS.md, TOOLS.md, the environment
    /// block, with the order named under them. Files that are there show their
    /// text under their path; the environment block shows what `noob debug
    /// env` answered, or why it did not.
    #[test]
    fn the_section_stacks_the_three_layers_in_assembly_order() {
        assert_eq!(
            SECTIONS,
            [AGENT, PROMPT, SESSIONS, SKILLS, MCP, COMMANDS, APPEARANCE],
            "the rail order is contract data"
        );
        let dir = scratch_dir("prompt-layers");
        std::fs::write(dir.join(agent::AGENTS_MD), "# Mine\n\nbe brief\n").expect("a file");
        std::fs::write(dir.join(agent::TOOLS_MD), "batch the reads\n").expect("a file");
        let mut panel = Settings::open(&Config::default(), None, read(&dir));
        go_to(&mut panel, PROMPT);
        let titles: Vec<String> = panel
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Paper(paper) => Some(paper.title.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(titles, ["AGENTS.md", "TOOLS.md", "THE ENVIRONMENT BLOCK"]);
        let text = said(&panel);
        assert!(text.contains(&dir.join(agent::AGENTS_MD).display().to_string()), "{text}");
        assert!(text.contains(&dir.join(agent::TOOLS_MD).display().to_string()), "{text}");
        assert!(text.contains("be brief"), "{text}");
        assert!(text.contains("batch the reads"), "{text}");
        assert!(
            text.contains("AGENTS.md, then TOOLS.md, then the environment block"),
            "nothing names the assembly order: {text}"
        );

        // The environment block says it is being read, then what came back,
        // with the one line on why there is nothing to edit; a run that failed
        // says why instead of showing nothing.
        let env = panel.paper(2).expect("the environment block").clone();
        assert!(env.under.contains("running noob debug env"), "{}", env.under);
        panel.adopt_env(
            String::from("/tmp/work"),
            Ok(vec![String::from("<env>"), String::from("cwd: /tmp/work")]),
            &Config::default(),
        );
        go_to(&mut panel, PROMPT);
        let env = panel.paper(2).expect("the environment block");
        assert_eq!(env.body, ["<env>", "cwd: /tmp/work"]);
        assert!(env.under.contains("nothing here to edit"), "{}", env.under);
        panel.adopt_env(
            String::from("/tmp/work"),
            Err(String::from("noob debug env failed: no such subcommand")),
            &Config::default(),
        );
        go_to(&mut panel, PROMPT);
        let env = panel.paper(2).expect("the environment block");
        assert!(env.bad, "a failure is not marked as one");
        assert!(env.under.contains("no such subcommand"), "{}", env.under);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that is not there shows the shipped default under an honest
    /// note, and saving after enabling edition writes that text as the file:
    /// the built-in becomes owned.
    #[test]
    fn a_missing_file_shows_the_built_in_text_and_saving_owns_it() {
        let config = Config::default();
        let dir = scratch_dir("prompt-default");
        let path = dir.join(agent::AGENTS_MD);
        let mut panel = Settings::open(&config, None, read(&dir));
        go_to(&mut panel, PROMPT);
        let paper = panel.paper(0).expect("the block").clone();
        assert!(paper.under.contains("not written yet"), "{}", paper.under);
        assert!(paper.under.contains(&path.display().to_string()), "{}", paper.under);
        assert_eq!(
            paper.body.join("\n"),
            agent::AGENTS_DEFAULT.trim_end_matches('\n'),
            "the block does not show the text the agent runs with"
        );

        // Edition opens on that same text, and the save writes it, owned.
        assert!(panel.toggle_edition(0, &config));
        assert!(panel.type_instructions("!", &config));
        let deed = panel.finish_instructions().expect("something to save");
        let Deed::SaveInstructions { path: to, text } = &deed else {
            panic!("{deed:?}");
        };
        assert_eq!(to, &path);
        assert!(text.starts_with('!'), "{text}");
        assert!(text.contains("You are noob"), "{text}");
        agent::write_instructions(to, text).expect("the file takes it");
        panel.end_instructions_edit();
        panel.adopt_agent(read(&dir), &config);
        go_to(&mut panel, PROMPT);
        let paper = panel.paper(0).expect("the block");
        assert!(!paper.under.contains("not written yet"), "{}", paper.under);
        assert!(paper.body[0].starts_with('!'), "{:?}", paper.body[0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The checkbox gates the editor: typing does nothing until edition is
    /// enabled, enabling it puts the caret on the document, pressing it again
    /// or Escape drops the buffer with the file untouched.
    #[test]
    fn edition_gates_the_editor_and_dropping_it_keeps_the_file() {
        let config = Config::default();
        let dir = scratch_dir("prompt-gate");
        let path = dir.join(agent::AGENTS_MD);
        std::fs::write(&path, "keep me\n").expect("a file");
        let mut panel = Settings::open(&config, None, read(&dir));
        go_to(&mut panel, PROMPT);

        // Off: the document is read, not typed into.
        assert!(!panel.editing_instructions());
        assert!(!panel.type_instructions("x", &config), "typing landed with edition off");
        assert!(panel.hint().contains("enter enables edition"), "{}", panel.hint());

        // On: the caret is on the document and the block shows the buffer.
        assert!(panel.toggle_edition(0, &config));
        assert!(panel.editing_instructions());
        assert!(panel.edition_on(0));
        assert!(!panel.edition_on(1));
        assert_eq!(panel.instructions_caret(0), Some((0, 0)));
        assert_eq!(panel.instructions_caret(1), None, "a caret on the block not being edited");
        assert!(panel.type_instructions("gone ", &config));
        assert!(panel.hint().contains("ctrl+s writes the file"), "{}", panel.hint());

        // The checkbox again: the buffer goes, the file was never touched.
        assert!(panel.toggle_edition(0, &config));
        assert!(!panel.editing_instructions());
        assert_eq!(panel.paper(0).expect("the block").body, ["keep me"]);

        // Escape does the same.
        assert!(panel.toggle_edition(0, &config));
        assert!(panel.type_instructions("gone ", &config));
        assert!(panel.cancel_instructions(&config));
        assert!(!panel.editing_instructions());
        assert_eq!(
            std::fs::read_to_string(&path).expect("the file"),
            "keep me\n",
            "an abandoned edit reached the file"
        );

        // And enabling edition on one file drops an edit running on the other:
        // one keyboard, one caret.
        assert!(panel.toggle_edition(0, &config));
        assert!(panel.toggle_edition(1, &config));
        assert!(panel.edition_on(1));
        assert!(!panel.edition_on(0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two independent editors: each block saves its own file and only that
    /// file, through the same whole-file deed.
    #[test]
    fn each_block_saves_its_own_file() {
        let config = Config::default();
        let dir = scratch_dir("prompt-two-files");
        let agents = dir.join(agent::AGENTS_MD);
        let tools = dir.join(agent::TOOLS_MD);
        std::fs::write(&agents, "agents text\n").expect("a file");
        std::fs::write(&tools, "tools text\n").expect("a file");
        let mut panel = Settings::open(&config, None, read(&dir));
        go_to(&mut panel, PROMPT);

        // TOOLS.md first: the deed names the tools file, and landing it
        // changes nothing about AGENTS.md.
        assert!(panel.toggle_edition(1, &config));
        assert!(panel.type_instructions("more ", &config));
        let deed = panel.finish_instructions().expect("something to save");
        let Deed::SaveInstructions { path, text } = &deed else {
            panic!("{deed:?}");
        };
        assert_eq!(path, &tools);
        assert_eq!(text, "more tools text");
        agent::write_instructions(path, text).expect("the file takes it");
        panel.end_instructions_edit();
        panel.adopt_agent(read(&dir), &config);
        go_to(&mut panel, PROMPT);
        assert_eq!(
            std::fs::read_to_string(&agents).expect("the file"),
            "agents text\n",
            "saving TOOLS.md touched AGENTS.md"
        );
        assert_eq!(panel.paper(1).expect("the block").body, ["more tools text"]);

        // AGENTS.md the same way round.
        assert!(panel.toggle_edition(0, &config));
        assert!(panel.type_instructions("first ", &config));
        let deed = panel.finish_instructions().expect("something to save");
        let Deed::SaveInstructions { path, text } = &deed else {
            panic!("{deed:?}");
        };
        assert_eq!(path, &agents);
        agent::write_instructions(path, text).expect("the file takes it");
        assert_eq!(
            std::fs::read_to_string(&tools).expect("the file"),
            "more tools text\n",
            "saving AGENTS.md touched TOOLS.md"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The restore is armed, then parks the file in the `.bak` beside it and
    /// writes the shipped default in its place; the block then shows the
    /// default as the file's own text.
    #[test]
    fn restore_arms_then_parks_a_bak_and_writes_the_default() {
        let config = Config::default();
        let dir = scratch_dir("prompt-restore");
        let path = dir.join(agent::TOOLS_MD);
        std::fs::write(&path, "mine\n").expect("a file");
        let mut panel = Settings::open(&config, None, read(&dir));
        go_to(&mut panel, PROMPT);

        // The action stands in the enabled footer, and it takes two presses.
        assert_eq!(panel.restore_prompt(1), None, "restore acted with edition off");
        assert!(panel.toggle_edition(1, &config));
        assert_eq!(panel.restore_prompt(1), None, "one press acted");
        assert!(panel.says().contains(".bak"), "{}", panel.says());
        let deed = panel.restore_prompt(1).expect("the second press acts");
        let Deed::RestorePrompt { path: to, default } = &deed else {
            panic!("{deed:?}");
        };
        assert_eq!(to, &path);
        assert_eq!(*default, agent::TOOLS_DEFAULT);

        // The disk half, as main does it: the bak first, then the default.
        agent::restore_prompt(to, default).expect("the restore lands");
        panel.end_instructions_edit();
        panel.adopt_agent(read(&dir), &config);
        go_to(&mut panel, PROMPT);
        assert_eq!(
            std::fs::read_to_string(dir.join("TOOLS.md.bak")).expect("the bak"),
            "mine\n"
        );
        let paper = panel.paper(1).expect("the block");
        assert!(!paper.under.contains("not written yet"), "{}", paper.under);
        assert_eq!(
            paper.body.join("\n"),
            agent::TOOLS_DEFAULT.trim_end_matches('\n')
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The load action reads a named `.md` into the AGENTS.md editor as an
    /// unsaved edit: nothing lands anywhere until the save, and the save
    /// writes the config directory's file, never the one that was loaded.
    #[test]
    fn loading_an_md_fills_the_editor_and_writes_nothing() {
        let config = Config::default();
        let dir = scratch_dir("prompt-load");
        let mine = dir.join("mine.md");
        std::fs::write(&mine, "# Loaded\n\nfrom disk\n").expect("a file");
        let mut panel = Settings::open(&config, None, read(&dir));
        go_to(&mut panel, PROMPT);

        // Only the AGENTS.md block offers it.
        assert!(!panel.begin_load(1), "TOOLS.md offered a load");
        assert!(panel.begin_load(0));
        assert!(panel.loading());
        assert!(panel.type_text(&mine.display().to_string()));
        assert!(panel.says().contains("load:"), "{}", panel.says());
        assert_eq!(panel.load_path().as_deref(), Some(mine.as_path()));

        // The read, as main does it, and the buffer takes the lines: edition
        // is on, nothing is on the disk that was not there before.
        let body = agent::load_md(&mine).expect("it reads");
        panel.take_loaded(body, &config);
        assert!(!panel.loading());
        assert!(panel.edition_on(0));
        assert_eq!(
            panel.paper(0).expect("the block").body,
            ["# Loaded", "", "from disk"]
        );
        assert!(!dir.join(agent::AGENTS_MD).exists(), "the load wrote a file");

        // The save writes the config directory's AGENTS.md with the loaded
        // text; the loaded file itself is untouched.
        let deed = panel.finish_instructions().expect("something to save");
        let Deed::SaveInstructions { path, text } = &deed else {
            panic!("{deed:?}");
        };
        assert_eq!(path, &dir.join(agent::AGENTS_MD));
        assert_eq!(text, "# Loaded\n\nfrom disk");

        // Escape while typing the path leaves everything alone.
        assert!(panel.begin_load(0));
        assert!(panel.cancel_edit());
        assert!(!panel.loading());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The environment block is read and never edited: no edition, no typing,
    /// and the page keys read it like any other block.
    #[test]
    fn the_environment_block_is_read_only() {
        let config = Config::default();
        let dir = scratch_dir("prompt-env-readonly");
        let mut panel = Settings::open(&config, None, read(&dir));
        panel.adopt_env(
            String::from("/tmp/work"),
            Ok((0..PAPER_LINES * 2).map(|at| format!("env line {at}")).collect()),
            &config,
        );
        go_to(&mut panel, PROMPT);
        assert_eq!(panel.paper(2).expect("the block").does, None, "the block offers edition");
        assert!(panel.point_at(2, crate::settings::Side::Left));
        assert!(!panel.toggle_edition(2, &config), "edition opened on the environment");
        assert!(!panel.type_instructions("x", &config));
        assert!(!panel.begin_load(2), "a load aimed at the environment");
        assert!(panel.hint().contains("page"), "{}", panel.hint());
        assert!(panel.page(20, true));
        assert_eq!(panel.paper(2).expect("the block").first, PAPER_LINES);
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
        let mut panel = Settings::open(&config, None, read(&dir));
        go_to(&mut panel, PROMPT);
        assert!(panel.toggle_edition(0, &config));
        for _ in 0..PAPER_LINES + 2 {
            assert!(panel.instructions_step(true, &config));
        }
        assert_eq!(panel.instructions_caret(0), Some((PAPER_LINES + 2, 0)));
        assert_eq!(
            panel.paper(0).expect("the block").first,
            3,
            "the block did not scroll with the caret"
        );
        for _ in 0..PAPER_LINES + 2 {
            assert!(panel.instructions_step(false, &config));
        }
        assert!(!panel.instructions_step(false, &config), "above the first line");
        assert_eq!(panel.paper(0).expect("the block").first, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A save that cannot land is said on the footer and loses nothing: the
    /// editor stays open with everything typed, and the file is as it was.
    /// A file longer than the CLI reads refuses edition at the way in, because
    /// saving the capped text would quietly lose the tail.
    #[test]
    fn a_failed_write_is_said_and_the_buffer_survives_it() {
        let config = Config::default();
        let dir = scratch_dir("prompt-refused");
        let path = dir.join(agent::AGENTS_MD);
        std::fs::write(&path, "mine\n").expect("a file");
        let mut panel = Settings::open(&config, None, read(&dir));
        go_to(&mut panel, PROMPT);
        assert!(panel.toggle_edition(0, &config));
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

        // Past the CLI's cap edition refuses to open at all, with the reason
        // on the footer: a saved capped buffer would lose the tail.
        std::fs::write(&path, "x".repeat(agent::AGENTS_CAP as usize + 500)).expect("a file");
        let mut panel = Settings::open(&config, None, read(&dir));
        go_to(&mut panel, PROMPT);
        assert!(!panel.toggle_edition(0, &config));
        assert!(!panel.editing_instructions());
        assert!(
            panel.trouble().is_some_and(|why| why.contains("16 KiB")),
            "{:?}",
            panel.trouble()
        );

        // And with no config directory there is nowhere to keep either file,
        // which is said as trouble rather than offered.
        let mut nowhere = Settings::open(&config, None, Agent::default());
        go_to(&mut nowhere, PROMPT);
        let paper = nowhere.paper(0).expect("the block");
        assert!(paper.bad, "no config directory is not marked as trouble");
        assert_eq!(paper.does, None, "it offers edition with nowhere to save");
        assert!(!nowhere.toggle_edition(0, &config));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
