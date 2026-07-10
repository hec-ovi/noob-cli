//! The built-in tools: pure functions of (context, args) -> outcome against
//! the filesystem and shell. No knowledge of the agent loop or the wire.
//! Truncation happens here, once, at emission; results are byte-frozen after.

pub mod bash;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod guard;
pub mod ls;
pub mod read;
pub mod skill;
pub mod truncate;
pub mod write;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use serde_json::{Value, json};

use guard::{Sandbox, SeenFiles};
use noob_provider::types::ToolSpec;

/// Session-scoped state the tools share. Interior mutability because
/// read-only tools run concurrently on scoped threads.
pub struct ToolCtx {
    /// Canonicalized working directory; the root of relative paths.
    pub workspace: PathBuf,
    pub sandbox: Sandbox,
    pub seen: SeenFiles,
    /// Failed-edit counter per (path, old) for the escalation ladder.
    pub edit_failures: Mutex<std::collections::HashMap<(PathBuf, u64), u32>>,
    /// One-time "no sandbox" warning for the bash tool (workspace mode).
    pub bash_warned: AtomicBool,
    /// Skills discovered at session start; empty means the skill tool is
    /// not registered. Set once at bootstrap, read-only afterwards.
    pub skills: Vec<crate::skills::Skill>,
    /// Names of skills loaded this session, in load order, for the
    /// post-compaction re-listing.
    pub loaded_skills: Mutex<Vec<String>>,
}

impl ToolCtx {
    pub fn new(workspace: PathBuf, sandbox: Sandbox) -> ToolCtx {
        ToolCtx {
            workspace,
            sandbox,
            seen: SeenFiles::new(),
            edit_failures: Mutex::new(std::collections::HashMap::new()),
            bash_warned: AtomicBool::new(false),
            skills: Vec::new(),
            loaded_skills: Mutex::new(Vec::new()),
        }
    }
}

/// What one tool call produced. `content` goes into the transcript verbatim;
/// `summary` is the short human line the UI renders (`* read src/main.rs
/// (312 lines)`); `warning` is UI-only, never in the transcript.
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
    pub summary: String,
    pub warning: Option<String>,
}

impl ToolOutcome {
    pub fn ok(content: impl Into<String>, summary: impl Into<String>) -> ToolOutcome {
        ToolOutcome {
            content: content.into(),
            is_error: false,
            summary: summary.into(),
            warning: None,
        }
    }

    pub fn err(content: impl Into<String>) -> ToolOutcome {
        ToolOutcome {
            content: content.into(),
            is_error: true,
            summary: "error".to_string(),
            warning: None,
        }
    }
}

/// Read-only calls run concurrently; anything else is a sequential barrier.
pub fn is_read_only(name: &str) -> bool {
    matches!(name, "read" | "grep" | "glob" | "ls" | "skill" | "mcp_connect")
}

/// Execute one tool call. `args` is the parsed arguments object.
pub fn dispatch(ctx: &ToolCtx, name: &str, args: &Value) -> ToolOutcome {
    match name {
        "read" => read::run(ctx, args),
        "write" => write::run(ctx, args),
        "edit" => edit::run(ctx, args),
        "bash" => bash::run(ctx, args),
        "grep" => grep::run(ctx, args),
        "glob" => glob::run(ctx, args),
        "ls" => ls::run(ctx, args),
        "skill" => skill::run(ctx, args),
        other => ToolOutcome::err(format!(
            "unknown tool {other:?}; the available tools are listed in your tool schemas"
        )),
    }
}

/// The 7 core tool schemas, registered at session start and byte-stable for
/// the whole session. Descriptions <= 20 words each; the serialized array is
/// budget-tested against the 940-token ceiling.
pub fn specs() -> Vec<ToolSpec> {
    fn spec(name: &str, description: &str, parameters: Value) -> ToolSpec {
        ToolSpec {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
        }
    }
    vec![
        spec(
            "read",
            "Read a text file as plain lines; page big files with offset and limit.",
            json!({"type": "object", "properties": {
                "path": {"type": "string"},
                "offset": {"type": "integer", "description": "1-based first line"},
                "limit": {"type": "integer", "description": "max lines, default 500"}
            }, "required": ["path"]}),
        ),
        spec(
            "write",
            "Create or replace a file with the given content; read existing files first.",
            json!({"type": "object", "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            }, "required": ["path", "content"]}),
        ),
        spec(
            "edit",
            "Replace old with new in a file; old must match exactly one spot; read first.",
            json!({"type": "object", "properties": {
                "path": {"type": "string"},
                "old": {"type": "string"},
                "new": {"type": "string"},
                "all": {"type": "boolean", "description": "replace every match"}
            }, "required": ["path", "old", "new"]}),
        ),
        spec(
            "bash",
            "Run a shell command; returns merged stdout and stderr; default timeout 120s.",
            json!({"type": "object", "properties": {
                "cmd": {"type": "string"},
                "timeout_s": {"type": "integer", "description": "seconds, max 600"}
            }, "required": ["cmd"]}),
        ),
        spec(
            "grep",
            "Search file contents with a regex; returns path: line matches, gitignore-aware.",
            json!({"type": "object", "properties": {
                "pattern": {"type": "string"},
                "path": {"type": "string", "description": "file or directory to search"},
                "glob": {"type": "string", "description": "filter files, e.g. *.rs"},
                "ignore_case": {"type": "boolean"}
            }, "required": ["pattern"]}),
        ),
        spec(
            "glob",
            "List files matching a glob pattern, newest first, gitignore-aware.",
            json!({"type": "object", "properties": {
                "pattern": {"type": "string", "description": "e.g. src/**/*.rs"}
            }, "required": ["pattern"]}),
        ),
        spec(
            "ls",
            "List a directory; directories end with /.",
            json!({"type": "object", "properties": {
                "path": {"type": "string", "description": "default: working directory"}
            }}),
        ),
    ]
}

// --- shared argument helpers -------------------------------------------------

pub(crate) fn need_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    match args.get(key) {
        Some(Value::String(s)) => Ok(s),
        Some(other) => Err(format!(
            "parameter {key:?} must be a string, got {other}; resend the call"
        )),
        None => Err(format!("missing required parameter {key:?}; resend the call")),
    }
}

pub(crate) fn opt_str<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(other) => Err(format!(
            "parameter {key:?} must be a string, got {other}; resend the call"
        )),
    }
}

pub(crate) fn opt_u64(args: &Value, key: &str) -> Result<Option<u64>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_u64() {
            Some(n) => Ok(Some(n)),
            // Models sometimes send numbers as strings; accept them.
            None => v
                .as_str()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(Some)
                .ok_or_else(|| {
                    format!(
                        "parameter {key:?} must be a non-negative integer, got {v}; \
                         resend the call"
                    )
                }),
        },
    }
}

pub(crate) fn opt_bool(args: &Value, key: &str) -> Result<Option<bool>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(Value::String(s)) if s == "true" => Ok(Some(true)),
        Some(Value::String(s)) if s == "false" => Ok(Some(false)),
        Some(other) => Err(format!(
            "parameter {key:?} must be true or false, got {other}; resend the call"
        )),
    }
}

/// Render a path relative to the workspace when it is inside it (short,
/// stable output for transcripts and summaries). The workspace itself is ".".
pub(crate) fn display_path(ctx: &ToolCtx, path: &std::path::Path) -> String {
    match path.strip_prefix(&ctx.workspace) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => rel.display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
pub(crate) fn test_ctx() -> (tempfile::TempDir, ToolCtx) {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().canonicalize().unwrap();
    (tmp, ToolCtx::new(ws, Sandbox::Container))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_partition_matches_the_locked_decision() {
        for t in ["read", "grep", "glob", "ls", "skill", "mcp_connect"] {
            assert!(is_read_only(t), "{t} must be read-only");
        }
        for t in ["write", "edit", "bash", "mcp_call", "task"] {
            assert!(!is_read_only(t), "{t} must be a barrier");
        }
    }

    #[test]
    fn seven_core_specs_with_short_descriptions() {
        let specs = specs();
        assert_eq!(specs.len(), 7);
        for s in &specs {
            let words = s.description.split_whitespace().count();
            assert!(words <= 20, "{} description has {words} words", s.name);
            assert!(s.parameters.get("type").is_some());
        }
    }

    #[test]
    fn unknown_tool_is_a_typed_error() {
        let (_tmp, ctx) = test_ctx();
        let out = dispatch(&ctx, "teleport", &json!({}));
        assert!(out.is_error);
        assert!(out.content.contains("unknown tool"));
    }

    #[test]
    fn numeric_strings_are_accepted_for_integer_params() {
        assert_eq!(opt_u64(&json!({"offset": "12"}), "offset").unwrap(), Some(12));
        assert_eq!(opt_u64(&json!({"offset": 12}), "offset").unwrap(), Some(12));
        assert!(opt_u64(&json!({"offset": -3}), "offset").is_err());
    }
}
