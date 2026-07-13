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
}

const REPLAY_SKIP_CAP: u16 = 999;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplayReport {
    skipped: u16,
    capped: bool,
}

impl ReplayReport {
    fn record_skip(&mut self) {
        if self.skipped < REPLAY_SKIP_CAP {
            self.skipped += 1;
        } else {
            self.capped = true;
        }
    }

    pub fn warning(self) -> Option<String> {
        if self.skipped == 0 {
            return None;
        }
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
        Some(format!(
            "session recovery warning: skipped {count} unreadable or malformed session {record}; restored valid history"
        ))
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
        let id = match id {
            Some(id) if !id.is_empty() => sanitize(id)?,
            _ => fresh_id(),
        };
        let path = dir.join(format!("{id}.jsonl"));
        let mut items = Vec::new();
        let mut replay_report = ReplayReport::default();
        let existed = path.is_file();
        if existed {
            let input = std::fs::File::open(&path)
                .map_err(|e| format!("cannot read session {}: {e}", path.display()))?;
            (items, replay_report) = replay(BufReader::new(input));
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("cannot open session {}: {e}", path.display()))?;
        if !existed {
            let meta = json!({"t": "meta", "v": 1, "id": id, "created_ms": now_ms()});
            writeln!(file, "{meta}")
                .and_then(|_| file.flush())
                .map_err(|e| format!("cannot initialize session {}: {e}", path.display()))?;
        }
        let mut session = Session { id, path, file };
        // A session killed mid-tool-batch (second Ctrl-C, SIGKILL, power
        // loss) ends with unanswered tool calls; replaying that verbatim
        // would make every future request API-invalid. Heal it here, in the
        // file too, so the repair is durable.
        for repair in repair_dangling_calls(&mut items) {
            session.log_item(&repair)?;
        }
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
        writeln!(self.file, "{line}")
            .and_then(|_| self.file.flush())
            .map_err(|e| format!("cannot append session {}: {e}", self.path.display()))
    }
}

/// Synthetic results for tool calls the replayed transcript never answered.
/// Appended to `items` AND returned so the caller can persist them.
fn repair_dangling_calls(items: &mut Vec<Item>) -> Vec<Item> {
    let mut pending: Vec<String> = Vec::new();
    for item in items.iter() {
        match item {
            Item::Assistant { tool_calls, .. } => {
                pending = tool_calls.iter().map(|c| c.id.clone()).collect();
            }
            Item::ToolResult { call_id, .. } => {
                pending.retain(|id| id != call_id);
            }
            Item::User(_) => pending.clear(),
        }
    }
    let repairs: Vec<Item> = pending
        .into_iter()
        .map(|call_id| Item::ToolResult {
            call_id,
            content: "canceled: the session ended before this call finished".to_string(),
        })
        .collect();
    items.extend(repairs.iter().cloned());
    repairs
}

/// Session ids become file names; keep them boring.
fn sanitize(id: &str) -> Result<String, String> {
    if id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Ok(id.to_string())
    } else {
        Err(format!(
            "session id {id:?} is invalid; use letters, digits, - and _ (max 64 chars)"
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
    format!("{:x}-{:x}-{serial:x}", now_ms(), std::process::id())
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
        };
        let error = session
            .log_item(&Item::User("important".into()))
            .unwrap_err();
        assert!(error.contains("cannot append session"), "{error}");
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
