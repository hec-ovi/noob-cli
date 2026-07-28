//! Turning a path the model *meant* into one that exists.
//!
//! A wrong path used to return an error, and an error is not a correction: the
//! model has nothing to act on except its own prior belief, so it repeats it.
//! A real session (`19fa5b9d7b9-1-0-dec6b89f`) spent 67 tool calls over 45
//! rounds against a directory it kept spelling `simulacrum` instead of
//! `simulacrium`, and its very first grep had already printed the correct name.
//! Search was not the problem. Failing instead of correcting was.
//!
//! So a near miss with exactly one candidate resolves and says so. The call
//! succeeds, the model reads the real name in the result, and there is no
//! second attempt to intercept. Several candidates return the closed list,
//! which is at least actionable; none returns what is actually in the
//! directory.
//!
//! Read-side tools only. `write` may legitimately name a file that does not
//! exist yet, so silently redirecting it to a similarly named neighbor would
//! turn a new file into a clobbered one.

use std::path::{Component, Path, PathBuf};

use super::guard::resolve_path;

/// Components this will fix in one path. A single typo is a typo; three of
/// them is a different path, and guessing that far is how a tool corrupts
/// something quietly.
const MAX_CORRECTIONS: usize = 2;
/// Entries scanned per directory. A generated tree can hold a lot of siblings
/// and this runs on the failure path of every read.
const MAX_ENTRIES: usize = 4_096;
/// Names listed back when nothing matches.
const MAX_LISTED: usize = 24;

#[derive(Debug)]
pub struct Found {
    pub path: PathBuf,
    /// Present when the path was corrected. Belongs at the top of the tool
    /// result: it is how the model learns the real name.
    pub note: Option<String>,
}

/// Resolve `raw` to a path that exists, fixing up to [`MAX_CORRECTIONS`]
/// mistyped components on the way.
pub fn resolve_existing(workspace: &Path, raw: &str) -> Result<Found, String> {
    let target = resolve_path(workspace, raw);
    if target.exists() {
        return Ok(Found {
            path: target,
            note: None,
        });
    }

    let mut current = PathBuf::new();
    let mut fixes: Vec<(String, String)> = Vec::new();
    for comp in target.components() {
        let Component::Normal(name) = comp else {
            current.push(comp);
            continue;
        };
        let direct = current.join(name);
        if direct.exists() {
            current = direct;
            continue;
        }
        let name = name.to_string_lossy();
        let mut matches = candidates(&current, &name);
        if matches.len() > 1 {
            matches.sort();
            matches.truncate(MAX_LISTED);
            return Err(format!(
                "{raw} does not exist; {} in {} could be meant: {}. Name the one you want.",
                matches.len(),
                display(&current, workspace),
                matches.join(", ")
            ));
        }
        let Some(hit) = matches.pop() else {
            return Err(missing(raw, &current, workspace));
        };
        if fixes.len() == MAX_CORRECTIONS {
            return Err(missing(raw, &current, workspace));
        }
        current = current.join(&hit);
        fixes.push((name.into_owned(), hit));
    }

    let note = fixes
        .iter()
        .map(|(wrong, right)| format!("{wrong} does not exist; used {right}"))
        .collect::<Vec<_>>()
        .join("; ");
    Ok(Found {
        path: current,
        note: Some(format!("note: {note}")),
    })
}

/// Names in `dir` that could be what `name` meant, best class first: a
/// case-only difference beats a punctuation difference beats a typo. Returning
/// only the best class keeps a near-tie from reading as ambiguity.
fn candidates(dir: &Path, name: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let lower = name.to_lowercase();
    let squashed = squash(name);
    let budget = budget(name);
    let (mut folded, mut punctuation, mut typo) = (Vec::new(), Vec::new(), Vec::new());
    for entry in entries.flatten().take(MAX_ENTRIES) {
        let found = entry.file_name().to_string_lossy().into_owned();
        let found_lower = found.to_lowercase();
        if found_lower == lower {
            folded.push(found);
        } else if !squashed.is_empty() && squash(&found) == squashed {
            punctuation.push(found);
        } else if budget > 0 && within(&found_lower, &lower, budget) {
            typo.push(found);
        }
    }
    if !folded.is_empty() {
        return folded;
    }
    if !punctuation.is_empty() {
        return punctuation;
    }
    typo
}

fn missing(raw: &str, deepest: &Path, workspace: &Path) -> String {
    let mut names: Vec<String> = std::fs::read_dir(deepest)
        .map(|entries| {
            entries
                .flatten()
                .take(MAX_ENTRIES)
                .map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if e.path().is_dir() {
                        format!("{name}/")
                    } else {
                        name
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        return format!(
            "{raw} does not exist; {} is empty",
            display(deepest, workspace)
        );
    }
    names.sort();
    let total = names.len();
    names.truncate(MAX_LISTED);
    let more = if total > MAX_LISTED {
        format!(" (+{} more)", total - MAX_LISTED)
    } else {
        String::new()
    };
    format!(
        "{raw} does not exist, and nothing in {} is close. That directory holds: {}{more}",
        display(deepest, workspace),
        names.join(", ")
    )
}

/// Workspace-relative when possible, so the model reads a path in the shape it
/// sends them.
fn display(path: &Path, workspace: &Path) -> String {
    match path.strip_prefix(workspace) {
        Ok(rest) if rest.as_os_str().is_empty() => ".".to_string(),
        Ok(rest) => rest.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

/// Lowercase alphanumerics only, so `my-file_v2` and `myFile.v2` agree.
fn squash(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Edits allowed for a name of this length. Short names get none: at three
/// characters almost everything is within one edit of everything else.
fn budget(name: &str) -> usize {
    match name.chars().count() {
        0..=4 => 0,
        5..=8 => 1,
        _ => 2,
    }
}

/// Is the edit distance between `a` and `b` at most `max`?
///
/// Stops as soon as the answer is no, so a long name against a directory of
/// long names does not pay for a full matrix per entry.
fn within(a: &str, b: &str, max: usize) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > max {
        return false;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut row = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        row[0] = i + 1;
        let mut best = row[0];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            row[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(row[j] + 1);
            best = best.min(row[j + 1]);
        }
        if best > max {
            return false;
        }
        std::mem::swap(&mut prev, &mut row);
    }
    prev[b.len()] <= max
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(entries: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for entry in entries {
            let path = dir.path().join(entry);
            if entry.ends_with('/') {
                std::fs::create_dir_all(&path).unwrap();
            } else {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, "x").unwrap();
            }
        }
        dir
    }

    /// The reported bug. One call, corrected, with the real name in the note
    /// so the model stops repeating the wrong one.
    #[test]
    fn the_simulacrum_typo_resolves_in_one_call() {
        let dir = tree(&["simulacrium/README.md"]);
        let found = resolve_existing(dir.path(), "simulacrum/README.md").unwrap();
        assert_eq!(found.path, dir.path().join("simulacrium/README.md"));
        let note = found.note.unwrap();
        assert!(note.contains("simulacrium"), "{note}");
        assert!(note.contains("simulacrum does not exist"), "{note}");
    }

    /// An exact hit stays silent. The note is a correction, not commentary.
    #[test]
    fn an_exact_path_is_not_annotated() {
        let dir = tree(&["src/app.py"]);
        let found = resolve_existing(dir.path(), "src/app.py").unwrap();
        assert!(found.note.is_none());
    }

    #[test]
    fn case_and_punctuation_differences_resolve() {
        let dir = tree(&["Cargo.toml", "my-file_v2.txt"]);
        assert_eq!(
            resolve_existing(dir.path(), "cargo.toml").unwrap().path,
            dir.path().join("Cargo.toml")
        );
        assert_eq!(
            resolve_existing(dir.path(), "myfilev2.txt").unwrap().path,
            dir.path().join("my-file_v2.txt")
        );
    }

    /// Two plausible targets are not a typo, they are a question. Guessing
    /// here is how a tool edits the wrong file.
    #[test]
    fn several_candidates_are_listed_instead_of_picked() {
        let dir = tree(&["handler_a.rs", "handler_b.rs"]);
        let error = resolve_existing(dir.path(), "handler_c.rs").unwrap_err();
        assert!(error.contains("handler_a.rs"), "{error}");
        assert!(error.contains("handler_b.rs"), "{error}");
        assert!(error.contains("Name the one you want"), "{error}");
    }

    /// A miss with nothing close says what is actually there, so the next call
    /// is informed rather than another guess.
    #[test]
    fn a_real_miss_lists_the_directory() {
        let dir = tree(&["alpha/", "beta.txt"]);
        let error = resolve_existing(dir.path(), "nowhere.txt").unwrap_err();
        assert!(error.contains("alpha/"), "{error}");
        assert!(error.contains("beta.txt"), "{error}");
    }

    /// Short names are left alone: at four characters a one-edit budget
    /// matches half the directory.
    #[test]
    fn short_names_are_not_guessed_at() {
        let dir = tree(&["main.rs", "lib.rs"]);
        assert!(resolve_existing(dir.path(), "mod.rs").is_err());
        assert_eq!(budget("abcd"), 0);
        assert_eq!(budget("abcdef"), 1);
        assert_eq!(budget("abcdefghi"), 2);
    }

    /// A path that needs more fixing than this is not a typo, and correcting
    /// it would be inventing a destination.
    #[test]
    fn correction_stops_after_two_components() {
        let dir = tree(&["alphaa/betaa/gammaa/file.txt"]);
        assert!(
            resolve_existing(dir.path(), "alphab/betab/gammab/file.txt").is_err(),
            "three corrected components must not resolve"
        );
        let found = resolve_existing(dir.path(), "alphab/betab/gammaa/file.txt").unwrap();
        assert_eq!(found.path, dir.path().join("alphaa/betaa/gammaa/file.txt"));
    }

    #[test]
    fn edit_distance_stops_early_and_stays_correct() {
        assert!(within("simulacrium", "simulacrum", 2));
        assert!(within("simulacrium", "simulacrum", 1));
        assert!(!within("simulacrium", "simulacrum", 0));
        assert!(!within("handler", "completely-different", 2));
        assert!(within("abc", "abc", 0));
    }
}
