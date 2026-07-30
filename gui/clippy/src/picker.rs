//! Choosing the folder the agent works in.
//!
//! Launched from the dock with no argument, NO0B used to call `current_dir()`,
//! which under a desktop launcher is `$HOME`, and hand the agent the home
//! directory without ever saying so. This module is the model behind the picker
//! that opens instead: what is listed, where the cursor is, walking in and out
//! of folders, and the filter that marks what you typed for.
//!
//! The list is a tree rather than one directory. A folder carries a mark you
//! press to put what is inside it into the list under it, one step further in,
//! and press again to take it away. [`Picker::inside`] holds that tree already
//! flattened, in the order it is read, so opening a folder is an insert after it
//! and shutting one is a drain of everything deeper that follows: no recursion,
//! and one index means the same thing to the drawing, the click and the cursor.
//! Walking in with the keyboard still lists a folder on its own, which is the
//! fast way down a deep tree and the way to start the list again once one has
//! grown long.
//!
//! Typing does not take rows away. Every folder stays in the list and the ones
//! the filter did not match are drawn dim, so the list you were reading is still
//! the list in front of you rather than three rows where forty were. [`Picker::matched`]
//! is the only place that answers whether a row belongs to what was typed, and
//! the drawing, the arrow keys and the click all ask it, so a row cannot be dim
//! on screen and live to the keyboard.
//!
//! Nothing here draws and nothing here needs a window. [`crate::view`] turns the
//! rows into rectangles and [`crate::main`] routes keys and clicks at them, and
//! the listing is read through [`Folders`] rather than straight off the disk, so
//! the whole model can be driven over a tree that was never written.
//!
//! Folders chosen before are remembered in a small file beside the settings, so
//! the second launch is one keystroke. A missing file is a first run, not an
//! error: the window has to open either way.
//!
//! The same list shows the sessions the agent has already written, because the
//! other thing you want on a first open is the conversation you were in an hour
//! ago. [`Picker::show_sessions`] swaps what the rows are made of and nothing
//! else: the same box, the same cursor, the same keys, the same scrolling. What
//! comes back out of [`Picker::confirm`] is a [`Chosen`], a folder and the
//! session to carry on in it, so the one path that starts an agent starts both
//! kinds.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::sessions::{Listing, Saved};

/// How many folders the file remembers. Long enough to hold the projects in
/// rotation, short enough that the list is still one glance.
pub const REMEMBERED: usize = 8;

/// Where the picker reads folders from.
///
/// A trait rather than direct calls to `std::fs`, so the cursor, the walking and
/// the filter can be driven in a test over a tree that only exists in the test.
/// The two questions the picker asks are all that is in it.
pub trait Folders {
    /// The names of the folders directly inside `at`, in any order, or why they
    /// could not be read.
    fn list(&self, at: &Path) -> Result<Vec<String>, String>;
    /// Whether this path is a folder at all. Asked of remembered paths, which
    /// may have been moved or deleted since they were written down.
    fn is_folder(&self, at: &Path) -> bool;
}

/// The real filesystem.
pub struct Disk;

impl Folders for Disk {
    fn list(&self, at: &Path) -> Result<Vec<String>, String> {
        let read = std::fs::read_dir(at).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for entry in read.flatten() {
            // `is_dir` on the path rather than the entry's own file type,
            // because it follows symlinks and a checkout reached through one is
            // still a workspace.
            if !entry.path().is_dir() {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                out.push(name.to_string());
            }
        }
        Ok(out)
    }

    fn is_folder(&self, at: &Path) -> bool {
        at.is_dir()
    }
}

/// The same listing whatever folder is asked for, for the tests in other modules
/// that need a picker and no filesystem.
#[cfg(test)]
pub struct Fixed(pub Vec<String>);

#[cfg(test)]
impl Folders for Fixed {
    fn list(&self, _at: &Path) -> Result<Vec<String>, String> {
        Ok(self.0.clone())
    }

    fn is_folder(&self, _at: &Path) -> bool {
        true
    }
}

/// One row of the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Row {
    /// The folder being listed. This is how a folder you walked into gets
    /// chosen: without it, the only way to pick a folder would be to stand in
    /// its parent, and there would be no way to look inside it first.
    Here,
    /// The folder above it.
    Up,
    /// A folder chosen in an earlier session, by full path.
    Recent(PathBuf),
    /// A folder in the tree under the one being listed.
    Folder {
        /// What the row says: the name alone, however deep it sits.
        name: String,
        /// Where it is, in full. Not the folder being listed joined to the
        /// name any more: a folder two levels into an opened tree is several
        /// names below it.
        path: PathBuf,
        /// How far in it sits. Zero is directly inside the folder being
        /// listed.
        depth: usize,
        /// Whether what is inside it is in the list under it.
        open: bool,
    },
    /// A session the agent has already written, while the list is showing those
    /// instead of folders. Carries the whole of what was read off the file, so
    /// the row can say when it was, where it was and what it was about without
    /// going back to the disk while it is being drawn.
    Session(Saved),
    /// Why a folder that was opened could not be read, on its own row under it.
    /// Without it an unreadable folder opens to nothing, which on screen is a
    /// folder that happens to be empty.
    Locked {
        /// The name of the folder it belongs to, so the message dims with that
        /// folder rather than staying bright among rows the filter passed over.
        of: String,
        depth: usize,
        why: String,
    },
}

impl Row {
    /// How far into the tree the row sits. Everything that is not part of the
    /// tree sits at the top of it.
    pub fn depth(&self) -> usize {
        match self {
            Row::Here | Row::Up | Row::Recent(_) | Row::Session(_) => 0,
            Row::Folder { depth, .. } | Row::Locked { depth, .. } => *depth,
        }
    }

    /// Whether the row carries a mark to open and shut it, and which way that
    /// mark points. Nothing at all for a row that is not a folder in the tree:
    /// the folder being listed, the way out and a folder remembered from an
    /// earlier session are not branches of it.
    pub fn open(&self) -> Option<bool> {
        match self {
            Row::Folder { open, .. } => Some(*open),
            _ => None,
        }
    }

    /// The folder this row is, for the rows that are one.
    fn path(&self) -> Option<&Path> {
        match self {
            Row::Folder { path, .. } => Some(path),
            _ => None,
        }
    }
}

/// The folders inside `at` as rows one step deeper than it, sorted and shut.
///
/// A free function rather than a method: opening a folder reads the disk
/// through [`Folders`] first and builds the rows after, so this must not want
/// the picker borrowed while that is happening.
fn branches(at: &Path, depth: usize, mut names: Vec<String>) -> Vec<Row> {
    names.sort_by_key(|name| name.to_lowercase());
    names
        .into_iter()
        .map(|name| Row::Folder {
            path: at.join(&name),
            name,
            depth,
            open: false,
        })
        .collect()
}

/// What the picker hands back when a row is confirmed: the folder to start the
/// agent in, and the session to carry on there when the row was one.
///
/// One type for both, so there is one path from a chosen row to a running
/// agent. A second way in for sessions would be a second place for the folder
/// to be decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chosen {
    pub workspace: PathBuf,
    pub session: Option<String>,
}

impl Chosen {
    /// A fresh session in this folder, which is what every row that names a
    /// folder means.
    pub fn folder(workspace: PathBuf) -> Chosen {
        Chosen {
            workspace,
            session: None,
        }
    }
}

pub struct Picker {
    folders: Box<dyn Folders>,
    /// The folder being listed.
    at: PathBuf,
    /// Where the picker opened. Remembered folders are offered there and
    /// nowhere else: once you have walked somewhere, the list is that folder's
    /// contents, and a row pointing at another branch of the tree would read as
    /// one of them.
    start: PathBuf,
    recents: Vec<PathBuf>,
    /// The tree under `at`, flattened into the order it is read, before the
    /// filter. Only [`Row::Folder`] and [`Row::Locked`] live here: the folder
    /// being listed and the way out of it are put in front of it when the rows
    /// are built, because they are how the list is walked rather than part of
    /// what it holds.
    inside: Vec<Row>,
    /// Why `at` could not be read, when it could not. Worth saying out loud: an
    /// empty list looks exactly like an empty folder.
    trouble: Option<String>,
    /// The saved sessions, while the list is showing those instead of the tree.
    /// `None` is the folder list, which is what the picker opens on.
    sessions: Option<Vec<Saved>>,
    /// What time it was when they were read, so a row's age is decided once
    /// rather than by a clock read while the window is drawing.
    sessions_at: SystemTime,
    /// What the session list says about itself: how many there are and how many
    /// files could not be described.
    note: Option<String>,
    /// Why the last press did nothing, when it did nothing. A session whose
    /// folder has been deleted cannot be resumed anywhere, and a button that
    /// silently does not work is the worst answer to that.
    refused: Option<String>,
    filter: String,
    /// The rows as they are now. Built once whenever anything they are made of
    /// changes, so what is drawn, what a click resolves against and what the
    /// cursor indexes cannot disagree.
    rows: Vec<Row>,
    cursor: usize,
    /// The top row on screen. Top anchored, like the file explorer's list.
    first: usize,
}

impl Picker {
    /// Open on `start`, offering `recents` above what is inside it.
    pub fn open(folders: Box<dyn Folders>, start: PathBuf, recents: Vec<PathBuf>) -> Picker {
        // A remembered folder that has been moved or deleted would start the
        // agent in a directory that is not there, so it is dropped on the way
        // in rather than failing at the moment it is picked.
        let recents: Vec<PathBuf> = recents
            .into_iter()
            .filter(|path| folders.is_folder(path))
            .collect();
        let mut picker = Picker {
            folders,
            at: start.clone(),
            start,
            recents,
            inside: Vec::new(),
            trouble: None,
            sessions: None,
            sessions_at: SystemTime::UNIX_EPOCH,
            note: None,
            refused: None,
            filter: String::new(),
            rows: Vec::new(),
            cursor: 0,
            first: 0,
        };
        picker.relist();
        // The first row, which is the folder used last when there is one. That
        // is the whole point of remembering them: the second launch is Enter.
        picker.cursor = 0;
        picker
    }

    pub fn at(&self) -> &Path {
        &self.at
    }

    /// Every row, for a test that wants the whole list at once.
    ///
    /// Test-only, and it was not always: the picker's box used to be sized from
    /// this count, which is what made the dialog change shape every time you
    /// walked into a folder with a different number of entries. Nothing that
    /// draws asks how many rows there are any more. [`Picker::row`] answers for
    /// the row a layout actually placed.
    #[cfg(test)]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn row(&self, index: usize) -> Option<&Row> {
        self.rows.get(index)
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn first(&self) -> usize {
        self.first
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn trouble(&self) -> Option<&str> {
        self.trouble.as_deref()
    }

    /// Whether the list is showing saved sessions rather than folders.
    pub fn on_sessions(&self) -> bool {
        self.sessions.is_some()
    }

    /// What the session list says about itself, for the line above it.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Why the last press did nothing, for the same line the filter is on.
    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    /// Show the saved sessions instead of the folders.
    ///
    /// The ones belonging to the folder being looked at come first, since that
    /// is the one you are most likely to be carrying on with; the rest keep the
    /// order they were read in, which is newest first.
    pub fn show_sessions(&mut self, listing: Listing) {
        self.show_sessions_at(listing, SystemTime::now());
    }

    /// The half of it that knows what time it is, so a test can say.
    pub fn show_sessions_at(&mut self, listing: Listing, now: SystemTime) {
        let Listing {
            mut sessions,
            skipped,
        } = listing;
        let here = self.at.clone();
        // Stable, so within each group the reading order (newest first) holds.
        sessions.sort_by_key(|saved| saved.workspace.as_deref() != Some(here.as_path()));
        self.note = Some(note_for(sessions.len(), skipped.len()));
        self.refused = None;
        self.sessions = Some(sessions);
        self.sessions_at = now;
        // Typed against the folders, and it would leave most of the sessions
        // dim for no reason anybody typed.
        self.filter.clear();
        self.rebuild();
        self.cursor = 0;
        self.first = 0;
    }

    /// Back to the folders. False when they were already showing, which is what
    /// makes Escape fall through to closing the window.
    pub fn show_folders(&mut self) -> bool {
        if self.sessions.take().is_none() {
            return false;
        }
        self.note = None;
        self.refused = None;
        self.filter.clear();
        self.rebuild();
        self.cursor = self
            .rows
            .iter()
            .position(|row| *row == Row::Here)
            .unwrap_or(0);
        self.first = 0;
        true
    }

    /// What a row says on screen. The full path for anything naming a folder
    /// somewhere else, a bare name for a folder in the list.
    ///
    /// The folder being listed says "this folder" rather than repeating its own
    /// path: the path is already written above the list, and the same string
    /// twice in two rows reads as two folders.
    pub fn label(&self, row: &Row) -> String {
        match row {
            Row::Here => String::from("this folder"),
            Row::Up => String::from(".."),
            Row::Recent(path) => path.display().to_string(),
            Row::Folder { name, .. } => name.clone(),
            Row::Locked { why, .. } => why.clone(),
            Row::Session(saved) => self.session_label(saved),
        }
    }

    /// What a session row says: when it was, which folder it belongs to, and
    /// the opening of the first thing that was said in it.
    ///
    /// In that order because that is the order they narrow by. The age and the
    /// folder are short and fixed width-ish, so the message that identifies the
    /// session starts in about the same column on every row; a long path first
    /// would push it off the end of every row that has one.
    fn session_label(&self, saved: &Saved) -> String {
        let folder = match (&saved.workspace, saved.gone) {
            (Some(path), true) => format!("{} (gone)", short_folder(path)),
            (Some(path), false) => short_folder(path),
            // Written by the CLI rather than by this window, so nothing here
            // ever noted where it was, and the folder above the list is the one
            // it would be resumed in.
            (None, _) => String::from("this folder"),
        };
        let said = match saved.opening.is_empty() {
            true => "nothing was said",
            false => saved.opening.as_str(),
        };
        format!(
            "{}  {folder}  {said}",
            crate::sessions::ago(saved.when, self.sessions_at)
        )
    }

    /// The folder a row names, or nothing when there is none (the root has no
    /// parent, so it has no way out, and a row saying why a folder could not be
    /// read is a message rather than somewhere to go).
    fn path_of(&self, row: &Row) -> Option<PathBuf> {
        match row {
            Row::Here => Some(self.at.clone()),
            Row::Up => self.at.parent().map(Path::to_path_buf),
            Row::Recent(path) => Some(path.clone()),
            Row::Folder { path, .. } => Some(path.clone()),
            Row::Locked { .. } => None,
            // Whatever was noted when it was started, which for a session the
            // CLI wrote on its own is nothing at all.
            Row::Session(saved) => saved.workspace.clone(),
        }
    }

    /// Whether a row belongs to what has been typed.
    ///
    /// The one rule. What is drawn bright, what the arrow keys stop on and what
    /// a screenful lands on all come through here, so a row cannot read as dim
    /// and behave as a match, or the other way round.
    ///
    /// The folder being listed and the way out of it are always a match. They
    /// are how the list is walked rather than entries in it, and a filter that
    /// took the keyboard's way out of the folder away would leave "wor" with no
    /// route back to anywhere. Everything else is matched on its label: a
    /// remembered folder on its whole path, a folder in the list on its name,
    /// and the message under an unreadable folder on that folder's name, so the
    /// two dim together.
    pub fn matched(&self, row: &Row) -> bool {
        match row {
            Row::Here | Row::Up => true,
            Row::Recent(path) => self.hit(&path.display().to_string()),
            Row::Folder { name, .. } => self.hit(name),
            Row::Locked { of, .. } => self.hit(of),
            // On the whole of what the row says, so a session is reachable by
            // its folder or by a word from the first thing that was asked.
            Row::Session(saved) => self.hit(&self.session_label(saved)),
        }
    }

    /// Move the cursor one row, onto the next row the filter matched.
    ///
    /// The arrows walk the matches. With nothing typed every row is a match and
    /// this is a plain step; with something typed the dim rows are passed over,
    /// because stepping through forty folders to reach the one you named is what
    /// the typing was meant to save.
    pub fn step(&mut self, down: bool) -> bool {
        let next = match down {
            true => self.match_from(self.cursor + 1, true),
            false => self
                .cursor
                .checked_sub(1)
                .and_then(|from| self.match_from(from, false)),
        };
        let Some(next) = next else {
            return false;
        };
        let moved = next != self.cursor;
        self.cursor = next;
        moved
    }

    /// Move it by a screenful, landing on a match.
    ///
    /// A screenful of dim rows is still a screenful, so the jump is by rows and
    /// the landing is the nearest match on from there. Past the end of the
    /// matches in the direction of travel it takes the last one, which is what
    /// makes PageDown at the bottom stop rather than do nothing.
    pub fn page(&mut self, rows: usize, down: bool) -> bool {
        let by = rows.max(1);
        let last = self.rows.len().saturating_sub(1);
        let target = match down {
            true => (self.cursor + by).min(last),
            false => self.cursor.saturating_sub(by),
        };
        let next = self
            .match_from(target, down)
            .or_else(|| self.match_from(target, !down))
            .unwrap_or(self.cursor);
        let moved = next != self.cursor;
        self.cursor = next;
        moved
    }

    /// To the first or the last row the filter matched.
    pub fn jump(&mut self, last: bool) -> bool {
        let next = match last {
            true => self.match_from(self.rows.len().saturating_sub(1), false),
            false => self.match_from(0, true),
        }
        .unwrap_or(self.cursor);
        let moved = next != self.cursor;
        self.cursor = next;
        moved
    }

    /// The nearest row at or after `from`, or at or before it going up, that the
    /// filter matched. Nothing when there is none that way.
    fn match_from(&self, from: usize, down: bool) -> Option<usize> {
        let last = self.rows.len().checked_sub(1)?;
        let mut at = from.min(last);
        loop {
            if self.matched(&self.rows[at]) {
                return Some(at);
            }
            match down {
                true if at < last => at += 1,
                false if at > 0 => at -= 1,
                _ => return None,
            }
        }
    }

    /// Put the cursor on the row the pointer is on. A click that lands on a row
    /// that is no longer there is ignored rather than clamped: the list moved
    /// under the pointer, so the nearest row is not what was aimed at.
    ///
    /// A dim row takes the click. It is a real folder that happens not to be
    /// what was typed, and the pointer is aimed at one row rather than walking
    /// through them, so there is nothing to skip over.
    pub fn point_at(&mut self, index: usize) -> bool {
        if index >= self.rows.len() || index == self.cursor {
            return false;
        }
        self.cursor = index;
        true
    }

    /// Open or shut the folder on the row at `index`, in place.
    ///
    /// What the mark in front of a folder does, and the whole of the tree: the
    /// rows underneath change and the folder being listed does not. Walking in
    /// is the other gesture, and it lists that folder on its own instead.
    ///
    /// The cursor is put back on the row it was on however many rows appeared or
    /// disappeared above it. Shutting a folder that held the cursor somewhere
    /// inside it leaves it on that folder: the row it was on is not on screen
    /// any more, and a cursor left at the same index would be pointing at a
    /// folder nobody chose.
    pub fn toggle(&mut self, index: usize) -> bool {
        let Some(row) = self.rows.get(index).cloned() else {
            return false;
        };
        let (Some(path), Some(open)) = (row.path().map(Path::to_path_buf), row.open()) else {
            return false;
        };
        let Some(at) = self.inside.iter().position(|row| row.path() == Some(&path)) else {
            return false;
        };
        let held = self.rows.get(self.cursor).cloned();
        match open {
            true => self.shut(at),
            false => self.opened(at),
        }
        self.rebuild();
        // The same row if it is still on screen, and the folder that was just
        // shut if it is not, which is where its children went.
        self.cursor = held
            .and_then(|row| self.rows.iter().position(|other| *other == row))
            .or_else(|| {
                self.rows
                    .iter()
                    .position(|other| other.path() == Some(&path))
            })
            .unwrap_or(0)
            .min(self.rows.len().saturating_sub(1));
        true
    }

    /// Put what is inside the folder at `at` into the tree under it.
    ///
    /// Through [`Folders`] like every other listing, so a folder that cannot be
    /// read says so on a row of its own rather than opening to nothing.
    fn opened(&mut self, at: usize) {
        let Some(Row::Folder { path, depth, .. }) = self.inside.get(at).cloned() else {
            return;
        };
        let kids = match self.folders.list(&path) {
            Ok(names) => branches(&path, depth + 1, names),
            Err(why) => vec![Row::Locked {
                of: path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string(),
                depth: depth + 1,
                why,
            }],
        };
        if let Some(Row::Folder { open, .. }) = self.inside.get_mut(at) {
            *open = true;
        }
        self.inside.splice(at + 1..at + 1, kids);
    }

    /// Take the whole of the folder at `at` back out of the tree.
    ///
    /// Everything deeper that follows it, which in a flattened tree is exactly
    /// its descendants and nothing else: the next row at its own depth or
    /// shallower is the end of it.
    fn shut(&mut self, at: usize) {
        let Some(depth) = self.inside.get(at).map(Row::depth) else {
            return;
        };
        if let Some(Row::Folder { open, .. }) = self.inside.get_mut(at) {
            *open = false;
        }
        let end = self.inside[at + 1..]
            .iter()
            .position(|row| row.depth() <= depth)
            .map(|step| at + 1 + step)
            .unwrap_or(self.inside.len());
        self.inside.drain(at + 1..end);
    }

    /// List what is inside the cursor's row.
    pub fn walk_in(&mut self) -> bool {
        let Some(row) = self.rows.get(self.cursor).cloned() else {
            return false;
        };
        match row {
            // Already the folder being listed.
            Row::Here => false,
            Row::Up => self.walk_out(),
            Row::Recent(path) => self.go_to(path),
            Row::Folder { path, .. } => self.go_to(path),
            // A message, not a folder.
            Row::Locked { .. } => false,
            // There is nothing to walk into: a session is a whole answer
            // already, and walking would reorder the list under the cursor.
            Row::Session(_) => false,
        }
    }

    /// List the folder above the one being listed.
    ///
    /// Nothing at all while the sessions are showing. The list is not the
    /// folder's, so walking would change the order under the cursor and leave
    /// the same rows on screen.
    pub fn walk_out(&mut self) -> bool {
        if self.on_sessions() {
            return false;
        }
        let Some(up) = self.at.parent().map(Path::to_path_buf) else {
            return false;
        };
        self.go_to(up)
    }

    /// Confirm the cursor's row: what to start the agent with, or nothing.
    ///
    /// Nothing means the row was a way through the tree rather than a choice, in
    /// which case it has been walked. `..` reads as "go up" on screen and has to
    /// do that when it is confirmed; returning the parent as the workspace would
    /// open a folder nobody pointed at.
    pub fn confirm(&mut self) -> Option<Chosen> {
        let row = self.rows.get(self.cursor).cloned()?;
        if row == Row::Up {
            self.walk_out();
            return None;
        }
        self.chosen(&row)
    }

    /// What a row is worth as a choice.
    ///
    /// A session is a folder and an id together. One whose folder has been
    /// deleted since is not a choice at all: `noob serve` is started in that
    /// directory, and there is no honest guess at another one, so the press is
    /// refused and the row says why rather than starting an agent somewhere
    /// nobody named. A session written by the CLI, which this window has no note
    /// of, resumes in the folder being looked at: that is the folder the row was
    /// picked from, and the alternative is a session that can never be opened.
    fn chosen(&mut self, row: &Row) -> Option<Chosen> {
        let Row::Session(saved) = row else {
            return self.path_of(row).map(Chosen::folder);
        };
        if saved.gone {
            let gone = saved
                .workspace
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            self.refused = Some(format!("{gone} is not there any more"));
            return None;
        }
        self.refused = None;
        Some(Chosen {
            workspace: saved.workspace.clone().unwrap_or_else(|| self.at.clone()),
            session: Some(saved.id.clone()),
        })
    }

    /// What a double click on a row does.
    ///
    /// A row naming a folder that was chosen before, or the folder being listed,
    /// opens: those are whole answers already. A row that is a step through the
    /// tree is walked into, which is what a double click does in every file
    /// manager.
    pub fn double(&mut self, index: usize) -> Option<Chosen> {
        // The row that was clicked, not the row the cursor is on: a click on a
        // row that is no longer there must not open whatever happens to be
        // under the cursor instead.
        let row = self.rows.get(index).cloned()?;
        self.point_at(index);
        match row {
            Row::Here | Row::Recent(_) | Row::Session(_) => self.chosen(&row),
            Row::Up | Row::Folder { .. } => {
                self.walk_in();
                None
            }
            Row::Locked { .. } => None,
        }
    }

    /// Add to what has been typed. The list keeps every row it had; the ones
    /// that do not match go dim.
    pub fn type_text(&mut self, text: &str) -> bool {
        let typed: String = text.chars().filter(|c| !c.is_control()).collect();
        if typed.is_empty() {
            return false;
        }
        self.filter.push_str(&typed);
        self.refilter();
        true
    }

    /// Take back the last character. With nothing typed this is the way out of
    /// the folder, so Backspace walks up the tree the way it does in a file
    /// manager rather than doing nothing at all.
    pub fn backspace(&mut self) -> bool {
        if self.filter.pop().is_some() {
            self.refilter();
            return true;
        }
        self.walk_out()
    }

    pub fn clear_filter(&mut self) -> bool {
        if self.filter.is_empty() {
            return false;
        }
        self.filter.clear();
        self.refilter();
        true
    }

    /// One row per entry, as heights, for the scroll window. The list clips a
    /// label that does not fit rather than wrapping it, so a row is always one
    /// row and a click cannot resolve to a folder other than the one under the
    /// pointer.
    pub fn heights(&self) -> Vec<usize> {
        text_geometry::heights(self.rows.iter().map(|_| 0), 1)
    }

    /// Bring the cursor on screen, for a `rows` tall list.
    pub fn reveal(&mut self, rows: usize) -> bool {
        if rows == 0 || self.rows.is_empty() {
            return false;
        }
        let most = text_geometry::max_scrollback(&self.heights(), rows);
        let mut next = self.first.min(self.cursor);
        if self.cursor + 1 > next + rows {
            next = self.cursor + 1 - rows;
        }
        let next = next.min(most);
        let moved = next != self.first;
        self.first = next;
        moved
    }

    /// Move the window without moving the cursor, for the wheel.
    pub fn scroll(&mut self, by: usize, down: bool, rows: usize) -> bool {
        let most = text_geometry::max_scrollback(&self.heights(), rows);
        let next = match down {
            true => (self.first + by).min(most),
            false => self.first.saturating_sub(by),
        };
        let moved = next != self.first;
        self.first = next;
        moved
    }

    /// How much of the list is on screen, for the scrollbar.
    pub fn thumb(&self, rows: usize) -> Option<(f32, f32)> {
        let heights = self.heights();
        let back = text_geometry::scrollback_for(&heights, rows, self.first);
        text_geometry::thumb(&heights, rows, back)
    }

    /// List a different folder. Clears the filter, because it was typed against
    /// the folder being left and would silently hide most of the new one.
    fn go_to(&mut self, at: PathBuf) -> bool {
        if at == self.at {
            return false;
        }
        self.at = at;
        self.filter.clear();
        self.relist();
        true
    }

    /// Read the folder being listed and rebuild the rows.
    ///
    /// The cursor lands on [`Row::Here`]: you have just walked into this folder,
    /// so choosing it is the likely next keystroke, and the row above and below
    /// are one step away either way.
    fn relist(&mut self) {
        let at = self.at.clone();
        match self.folders.list(&at) {
            Ok(names) => {
                self.inside = branches(&at, 0, names);
                self.trouble = None;
            }
            Err(why) => {
                self.inside = Vec::new();
                self.trouble = Some(why);
            }
        }
        self.rebuild();
        self.cursor = self
            .rows
            .iter()
            .position(|row| *row == Row::Here)
            .unwrap_or(0);
        self.first = 0;
    }

    /// Put the cursor on something the filter matched, for a filter that changed.
    ///
    /// The rows themselves do not move: typing dims, it does not drop, so this
    /// only has to find the cursor a home. Not [`Row::Here`], which is always a
    /// match: after typing a name, Enter has to open the folder that was typed,
    /// not the folder being listed. With nothing matched it falls back to the
    /// first row, which is a way through the tree rather than a wrong choice.
    fn refilter(&mut self) {
        self.rebuild();
        self.cursor = self
            .rows
            .iter()
            .position(|row| {
                matches!(
                    row,
                    Row::Recent(_) | Row::Folder { .. } | Row::Session(_)
                ) && self.matched(row)
            })
            .or_else(|| self.match_from(0, true))
            .unwrap_or(0);
        self.first = 0;
    }

    fn rebuild(&mut self) {
        // The sessions are the whole list while they are showing: there is no
        // folder to stand in and no way out of one, because the rows are not a
        // place in the tree.
        if let Some(saved) = &self.sessions {
            self.rows = saved.iter().cloned().map(Row::Session).collect();
            self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
            return;
        }
        let mut rows = Vec::new();
        if self.at == self.start {
            rows.extend(
                self.recents
                    .iter()
                    .filter(|path| self.listed(&path.display().to_string()))
                    .cloned()
                    .map(Row::Recent),
            );
        }
        // The folder you are in and the way out of it, always. They are how the
        // list is navigated, and a way out that disappears leaves the keyboard
        // with nowhere to go.
        rows.push(Row::Here);
        if self.at.parent().is_some() {
            rows.push(Row::Up);
        }
        // The tree, with a folder that is not listed taking everything it holds
        // out with it: a child left behind when its parent has gone is a row
        // indented under nothing.
        let mut buried: Option<usize> = None;
        for row in &self.inside {
            if let Some(depth) = buried {
                if row.depth() > depth {
                    continue;
                }
                buried = None;
            }
            if let Row::Folder { name, depth, .. } = row
                && !self.listed(name)
            {
                buried = Some(*depth);
                continue;
            }
            rows.push(row.clone());
        }
        self.rows = rows;
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    /// Whether a folder is in the list at all.
    ///
    /// This is not the search. The only thing kept out of the list is a folder
    /// whose name starts with a dot, until the filter starts with one, which is
    /// the rule `ls` taught everybody. What has been typed past that dims rows,
    /// it does not remove them: see [`Picker::matched`].
    fn listed(&self, label: &str) -> bool {
        let name = label.rsplit('/').next().unwrap_or(label);
        !name.starts_with('.') || self.filter.starts_with('.')
    }

    /// Whether a label carries what has been typed.
    ///
    /// Case insensitive and anywhere in the label rather than only at the front:
    /// a project is remembered by a word in the middle of its name as often as
    /// by the start of it.
    fn hit(&self, label: &str) -> bool {
        if self.filter.is_empty() {
            return true;
        }
        label.to_lowercase().contains(&self.filter.to_lowercase())
    }
}

/// A folder as a session row says it: its own name, which is what tells two
/// projects apart, and the whole path when it has no name (the root).
fn short_folder(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .unwrap_or_else(|| path.display().to_string())
}

/// What the line above the session list says: how many there are, and how many
/// files in the directory could not be described.
///
/// The skipped ones are counted out loud. A session that quietly did not appear
/// reads as a session that was never saved, and these files are written by a
/// process that can be killed between the write and the newline.
fn note_for(listed: usize, skipped: usize) -> String {
    let sessions = match listed {
        0 => String::from("no saved sessions"),
        1 => String::from("1 saved session"),
        _ => format!("{listed} saved sessions"),
    };
    match skipped {
        0 => sessions,
        1 => format!("{sessions}, 1 file could not be read"),
        _ => format!("{sessions}, {skipped} files could not be read"),
    }
}

/// Where the remembered folders live: beside `no0b.conf`, under the same rules.
/// The window has written nothing but its settings until now, so this is the
/// second file and it goes in the same place.
pub fn recents_path() -> Option<PathBuf> {
    Some(crate::config::path()?.with_file_name("no0b.recent"))
}

/// Read the file. A missing or unreadable one is a first run.
///
/// The list written under the old name is taken over on the way in, so a rename
/// does not read as somebody who has never opened a folder.
pub fn load_recents(path: &Path) -> Vec<PathBuf> {
    crate::config::adopt_legacy(path);
    match std::fs::read_to_string(path) {
        Ok(text) => parse_recents(&text),
        Err(_) => Vec::new(),
    }
}

/// One path per line, newest first, `#` a comment. Not `key = value` like the
/// settings and the totals: this is a list, and numbering the lines would only
/// give the file a way to disagree with its own order.
fn parse_recents(text: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let path = PathBuf::from(line);
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out.truncate(REMEMBERED);
    out
}

fn recents_text(list: &[PathBuf]) -> String {
    let mut out = String::from(
        "# Folders NO0B has opened, newest first. Delete a line to forget it.\n",
    );
    for path in list.iter().take(REMEMBERED) {
        out.push_str(&path.display().to_string());
        out.push('\n');
    }
    out
}

/// The list with `chosen` at the front: newest first, no duplicates, capped.
///
/// Pure, and it takes the list rather than reading the file, so the caller can
/// re-read it immediately before writing and two windows closing at once cannot
/// erase each other's history.
pub fn remember(list: &[PathBuf], chosen: &Path) -> Vec<PathBuf> {
    let mut out = vec![chosen.to_path_buf()];
    out.extend(list.iter().filter(|path| path.as_path() != chosen).cloned());
    out.truncate(REMEMBERED);
    out
}

/// Replace the file, by rename, through the same writer the settings and the
/// totals use.
pub fn save_recents(path: &Path, list: &[PathBuf]) -> Result<(), String> {
    crate::config::replace_file(path, &recents_text(list))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A tree that was never written. The key is a folder, the value the folders
    /// inside it; a key that is absent is a folder that cannot be read, which is
    /// how the no-permission path is driven.
    struct Fake {
        tree: BTreeMap<String, Vec<String>>,
    }

    impl Fake {
        fn new(entries: &[(&str, &[&str])]) -> Box<Fake> {
            Box::new(Fake {
                tree: entries
                    .iter()
                    .map(|(at, inside)| {
                        (
                            at.to_string(),
                            inside.iter().map(|s| s.to_string()).collect(),
                        )
                    })
                    .collect(),
            })
        }
    }

    impl Folders for Fake {
        fn list(&self, at: &Path) -> Result<Vec<String>, String> {
            match self.tree.get(&at.display().to_string()) {
                Some(inside) => Ok(inside.clone()),
                None => Err(String::from("permission denied")),
            }
        }

        fn is_folder(&self, at: &Path) -> bool {
            self.tree.contains_key(&at.display().to_string())
        }
    }

    fn home() -> Box<Fake> {
        Fake::new(&[
            (
                "/home/hec",
                &["workspace", "Pictures", ".cache", "models", "Desktop"],
            ),
            ("/home/hec/workspace", &["noob-cli", "anna"]),
            ("/home/hec/workspace/noob-cli", &["gui", "crates"]),
            ("/home/hec/models", &[]),
            ("/home/hec/.cache", &["thumbs"]),
            ("/home", &["hec"]),
            ("/", &["home", "tmp"]),
        ])
    }

    fn opened(recents: &[&str]) -> Picker {
        Picker::open(
            home(),
            PathBuf::from("/home/hec"),
            recents.iter().map(PathBuf::from).collect(),
        )
    }

    /// Whether the row at `index` is one the filter dims.
    fn dimmed(picker: &Picker, index: usize) -> bool {
        picker.row(index).is_some_and(|row| !picker.matched(row))
    }

    fn labels(picker: &Picker) -> Vec<String> {
        picker
            .rows()
            .iter()
            .map(|row| picker.label(row))
            .collect()
    }

    /// What the cursor is on, by what its row says. A folder in the list says
    /// its own name, one remembered from an earlier session says its whole
    /// path, and the two ways through the tree say "this folder" and "..", so
    /// the label answers which row it is as well as which folder.
    fn on(picker: &Picker) -> String {
        picker
            .row(picker.cursor())
            .map(|row| picker.label(row))
            .unwrap_or_default()
    }

    /// The row that says this, by position. How a test points at a folder
    /// without spelling out the whole row.
    fn at(picker: &Picker, label: &str) -> usize {
        labels(picker)
            .iter()
            .position(|said| said == label)
            .unwrap_or_else(|| panic!("no row says {label:?}: {:?}", labels(picker)))
    }

    /// The list as it reads on screen: two spaces per step into the tree, then
    /// the mark the row carries (`+` to open it, `-` to shut it, nothing for a
    /// row that is not a folder), then what it says.
    fn tree(picker: &Picker) -> Vec<String> {
        picker
            .rows()
            .iter()
            .map(|row| {
                let mark = match row.open() {
                    Some(true) => '-',
                    Some(false) => '+',
                    None => ' ',
                };
                format!("{}{mark} {}", "  ".repeat(row.depth()), picker.label(row))
            })
            .collect()
    }

    /// The folders inside the starting point, sorted, with the folder itself and
    /// the way out above them. Nothing that is not a folder, and nothing hidden.
    #[test]
    fn the_picker_lists_the_folders_inside_where_it_opened() {
        let picker = opened(&[]);
        assert_eq!(picker.at(), Path::new("/home/hec"));
        assert_eq!(
            labels(&picker),
            vec![
                "this folder",
                "..",
                "Desktop",
                "models",
                "Pictures",
                "workspace"
            ],
            "sorted case insensitively, with .cache out of the way"
        );
        assert_eq!(
            picker.row(0),
            Some(&Row::Here),
            "the folder being listed is a row, or a folder walked into could not be chosen"
        );
        assert_eq!(picker.path_of(&Row::Up), Some(PathBuf::from("/home")));
        assert_eq!(
            picker.path_of(picker.row(at(&picker, "workspace")).unwrap()),
            Some(PathBuf::from("/home/hec/workspace"))
        );
        // With nothing remembered, the cursor is on the folder being listed, so
        // there is no row a blind Enter opens that was never pointed at.
        assert_eq!(picker.cursor(), 0);
        // Nothing typed, so every row is a match and nothing is drawn dim.
        assert!(picker.rows().iter().all(|row| picker.matched(row)));
    }

    /// The whole reason the file exists: the folder used last is the first row,
    /// and Enter is the only keystroke it takes.
    #[test]
    fn the_folder_used_last_is_one_keystroke_away() {
        let mut picker = opened(&["/home/hec/workspace/noob-cli", "/home/hec/models"]);
        assert_eq!(
            picker.rows()[..2],
            [
                Row::Recent(PathBuf::from("/home/hec/workspace/noob-cli")),
                Row::Recent(PathBuf::from("/home/hec/models")),
            ]
        );
        assert_eq!(picker.cursor(), 0);
        assert_eq!(
            picker.confirm(),
            Some(Chosen::folder(PathBuf::from("/home/hec/workspace/noob-cli")))
        );

        // A folder that has been moved or deleted since is dropped, rather than
        // starting the agent in a directory that is not there.
        let picker = opened(&["/home/hec/gone", "/home/hec/models"]);
        assert_eq!(
            picker.rows()[0],
            Row::Recent(PathBuf::from("/home/hec/models"))
        );

        // And they are offered where the picker opened, not everywhere: walked
        // somewhere else, the list is that folder's own contents.
        let mut picker = opened(&["/home/hec/models"]);
        picker.point_at(at(&picker, "workspace"));
        assert!(picker.walk_in());
        assert!(
            !picker
                .rows()
                .iter()
                .any(|row| matches!(row, Row::Recent(_))),
            "{:?}",
            labels(&picker)
        );
    }

    /// Walking in lists what is there, walking out comes back, and the root has
    /// no way out because there is nowhere above it.
    #[test]
    fn walking_in_and_out_lists_what_is_there() {
        let mut picker = opened(&[]);
        let workspace = at(&picker, "workspace");
        picker.point_at(workspace);
        assert!(picker.walk_in());
        assert_eq!(picker.at(), Path::new("/home/hec/workspace"));
        assert_eq!(labels(&picker), vec!["this folder", "..", "anna", "noob-cli"]);
        assert_eq!(
            picker.row(picker.cursor()),
            Some(&Row::Here),
            "the folder just walked into is what the cursor lands on"
        );

        assert!(picker.walk_out());
        assert_eq!(picker.at(), Path::new("/home/hec"));
        // Up to the root, where there is no parent and so no way out.
        for _ in 0..3 {
            picker.walk_out();
        }
        assert_eq!(picker.at(), Path::new("/"));
        assert!(!picker.rows().contains(&Row::Up));
        assert!(!picker.walk_out(), "and it stops there");
        assert_eq!(picker.path_of(&Row::Up), None);

        // Walking in on the folder being listed is a no-op: it is already the
        // folder being listed.
        assert_eq!(picker.row(picker.cursor()), Some(&Row::Here));
        assert!(!picker.walk_in());
    }

    /// Typing dims what it did not match rather than taking it away, and the
    /// cursor lands on something that was typed for rather than on the folder
    /// that is always there.
    #[test]
    fn the_filter_dims_what_it_did_not_match_and_keeps_every_row() {
        let mut picker = opened(&["/home/hec/models"]);
        let before = labels(&picker);
        assert_eq!(
            before,
            vec![
                "/home/hec/models",
                "this folder",
                "..",
                "Desktop",
                "models",
                "Pictures",
                "workspace"
            ]
        );

        assert!(picker.type_text("wor"));
        assert_eq!(
            labels(&picker),
            before,
            "every row is still there, matched or not"
        );
        let dim: Vec<String> = picker
            .rows()
            .iter()
            .filter(|row| !picker.matched(row))
            .map(|row| picker.label(row))
            .collect();
        assert_eq!(
            dim,
            vec!["/home/hec/models", "Desktop", "models", "Pictures"],
            "the folder being listed and the way out of it are never dim"
        );
        assert_eq!(
            on(&picker),
            "workspace",
            "Enter after typing has to open what was typed"
        );

        // The arrows walk the matches: from `workspace` the row above is the way
        // out, four dim rows further up, and back down again is `workspace`.
        assert!(picker.step(false));
        assert_eq!(picker.row(picker.cursor()), Some(&Row::Up));
        assert!(picker.step(true));
        assert_eq!(on(&picker), "workspace");
        assert!(!picker.step(true), "and there is no match below it");
        assert!(picker.jump(false));
        assert_eq!(
            picker.row(picker.cursor()),
            Some(&Row::Here),
            "the top of the list is the first match, not the first row"
        );
        assert!(picker.jump(true));
        assert_eq!(on(&picker), "workspace");
        // A screenful lands on a match too, rather than on whatever row is that
        // many down.
        assert!(picker.page(3, false));
        assert!(!dimmed(&picker, picker.cursor()));

        // A dim row is a real folder: the pointer may land on it, and Enter then
        // opens it. Only the arrows skip it.
        let desktop = before.iter().position(|l| l == "Desktop").unwrap();
        assert!(dimmed(&picker, desktop));
        assert!(picker.point_at(desktop));
        assert_eq!(
            picker.confirm(),
            Some(Chosen::folder(PathBuf::from("/home/hec/Desktop")))
        );

        // Case does not matter, and it matches anywhere in the name.
        let mut picker = opened(&["/home/hec/models"]);
        assert!(picker.type_text("KSP"));
        assert_eq!(on(&picker), "workspace");

        // A remembered folder is matched on its whole path, so a project is
        // reachable by a word from any part of it.
        picker.clear_filter();
        assert!(picker.type_text("model"));
        assert_eq!(
            picker.rows()[0],
            Row::Recent(PathBuf::from("/home/hec/models"))
        );
        assert_eq!(picker.cursor(), 0);

        // Nothing matched at all leaves the cursor on the folder being listed,
        // which is a way through the tree rather than a wrong choice.
        picker.clear_filter();
        assert!(picker.type_text("zzz"));
        assert_eq!(labels(&picker), before, "and the list is still whole");
        assert_eq!(picker.row(picker.cursor()), Some(&Row::Here));
        assert!(picker.step(true), "the way out is always a match");
        assert_eq!(picker.row(picker.cursor()), Some(&Row::Up));
        assert!(!picker.step(true), "and below it there is nothing to walk to");

        // A dot in front brings the hidden folders into the list. That is the
        // one thing typing still adds or removes rows for: it is about what is
        // worth listing, not about what is being searched for.
        picker.clear_filter();
        assert!(!labels(&picker).contains(&String::from(".cache")));
        assert!(picker.type_text(".ca"));
        assert!(labels(&picker).contains(&String::from(".cache")));
        assert_eq!(on(&picker), ".cache");

        // Backspace takes it back a character at a time, and with nothing typed
        // it is the way out of the folder.
        assert!(picker.backspace());
        assert_eq!(picker.filter(), ".c");
        assert!(picker.clear_filter());
        assert!(picker.backspace());
        assert_eq!(picker.at(), Path::new("/home"));
        assert!(!picker.clear_filter(), "and there is nothing left to clear");

        // A filter typed against one folder does not survive into the next: it
        // would leave most of the new one dim for no reason anybody typed.
        assert_eq!(picker.at(), Path::new("/home"));
        assert!(picker.type_text("hec"));
        assert!(picker.walk_in());
        assert_eq!(picker.at(), Path::new("/home/hec"));
        assert_eq!(picker.filter(), "");
    }

    /// The cursor stays inside the list whatever is pressed, and the window
    /// follows it rather than leaving it off screen.
    #[test]
    fn the_cursor_stays_in_the_list_and_the_window_follows_it() {
        let many: Vec<String> = (0..30).map(|n| format!("dir{n:02}")).collect();
        let inside: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut picker = Picker::open(
            Fake::new(&[("/tree", &inside), ("/", &["tree"])]),
            PathBuf::from("/tree"),
            Vec::new(),
        );
        assert_eq!(picker.rows().len(), 32, "here, up, and thirty folders");

        assert!(!picker.step(false), "already at the top");
        assert!(picker.step(true));
        assert_eq!(picker.cursor(), 1);
        assert!(picker.jump(true));
        assert_eq!(picker.cursor(), 31);
        assert!(!picker.step(true), "and it stops at the last row");
        assert!(picker.page(10, false));
        assert_eq!(picker.cursor(), 21);
        assert!(picker.page(100, false));
        assert_eq!(picker.cursor(), 0);

        // Eight rows on screen: the window moves by the least it takes and
        // stops where the last row is at the bottom.
        assert!(!picker.reveal(8));
        picker.jump(true);
        assert!(picker.reveal(8));
        assert_eq!(picker.first(), 24);
        picker.jump(false);
        assert!(picker.reveal(8));
        assert_eq!(picker.first(), 0);

        // The wheel moves the window without moving the cursor, and clamps.
        assert!(picker.scroll(5, true, 8));
        assert_eq!((picker.first(), picker.cursor()), (5, 0));
        assert!(picker.scroll(100, true, 8));
        assert_eq!(picker.first(), 24);
        assert!(!picker.scroll(1, true, 8));
        assert!(picker.scroll(100, false, 8));
        assert_eq!(picker.first(), 0);
        assert!(picker.thumb(8).is_some(), "and it says how much is showing");
        assert!(
            picker.thumb(40).is_none(),
            "a list that fits has no thumb to draw"
        );
        // A pane with no room and an empty list are both no-ops rather than a
        // position nothing can be drawn at.
        assert!(!picker.reveal(0));
    }

    /// `..` reads as a way out, so confirming it goes out rather than handing
    /// the parent to the agent. A double click reads the same way.
    #[test]
    fn confirming_the_way_out_walks_it_rather_than_choosing_it() {
        let mut picker = opened(&["/home/hec/models"]);
        let up = picker.rows().iter().position(|row| *row == Row::Up).unwrap();
        picker.point_at(up);
        assert_eq!(picker.confirm(), None);
        assert_eq!(picker.at(), Path::new("/home"));

        // A double click walks into a folder in the list, and opens one that was
        // already a whole answer.
        let mut picker = opened(&["/home/hec/models"]);
        let workspace = at(&picker, "workspace");
        assert_eq!(picker.double(workspace), None);
        assert_eq!(picker.at(), Path::new("/home/hec/workspace"));

        let mut picker = opened(&["/home/hec/models"]);
        assert_eq!(
            picker.double(0),
            Some(Chosen::folder(PathBuf::from("/home/hec/models"))),
            "a remembered folder opens"
        );
        let here = picker.rows().iter().position(|r| *r == Row::Here).unwrap();
        assert_eq!(
            picker.double(here),
            Some(Chosen::folder(PathBuf::from("/home/hec")))
        );
        // A row that is no longer there is not clamped onto its neighbour.
        assert_eq!(picker.double(99), None);
        assert!(!picker.point_at(99));
    }

    /// A folder that cannot be read says why. An empty list on its own looks
    /// exactly like an empty folder.
    #[test]
    fn a_folder_that_cannot_be_read_says_so() {
        let mut picker = Picker::open(
            Fake::new(&[("/tree", &["locked"]), ("/", &["tree"])]),
            PathBuf::from("/tree"),
            Vec::new(),
        );
        assert_eq!(picker.trouble(), None);
        let locked = at(&picker, "locked");
        picker.point_at(locked);
        assert!(picker.walk_in());
        assert_eq!(picker.trouble(), Some("permission denied"));
        // Still navigable: the way out is not part of the listing that failed.
        assert_eq!(labels(&picker), vec!["this folder", ".."]);
        assert!(picker.walk_out());
        assert_eq!(picker.trouble(), None);

        // Opened in place rather than walked into, it says the same thing on a
        // row of its own under it. Without that row an unreadable folder opens
        // to nothing, and on screen nothing is a folder that is empty.
        let locked = at(&picker, "locked");
        assert!(picker.toggle(locked));
        assert_eq!(
            tree(&picker),
            ["  this folder", "  ..", "- locked", "    permission denied"]
        );

        // The message is a message: there is nowhere to go from it, and it
        // carries no mark of its own because there is nothing to open.
        let why = at(&picker, "permission denied");
        assert_eq!(picker.row(why).and_then(Row::open), None);
        assert_eq!(picker.path_of(picker.row(why).unwrap()), None);
        assert!(picker.point_at(why));
        assert_eq!(picker.confirm(), None);
        assert_eq!(picker.at(), Path::new("/tree"), "and nothing was walked");
        assert!(!picker.walk_in());
        assert_eq!(picker.double(why), None);

        // Shutting the folder takes its message with it, and the cursor that
        // was standing on that message lands on the folder it belonged to.
        assert!(picker.toggle(locked));
        assert_eq!(tree(&picker), ["  this folder", "  ..", "+ locked"]);
        assert_eq!(on(&picker), "locked");
    }

    /// A folder opens where it stands, one step further in, and shuts again
    /// taking the whole of itself with it and nothing else.
    #[test]
    fn a_folder_opens_under_itself_and_shuts_back_up() {
        let mut picker = opened(&[]);
        let flat = [
            "  this folder",
            "  ..",
            "+ Desktop",
            "+ models",
            "+ Pictures",
            "+ workspace",
        ];
        assert_eq!(tree(&picker), flat);

        // What is inside it goes under it, one step in, in the same order it
        // would be listed in on its own.
        let workspace = at(&picker, "workspace");
        assert!(picker.toggle(workspace));
        assert_eq!(
            tree(&picker),
            [
                "  this folder",
                "  ..",
                "+ Desktop",
                "+ models",
                "+ Pictures",
                "- workspace",
                "  + anna",
                "  + noob-cli",
            ]
        );
        // And the folder being listed did not change: this is a tree, not a
        // walk. The rows above it did not move either.
        assert_eq!(picker.at(), Path::new("/home/hec"));
        assert_eq!(tree(&picker)[..5], flat[..5]);

        // A second level, whose rows carry the whole path rather than a name
        // joined to the folder being listed.
        let noob = at(&picker, "noob-cli");
        assert!(picker.toggle(noob));
        assert_eq!(
            tree(&picker)[6..],
            [
                "  + anna",
                "  - noob-cli",
                "    + crates",
                "    + gui",
            ]
        );
        let gui = at(&picker, "gui");
        assert_eq!(
            picker.path_of(picker.row(gui).unwrap()),
            Some(PathBuf::from("/home/hec/workspace/noob-cli/gui"))
        );

        // Shutting the top of it takes both levels and stops there: the rows
        // above and below the folder are exactly the ones that were there.
        assert!(picker.toggle(workspace));
        assert_eq!(tree(&picker), flat);

        // A folder with nothing in it opens to nothing, and says so by having
        // no rows under it rather than by refusing to open.
        let models = at(&picker, "models");
        assert!(picker.toggle(models));
        assert_eq!(picker.row(models).and_then(Row::open), Some(true));
        assert_eq!(tree(&picker).len(), flat.len());
        assert!(picker.toggle(models));

        // Walking still lists a folder on its own, from any depth, and the tree
        // starts again from there.
        picker.toggle(workspace);
        picker.toggle(at(&picker, "noob-cli"));
        assert!(picker.point_at(at(&picker, "noob-cli")));
        assert!(picker.walk_in());
        assert_eq!(picker.at(), Path::new("/home/hec/workspace/noob-cli"));
        assert_eq!(
            tree(&picker),
            ["  this folder", "  ..", "+ crates", "+ gui"],
            "the tree is the new folder's own, at the top of it"
        );
        assert!(picker.walk_out());
        assert_eq!(picker.at(), Path::new("/home/hec/workspace"));
        assert_eq!(tree(&picker), ["  this folder", "  ..", "+ anna", "+ noob-cli"]);

        // A row that is not a folder in the tree has no mark, and pressing
        // where one would be does nothing rather than opening the wrong row.
        for label in ["this folder", ".."] {
            let index = at(&picker, label);
            assert_eq!(picker.row(index).and_then(Row::open), None, "{label}");
            assert!(!picker.toggle(index), "{label}");
        }
        assert!(!picker.toggle(99));
    }

    /// The bug a tree is most likely to have: shutting a folder that held the
    /// cursor somewhere inside it has to leave the cursor on a real row, and
    /// opening one above the cursor must not slide the cursor onto its
    /// neighbour.
    #[test]
    fn the_cursor_lands_somewhere_real_when_a_folder_shuts_under_it() {
        let mut picker = opened(&[]);
        let workspace = at(&picker, "workspace");
        picker.toggle(workspace);
        picker.toggle(at(&picker, "noob-cli"));
        let gui = at(&picker, "gui");
        assert!(picker.point_at(gui));
        assert_eq!(on(&picker), "gui");

        // Two levels above it, shut. The row the cursor was on is not in the
        // list any more, so it lands on the folder that swallowed it.
        assert!(picker.toggle(workspace));
        assert_eq!(on(&picker), "workspace");
        assert_eq!(picker.cursor(), workspace);
        assert!(picker.cursor() < picker.rows().len());

        // A folder opening above the cursor moves every row under it down, and
        // the cursor goes with the row it was on rather than staying at its
        // index and reading as a different folder.
        let desktop = at(&picker, "Desktop");
        assert_eq!(picker.cursor(), at(&picker, "workspace"));
        assert!(picker.toggle(desktop), "Desktop cannot be read, so it says so");
        assert_eq!(on(&picker), "workspace");
        assert_eq!(picker.cursor(), workspace + 1);
        assert!(picker.toggle(desktop));
        assert_eq!(picker.cursor(), workspace, "and back again when it shuts");

        // The folder under the cursor opening keeps the cursor on it, mark and
        // all, so the next keystroke still acts on the folder that was pressed.
        assert!(picker.toggle(picker.cursor()));
        assert_eq!(on(&picker), "workspace");
        assert_eq!(picker.row(picker.cursor()).and_then(Row::open), Some(true));
    }

    /// A hidden folder that was opened takes what is inside it out of the list
    /// with it. A child left behind when its parent has gone is a row indented
    /// under nothing.
    #[test]
    fn a_folder_the_list_drops_takes_its_own_rows_with_it() {
        let mut picker = opened(&[]);
        assert!(picker.type_text(".ca"));
        let cache = at(&picker, ".cache");
        assert!(picker.toggle(cache));
        assert!(tree(&picker).contains(&String::from("  + thumbs")));

        assert!(picker.clear_filter());
        assert!(!labels(&picker).contains(&String::from(".cache")));
        assert!(
            !labels(&picker).contains(&String::from("thumbs")),
            "{:?}",
            tree(&picker)
        );
        // And it is still open underneath, so bringing it back brings back what
        // was showing rather than a folder that has to be opened again.
        assert!(picker.type_text(".ca"));
        assert!(tree(&picker).contains(&String::from("  + thumbs")));
    }

    /// Newest first, no duplicates, capped, and readable back.
    #[test]
    fn the_remembered_list_is_newest_first_and_capped() {
        let list = remember(&[], Path::new("/a"));
        assert_eq!(list, vec![PathBuf::from("/a")]);
        let list = remember(&list, Path::new("/b"));
        assert_eq!(list, vec![PathBuf::from("/b"), PathBuf::from("/a")]);
        // The same folder again moves to the front rather than appearing twice.
        let list = remember(&list, Path::new("/a"));
        assert_eq!(list, vec![PathBuf::from("/a"), PathBuf::from("/b")]);

        let long: Vec<PathBuf> = (0..REMEMBERED + 4)
            .map(|n| PathBuf::from(format!("/p{n}")))
            .collect();
        let list = remember(&long, Path::new("/new"));
        assert_eq!(list.len(), REMEMBERED);
        assert_eq!(list[0], PathBuf::from("/new"));

        // Through the file and back.
        assert_eq!(parse_recents(&recents_text(&list)), list);
    }

    /// The file is a nicety, so anything unreadable in it is a line that does
    /// not exist rather than a reason to refuse to open the window.
    #[test]
    fn a_missing_or_scribbled_file_is_a_first_run() {
        assert!(parse_recents("").is_empty());
        assert!(load_recents(Path::new("/nowhere/at/all/no0b.recent")).is_empty());
        let text = "# a comment\n\n/home/hec/one\n  /home/hec/two  \n/home/hec/one\n";
        assert_eq!(
            parse_recents(text),
            vec![
                PathBuf::from("/home/hec/one"),
                PathBuf::from("/home/hec/two")
            ],
            "comments and blank lines skipped, whitespace trimmed, no duplicates"
        );
        let long = (0..REMEMBERED + 5)
            .map(|n| format!("/p{n}\n"))
            .collect::<String>();
        assert_eq!(parse_recents(&long).len(), REMEMBERED);
    }

    /// The folders somebody has opened are the difference between the picker
    /// being one Enter and being a walk through the filesystem, so the rename
    /// carries the list rather than starting it again.
    #[test]
    fn the_folders_remembered_under_the_old_name_are_carried_over() {
        let dir = std::env::temp_dir().join(format!("no0b-recent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");
        let (legacy, current) = (dir.join("clippy.recent"), dir.join("no0b.recent"));
        std::fs::write(&legacy, "/home/hec/one\n/home/hec/two\n").expect("the old file");

        assert_eq!(
            load_recents(&current),
            vec![
                PathBuf::from("/home/hec/one"),
                PathBuf::from("/home/hec/two")
            ]
        );
        assert!(!legacy.exists(), "the old name is still there");

        // Both names present is not a migration: the current list wins and the
        // old file is left alone.
        std::fs::write(&legacy, "/home/hec/three\n").expect("the old file again");
        assert_eq!(load_recents(&current)[0], PathBuf::from("/home/hec/one"));
        assert!(legacy.exists(), "the old file was deleted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// When the fixture sessions are read, and when they were written. Fixed,
    /// so the ages the rows say are the same on every run.
    fn clock() -> SystemTime {
        SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000)
    }

    fn saved(id: &str, at: Option<&str>, said: &str, ago: u64) -> Saved {
        Saved {
            id: String::from(id),
            when: clock() - std::time::Duration::from_secs(ago),
            workspace: at.map(PathBuf::from),
            gone: false,
            opening: String::from(said),
        }
    }

    /// Three sessions and one file that could not be read: one belonging to the
    /// folder being looked at, one to another folder, one to a folder that has
    /// since been deleted, and one nothing ever noted a folder for.
    fn some_sessions() -> Listing {
        let mut deleted = saved("gone", Some("/home/hec/deleted"), "third", 3 * 86_400);
        deleted.gone = true;
        Listing {
            sessions: vec![
                saved("elsewhere", Some("/home/hec/workspace"), "second", 7200),
                deleted,
                saved("nowhere", None, "fourth", 8 * 86_400),
                saved("here", Some("/home/hec"), "first", 300),
            ],
            skipped: vec![String::from("half: no meta line")],
        }
    }

    fn showing(picker: &mut Picker) {
        picker.show_sessions_at(some_sessions(), clock());
    }

    /// The same box, the same cursor and the same keys, listing what the agent
    /// has already written instead of what is on the disk. The sessions for the
    /// folder being looked at come first, because that is the one you are most
    /// likely to be carrying on with.
    #[test]
    fn the_list_shows_the_saved_sessions_and_the_folder_you_are_on_comes_first() {
        let mut picker = opened(&["/home/hec/models"]);
        assert!(!picker.on_sessions());
        showing(&mut picker);
        assert!(picker.on_sessions());

        let ids: Vec<String> = picker
            .rows()
            .iter()
            .map(|row| match row {
                Row::Session(saved) => saved.id.clone(),
                other => panic!("not a session: {other:?}"),
            })
            .collect();
        assert_eq!(
            ids,
            ["here", "elsewhere", "gone", "nowhere"],
            "the folder being looked at first, then the order they were read in"
        );
        // Nothing else is in the list: the folder you are in and the way out of
        // it are places in a tree, and these rows are not in one.
        assert_eq!(picker.rows().len(), 4);
        assert_eq!(picker.cursor(), 0);
        assert_eq!(picker.first(), 0);

        // What a row says: how long ago it was, which folder it belongs to and
        // enough of the first thing that was said to recognise it.
        assert_eq!(labels(&picker)[0], "5m ago  hec  first");
        assert_eq!(labels(&picker)[1], "2h ago  workspace  second");
        assert_eq!(
            labels(&picker)[2],
            "3d ago  deleted (gone)  third",
            "a folder that is not there any more says so on the row"
        );
        assert_eq!(
            labels(&picker)[3],
            "1w ago  this folder  fourth",
            "a session the CLI wrote on its own opens in the folder above the list"
        );

        // And the line above the list says how many there are, including the
        // file that could not be described.
        assert_eq!(
            picker.note(),
            Some("4 saved sessions, 1 file could not be read")
        );
        assert_eq!(picker.refused(), None);

        // Back to the folders, and the second press has nothing left to close,
        // which is what makes Escape fall through to closing the window.
        assert!(picker.show_folders());
        assert!(!picker.on_sessions());
        assert_eq!(picker.note(), None);
        assert_eq!(picker.row(picker.cursor()), Some(&Row::Here));
        assert!(labels(&picker).contains(&String::from("workspace")));
        assert!(!picker.show_folders());
    }

    /// Confirming a session hands back the folder and the id together, which is
    /// what starts `noob serve --resume`.
    #[test]
    fn choosing_a_session_carries_its_id_and_the_folder_it_belongs_to() {
        let mut picker = opened(&[]);
        showing(&mut picker);
        assert_eq!(
            picker.confirm(),
            Some(Chosen {
                workspace: PathBuf::from("/home/hec"),
                session: Some(String::from("here")),
            })
        );

        // A double click is the same answer: a session is a whole choice, so
        // there is nothing to walk into.
        let mut picker = opened(&[]);
        showing(&mut picker);
        assert_eq!(
            picker.double(1),
            Some(Chosen {
                workspace: PathBuf::from("/home/hec/workspace"),
                session: Some(String::from("elsewhere")),
            })
        );

        // One nothing noted a folder for opens in the folder the list was
        // opened from, which is the folder it was picked in.
        let mut picker = opened(&[]);
        showing(&mut picker);
        assert!(picker.point_at(3));
        assert_eq!(
            picker.confirm(),
            Some(Chosen {
                workspace: PathBuf::from("/home/hec"),
                session: Some(String::from("nowhere")),
            })
        );
    }

    /// A session whose folder has been deleted cannot be resumed into thin air:
    /// the press is refused and the picker says why, rather than starting an
    /// agent in a directory nobody named.
    #[test]
    fn a_session_whose_folder_has_gone_is_refused_out_loud() {
        let mut picker = opened(&[]);
        showing(&mut picker);
        assert!(picker.point_at(2));
        assert_eq!(picker.confirm(), None);
        assert_eq!(
            picker.refused(),
            Some("/home/hec/deleted is not there any more")
        );
        assert_eq!(picker.double(2), None, "and a double click is not a way in");

        // Choosing one that is still there clears it, so the message on screen
        // is about the press that was just made.
        assert!(picker.point_at(0));
        assert!(picker.confirm().is_some());
        assert_eq!(picker.refused(), None);

        // Leaving the list clears it too: it is about a row that is no longer
        // on screen.
        assert!(picker.point_at(2));
        assert_eq!(picker.confirm(), None);
        assert!(picker.show_folders());
        assert_eq!(picker.refused(), None);
    }

    /// The keys the folder list has, over the session list: typing dims what it
    /// did not match, the arrows walk the matches, and the two that walk the
    /// tree do nothing because there is no tree here.
    #[test]
    fn the_session_list_takes_the_same_keys_as_the_folder_list() {
        let mut picker = opened(&[]);
        showing(&mut picker);
        let before = labels(&picker);

        assert!(picker.type_text("second"));
        assert_eq!(labels(&picker), before, "every row is still there");
        let dim: Vec<usize> = (0..picker.rows().len())
            .filter(|index| dimmed(&picker, *index))
            .collect();
        assert_eq!(dim, vec![0, 2, 3]);
        assert_eq!(picker.cursor(), 1, "the cursor lands on what was typed for");
        assert!(!picker.step(true), "and there is no match below it");
        assert!(!picker.step(false), "or above it");

        // A word from the folder finds it too, since the row says both.
        assert!(picker.clear_filter());
        assert!(picker.type_text("deleted"));
        assert_eq!(picker.cursor(), 2);
        assert!(picker.clear_filter());

        // The scroll window is the list's, the same as the folders'.
        assert!(picker.scroll(2, true, 2));
        assert_eq!((picker.first(), picker.cursor()), (2, 0));
        assert!(picker.reveal(2));
        assert_eq!(picker.first(), 0);
        assert!(picker.thumb(2).is_some());

        // Nothing to walk into and nothing to open: the rows are not places in
        // a tree, and walking would reorder the list under the cursor.
        assert!(!picker.walk_in());
        assert!(!picker.walk_out());
        assert!(!picker.toggle(0));
        assert_eq!(picker.row(0).and_then(Row::open), None);
        assert_eq!(picker.at(), Path::new("/home/hec"));

        // Backspace with nothing typed is the way out of a folder, and there is
        // no folder here either.
        assert!(!picker.backspace());
    }

    /// A directory with nothing in it is a list that says so, not an empty box.
    #[test]
    fn no_saved_sessions_at_all_is_a_line_that_says_so() {
        let mut picker = opened(&[]);
        picker.show_sessions_at(Listing::default(), clock());
        assert!(picker.on_sessions());
        assert!(picker.rows().is_empty());
        assert_eq!(picker.note(), Some("no saved sessions"));
        assert_eq!(picker.confirm(), None);
        assert!(!picker.step(true));
        assert!(!picker.reveal(8));

        // One is one, and the count of what could not be read is its own
        // sentence rather than a number nobody can parse.
        picker.show_sessions_at(
            Listing {
                sessions: vec![saved("only", None, "hello", 60)],
                skipped: vec![String::from("a: no meta line"), String::from("b: no meta line")],
            },
            clock(),
        );
        assert_eq!(
            picker.note(),
            Some("1 saved session, 2 files could not be read")
        );
    }
}
