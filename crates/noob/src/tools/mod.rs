//! The built-in tools: pure functions of (context, args) -> outcome against
//! the filesystem and shell. No knowledge of the agent loop or the wire.
//! Truncation happens here, once, at emission; results are byte-frozen after.

pub mod bash;
pub mod context;
pub mod edit;
pub mod glob;
pub mod grep;
pub mod guard;
pub mod ls;
pub mod mcp;
pub mod read;
pub mod skill;
pub mod todo;
pub mod truncate;
pub mod write;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use guard::{Sandbox, SeenFiles};
use noob_provider::types::ToolSpec;

/// One line of the agentic checklist the `plan` tool maintains. The model
/// sends the whole list each call (overwrite semantics); the rendered list is
/// the tool result, so every surface and the model see the same plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Default)]
pub(crate) struct PlanTiming {
    pub started: Option<Instant>,
    pub active: HashMap<String, Instant>,
    pub completed: HashMap<String, Duration>,
}

impl TodoStatus {
    /// ASCII-safe status glyph, identical on every surface; the themed REPL
    /// only recolors it, never changes the characters.
    pub fn glyph(self) -> &'static str {
        match self {
            TodoStatus::Completed => "[x]",
            TodoStatus::InProgress => "[~]",
            TodoStatus::Pending => "[ ]",
        }
    }

    fn parse(s: &str) -> Option<TodoStatus> {
        match s {
            "pending" => Some(TodoStatus::Pending),
            "in_progress" => Some(TodoStatus::InProgress),
            "completed" => Some(TodoStatus::Completed),
            _ => None,
        }
    }
}

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
    /// One execution grant per confirmed skills-dir write, counted by real
    /// target path. Counts preserve two explicit approvals for two calls to
    /// the same path while preventing either grant from leaking to a later
    /// operation.
    pub approved_skill_writes: Mutex<std::collections::HashMap<PathBuf, usize>>,
    /// MCP manager; Some only when mcp.json configured at least one server
    /// (and then mcp_connect/mcp_call are registered). Set at bootstrap.
    pub mcp: Option<crate::mcp::Mcp>,
    /// Sub-agent settings; Some only when the task tool is registered
    /// (depth below the ceiling, full tool set). Set at bootstrap.
    pub task: Option<crate::subagent::TaskCfg>,
    /// The agentic checklist the `plan` tool maintains for this session.
    /// Overwritten wholesale on each `plan` call; starts empty.
    pub todos: Mutex<Vec<TodoItem>>,
    /// Wall-clock lifecycle for the visible plan and each observed active item.
    /// This is display state only and never enters a provider request except via
    /// the rendered plan tool result.
    pub(crate) plan_timing: Mutex<PlanTiming>,
    /// Truncation policy for every tool result (NOOB_TOOL_CAPS). Defaults to
    /// the shipped caps; bootstrap swaps in Caps::uncapped() when the setting
    /// says 0/off. Copy semantics, read-only after bootstrap.
    pub caps: truncate::Caps,
    /// Context accounting shared with the model-callable `context` tool.
    /// The agent refreshes the estimate at transcript boundaries; the tool
    /// only reads these atomics, so concurrent read batches stay lock-free.
    context_used: AtomicU64,
    context_total: AtomicU64,
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
            approved_skill_writes: Mutex::new(std::collections::HashMap::new()),
            mcp: None,
            task: None,
            todos: Mutex::new(Vec::new()),
            plan_timing: Mutex::new(PlanTiming::default()),
            caps: truncate::Caps::default(),
            context_used: AtomicU64::new(0),
            context_total: AtomicU64::new(0),
        }
    }

    pub(crate) fn set_context(&self, used: u64, total: u64) {
        self.context_used.store(used, Ordering::Relaxed);
        self.context_total.store(total, Ordering::Relaxed);
    }

    pub(crate) fn context(&self) -> (u64, u64) {
        (
            self.context_used.load(Ordering::Relaxed),
            self.context_total.load(Ordering::Relaxed),
        )
    }

    /// The fan-out cap for a group of task calls in one batch. A depth-1
    /// process may still delegate, but does so one child at a time. Otherwise
    /// C root children could each fan out C more model requests and turn the
    /// configured cap into C squared across the process tree.
    pub(crate) fn task_concurrency(&self) -> usize {
        self.task
            .as_ref()
            .map(|t| if t.depth == 0 { t.concurrency } else { 1 })
            .unwrap_or(1)
            .max(1)
    }

    /// Execution-time half of the skills-dir write gate: refuse a write/edit
    /// whose real target lands in a skills directory unless the user
    /// confirmed exactly that target. Returns the refusal message, or None
    /// when the write may proceed. Closes the plan-time-vs-execution-time
    /// gap a same-batch symlink would otherwise open.
    pub(crate) fn skills_write_refusal(&self, raw: &str) -> Option<String> {
        let target = guard::skill_write_target(&self.workspace, raw)?;
        let mut approvals = self.approved_skill_writes.lock().unwrap();
        if let std::collections::hash_map::Entry::Occupied(mut entry) = approvals.entry(target) {
            if *entry.get() > 1 {
                *entry.get_mut() -= 1;
            } else {
                entry.remove();
            }
            return None; // exactly one grant consumed for this operation
        }
        Some(
            "refused: writing into a skills directory needs the user's confirmation \
             and it was not granted; leave skill files unchanged and continue \
             without them"
                .to_string(),
        )
    }
}

/// What one tool call produced. `content` goes into the transcript verbatim;
/// `summary` is the short human line the UI renders (`* read src/main.rs
/// (312 lines)`); `warning` is UI-only, never in the transcript. `canceled`
/// is set only by the scheduler when a Ctrl-C skipped the call, so the loop
/// recognizes cancellation structurally instead of by matching the content
/// string (which a tool could otherwise echo to forge one). Tools that were
/// already running also set it when they observe the shared interrupt.
pub struct ToolOutcome {
    pub content: String,
    pub is_error: bool,
    pub summary: String,
    pub warning: Option<String>,
    pub canceled: bool,
}

impl ToolOutcome {
    pub fn ok(content: impl Into<String>, summary: impl Into<String>) -> ToolOutcome {
        ToolOutcome {
            content: content.into(),
            is_error: false,
            summary: summary.into(),
            warning: None,
            canceled: false,
        }
    }

    pub fn err(content: impl Into<String>) -> ToolOutcome {
        ToolOutcome {
            content: content.into(),
            is_error: true,
            summary: "error".to_string(),
            warning: None,
            canceled: false,
        }
    }

    /// A call the scheduler skipped because the user interrupted the batch.
    pub fn canceled() -> ToolOutcome {
        Self::canceled_with("canceled by user")
    }

    /// A running tool that observed cancellation, optionally preserving
    /// useful partial output while keeping cancellation structural.
    pub fn canceled_with(content: impl Into<String>) -> ToolOutcome {
        ToolOutcome {
            content: content.into(),
            is_error: true,
            summary: "canceled".to_string(),
            warning: None,
            canceled: true,
        }
    }
}

/// Read-only calls run concurrently; anything else is a sequential barrier.
pub fn is_read_only(name: &str) -> bool {
    matches!(
        name,
        "read" | "grep" | "glob" | "ls" | "context" | "skill" | "mcp_connect"
    )
}

/// The read-only tool SET (plan mode and read-only children): exploration
/// plus skills. Narrower than `is_read_only` on purpose: mcp_connect is
/// safe to parallelize but pointless without mcp_call, so it stays out.
pub const READ_ONLY_SET: &[&str] = &["read", "grep", "glob", "ls", "context", "skill"];

/// Workspace-nonmutating research child set. Unlike plan mode, this may call
/// the one configured web-search MCP server, but it cannot run Bash, change
/// files, alter the plan, or delegate again.
pub const WEB_RESEARCH_SET: &[&str] = &[
    "read",
    "grep",
    "glob",
    "ls",
    "context",
    "skill",
    "mcp_connect",
    "mcp_call",
];

/// Execute one tool call. `args` is the parsed arguments object.
pub fn dispatch(ctx: &ToolCtx, name: &str, args: &Value) -> ToolOutcome {
    // File-tool mutations take an OS lock on the mounted directory. Bash is
    // deliberately not leased: builds, tests, searches, and status commands
    // must remain usable while agents work, and no shell parser can reliably
    // classify arbitrary scripts. The system contract tells agents to make
    // source changes through write/edit, where this guard is enforceable.
    let _workspace_lease = if matches!(name, "write" | "edit") {
        let depth = std::env::var("NOOB_DEPTH")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0);
        let wait = if depth == 0 {
            std::time::Duration::ZERO
        } else {
            std::time::Duration::from_secs(30)
        };
        match guard::workspace_write_lease(&ctx.workspace, wait, || {
            noob_provider::http::INTERRUPTED.load(Ordering::SeqCst)
        }) {
            Ok(lease) => Some(lease),
            Err(guard::WorkspaceLeaseError::Canceled) => return ToolOutcome::canceled(),
            Err(guard::WorkspaceLeaseError::Busy) => {
                return ToolOutcome::err(
                    "workspace write blocked: another parent or sub-agent mutation is active; \
                     continue read-only, wait for it to finish, or cancel the relevant agent \
                     with /agents cancel <agent-N>",
                );
            }
            Err(guard::WorkspaceLeaseError::Io(error)) => {
                return ToolOutcome::err(format!(
                    "cannot lock the workspace before {name}: {error}; no files were changed"
                ));
            }
        }
    } else {
        None
    };
    dispatch_unlocked(ctx, name, args)
}

fn dispatch_unlocked(ctx: &ToolCtx, name: &str, args: &Value) -> ToolOutcome {
    match name {
        "read" => read::run(ctx, args),
        "write" => write::run(ctx, args),
        "edit" => edit::run(ctx, args),
        "bash" => bash::run(ctx, args),
        "context" => context::run(ctx, args),
        "grep" => grep::run(ctx, args),
        "glob" => glob::run(ctx, args),
        "ls" => ls::run(ctx, args),
        "skill" => skill::run(ctx, args),
        // `todo` accepts historical/replayed calls; only `plan` is registered.
        "plan" | "todo" => todo::run(ctx, args),
        "mcp_connect" => mcp::run_connect(ctx, args),
        "mcp_call" => mcp::run_call(ctx, args),
        "subagent" => crate::subagent::run(ctx, args),
        other => ToolOutcome::err(format!(
            "unknown tool {other:?}; the available tools are listed in your tool schemas"
        )),
    }
}

/// The core tool schemas, registered at session start and byte-stable for
/// the whole session (both bootstrap sites start from this set). Descriptions
/// <= 20 words each; the serialized array is budget-tested against the
/// 940-token ceiling.
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
        context::spec(),
        todo::spec(),
    ]
}

// --- shared argument helpers -------------------------------------------------

pub(crate) fn need_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    match args.get(key) {
        Some(Value::String(s)) => Ok(s),
        Some(other) => Err(format!(
            "parameter {key:?} must be a string, got {other}; resend the call"
        )),
        None => Err(format!(
            "missing required parameter {key:?}; resend the call"
        )),
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
        for t in [
            "read",
            "grep",
            "glob",
            "ls",
            "context",
            "skill",
            "mcp_connect",
        ] {
            assert!(is_read_only(t), "{t} must be read-only");
        }
        // todo mutates shared state, so it is a sequential barrier, never a
        // concurrent read-only call.
        for t in [
            "write", "edit", "bash", "mcp_call", "subagent", "plan", "todo",
        ] {
            assert!(!is_read_only(t), "{t} must be a barrier");
        }
    }

    #[test]
    fn core_specs_include_context_and_todo_with_short_descriptions() {
        let specs = specs();
        assert_eq!(specs.len(), 9);
        assert!(
            specs.iter().any(|s| s.name == "context"),
            "context must be a core spec"
        );
        assert!(
            specs.iter().any(|s| s.name == "plan"),
            "plan must be a core spec"
        );
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
        assert_eq!(
            opt_u64(&json!({"offset": "12"}), "offset").unwrap(),
            Some(12)
        );
        assert_eq!(opt_u64(&json!({"offset": 12}), "offset").unwrap(), Some(12));
        assert!(opt_u64(&json!({"offset": -3}), "offset").is_err());
    }

    #[test]
    fn nested_agents_delegate_without_multiplying_the_root_fanout_cap() {
        let (_tmp, mut ctx) = test_ctx();
        let cfg = crate::subagent::TaskCfg {
            depth: 0,
            concurrency: 4,
            max_turns: 25,
            wall_clock: std::time::Duration::from_secs(300),
            verbose: false,
            overrides: noob_provider::types::Overrides::default(),
            yolo: false,
            ancestor_skills: Vec::new(),
            background: None,
        };
        ctx.task = Some(cfg.clone());
        assert_eq!(ctx.task_concurrency(), 4);

        ctx.task = Some(crate::subagent::TaskCfg { depth: 1, ..cfg });
        assert_eq!(ctx.task_concurrency(), 1);
        assert!(ctx.task.is_some(), "nested delegation remains registered");
    }

    #[test]
    fn writable_background_lease_refuses_a_conflicting_parent_write() {
        let (_tmp, mut ctx) = test_ctx();
        ctx.task = Some(crate::subagent::TaskCfg {
            depth: 0,
            concurrency: 2,
            max_turns: 25,
            wall_clock: std::time::Duration::from_secs(300),
            verbose: false,
            overrides: noob_provider::types::Overrides::default(),
            yolo: false,
            ancestor_skills: Vec::new(),
            background: None,
        });
        let child_lease =
            guard::workspace_write_lease(&ctx.workspace, std::time::Duration::ZERO, || false)
                .unwrap();
        let blocked = dispatch(
            &ctx,
            "write",
            &json!({"path": "race.txt", "content": "parent"}),
        );
        assert!(blocked.is_error);
        assert!(
            blocked
                .content
                .contains("another parent or sub-agent mutation")
        );
        assert!(!ctx.workspace.join("race.txt").exists());

        let concurrent_bash = dispatch(&ctx, "bash", &json!({"cmd": "pwd"}));
        assert!(
            !concurrent_bash.is_error,
            "read/build/test Bash must not contend with the file mutation lease: {}",
            concurrent_bash.content
        );

        drop(child_lease);
        let written = dispatch(
            &ctx,
            "write",
            &json!({"path": "race.txt", "content": "parent"}),
        );
        assert!(!written.is_error, "{}", written.content);
        assert_eq!(
            std::fs::read_to_string(ctx.workspace.join("race.txt")).unwrap(),
            "parent"
        );
    }
}
