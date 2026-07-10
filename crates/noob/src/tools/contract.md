# noob/src/tools

The built-in tool set: read, write, edit, bash, grep, glob, ls, plus skill
(registered only when discovery found at least one skill) and the registry
(`specs()` + `skill::spec()`, byte-stable for the session). Each tool is a
pure fn(ctx, args) -> ToolOutcome with no knowledge of the agent loop.

Key rules:
- read returns plain text with NO line numbers (number prefixes contaminate
  small-model edit strings); the header states the page and total for
  offset/limit paging.
- edit is exact string replace with a deterministic ladder (A trailing
  whitespace, B typographic fold, C uniform indent shift), hard ambiguity
  rejection at every stage, and teaching errors that escalate to the real
  file region on repeat failures. No similarity-score fuzzing, ever.
- write/edit sit behind check-and-set (`guard.rs`: fnv1a64 stamps recorded
  by read, verified before mutation) and the workspace sandbox path policy
  (symlink escapes rejected); writes are atomic (temp + fsync + rename,
  mode preserved).
- bash merges stdout/stderr at the fd level, runs in its own process group,
  and is killed as a group on timeout (default 120s, max 600s).
- grep/glob are gitignore-aware (ignore crate); ls is not (explicit listing).
- skill returns the SKILL.md body (frontmatter stripped, byte-exact) plus
  the skill's directory, capped at 24 KiB with a read pointer; oversize
  bodies raise a UI-only warning citing the standard's ~5k-token
  recommendation; loads are tracked in `ToolCtx.loaded_skills` for the
  post-compaction re-listing.
- Every result is truncated ONCE at emission (`truncate.rs`, caps and
  marker phrasing frozen in golden tests; every marker names the next
  action), then byte-frozen in the transcript.
