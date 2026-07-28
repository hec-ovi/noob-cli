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
//! to missing features instead of a parse error that kills the stream.
//!
//! **Correlation is explicit.** Every tool frame carries the same `call_id`
//! from start to end. The stream this replaces put an id on the result and not
//! on the call, so pairing them meant counting in order, which is wrong the
//! moment anything runs concurrently. Everything here that can be concurrent
//! carries the id that identifies it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Protocol version. Additive changes bump this; readers accept `<=` their own.
pub const VERSION: u16 = 1;

/// One frame on the wire: a version and a body.
///
/// `v` is outside the body so a reader can check it before it knows whether it
/// understands the type, which is what makes an old reader safe against a new
/// writer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame<T> {
    pub v: u16,
    #[serde(flatten)]
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

/// Why a tool call failed, as fields rather than prose.
///
/// The prose version of this is what made a failed command render as
/// `exit code 1`: the interesting part was buried in a body nobody parsed, and
/// the caller had to guess which line mattered. `remedy` exists because the
/// model's next action is the thing a failure most needs to determine, and
/// inferring it from a message is work every consumer would otherwise repeat.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolError {
    /// Coarse class, for routing and display: `not_found`, `denied`,
    /// `timeout`, `canceled`, `exit_status`, `invalid_argument`, `internal`.
    pub kind: String,
    /// Machine detail where one exists, such as a process exit status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
    /// One line, already the most useful line rather than the first one.
    pub message: String,
    /// The rest, unbounded and unsummarized. Consumers decide how much to show.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// What to do next, when that is knowable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remedy: Option<String>,
}

/// What one request cost.
///
/// `cached_prompt` is separate from `prompt` because summing raw prompt tokens
/// across a session counts work nobody did: every request resends the whole
/// transcript and the endpoint serves most of it from cache. Prefill is
/// `prompt - cached_prompt`, and that is the number that means anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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
}

/// A span of a file, 1-based and inclusive, as line numbers.
///
/// Lines rather than byte offsets, and deliberately not tied to how the span
/// was found: a scanner, a real parser, or a human cursor all produce the same
/// thing, so the consumer stays indifferent to that choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    /// What the span is, when known: `fn`, `class`, `section`, `selection`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// One measured quantity at one instant.
///
/// Shaped to carry what both reference tools need without deciding how either
/// is drawn. radeontop is a list of labelled bars, each a value against a
/// maximum. btop is the same values sampled repeatedly and drawn as a rolling
/// graph. Both fall out of the same sample: a `max` means the value is a
/// proportion and can be a bar; a stream of samples over time is a series and
/// can be a sparkline. The renderer chooses; the protocol does not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    /// Stable identifier, so a consumer can follow one quantity across frames
    /// even if its label changes.
    pub key: String,
    /// What to show a human.
    pub label: String,
    pub value: f64,
    /// Full scale, when the quantity has one. Absent means unbounded, which a
    /// bar cannot render and a graph can.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// How a sub-agent is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Queued,
    Running,
    Done,
    Failed,
    Canceled,
}

/// Agent to front end.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Event {
    #[serde(rename = "session.start")]
    SessionStart {
        id: String,
        workspace: String,
        model: String,
        resumed: bool,
    },
    #[serde(rename = "session.end")]
    SessionEnd { id: String },

    #[serde(rename = "turn.start")]
    TurnStart { turn: u32 },
    #[serde(rename = "turn.end")]
    TurnEnd {
        turn: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        interrupted: Option<bool>,
    },

    #[serde(rename = "text.delta")]
    TextDelta { d: String },
    /// Ephemeral. Never enters a transcript, and a consumer may drop it.
    #[serde(rename = "reasoning.delta")]
    ReasoningDelta { d: String },

    #[serde(rename = "tool.start")]
    ToolStart {
        call_id: String,
        name: String,
        /// The short human label, already built by the agent so every consumer
        /// renders the same words.
        brief: String,
        args: Value,
    },
    #[serde(rename = "tool.progress")]
    ToolProgress { call_id: String, line: String },
    #[serde(rename = "tool.end")]
    ToolEnd {
        call_id: String,
        summary: String,
        elapsed_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<ToolError>,
    },

    /// The agent is now looking at this file. Drives the code pane.
    #[serde(rename = "file.open")]
    FileOpen { path: String, lines: u32 },
    /// The part of that file it is about to work on.
    #[serde(rename = "file.span")]
    FileSpan { path: String, span: Span },
    /// A range that just changed, with both sides, so a diff needs no guessing
    /// and no second read of the file.
    #[serde(rename = "file.edit")]
    FileEdit {
        path: String,
        span: Span,
        before: String,
        after: String,
    },
    #[serde(rename = "file.close")]
    FileClose { path: String },

    #[serde(rename = "agent.spawn")]
    AgentSpawn {
        agent_id: String,
        prompt: String,
        tools: String,
    },
    #[serde(rename = "agent.state")]
    AgentStateChanged {
        agent_id: String,
        state: AgentState,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    #[serde(rename = "agent.output")]
    AgentOutput { agent_id: String, line: String },

    #[serde(rename = "skill.list")]
    SkillList { names: Vec<String> },
    #[serde(rename = "mcp.list")]
    McpList { names: Vec<String> },
    #[serde(rename = "mcp.state")]
    McpState {
        name: String,
        connected: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        tools: Option<u32>,
    },

    #[serde(rename = "usage")]
    UsageReport { usage: Usage },

    /// A batch of measurements taken together.
    ///
    /// Deliberately generic: GPU counters, token economy and anything measured
    /// later all travel this frame, so adding a new readout is a new `group`
    /// rather than a new frame type and a protocol bump. `at_ms` is what lets a
    /// consumer assemble successive frames into a time series without needing
    /// its own clock.
    #[serde(rename = "metrics")]
    Metrics {
        group: String,
        at_ms: u64,
        samples: Vec<Sample>,
    },

    #[serde(rename = "note")]
    Note { line: String },
    #[serde(rename = "error")]
    Error { line: String },

    /// A frame this reader does not know. Present so a newer writer degrades to
    /// missing features rather than a dead stream.
    #[serde(other)]
    Unknown,
}

/// Front end to agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum Command {
    #[serde(rename = "prompt.submit")]
    PromptSubmit { text: String },
    /// Queue behind the running turn instead of interrupting it.
    #[serde(rename = "prompt.queue")]
    PromptQueue { text: String },
    #[serde(rename = "turn.cancel")]
    TurnCancel,

    #[serde(rename = "agent.cancel")]
    AgentCancel { agent_id: String },

    #[serde(rename = "skill.add")]
    SkillAdd { source: String },
    #[serde(rename = "skill.remove")]
    SkillRemove { name: String },
    #[serde(rename = "mcp.add")]
    McpAdd { name: String, spec: String },
    #[serde(rename = "mcp.remove")]
    McpRemove { name: String },
    #[serde(rename = "mcp.connect")]
    McpConnect { name: String },

    #[serde(rename = "config.set")]
    ConfigSet { key: String, value: String },
    #[serde(rename = "config.unset")]
    ConfigUnset { key: String },

    #[serde(rename = "session.list")]
    SessionList,
    #[serde(rename = "session.open")]
    SessionOpen { id: String },

    #[serde(other)]
    Unknown,
}

/// Decode one line, or `None` if it is not a frame this reader can use.
///
/// Fails closed and silently by design: a malformed or too-new line is skipped
/// so one bad frame cannot end a session. Callers that want to know decode
/// with serde directly.
pub fn decode<T: for<'de> Deserialize<'de>>(line: &str) -> Option<Frame<T>> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let frame: Frame<T> = serde_json::from_str(line).ok()?;
    frame.readable().then_some(frame)
}

/// Encode one frame as a line, newline included.
pub fn encode<T: Serialize>(body: &T) -> String {
    let frame = Frame { v: VERSION, body };
    let mut line = serde_json::to_string(&frame).unwrap_or_else(|_| String::from("{}"));
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(event: Event) {
        let line = encode(&event);
        assert!(line.ends_with('\n'), "frames are newline delimited");
        let back: Frame<Event> = decode(&line).expect("a frame we just wrote must decode");
        assert_eq!(back.v, VERSION);
        assert_eq!(back.body, event);
    }

    #[test]
    fn every_event_survives_a_round_trip() {
        round_trip(Event::SessionStart {
            id: "s1".into(),
            workspace: "/work".into(),
            model: "laguna".into(),
            resumed: false,
        });
        round_trip(Event::TextDelta { d: "hello".into() });
        round_trip(Event::ToolStart {
            call_id: "c1".into(),
            name: "bash".into(),
            brief: "cargo test".into(),
            args: serde_json::json!({"cmd": "cargo test"}),
        });
        round_trip(Event::FileEdit {
            path: "src/a.rs".into(),
            span: Span {
                start: 10,
                end: 14,
                kind: Some("fn".into()),
                name: Some("add".into()),
            },
            before: "a - b".into(),
            after: "a + b".into(),
        });
        round_trip(Event::AgentStateChanged {
            agent_id: "agent-1".into(),
            state: AgentState::Running,
            detail: None,
        });
        round_trip(Event::UsageReport {
            usage: Usage {
                prompt: 1832,
                cached_prompt: 1334,
                completion: 2,
                context_total: 65536,
            },
        });
    }

    #[test]
    fn every_command_survives_a_round_trip() {
        for command in [
            Command::PromptSubmit {
                text: "do the thing".into(),
            },
            Command::TurnCancel,
            Command::AgentCancel {
                agent_id: "agent-2".into(),
            },
            Command::McpConnect { name: "fs".into() },
            Command::SessionList,
        ] {
            let line = encode(&command);
            let back: Frame<Command> = decode(&line).expect("must decode");
            assert_eq!(back.body, command);
        }
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
