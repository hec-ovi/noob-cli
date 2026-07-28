//! The contract between the agent and anything watching it.
//!
//! One newline-delimited JSON frame per line, in both directions: `Event` from
//! the agent outward, `Command` inward. Nothing else crosses. A front end that
//! speaks this never links against the agent, and the agent never knows a front
//! end exists, so either side can be replaced whole.
//!
//! Three rules hold this together.
//!
//! **Every frame carries a version.** [`VERSION`] is bumped for additive
//! changes and the reader accepts anything less than or equal to its own.
//! A breaking change ships a new frame type beside the old one and retires the
//! old one after callers migrate; frames are never redefined in place.
//!
//! **An unknown frame is skipped, never guessed at.** Both enums carry an
//! `Unknown` catch-all, so a newer agent talking to an older front end degrades
//! to missing features instead of a parse error that kills the stream. The same
//! rule applies one level down: [`AgentState`] has an `Unknown` variant so a
//! state nobody has heard of costs one field, not the whole frame.
//!
//! **Correlation is explicit.** Every tool frame carries the same `call_id`
//! from start to end. The stream this replaces put an id on the result and not
//! on the call, so pairing them meant counting in order, which is wrong the
//! moment anything runs concurrently. Everything here that can be concurrent
//! carries the id that identifies it.
//!
//! ## Why the serialization is written out by hand
//!
//! `#[derive(Serialize)]` costs five crates: `serde_derive`, `proc-macro2`,
//! `quote`, `syn` and `unicode-ident`. The binary is capped at 45 runtime
//! crates and 8 MiB, and those are published claims rather than preferences, so
//! deriving here took the graph to 47 and the gate refused it. `serde_json` is
//! already in the graph, so writing to a `String` and reading from a
//! `serde_json::Value` costs nothing new. It also removes the one thing derive
//! was buying that mattered, which was field order: `v` and `t` are written
//! first, so a reader can classify a line before it parses the rest, and a
//! human tailing the file can see what each line is.
//!
//! The compiler still checks the writing half exhaustively, since every
//! `write_fields` is a match over the enum. The reading half is not checked by
//! anything but the round-trip tests, which is why they cover every variant of
//! both enums rather than a sample.

use std::fmt::Write as _;

/// Re-exported because [`Event::ToolStart`] carries one. A consumer that has to
/// add `serde_json` to read a field of a frame does not have a self-contained
/// contract.
pub use serde_json::Value;

/// Protocol version. Additive changes bump this; readers accept `<=` their own.
pub const VERSION: u16 = 1;

/// One frame on the wire: a version and a body.
///
/// `v` is outside the body so a reader can check it before it knows whether it
/// understands the type, which is what makes an old reader safe against a new
/// writer.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame<T> {
    pub v: u16,
    pub body: T,
}

impl<T> Frame<T> {
    pub fn new(body: T) -> Frame<T> {
        Frame { v: VERSION, body }
    }

    /// Is this frame from a writer this reader can understand?
    pub fn readable(&self) -> bool {
        self.v <= VERSION
    }
}

/// Something that can travel as a frame body: [`Event`] or [`Command`].
///
/// Implemented by hand rather than derived. See the module docs for why.
pub trait Body: Sized {
    /// The value of the `t` field that names this variant.
    fn tag(&self) -> &'static str;

    /// Append this body's fields to an object that already holds `v` and `t`.
    /// Every field starts with its own comma, so an empty body writes nothing.
    fn write_fields(&self, out: &mut String);

    /// Read a body out of a parsed frame. Never fails: an unrecognised `t`
    /// gives `Unknown`, and a missing field gives that field's zero value, so
    /// one malformed frame cannot end a session.
    fn from_value(value: &Value) -> Self;
}

/// Why a tool call failed, as fields rather than prose.
///
/// The prose version of this is what made a failed command render as
/// `exit code 1`: the interesting part was buried in a body nobody parsed, and
/// the caller had to guess which line mattered. `remedy` exists because the
/// model's next action is the thing a failure most needs to determine, and
/// inferring it from a message is work every consumer would otherwise repeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolError {
    /// Coarse class, for routing and display: `not_found`, `denied`,
    /// `timeout`, `canceled`, `exit_status`, `invalid_argument`, `internal`.
    pub kind: String,
    /// Machine detail where one exists, such as a process exit status.
    pub code: Option<i32>,
    /// One line, already the most useful line rather than the first one.
    pub message: String,
    /// The rest, unbounded and unsummarized. Consumers decide how much to show.
    pub detail: Option<String>,
    /// What to do next, when that is knowable.
    pub remedy: Option<String>,
}

impl ToolError {
    fn to_json(&self) -> String {
        let mut out = String::new();
        out.push('{');
        push_key(&mut out, "kind");
        push_json_str(&mut out, &self.kind);
        f_opt_i64(&mut out, "code", self.code.map(i64::from));
        f_str(&mut out, "message", &self.message);
        f_opt_str(&mut out, "detail", &self.detail);
        f_opt_str(&mut out, "remedy", &self.remedy);
        out.push('}');
        out
    }

    fn read(value: &Value) -> ToolError {
        ToolError {
            kind: get_str(value, "kind"),
            code: get_i64(value, "code").map(|n| n as i32),
            message: get_str(value, "message"),
            detail: get_opt_str(value, "detail"),
            remedy: get_opt_str(value, "remedy"),
        }
    }
}

/// What one request cost.
///
/// `cached_prompt` is separate from `prompt` because summing raw prompt tokens
/// across a session counts work nobody did: every request resends the whole
/// transcript and the endpoint serves most of it from cache. Prefill is
/// `prompt - cached_prompt`, and that is the number that means anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub prompt: u64,
    pub cached_prompt: u64,
    pub completion: u64,
    /// The configured context window, so a consumer can render a budget
    /// without knowing how the agent is configured.
    pub context_total: u64,
}

impl Usage {
    /// Tokens the endpoint actually had to compute for this request.
    pub fn prefilled(&self) -> u64 {
        self.prompt.saturating_sub(self.cached_prompt)
    }

    fn to_json(self) -> String {
        let mut out = String::new();
        out.push('{');
        push_key(&mut out, "prompt");
        push_u64(&mut out, self.prompt);
        f_u64(&mut out, "cached_prompt", self.cached_prompt);
        f_u64(&mut out, "completion", self.completion);
        f_u64(&mut out, "context_total", self.context_total);
        out.push('}');
        out
    }

    fn read(value: &Value) -> Usage {
        Usage {
            prompt: get_u64(value, "prompt"),
            cached_prompt: get_u64(value, "cached_prompt"),
            completion: get_u64(value, "completion"),
            context_total: get_u64(value, "context_total"),
        }
    }
}

/// A span of a file, 1-based and inclusive, as line numbers.
///
/// Lines rather than byte offsets, and deliberately not tied to how the span
/// was found: a scanner, a real parser, or a human cursor all produce the same
/// thing, so the consumer stays indifferent to that choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    /// What the span is, when known: `fn`, `class`, `section`, `selection`.
    pub kind: Option<String>,
    pub name: Option<String>,
}

impl Span {
    fn to_json(&self) -> String {
        let mut out = String::new();
        out.push('{');
        push_key(&mut out, "start");
        push_u64(&mut out, u64::from(self.start));
        f_u64(&mut out, "end", u64::from(self.end));
        f_opt_str(&mut out, "kind", &self.kind);
        f_opt_str(&mut out, "name", &self.name);
        out.push('}');
        out
    }

    fn read(value: &Value) -> Span {
        Span {
            start: get_u32(value, "start"),
            end: get_u32(value, "end"),
            kind: get_opt_str(value, "kind"),
            name: get_opt_str(value, "name"),
        }
    }
}

/// One measured quantity at one instant.
///
/// Shaped to carry what both reference tools need without deciding how either
/// is drawn. radeontop is a list of labelled bars, each a value against a
/// maximum. btop is the same values sampled repeatedly and drawn as a rolling
/// graph. Both fall out of the same sample: a `max` means the value is a
/// proportion and can be a bar; a stream of samples over time is a series and
/// can be a sparkline. The renderer chooses; the protocol does not.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    /// Stable identifier, so a consumer can follow one quantity across frames
    /// even if its label changes.
    pub key: String,
    /// What to show a human.
    pub label: String,
    pub value: f64,
    /// Full scale, when the quantity has one. Absent means unbounded, which a
    /// bar cannot render and a graph can.
    pub max: Option<f64>,
    pub unit: Option<String>,
}

impl Sample {
    fn to_json(&self) -> String {
        let mut out = String::new();
        out.push('{');
        push_key(&mut out, "key");
        push_json_str(&mut out, &self.key);
        f_str(&mut out, "label", &self.label);
        f_f64(&mut out, "value", self.value);
        f_opt_f64(&mut out, "max", self.max);
        f_opt_str(&mut out, "unit", &self.unit);
        out.push('}');
        out
    }

    fn read(value: &Value) -> Sample {
        Sample {
            key: get_str(value, "key"),
            label: get_str(value, "label"),
            value: get_f64(value, "value").unwrap_or(0.0),
            max: get_f64(value, "max"),
            unit: get_opt_str(value, "unit"),
        }
    }
}

/// How a sub-agent is doing.
///
/// `Unknown` is the same degradation rule the frame types have, one level down:
/// a state a newer agent invented costs this one field rather than the frame it
/// arrived in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Queued,
    Running,
    Done,
    Failed,
    Canceled,
    Unknown,
}

impl AgentState {
    pub fn as_str(&self) -> &'static str {
        match self {
            AgentState::Queued => "queued",
            AgentState::Running => "running",
            AgentState::Done => "done",
            AgentState::Failed => "failed",
            AgentState::Canceled => "canceled",
            AgentState::Unknown => "unknown",
        }
    }

    fn read(value: &Value, key: &str) -> AgentState {
        match value.get(key).and_then(Value::as_str).unwrap_or_default() {
            "queued" => AgentState::Queued,
            "running" => AgentState::Running,
            "done" => AgentState::Done,
            "failed" => AgentState::Failed,
            "canceled" => AgentState::Canceled,
            _ => AgentState::Unknown,
        }
    }
}

/// Agent to front end.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    SessionStart {
        id: String,
        workspace: String,
        model: String,
        resumed: bool,
    },
    SessionEnd {
        id: String,
    },

    TurnStart {
        turn: u32,
    },
    TurnEnd {
        turn: u32,
        interrupted: Option<bool>,
    },

    TextDelta {
        d: String,
    },
    /// Ephemeral. Never enters a transcript, and a consumer may drop it.
    ReasoningDelta {
        d: String,
    },

    ToolStart {
        call_id: String,
        name: String,
        /// The short human label, already built by the agent so every consumer
        /// renders the same words.
        brief: String,
        args: Value,
    },
    ToolProgress {
        call_id: String,
        line: String,
    },
    ToolEnd {
        call_id: String,
        summary: String,
        elapsed_ms: u64,
        error: Option<ToolError>,
    },

    /// The agent is now looking at this file. Drives the code pane.
    ///
    /// Every file frame carries the tool call that caused it. Read-only tools
    /// run up to eight at a time, so without this their frames interleave and
    /// a consumer cannot tell which read produced which. Optional because a
    /// file event can also come from outside any call, such as compaction
    /// dropping what the agent had seen.
    FileOpen {
        path: String,
        lines: u32,
        call_id: Option<String>,
    },
    /// The part of that file it is about to work on.
    FileSpan {
        path: String,
        span: Span,
        call_id: Option<String>,
    },
    /// A range that just changed, with both sides, so a diff needs no guessing
    /// and no second read of the file.
    FileEdit {
        path: String,
        span: Span,
        before: String,
        after: String,
        call_id: Option<String>,
    },
    FileClose {
        path: String,
        call_id: Option<String>,
    },

    AgentSpawn {
        agent_id: String,
        prompt: String,
        tools: String,
    },
    AgentStateChanged {
        agent_id: String,
        state: AgentState,
        detail: Option<String>,
    },
    AgentOutput {
        agent_id: String,
        line: String,
    },

    SkillList {
        names: Vec<String>,
    },
    McpList {
        names: Vec<String>,
    },
    McpState {
        name: String,
        connected: bool,
        tools: Option<u32>,
    },

    UsageReport {
        usage: Usage,
    },

    /// A batch of measurements taken together.
    ///
    /// Deliberately generic: GPU counters, token economy and anything measured
    /// later all travel this frame, so adding a new readout is a new `group`
    /// rather than a new frame type and a protocol bump. `at_ms` is what lets a
    /// consumer assemble successive frames into a time series without needing
    /// its own clock.
    Metrics {
        group: String,
        at_ms: u64,
        samples: Vec<Sample>,
    },

    Note {
        line: String,
    },
    Error {
        line: String,
    },

    /// A frame this reader does not know. Present so a newer writer degrades to
    /// missing features rather than a dead stream.
    Unknown,
}

impl Body for Event {
    fn tag(&self) -> &'static str {
        match self {
            Event::SessionStart { .. } => "session.start",
            Event::SessionEnd { .. } => "session.end",
            Event::TurnStart { .. } => "turn.start",
            Event::TurnEnd { .. } => "turn.end",
            Event::TextDelta { .. } => "text.delta",
            Event::ReasoningDelta { .. } => "reasoning.delta",
            Event::ToolStart { .. } => "tool.start",
            Event::ToolProgress { .. } => "tool.progress",
            Event::ToolEnd { .. } => "tool.end",
            Event::FileOpen { .. } => "file.open",
            Event::FileSpan { .. } => "file.span",
            Event::FileEdit { .. } => "file.edit",
            Event::FileClose { .. } => "file.close",
            Event::AgentSpawn { .. } => "agent.spawn",
            Event::AgentStateChanged { .. } => "agent.state",
            Event::AgentOutput { .. } => "agent.output",
            Event::SkillList { .. } => "skill.list",
            Event::McpList { .. } => "mcp.list",
            Event::McpState { .. } => "mcp.state",
            Event::UsageReport { .. } => "usage",
            Event::Metrics { .. } => "metrics",
            Event::Note { .. } => "note",
            Event::Error { .. } => "error",
            Event::Unknown => "unknown",
        }
    }

    fn write_fields(&self, out: &mut String) {
        match self {
            Event::SessionStart {
                id,
                workspace,
                model,
                resumed,
            } => {
                f_str(out, "id", id);
                f_str(out, "workspace", workspace);
                f_str(out, "model", model);
                f_bool(out, "resumed", *resumed);
            }
            Event::SessionEnd { id } => f_str(out, "id", id),
            Event::TurnStart { turn } => f_u64(out, "turn", u64::from(*turn)),
            Event::TurnEnd { turn, interrupted } => {
                f_u64(out, "turn", u64::from(*turn));
                f_opt_bool(out, "interrupted", *interrupted);
            }
            Event::TextDelta { d } | Event::ReasoningDelta { d } => f_str(out, "d", d),
            Event::ToolStart {
                call_id,
                name,
                brief,
                args,
            } => {
                f_str(out, "call_id", call_id);
                f_str(out, "name", name);
                f_str(out, "brief", brief);
                f_raw(out, "args", &args.to_string());
            }
            Event::ToolProgress { call_id, line } => {
                f_str(out, "call_id", call_id);
                f_str(out, "line", line);
            }
            Event::ToolEnd {
                call_id,
                summary,
                elapsed_ms,
                error,
            } => {
                f_str(out, "call_id", call_id);
                f_str(out, "summary", summary);
                f_u64(out, "elapsed_ms", *elapsed_ms);
                if let Some(error) = error {
                    f_raw(out, "error", &error.to_json());
                }
            }
            Event::FileOpen {
                path,
                lines,
                call_id,
            } => {
                f_str(out, "path", path);
                f_u64(out, "lines", u64::from(*lines));
                f_opt_str(out, "call_id", call_id);
            }
            Event::FileSpan {
                path,
                span,
                call_id,
            } => {
                f_str(out, "path", path);
                f_raw(out, "span", &span.to_json());
                f_opt_str(out, "call_id", call_id);
            }
            Event::FileEdit {
                path,
                span,
                before,
                after,
                call_id,
            } => {
                f_str(out, "path", path);
                f_raw(out, "span", &span.to_json());
                f_str(out, "before", before);
                f_str(out, "after", after);
                f_opt_str(out, "call_id", call_id);
            }
            Event::FileClose { path, call_id } => {
                f_str(out, "path", path);
                f_opt_str(out, "call_id", call_id);
            }
            Event::AgentSpawn {
                agent_id,
                prompt,
                tools,
            } => {
                f_str(out, "agent_id", agent_id);
                f_str(out, "prompt", prompt);
                f_str(out, "tools", tools);
            }
            Event::AgentStateChanged {
                agent_id,
                state,
                detail,
            } => {
                f_str(out, "agent_id", agent_id);
                f_str(out, "state", state.as_str());
                f_opt_str(out, "detail", detail);
            }
            Event::AgentOutput { agent_id, line } => {
                f_str(out, "agent_id", agent_id);
                f_str(out, "line", line);
            }
            Event::SkillList { names } | Event::McpList { names } => {
                f_str_list(out, "names", names)
            }
            Event::McpState {
                name,
                connected,
                tools,
            } => {
                f_str(out, "name", name);
                f_bool(out, "connected", *connected);
                f_opt_i64(out, "tools", tools.map(i64::from));
            }
            Event::UsageReport { usage } => f_raw(out, "usage", &usage.to_json()),
            Event::Metrics {
                group,
                at_ms,
                samples,
            } => {
                f_str(out, "group", group);
                f_u64(out, "at_ms", *at_ms);
                f_sample_list(out, "samples", samples);
            }
            Event::Note { line } | Event::Error { line } => f_str(out, "line", line),
            Event::Unknown => {}
        }
    }

    fn from_value(value: &Value) -> Event {
        match value.get("t").and_then(Value::as_str).unwrap_or_default() {
            "session.start" => Event::SessionStart {
                id: get_str(value, "id"),
                workspace: get_str(value, "workspace"),
                model: get_str(value, "model"),
                resumed: get_bool(value, "resumed").unwrap_or(false),
            },
            "session.end" => Event::SessionEnd {
                id: get_str(value, "id"),
            },
            "turn.start" => Event::TurnStart {
                turn: get_u32(value, "turn"),
            },
            "turn.end" => Event::TurnEnd {
                turn: get_u32(value, "turn"),
                interrupted: get_bool(value, "interrupted"),
            },
            "text.delta" => Event::TextDelta {
                d: get_str(value, "d"),
            },
            "reasoning.delta" => Event::ReasoningDelta {
                d: get_str(value, "d"),
            },
            "tool.start" => Event::ToolStart {
                call_id: get_str(value, "call_id"),
                name: get_str(value, "name"),
                brief: get_str(value, "brief"),
                args: value.get("args").cloned().unwrap_or(Value::Null),
            },
            "tool.progress" => Event::ToolProgress {
                call_id: get_str(value, "call_id"),
                line: get_str(value, "line"),
            },
            "tool.end" => Event::ToolEnd {
                call_id: get_str(value, "call_id"),
                summary: get_str(value, "summary"),
                elapsed_ms: get_u64(value, "elapsed_ms"),
                error: value.get("error").filter(|e| e.is_object()).map(ToolError::read),
            },
            "file.open" => Event::FileOpen {
                path: get_str(value, "path"),
                lines: get_u32(value, "lines"),
                call_id: get_opt_str(value, "call_id"),
            },
            "file.span" => Event::FileSpan {
                path: get_str(value, "path"),
                span: get_span(value, "span"),
                call_id: get_opt_str(value, "call_id"),
            },
            "file.edit" => Event::FileEdit {
                path: get_str(value, "path"),
                span: get_span(value, "span"),
                before: get_str(value, "before"),
                after: get_str(value, "after"),
                call_id: get_opt_str(value, "call_id"),
            },
            "file.close" => Event::FileClose {
                path: get_str(value, "path"),
                call_id: get_opt_str(value, "call_id"),
            },
            "agent.spawn" => Event::AgentSpawn {
                agent_id: get_str(value, "agent_id"),
                prompt: get_str(value, "prompt"),
                tools: get_str(value, "tools"),
            },
            "agent.state" => Event::AgentStateChanged {
                agent_id: get_str(value, "agent_id"),
                state: AgentState::read(value, "state"),
                detail: get_opt_str(value, "detail"),
            },
            "agent.output" => Event::AgentOutput {
                agent_id: get_str(value, "agent_id"),
                line: get_str(value, "line"),
            },
            "skill.list" => Event::SkillList {
                names: get_str_list(value, "names"),
            },
            "mcp.list" => Event::McpList {
                names: get_str_list(value, "names"),
            },
            "mcp.state" => Event::McpState {
                name: get_str(value, "name"),
                connected: get_bool(value, "connected").unwrap_or(false),
                tools: get_i64(value, "tools").map(|n| n as u32),
            },
            "usage" => Event::UsageReport {
                usage: value.get("usage").map(Usage::read).unwrap_or_default(),
            },
            "metrics" => Event::Metrics {
                group: get_str(value, "group"),
                at_ms: get_u64(value, "at_ms"),
                samples: value
                    .get("samples")
                    .and_then(Value::as_array)
                    .map(|items| items.iter().map(Sample::read).collect())
                    .unwrap_or_default(),
            },
            "note" => Event::Note {
                line: get_str(value, "line"),
            },
            "error" => Event::Error {
                line: get_str(value, "line"),
            },
            _ => Event::Unknown,
        }
    }
}

/// Front end to agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    PromptSubmit {
        text: String,
    },
    /// Queue behind the running turn instead of interrupting it.
    PromptQueue {
        text: String,
    },
    TurnCancel,

    AgentCancel {
        agent_id: String,
    },

    SkillAdd {
        source: String,
    },
    SkillRemove {
        name: String,
    },
    McpAdd {
        name: String,
        spec: String,
    },
    McpRemove {
        name: String,
    },
    McpConnect {
        name: String,
    },

    ConfigSet {
        key: String,
        value: String,
    },
    ConfigUnset {
        key: String,
    },

    SessionList,
    SessionOpen {
        id: String,
    },

    Unknown,
}

impl Body for Command {
    fn tag(&self) -> &'static str {
        match self {
            Command::PromptSubmit { .. } => "prompt.submit",
            Command::PromptQueue { .. } => "prompt.queue",
            Command::TurnCancel => "turn.cancel",
            Command::AgentCancel { .. } => "agent.cancel",
            Command::SkillAdd { .. } => "skill.add",
            Command::SkillRemove { .. } => "skill.remove",
            Command::McpAdd { .. } => "mcp.add",
            Command::McpRemove { .. } => "mcp.remove",
            Command::McpConnect { .. } => "mcp.connect",
            Command::ConfigSet { .. } => "config.set",
            Command::ConfigUnset { .. } => "config.unset",
            Command::SessionList => "session.list",
            Command::SessionOpen { .. } => "session.open",
            Command::Unknown => "unknown",
        }
    }

    fn write_fields(&self, out: &mut String) {
        match self {
            Command::PromptSubmit { text } | Command::PromptQueue { text } => {
                f_str(out, "text", text)
            }
            Command::TurnCancel | Command::SessionList | Command::Unknown => {}
            Command::AgentCancel { agent_id } => f_str(out, "agent_id", agent_id),
            Command::SkillAdd { source } => f_str(out, "source", source),
            Command::SkillRemove { name }
            | Command::McpRemove { name }
            | Command::McpConnect { name } => f_str(out, "name", name),
            Command::McpAdd { name, spec } => {
                f_str(out, "name", name);
                f_str(out, "spec", spec);
            }
            Command::ConfigSet { key, value } => {
                f_str(out, "key", key);
                f_str(out, "value", value);
            }
            Command::ConfigUnset { key } => f_str(out, "key", key),
            Command::SessionOpen { id } => f_str(out, "id", id),
        }
    }

    fn from_value(value: &Value) -> Command {
        match value.get("t").and_then(Value::as_str).unwrap_or_default() {
            "prompt.submit" => Command::PromptSubmit {
                text: get_str(value, "text"),
            },
            "prompt.queue" => Command::PromptQueue {
                text: get_str(value, "text"),
            },
            "turn.cancel" => Command::TurnCancel,
            "agent.cancel" => Command::AgentCancel {
                agent_id: get_str(value, "agent_id"),
            },
            "skill.add" => Command::SkillAdd {
                source: get_str(value, "source"),
            },
            "skill.remove" => Command::SkillRemove {
                name: get_str(value, "name"),
            },
            "mcp.add" => Command::McpAdd {
                name: get_str(value, "name"),
                spec: get_str(value, "spec"),
            },
            "mcp.remove" => Command::McpRemove {
                name: get_str(value, "name"),
            },
            "mcp.connect" => Command::McpConnect {
                name: get_str(value, "name"),
            },
            "config.set" => Command::ConfigSet {
                key: get_str(value, "key"),
                value: get_str(value, "value"),
            },
            "config.unset" => Command::ConfigUnset {
                key: get_str(value, "key"),
            },
            "session.list" => Command::SessionList,
            "session.open" => Command::SessionOpen {
                id: get_str(value, "id"),
            },
            _ => Command::Unknown,
        }
    }
}

/// Decode one line, or `None` if it is not a frame this reader can use.
///
/// Fails closed and silently by design: a malformed or too-new line is skipped
/// so one bad frame cannot end a session. A line that parses but names a type
/// this reader does not know decodes to `Unknown` rather than `None`, which is
/// the difference between a feature this front end lacks and a stream it cannot
/// read.
pub fn decode<T: Body>(line: &str) -> Option<Frame<T>> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    let v = u16::try_from(value.get("v").and_then(Value::as_u64)?).ok()?;
    let frame = Frame {
        v,
        body: T::from_value(&value),
    };
    frame.readable().then_some(frame)
}

/// Encode one frame as a line, newline included.
pub fn encode<T: Body>(body: &T) -> String {
    let mut line = String::with_capacity(96);
    line.push_str("{\"v\":");
    push_u64(&mut line, u64::from(VERSION));
    line.push_str(",\"t\":");
    push_json_str(&mut line, body.tag());
    body.write_fields(&mut line);
    line.push_str("}\n");
    line
}

// ---------------------------------------------------------------------------
// Writing. Keys are identifiers this crate controls, so they skip escaping;
// values never do.
// ---------------------------------------------------------------------------

fn push_key(out: &mut String, key: &str) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
}

/// Quote and escape one string per RFC 8259.
///
/// Escapes exactly what the grammar requires: the quote, the backslash and
/// everything below 0x20. Every byte it acts on is ASCII, so slicing at those
/// positions is always on a character boundary and multi-byte UTF-8 passes
/// through in one copy.
fn push_json_str(out: &mut String, s: &str) {
    out.push('"');
    let mut start = 0;
    for (i, byte) in s.bytes().enumerate() {
        let escape = match byte {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            b'\n' => "\\n",
            b'\r' => "\\r",
            b'\t' => "\\t",
            0x00..=0x1f => "",
            _ => continue,
        };
        out.push_str(&s[start..i]);
        if escape.is_empty() {
            let _ = write!(out, "\\u{byte:04x}");
        } else {
            out.push_str(escape);
        }
        start = i + 1;
    }
    out.push_str(&s[start..]);
    out.push('"');
}

fn push_u64(out: &mut String, value: u64) {
    let _ = write!(out, "{value}");
}

/// Non-finite becomes `null`, since JSON has no NaN or infinity and a frame
/// that cannot be parsed is worse than a reading that is missing.
fn push_f64(out: &mut String, value: f64) {
    if value.is_finite() {
        let _ = write!(out, "{value}");
    } else {
        out.push_str("null");
    }
}

fn f_str(out: &mut String, key: &str, value: &str) {
    out.push(',');
    push_key(out, key);
    push_json_str(out, value);
}

fn f_opt_str(out: &mut String, key: &str, value: &Option<String>) {
    if let Some(value) = value {
        f_str(out, key, value);
    }
}

fn f_u64(out: &mut String, key: &str, value: u64) {
    out.push(',');
    push_key(out, key);
    push_u64(out, value);
}

fn f_opt_i64(out: &mut String, key: &str, value: Option<i64>) {
    if let Some(value) = value {
        out.push(',');
        push_key(out, key);
        let _ = write!(out, "{value}");
    }
}

fn f_f64(out: &mut String, key: &str, value: f64) {
    out.push(',');
    push_key(out, key);
    push_f64(out, value);
}

fn f_opt_f64(out: &mut String, key: &str, value: Option<f64>) {
    if let Some(value) = value {
        f_f64(out, key, value);
    }
}

fn f_bool(out: &mut String, key: &str, value: bool) {
    out.push(',');
    push_key(out, key);
    out.push_str(if value { "true" } else { "false" });
}

fn f_opt_bool(out: &mut String, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        f_bool(out, key, value);
    }
}

/// A value that is already rendered JSON: a nested object, or `args`.
fn f_raw(out: &mut String, key: &str, rendered: &str) {
    out.push(',');
    push_key(out, key);
    out.push_str(rendered);
}

fn f_str_list(out: &mut String, key: &str, values: &[String]) {
    out.push(',');
    push_key(out, key);
    out.push('[');
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_json_str(out, value);
    }
    out.push(']');
}

fn f_sample_list(out: &mut String, key: &str, values: &[Sample]) {
    out.push(',');
    push_key(out, key);
    out.push('[');
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&value.to_json());
    }
    out.push(']');
}

// ---------------------------------------------------------------------------
// Reading. A missing or wrongly typed field is that field's zero value, never
// an error, so a frame is always usable in part.
// ---------------------------------------------------------------------------

fn get_str(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn get_opt_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn get_u64(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or_default()
}

fn get_u32(value: &Value, key: &str) -> u32 {
    u32::try_from(get_u64(value, key)).unwrap_or(u32::MAX)
}

fn get_i64(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

fn get_f64(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
}

fn get_bool(value: &Value, key: &str) -> Option<bool> {
    value.get(key).and_then(Value::as_bool)
}

fn get_str_list(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn get_span(value: &Value, key: &str) -> Span {
    value.get(key).map(Span::read).unwrap_or(Span {
        start: 0,
        end: 0,
        kind: None,
        name: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T: Body + std::fmt::Debug + PartialEq>(body: T) {
        let line = encode(&body);
        assert!(line.ends_with('\n'), "frames are newline delimited");
        serde_json::from_str::<Value>(&line).unwrap_or_else(|e| panic!("invalid JSON {line}: {e}"));
        let back: Frame<T> = decode(&line).expect("a frame we just wrote must decode");
        assert_eq!(back.v, VERSION);
        assert_eq!(back.body, body, "wrote {line}");
    }

    fn span() -> Span {
        Span {
            start: 10,
            end: 14,
            kind: Some("fn".into()),
            name: Some("add".into()),
        }
    }

    /// Every variant, not a sample. Writing is checked by the compiler because
    /// `write_fields` is an exhaustive match; reading is checked by nothing but
    /// this, so a key typed one way in one half and another way in the other is
    /// only caught here.
    #[test]
    fn every_event_variant_survives_a_round_trip() {
        let every = [
            Event::SessionStart {
                id: "s1".into(),
                workspace: "/work".into(),
                model: "laguna".into(),
                resumed: false,
            },
            Event::SessionEnd { id: "s1".into() },
            Event::TurnStart { turn: 3 },
            Event::TurnEnd {
                turn: 3,
                interrupted: Some(true),
            },
            Event::TurnEnd {
                turn: 4,
                interrupted: None,
            },
            Event::TextDelta { d: "hello".into() },
            Event::ReasoningDelta {
                d: "thinking".into(),
            },
            Event::ToolStart {
                call_id: "c1".into(),
                name: "bash".into(),
                brief: "cargo test".into(),
                args: serde_json::json!({"cmd": "cargo test", "n": 2, "deep": [1, {"a": null}]}),
            },
            Event::ToolProgress {
                call_id: "c1".into(),
                line: "compiling".into(),
            },
            Event::ToolEnd {
                call_id: "c1".into(),
                summary: "bash cargo test".into(),
                elapsed_ms: 2100,
                error: None,
            },
            Event::ToolEnd {
                call_id: "c1".into(),
                summary: "bash cargo test".into(),
                elapsed_ms: 2100,
                error: Some(ToolError {
                    kind: "exit_status".into(),
                    code: Some(-1),
                    message: "killed".into(),
                    detail: None,
                    remedy: None,
                }),
            },
            Event::FileOpen {
                path: "src/a.rs".into(),
                lines: 120,
                call_id: Some("c3".into()),
            },
            Event::FileSpan {
                path: "src/a.rs".into(),
                span: span(),
                call_id: None,
            },
            Event::FileEdit {
                path: "src/a.rs".into(),
                span: span(),
                before: "a - b".into(),
                after: "a + b".into(),
                call_id: Some("c7".into()),
            },
            Event::FileClose {
                path: "src/a.rs".into(),
                call_id: None,
            },
            Event::AgentSpawn {
                agent_id: "agent-1".into(),
                prompt: "audit the parser".into(),
                tools: "read,grep".into(),
            },
            Event::AgentStateChanged {
                agent_id: "agent-1".into(),
                state: AgentState::Running,
                detail: None,
            },
            Event::AgentOutput {
                agent_id: "agent-1".into(),
                line: "found it".into(),
            },
            Event::SkillList {
                names: vec!["websearch".into(), "blockchain".into()],
            },
            Event::McpList { names: vec![] },
            Event::McpState {
                name: "fs".into(),
                connected: true,
                tools: Some(7),
            },
            Event::McpState {
                name: "fs".into(),
                connected: false,
                tools: None,
            },
            Event::UsageReport {
                usage: Usage {
                    prompt: 1832,
                    cached_prompt: 1334,
                    completion: 2,
                    context_total: 65536,
                },
            },
            Event::Metrics {
                group: "gpu".into(),
                at_ms: 1_500,
                samples: vec![],
            },
            Event::Note {
                line: "resumed".into(),
            },
            Event::Error {
                line: "endpoint refused".into(),
            },
            Event::Unknown,
        ];
        for event in every {
            round_trip(event);
        }
    }

    /// Every state, including the one that exists so an unrecognised state
    /// costs a field rather than the frame.
    #[test]
    fn every_agent_state_survives_a_round_trip() {
        for state in [
            AgentState::Queued,
            AgentState::Running,
            AgentState::Done,
            AgentState::Failed,
            AgentState::Canceled,
            AgentState::Unknown,
        ] {
            round_trip(Event::AgentStateChanged {
                agent_id: "a".into(),
                state,
                detail: Some("because".into()),
            });
        }
    }

    #[test]
    fn a_state_from_a_newer_writer_costs_one_field_not_the_frame() {
        let line = r#"{"v":1,"t":"agent.state","agent_id":"a","state":"hibernating"}"#;
        let frame: Frame<Event> = decode(line).expect("the frame still parses");
        assert_eq!(
            frame.body,
            Event::AgentStateChanged {
                agent_id: "a".into(),
                state: AgentState::Unknown,
                detail: None,
            }
        );
    }

    #[test]
    fn every_command_variant_survives_a_round_trip() {
        let every = [
            Command::PromptSubmit {
                text: "do the thing".into(),
            },
            Command::PromptQueue {
                text: "then this".into(),
            },
            Command::TurnCancel,
            Command::AgentCancel {
                agent_id: "agent-2".into(),
            },
            Command::SkillAdd {
                source: "github.com/x/y".into(),
            },
            Command::SkillRemove {
                name: "websearch".into(),
            },
            Command::McpAdd {
                name: "fs".into(),
                spec: "npx -y server".into(),
            },
            Command::McpRemove { name: "fs".into() },
            Command::McpConnect { name: "fs".into() },
            Command::ConfigSet {
                key: "model".into(),
                value: "laguna".into(),
            },
            Command::ConfigUnset { key: "model".into() },
            Command::SessionList,
            Command::SessionOpen { id: "s1".into() },
            Command::Unknown,
        ];
        for command in every {
            round_trip(command);
        }
    }

    /// Hand-written escaping is the one place a JSON bug could hide, so the
    /// nasty cases are exercised directly rather than trusted.
    #[test]
    fn strings_survive_quotes_backslashes_control_bytes_and_unicode() {
        for text in [
            "",
            "plain",
            "he said \"no\"",
            r"C:\path\to\thing",
            "line one\nline two\r\n\ttabbed",
            "null\u{0}bell\u{7}escape\u{1b}[0m",
            "emoji 🦀 and accents éàü and CJK 日本語",
            "\u{7f}delete and \u{a0}nbsp",
            "}\",\"t\":\"forged",
        ] {
            round_trip(Event::TextDelta { d: text.into() });
            round_trip(Event::Note { line: text.into() });
        }
    }

    /// A field the writer never escapes must not be forgeable from a value.
    /// This is the injection case for a hand-written encoder.
    #[test]
    fn a_value_cannot_forge_a_field() {
        let event = Event::TextDelta {
            d: r#"","t":"session.end","id":"pwned"#.into(),
        };
        let line = encode(&event);
        let frame: Frame<Event> = decode(&line).expect("must decode");
        assert_eq!(frame.body, event, "the payload stayed inside its string");
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["t"], "text.delta");
        assert!(value.get("id").is_none());
    }

    /// `v` and `t` lead every line, so a reader can classify a frame before it
    /// parses the rest and a human tailing the file can read it.
    #[test]
    fn the_version_and_type_lead_every_line() {
        let line = encode(&Event::ToolStart {
            call_id: "c1".into(),
            name: "read".into(),
            brief: "src/a.rs".into(),
            args: Value::Null,
        });
        assert!(line.starts_with(r#"{"v":1,"t":"tool.start","#), "{line}");
    }

    /// The correlation fix. Every tool frame carries the same id, so a consumer
    /// pairs a call with its result by identity rather than by counting, which
    /// is the only thing that survives concurrency.
    #[test]
    fn a_tool_call_is_correlated_by_id_from_start_to_end() {
        let start = Event::ToolStart {
            call_id: "abc".into(),
            name: "read".into(),
            brief: "src/a.rs".into(),
            args: Value::Null,
        };
        let progress = Event::ToolProgress {
            call_id: "abc".into(),
            line: "half way".into(),
        };
        let end = Event::ToolEnd {
            call_id: "abc".into(),
            summary: "read src/a.rs".into(),
            elapsed_ms: 12,
            error: None,
        };
        for event in [start, progress, end] {
            let line = encode(&event);
            let value: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(
                value["call_id"], "abc",
                "every tool frame must carry the id: {line}"
            );
        }
    }

    /// A newer agent must not kill an older front end. The unknown frame is
    /// skipped and the stream continues.
    #[test]
    fn an_unknown_frame_type_degrades_instead_of_breaking_the_stream() {
        let line = r#"{"v":1,"t":"something.invented.later","payload":{"a":1}}"#;
        let frame: Frame<Event> = decode(line).expect("an unknown type still parses");
        assert_eq!(frame.body, Event::Unknown);
    }

    /// A frame from a future protocol version is refused rather than
    /// half-understood, because its fields may mean something else.
    #[test]
    fn a_future_version_is_refused() {
        let line = format!(r#"{{"v":{},"t":"text.delta","d":"x"}}"#, VERSION + 1);
        assert!(decode::<Event>(&line).is_none());
    }

    #[test]
    fn a_malformed_line_is_skipped_rather_than_fatal() {
        assert!(decode::<Event>("not json").is_none());
        assert!(decode::<Event>("").is_none());
        assert!(decode::<Event>("   ").is_none());
        assert!(decode::<Event>("[1,2,3]").is_none(), "not an object");
        assert!(decode::<Event>(r#"{"t":"note"}"#).is_none(), "no version");
        assert!(decode::<Event>(r#"{"v":"one"}"#).is_none(), "bad version");
    }

    /// A frame missing the fields its type declares is still usable in part,
    /// rather than dropped whole.
    #[test]
    fn a_frame_missing_fields_reads_as_far_as_it_can() {
        let frame: Frame<Event> = decode(r#"{"v":1,"t":"tool.end","call_id":"c1"}"#).unwrap();
        assert_eq!(
            frame.body,
            Event::ToolEnd {
                call_id: "c1".into(),
                summary: String::new(),
                elapsed_ms: 0,
                error: None,
            }
        );
    }

    /// Summing raw prompt tokens counts work the endpoint never did.
    #[test]
    fn prefill_excludes_what_the_cache_served() {
        let usage = Usage {
            prompt: 1832,
            cached_prompt: 1334,
            completion: 2,
            context_total: 65536,
        };
        assert_eq!(usage.prefilled(), 498);
        let nothing_cached = Usage {
            prompt: 100,
            cached_prompt: 0,
            ..Default::default()
        };
        assert_eq!(nothing_cached.prefilled(), 100);
        // A server that reports more cached than prompt must not underflow.
        let odd = Usage {
            prompt: 10,
            cached_prompt: 99,
            ..Default::default()
        };
        assert_eq!(odd.prefilled(), 0);
    }

    /// A failure is fields, not prose: the whole point of replacing a body
    /// nobody parsed with a shape every consumer can route on.
    #[test]
    fn a_tool_failure_carries_its_parts_separately() {
        let event = Event::ToolEnd {
            call_id: "c9".into(),
            summary: "bash cargo test (2.1s, exit 1)".into(),
            elapsed_ms: 2100,
            error: Some(ToolError {
                kind: "exit_status".into(),
                code: Some(1),
                message: "error: could not compile `noob`".into(),
                detail: Some("... 300 lines ...".into()),
                remedy: Some("fix the type error on line 42".into()),
            }),
        };
        let line = encode(&event);
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["error"]["kind"], "exit_status");
        assert_eq!(value["error"]["code"], 1);
        assert!(value["error"]["remedy"].is_string());
        let back: Frame<Event> = decode(&line).unwrap();
        assert_eq!(back.body, event);
    }

    /// The two reference readouts, carried by one frame shape. radeontop is a
    /// list of bars, so its samples have a `max`; a token rate has none and can
    /// only be a graph. Adding a readout later must not need a protocol bump.
    #[test]
    fn metrics_carry_both_a_bar_and_an_unbounded_series() {
        let event = Event::Metrics {
            group: "gpu".into(),
            at_ms: 1_500,
            samples: vec![
                Sample {
                    key: "graphics_pipe".into(),
                    label: "Graphics pipe".into(),
                    value: 32.5,
                    max: Some(100.0),
                    unit: Some("%".into()),
                },
                Sample {
                    key: "vram".into(),
                    label: "VRAM".into(),
                    value: 1619.0,
                    max: Some(1875.0),
                    unit: Some("MiB".into()),
                },
                Sample {
                    key: "tokens_per_s".into(),
                    label: "generation".into(),
                    value: 47.2,
                    max: None,
                    unit: Some("tok/s".into()),
                },
            ],
        };
        let line = encode(&event);
        let back: Frame<Event> = decode(&line).unwrap();
        assert_eq!(back.body, event);

        let value: Value = serde_json::from_str(&line).unwrap();
        // A bounded sample keeps its scale; an unbounded one omits the field
        // rather than inventing one.
        assert_eq!(value["samples"][1]["max"], 1875.0);
        assert!(value["samples"][2].get("max").is_none());
    }

    /// A counter that has not been read yet, or a rate over zero elapsed time,
    /// must not produce a line no parser accepts.
    #[test]
    fn a_non_finite_reading_stays_valid_json() {
        let line = encode(&Event::Metrics {
            group: "gpu".into(),
            at_ms: 0,
            samples: vec![Sample {
                key: "rate".into(),
                label: "rate".into(),
                value: f64::NAN,
                max: Some(f64::INFINITY),
                unit: None,
            }],
        });
        let value: Value = serde_json::from_str(&line).expect("must still parse");
        assert!(value["samples"][0]["value"].is_null());
    }

    /// Eight reads run concurrently, so a file frame that cannot name its call
    /// is unattributable. The field is optional because compaction closes files
    /// outside any call.
    #[test]
    fn file_frames_carry_the_call_that_caused_them() {
        let line = encode(&Event::FileOpen {
            path: "src/a.rs".into(),
            lines: 120,
            call_id: Some("c3".into()),
        });
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["call_id"], "c3");

        let orphan = encode(&Event::FileClose {
            path: "src/a.rs".into(),
            call_id: None,
        });
        let value: Value = serde_json::from_str(&orphan).unwrap();
        assert!(value.get("call_id").is_none(), "{orphan}");
    }

    /// Absent optional fields stay off the wire. Every frame is paid for on
    /// every turn of a long session, and `"detail": null` is bytes for nothing.
    #[test]
    fn absent_options_are_omitted_rather_than_null() {
        let line = encode(&Event::ToolEnd {
            call_id: "c1".into(),
            summary: "ok".into(),
            elapsed_ms: 1,
            error: None,
        });
        assert!(!line.contains("null"), "{line}");
        assert!(!line.contains("error"), "{line}");
    }
}
