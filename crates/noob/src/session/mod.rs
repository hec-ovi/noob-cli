//! Append-only JSONL session transcripts under `<config>/sessions/`.
//! State lives in the mounts, never in the image. Resume rebuilds the exact
//! transcript items so the next request byte-extends the replayed prefix.
//!
//! Line shapes:
//!   {"t":"meta","v":1,"id":"...","created_ms":...}
//!   {"t":"item","item":{...}}            one transcript item appended
//!   {"t":"reset","items":[...]}          compaction replaced the transcript

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use noob_provider::types::{Item, ToolCall};

pub struct Session {
    id: String,
    path: PathBuf,
    file: std::fs::File,
    /// Set when persisting the resume-time transcript repair failed: the
    /// session continues in memory only and append() degrades to a no-op,
    /// mirroring how the agent detaches on a later append failure.
    detached: bool,
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
        let mut replay_report = ReplayReport::default();
        let (id, path, mut file, existed) = match id {
            Some(id) if !id.is_empty() => {
                let id = sanitize(id)?;
                let path = dir.join(format!("{id}.jsonl"));
                let existed = path.is_file();
                if existed {
                    let input = std::fs::File::open(&path)
                        .map_err(|e| format!("cannot read session {}: {e}", path.display()))?;
                    (items, replay_report) = replay(BufReader::new(input));
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

fn replay(mut reader: impl BufRead) -> (Vec<Item>, ReplayReport) {
    let mut items = Vec::new();
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
            _ => report.record_skip(),
        }
    }
    (items, report)
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

        let (items, report) = replay(std::io::Cursor::new(input));

        assert_eq!(report.skipped, 5);
        assert!(!report.capped);
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], Item::User(text) if text == "summary"));
        assert!(matches!(&items[1], Item::User(text) if text == "after"));
    }

    #[test]
    fn replay_skip_count_is_bounded() {
        let input = "GARBAGE\n".repeat(usize::from(REPLAY_SKIP_CAP) + 20);

        let (items, report) = replay(std::io::Cursor::new(input));

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

        let (items, report) = replay(std::io::Cursor::new(input));

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

        let (items, report) = replay(Unreadable);

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
