//! Choosing the folder the agent works in.
//!
//! Launched from the dock with no argument, NO0B used to call `current_dir()`,
//! which under a desktop launcher is `$HOME`, and hand the agent the home
//! directory without ever saying so. This module is the model behind the picker
//! that opens instead: what is listed, where the cursor is, walking in and out
//! of folders, and the filter that marks what you typed for.
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

use std::path::{Path, PathBuf};

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
    /// A folder inside the one being listed, by name.
    Folder(String),
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
    /// Every folder inside `at`, sorted, before the filter.
    inside: Vec<String>,
    /// Why `at` could not be read, when it could not. Worth saying out loud: an
    /// empty list looks exactly like an empty folder.
    trouble: Option<String>,
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
            Row::Folder(name) => name.clone(),
        }
    }

    /// The folder a row names, or nothing when there is none (the root has no
    /// parent, so it has no way out).
    pub fn path_of(&self, row: &Row) -> Option<PathBuf> {
        match row {
            Row::Here => Some(self.at.clone()),
            Row::Up => self.at.parent().map(Path::to_path_buf),
            Row::Recent(path) => Some(path.clone()),
            Row::Folder(name) => Some(self.at.join(name)),
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
    /// remembered folder on its whole path, a folder in the list on its name.
    pub fn matched(&self, row: &Row) -> bool {
        match row {
            Row::Here | Row::Up => true,
            Row::Recent(path) => self.hit(&path.display().to_string()),
            Row::Folder(name) => self.hit(name),
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
            Row::Folder(name) => {
                let at = self.at.join(name);
                self.go_to(at)
            }
        }
    }

    /// List the folder above the one being listed.
    pub fn walk_out(&mut self) -> bool {
        let Some(up) = self.at.parent().map(Path::to_path_buf) else {
            return false;
        };
        self.go_to(up)
    }

    /// Confirm the cursor's row: the folder to hand the agent, or nothing.
    ///
    /// Nothing means the row was a way through the tree rather than a choice, in
    /// which case it has been walked. `..` reads as "go up" on screen and has to
    /// do that when it is confirmed; returning the parent as the workspace would
    /// open a folder nobody pointed at.
    pub fn confirm(&mut self) -> Option<PathBuf> {
        let row = self.rows.get(self.cursor).cloned()?;
        if row == Row::Up {
            self.walk_out();
            return None;
        }
        self.path_of(&row)
    }

    /// What a double click on a row does.
    ///
    /// A row naming a folder that was chosen before, or the folder being listed,
    /// opens: those are whole answers already. A row that is a step through the
    /// tree is walked into, which is what a double click does in every file
    /// manager.
    pub fn double(&mut self, index: usize) -> Option<PathBuf> {
        // The row that was clicked, not the row the cursor is on: a click on a
        // row that is no longer there must not open whatever happens to be
        // under the cursor instead.
        let row = self.rows.get(index).cloned()?;
        self.point_at(index);
        match row {
            Row::Here | Row::Recent(_) => self.path_of(&row),
            Row::Up | Row::Folder(_) => {
                self.walk_in();
                None
            }
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
        match self.folders.list(&self.at) {
            Ok(mut names) => {
                names.sort_by_key(|name| name.to_lowercase());
                self.inside = names;
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
            .position(|row| matches!(row, Row::Recent(_) | Row::Folder(_)) && self.matched(row))
            .or_else(|| self.match_from(0, true))
            .unwrap_or(0);
        self.first = 0;
    }

    fn rebuild(&mut self) {
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
        rows.extend(
            self.inside
                .iter()
                .filter(|name| self.listed(name))
                .cloned()
                .map(Row::Folder),
        );
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
pub fn parse_recents(text: &str) -> Vec<PathBuf> {
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

pub fn recents_text(list: &[PathBuf]) -> String {
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
            picker.path_of(&Row::Folder(String::from("workspace"))),
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
            Some(PathBuf::from("/home/hec/workspace/noob-cli"))
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
        picker.point_at(labels(&picker).iter().position(|l| l == "workspace").unwrap());
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
        let workspace = labels(&picker).iter().position(|l| l == "workspace").unwrap();
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
            picker.row(picker.cursor()),
            Some(&Row::Folder(String::from("workspace"))),
            "Enter after typing has to open what was typed"
        );

        // The arrows walk the matches: from `workspace` the row above is the way
        // out, four dim rows further up, and back down again is `workspace`.
        assert!(picker.step(false));
        assert_eq!(picker.row(picker.cursor()), Some(&Row::Up));
        assert!(picker.step(true));
        assert_eq!(
            picker.row(picker.cursor()),
            Some(&Row::Folder(String::from("workspace")))
        );
        assert!(!picker.step(true), "and there is no match below it");
        assert!(picker.jump(false));
        assert_eq!(
            picker.row(picker.cursor()),
            Some(&Row::Here),
            "the top of the list is the first match, not the first row"
        );
        assert!(picker.jump(true));
        assert_eq!(
            picker.row(picker.cursor()),
            Some(&Row::Folder(String::from("workspace")))
        );
        // A screenful lands on a match too, rather than on whatever row is that
        // many down.
        assert!(picker.page(3, false));
        assert!(!dimmed(&picker, picker.cursor()));

        // A dim row is a real folder: the pointer may land on it, and Enter then
        // opens it. Only the arrows skip it.
        let desktop = before.iter().position(|l| l == "Desktop").unwrap();
        assert!(dimmed(&picker, desktop));
        assert!(picker.point_at(desktop));
        assert_eq!(picker.confirm(), Some(PathBuf::from("/home/hec/Desktop")));

        // Case does not matter, and it matches anywhere in the name.
        let mut picker = opened(&["/home/hec/models"]);
        assert!(picker.type_text("KSP"));
        assert_eq!(
            picker.row(picker.cursor()),
            Some(&Row::Folder(String::from("workspace")))
        );

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
        assert_eq!(
            picker.row(picker.cursor()),
            Some(&Row::Folder(String::from(".cache")))
        );

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
        let workspace = labels(&picker).iter().position(|l| l == "workspace").unwrap();
        assert_eq!(picker.double(workspace), None);
        assert_eq!(picker.at(), Path::new("/home/hec/workspace"));

        let mut picker = opened(&["/home/hec/models"]);
        assert_eq!(
            picker.double(0),
            Some(PathBuf::from("/home/hec/models")),
            "a remembered folder opens"
        );
        let here = picker.rows().iter().position(|r| *r == Row::Here).unwrap();
        assert_eq!(picker.double(here), Some(PathBuf::from("/home/hec")));
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
        let locked = picker
            .rows()
            .iter()
            .position(|row| *row == Row::Folder(String::from("locked")))
            .unwrap();
        picker.point_at(locked);
        assert!(picker.walk_in());
        assert_eq!(picker.trouble(), Some("permission denied"));
        // Still navigable: the way out is not part of the listing that failed.
        assert_eq!(labels(&picker), vec!["this folder", ".."]);
        assert!(picker.walk_out());
        assert_eq!(picker.trouble(), None);
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
}
