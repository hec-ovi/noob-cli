//! Installing a skill from the window: a repository, a shorthand or a path in,
//! a directory under the agent's `skills/` out.
//!
//! The section could list skills, turn them on and off and delete them, and
//! install nothing at all. "on skills allow to install more by command as well"
//! is the last thing it could not do.
//!
//! ## Why the window does this itself
//!
//! The CLI installs skills, but only as the REPL slash command `/skills add`,
//! and it puts them in `<workspace>/.noob/skills`, which is the project's own
//! directory. This section lists `<config dir>/skills`, which is the one the
//! agent loads in every folder, so a run of the CLI's own installer would land a
//! skill somewhere this list does not look and read as an install that silently
//! did nothing. There is no `noob skills add` to run instead: the dispatch knows
//! exec, serve, tokens, debug, child, sessions and doctor, and answers anything
//! else with `unknown command`. Driving the REPL by piping a line into it starts
//! a session, writes a transcript into the list two sections up, prints its
//! failures to stdout and exits 0 whatever happened.
//!
//! So the window does the work, against the directory it lists, and mirrors the
//! CLI's rules rather than inventing its own: the same sources
//! ([`source_for`]), the same shallow clone, the same required front matter with
//! the same limits ([`front_matter`]), the same refusal of symlinks and special
//! files, and the same staging-then-rename so a failed install leaves nothing
//! half written. The one difference on purpose is where it lands and what a
//! duplicate is told to do about it: `/skills remove` is a command this window
//! does not have, and the uninstall button beside the row is.
//!
//! Everything here is a builder or a pure function over what a process answered,
//! the way [`crate::link`] is: the command line and the reading of a clone's
//! output are both checked without a network, a repository or a child process.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long a clone is given before it is killed, in seconds. The CLI's own
/// number, so a repository that is too slow for one is too slow for both.
pub const CLONE_SECONDS: u64 = 120;

/// How much of what `git` said is kept. Enough for the several lines a refused
/// clone writes, and not a repository's worth of output.
const GIT_SAID_CAP: usize = 8 * 1024;

/// The most a `SKILL.md`'s front matter is read at: the block is a handful of
/// keys and a file whose first 64 KiB carry no closing `---` has no front matter
/// anybody meant.
const FRONT_MATTER_CAP: usize = 64 * 1024;

/// The limits the standard puts on the two required keys, and the CLI enforces.
const NAME_CHARS: usize = 64;
const ABOUT_CHARS: usize = 1024;

/// What was typed into the field: something to clone, or something on this
/// machine to copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A repository, as the URL `git clone` is handed.
    Git(String),
    /// A directory holding a `SKILL.md`, or a bare `SKILL.md`.
    Local(PathBuf),
}

/// What a typed source means.
///
/// The CLI's own rule, so the same string means the same thing in both: an
/// explicit URL passes through, `owner/repo` expands to GitHub, and everything
/// else is a path. `here` is whether something of that name is already on the
/// disk, and it wins: a directory really called `owner/repo` is that directory
/// and not a repository on the internet.
pub fn source_for(text: &str, here: bool) -> Source {
    let text = text.trim();
    let git = text.starts_with("http://")
        || text.starts_with("https://")
        || text.starts_with("git@")
        || text.ends_with(".git");
    if git {
        return Source::Git(String::from(text));
    }
    if here {
        return Source::Local(PathBuf::from(text));
    }
    // Exactly one slash, both halves in the alphabet GitHub allows, and neither
    // starting with a dot, so `./somewhere` and a hidden path are never read as
    // a repository.
    let named = |part: &str| {
        !part.is_empty()
            && !part.starts_with('.')
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    };
    match text.split_once('/') {
        Some((owner, repo)) if named(owner) && named(repo) && !repo.contains('/') => {
            Source::Git(format!("https://github.com/{owner}/{repo}.git"))
        }
        _ => Source::Local(PathBuf::from(text)),
    }
}

/// What the typed source would install, without installing it: the validate
/// half of the card's cycle. Ok is the plain sentence the panel shows before
/// the button turns into install; Err is why nothing would.
///
/// A repository is answered from its address alone, since cloning it IS the
/// install; a local path is answered from the disk, with the same
/// one-skill-only rule the install applies.
pub fn check(source: &str) -> Result<String, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err(String::from(
            "type a repository, an owner/name or a path first",
        ));
    }
    let here = Path::new(source).exists();
    match source_for(source, here) {
        Source::Git(url) => Ok(format!("a repository: install clones {url}")),
        Source::Local(path) if !path.exists() => Err(format!(
            "{} is not on this machine, and does not read as a repository",
            path.display()
        )),
        Source::Local(path) => {
            let found = skill_in(&path)?;
            Ok(format!("a local skill at {}", found.display()))
        }
    }
}

/// Exactly the clone the window runs, built without running it.
///
/// Its own function for the reason [`crate::link::prompt_command`] is one: what
/// the window asks a process to do is worth asserting without a repository and a
/// network behind it. Shallow, quiet, and with `--` in front of the URL so a
/// source beginning with a hyphen is a URL rather than a flag. The terminal
/// prompt is turned off, since a clone that stops to ask for a password would
/// hang a thread nobody can see with nothing on screen saying why.
pub fn clone_command(url: &str, into: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .args(["clone", "--quiet", "--depth", "1", "--", url])
        .arg(into)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

/// What the panel makes of a finished clone.
///
/// The other half of the seam: `ok` is whether git exited cleanly and `said` is
/// what it wrote to its error stream, and a refusal is reported as git wrote it.
/// A wrong URL, a repository nobody may read and a machine with no network all
/// say different things there, and the panel showing "install failed" for the
/// three of them is the panel knowing which one and not saying.
pub fn cloned(ok: bool, said: &[u8]) -> Result<(), String> {
    if ok {
        return Ok(());
    }
    let said = String::from_utf8_lossy(said);
    let why: Vec<&str> = said
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    Err(match why.is_empty() {
        true => String::from("git clone failed and said nothing"),
        false => format!("git clone failed: {}", why.join("; ")),
    })
}

/// The name and the description a `SKILL.md` has to carry, out of its front
/// matter, or why it is not a skill.
///
/// The CLI's own reading and the CLI's own limits, because the CLI is what
/// refuses to load the thing afterwards: a window that accepted a skill the
/// agent then skips would be installing something that does nothing, with
/// nothing anywhere saying so. Plain scalars and the folded and literal blocks,
/// which is what the skills in the wild are written with.
pub fn front_matter(text: &str) -> Result<(String, String), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(String::from(
            "no front matter: a SKILL.md starts with a --- line",
        ));
    }
    let (mut name, mut about) = (None, None);
    let mut closed = false;
    let mut folding: Option<&mut Option<String>> = None;
    for line in lines {
        let line = line.trim_end();
        if line == "---" {
            closed = true;
            break;
        }
        // An indented line under a block scalar is more of that value.
        if line.starts_with(char::is_whitespace) {
            if let Some(slot) = folding.as_deref_mut() {
                let piece = line.trim();
                if !piece.is_empty() {
                    match slot {
                        Some(held) => {
                            held.push(' ');
                            held.push_str(piece);
                        }
                        None => *slot = Some(String::from(piece)),
                    }
                }
            }
            continue;
        }
        folding = None;
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let slot = match key.trim() {
            "name" => &mut name,
            "description" => &mut about,
            _ => continue,
        };
        let value = value.trim();
        if value.is_empty() || value.starts_with('|') || value.starts_with('>') {
            folding = Some(slot);
            continue;
        }
        *slot = Some(String::from(value.trim_matches(['"', '\''])));
    }
    if !closed {
        return Err(String::from(
            "unterminated front matter: no closing --- line",
        ));
    }
    let name = name.filter(|it: &String| !it.is_empty()).ok_or(
        "the front matter has no name: a skill is named by the name key, not by its folder",
    )?;
    if name.chars().count() > NAME_CHARS {
        return Err(format!(
            "the name is {} characters and the most is {NAME_CHARS}",
            name.chars().count()
        ));
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!(
            "the name {name:?} has to be lower case letters, digits and hyphens"
        ));
    }
    let about = about
        .filter(|it: &String| !it.is_empty())
        .ok_or("the front matter has no description: it is what the agent reads to pick a skill")?;
    if about.chars().count() > ABOUT_CHARS {
        return Err(format!(
            "the description is {} characters and the most is {ABOUT_CHARS}",
            about.chars().count()
        ));
    }
    Ok((name, about))
}

/// The skill inside a cloned repository: the root when it holds a `SKILL.md`,
/// or the one directory under it that does.
///
/// Several is refused rather than guessed at, the same way the CLI refuses it:
/// installing one of four skills nobody chose is worse than saying which four
/// there were.
pub fn skill_in(root: &Path) -> Result<PathBuf, String> {
    if root.join("SKILL.md").is_file() {
        return Ok(root.to_path_buf());
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir(root)
        .map_err(|e| format!("cannot read what was cloned: {e}"))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("SKILL.md").is_file())
        .collect();
    found.sort();
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(String::from(
            "there is no SKILL.md in that repository, at its root or one directory down",
        )),
        many => Err(format!(
            "that repository holds {many} skills: point this at the one directory you want"
        )),
    }
}

/// Install one skill into the directory the agent loads from, and answer with
/// the name it landed under.
///
/// Runs on a thread of its own, because a clone is given two minutes and the
/// window is one process: this is never called on the interface thread.
pub fn install(source: &str, skills_at: &Path) -> Result<String, String> {
    let source = source.trim();
    if source.is_empty() {
        return Err(String::from(
            "type a repository, an owner/name or a path first",
        ));
    }
    match source_for(source, Path::new(source).exists()) {
        Source::Git(url) => from_git(&url, skills_at),
        Source::Local(path) => from_path(&path, skills_at),
    }
}

/// Clone into a hidden directory beside the skills, take the skill out of it and
/// throw the clone away.
///
/// Hidden and beside them rather than in a temporary directory somewhere else,
/// so the publish is a rename inside one filesystem and can never leave half a
/// skill where the agent looks. The reader skips a dotted directory, so a clone
/// that is still running is not a skill on the list.
fn from_git(url: &str, skills_at: &Path) -> Result<String, String> {
    std::fs::create_dir_all(skills_at)
        .map_err(|e| format!("cannot make {}: {e}", skills_at.display()))?;
    let staging = staging(skills_at);
    let _ = std::fs::remove_dir_all(&staging);
    let mut child = clone_command(url, &staging)
        .spawn()
        .map_err(|e| format!("cannot run git (is it installed?): {e}"))?;
    let said = child.stderr.take().map(|stream| {
        std::thread::spawn(move || {
            let mut kept = Vec::new();
            let _ = stream.take(GIT_SAID_CAP as u64).read_to_end(&mut kept);
            kept
        })
    });
    let by = Instant::now() + Duration::from_secs(CLONE_SECONDS);
    let ended = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status.success()),
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                break Err(format!("cannot wait for git: {e}"));
            }
        }
        if Instant::now() >= by {
            let _ = child.kill();
            let _ = child.wait();
            break Err(format!(
                "git clone gave up after {CLONE_SECONDS}s: check the address and the network"
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let said = said.and_then(|it| it.join().ok()).unwrap_or_default();
    let out = ended
        .and_then(|ok| cloned(ok, &said))
        .and_then(|()| skill_in(&staging))
        .and_then(|dir| from_path(&dir, skills_at));
    let _ = std::fs::remove_dir_all(&staging);
    out
}

/// Copy a skill that is already on this machine.
fn from_path(source: &Path, skills_at: &Path) -> Result<String, String> {
    let kind = std::fs::symlink_metadata(source)
        .map_err(|e| format!("cannot read {}: {e}", source.display()))?
        .file_type();
    if kind.is_symlink() {
        return Err(format!("{} is a link, not a skill", source.display()));
    }
    let (dir, file) = match kind.is_dir() {
        true => (Some(source.to_path_buf()), source.join("SKILL.md")),
        false if source.file_name() == Some(std::ffi::OsStr::new("SKILL.md")) => {
            (None, source.to_path_buf())
        }
        false => {
            return Err(format!(
                "{} is neither a directory with a SKILL.md in it nor a SKILL.md",
                source.display()
            ));
        }
    };
    let text = read_head(&file)?;
    // Read and refused before anything at all is written, so a skill that is not
    // one leaves nothing behind to clean up.
    let (name, _) = front_matter(&text)?;
    already_there(skills_at, &name)?;
    let staging = staging(skills_at);
    let _ = std::fs::remove_dir_all(&staging);
    let copied = match &dir {
        Some(dir) => copy_dir(dir, &staging, &staging),
        None => std::fs::create_dir_all(&staging).and_then(|()| {
            std::fs::copy(&file, staging.join("SKILL.md")).map(|_| ())
        }),
    };
    if let Err(e) = copied {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("copying the skill failed: {e}"));
    }
    // Checked again with the copy standing ready, because a clone takes minutes
    // and another window may have installed the same name while it ran.
    if let Err(why) = already_there(skills_at, &name) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(why);
    }
    if let Err(e) = std::fs::rename(&staging, skills_at.join(&name)) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("cannot put the skill in place: {e}"));
    }
    Ok(name)
}

/// Whether a skill of that name is already installed, on either side of the
/// on-and-off move.
///
/// Both, because they are one name: a skill turned off is still installed, and
/// an install that landed on top of it would leave two directories the reader
/// lists as two rows with one name.
fn already_there(skills_at: &Path, name: &str) -> Result<(), String> {
    if skills_at.join(name).exists() {
        return Err(format!(
            "{name} is already installed: uninstall it below before installing it again"
        ));
    }
    if crate::agent::skills_off(skills_at).join(name).exists() {
        return Err(format!(
            "{name} is already installed and turned off: turn it on, or uninstall it below"
        ));
    }
    Ok(())
}

/// The hidden directory a skill is built in before it is published.
///
/// Named after this process, so two windows never build in one place, and
/// removed on every path out of an install. What a crash in the middle leaves is
/// one dotted directory beside the skills, which the reader does not list and
/// the next install of this window removes.
fn staging(skills_at: &Path) -> PathBuf {
    skills_at.join(format!(".installing-{}", std::process::id()))
}

/// The front of a file, as text.
///
/// Opened through its own metadata first: a `SKILL.md` that is a link, a device
/// or a pipe is refused rather than read, which is the CLI's own rule and the
/// reason is the same. Reading a pipe blocks forever.
fn read_head(file: &Path) -> Result<String, String> {
    let kind = std::fs::symlink_metadata(file)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?
        .file_type();
    if !kind.is_file() {
        return Err(format!(
            "{} has to be a plain file, not a link or a device",
            file.display()
        ));
    }
    let handle =
        std::fs::File::open(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    let mut kept = Vec::new();
    handle
        .take(FRONT_MATTER_CAP as u64)
        .read_to_end(&mut kept)
        .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    Ok(String::from_utf8_lossy(&kept).into_owned())
}

/// Copy a directory's plain files and its subdirectories, and nothing else.
///
/// No links, no devices, no `.git`, and never the staging directory itself,
/// which is what a skill installed from the folder it is being built in would
/// otherwise copy into itself forever.
fn copy_dir(from: &Path, to: &Path, avoid: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        if path.starts_with(avoid) || entry.file_name() == std::ffi::OsStr::new(".git") {
            continue;
        }
        let kind = entry.file_type()?;
        let landing = to.join(entry.file_name());
        if kind.is_dir() {
            copy_dir(&path, &landing, avoid)?;
        } else if kind.is_file() {
            std::fs::copy(&path, &landing)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "no0b-install-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp dir");
        dir
    }

    fn a_skill(at: &Path, front: &str) -> PathBuf {
        std::fs::create_dir_all(at).expect("a skill directory");
        std::fs::write(at.join("SKILL.md"), front).expect("a SKILL.md");
        at.to_path_buf()
    }

    const GOOD: &str = "---\nname: coding\ndescription: how this repo is built\n---\n\n# coding\n";

    /// The validate half answers without installing: an empty or missing
    /// source is refused with why, a shorthand names the repository it would
    /// clone, and a real local skill checks out.
    #[test]
    fn the_check_answers_before_anything_installs() {
        assert!(check("").is_err(), "an empty source validated");
        assert!(check("   ").is_err());
        let repo = check("someone/writing").expect("a shorthand is checkable");
        assert!(repo.contains("https://github.com/someone/writing.git"), "{repo}");
        let missing = check("/nowhere/of/the/kind").expect_err("a missing path validated");
        assert!(missing.contains("not on this machine"), "{missing}");
        let dir = a_skill(&temp("check"), GOOD);
        let local = check(dir.to_str().expect("utf8")).expect("a real skill checks out");
        assert!(local.contains("a local skill"), "{local}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What is typed decides what happens, and it decides it the way the CLI
    /// decides it: the same string typed into either has to mean the same skill,
    /// or the two disagree about what `owner/repo` is.
    #[test]
    fn a_source_is_a_repository_a_shorthand_or_a_path() {
        assert_eq!(
            source_for("https://github.com/someone/coding", false),
            Source::Git(String::from("https://github.com/someone/coding"))
        );
        assert_eq!(
            source_for("git@github.com:someone/coding.git", false),
            Source::Git(String::from("git@github.com:someone/coding.git"))
        );
        assert_eq!(
            source_for("someone/coding", false),
            Source::Git(String::from("https://github.com/someone/coding.git")),
            "the shorthand every registry uses has to expand"
        );
        // A real directory of that name wins: the disk is what is there.
        assert_eq!(
            source_for("someone/coding", true),
            Source::Local(PathBuf::from("someone/coding"))
        );
        assert_eq!(
            source_for("./skills/coding", false),
            Source::Local(PathBuf::from("./skills/coding")),
            "a path with a dot in front is not a repository"
        );
        assert_eq!(
            source_for("/home/hec/skills/coding", false),
            Source::Local(PathBuf::from("/home/hec/skills/coding"))
        );
    }

    /// The command line the clone runs with, asserted without a network: what
    /// the window asks git to do is what decides whether an install can hang the
    /// thread it is on.
    #[test]
    fn the_clone_is_shallow_quiet_and_never_asks_for_a_password() {
        let into = std::env::temp_dir().join("no0b-clone-here");
        let command = clone_command("https://github.com/someone/coding.git", &into);
        let args: Vec<&str> = command
            .get_args()
            .map(|arg| arg.to_str().expect("a flag is text"))
            .collect();
        assert_eq!(
            args,
            [
                "clone",
                "--quiet",
                "--depth",
                "1",
                "--",
                "https://github.com/someone/coding.git",
                into.to_str().expect("a path"),
            ],
            "the -- in front of the URL is what stops a source reading as a flag"
        );
        assert_eq!(command.get_program(), "git");
        let asked: Vec<(&str, Option<&str>)> = command
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_str().expect("a name is text"),
                    value.map(|it| it.to_str().expect("a value is text")),
                )
            })
            .collect();
        assert_eq!(asked, [("GIT_TERMINAL_PROMPT", Some("0"))]);
    }

    /// Every way a clone can end, as the line the panel shows.
    #[test]
    fn a_refused_clone_says_what_git_said() {
        assert!(cloned(true, b"").is_ok());
        let why = cloned(
            false,
            b"fatal: repository 'https://github.com/someone/nothing/' not found\n",
        )
        .expect_err("it failed");
        assert!(why.contains("not found"), "{why}");
        assert!(why.starts_with("git clone failed"), "{why}");
        let quiet = cloned(false, b"  \n\n").expect_err("it failed");
        assert!(quiet.contains("said nothing"), "{quiet}");
    }

    /// The front matter is the skill: no name, no description, a name nothing
    /// can load, or no block at all, each said as itself.
    #[test]
    fn front_matter_is_read_the_way_the_agent_reads_it() {
        let (name, about) = front_matter(GOOD).expect("a skill");
        assert_eq!(name, "coding");
        assert_eq!(about, "how this repo is built");

        // A folded description is one of the two shapes skills are written in.
        let folded = front_matter("---\nname: coding\ndescription: >\n  the first half\n  and the second\n---\n")
            .expect("a skill");
        assert_eq!(folded.1, "the first half and the second");

        for (text, says) in [
            ("# coding\n", "no front matter"),
            ("---\nname: coding\n", "unterminated"),
            ("---\ndescription: words\n---\n", "no name"),
            ("---\nname: coding\n---\n", "no description"),
            (
                "---\nname: Coding Skill\ndescription: words\n---\n",
                "lower case",
            ),
        ] {
            let why = front_matter(text).expect_err("not a skill");
            assert!(why.contains(says), "{text:?} answered {why:?}");
        }
        let long = format!(
            "---\nname: {}\ndescription: words\n---\n",
            "a".repeat(NAME_CHARS + 1)
        );
        assert!(front_matter(&long).expect_err("too long").contains("most"));
    }

    /// A skill on this machine is copied in under the name its front matter
    /// gives it, and it is the name the reader beside it lists.
    #[test]
    fn a_local_skill_lands_under_the_name_its_front_matter_gives_it() {
        let dir = temp("local");
        let skills = dir.join("skills");
        let from = a_skill(&dir.join("source"), GOOD);
        std::fs::write(from.join("notes.md"), "more of it").expect("a second file");
        std::fs::create_dir_all(from.join("bits")).expect("a subdirectory");
        std::fs::write(from.join("bits/one.txt"), "one").expect("a nested file");

        let name = install(from.to_str().expect("a path"), &skills).expect("it installs");
        assert_eq!(name, "coding", "the folder is named by the front matter");
        assert!(skills.join("coding/SKILL.md").is_file());
        assert!(skills.join("coding/notes.md").is_file(), "the rest of it too");
        assert!(skills.join("coding/bits/one.txt").is_file());
        assert!(
            !skills.join(format!(".installing-{}", std::process::id())).exists(),
            "the staging directory is left behind"
        );
        // And the window's own reader sees exactly one skill, which is what the
        // list is rebuilt from.
        let read = crate::agent::read_skills(&skills, true);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].name, "coding");
        assert_eq!(read[0].about, "how this repo is built");

        // Twice is refused, and refused by name, with the button that undoes it
        // named rather than a command this window does not have.
        let again = install(from.to_str().expect("a path"), &skills).expect_err("already there");
        assert!(again.contains("already installed"), "{again}");
        assert!(again.contains("uninstall"), "{again}");
        assert!(!again.contains("/skills"), "{again}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A source that is not a skill writes nothing at all, and says which of the
    /// things that can be wrong with it is.
    #[test]
    fn a_source_that_is_not_a_skill_is_refused_without_writing_anything() {
        let dir = temp("refused");
        let skills = dir.join("skills");
        let bad = a_skill(&dir.join("bad"), "---\nname: coding\n---\n");
        let why = install(bad.to_str().expect("a path"), &skills).expect_err("no description");
        assert!(why.contains("no description"), "{why}");
        assert!(
            !skills.join("coding").exists(),
            "a refused skill left a directory behind"
        );

        let missing = dir.join("nowhere");
        let why = install(missing.to_str().expect("a path"), &skills).expect_err("not there");
        assert!(why.contains("cannot read"), "{why}");

        let plain = dir.join("notes.md");
        std::fs::write(&plain, "not a skill").expect("a file");
        let why = install(plain.to_str().expect("a path"), &skills).expect_err("not a skill");
        assert!(why.contains("neither a directory"), "{why}");

        let why = install("   ", &skills).expect_err("nothing typed");
        assert!(why.contains("type a repository"), "{why}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A skill already there under the off sibling is still installed. Landing a
    /// second copy in the live directory would put two rows with one name on the
    /// list and leave the turn-on move with nowhere to go.
    #[test]
    fn a_skill_that_is_turned_off_still_counts_as_installed() {
        let dir = temp("off");
        let skills = dir.join("skills");
        std::fs::create_dir_all(&skills).expect("a skills directory");
        let from = a_skill(&dir.join("source"), GOOD);
        a_skill(&crate::agent::skills_off(&skills).join("coding"), GOOD);

        let why = install(from.to_str().expect("a path"), &skills).expect_err("already there");
        assert!(why.contains("turned off"), "{why}");
        assert!(!skills.join("coding").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Which directory of a clone is the skill, including the two answers that
    /// are refusals rather than guesses.
    #[test]
    fn the_skill_in_a_repository_is_found_or_said_to_be_missing() {
        let dir = temp("inside");
        let root = dir.join("clone");
        std::fs::create_dir_all(&root).expect("a clone");
        let why = skill_in(&root).expect_err("nothing in it");
        assert!(why.contains("no SKILL.md"), "{why}");

        a_skill(&root.join("coding"), GOOD);
        assert_eq!(skill_in(&root).expect("one skill"), root.join("coding"));

        a_skill(&root.join("writing"), GOOD);
        let why = skill_in(&root).expect_err("two of them");
        assert!(why.contains("2 skills"), "{why}");

        // A repository whose own root is the skill is the skill.
        a_skill(&root, GOOD);
        assert_eq!(skill_in(&root).expect("the root"), root);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
