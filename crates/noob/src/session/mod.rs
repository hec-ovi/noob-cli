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

impl Session {
    /// Open (resuming) or create the session `id`; a fresh id combines time,
    /// process, and serial components when none is given.
    pub fn open(
        config_dir: &Path,
        id: Option<&str>,
    ) -> Result<(Session, Vec<Item>), String> {
        let dir = config_dir.join("sessions");
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let id = match id {
            Some(id) if !id.is_empty() => sanitize(id)?,
            _ => fresh_id(),
        };
        let path = dir.join(format!("{id}.jsonl"));
        let mut items = Vec::new();
        let existed = path.is_file();
        if existed {
            let input = std::fs::File::open(&path)
                .map_err(|e| format!("cannot read session {}: {e}", path.display()))?;
            items = replay(BufReader::new(input));
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
        Ok((session, items))
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

fn replay(reader: impl BufRead) -> Vec<Item> {
    let mut items = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        match v.get("t").and_then(Value::as_str) {
            Some("item") => {
                if let Some(item) = v.get("item").and_then(item_from_json) {
                    items.push(item);
                }
            }
            Some("reset") => {
                items.clear();
                if let Some(reset) = v.get("items").and_then(Value::as_array) {
                    items.extend(reset.iter().filter_map(item_from_json));
                }
            }
            _ => {}
        }
    }
    items
}

fn item_to_json(item: &Item) -> Value {
    match item {
        Item::User(text) => json!({"role": "user", "text": text}),
        Item::Assistant { text, tool_calls, raw_items } => {
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
        let (mut s, replayed) = Session::open(tmp.path(), Some("t1")).unwrap();
        assert!(replayed.is_empty());
        s.log_item(&Item::User("hello".into())).unwrap();
        s.log_item(&Item::Assistant {
            text: "hi".into(),
            tool_calls: vec![call()],
            raw_items: vec![json!({"type": "message"})],
        })
        .unwrap();
        s.log_item(&Item::ToolResult { call_id: "call_1".into(), content: "f lines".into() })
            .unwrap();
        drop(s);

        let (_s2, items) = Session::open(tmp.path(), Some("t1")).unwrap();
        assert_eq!(items.len(), 3);
        match &items[1] {
            Item::Assistant { text, tool_calls, raw_items } => {
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
        let (mut s, _) = Session::open(tmp.path(), Some("t2")).unwrap();
        s.log_item(&Item::User("one".into())).unwrap();
        s.log_item(&Item::User("two".into())).unwrap();
        s.log_reset(&[Item::User("[summary]".into())]).unwrap();
        s.log_item(&Item::User("three".into())).unwrap();
        drop(s);
        let (_s, items) = Session::open(tmp.path(), Some("t2")).unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(&items[0], Item::User(t) if t == "[summary]"));
        assert!(matches!(&items[1], Item::User(t) if t == "three"));
    }

    #[test]
    fn fresh_ids_are_hex_and_files_land_in_sessions_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let (s, _) = Session::open(tmp.path(), None).unwrap();
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
        let (mut s, _) = Session::open(tmp.path(), Some("t4")).unwrap();
        s.log_item(&Item::User("go".into())).unwrap();
        s.log_item(&Item::Assistant {
            text: String::new(),
            tool_calls: vec![
                ToolCall { id: "c1".into(), name: "bash".into(), arguments: "{}".into() },
                ToolCall { id: "c2".into(), name: "read".into(), arguments: "{}".into() },
            ],
            raw_items: vec![],
        })
        .unwrap();
        s.log_item(&Item::ToolResult { call_id: "c1".into(), content: "partial".into() })
            .unwrap();
        drop(s); // killed before c2's result landed

        let (_s2, items) = Session::open(tmp.path(), Some("t4")).unwrap();
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
        let (_s3, items) = Session::open(tmp.path(), Some("t4")).unwrap();
        assert_eq!(items.len(), 4);
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
        let (_s, items) = Session::open(tmp.path(), Some("t3")).unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn append_errors_are_reported_instead_of_silently_losing_the_session() {
        let file = std::fs::OpenOptions::new().write(true).open("/dev/full").unwrap();
        let mut session = Session {
            id: "full".into(),
            path: PathBuf::from("/dev/full"),
            file,
        };
        let error = session.log_item(&Item::User("important".into())).unwrap_err();
        assert!(error.contains("cannot append session"), "{error}");
    }
}
