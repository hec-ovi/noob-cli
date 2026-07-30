//! The sessions the agent has already written, as a list you can pick from.
//!
//! `noob serve` keeps every conversation in `<config>/sessions/<id>.jsonl`, one
//! JSON object per line, and takes `--resume <id>` to carry one on. That is the
//! whole of the plumbing: this module is the reading half, so the window can
//! offer those files instead of a fresh session.
//!
//! Only the head of each file is read. A session that has been running all
//! afternoon is megabytes of transcript, and the four things a row needs (which
//! session, when it was last touched, which folder it belongs to and enough of
//! the first thing that was said to recognise it) are all in the first two
//! lines. [`HEAD_BYTES`] is the cap; nothing here ever reads a whole file.
//!
//! Nothing here is fatal. These files are appended to by a process that can be
//! killed between the write and the newline, so a line that does not parse is a
//! line that does not exist, and a file with nothing usable in it is one row
//! missing from the list rather than a list that refuses to be drawn. What was
//! skipped is counted and said out loud, because a session quietly missing from
//! the list is worse than a session listed as unreadable.
//!
//! ## Where the folder comes from
//!
//! The session file does not record it. The transcript format is the CLI's and
//! it holds the conversation, not the directory it happened in, so the window
//! keeps its own note: [`Index`] is a small file beside the settings, written
//! when the agent reports a session has started, mapping session to folder.
//! That is the only place the two are ever tied together, which means a session
//! written by the CLI on its own has no folder here and says so.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::picker::Folders;

/// How much of a session file is read to describe it.
///
/// The meta line and the first message are the first two lines of the file, so
/// this is generous by three orders of magnitude and still bounded: a resumed
/// session that has compacted twice can carry a `reset` line holding the whole
/// transcript, and reading that to find out what somebody typed first is work
/// nobody asked for.
const HEAD_BYTES: u64 = 64 * 1024;

/// How much of the first message is kept. What a row can show is narrower than
/// this; the rest is there so the filter has something to match on.
const OPENING_CHARS: usize = 200;

/// How many sessions are described, newest first.
///
/// The whole directory is listed (that is one syscall and a stat per file), but
/// only this many are opened and read. A machine that has been talking to the
/// agent for a year has thousands of these, and nobody is scrolling to the one
/// from March.
pub const MOST: usize = 200;

/// How many folders the index remembers. One per session listed, with room to
/// spare, so a session that is still on screen cannot have lost its folder.
pub const REMEMBERED: usize = 400;

/// One saved session, as much of it as a row needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    /// The file name without its suffix, which is what `--resume` takes.
    pub id: String,
    /// When the file was last written to.
    pub when: SystemTime,
    /// The folder it was started in, when the window has a note of it.
    pub workspace: Option<PathBuf>,
    /// Whether that folder is missing now. A session cannot be resumed into a
    /// directory that is not there, so this is decided when the list is read
    /// and carried on the row rather than asked again while drawing.
    pub gone: bool,
    /// The opening of the first thing the human said, on one line.
    pub opening: String,
}

/// What a read of the directory came back with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// Newest first.
    pub sessions: Vec<Saved>,
    /// One line per file that could not be described, and why.
    pub skipped: Vec<String>,
}

/// Where the agent keeps them: `sessions/` under its own config directory.
///
/// The agent's, not the window's. `noob` resolves that directory as
/// `$NOOB_CONFIG_DIR`, then `/config` when it is there (the bind mount it is
/// given inside a container), then `~/.config/noob`, and the files being read
/// here are the ones it wrote. Deriving this from where the window keeps its
/// own settings would come apart the moment either rule changed, and on a
/// machine with `$XDG_CONFIG_HOME` set it would already be looking in the wrong
/// place.
pub fn dir() -> Option<PathBuf> {
    dir_from(
        std::env::var_os("NOOB_CONFIG_DIR").map(PathBuf::from),
        Some(PathBuf::from("/config")).filter(|path| path.is_dir()),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// That rule with the machine's answers passed in, so it can be checked without
/// setting environment variables under a test runner that shares them.
fn dir_from(
    named: Option<PathBuf>,
    container: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    let base = named
        .filter(|path| !path.as_os_str().is_empty())
        .or(container)
        .or_else(|| Some(home?.join(".config").join("noob")))?;
    Some(base.join("sessions"))
}

/// Describe every session in `at`, newest first.
///
/// `index` says which folder each one belongs to and `folders` answers whether
/// that folder is still there, so the list knows which rows can be resumed
/// before anybody presses one.
///
/// A directory that is not there is not an error: it is a machine where the
/// agent has never run.
pub fn read(at: &Path, index: &Index, folders: &dyn Folders) -> Listing {
    let mut listing = Listing::default();
    let mut files: Vec<(String, SystemTime)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(at) else {
        return listing;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".jsonl")) else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            listing.skipped.push(format!("{id}: cannot be looked at"));
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        files.push((id.to_string(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
    }
    // Newest first before anything is opened, so the cap falls on the sessions
    // nobody was going to scroll to.
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (id, when) in files.into_iter().take(MOST) {
        match opening_of(&at.join(format!("{id}.jsonl"))) {
            Ok(opening) => {
                let workspace = index.folder_of(&id);
                let gone = workspace
                    .as_deref()
                    .is_some_and(|path| !folders.is_folder(path));
                listing.sessions.push(Saved {
                    id,
                    when,
                    workspace,
                    gone,
                    opening,
                });
            }
            Err(why) => listing.skipped.push(format!("{id}: {why}")),
        }
    }
    listing
}

/// The head of one session file, as the first thing the human said in it.
///
/// The id is taken from the file name rather than from the meta line, because
/// the file name is what `--resume` opens: a file whose meta line disagrees
/// with its name would otherwise be resumed as a session that does not exist.
/// The meta line still has to be there, since that is what makes the file a
/// session rather than something else that happens to end in `.jsonl`.
fn opening_of(path: &Path) -> Result<String, String> {
    let mut head = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(HEAD_BYTES)
        .read_to_end(&mut head)
        .map_err(|e| e.to_string())?;
    // Lossy on purpose: the cap above can land in the middle of a character,
    // and so can a kill in the middle of a write.
    let head = String::from_utf8_lossy(&head);
    let mut meta = false;
    let mut opening: Option<String> = None;
    for line in head.lines() {
        // A line that does not parse is a line that does not exist: the last
        // one in a file whose writer was killed is half a line.
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match value.get("t").and_then(Value::as_str) {
            Some("meta") => meta = true,
            Some("item") => opening = opening.or_else(|| said(value.get("item"))),
            // Compaction replaces the transcript, so on a session that has
            // compacted the first thing anybody said is inside this line.
            Some("reset") => {
                opening = opening.or_else(|| {
                    value
                        .get("items")
                        .and_then(Value::as_array)?
                        .iter()
                        .find_map(|item| said(Some(item)))
                });
            }
            _ => {}
        }
        if meta && opening.is_some() {
            break;
        }
    }
    match meta {
        true => Ok(opening.unwrap_or_default()),
        false => Err(String::from("no meta line")),
    }
}

/// What the human said in one transcript item, on a single line, or nothing
/// when the item is not one of theirs.
fn said(item: Option<&Value>) -> Option<String> {
    let item = item?;
    if item.get("role").and_then(Value::as_str) != Some("user") {
        return None;
    }
    let text = item.get("text").and_then(Value::as_str)?;
    let text = one_line(text);
    match text.is_empty() {
        true => None,
        false => Some(text),
    }
}

/// A message as one row of text: every run of whitespace becomes one space, and
/// what is left is cut to [`OPENING_CHARS`] characters.
///
/// By character rather than by byte, or a prompt that opens with an emoji would
/// be cut through the middle of it.
fn one_line(text: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for ch in text.chars() {
        if ch.is_whitespace() || ch.is_control() {
            gap = !out.is_empty();
            continue;
        }
        if gap {
            out.push(' ');
            gap = false;
        }
        if out.chars().count() >= OPENING_CHARS {
            break;
        }
        out.push(ch);
    }
    out
}

/// How long ago `when` was, in the shortest form that still says it.
///
/// `now` is passed in rather than read here, so a row's age is decided once
/// when the list is built and a test can say what time it is.
pub fn ago(when: SystemTime, now: SystemTime) -> String {
    let Ok(gap) = now.duration_since(when) else {
        // A file dated in the future is a clock that has been changed, not a
        // session from tomorrow.
        return String::from("just now");
    };
    let secs = gap.as_secs();
    match secs {
        0..=59 => String::from("just now"),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        86_400..=604_799 => format!("{}d ago", secs / 86_400),
        _ => format!("{}w ago", secs / 604_800),
    }
}

/// Which folder each session was started in.
///
/// Newest first, one entry per session, capped. The list is a `Vec` rather than
/// a map because it is written back in order: the file is meant to be readable,
/// and the newest session being the first line is what makes it so.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index(Vec<(String, PathBuf)>);

impl Index {
    pub fn folder_of(&self, id: &str) -> Option<PathBuf> {
        self.0
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, path)| path.clone())
    }

    /// The same list with `id` at the front. Pure, and it takes the list rather
    /// than reading the file, so the caller can re-read immediately before
    /// writing and two windows cannot erase each other's notes.
    pub fn plus(&self, id: &str, workspace: &Path) -> Index {
        let mut out = vec![(id.to_string(), workspace.to_path_buf())];
        out.extend(
            self.0
                .iter()
                .filter(|(known, _)| known != id)
                .cloned(),
        );
        out.truncate(REMEMBERED);
        Index(out)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.len()
    }
}

/// Where the note lives: beside `no0b.conf`, under the same rules as the
/// folders the picker remembers.
pub fn index_path() -> Option<PathBuf> {
    Some(crate::config::path()?.with_file_name("no0b.sessions"))
}

/// Read it. A missing or unreadable file is a window that has not started a
/// session yet, which is a list of sessions with no folders on them rather than
/// anything to report.
pub fn load_index(path: &Path) -> Index {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_index(&text),
        Err(_) => Index::default(),
    }
}

/// One entry per line, `<id> <folder>`, `#` a comment.
///
/// The id is split off at the first space because it cannot contain one and a
/// path can: splitting the other way round would break on the first folder with
/// a space in its name.
pub fn parse_index(text: &str) -> Index {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((id, path)) = line.split_once(' ') else {
            continue;
        };
        let path = path.trim();
        if id.is_empty() || path.is_empty() || out.iter().any(|(known, _)| known == id) {
            continue;
        }
        out.push((id.to_string(), PathBuf::from(path)));
    }
    out.truncate(REMEMBERED);
    Index(out)
}

pub fn index_text(index: &Index) -> String {
    let mut out = String::from(
        "# Which folder each noob session was started in, newest first.\n\
         # Written by NO0B so a saved session can be resumed where it belongs.\n",
    );
    for (id, path) in index.0.iter().take(REMEMBERED) {
        out.push_str(id);
        out.push(' ');
        out.push_str(&path.display().to_string());
        out.push('\n');
    }
    out
}

/// Replace the file, by rename, through the same writer the settings and the
/// remembered folders use.
pub fn save_index(path: &Path, index: &Index) -> Result<(), String> {
    crate::config::replace_file(path, &index_text(index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The real filesystem, which is what these tests drive: the fixtures are
    /// written to a temp directory, so the reader is exercised over files a
    /// process could actually have left behind.
    struct Real;

    impl Folders for Real {
        fn list(&self, _at: &Path) -> Result<Vec<String>, String> {
            Ok(Vec::new())
        }

        fn is_folder(&self, at: &Path) -> bool {
            at.is_dir()
        }
    }

    /// A directory of this test's own, removed and remade so a rerun starts
    /// clean.
    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "no0b-sessions-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");
        dir
    }

    fn write(dir: &Path, id: &str, text: &str) {
        std::fs::write(dir.join(format!("{id}.jsonl")), text).expect("a fixture");
    }

    /// A whole, well formed session: the meta line, what was asked, and what
    /// came back.
    fn good(id: &str, said: &str) -> String {
        format!(
            "{{\"created_ms\":1785404688765,\"id\":\"{id}\",\"t\":\"meta\",\"v\":1}}\n\
             {{\"item\":{{\"role\":\"user\",\"text\":\"{said}\"}},\"t\":\"item\"}}\n\
             {{\"item\":{{\"calls\":[],\"role\":\"assistant\",\"text\":\"sure\"}},\"t\":\"item\"}}\n"
        )
    }

    fn ids(listing: &Listing) -> Vec<String> {
        listing.sessions.iter().map(|s| s.id.clone()).collect()
    }

    /// The four files a directory of these can hold: one that is whole, one cut
    /// off mid-line by a process that was killed, one that is not a session at
    /// all, and one that is empty.
    #[test]
    fn a_directory_of_sessions_reads_as_the_ones_that_can_be_described() {
        let dir = temp("mixed");
        write(&dir, "whole", &good("whole", "add a session list to the picker"));
        // Killed between the write and the newline: the last line is half a
        // JSON object. Everything before it still describes the session.
        write(
            &dir,
            "cut",
            &format!(
                "{}{{\"item\":{{\"role\":\"assist",
                good("cut", "what does dev.sh gui-check do?")
            ),
        );
        // No meta line at all, so nothing says this file is a session.
        write(
            &dir,
            "headless",
            "{\"item\":{\"role\":\"user\",\"text\":\"hello\"},\"t\":\"item\"}\n",
        );
        write(&dir, "empty", "");
        // Not a session file, and not a reason to skip anything either.
        std::fs::write(dir.join("notes.txt"), "nothing").expect("a stray file");

        let listing = read(&dir, &Index::default(), &Real);
        let described: Vec<&str> = listing
            .sessions
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(
            described.iter().copied().collect::<std::collections::BTreeSet<_>>(),
            ["cut", "whole"].into_iter().collect(),
            "a truncated file still describes itself; one with no meta line does not"
        );
        let cut = listing.sessions.iter().find(|s| s.id == "cut").unwrap();
        assert_eq!(cut.opening, "what does dev.sh gui-check do?");
        let whole = listing.sessions.iter().find(|s| s.id == "whole").unwrap();
        assert_eq!(whole.opening, "add a session list to the picker");
        assert_eq!(whole.workspace, None, "nothing has said where it was");
        assert!(!whole.gone, "a folder nobody knows is not a folder that went");

        // What was left out is said, not swallowed.
        let mut skipped = listing.skipped.clone();
        skipped.sort();
        assert_eq!(
            skipped,
            vec!["empty: no meta line", "headless: no meta line"],
            "{:?}",
            listing.skipped
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A directory with nothing in it, and one that was never there. Both are a
    /// machine where the agent has not run, and neither is an error.
    #[test]
    fn an_empty_directory_is_an_empty_list() {
        let dir = temp("empty");
        assert_eq!(read(&dir, &Index::default(), &Real), Listing::default());
        assert_eq!(
            read(&dir.join("never"), &Index::default(), &Real),
            Listing::default()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The folder comes from the index, and a folder that has been deleted
    /// since is marked rather than dropped: a session you can see and cannot
    /// resume beats a session that has silently gone.
    #[test]
    fn a_session_carries_the_folder_it_was_started_in() {
        let dir = temp("folders");
        write(&dir, "here", &good("here", "one"));
        write(&dir, "gone", &good("gone", "two"));
        let here = dir.join("workspace");
        std::fs::create_dir_all(&here).expect("a folder to belong to");
        let index = Index::default()
            .plus("here", &here)
            .plus("gone", Path::new("/nowhere/at/all"));

        let listing = read(&dir, &index, &Real);
        let of = |id: &str| {
            listing
                .sessions
                .iter()
                .find(|s| s.id == id)
                .unwrap()
                .clone()
        };
        assert_eq!(of("here").workspace, Some(here));
        assert!(!of("here").gone);
        assert_eq!(of("gone").workspace, Some(PathBuf::from("/nowhere/at/all")));
        assert!(of("gone").gone, "a folder that is not there has to say so");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Newest first, and no more than the cap however many there are.
    #[test]
    fn the_list_is_newest_first_and_bounded() {
        let dir = temp("order");
        for n in 0..3 {
            write(&dir, &format!("s{n}"), &good(&format!("s{n}"), "hello"));
            // The files are written in order, so their times are in order, and
            // a filesystem with one second of resolution still sorts them by
            // the tie break on the name.
            std::thread::sleep(Duration::from_millis(10));
        }
        let listing = read(&dir, &Index::default(), &Real);
        let newest = listing.sessions.first().map(|s| s.id.clone());
        assert_eq!(newest, Some(String::from("s2")), "{:?}", ids(&listing));
        assert!(listing.sessions.len() <= MOST);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A prompt several paragraphs long is one row of text, and a huge one is
    /// cut rather than carried whole.
    #[test]
    fn the_opening_is_one_line_of_it() {
        let dir = temp("opening");
        write(
            &dir,
            "wrapped",
            "{\"t\":\"meta\",\"v\":1,\"id\":\"wrapped\"}\n\
             {\"t\":\"item\",\"item\":{\"role\":\"user\",\"text\":\"  first line\\n\\nsecond   line \"}}\n",
        );
        let long = "x".repeat(OPENING_CHARS * 3);
        write(
            &dir,
            "long",
            &format!(
                "{{\"t\":\"meta\",\"v\":1,\"id\":\"long\"}}\n\
                 {{\"t\":\"item\",\"item\":{{\"role\":\"user\",\"text\":\"{long}\"}}}}\n"
            ),
        );
        // A session that has compacted: the transcript was replaced, so what
        // was first said is inside the reset line.
        write(
            &dir,
            "compacted",
            "{\"t\":\"meta\",\"v\":1,\"id\":\"compacted\"}\n\
             {\"t\":\"reset\",\"items\":[{\"role\":\"assistant\",\"text\":\"summary\"},\
             {\"role\":\"user\",\"text\":\"carry on\"}]}\n",
        );
        // Nothing was ever said in this one. It is still a session.
        write(&dir, "silent", "{\"t\":\"meta\",\"v\":1,\"id\":\"silent\"}\n");

        let listing = read(&dir, &Index::default(), &Real);
        let of = |id: &str| {
            listing
                .sessions
                .iter()
                .find(|s| s.id == id)
                .unwrap()
                .opening
                .clone()
        };
        assert_eq!(of("wrapped"), "first line second line");
        assert_eq!(of("long").chars().count(), OPENING_CHARS);
        assert_eq!(of("compacted"), "carry on");
        assert_eq!(of("silent"), "");
        assert!(listing.skipped.is_empty(), "{:?}", listing.skipped);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The ages a row can say, including a file dated after the clock it is
    /// being read against.
    #[test]
    fn an_age_reads_in_the_largest_unit_that_fits() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        let back = |secs: u64| ago(now - Duration::from_secs(secs), now);
        assert_eq!(back(0), "just now");
        assert_eq!(back(59), "just now");
        assert_eq!(back(60), "1m ago");
        assert_eq!(back(3599), "59m ago");
        assert_eq!(back(3600), "1h ago");
        assert_eq!(back(86_399), "23h ago");
        assert_eq!(back(86_400), "1d ago");
        assert_eq!(back(604_799), "6d ago");
        assert_eq!(back(604_800), "1w ago");
        assert_eq!(
            ago(now + Duration::from_secs(60), now),
            "just now",
            "a clock that was changed is not a session from tomorrow"
        );
    }

    /// The note of which folder a session belongs to: newest first, one entry
    /// per session, and readable back exactly as written.
    #[test]
    fn the_folder_note_is_newest_first_and_survives_the_file() {
        let index = Index::default()
            .plus("one", Path::new("/home/hec/workspace/noob-cli"))
            .plus("two", Path::new("/home/hec/a folder with spaces"));
        assert_eq!(
            index.folder_of("two"),
            Some(PathBuf::from("/home/hec/a folder with spaces")),
            "a path with a space in it survives the split"
        );
        assert_eq!(index.folder_of("nobody"), None);
        assert_eq!(parse_index(&index_text(&index)), index);

        // The same session again moves to the front and is not written twice.
        let again = index.plus("one", Path::new("/home/hec/elsewhere"));
        assert_eq!(again.len(), 2);
        assert_eq!(
            again.folder_of("one"),
            Some(PathBuf::from("/home/hec/elsewhere"))
        );

        // Scribbles are lines that do not exist, and a missing file is a window
        // that has not started a session yet.
        let scratch = parse_index("# a comment\n\nnospace\n  \nid /a/path\nid /another\n");
        assert_eq!(scratch.folder_of("id"), Some(PathBuf::from("/a/path")));
        assert_eq!(scratch.len(), 1, "the first line about a session wins");
        assert_eq!(load_index(Path::new("/nowhere/at/all")), Index::default());

        let mut long = Index::default();
        for n in 0..REMEMBERED + 10 {
            long = long.plus(&format!("s{n}"), Path::new("/p"));
        }
        assert_eq!(long.len(), REMEMBERED);
        assert_eq!(parse_index(&index_text(&long)).len(), REMEMBERED);
    }

    /// The window has to look where the agent writes, which is the agent's rule
    /// and not the window's: a named directory wins, then the mount a container
    /// is given, then the home directory.
    #[test]
    fn the_sessions_are_read_from_the_agents_own_config_directory() {
        let named = Some(PathBuf::from("/somewhere/noob"));
        let container = Some(PathBuf::from("/config"));
        let home = Some(PathBuf::from("/home/hec"));
        assert_eq!(
            dir_from(named.clone(), container.clone(), home.clone()),
            Some(PathBuf::from("/somewhere/noob/sessions"))
        );
        assert_eq!(
            dir_from(None, container, home.clone()),
            Some(PathBuf::from("/config/sessions"))
        );
        assert_eq!(
            dir_from(None, None, home.clone()),
            Some(PathBuf::from("/home/hec/.config/noob/sessions"))
        );
        assert_eq!(
            dir_from(Some(PathBuf::new()), None, home),
            Some(PathBuf::from("/home/hec/.config/noob/sessions")),
            "an empty variable is a variable nobody set"
        );
        assert_eq!(dir_from(None, None, None), None, "and nowhere is nowhere");
    }

    /// Through a real file, since that is the only way the write half is
    /// exercised at all.
    #[test]
    fn the_note_is_written_where_it_can_be_read_again() {
        let dir = temp("note");
        let path = dir.join("no0b.sessions");
        let index = Index::default().plus("abc", Path::new("/home/hec/workspace"));
        save_index(&path, &index).expect("the note is writable");
        assert_eq!(load_index(&path), index);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

