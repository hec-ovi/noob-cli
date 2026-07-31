//! Append-only JSONL session transcripts under `<config>/sessions/`.
//! State lives in the mounts, never in the image. Resume rebuilds the exact
//! transcript items so the next request byte-extends the replayed prefix.
//!
//! Line shapes:
//!   {"t":"meta","v":1,"id":"...","created_ms":...}
//!   {"t":"item","item":{...}}            one transcript item appended
//!   {"t":"reset","items":[...]}          compaction replaced the transcript
//!   {"t":"usage","prefilled":N,"generated":N}   one request's cost

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use noob_provider::types::{Item, ToolCall, Usage};

/// What this session has cost so far, in tokens.
///
/// `prefilled` counts only what the server actually had to compute. Every
/// request re-sends the whole transcript, so summing raw `prompt_tokens` would
/// grow with the square of the conversation and describe work nobody did: a
/// prompt token served from the provider's cache is not prefilled again. The
/// local llama.cpp server reports the split as
/// `prompt_tokens_details.cached_tokens`, and on a repeated prefix it reported
/// 40 of 44 prompt tokens cached, so the difference is the whole number.
///
/// Kept per request rather than per turn, and on disk rather than in memory,
/// so the count describes the session and survives a resume.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionTokens {
    pub prefilled: u64,
    pub generated: u64,
}

impl SessionTokens {
    pub fn add(&mut self, u: Usage) {
        self.prefilled += u.prompt_tokens.saturating_sub(u.cached_prompt_tokens);
        self.generated += u.completion_tokens;
    }

    pub fn is_zero(&self) -> bool {
        self.prefilled == 0 && self.generated == 0
    }
}

pub struct Session {
    id: String,
    path: PathBuf,
    file: std::fs::File,
    /// Set when persisting the resume-time transcript repair failed: the
    /// session continues in memory only and append() degrades to a no-op,
    /// mirroring how the agent detaches on a later append failure.
    detached: bool,
    /// Running totals, seeded from the replayed log on a resume.
    tokens: SessionTokens,
}

const REPLAY_SKIP_CAP: u16 = 999;
const FRESH_ID_ATTEMPTS: usize = 8;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReplayReport {
    skipped: u16,
    capped: bool,
    /// The one warning for a resume whose durable repair failed and
    /// detached the session (see Session::detached).
    detached: Option<String>,
}

impl ReplayReport {
    fn record_skip(&mut self) {
        if self.skipped < REPLAY_SKIP_CAP {
            self.skipped += 1;
        } else {
            self.capped = true;
        }
    }

    pub fn warning(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.skipped > 0 {
            let count = if self.capped {
                format!("{}+", self.skipped)
            } else {
                self.skipped.to_string()
            };
            let record = if self.skipped == 1 && !self.capped {
                "record"
            } else {
                "records"
            };
            parts.push(format!(
                "session recovery warning: skipped {count} unreadable or malformed session {record}; restored valid history"
            ));
        }
        if let Some(detail) = &self.detached {
            parts.push(detail.clone());
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n"))
        }
    }
}

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub id: String,
    pub bytes: u64,
    modified: std::time::SystemTime,
}

impl Session {
    /// Saved sessions, newest first. Ignore directories, symlinks, malformed
    /// names, and unrelated files in the config directory.
    pub fn list(config_dir: &Path) -> Result<Vec<SessionInfo>, String> {
        let dir = config_dir.join("sessions");
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("cannot list {}: {error}", dir.display())),
        };
        let mut sessions = Vec::new();
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if !kind.is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Some(id) = name.strip_suffix(".jsonl") else {
                continue;
            };
            if sanitize(id).is_err() {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            sessions.push(SessionInfo {
                id: id.to_string(),
                bytes: metadata.len(),
                modified: metadata.modified().unwrap_or(std::time::UNIX_EPOCH),
            });
        }
        sessions.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| b.id.cmp(&a.id)));
        Ok(sessions)
    }

    pub fn latest_id(config_dir: &Path) -> Result<Option<String>, String> {
        Ok(Self::list(config_dir)?
            .into_iter()
            .next()
            .map(|session| session.id))
    }

    /// Open (resuming) or create the session `id`; a fresh id combines time,
    /// process, and serial components when none is given. The returned bool is
    /// whether the session file already existed: true on a real resume, false
    /// when this call created it, so an explicit `--resume <id>` miss can be
    /// reported to the human instead of silently starting fresh. The replay
    /// report describes any unreadable or malformed records that were skipped.
    pub fn open(
        config_dir: &Path,
        id: Option<&str>,
    ) -> Result<(Session, Vec<Item>, bool, ReplayReport), String> {
        let dir = config_dir.join("sessions");
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let mut items = Vec::new();
        let mut tokens = SessionTokens::default();
        let mut replay_report = ReplayReport::default();
        let (id, path, mut file, existed) = match id {
            Some(id) if !id.is_empty() => {
                let id = sanitize(id)?;
                let path = dir.join(format!("{id}.jsonl"));
                let existed = path.is_file();
                if existed {
                    let input = std::fs::File::open(&path)
                        .map_err(|e| format!("cannot read session {}: {e}", path.display()))?;
                    (items, tokens, replay_report) = replay(BufReader::new(input));
                }
                let file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .map_err(|e| format!("cannot open session {}: {e}", path.display()))?;
                (id, path, file, existed)
            }
            // Fresh ids claim their file exclusively (create_new) so two
            // processes that mint the same id can never interleave one file.
            _ => {
                let (id, path, file) = claim_fresh(
                    &dir,
                    std::iter::repeat_with(fresh_id).take(FRESH_ID_ATTEMPTS),
                )?;
                (id, path, file, false)
            }
        };
        if !existed {
            let meta = json!({"t": "meta", "v": 1, "id": id, "created_ms": now_ms()});
            writeln!(file, "{meta}")
                .and_then(|_| file.flush())
                .map_err(|e| format!("cannot initialize session {}: {e}", path.display()))?;
        }
        let mut session = Session {
            id,
            path,
            file,
            detached: false,
            tokens,
        };
        // A session killed mid-tool-batch (second Ctrl-C, SIGKILL, power
        // loss) ends with unanswered tool calls; replaying that verbatim
        // would make every future request API-invalid. Heal it here, in the
        // file too, so the repair is durable.
        let repair = repair_dangling_calls(&mut items);
        persist_repair(&mut session, &items, &repair, &mut replay_report);
        Ok((session, items, existed, replay_report))
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn log_item(&mut self, item: &Item) -> Result<(), String> {
        let line = json!({"t": "item", "item": item_to_json(item)});
        self.append(&line)
    }

    /// Compaction replaced the transcript; the log records the full new
    /// state so resume never sees the dropped middle.
    pub fn log_reset(&mut self, items: &[Item]) -> Result<(), String> {
        let arr: Vec<Value> = items.iter().map(item_to_json).collect();
        let line = json!({"t": "reset", "items": arr});
        self.append(&line)
    }

    /// Record one request's cost. Written per request because a turn can make
    /// many, and stored as the computed prefill rather than the raw prompt so
    /// the sum stays meaningful however long the transcript grows.
    pub fn log_usage(&mut self, u: Usage) -> Result<(), String> {
        self.tokens.add(u);
        let prefilled = u.prompt_tokens.saturating_sub(u.cached_prompt_tokens);
        if prefilled == 0 && u.completion_tokens == 0 {
            return Ok(());
        }
        let line = json!({
            "t": "usage",
            "prefilled": prefilled,
            "generated": u.completion_tokens,
        });
        self.append(&line)
    }

    /// Totals for this session, including everything replayed from the log.
    pub fn tokens(&self) -> SessionTokens {
        self.tokens
    }

    fn append(&mut self, line: &Value) -> Result<(), String> {
        // A detached session already surfaced its one persistence warning;
        // later items continue in memory only, without fresh errors.
        if self.detached {
            return Ok(());
        }
        writeln!(self.file, "{line}")
            .and_then(|_| self.file.flush())
            .map_err(|e| format!("cannot append session {}: {e}", self.path.display()))
    }
}

/// What healing did to the replayed transcript, and how to persist it.
enum Repair {
    /// Already healthy; nothing to write.
    None,
    /// Synthetic results appended at the very end (a session killed
    /// mid-batch); persisted as ordinary appends.
    Tail(Vec<Item>),
    /// The middle changed: a dangling assistant block got its synthetic
    /// results spliced in place, or an orphan ToolResult was dropped (one
    /// corrupt tool-result line skipped on replay produces both shapes).
    /// Only a reset record can persist a rewrite of the middle.
    Splice,
}

/// Heal a transcript whose tool calls and results do not pair up; either
/// shape is API-invalid and would 400 every future request. Calls left
/// unanswered when the next Assistant or User item arrives (or the
/// transcript ends) get synthetic terminal results spliced directly after
/// their block's real ones; a ToolResult with no preceding matching call
/// is dropped.
fn repair_dangling_calls(items: &mut Vec<Item>) -> Repair {
    let synthetic = |call_id: String| Item::ToolResult {
        call_id,
        content: "canceled: the session ended before this call finished".to_string(),
    };
    let mut out: Vec<Item> = Vec::with_capacity(items.len());
    let mut pending: Vec<String> = Vec::new();
    let mut spliced = false;
    for item in items.drain(..) {
        match &item {
            Item::Assistant { tool_calls, .. } => {
                if !pending.is_empty() {
                    spliced = true;
                    out.extend(pending.drain(..).map(synthetic));
                }
                pending = tool_calls.iter().map(|c| c.id.clone()).collect();
                out.push(item);
            }
            Item::ToolResult { call_id, .. } => {
                if let Some(at) = pending.iter().position(|id| id == call_id) {
                    pending.remove(at);
                    out.push(item);
                } else {
                    spliced = true; // orphan: no live call to answer
                }
            }
            Item::User(_) => {
                if !pending.is_empty() {
                    spliced = true;
                    out.extend(pending.drain(..).map(synthetic));
                }
                out.push(item);
            }
        }
    }
    let tail: Vec<Item> = pending.drain(..).map(synthetic).collect();
    out.extend(tail.iter().cloned());
    *items = out;
    if spliced {
        Repair::Splice
    } else if tail.is_empty() {
        Repair::None
    } else {
        Repair::Tail(tail)
    }
}

/// Durably persist a transcript repair. Tail-only repairs append (cheap);
/// a splice rewrites the whole state as a reset record, which replay then
/// applies idempotently. A persistence failure detaches the session and
/// leaves the one warning on the report instead of aborting the resume,
/// the same degradation the agent applies when a later append fails.
fn persist_repair(
    session: &mut Session,
    items: &[Item],
    repair: &Repair,
    report: &mut ReplayReport,
) {
    let persisted = match repair {
        Repair::None => Ok(()),
        Repair::Tail(tail) => tail.iter().try_for_each(|item| session.log_item(item)),
        Repair::Splice => session.log_reset(items),
    };
    if let Err(error) = persisted {
        session.detached = true;
        report.detached = Some(format!(
            "session persistence failed while repairing the transcript: {error}; \
             continuing in memory without a saved session"
        ));
    }
}

/// Session ids become file names; keep them boring. "latest" is reserved:
/// the resume flag resolves it to the newest saved session, so a session
/// actually named that could never be addressed again.
fn sanitize(id: &str) -> Result<String, String> {
    if id != "latest"
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Ok(id.to_string())
    } else {
        Err(format!(
            "session id {id:?} is invalid; use letters, digits, - and _ \
             (max 64 chars; \"latest\" is reserved)"
        ))
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn fresh_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!(
        "{:x}-{:x}-{serial:x}-{:08x}",
        now_ms(),
        std::process::id(),
        entropy()
    )
}

/// Four random bytes so two same-millisecond starts with equal pids (two
/// containers sharing /config both run as pid 1) mint different ids.
/// /dev/urandom, with a hash of per-process entropy sources as the
/// fallback; create_new in claim_fresh stays the correctness backstop.
fn entropy() -> u32 {
    let mut bytes = [0u8; 4];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))
        .is_ok()
    {
        return u32::from_le_bytes(bytes);
    }
    let stack = &bytes as *const _ as usize as u64; // ASLR-shifted address
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    crate::tools::guard::fnv1a64(&(stack ^ nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15)).to_le_bytes())
        as u32
}

/// Claim a brand-new session file. create_new makes the filesystem the
/// arbiter: when two processes mint the same fresh id, the loser
/// regenerates instead of silently interleaving two sessions in one file.
fn claim_fresh(
    dir: &Path,
    candidates: impl IntoIterator<Item = String>,
) -> Result<(String, PathBuf, std::fs::File), String> {
    let mut last_collision = String::new();
    for id in candidates {
        let path = dir.join(format!("{id}.jsonl"));
        match std::fs::OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)
        {
            Ok(file) => return Ok((id, path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                last_collision = format!("{} already exists", path.display());
            }
            Err(e) => return Err(format!("cannot create session {}: {e}", path.display())),
        }
    }
    Err(format!(
        "cannot create a fresh session in {}: every generated id collided ({last_collision})",
        dir.display()
    ))
}

fn replay(mut reader: impl BufRead) -> (Vec<Item>, SessionTokens, ReplayReport) {
    let mut items = Vec::new();
    let mut tokens = SessionTokens::default();
    let mut report = ReplayReport::default();
    loop {
        let mut line = Vec::new();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => {
                report.record_skip();
                break;
            }
        }
        let Ok(line) = std::str::from_utf8(&line) else {
            report.record_skip();
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            report.record_skip();
            continue;
        };
        match v.get("t").and_then(Value::as_str) {
            Some("meta") => {}
            Some("item") => {
                if let Some(item) = v.get("item").and_then(item_from_json) {
                    items.push(item);
                } else {
                    report.record_skip();
                }
            }
            Some("reset") => {
                let Some(reset) = v.get("items").and_then(Value::as_array) else {
                    report.record_skip();
                    continue;
                };
                let mut replacement = Vec::with_capacity(reset.len());
                for value in reset {
                    if let Some(item) = item_from_json(value) {
                        replacement.push(item);
                    } else {
                        report.record_skip();
                    }
                }
                items = replacement;
            }
            // Compaction rewrites the transcript but not the bill: tokens
            // already spent stay spent, so a reset never clears these.
            Some("usage") => {
                let n = |key| v.get(key).and_then(Value::as_u64).unwrap_or(0);
                tokens.prefilled += n("prefilled");
                tokens.generated += n("generated");
            }
            _ => report.record_skip(),
        }
    }
    (items, tokens, report)
}

fn item_to_json(item: &Item) -> Value {
    match item {
        Item::User(text) => json!({"role": "user", "text": text}),
        Item::Assistant {
            text,
            tool_calls,
            raw_items,
        } => {
            let calls: Vec<Value> = tool_calls
                .iter()
                .map(|c| json!({"id": c.id, "name": c.name, "args": c.arguments}))
                .collect();
            json!({"role": "assistant", "text": text, "calls": calls, "raw": raw_items})
        }
        Item::ToolResult { call_id, content } => {
            json!({"role": "tool", "id": call_id, "content": content})
        }
    }
}

fn item_from_json(v: &Value) -> Option<Item> {
    let str_of = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    match v.get("role").and_then(Value::as_str)? {
        "user" => Some(Item::User(str_of(v, "text")?)),
        "assistant" => {
            let calls = v
                .get("calls")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| {
                            Some(ToolCall {
                                id: str_of(c, "id")?,
                                name: str_of(c, "name")?,
                                arguments: str_of(c, "args")?,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(Item::Assistant {
                text: str_of(v, "text")?,
                tool_calls: calls,
                raw_items: v
                    .get("raw")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            })
        }
        "tool" => Some(Item::ToolResult {
            call_id: str_of(v, "id")?,
            content: str_of(v, "content")?,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call() -> ToolCall {
        ToolCall {
            id: "call_1".into(),
            name: "read".into(),
            arguments: r#"{"path":"f"}"#.into(),
        }
    }

    /// The committed schema fixtures under `fixtures/valid` are what the
    /// writer really produces: write the same content through the Session
    /// API and compare, shape for shape. The python contract runner
    /// validates the fixtures against `schema/`; this pins them to the
    /// encoder, so neither can drift alone.
    #[test]
    fn the_committed_fixtures_match_the_writer() {
        fn fixture(name: &str) -> Value {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/session/fixtures/valid")
                .join(name);
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
        }

        let tmp = tempfile::tempdir().unwrap();
        let (mut s, _, _, _) = Session::open(tmp.path(), Some("t-fixtures")).unwrap();
        s.log_item(&Item::User("hello".into())).unwrap();
        s.log_item(&Item::Assistant {
            text: "hi".into(),
            tool_calls: vec![call()],
            raw_items: vec![json!({"type": "message"})],
        })
        .unwrap();
        s.log_item(&Item::ToolResult {
            call_id: "call_1".into(),
            content: "f lines".into(),
        })
        .unwrap();
        s.log_usage(Usage {
            prompt_tokens: 14,
            cached_prompt_tokens: 4,
            completion_tokens: 5,
        })
        .unwrap();
        s.log_reset(&[Item::User("hello".into())]).unwrap();
        drop(s);

        let text =
            std::fs::read_to_string(tmp.path().join("sessions/t-fixtures.jsonl")).unwrap();
        let mut lines = text
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap());

        // The meta line's id and stamp are per run; the shape is the promise.
        let mut meta = lines.next().unwrap();
        meta["id"] = fixture("meta.json")["id"].clone();
        meta["created_ms"] = fixture("meta.json")["created_ms"].clone();
        assert_eq!(meta, fixture("meta.json"));
        assert_eq!(lines.next().unwrap(), fixture("item--user.json"));
        assert_eq!(lines.next().unwrap(), fixture("item--assistant.json"));
        assert_eq!(lines.next().unwrap(), fixture("item--tool.json"));
        assert_eq!(lines.next().unwrap(), fixture("usage.json"));
        assert_eq!(lines.next().unwrap(), fixture("reset.json"));
        assert!(lines.next().is_none());
    }

    #[test]
    fn round_trip_all_item_kinds() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut s, replayed, existed, report) = Session::open(tmp.path(), Some("t1")).unwrap();
        assert!(replayed.is_empty());
        assert!(!existed, "a first open must report the file did not exist");
        assert_eq!(report, ReplayReport::default());
        s.log_item(&Item::User("hello".into())).unwrap();
        s.log_item(&Item::Assistant {
            text: "hi".into(),
            tool_calls: vec![call()],
            raw_items: vec![json!({"type": "message"})],
        })
        .unwrap();
        s.log_item(&Item::ToolResult {
            call_id: "call_1".into(),
            content: "f lines".into(),
        })
        .unwrap();
        drop(s);

        let (_s2, items, existed, report) = Session::open(tmp.path(), Some("t1")).unwrap();
        assert!(
            existed,
            "reopening a written session must report it existed"
        );
        assert_eq!(report, ReplayReport::default());
        assert_eq!(items.len(), 3);
        match &items[1] {
            Item::Assistant {
                text,
                tool_calls,
                raw_items,
            } => {
                assert_eq!(text, "hi");
                assert_eq!(tool_calls[0].arguments, r#"{"path":"f"}"#);
                assert_eq!(raw_items[0], json!({"type": "message"}));
            }
            other => panic!("wrong item {other:?}"),
        }
    }

    #[test]
    fn reset_replaces_earlier_items_on_replay() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut s, _, _, _) = Session::open(tmp.path(), Some("t2")).unwrap();
        s.log_item(&Item::User("one".into())).unwrap();
        s.log_item(&Item::User("two".into())).unwrap();
        s.log_reset(&[Item::User("[summary]".into())]).unwrap();
        s.log_item(&Item::User("three".into())).unwrap();
        drop(s);
        let (_s, items, _, report) = Session::open(tmp.path(), Some("t2")).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(report, ReplayReport::default());
        assert!(matches!(&items[0], Item::User(t) if t == "[summary]"));
        assert!(matches!(&items[1], Item::User(t) if t == "three"));
    }

    #[test]
    fn fresh_ids_are_hex_and_files_land_in_sessions_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let (s, _, _, _) = Session::open(tmp.path(), None).unwrap();
        assert!(
            s.id()
                .split('-')
                .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_hexdigit()))
        );
        assert!(s.path().starts_with(tmp.path().join("sessions")));
        assert!(s.path().is_file());
    }

    #[test]
    fn hostile_session_ids_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        for bad in ["../escape", "a/b", "x".repeat(65).as_str()] {
            let err = match Session::open(tmp.path(), Some(bad)) {
                Err(e) => e,
                Ok(_) => panic!("{bad:?} was accepted"),
            };
            assert!(err.contains("invalid"), "{bad}: {err}");
        }
    }

    #[test]
    fn resume_repairs_dangling_tool_calls() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut s, _, _, _) = Session::open(tmp.path(), Some("t4")).unwrap();
        s.log_item(&Item::User("go".into())).unwrap();
        s.log_item(&Item::Assistant {
            text: String::new(),
            tool_calls: vec![
                ToolCall {
                    id: "c1".into(),
                    name: "bash".into(),
                    arguments: "{}".into(),
                },
                ToolCall {
                    id: "c2".into(),
                    name: "read".into(),
                    arguments: "{}".into(),
                },
            ],
            raw_items: vec![],
        })
        .unwrap();
        s.log_item(&Item::ToolResult {
            call_id: "c1".into(),
            content: "partial".into(),
        })
        .unwrap();
        drop(s); // killed before c2's result landed

        let (_s2, items, _, report) = Session::open(tmp.path(), Some("t4")).unwrap();
        assert_eq!(report, ReplayReport::default());
        assert_eq!(items.len(), 4, "one synthetic result appended");
        match &items[3] {
            Item::ToolResult { call_id, content } => {
                assert_eq!(call_id, "c2");
                assert!(content.contains("session ended before this call finished"));
            }
            other => panic!("wrong repair {other:?}"),
        }
        // Durable and idempotent: the repair went into the file, so a third
        // open sees a healed transcript and adds nothing.
        let (_s3, items, _, report) = Session::open(tmp.path(), Some("t4")).unwrap();
        assert_eq!(items.len(), 4);
        assert_eq!(report, ReplayReport::default());
    }

    #[test]
    fn corrupt_lines_are_skipped_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("t3.jsonl"),
            "{\"t\":\"item\",\"item\":{\"role\":\"user\",\"text\":\"ok\"}}\nGARBAGE\n",
        )
        .unwrap();
        let (_s, items, _, report) = Session::open(tmp.path(), Some("t3")).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(
            report.warning().as_deref(),
            Some(
                "session recovery warning: skipped 1 unreadable or malformed session record; restored valid history"
            )
        );
    }

    /// Only the tokens the server actually computed are counted. Every request
    /// re-sends the whole transcript, so counting raw prompt tokens would grow
    /// with the square of the conversation and bill work the cache did for
    /// free: here the second request re-sent 1,000 prompt tokens of which 950
    /// were cached, and that is 50 of prefill, not 1,000.
    #[test]
    fn only_the_uncached_part_of_a_prompt_counts_as_prefill() {
        let mut tokens = SessionTokens::default();
        assert!(tokens.is_zero());
        tokens.add(Usage {
            prompt_tokens: 500,
            completion_tokens: 120,
            cached_prompt_tokens: 0,
        });
        tokens.add(Usage {
            prompt_tokens: 1_000,
            completion_tokens: 80,
            cached_prompt_tokens: 950,
        });
        assert_eq!(tokens.prefilled, 550);
        assert_eq!(tokens.generated, 200);
        // A provider that reports more cached than prompt (or none at all)
        // must not underflow a u64 into billions.
        tokens.add(Usage {
            prompt_tokens: 10,
            completion_tokens: 0,
            cached_prompt_tokens: 99,
        });
        assert_eq!(tokens.prefilled, 550);
    }

    /// The count belongs to the session, so a resume continues it. Compaction
    /// rewrites the transcript but not the bill: a `reset` replaces items and
    /// leaves the totals alone.
    #[test]
    fn resume_replays_the_running_token_totals_and_compaction_does_not_clear_them() {
        let input = concat!(
            "{\"t\":\"meta\",\"v\":1}\n",
            "{\"t\":\"usage\",\"prefilled\":500,\"generated\":120}\n",
            "{\"t\":\"item\",\"item\":{\"role\":\"user\",\"text\":\"before\"}}\n",
            "{\"t\":\"usage\",\"prefilled\":50,\"generated\":80}\n",
            "{\"t\":\"reset\",\"items\":[{\"role\":\"user\",\"text\":\"summary\"}]}\n",
            "{\"t\":\"usage\",\"prefilled\":7}\n",
        );
        let (items, tokens, report) = replay(std::io::Cursor::new(input));
        assert_eq!(tokens.prefilled, 557);
        assert_eq!(tokens.generated, 200);
        assert_eq!(items.len(), 1, "the reset still replaced the transcript");
        assert_eq!(report.warning(), None, "a usage record is not a skip");
    }

    /// The whole point of writing them down: open, spend, reopen, keep counting.
    #[test]
    fn a_reopened_session_keeps_counting_from_what_it_spent_before() {
        let dir = tempfile::tempdir().unwrap();
        let (mut session, _, _, _) = Session::open(dir.path(), Some("resume-me")).unwrap();
        assert!(session.tokens().is_zero());
        session
            .log_usage(Usage {
                prompt_tokens: 400,
                completion_tokens: 60,
                cached_prompt_tokens: 0,
            })
            .unwrap();
        session
            .log_usage(Usage {
                prompt_tokens: 900,
                completion_tokens: 40,
                cached_prompt_tokens: 880,
            })
            .unwrap();
        assert_eq!(session.tokens().prefilled, 420);
        drop(session);

        let (session, _, existed, _) = Session::open(dir.path(), Some("resume-me")).unwrap();
        assert!(existed);
        assert_eq!(session.tokens().prefilled, 420);
        assert_eq!(session.tokens().generated, 100);
    }

    /// A request that cost nothing writes nothing: a stream that ends without
    /// usage should not pad the log with empty records.
    #[test]
    fn a_zero_usage_request_writes_no_record() {
        let dir = tempfile::tempdir().unwrap();
        let (mut session, _, _, _) = Session::open(dir.path(), Some("quiet")).unwrap();
        let before = std::fs::read_to_string(session.path()).unwrap();
        session
            .log_usage(Usage {
                prompt_tokens: 30,
                completion_tokens: 0,
                cached_prompt_tokens: 30,
            })
            .unwrap();
        assert_eq!(std::fs::read_to_string(session.path()).unwrap(), before);
        assert!(session.tokens().is_zero());
    }

    #[test]
    fn replay_counts_each_skipped_record_and_keeps_valid_history() {
        let input = concat!(
            "{\"t\":\"meta\",\"v\":1}\n",
            "{\"t\":\"item\",\"item\":{\"role\":\"user\",\"text\":\"before\"}}\n",
            "{\"t\":\"reset\",\"items\":[{\"role\":\"user\",\"text\":\"summary\"},{\"role\":\"tool\",\"id\":\"missing-content\"}]}\n",
            "GARBAGE\n",
            "{\"t\":\"future-record\"}\n",
            "{\"t\":\"item\",\"item\":{\"role\":\"future-role\"}}\n",
            "{\"t\":\"reset\",\"items\":\"not-an-array\"}\n",
            "{\"t\":\"item\",\"item\":{\"role\":\"user\",\"text\":\"after\"}}\n",
        );

        let (items, _tokens, report) = replay(std::io::Cursor::new(input));

        assert_eq!(report.skipped, 5);
        assert!(!report.capped);
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], Item::User(text) if text == "summary"));
        assert!(matches!(&items[1], Item::User(text) if text == "after"));
    }

    #[test]
    fn replay_skip_count_is_bounded() {
        let input = "GARBAGE\n".repeat(usize::from(REPLAY_SKIP_CAP) + 20);

        let (items, _tokens, report) = replay(std::io::Cursor::new(input));

        assert!(items.is_empty());
        assert_eq!(report.skipped, REPLAY_SKIP_CAP);
        assert!(report.capped);
        assert!(report.warning().unwrap().contains("skipped 999+"));
    }

    #[test]
    fn replay_skips_non_utf8_record_and_continues() {
        let mut input =
            b"{\"t\":\"item\",\"item\":{\"role\":\"user\",\"text\":\"before\"}}\n".to_vec();
        input.extend_from_slice(&[0xff, b'\n']);
        input.extend_from_slice(
            b"{\"t\":\"item\",\"item\":{\"role\":\"user\",\"text\":\"after\"}}\n",
        );

        let (items, _tokens, report) = replay(std::io::Cursor::new(input));

        assert_eq!(report.skipped, 1);
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], Item::User(text) if text == "before"));
        assert!(matches!(&items[1], Item::User(text) if text == "after"));
    }

    #[test]
    fn replay_counts_an_unreadable_tail_once_and_stops() {
        struct Unreadable;

        impl std::io::Read for Unreadable {
            fn read(&mut self, _buffer: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "not UTF-8",
                ))
            }
        }

        impl BufRead for Unreadable {
            fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "not UTF-8",
                ))
            }

            fn consume(&mut self, _amount: usize) {}
        }

        let (items, _tokens, report) = replay(Unreadable);

        assert!(items.is_empty());
        assert_eq!(report.skipped, 1);
        assert!(!report.capped);
    }

    #[test]
    fn append_errors_are_reported_instead_of_silently_losing_the_session() {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap();
        let mut session = Session {
            id: "full".into(),
            path: PathBuf::from("/dev/full"),
            file,
            detached: false,
            tokens: SessionTokens::default(),
        };
        let error = session
            .log_item(&Item::User("important".into()))
            .unwrap_err();
        assert!(error.contains("cannot append session"), "{error}");
    }

    #[test]
    fn mid_transcript_dangle_is_repaired_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut s, _, _, _) = Session::open(tmp.path(), Some("t5")).unwrap();
        s.log_item(&Item::User("go".into())).unwrap();
        s.log_item(&Item::Assistant {
            text: String::new(),
            tool_calls: vec![
                ToolCall {
                    id: "c1".into(),
                    name: "bash".into(),
                    arguments: "{}".into(),
                },
                ToolCall {
                    id: "c2".into(),
                    name: "read".into(),
                    arguments: "{}".into(),
                },
            ],
            raw_items: vec![],
        })
        .unwrap();
        s.log_item(&Item::ToolResult {
            call_id: "c1".into(),
            content: "partial".into(),
        })
        .unwrap();
        // c2's result line was corrupted and skipped on replay; the turn
        // continued, so the dangle sits in the MIDDLE of the transcript
        // and a tail-only repair would leave every request API-invalid.
        s.log_item(&Item::User("next".into())).unwrap();
        s.log_item(&Item::Assistant {
            text: "done".into(),
            tool_calls: vec![],
            raw_items: vec![],
        })
        .unwrap();
        drop(s);

        let (_s2, items, _, report) = Session::open(tmp.path(), Some("t5")).unwrap();
        assert!(report.warning().is_none());
        assert_eq!(items.len(), 6, "one synthetic result spliced in place");
        match &items[3] {
            Item::ToolResult { call_id, content } => {
                assert_eq!(call_id, "c2");
                assert!(content.contains("session ended before this call finished"));
            }
            other => panic!("wrong splice {other:?}"),
        }
        assert!(matches!(&items[4], Item::User(t) if t == "next"));
        // Durable via a reset record and idempotent on the next open.
        let (_s3, items, _, report) = Session::open(tmp.path(), Some("t5")).unwrap();
        assert_eq!(items.len(), 6);
        assert!(report.warning().is_none());
        match &items[3] {
            Item::ToolResult { call_id, .. } => assert_eq!(call_id, "c2"),
            other => panic!("wrong replayed splice {other:?}"),
        }
    }

    #[test]
    fn orphan_tool_result_is_dropped_and_the_drop_is_durable() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut s, _, _, _) = Session::open(tmp.path(), Some("t6")).unwrap();
        s.log_item(&Item::User("go".into())).unwrap();
        // No preceding assistant call carries this id (its call line was
        // the corrupt record); replaying the result verbatim is API-invalid.
        s.log_item(&Item::ToolResult {
            call_id: "ghost".into(),
            content: "x".into(),
        })
        .unwrap();
        s.log_item(&Item::User("next".into())).unwrap();
        drop(s);
        let (_s2, items, _, _) = Session::open(tmp.path(), Some("t6")).unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], Item::User(t) if t == "go"));
        assert!(matches!(&items[1], Item::User(t) if t == "next"));
        let (_s3, items, _, _) = Session::open(tmp.path(), Some("t6")).unwrap();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn repair_drops_a_result_answering_an_already_answered_call() {
        let mut items = vec![
            Item::Assistant {
                text: String::new(),
                tool_calls: vec![call()],
                raw_items: vec![],
            },
            Item::ToolResult {
                call_id: "call_1".into(),
                content: "one".into(),
            },
            Item::ToolResult {
                call_id: "call_1".into(),
                content: "dup".into(),
            },
        ];
        let repair = repair_dangling_calls(&mut items);
        assert!(matches!(repair, Repair::Splice));
        assert_eq!(items.len(), 2, "the duplicate answer is dropped");
        assert!(matches!(&items[1], Item::ToolResult { content, .. } if content == "one"));
    }

    #[test]
    fn failed_durable_repair_detaches_with_a_warning_instead_of_aborting() {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open("/dev/full")
            .unwrap();
        let mut session = Session {
            id: "full".into(),
            path: PathBuf::from("/dev/full"),
            file,
            detached: false,
            tokens: SessionTokens::default(),
        };
        let mut items = vec![Item::Assistant {
            text: String::new(),
            tool_calls: vec![call()],
            raw_items: vec![],
        }];
        let mut report = ReplayReport::default();
        let repair = repair_dangling_calls(&mut items);
        persist_repair(&mut session, &items, &repair, &mut report);
        assert_eq!(items.len(), 2, "the in-memory repair still applies");
        let warning = report.warning().unwrap();
        assert!(warning.contains("session persistence failed"), "{warning}");
        // Detached: later appends degrade to in-memory no-ops, no new errors.
        assert!(session.log_item(&Item::User("more".into())).is_ok());
    }

    #[test]
    fn fresh_ids_carry_an_entropy_component() {
        let id = fresh_id();
        assert_eq!(id.split('-').count(), 4, "{id}");
        let entropies: std::collections::HashSet<String> = (0..8)
            .map(|_| fresh_id().rsplit('-').next().unwrap().to_string())
            .collect();
        assert!(entropies.len() > 1, "the entropy component never varies");
    }

    #[test]
    fn fresh_open_never_adopts_an_existing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sessions");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("dup.jsonl"), "{\"t\":\"meta\",\"v\":1}\n").unwrap();
        // Two processes minted the same id: the loser must regenerate, not
        // silently interleave into the winner's file.
        let ids = ["dup".to_string(), "dup".to_string(), "fresh2".to_string()];
        let (id, path, _file) = claim_fresh(&dir, ids).unwrap();
        assert_eq!(id, "fresh2");
        assert!(path.ends_with("fresh2.jsonl"));
        assert_eq!(
            std::fs::read_to_string(dir.join("dup.jsonl")).unwrap(),
            "{\"t\":\"meta\",\"v\":1}\n",
            "the colliding file must be untouched"
        );
        let err = claim_fresh(&dir, ["dup".to_string()]).unwrap_err();
        assert!(err.contains("collided"), "{err}");
    }

    #[test]
    fn latest_is_reserved_as_a_session_id() {
        let tmp = tempfile::tempdir().unwrap();
        let err = match Session::open(tmp.path(), Some("latest")) {
            Err(e) => e,
            Ok(_) => panic!("the reserved id \"latest\" was accepted"),
        };
        assert!(err.contains("invalid"), "{err}");
        assert!(err.contains("latest"), "{err}");
        assert!(!tmp.path().join("sessions/latest.jsonl").exists());
    }

    #[test]
    fn list_is_newest_first_and_latest_ignores_unrelated_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let (s1, _, _, _) = Session::open(tmp.path(), Some("older")).unwrap();
        drop(s1);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let (s2, _, _, _) = Session::open(tmp.path(), Some("newer")).unwrap();
        drop(s2);
        std::fs::write(tmp.path().join("sessions/notes.txt"), "ignore").unwrap();
        std::fs::create_dir(tmp.path().join("sessions/fake.jsonl")).unwrap();

        let listed = Session::list(tmp.path()).unwrap();
        assert_eq!(
            listed.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            ["newer", "older"]
        );
        assert_eq!(
            Session::latest_id(tmp.path()).unwrap().as_deref(),
            Some("newer")
        );
    }
}
