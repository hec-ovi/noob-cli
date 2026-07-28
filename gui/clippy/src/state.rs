//! What the agent is doing, sorted into the things you actually want to look
//! at separately.
//!
//! | view | carries |
//! |---|---|
//! | `talk` | the model's prose and reasoning, streamed |
//! | `activity` | every call it makes, colored by what kind of thing it is |
//! | `plan` | the checklist, from the plan tool's own arguments |
//! | `agents` | sub-agents, their brief and how they ended |
//! | `files` | one tab per file it has touched, with the diff |
//!
//! Activity is one stream, not two. Splitting `bash` from the rest looked
//! right on paper and read as arbitrary in use: `ls` is the `ls` tool and
//! `rm -rf` is `bash`, so the split put two neighbouring thoughts in two
//! different panes. They are one list now, and what separates them is color:
//! a shell command, a file being written and a web search are three colors,
//! which is the distinction that was actually wanted.
//!
//! Nothing here asks the model to classify anything. The kind comes from the
//! tool name and the syntax comes from the file extension, both of which the
//! harness already has.
//!
//! This module is pure. It takes frames and produces lines, and it is where
//! nearly all of the front end's behaviour can be tested without a GPU.

use std::collections::HashMap;
use std::collections::VecDeque;

use noob_proto::{Event, Usage};

/// How a line reads, resolved to a color by the skin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Dim,
    Body,
    Bright,
    Good,
    Bad,
    Minus,
    Plus,
    /// A call, colored by what kind of call it is.
    Call(Kind),
}

/// Which tool a call is. One variant per tool, not one per category.
///
/// Categories were the first attempt and read as no distinction at all: most
/// of a session is `read`, `ls` and `grep`, so grouping them under one colour
/// left the list looking uncoloured. Every tool gets its own colour and its
/// own name in the tag column, which is what makes a list of forty rows
/// scannable without reading any of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bash,
    Read,
    Ls,
    Glob,
    Grep,
    Context,
    Write,
    Edit,
    Web,
    Skill,
    Mcp,
    Agent,
    Plan,
    Other,
}

impl Kind {
    /// Every tool this build knows, for tests and for the palette.
    pub const ALL: [Kind; 14] = [
        Kind::Bash,
        Kind::Read,
        Kind::Ls,
        Kind::Glob,
        Kind::Grep,
        Kind::Context,
        Kind::Write,
        Kind::Edit,
        Kind::Web,
        Kind::Skill,
        Kind::Mcp,
        Kind::Agent,
        Kind::Plan,
        Kind::Other,
    ];

    /// From the tool's name, which is all the harness has and all it needs.
    pub fn of(name: &str) -> Kind {
        match name {
            "bash" => Kind::Bash,
            "read" => Kind::Read,
            "ls" => Kind::Ls,
            "glob" => Kind::Glob,
            "grep" => Kind::Grep,
            "context" => Kind::Context,
            "write" => Kind::Write,
            "edit" => Kind::Edit,
            "websearch" => Kind::Web,
            "skill" => Kind::Skill,
            "subagent" => Kind::Agent,
            "plan" | "todo" => Kind::Plan,
            name if name.starts_with("mcp") => Kind::Mcp,
            _ => Kind::Other,
        }
    }

    /// The tag printed before every row: the tool's own name, so the colour
    /// has a name for anyone who cannot rely on colour alone.
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Bash => "bash",
            Kind::Read => "read",
            Kind::Ls => "ls",
            Kind::Glob => "glob",
            Kind::Grep => "grep",
            Kind::Context => "ctx",
            Kind::Write => "write",
            Kind::Edit => "edit",
            Kind::Web => "web",
            Kind::Skill => "skill",
            Kind::Mcp => "mcp",
            Kind::Agent => "agent",
            Kind::Plan => "plan",
            Kind::Other => "tool",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub tone: Tone,
    /// The line this is in its file, for the gutter. Only file views set it.
    pub number: Option<u32>,
}

impl Line {
    pub fn new(text: impl Into<String>, tone: Tone) -> Line {
        Line {
            text: text.into(),
            tone,
            number: None,
        }
    }

    pub fn at(mut self, number: u32) -> Line {
        self.number = Some(number);
        self
    }
}

/// A bounded scrollback. Old lines fall off the top rather than growing until
/// the process is the size of the session.
pub struct Pane {
    lines: VecDeque<Line>,
    cap: usize,
    /// Rows scrolled back from the tail. Zero means following the live end,
    /// which is what a pane returns to whenever new content arrives.
    pub scrollback: usize,
}

impl Pane {
    pub fn new(cap: usize) -> Pane {
        Pane {
            lines: VecDeque::new(),
            cap,
            scrollback: 0,
        }
    }

    pub fn push(&mut self, line: Line) {
        self.lines.push_back(line);
        while self.lines.len() > self.cap {
            self.lines.pop_front();
        }
        // New content pulls the view back to the live end. A pane that stayed
        // where it was would silently stop showing what is happening.
        self.scrollback = 0;
    }

    pub fn say(&mut self, text: impl Into<String>, tone: Tone) {
        self.push(Line::new(text, tone));
    }

    /// A line that knows where it is in its file.
    pub fn say_at(&mut self, text: impl Into<String>, tone: Tone, number: u32) {
        self.push(Line::new(text, tone).at(number));
    }

    pub fn blank_if_needed(&mut self) {
        if self.lines.back().is_some_and(|l| !l.text.is_empty()) {
            self.push(Line::new("", Tone::Dim));
        }
    }

    /// Append streamed text, starting new lines on every newline. This is what
    /// makes token-by-token prose land as paragraphs rather than as one line
    /// per token.
    pub fn stream(&mut self, chunk: &str, tone: Tone) {
        for (i, part) in chunk.split('\n').enumerate() {
            if i > 0 {
                self.push(Line::new("", tone));
            }
            match self.lines.back_mut() {
                Some(last) if last.tone == tone => last.text.push_str(part),
                _ => self.push(Line::new(part, tone)),
            }
        }
        self.scrollback = 0;
    }

    /// The `rows` lines this pane is currently showing, honouring scrollback.
    pub fn visible(&self, rows: usize) -> Vec<&Line> {
        if rows == 0 {
            return Vec::new();
        }
        let end = self.lines.len().saturating_sub(self.scrollback);
        let start = end.saturating_sub(rows);
        self.lines.range(start..end).collect()
    }

    /// Scroll back by `rows`, stopping at the oldest line still held. Returns
    /// whether anything moved, so a caller only redraws when it did.
    pub fn scroll_back(&mut self, rows: usize, visible: usize) -> bool {
        let most = self.lines.len().saturating_sub(visible);
        let next = (self.scrollback + rows).min(most);
        let moved = next != self.scrollback;
        self.scrollback = next;
        moved
    }

    pub fn scroll_forward(&mut self, rows: usize) -> bool {
        let next = self.scrollback.saturating_sub(rows);
        let moved = next != self.scrollback;
        self.scrollback = next;
        moved
    }

    /// Whether a fenced code block is open where this window starts.
    ///
    /// A transcript is drawn from a scrolling window, so a block that opened
    /// above it would otherwise render as prose. Scanning the lines above is a
    /// prefix check each, which is nothing next to shaping them.
    pub fn fence_before(&self, rows: usize) -> crate::markdown::Fence {
        let end = self.lines.len().saturating_sub(self.scrollback);
        let start = end.saturating_sub(rows);
        crate::markdown::fence_after(
            self.lines
                .range(..start)
                .filter(|line| line.tone == Tone::Body)
                .map(|line| line.text.as_str()),
        )
    }

    /// Where the thumb sits and how tall it is, as fractions of the track, or
    /// `None` when everything fits and there is nothing to indicate.
    pub fn thumb(&self, rows: usize) -> Option<(f32, f32)> {
        if rows == 0 || self.lines.len() <= rows {
            return None;
        }
        let total = self.lines.len() as f32;
        let size = (rows as f32 / total).clamp(0.06, 1.0);
        let top = (total - rows as f32 - self.scrollback as f32).max(0.0) / total;
        Some((top.min(1.0 - size), size))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TodoState {
    Pending,
    Active,
    Done,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Todo {
    pub text: String,
    pub state: TodoState,
}

#[derive(Clone, Debug)]
pub struct AgentRow {
    pub label: String,
    pub brief: String,
    pub state: &'static str,
    pub tone: Tone,
}

/// One file the agent has touched, and everything it did to it.
pub struct FileView {
    pub path: String,
    pub pane: Pane,
    /// Set once anything was written to it, so the tab can say so.
    pub changed: bool,
}

/// The one word the collapsed bar shows. A window shaded to a single strip has
/// room for exactly this, so it has to be the thing worth knowing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Starting,
    Ready,
    Thinking,
    Working,
    Finished,
    Interrupted,
    Gone,
}

impl Phase {
    pub fn word(self) -> &'static str {
        match self {
            Phase::Starting => "STARTING",
            Phase::Ready => "READY",
            Phase::Thinking => "THINKING",
            Phase::Working => "WORKING",
            Phase::Finished => "FINISHED",
            Phase::Interrupted => "INTERRUPTED",
            Phase::Gone => "AGENT GONE",
        }
    }

    pub fn busy(self) -> bool {
        matches!(self, Phase::Thinking | Phase::Working)
    }
}

/// How fast the endpoint is actually going, measured rather than reported.
///
/// Two phases and two rates. Prefill is from the request leaving to the first
/// token arriving, which is what a long transcript costs. Decode is from the
/// first token to the last, which is what the answer costs. Averaged over the
/// session, because a single request is noise and the average is what tells
/// you whether the machine is doing what it did yesterday.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rates {
    prefill_tokens: u64,
    prefill_seconds: f64,
    decode_tokens: u64,
    decode_seconds: f64,
    /// When the request in flight went out, and when its first token landed.
    started: Option<f64>,
    first_token: Option<f64>,
}

impl Rates {
    /// Tokens per second the endpoint prefilled at, over the session.
    pub fn prefill(&self) -> f64 {
        rate(self.prefill_tokens, self.prefill_seconds)
    }

    /// Tokens per second it generated at, over the session.
    pub fn decode(&self) -> f64 {
        rate(self.decode_tokens, self.decode_seconds)
    }

    fn request_started(&mut self, at: f64) {
        self.started = Some(at);
        self.first_token = None;
    }

    fn token_arrived(&mut self, at: f64) {
        self.first_token.get_or_insert(at);
    }

    /// The request finished and reported what it cost. Windows shorter than a
    /// millisecond are dropped rather than dividing by nearly zero: a cached
    /// prefill really does take no measurable time, and counting it as
    /// infinitely fast makes the average meaningless.
    fn request_finished(&mut self, usage: Usage, at: f64) {
        if let (Some(started), Some(first)) = (self.started, self.first_token) {
            let prefill = first - started;
            if prefill > 0.001 && usage.prefilled() > 0 {
                self.prefill_tokens += usage.prefilled();
                self.prefill_seconds += prefill;
            }
            let decode = at - first;
            if decode > 0.001 && usage.completion > 0 {
                self.decode_tokens += usage.completion;
                self.decode_seconds += decode;
            }
        }
        // The next request starts where this one ended, which is what the
        // agent actually does: it turns straight around and sends again.
        self.request_started(at);
    }
}

fn rate(tokens: u64, seconds: f64) -> f64 {
    if seconds <= 0.0 {
        return 0.0;
    }
    tokens as f64 / seconds
}

/// A call in flight, kept so its end can be reported the way its start was.
struct Open {
    kind: Kind,
    brief: String,
}

pub struct State {
    pub session: String,
    pub model: String,
    pub workspace: String,
    pub resumed: bool,

    pub talk: Pane,
    pub activity: Pane,
    pub plan: Vec<Todo>,
    pub agents: Vec<AgentRow>,
    pub files: Vec<FileView>,
    pub open_file: usize,

    pub usage: Option<Usage>,
    pub prefilled: u64,
    pub generated: u64,
    pub requests: u32,
    /// What the last request alone cost, as against the session totals.
    pub last_prefill: u64,
    pub last_generated: u64,
    pub rates: Rates,

    pub turn: u32,
    pub phase: Phase,
    /// What is happening right now, in a few words, for the status bar.
    pub status: String,

    open: HashMap<String, Open>,
}

impl Default for State {
    fn default() -> State {
        State::new()
    }
}

impl State {
    pub fn new() -> State {
        State {
            session: String::new(),
            model: String::new(),
            workspace: String::new(),
            resumed: false,
            talk: Pane::new(6000),
            activity: Pane::new(4000),
            plan: Vec::new(),
            agents: Vec::new(),
            files: Vec::new(),
            open_file: 0,
            usage: None,
            prefilled: 0,
            generated: 0,
            requests: 0,
            last_prefill: 0,
            last_generated: 0,
            rates: Rates::default(),
            turn: 0,
            phase: Phase::Starting,
            status: String::from("starting the agent"),
            open: HashMap::new(),
        }
    }

    /// What the human typed, echoed into the transcript so the conversation
    /// reads as a conversation.
    pub fn submitted(&mut self, text: &str) {
        self.talk.blank_if_needed();
        self.talk.say(format!("› {text}"), Tone::Bright);
        self.talk.push(Line::new("", Tone::Body));
        self.phase = Phase::Thinking;
        self.status = String::from("thinking");
    }

    /// The file's view, creating it the first time it is mentioned. Opening it
    /// selects it, which is what makes the tab strip follow the agent without
    /// anyone clicking.
    fn file_mut(&mut self, path: &str) -> &mut FileView {
        if let Some(index) = self.files.iter().position(|f| f.path == path) {
            self.open_file = index;
            return &mut self.files[index];
        }
        // Bounded: a session that touches a thousand files must not keep a
        // thousand scrollbacks alive.
        if self.files.len() >= MAX_FILES {
            self.files.remove(0);
        }
        self.files.push(FileView {
            path: path.to_string(),
            pane: Pane::new(3000),
            changed: false,
        });
        self.open_file = self.files.len() - 1;
        self.files.last_mut().expect("just pushed")
    }

    /// Fold one frame in, untimed. The window always has a clock and uses
    /// [`State::apply_at`]; this is the shape the tests read better in.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn apply(&mut self, event: Event) -> bool {
        self.apply_at(event, None)
    }

    /// Fold one frame in, timed. `at` is monotonic seconds from any fixed
    /// point; it is passed in rather than read here so this module stays pure
    /// and the rates are testable without a clock.
    pub fn apply_at(&mut self, event: Event, at: Option<f64>) -> bool {
        match (&event, at) {
            (Event::TurnStart { .. }, Some(at)) => self.rates.request_started(at),
            (Event::TextDelta { .. } | Event::ReasoningDelta { .. }, Some(at)) => {
                self.rates.token_arrived(at);
            }
            (Event::UsageReport { usage }, Some(at)) => {
                self.rates.request_finished(*usage, at);
            }
            _ => {}
        }
        match event {
            Event::SessionStart {
                id,
                workspace,
                model,
                resumed,
            } => {
                self.session = id;
                self.workspace = workspace;
                self.model = model;
                self.resumed = resumed;
                self.phase = Phase::Ready;
                self.status = String::from("ready");
            }
            Event::SessionEnd { .. } => {
                self.phase = Phase::Gone;
                self.status = String::from("the agent stopped");
            }
            Event::TurnStart { turn } => {
                self.turn = turn;
                self.phase = Phase::Thinking;
                self.status = String::from("thinking");
            }
            Event::TurnEnd { interrupted, .. } => {
                self.phase = if interrupted == Some(true) {
                    Phase::Interrupted
                } else {
                    Phase::Finished
                };
                self.status = self.phase.word().to_lowercase();
                // Close anything left open, so a row cannot show as running
                // after the turn that owned it has ended.
                let stragglers: Vec<String> =
                    self.open.drain().map(|(_, open)| open.brief).collect();
                for brief in stragglers {
                    self.activity
                        .say(format!("       {brief} never reported back"), Tone::Bad);
                }
            }
            Event::TextDelta { d } => self.talk.stream(&d, Tone::Body),
            Event::ReasoningDelta { d } => self.talk.stream(&d, Tone::Dim),

            Event::ToolStart {
                call_id,
                name,
                brief,
                args,
            } => {
                let kind = Kind::of(&name);
                self.phase = Phase::Working;
                self.status = format!("{name} {brief}");
                match kind {
                    // The plan is a view, not a log line: the call carries the
                    // whole updated checklist, so it replaces the last one.
                    Kind::Plan => self.plan = read_todos(&args),
                    Kind::Agent => {
                        let prompt = args
                            .get("prompt")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&brief)
                            .to_string();
                        self.agents.push(AgentRow {
                            label: format!("agent {}", self.agents.len() + 1),
                            brief: prompt,
                            state: "running",
                            tone: Tone::Bright,
                        });
                    }
                    _ => {}
                }
                self.activity.say(
                    format!("{:>5}  {}", kind.tag(), subject(kind, &brief, &args)),
                    Tone::Call(kind),
                );
                self.open.insert(call_id, Open { kind, brief });
            }
            Event::ToolProgress { call_id, line } => {
                let kind = self.open.get(&call_id).map_or(Kind::Other, |o| o.kind);
                self.activity
                    .say(format!("       {line}"), Tone::Call(kind));
            }
            Event::ToolEnd {
                call_id,
                summary,
                error,
                ..
            } => {
                let open = self.open.remove(&call_id);
                if open.as_ref().map(|o| o.kind) == Some(Kind::Agent)
                    && let Some(agent) = self.agents.last_mut()
                {
                    let failed = error.is_some();
                    agent.state = if failed { "failed" } else { "done" };
                    agent.tone = if failed { Tone::Bad } else { Tone::Good };
                }
                match error {
                    None => self.activity.say(format!("       {summary}"), Tone::Good),
                    Some(error) => {
                        self.activity.say(format!("       {summary}"), Tone::Bad);
                        self.activity
                            .say(format!("       {}", error.message), Tone::Bad);
                        if let Some(detail) = error.detail.as_deref() {
                            let mut rest = detail
                                .lines()
                                .skip(1)
                                .filter(|line| !line.trim().is_empty());
                            for line in rest.by_ref().take(DETAIL_LINES) {
                                self.activity.say(format!("       {line}"), Tone::Dim);
                            }
                            if rest.next().is_some() {
                                self.activity.say("       …", Tone::Dim);
                            }
                        }
                    }
                }
            }

            Event::FileOpen { path, lines, .. } => {
                let file = self.file_mut(&path);
                file.pane.blank_if_needed();
                file.pane
                    .say(format!("read {lines} lines"), Tone::Call(Kind::Read));
            }
            Event::FileSpan { path, span, .. } => {
                let file = self.file_mut(&path);
                file.pane
                    .say(format!("      lines {}-{}", span.start, span.end), Tone::Dim);
            }
            Event::FileEdit {
                path,
                span,
                before,
                after,
                ..
            } => {
                let file = self.file_mut(&path);
                file.changed = true;
                file.pane.blank_if_needed();
                file.pane.say(
                    format!("write lines {}-{}", span.start, span.end),
                    Tone::Call(Kind::Write),
                );
                let mut clipped = before.lines().count() > DIFF_LINES;
                for (n, line) in before.lines().take(DIFF_LINES).enumerate() {
                    file.pane
                        .say_at(format!("- {line}"), Tone::Minus, span.start + n as u32);
                }
                clipped |= after.lines().count() > DIFF_LINES;
                for (n, line) in after.lines().take(DIFF_LINES).enumerate() {
                    file.pane
                        .say_at(format!("+ {line}"), Tone::Plus, span.start + n as u32);
                }
                if clipped {
                    file.pane.say("  …", Tone::Dim);
                }
                // A blank line after the block, so two edits in a row read as
                // two edits rather than as one long stretch of file.
                file.pane.say("", Tone::Dim);
            }
            Event::FileClose { path, .. } => {
                if let Some(file) = self.files.iter_mut().find(|f| f.path == path) {
                    file.pane.say("left the context", Tone::Dim);
                }
            }

            Event::AgentSpawn {
                agent_id, prompt, ..
            } => {
                self.agents.push(AgentRow {
                    label: agent_id,
                    brief: prompt,
                    state: "queued",
                    tone: Tone::Dim,
                });
            }
            Event::AgentStateChanged {
                agent_id, state, ..
            } => {
                let (word, tone) = match state {
                    noob_proto::AgentState::Done => ("done", Tone::Good),
                    noob_proto::AgentState::Failed => ("failed", Tone::Bad),
                    noob_proto::AgentState::Canceled => ("canceled", Tone::Dim),
                    noob_proto::AgentState::Running => ("running", Tone::Bright),
                    _ => ("queued", Tone::Dim),
                };
                if let Some(agent) = self.agents.iter_mut().find(|a| a.label == agent_id) {
                    agent.state = word;
                    agent.tone = tone;
                }
            }
            Event::AgentOutput { agent_id, line } => {
                self.activity
                    .say(format!("agent  {agent_id} {line}"), Tone::Call(Kind::Agent));
            }

            Event::UsageReport { usage } => {
                self.prefilled += usage.prefilled();
                self.generated += usage.completion;
                self.last_prefill = usage.prefilled();
                self.last_generated = usage.completion;
                self.requests += 1;
                self.usage = Some(usage);
            }

            Event::Note { line } => self.talk.say(line, Tone::Dim),
            Event::Error { line } => {
                self.talk.blank_if_needed();
                self.talk.say(line, Tone::Bad);
            }

            // Nothing this window shows yet. Skipped rather than guessed at,
            // which is what keeps a newer agent from breaking an older window.
            Event::SkillList { .. }
            | Event::McpList { .. }
            | Event::McpState { .. }
            | Event::Metrics { .. }
            | Event::Unknown => return false,
        }
        true
    }

    /// How much of the context window this session is holding, 0.0 to 1.0.
    pub fn context_fraction(&self) -> f32 {
        match self.usage {
            Some(usage) if usage.context_total > 0 => {
                (usage.prompt as f32 / usage.context_total as f32).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    /// The session budget. Prefill and cache separated, because summing raw
    /// prompt tokens counts work nobody did.
    pub fn budget_line(&self) -> String {
        match self.usage {
            None => String::from("context —"),
            Some(usage) => format!(
                "context {} / {} ({:.0}%)   prefilled {}   cached {}   generated {}   requests {}",
                thousands(usage.prompt),
                thousands(usage.context_total),
                self.context_fraction() * 100.0,
                thousands(self.prefilled),
                thousands(usage.cached_prompt),
                thousands(self.generated),
                self.requests,
            ),
        }
    }

    /// The line the shaded window shows, which is the only thing visible when
    /// it is collapsed to a strip.
    pub fn headline(&self) -> String {
        let done = self
            .plan
            .iter()
            .filter(|t| t.state == TodoState::Done)
            .count();
        let plan = if self.plan.is_empty() {
            String::new()
        } else {
            format!("   plan {done}/{}", self.plan.len())
        };
        let files = match self.files.iter().filter(|f| f.changed).count() {
            0 => String::new(),
            1 => String::from("   1 file changed"),
            n => format!("   {n} files changed"),
        };
        match self.phase {
            Phase::Working => format!("{}   {}{plan}{files}", self.phase.word(), self.status),
            _ => format!("{}{plan}{files}", self.phase.word()),
        }
    }
}

/// What a call is about, in one line.
///
/// The brief the agent builds is empty for some tools, which left a row that
/// was a colored tag and nothing else. So: the shell command itself for bash,
/// the brief when there is one, and otherwise the first useful-looking string
/// in the arguments. A row always says something.
fn subject(kind: Kind, brief: &str, args: &noob_proto::Value) -> String {
    let field = |name: &str| {
        args.get(name)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    };
    let found = match kind {
        Kind::Bash => field("cmd"),
        Kind::Web => field("query").or_else(|| field("url")).or_else(|| field("action")),
        _ => None,
    };
    let text = found
        .or_else(|| Some(brief.to_string()).filter(|b| !b.trim().is_empty()))
        .or_else(|| {
            args.as_object()?
                .values()
                .find_map(|v| v.as_str().filter(|s| !s.trim().is_empty()))
                .map(str::to_string)
        })
        .unwrap_or_else(|| String::from("(no arguments)"));
    shorten(&text)
}

/// An absolute path inside a deep tree wraps over three rows and buries the
/// rest of the list. Keep the end, which is the part that identifies it.
fn shorten(text: &str) -> String {
    let text = text.replace('\n', " ");
    if text.chars().count() <= SUBJECT_CHARS {
        return text;
    }
    let tail: String = text
        .chars()
        .skip(text.chars().count() - SUBJECT_CHARS + 1)
        .collect();
    format!("\u{2026}{tail}")
}

const SUBJECT_CHARS: usize = 96;
const DIFF_LINES: usize = 60;
const DETAIL_LINES: usize = 6;
const MAX_FILES: usize = 40;

/// The checklist, straight out of the plan tool's own arguments. The call
/// carries the whole updated list by contract, so nothing has to be merged and
/// no protocol addition was needed to show it.
fn read_todos(args: &noob_proto::Value) -> Vec<Todo> {
    args.get("todos")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let text = item.get("content")?.as_str()?.to_string();
                    let state = match item.get("status").and_then(|v| v.as_str()) {
                        Some("completed") => TodoState::Done,
                        Some("in_progress") => TodoState::Active,
                        _ => TodoState::Pending,
                    };
                    Some(Todo { text, state })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use noob_proto::{Span, ToolError};

    fn tool_start(call_id: &str, name: &str, args: serde_json::Value) -> Event {
        Event::ToolStart {
            call_id: call_id.into(),
            name: name.into(),
            brief: String::from("brief"),
            args,
        }
    }

    fn texts(pane: &Pane) -> Vec<String> {
        pane.visible(usize::MAX)
            .iter()
            .map(|l| l.text.clone())
            .collect()
    }

    /// A row that is a colored tag and nothing else says nothing. Every call
    /// finds something to be about.
    #[test]
    fn a_row_always_says_what_it_is_about() {
        let mut state = State::new();
        // A tool whose brief the agent left empty.
        state.apply(Event::ToolStart {
            call_id: "w".into(),
            name: "websearch".into(),
            brief: String::new(),
            args: serde_json::json!({"query": "laguna s 2.1"}),
        });
        // No brief and no known field: anything in the arguments will do.
        state.apply(Event::ToolStart {
            call_id: "x".into(),
            name: "mcp_call".into(),
            brief: String::new(),
            args: serde_json::json!({"server": "fs", "tool": "list"}),
        });
        // Nothing at all still names itself rather than rendering blank.
        state.apply(Event::ToolStart {
            call_id: "y".into(),
            name: "context".into(),
            brief: String::new(),
            args: serde_json::json!({}),
        });
        let lines = texts(&state.activity);
        assert!(lines[0].contains("laguna s 2.1"), "{lines:?}");
        assert!(lines[1].contains("fs") || lines[1].contains("list"), "{lines:?}");
        assert!(lines[2].contains("no arguments"), "{lines:?}");
        for line in &lines {
            assert!(line.trim().len() > 6, "a bare tag: {line:?}");
        }
    }

    /// A deep absolute path wraps over three rows and buries the list.
    #[test]
    fn a_long_subject_is_clipped_to_its_tail() {
        let deep = format!("/tmp/{}/calc.py", "very-long-directory/".repeat(12));
        let mut state = State::new();
        state.apply(Event::ToolStart {
            call_id: "r".into(),
            name: "read".into(),
            brief: deep.clone(),
            args: serde_json::json!({"path": deep}),
        });
        let line = &texts(&state.activity)[0];
        assert!(line.chars().count() <= SUBJECT_CHARS + 8, "{line}");
        assert!(line.ends_with("calc.py"), "the end is what identifies it: {line}");
        assert!(line.contains('\u{2026}'), "{line}");
    }

    /// The split that was wrong. `ls` is the `ls` tool and `rm` is `bash`, so
    /// separating them by pane put two neighbouring thoughts in two places.
    /// They are one list; color is what separates them.
    #[test]
    fn every_call_lands_in_one_list_and_carries_its_kind() {
        let mut state = State::new();
        state.apply(tool_start(
            "a",
            "bash",
            serde_json::json!({"cmd": "rm -rf build"}),
        ));
        state.apply(tool_start("b", "ls", serde_json::json!({"path": "."})));
        state.apply(tool_start("c", "write", serde_json::json!({"path": "a.rs"})));
        state.apply(tool_start("d", "websearch", serde_json::json!({"query": "x"})));
        state.apply(tool_start("e", "skill", serde_json::json!({"name": "web"})));

        let lines = state.activity.visible(usize::MAX);
        let kinds: Vec<Tone> = lines.iter().map(|l| l.tone).collect();
        assert_eq!(
            kinds,
            [
                Tone::Call(Kind::Bash),
                Tone::Call(Kind::Ls),
                Tone::Call(Kind::Write),
                Tone::Call(Kind::Web),
                Tone::Call(Kind::Skill),
            ]
        );
        // Every kind is distinguishable without color, too.
        assert!(lines[0].text.contains("bash  rm -rf build"), "{:?}", lines[0]);
        assert!(lines[3].text.contains("web"), "{:?}", lines[3]);
    }

    #[test]
    fn tool_names_map_to_the_kind_you_would_guess() {
        assert_eq!(Kind::of("bash"), Kind::Bash);
        assert_eq!(Kind::of("grep"), Kind::Grep);
        assert_eq!(Kind::of("edit"), Kind::Edit);
        assert_eq!(Kind::of("ls"), Kind::Ls);
        assert_eq!(Kind::of("read"), Kind::Read);
        assert_eq!(Kind::of("mcp_call"), Kind::Mcp);
        assert_eq!(Kind::of("mcp_connect"), Kind::Mcp);
        assert_eq!(Kind::of("subagent"), Kind::Agent);
        assert_eq!(Kind::of("plan"), Kind::Plan);
        assert_eq!(Kind::of("something_new"), Kind::Other);
    }

    /// The plan tool sends the whole updated list every call, so the view
    /// replaces rather than merges, and it needs no protocol addition.
    #[test]
    fn the_plan_comes_from_the_calls_arguments() {
        let mut state = State::new();
        state.apply(tool_start(
            "p",
            "plan",
            serde_json::json!({"todos": [
                {"content": "read the file", "status": "completed"},
                {"content": "fix the bug", "status": "in_progress"},
                {"content": "run the tests", "status": "pending"},
            ]}),
        ));
        assert_eq!(
            state.plan,
            [
                Todo {
                    text: "read the file".into(),
                    state: TodoState::Done
                },
                Todo {
                    text: "fix the bug".into(),
                    state: TodoState::Active
                },
                Todo {
                    text: "run the tests".into(),
                    state: TodoState::Pending
                },
            ]
        );
        // A second call replaces the first outright.
        state.apply(tool_start(
            "p2",
            "plan",
            serde_json::json!({"todos": [{"content": "done", "status": "completed"}]}),
        ));
        assert_eq!(state.plan.len(), 1);
    }

    #[test]
    fn a_malformed_plan_does_not_panic_or_half_apply() {
        let mut state = State::new();
        state.apply(tool_start("p", "plan", serde_json::json!({"todos": "nope"})));
        assert!(state.plan.is_empty());
        state.apply(tool_start(
            "p",
            "plan",
            serde_json::json!({"todos": [{"content": 7}, {"content": "ok", "status": "weird"}]}),
        ));
        assert_eq!(
            state.plan,
            [Todo {
                text: "ok".into(),
                state: TodoState::Pending
            }]
        );
    }

    /// Sub-agents get their own list rather than a row in the activity log,
    /// which is where they used to disappear.
    #[test]
    fn a_sub_agent_gets_a_row_of_its_own_that_resolves() {
        let mut state = State::new();
        state.apply(tool_start(
            "s",
            "subagent",
            serde_json::json!({"prompt": "search the web for elcensuradoweb.com"}),
        ));
        assert_eq!(state.agents.len(), 1);
        assert_eq!(state.agents[0].state, "running");
        assert!(state.agents[0].brief.contains("elcensuradoweb"));
        state.apply(Event::ToolEnd {
            call_id: "s".into(),
            summary: "subagent failed".into(),
            elapsed_ms: 5,
            error: Some(ToolError {
                kind: "error".into(),
                code: None,
                message: "needs the websearch CLI on PATH".into(),
                detail: None,
                remedy: None,
            }),
        });
        assert_eq!(state.agents[0].state, "failed");
        assert_eq!(state.agents[0].tone, Tone::Bad);
    }

    /// One tab per file, selected by whatever the agent just touched, so the
    /// strip follows it without anyone clicking.
    #[test]
    fn files_get_a_tab_each_and_the_newest_is_selected() {
        let mut state = State::new();
        state.apply(Event::FileOpen {
            path: "a.py".into(),
            lines: 10,
            call_id: None,
        });
        state.apply(Event::FileEdit {
            path: "b.md".into(),
            span: Span {
                start: 1,
                end: 1,
                kind: None,
                name: None,
            },
            before: String::new(),
            after: "# Title".into(),
            call_id: None,
        });
        assert_eq!(state.files.len(), 2);
        assert_eq!(state.open_file, 1);
        assert_eq!(state.files[1].path, "b.md");
        assert!(state.files[1].changed, "b.md was written");
        assert!(!state.files[0].changed, "a.py was only read");
        // Touching the first again selects it rather than adding a duplicate.
        state.apply(Event::FileOpen {
            path: "a.py".into(),
            lines: 12,
            call_id: None,
        });
        assert_eq!(state.files.len(), 2);
        assert_eq!(state.open_file, 0);
    }

    /// A session that touches hundreds of files must not keep a scrollback for
    /// every one of them alive forever.
    #[test]
    fn the_file_list_is_bounded() {
        let mut state = State::new();
        for n in 0..MAX_FILES + 10 {
            state.apply(Event::FileOpen {
                path: format!("f{n}.rs"),
                lines: 1,
                call_id: None,
            });
        }
        assert_eq!(state.files.len(), MAX_FILES);
        assert_eq!(
            state.files.last().unwrap().path,
            format!("f{}.rs", MAX_FILES + 9)
        );
        assert!(
            state.open_file < state.files.len(),
            "the selection stays valid"
        );
    }

    /// A failure says what broke. This is the same failure that used to render
    /// as a bare `exit code 1`.
    #[test]
    fn a_failure_shows_its_message_and_a_bounded_tail() {
        let mut state = State::new();
        state.apply(tool_start(
            "x",
            "bash",
            serde_json::json!({"cmd": "cargo test"}),
        ));
        let detail = std::iter::once(String::from("exit code 1"))
            .chain((0..50).map(|n| format!("trace line {n}")))
            .collect::<Vec<_>>()
            .join("\n");
        state.apply(Event::ToolEnd {
            call_id: "x".into(),
            summary: "bash cargo test (exit 1)".into(),
            elapsed_ms: 10,
            error: Some(ToolError {
                kind: "error".into(),
                code: None,
                message: "error: could not compile".into(),
                detail: Some(detail),
                remedy: None,
            }),
        });
        let lines = texts(&state.activity);
        assert!(
            lines.iter().any(|l| l.contains("could not compile")),
            "{lines:?}"
        );
        assert!(lines.iter().any(|l| l.contains("trace line 0")), "{lines:?}");
        assert!(lines.last().unwrap().contains('…'), "{lines:?}");
        assert!(lines.len() < 20, "the tail is bounded: {}", lines.len());
    }

    #[test]
    fn streamed_text_accumulates_into_lines() {
        let mut state = State::new();
        for chunk in ["Hel", "lo ", "there", "\nsecond ", "line"] {
            state.apply(Event::TextDelta { d: chunk.into() });
        }
        assert_eq!(texts(&state.talk), ["Hello there", "second line"]);
    }

    #[test]
    fn reasoning_does_not_merge_into_the_answer() {
        let mut state = State::new();
        state.apply(Event::ReasoningDelta {
            d: "let me think".into(),
        });
        state.apply(Event::TextDelta {
            d: "the answer".into(),
        });
        let lines: Vec<_> = state.talk.visible(usize::MAX);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].tone, Tone::Dim);
        assert_eq!(lines[1].tone, Tone::Body);
    }

    /// The collapsed window is one strip, so its line has to be the thing
    /// worth knowing without opening it again.
    #[test]
    fn the_headline_says_what_is_happening_in_one_line() {
        let mut state = State::new();
        assert_eq!(state.headline(), "STARTING");
        state.apply(Event::TurnStart { turn: 1 });
        assert_eq!(state.headline(), "THINKING");
        state.apply(tool_start(
            "p",
            "plan",
            serde_json::json!({"todos": [
                {"content": "one", "status": "completed"},
                {"content": "two", "status": "in_progress"},
            ]}),
        ));
        assert!(
            state.headline().starts_with("WORKING"),
            "{}",
            state.headline()
        );
        assert!(state.headline().contains("plan 1/2"), "{}", state.headline());
        state.apply(Event::FileEdit {
            path: "a.rs".into(),
            span: Span {
                start: 1,
                end: 1,
                kind: None,
                name: None,
            },
            before: String::new(),
            after: "x".into(),
            call_id: None,
        });
        state.apply(Event::TurnEnd {
            turn: 1,
            interrupted: None,
        });
        assert_eq!(state.headline(), "FINISHED   plan 1/2   1 file changed");
        assert!(!state.phase.busy());
    }

    #[test]
    fn an_interrupted_turn_says_so_and_closes_open_rows() {
        let mut state = State::new();
        state.apply(Event::TurnStart { turn: 1 });
        state.apply(tool_start(
            "g",
            "bash",
            serde_json::json!({"cmd": "sleep 9"}),
        ));
        state.apply(Event::TurnEnd {
            turn: 1,
            interrupted: Some(true),
        });
        assert_eq!(state.phase, Phase::Interrupted);
        assert!(
            texts(&state.activity)
                .iter()
                .any(|l| l.contains("never reported back"))
        );
    }

    #[test]
    fn the_budget_sums_prefill_and_not_the_whole_prompt() {
        let mut state = State::new();
        for (prompt, cached) in [(1000, 0), (1200, 1000), (1500, 1200)] {
            state.apply(Event::UsageReport {
                usage: Usage {
                    prompt,
                    cached_prompt: cached,
                    completion: 10,
                    context_total: 65536,
                },
            });
        }
        assert_eq!(state.prefilled, 1500);
        let line = state.budget_line();
        assert!(line.contains("1,500 / 65,536"), "{line}");
    }

    #[test]
    fn a_pane_keeps_only_its_last_lines() {
        let mut pane = Pane::new(8);
        for n in 0..40 {
            pane.say(format!("{n}"), Tone::Body);
        }
        assert_eq!(pane.visible(usize::MAX).len(), 8);
        assert_eq!(pane.visible(3).last().unwrap().text, "39");
    }

    #[test]
    fn scrolling_back_then_receiving_returns_to_the_live_end() {
        let mut pane = Pane::new(100);
        for n in 0..50 {
            pane.say(format!("{n}"), Tone::Body);
        }
        assert!(pane.scroll_back(10, 10));
        assert_eq!(pane.visible(10).last().unwrap().text, "39");
        pane.say("new", Tone::Body);
        assert_eq!(pane.scrollback, 0);
        assert_eq!(pane.visible(10).last().unwrap().text, "new");
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        let mut pane = Pane::new(100);
        for n in 0..20 {
            pane.say(format!("{n}"), Tone::Body);
        }
        assert!(pane.scroll_back(999, 10));
        assert_eq!(pane.scrollback, 10);
        assert!(!pane.scroll_back(1, 10), "already at the oldest line");
        assert!(pane.scroll_forward(999));
        assert_eq!(pane.scrollback, 0);
        assert!(!pane.scroll_forward(1), "already at the live end");
    }

    /// The scrollbar has to say where you are, and has to disappear when
    /// everything already fits.
    #[test]
    fn the_thumb_reports_position_and_size_or_nothing_at_all() {
        let mut pane = Pane::new(200);
        for n in 0..10 {
            pane.say(format!("{n}"), Tone::Body);
        }
        assert_eq!(pane.thumb(10), None, "everything fits");
        assert_eq!(pane.thumb(20), None);
        for n in 10..100 {
            pane.say(format!("{n}"), Tone::Body);
        }
        let (top, size) = pane.thumb(10).expect("ninety lines do not fit in ten");
        assert!((size - 0.1).abs() < 0.01, "{size}");
        assert!((top - 0.9).abs() < 0.01, "at the live end: {top}");
        pane.scroll_back(90, 10);
        let (top, _) = pane.thumb(10).unwrap();
        assert_eq!(top, 0.0, "scrolled to the very top");
        // The thumb never runs off the end of its track.
        for rows in [1, 3, 7, 10, 99] {
            if let Some((top, size)) = pane.thumb(rows) {
                assert!(top + size <= 1.001, "{top} + {size}");
            }
        }
    }

    /// A diff line knows where it is in the file, so the gutter can say so.
    #[test]
    fn diff_lines_carry_their_line_numbers() {
        let mut state = State::new();
        state.apply(Event::FileEdit {
            path: "a.rs".into(),
            span: Span {
                start: 17,
                end: 19,
                kind: None,
                name: None,
            },
            before: "one\ntwo".into(),
            after: "ONE\nTWO\nTHREE".into(),
            call_id: None,
        });
        let lines = state.files[0].pane.visible(usize::MAX);
        let numbered: Vec<(Option<u32>, &str)> = lines
            .iter()
            .map(|l| (l.number, l.text.as_str()))
            .collect();
        assert!(numbered.contains(&(None, "write lines 17-19")), "{numbered:?}");
        assert!(numbered.contains(&(Some(17), "- one")), "{numbered:?}");
        assert!(numbered.contains(&(Some(18), "- two")), "{numbered:?}");
        assert!(numbered.contains(&(Some(17), "+ ONE")), "{numbered:?}");
        assert!(numbered.contains(&(Some(19), "+ THREE")), "{numbered:?}");
        // And the block is closed off, so a second edit reads as a second one.
        assert_eq!(lines.last().unwrap().text, "");
    }

    /// A window scrolled into the middle of a fenced block has to know it is
    /// looking at code, or the block renders as prose.
    #[test]
    fn a_pane_reports_the_fence_open_above_its_window() {
        let mut pane = Pane::new(100);
        pane.say("prose", Tone::Body);
        pane.say("```python", Tone::Body);
        for n in 0..30 {
            pane.say(format!("x = {n}"), Tone::Body);
        }
        assert!(pane.fence_before(10).open(), "the block is still open");
        // The window is the lines at the end; `before` is everything above it,
        // so the closing fence has to be above the window to count as closed.
        pane.say("```", Tone::Body);
        pane.say("back to prose", Tone::Body);
        assert!(!pane.fence_before(1).open(), "and now it is closed");
        assert!(pane.fence_before(2).open(), "the window still holds the fence");
        // What the human typed is not the model's Markdown, so it cannot open
        // a block that the model then has to close.
        let mut typed = Pane::new(100);
        typed.say("run ```this```", Tone::Bright);
        typed.say("prose", Tone::Body);
        assert!(!typed.fence_before(1).open());
    }

    #[test]
    fn an_unknown_frame_changes_nothing() {
        let mut state = State::new();
        assert!(!state.apply(Event::Unknown));
        assert!(state.apply(Event::TurnStart { turn: 1 }));
    }

    /// Two phases, two rates. A long transcript costs prefill time and the
    /// answer costs decode time, and they are different numbers about
    /// different problems.
    #[test]
    fn the_rates_measure_prefill_and_decode_separately() {
        let mut state = State::new();
        let usage = |prompt, cached, completion| Usage {
            prompt,
            cached_prompt: cached,
            completion,
            context_total: 65536,
        };
        // A request that spent 2s prefilling 1000 tokens and 4s writing 200.
        state.apply_at(Event::TurnStart { turn: 1 }, Some(0.0));
        state.apply_at(Event::TextDelta { d: "a".into() }, Some(2.0));
        state.apply_at(Event::TextDelta { d: "b".into() }, Some(3.0));
        state.apply_at(
            Event::UsageReport {
                usage: usage(1000, 0, 200),
            },
            Some(6.0),
        );
        assert!((state.rates.prefill() - 500.0).abs() < 0.01, "{}", state.rates.prefill());
        assert!((state.rates.decode() - 50.0).abs() < 0.01, "{}", state.rates.decode());
        assert_eq!(state.last_prefill, 1000);
        assert_eq!(state.last_generated, 200);

        // A second request, averaged in rather than replacing.
        state.apply_at(Event::TextDelta { d: "c".into() }, Some(7.0));
        state.apply_at(
            Event::UsageReport {
                usage: usage(1500, 1000, 100),
            },
            Some(9.0),
        );
        assert_eq!(state.prefilled, 1500, "totals keep summing");
        assert_eq!(state.last_prefill, 500, "and the last one is its own number");
        let prefill = state.rates.prefill();
        assert!(prefill > 400.0 && prefill < 600.0, "{prefill}");
    }

    /// A fully cached prefill really does take no measurable time, and
    /// counting it as infinitely fast makes the average meaningless.
    #[test]
    fn an_immeasurable_window_does_not_poison_the_average() {
        let mut state = State::new();
        state.apply_at(Event::TurnStart { turn: 1 }, Some(0.0));
        state.apply_at(Event::TextDelta { d: "x".into() }, Some(0.0));
        state.apply_at(
            Event::UsageReport {
                usage: Usage {
                    prompt: 1000,
                    cached_prompt: 1000,
                    completion: 0,
                    context_total: 65536,
                },
            },
            Some(0.0),
        );
        assert_eq!(state.rates.prefill(), 0.0);
        assert_eq!(state.rates.decode(), 0.0);
        assert!(state.rates.prefill().is_finite());
    }

    /// Without a clock the counters still work; only the rates stay at zero.
    #[test]
    fn untimed_frames_still_count_tokens() {
        let mut state = State::new();
        state.apply(Event::UsageReport {
            usage: Usage {
                prompt: 100,
                cached_prompt: 0,
                completion: 10,
                context_total: 1000,
            },
        });
        assert_eq!(state.prefilled, 100);
        assert_eq!(state.rates.decode(), 0.0);
    }

    #[test]
    fn thousands_groups_from_the_right() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
