//! Neutral wire-independent types shared by all adapters.

use std::fmt;

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApiStyle {
    Chat,
    Responses,
}

/// Everything needed to reach one endpoint, resolved fresh per request.
#[derive(Clone, Debug)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub style: ApiStyle,
}

/// CLI-flag level overrides; highest precedence.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub api_style: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_prompt_tokens: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finish {
    Stop,
    ToolCalls,
    /// Output was never capped, so Length means the context is full.
    Length,
    ContentFilter,
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Always a string of JSON, re-serialized canonically if the server sent an object.
    pub arguments: String,
}

/// One assembled assistant turn.
#[derive(Clone, Debug)]
pub struct Turn {
    pub text: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
    pub finish: Finish,
    /// Responses API only: the completed output items in their captured
    /// wire form, replayed verbatim in the next request's `input` so
    /// reasoning items survive byte-identical (append-only cache discipline).
    /// Always empty for Chat Completions.
    pub raw_items: Vec<Value>,
}

/// One tool the model may call, in neutral form. Each adapter wraps it in
/// its wire shape (chat nests under `function`, responses flattens).
#[derive(Clone, Debug)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments object.
    pub parameters: Value,
}

/// One transcript item, adapter-independent. The agent loop (P2) owns the
/// transcript; adapters serialize it per wire shape.
#[derive(Clone, Debug)]
pub enum Item {
    User(String),
    /// A prior assistant turn, kept with everything needed to replay it.
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
        /// Captured Responses output items; replayed verbatim when present.
        raw_items: Vec<Value>,
    },
    ToolResult {
        call_id: String,
        content: String,
    },
}

/// Everything one model turn needs, minus the endpoint.
#[derive(Clone, Debug, Default)]
pub struct TurnRequest {
    /// System prompt; `instructions` on the Responses shape.
    pub system: Option<String>,
    pub items: Vec<Item>,
    pub tools: Vec<ToolSpec>,
}

/// Borrowed request view for the agent hot path. The transcript and tool
/// schemas are serialized directly from their session-owned storage instead
/// of cloning the full conversation before every model round.
#[derive(Clone, Copy, Debug)]
pub struct TurnRequestRef<'a> {
    pub system: Option<&'a str>,
    pub items: &'a [Item],
    pub tools: &'a [ToolSpec],
}

impl TurnRequest {
    pub fn borrowed(&self) -> TurnRequestRef<'_> {
        TurnRequestRef {
            system: self.system.as_deref(),
            items: &self.items,
            tools: &self.tools,
        }
    }
}

/// Stream events, delivered in arrival order. P0 uses only the assembled
/// `Turn`; P1 wires these through SSE streaming.
#[derive(Clone, Debug)]
pub enum Event {
    Text(String),
    Reasoning(String),
    ToolCallStart { index: u32, id: String, name: String },
    ToolArgsDelta { index: u32, delta: String },
    Usage(Usage),
    Done(Finish),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeoutKind {
    Connect,
    Send,
    FirstByte,
    Idle,
}

/// Typed provider errors. Every rendered message states its remedy.
#[derive(Debug)]
pub enum ProviderError {
    /// Missing or invalid settings.
    Config(String),
    /// Could not reach the endpoint at all.
    Connect(String),
    /// Non-2xx response.
    Http { status: u16, body: String },
    Timeout(TimeoutKind),
    /// Ctrl-C (or an explicit interrupt) during the request.
    Interrupted,
    /// The server answered with bytes we could not make sense of.
    Wire(String),
    Unsupported(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderError::Config(msg) => write!(f, "config: {msg}"),
            ProviderError::Connect(msg) => write!(f, "connect: {msg}"),
            ProviderError::Http { status, body } => {
                let body = body.trim();
                let shown: String = body.chars().take(400).collect();
                let remedy = match status {
                    401 | 403 => "check NOOB_API_KEY in your config .env",
                    404 | 405 => {
                        "check that NOOB_BASE_URL points at an OpenAI-compatible /v1 base, \
                         e.g. http://localhost:8090/v1"
                    }
                    _ => "the response body usually names the cause; check the server logs \
                          if it persists",
                };
                write!(f, "endpoint returned HTTP {status}: {shown}; {remedy}")
            }
            ProviderError::Timeout(TimeoutKind::Connect) => {
                write!(f, "timed out connecting; check NOOB_BASE_URL and that the server is up")
            }
            ProviderError::Timeout(TimeoutKind::Send) => {
                write!(
                    f,
                    "sending the request stalled; the server accepted the connection but \
                     stopped reading; retry, and restart the server if it persists"
                )
            }
            ProviderError::Timeout(TimeoutKind::FirstByte) => {
                write!(
                    f,
                    "the server accepted the request but sent nothing back in time; \
                     it may be overloaded or stuck"
                )
            }
            ProviderError::Timeout(TimeoutKind::Idle) => {
                write!(f, "the response stream stalled mid-way; retry the request")
            }
            ProviderError::Interrupted => write!(f, "interrupted"),
            ProviderError::Wire(msg) => {
                write!(
                    f,
                    "could not parse the server response: {msg}; the endpoint may not be \
                     OpenAI-compatible, or the wrong wire shape is selected (try \
                     NOOB_API_STYLE=chat)"
                )
            }
            ProviderError::Unsupported(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ProviderError {}
