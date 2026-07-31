# session

contractVersion: 1.0.0

## Purpose

Append-only JSONL transcripts under `<config>/sessions/`, replayed on resume
so the next request byte-extends exactly what was sent before, with repair
for transcripts torn mid-batch.

## Public surface

```rust
pub struct Session;
impl Session {
    pub fn open(config_dir: &Path, id: Option<&str>)
        -> Result<(Session, Vec<Item>, bool, ReplayReport), String>;
        // Some(id): resume that file (created if absent); None: mint a fresh
        // id, claimed create_new so two processes can never share one file.
        // Returns the replayed items, whether the file existed, and the
        // replay report. Healing runs here and is persisted (see invariants)
    pub fn list(config_dir: &Path) -> Result<Vec<SessionInfo>, String>;
    pub fn latest_id(config_dir: &Path) -> Result<Option<String>, String>;
    pub fn id(&self) -> &str;
    pub fn path(&self) -> &Path;
    pub fn log_item(&mut self, item: &Item) -> Result<(), String>;
    pub fn log_reset(&mut self, items: &[Item]) -> Result<(), String>;
        // compaction: the full new transcript in one record
    pub fn log_usage(&mut self, u: Usage) -> Result<(), String>;
        // one request's cost; a zero-cost request writes nothing
    pub fn tokens(&self) -> SessionTokens;   // totals incl. replayed lines
}

pub struct SessionTokens { pub prefilled: u64, pub generated: u64 }
impl SessionTokens { pub fn add(&mut self, u: Usage); pub fn is_zero(&self) -> bool }
    // prefilled = prompt minus cached prompt: what the server actually
    // computed, so the sum stays meaningful as the transcript grows

pub struct SessionInfo;      // id and file size, listed newest first
pub struct ReplayReport;
impl ReplayReport { pub fn warning(&self) -> Option<String> }
    // skipped-line and repair counts, one printable warning
```

## Line shapes

One JSON object per line, flushed per line. The writer's exact shapes are
[`schema/meta.json`](schema/meta.json), [`schema/item.json`](schema/item.json),
[`schema/reset.json`](schema/reset.json), [`schema/usage.json`](schema/usage.json),
with item payloads in
[`schema/transcript-item.json`](schema/transcript-item.json).
`tests/contract.py` validates the fixtures both ways, and the inline test
`the_committed_fixtures_match_the_writer` pins the fixtures to the real
encoder.

## Errors

Every failure is a `String` naming the path and the io error: cannot create
the sessions directory, cannot read/open/initialize/append a session file. A
rejected id (not 1..=64 chars of `[A-Za-z0-9_-]`, or the reserved `latest`)
names the rule. When persisting the resume-time repair fails, the session
detaches: one warning in the `ReplayReport`, and every later `log_*` call
succeeds as a no-op instead of raising fresh errors.

## Invariants

1. The reader is tolerant, the writer is exact: a corrupt, non-UTF-8, or
   unknown line is skipped and counted in the `ReplayReport`, never fatal;
   everything the writer appends matches the schemas.
2. Repair keeps every future request API-valid: tool calls left unanswered at
   replay's end get synthetic terminal results appended; a rewrite of the
   middle (dangling calls inside, orphan results dropped) is persisted as a
   `reset` record, because only a reset can rewrite history.
3. A `reset` replaces the transcript but never the bill: usage totals
   survive compaction and resume.
4. Fresh ids are claimed with `create_new`; collisions retry with a new id.
5. The first line of a fresh file is the `meta` record, version 1.

## Dependencies

Contracts: [`crates/noob-provider/CONTRACT.md`](../../../noob-provider/CONTRACT.md)
for `Item`, `ToolCall`, `Usage`. How the totals are worded on screen belongs
to the UI box, not here.

## Tests

Inline: round trips, repair shapes, claim exclusivity, the fixture pin.
Boundary: `crates/noob/tests/session_recovery.rs` (kill and resume against
the real binary), `crates/noob/tests/ui_session.rs` (create, persist, resume,
replay, list through the terminal).
