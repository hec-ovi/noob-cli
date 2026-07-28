//! What the agent is doing, as four separate streams.
//!
//! One scrollback jumbles a shell command, a file being rewritten and the
//! model's prose into the same column, and reading it means sorting them out
//! again by eye every time. So they are sorted once, here, by where each frame
//! came from:
//!
//! | pane | carries |
//! |---|---|
//! | `talk` | the model's prose and reasoning, streamed |
//! | `shell` | `bash`, its command and its result |
//! | `tools` | every other call: search, skills, MCP, sub-agents |
//! | `code`  | file activity, with the diff for anything written |
//!
//! Routing is by tool name and, for files, by extension, which is the only
//! information the harness has and the only information it needs. Nothing here
//! asks the model to classify anything: the agent is never taught that this
//! front end exists.
//!
//! This module is pure. It takes frames and produces lines, and it is where
//! nearly all of the front end's behaviour can be tested without a GPU.

use std::collections::HashMap;
use std::collections::VecDeque;

use noob_proto::{Event, Usage};

/// How a line reads, resolved to a color by the skin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Structure: headers, separators, timings.
    Dim,
    /// Ordinary content.
    Body,
    /// The thing that just happened.
    Bright,
    /// It worked.
    Good,
    /// It did not.
    Bad,
    /// Removed, in a diff.
    Minus,
    /// Added, in a diff.
    Plus,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub tone: Tone,
}

impl Line {
    pub fn new(text: impl Into<String>, tone: Tone) -> Line {
        Line {
            text: text.into(),
            tone,
        }
    }
}

/// A bounded scrollback. Old lines fall off the top rather than growing until
/// the process is the size of the session.
pub struct Pane {
    pub title: &'static str,
    lines: VecDeque<Line>,
    cap: usize,
    /// Rows scrolled back from the tail. Zero means following the live end,
    /// which is what a pane returns to whenever new content arrives.
    pub scrollback: usize,
}

impl Pane {
    pub fn new(title: &'static str, cap: usize) -> Pane {
        Pane {
            title,
            lines: VecDeque::new(),
            cap,
            scrollback: 0,
        }
    }

    pub fn push(&mut self, line: Line) {
        self.lines.push_back(line);
        while self.lines.len() > self.cap {
            self.lines.pop_front();
            self.scrollback = self.scrollback.saturating_sub(1);
        }
        // New content pulls the view back to the live end. A pane that stayed
        // where it was would silently stop showing what is happening.
        self.scrollback = 0;
    }

    pub fn say(&mut self, text: impl Into<String>, tone: Tone) {
        self.push(Line::new(text, tone));
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

    pub fn len(&self) -> usize {
        self.lines.len()
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
}

/// A call in flight, kept so its end can be reported where its start was.
struct Open {
    name: String,
    /// Which pane opened it, so the result lands in the same one.
    pane: Stream,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stream {
    Talk,
    Shell,
    Tools,
    Code,
}

pub struct State {
    pub session: String,
    pub model: String,
    pub workspace: String,
    pub resumed: bool,

    pub talk: Pane,
    pub shell: Pane,
    pub tools: Pane,
    pub code: Pane,

    pub usage: Option<Usage>,
    /// Prefill summed across the session: what the endpoint actually computed,
    /// which is the number that means anything. Raw prompt tokens summed across
    /// requests counts the transcript once per request.
    pub prefilled: u64,
    pub generated: u64,
    pub requests: u32,

    pub turn: u32,
    pub busy: bool,
    /// The file the code pane is currently about, for its header.
    pub focus: Option<String>,
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
            talk: Pane::new("talk", 4000),
            shell: Pane::new("shell", 2000),
            tools: Pane::new("tools", 2000),
            code: Pane::new("code", 4000),
            usage: None,
            prefilled: 0,
            generated: 0,
            requests: 0,
            turn: 0,
            busy: false,
            focus: None,
            status: String::from("starting the agent"),
            open: HashMap::new(),
        }
    }

    pub fn pane(&self, stream: Stream) -> &Pane {
        match stream {
            Stream::Talk => &self.talk,
            Stream::Shell => &self.shell,
            Stream::Tools => &self.tools,
            Stream::Code => &self.code,
        }
    }

    pub fn pane_mut(&mut self, stream: Stream) -> &mut Pane {
        match stream {
            Stream::Talk => &mut self.talk,
            Stream::Shell => &mut self.shell,
            Stream::Tools => &mut self.tools,
            Stream::Code => &mut self.code,
        }
    }

    /// What the human typed, echoed into the transcript so the conversation
    /// reads as a conversation.
    pub fn submitted(&mut self, text: &str) {
        self.talk.blank_if_needed();
        self.talk.say(format!("› {text}"), Tone::Bright);
        self.talk.push(Line::new("", Tone::Body));
        self.busy = true;
        self.status = String::from("thinking");
    }

    /// Fold one frame in. Returns whether anything visible changed.
    pub fn apply(&mut self, event: Event) -> bool {
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
                self.status = String::from("ready");
            }
            Event::SessionEnd { .. } => {
                self.busy = false;
                self.status = String::from("the agent stopped");
            }
            Event::TurnStart { turn } => {
                self.turn = turn;
                self.busy = true;
                self.status = String::from("thinking");
            }
            Event::TurnEnd { interrupted, .. } => {
                self.busy = false;
                self.status = String::from(if interrupted == Some(true) {
                    "interrupted"
                } else {
                    "ready"
                });
                // Close anything the agent left open, so a row cannot show as
                // running after the turn that owned it has ended.
                for (_, open) in self.open.drain() {
                    let pane = match open.pane {
                        Stream::Shell => &mut self.shell,
                        Stream::Code => &mut self.code,
                        _ => &mut self.tools,
                    };
                    pane.say(format!("  {} did not report back", open.name), Tone::Bad);
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
                let pane = route(&name);
                let subject = match pane {
                    // A shell line is the command itself; the brief clips it.
                    Stream::Shell => args
                        .get("cmd")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&brief)
                        .to_string(),
                    _ => brief.clone(),
                };
                self.pane_mut(pane).say(
                    match pane {
                        Stream::Shell => format!("$ {subject}"),
                        _ => format!("▸ {name}  {subject}"),
                    },
                    Tone::Bright,
                );
                self.open.insert(call_id, Open { name, pane });
            }
            Event::ToolProgress { call_id, line } => {
                let pane = self.open.get(&call_id).map_or(Stream::Tools, |o| o.pane);
                self.pane_mut(pane).say(format!("  {line}"), Tone::Dim);
            }
            Event::ToolEnd {
                call_id,
                summary,
                error,
                ..
            } => {
                let open = self.open.remove(&call_id);
                let pane = open.as_ref().map_or(Stream::Tools, |o| o.pane);
                match error {
                    None => self
                        .pane_mut(pane)
                        .say(format!("  {summary}"), Tone::Good),
                    Some(error) => {
                        let pane = self.pane_mut(pane);
                        pane.say(format!("  {summary}"), Tone::Bad);
                        pane.say(format!("  {}", error.message), Tone::Bad);
                        // The rest, bounded: a failure that printed a stack
                        // trace is why this exists, and all of it is on the
                        // wire, so the pane shows the head and says so.
                        if let Some(detail) = error.detail.as_deref() {
                            let mut rest = detail
                                .lines()
                                .skip(1)
                                .filter(|line| !line.trim().is_empty());
                            for line in rest.by_ref().take(DETAIL_LINES) {
                                pane.say(format!("  {line}"), Tone::Dim);
                            }
                            if rest.next().is_some() {
                                pane.say("  …", Tone::Dim);
                            }
                        }
                    }
                }
            }

            Event::FileOpen { path, lines, .. } => {
                self.focus = Some(path.clone());
                self.code.blank_if_needed();
                self.code
                    .say(format!("▸ {path}  {lines} lines"), Tone::Bright);
            }
            Event::FileSpan { path, span, .. } => {
                self.focus = Some(path);
                self.code.say(
                    format!("  lines {}-{}", span.start, span.end),
                    Tone::Dim,
                );
            }
            Event::FileEdit {
                path,
                span,
                before,
                after,
                ..
            } => {
                self.focus = Some(path.clone());
                self.code.blank_if_needed();
                self.code.say(
                    format!("▸ {path}  {}-{}", span.start, span.end),
                    Tone::Bright,
                );
                let syntax = crate::syntax::for_path(&path);
                for line in before.lines().take(DIFF_LINES) {
                    self.code.say(format!("- {line}"), Tone::Minus);
                }
                if before.lines().count() > DIFF_LINES {
                    self.code.say("- …", Tone::Minus);
                }
                for line in after.lines().take(DIFF_LINES) {
                    self.code.say(format!("+ {line}"), Tone::Plus);
                }
                if after.lines().count() > DIFF_LINES {
                    self.code.say("+ …", Tone::Plus);
                }
                let _ = syntax; // colored at render time, from the same path
            }
            Event::FileClose { path, .. } => {
                if self.focus.as_deref() == Some(path.as_str()) {
                    self.focus = None;
                }
                self.code
                    .say(format!("  {path} left the context"), Tone::Dim);
            }

            Event::AgentSpawn { agent_id, .. } => {
                self.tools.say(format!("▸ agent {agent_id}"), Tone::Bright);
            }
            Event::AgentStateChanged {
                agent_id, state, ..
            } => {
                self.tools.say(
                    format!("  agent {agent_id} {}", state.as_str()),
                    match state {
                        noob_proto::AgentState::Failed => Tone::Bad,
                        noob_proto::AgentState::Done => Tone::Good,
                        _ => Tone::Dim,
                    },
                );
            }
            Event::AgentOutput { agent_id, line } => {
                self.tools.say(format!("  {agent_id} {line}"), Tone::Dim);
            }

            Event::UsageReport { usage } => {
                self.prefilled += usage.prefilled();
                self.generated += usage.completion;
                self.requests += 1;
                self.usage = Some(usage);
            }

            Event::Note { line } => self.talk.say(line, Tone::Dim),
            Event::Error { line } => {
                self.talk.blank_if_needed();
                self.talk.say(line, Tone::Bad);
            }

            // Nothing this front end shows yet. Skipped rather than guessed at,
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

    /// The session budget, as a line. Prefill and cache separated, because
    /// summing raw prompt tokens counts work nobody did.
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
}

const DIFF_LINES: usize = 40;
const DETAIL_LINES: usize = 6;

/// Which stream a tool belongs to.
///
/// By name, because the name is what the harness has. `read` joins the file
/// tools: what it produces is file frames, and a reader watching the code pane
/// wants to see the file the agent just opened next to the one it just changed.
fn route(name: &str) -> Stream {
    match name {
        "bash" => Stream::Shell,
        "read" | "write" | "edit" => Stream::Code,
        _ => Stream::Tools,
    }
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

    /// The whole point of the layout: a shell command, a file write and the
    /// model's prose land in three different places without anyone sorting
    /// them by eye.
    #[test]
    fn each_kind_of_work_lands_in_its_own_pane() {
        let mut state = State::new();
        state.apply(tool_start("a", "bash", serde_json::json!({"cmd": "cargo test"})));
        state.apply(tool_start("b", "write", serde_json::json!({"path": "a.rs"})));
        state.apply(tool_start("c", "websearch", serde_json::json!({"query": "x"})));
        state.apply(Event::TextDelta {
            d: "here is what I found".into(),
        });

        assert_eq!(texts(&state.shell), ["$ cargo test"]);
        assert_eq!(texts(&state.tools), ["▸ websearch  brief"]);
        assert_eq!(texts(&state.code), ["▸ write  brief"]);
        assert_eq!(texts(&state.talk), ["here is what I found"]);
    }

    /// A result must land in the pane its call opened, whatever else happened
    /// in between. Calls run concurrently, so this cannot be "the last pane".
    #[test]
    fn a_result_lands_where_its_call_started_even_when_calls_interleave() {
        let mut state = State::new();
        state.apply(tool_start("sh", "bash", serde_json::json!({"cmd": "ls"})));
        state.apply(tool_start("rd", "read", serde_json::json!({"path": "a.rs"})));
        // The read finishes first, which is the ordinary case for a wave.
        state.apply(Event::ToolEnd {
            call_id: "rd".into(),
            summary: "read a.rs".into(),
            elapsed_ms: 1,
            error: None,
        });
        state.apply(Event::ToolEnd {
            call_id: "sh".into(),
            summary: "bash ls".into(),
            elapsed_ms: 2,
            error: None,
        });
        assert_eq!(texts(&state.shell), ["$ ls", "  bash ls"]);
        assert_eq!(texts(&state.code), ["▸ read  brief", "  read a.rs"]);
    }

    /// A failure says what broke, in the pane it broke in. This is the same
    /// failure that used to render as a bare `exit code 1`.
    #[test]
    fn a_failure_shows_its_message_and_a_bounded_tail() {
        let mut state = State::new();
        state.apply(tool_start("x", "bash", serde_json::json!({"cmd": "cargo test"})));
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
        let lines = texts(&state.shell);
        assert!(lines.iter().any(|l| l.contains("could not compile")), "{lines:?}");
        assert!(lines.iter().any(|l| l.contains("trace line 0")), "{lines:?}");
        assert!(lines.last().unwrap().contains('…'), "{lines:?}");
        assert!(lines.len() < 20, "the tail is bounded: {}", lines.len());
    }

    /// Streamed prose is a paragraph, not one line per token.
    #[test]
    fn streamed_text_accumulates_into_lines() {
        let mut state = State::new();
        for chunk in ["Hel", "lo ", "there", "\nsecond ", "line"] {
            state.apply(Event::TextDelta { d: chunk.into() });
        }
        assert_eq!(texts(&state.talk), ["Hello there", "second line"]);
    }

    /// Prose and reasoning are different tones, so they do not merge into one
    /// run when they interleave.
    #[test]
    fn reasoning_does_not_merge_into_the_answer() {
        let mut state = State::new();
        state.apply(Event::ReasoningDelta { d: "let me think".into() });
        state.apply(Event::TextDelta { d: "the answer".into() });
        let lines: Vec<_> = state.talk.visible(usize::MAX);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].tone, Tone::Dim);
        assert_eq!(lines[1].tone, Tone::Body);
    }

    /// A write shows both sides, so the diff needs no second read of the file.
    #[test]
    fn an_edit_shows_the_diff_it_carried() {
        let mut state = State::new();
        state.apply(Event::FileEdit {
            path: "calc.py".into(),
            span: Span {
                start: 2,
                end: 2,
                kind: None,
                name: None,
            },
            before: "    return a - b".into(),
            after: "    return a + b".into(),
            call_id: Some("c1".into()),
        });
        let lines = state.code.visible(usize::MAX);
        assert_eq!(lines[0].text, "▸ calc.py  2-2");
        assert_eq!(*lines[1], Line::new("-     return a - b", Tone::Minus));
        assert_eq!(*lines[2], Line::new("+     return a + b", Tone::Plus));
        assert_eq!(state.focus.as_deref(), Some("calc.py"));
    }

    /// A rewritten large file must not push everything else out of the pane.
    #[test]
    fn a_huge_edit_is_clipped_rather_than_flooding_the_pane() {
        let mut state = State::new();
        let after: String = (0..500).map(|n| format!("line {n}\n")).collect();
        state.apply(Event::FileEdit {
            path: "big.rs".into(),
            span: Span {
                start: 1,
                end: 500,
                kind: None,
                name: None,
            },
            before: String::new(),
            after,
            call_id: None,
        });
        assert!(state.code.len() <= DIFF_LINES + 4, "{}", state.code.len());
        assert!(texts(&state.code).last().unwrap().contains('…'));
    }

    /// A row that never closed must not read as still running once the turn
    /// that owned it has ended.
    #[test]
    fn a_turn_ending_closes_rows_the_agent_left_open() {
        let mut state = State::new();
        state.apply(Event::TurnStart { turn: 1 });
        state.apply(tool_start("ghost", "bash", serde_json::json!({"cmd": "sleep 9"})));
        state.apply(Event::TurnEnd {
            turn: 1,
            interrupted: Some(true),
        });
        assert!(!state.busy);
        assert_eq!(state.status, "interrupted");
        assert!(
            texts(&state.shell).iter().any(|l| l.contains("did not report back")),
            "{:?}",
            texts(&state.shell)
        );
    }

    /// Prefill is what the endpoint computed. Summing raw prompt tokens counts
    /// the transcript once per request, which is work nobody did.
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
        assert_eq!(state.prefilled, 1000 + 200 + 300);
        assert_eq!(state.generated, 30);
        assert_eq!(state.requests, 3);
        let line = state.budget_line();
        assert!(line.contains("1,500 / 65,536"), "{line}");
        assert!(line.contains("prefilled 1,500"), "{line}");
    }

    #[test]
    fn a_pane_keeps_only_its_last_lines() {
        let mut pane = Pane::new("t", 8);
        for n in 0..40 {
            pane.say(format!("{n}"), Tone::Body);
        }
        assert_eq!(pane.len(), 8);
        assert_eq!(pane.visible(3).last().unwrap().text, "39");
    }

    /// New content pulls a scrolled-back pane to the live end, or it silently
    /// stops showing what is happening.
    #[test]
    fn scrolling_back_then_receiving_returns_to_the_live_end() {
        let mut pane = Pane::new("t", 100);
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
        let mut pane = Pane::new("t", 100);
        for n in 0..20 {
            pane.say(format!("{n}"), Tone::Body);
        }
        // Ten visible of twenty means ten rows of history and no more.
        assert!(pane.scroll_back(999, 10));
        assert_eq!(pane.scrollback, 10);
        assert!(!pane.scroll_back(1, 10), "already at the oldest line");
        assert!(pane.scroll_forward(999));
        assert_eq!(pane.scrollback, 0);
        assert!(!pane.scroll_forward(1), "already at the live end");
    }

    /// A frame this window does not render must not make it redraw.
    #[test]
    fn an_unknown_frame_changes_nothing() {
        let mut state = State::new();
        assert!(!state.apply(Event::Unknown));
        assert!(!state.apply(Event::Metrics {
            group: "gpu".into(),
            at_ms: 0,
            samples: vec![],
        }));
        assert!(state.apply(Event::TurnStart { turn: 1 }));
    }

    #[test]
    fn thousands_groups_from_the_right() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(65_536), "65,536");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }
}
