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
const MOST: usize = 200;

/// How many folders the index remembers. One per session listed, with room to
/// spare, so a session that is still on screen cannot have lost its folder.
pub const REMEMBERED: usize = 400;

/// Where folder questions get answered. Owned here because this box asks
/// them (`read` resolves each saved session's workspace); the picker and
/// the shell implement it over the real filesystem or a test tree.
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
    /// How big the transcript is on disk. Free: the directory is stat'ed anyway
    /// to sort the list by age, and this is the other number that stat carries.
    pub bytes: u64,
    /// How full the context window was the last time this window watched this
    /// session run, when it ever did. Nothing at all for a session written
    /// before the note started carrying it, and for every session the CLI wrote
    /// on its own: the transcript does not record it and there is no honest
    /// guess at it. See [`Index`].
    pub context: Option<Context>,
    /// The opening of the first thing the human said, on one line.
    pub opening: String,
}

/// How full a session's context window was: how much of it was in use, out of
/// how much there is.
///
/// Two numbers rather than a fraction, because the file they are written to is
/// meant to be read by a human, and "48000/200000" says what "0.24" does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Context {
    pub used: u64,
    pub total: u64,
}

impl Context {
    /// How full, in whole percent, or nothing when the size of the window was
    /// never reported. A percentage of an unknown total is a made-up number.
    pub fn percent(&self) -> Option<u64> {
        match self.total {
            0 => None,
            total => Some((self.used.saturating_mul(100) / total).min(100)),
        }
    }
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
/// The agent's, not the window's, and the rule for finding it lives in
/// [`crate::agent`] because three other things in that directory are read the
/// same way. Deriving it from where the window keeps its own settings would come
/// apart the moment either rule changed, and on a machine with
/// `$XDG_CONFIG_HOME` set it would already be looking in the wrong place.
pub fn dir() -> Option<PathBuf> {
    Some(dir_in(&crate::agent::config_dir()?))
}

fn dir_in(config: &Path) -> PathBuf {
    config.join("sessions")
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
    let mut files: Vec<(String, SystemTime, u64)> = Vec::new();
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
        files.push((
            id.to_string(),
            meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            meta.len(),
        ));
    }
    // Newest first before anything is opened, so the cap falls on the sessions
    // nobody was going to scroll to.
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (id, when, bytes) in files.into_iter().take(MOST) {
        match opening_of(&at.join(format!("{id}.jsonl"))) {
            Ok(opening) => {
                let workspace = index.folder_of(&id);
                let gone = workspace
                    .as_deref()
                    .is_some_and(|path| !folders.is_folder(path));
                listing.sessions.push(Saved {
                    id: id.clone(),
                    when,
                    workspace,
                    gone,
                    bytes,
                    context: index.context_of(&id),
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

/// What the window knows about one session that the transcript does not say.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Note {
    id: String,
    workspace: PathBuf,
    context: Option<Context>,
}

/// Which folder each session was started in, and how full its context window
/// was when this window last watched it run.
///
/// Newest first, one entry per session, capped. The list is a `Vec` rather than
/// a map because it is written back in order: the file is meant to be readable,
/// and the newest session being the first line is what makes it so.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index(Vec<Note>);

impl Index {
    fn note_of(&self, id: &str) -> Option<&Note> {
        self.0.iter().find(|note| note.id == id)
    }

    fn folder_of(&self, id: &str) -> Option<PathBuf> {
        self.note_of(id).map(|note| note.workspace.clone())
    }

    fn context_of(&self, id: &str) -> Option<Context> {
        self.note_of(id).and_then(|note| note.context)
    }

    /// The same list with `id` at the front. Pure, and it takes the list rather
    /// than reading the file, so the caller can re-read immediately before
    /// writing and two windows cannot erase each other's notes.
    ///
    /// A context reading already written down for that session is kept: this is
    /// called again every time a session is resumed, and starting over would
    /// throw away the last thing known about it.
    pub fn plus(&self, id: &str, workspace: &Path) -> Index {
        let context = self.context_of(id);
        self.noting(id, workspace, context)
    }

    /// The same, with what the context window was holding written on it.
    pub fn plus_context(&self, id: &str, workspace: &Path, context: Context) -> Index {
        self.noting(id, workspace, Some(context))
    }

    fn noting(&self, id: &str, workspace: &Path, context: Option<Context>) -> Index {
        let mut out = vec![Note {
            id: id.to_string(),
            workspace: workspace.to_path_buf(),
            context,
        }];
        out.extend(self.0.iter().filter(|note| note.id != id).cloned());
        out.truncate(REMEMBERED);
        Index(out)
    }

    /// The same list without `id`, for a session whose file has been deleted.
    ///
    /// Left in, the note would outlive the transcript it describes and be read
    /// back for as long as the file holds it, which at [`REMEMBERED`] entries is
    /// a long time.
    pub fn minus(&self, id: &str) -> Index {
        Index(
            self.0
                .iter()
                .filter(|note| note.id != id)
                .cloned()
                .collect(),
        )
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.len()
    }
}

/// The file one session id names inside `dir`, or why that id is not one this
/// window will touch.
///
/// The only place a session id becomes a path. An id is a file name and nothing
/// else: it comes off a directory listing, but it also comes back through the
/// index file, which is plain text a hand can edit, so a name carrying a
/// separator or a step upwards is refused rather than joined. `dir.join(name)`
/// with a name like `../../.bashrc` in it is a delete outside the sessions
/// directory, and there is no reading of that which is somebody's intent.
pub fn session_file(dir: &Path, id: &str) -> Result<PathBuf, String> {
    let plain = !id.is_empty()
        && !id.starts_with('.')
        && !id.contains(['/', '\\', '\0'])
        && !id.contains("..");
    if !plain {
        return Err(format!("{id:?} is not a session name"));
    }
    let path = dir.join(format!("{id}.jsonl"));
    // Belt and braces: whatever the name was, the file has to be a direct child
    // of the directory the sessions live in.
    match path.parent() == Some(dir) {
        true => Ok(path),
        false => Err(format!("{id:?} is not in the sessions directory")),
    }
}

/// Delete one session's transcript.
///
/// The only thing in this window that destroys anything. It takes the directory
/// as an argument rather than reading [`dir`] itself, so what it is allowed to
/// touch is decided by the caller and can be a temp directory in a test.
pub fn forget(dir: &Path, id: &str) -> Result<(), String> {
    let path = session_file(dir, id)?;
    std::fs::remove_file(&path).map_err(|e| e.to_string())
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

/// One entry per line, `<id> <folder>` or `<id> ctx=<used>/<total> <folder>`,
/// `#` a comment.
///
/// The id is split off at the first space because it cannot contain one and a
/// path can: splitting the other way round would break on the first folder with
/// a space in its name. The context reading goes between the two, behind a
/// marker, for the same reason: a path is whatever is left of the line, so
/// anything optional has to sit in front of it and say what it is. A line
/// written before the reading existed has no marker and still reads.
fn parse_index(text: &str) -> Index {
    let mut out: Vec<Note> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((id, rest)) = line.split_once(' ') else {
            continue;
        };
        let (context, rest) = match rest.trim_start().split_once(' ') {
            Some((first, tail)) if first.starts_with(CONTEXT_MARK) => (parse_context(first), tail),
            _ => (None, rest),
        };
        let path = rest.trim();
        if id.is_empty() || path.is_empty() || out.iter().any(|note| note.id == id) {
            continue;
        }
        out.push(Note {
            id: id.to_string(),
            workspace: PathBuf::from(path),
            context,
        });
    }
    out.truncate(REMEMBERED);
    Index(out)
}

/// What the context reading is written behind, so the path can go on being
/// whatever is left of the line.
const CONTEXT_MARK: &str = "ctx=";

/// `ctx=48000/200000` back into two numbers. Anything else is a line somebody
/// edited by hand into something this cannot read, which is a session with no
/// reading rather than a file that fails to load.
fn parse_context(word: &str) -> Option<Context> {
    let (used, total) = word.strip_prefix(CONTEXT_MARK)?.split_once('/')?;
    Some(Context {
        used: used.parse().ok()?,
        total: total.parse().ok()?,
    })
}

fn index_text(index: &Index) -> String {
    let mut out = String::from(
        "# Which folder each noob session was started in, newest first, and how\n\
         # full its context window was when NO0B last watched it run.\n\
         # Written by NO0B so a saved session can be resumed where it belongs.\n",
    );
    for note in index.0.iter().take(REMEMBERED) {
        out.push_str(&note.id);
        out.push(' ');
        if let Some(context) = note.context {
            out.push_str(&format!("{CONTEXT_MARK}{}/{} ", context.used, context.total));
        }
        out.push_str(&note.workspace.display().to_string());
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
        // The size comes off the same stat the age does, so every row has one.
        assert_eq!(
            whole.bytes,
            std::fs::metadata(dir.join("whole.jsonl")).unwrap().len()
        );
        assert!(whole.bytes > 0);
        assert_eq!(
            whole.context, None,
            "nothing watched this one run, so there is no reading to show"
        );

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

        // A session whose file has gone takes its line with it.
        let fewer = index.minus("one");
        assert_eq!(fewer.folder_of("one"), None);
        assert_eq!(fewer.len(), 1, "and nothing else moved");
        assert_eq!(fewer.folder_of("two"), index.folder_of("two"));
        assert_eq!(
            index.minus("nobody").len(),
            2,
            "forgetting one that was never there changes nothing"
        );

        let mut long = Index::default();
        for n in 0..REMEMBERED + 10 {
            long = long.plus(&format!("s{n}"), Path::new("/p"));
        }
        assert_eq!(long.len(), REMEMBERED);
        assert_eq!(parse_index(&index_text(&long)).len(), REMEMBERED);
    }

    /// The window has to look where the agent writes: `sessions/` inside the
    /// agent's own config directory, wherever that resolves to. Which directory
    /// that is is the agent's rule and is checked where it lives, in
    /// [`crate::agent`].
    #[test]
    fn the_sessions_are_read_from_the_agents_own_config_directory() {
        assert_eq!(
            dir_in(Path::new("/somewhere/noob")),
            PathBuf::from("/somewhere/noob/sessions")
        );
        assert_eq!(
            dir_in(Path::new("/home/hec/.config/noob")),
            PathBuf::from("/home/hec/.config/noob/sessions")
        );
    }

    /// The context reading rides on the same note the folder does, because it
    /// is the same kind of thing: something only this window knows, which the
    /// transcript the agent writes has no room for.
    ///
    /// A line written before the reading existed still reads, and a session the
    /// CLI wrote on its own has no line at all, so both come back as nothing
    /// rather than as a number nobody measured.
    #[test]
    fn the_note_carries_how_full_the_context_window_was() {
        let full = Context {
            used: 48_000,
            total: 200_000,
        };
        let index = Index::default()
            .plus("folder-only", Path::new("/home/hec/one"))
            .plus_context("watched", Path::new("/home/hec/two"), full);
        assert_eq!(index.context_of("watched"), Some(full));
        assert_eq!(index.context_of("folder-only"), None);
        assert_eq!(index.context_of("never-seen"), None);
        assert_eq!(full.percent(), Some(24));
        assert_eq!(Context { used: 1, total: 0 }.percent(), None);
        assert_eq!(
            Context {
                used: 9,
                total: 4
            }
            .percent(),
            Some(100),
            "a reading past the end of the window is a full window"
        );

        // It survives the file, beside a path with a space in it.
        let spaced = index.plus_context(
            "spaced",
            Path::new("/home/hec/a folder with spaces"),
            Context {
                used: 7,
                total: 10,
            },
        );
        assert_eq!(parse_index(&index_text(&spaced)), spaced);
        assert_eq!(
            spaced.folder_of("spaced"),
            Some(PathBuf::from("/home/hec/a folder with spaces"))
        );

        // Starting the same session again keeps the reading: `plus` is called
        // on every SessionStart, and a resume would otherwise erase it.
        let again = index.plus("watched", Path::new("/home/hec/two"));
        assert_eq!(again.context_of("watched"), Some(full));

        // A line from before this existed, and one somebody edited into
        // something unreadable.
        let old = parse_index("old /home/hec/before\nbroken ctx=lots /home/hec/x\n");
        assert_eq!(old.folder_of("old"), Some(PathBuf::from("/home/hec/before")));
        assert_eq!(old.context_of("old"), None);
        assert_eq!(
            old.folder_of("broken"),
            Some(PathBuf::from("/home/hec/x")),
            "an unreadable reading is not an unreadable line"
        );
        assert_eq!(old.context_of("broken"), None);
    }

    /// Deleting a session, which is the one thing in this window that destroys
    /// anything: the file goes, and nothing outside the sessions directory can
    /// be named by an id however that id got into the file.
    #[test]
    fn a_session_is_deleted_by_name_and_only_inside_its_own_directory() {
        let dir = temp("delete");
        let outside = dir.join("outside.jsonl");
        std::fs::write(&outside, "not a session").expect("a file to try to reach");
        let inside = dir.join("sessions");
        std::fs::create_dir_all(&inside).expect("a sessions dir");
        write(&inside, "keep", &good("keep", "one"));
        write(&inside, "drop", &good("drop", "two"));

        // The guard, before anything is removed. Every one of these is an id
        // that would reach a file the window was never pointed at.
        for id in [
            "",
            ".",
            "..",
            "../outside",
            "../../etc/passwd",
            "sub/one",
            "a\\b",
            ".hidden",
        ] {
            assert!(
                session_file(&inside, id).is_err(),
                "{id:?} was accepted as a session name"
            );
            assert!(forget(&inside, id).is_err(), "{id:?} was deleted");
        }
        assert!(outside.exists(), "a file outside the directory was deleted");

        assert_eq!(
            session_file(&inside, "drop"),
            Ok(inside.join("drop.jsonl"))
        );
        assert!(forget(&inside, "drop").is_ok());
        assert!(!inside.join("drop.jsonl").exists());
        assert!(inside.join("keep.jsonl").exists(), "it took the wrong one");
        assert_eq!(
            ids(&read(&inside, &Index::default(), &Real)),
            vec![String::from("keep")],
            "and the list is one shorter"
        );

        // A second delete of the same session is a file that is not there, not
        // a panic.
        assert!(forget(&inside, "drop").is_err());

        let _ = std::fs::remove_dir_all(&dir);
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

