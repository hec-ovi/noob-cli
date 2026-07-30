//! What the agent is doing, sorted into the things you actually want to look
//! at separately.
//!
//! | view | carries |
//! |---|---|
//! | `output` | the model's prose and reasoning, streamed |
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
    fn of(name: &str) -> Kind {
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
    fn tag(self) -> &'static str {
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
    /// How many lines have fallen off the front over the pane's whole life.
    ///
    /// The absolute number of `lines[i]` is `dropped + i`, and that number is
    /// what a selection holds onto. Anchoring to a screen row instead would
    /// slide the selection onto different text the moment anything arrived,
    /// and anchoring to a position in the deque would slide it every time a
    /// line was evicted.
    dropped: usize,
    /// Wrapped heights for the width they were last asked for.
    heights: std::cell::RefCell<Heights>,
}

/// The wrapped height of every line, and what it was computed for.
#[derive(Default)]
struct Heights {
    rows: Vec<usize>,
    cols: usize,
    stale: bool,
}

impl Pane {
    pub fn new(cap: usize) -> Pane {
        Pane {
            lines: VecDeque::new(),
            cap,
            scrollback: 0,
            dropped: 0,
            heights: std::cell::RefCell::new(Heights {
                // Nothing has been measured yet, and a zero width would look
                // like a valid cache for a window mid-resize.
                stale: true,
                ..Heights::default()
            }),
        }
    }

    pub fn push(&mut self, line: Line) {
        self.lines.push_back(line);
        while self.lines.len() > self.cap {
            self.lines.pop_front();
            self.dropped += 1;
        }
        self.heights.borrow_mut().stale = true;
        // New content pulls the view back to the live end. A pane that stayed
        // where it was would silently stop showing what is happening.
        self.scrollback = 0;
    }

    pub fn say(&mut self, text: impl Into<String>, tone: Tone) {
        self.push(Line::new(text, tone));
    }

    /// A line that knows where it is in its file.
    fn say_at(&mut self, text: impl Into<String>, tone: Tone, number: u32) {
        self.push(Line::new(text, tone).at(number));
    }

    fn blank_if_needed(&mut self) {
        if self.lines.back().is_some_and(|l| !l.text.is_empty()) {
            self.push(Line::new("", Tone::Dim));
        }
    }

    /// Append streamed text, starting new lines on every newline. This is what
    /// makes token-by-token prose land as paragraphs rather than as one line
    /// per token.
    fn stream(&mut self, chunk: &str, tone: Tone) {
        for (i, part) in chunk.split('\n').enumerate() {
            if i > 0 {
                self.push(Line::new("", tone));
            }
            match self.lines.back_mut() {
                Some(last) if last.tone == tone => last.text.push_str(part),
                _ => self.push(Line::new(part, tone)),
            }
        }
        // The tail line grew in place, which changes its wrapped height
        // without changing the line count.
        self.heights.borrow_mut().stale = true;
        self.scrollback = 0;
    }

    /// The wrapped height of every line held, for a box `cols` wide.
    ///
    /// Cached because a full pane holds thousands of lines and this is asked
    /// for several times per frame. The cache is rebuilt when the width
    /// changes or when a line is added, which is the only way a height can
    /// move: lines are immutable once past the tail, and `stream` marks the
    /// tail dirty itself.
    fn heights(&self, cols: usize) -> std::cell::Ref<'_, Vec<usize>> {
        {
            let mut cache = self.heights.borrow_mut();
            if cache.cols != cols || cache.stale {
                cache.rows.clear();
                cache
                    .rows
                    .extend(self.lines.iter().map(|l| text_geometry::rows_of(l.text.chars().count(), cols)));
                cache.cols = cols;
                cache.stale = false;
            }
        }
        std::cell::Ref::map(self.heights.borrow(), |c| &c.rows)
    }

    /// Which lines to draw and how far the first one is scrolled off the top.
    pub fn window(&self, rows: usize, cols: usize) -> text_geometry::Window {
        text_geometry::window(&self.heights(cols), rows, self.scrollback)
    }

    /// The lines this pane is currently showing, honouring scrollback, counted
    /// in **visual** rows so a wrapped line cannot fall out of the box.
    pub fn visible(&self, rows: usize, cols: usize) -> Vec<&Line> {
        let w = self.window(rows, cols);
        self.lines.range(w.first..w.first + w.count).collect()
    }

    /// The absolute number of the first line currently on screen, so a screen
    /// row can be turned into the line it is actually showing.
    pub fn showing_from(&self, rows: usize, cols: usize) -> usize {
        self.dropped + self.window(rows, cols).first
    }

    /// Which line a visual row is showing, and the character offset that row
    /// starts at within it. `None` for a row below the last line.
    pub fn spot_in(&self, rows: usize, cols: usize, row: usize) -> Option<(usize, usize)> {
        let w = self.window(rows, cols);
        let (line, offset) = text_geometry::line_at(&self.heights(cols), w, cols, row)?;
        Some((self.dropped + line, offset))
    }

    /// The rows one line occupies on screen, clipped to the viewport.
    pub fn band_of(&self, rows: usize, cols: usize, absolute: usize) -> Option<(usize, usize)> {
        let line = absolute.checked_sub(self.dropped)?;
        let w = self.window(rows, cols);
        text_geometry::band(&self.heights(cols), w, rows, line)
    }

    /// One line by its absolute number, or nothing when it has been evicted.
    pub fn line(&self, absolute: usize) -> Option<&Line> {
        self.lines.get(absolute.checked_sub(self.dropped)?)
    }

    /// One past the last line this pane has ever held.
    pub fn last(&self) -> usize {
        self.dropped + self.lines.len()
    }

    /// Scroll back by `rows`, stopping at the oldest row still held. Returns
    /// whether anything moved, so a caller only redraws when it did.
    ///
    /// Counted in visual rows, so a pane of wrapped lines can scroll further
    /// than it has lines. Under the old line-count clamp the tail of a wrapped
    /// transcript was unreachable.
    pub fn scroll_back(&mut self, rows: usize, visible: usize, cols: usize) -> bool {
        let most = text_geometry::max_scrollback(&self.heights(cols), visible);
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
    pub fn fence_before(&self, rows: usize, cols: usize) -> crate::markdown::Fence {
        let start = self.window(rows, cols).first;
        crate::markdown::fence_after(
            self.lines
                .range(..start)
                .filter(|line| line.tone == Tone::Body)
                .map(|line| line.text.as_str()),
        )
    }

    /// Where the thumb sits and how tall it is, as fractions of the track, or
    /// `None` when everything fits and there is nothing to indicate.
    ///
    /// Measured in visual rows. Counting lines reported a pane of wrapped text
    /// as shorter than it is, so the thumb filled the track while content was
    /// still overflowing.
    pub fn thumb(&self, rows: usize, cols: usize) -> Option<(f32, f32)> {
        text_geometry::thumb(&self.heights(cols), rows, self.scrollback)
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
    /// The tools this child was given, which is what says whether it can
    /// write. Empty when the frame did not carry one.
    pub tools: String,
    /// The last thing it said, live while it runs and its reason once it ends.
    /// A fleet of eight children is unreadable as eight names and no news.
    pub last: String,
}

impl AgentRow {
    fn new(label: String, brief: String, tools: String) -> AgentRow {
        AgentRow {
            label,
            brief,
            state: "queued",
            tone: Tone::Dim,
            tools,
            last: String::new(),
        }
    }
}

/// How full the agent says its context is, right now.
///
/// Measured by the agent at every transcript boundary, so it moves while a
/// turn is still running. `usage` only reports once per request and describes
/// the request that already went out, which is a different and staler number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextFill {
    pub used: u64,
    pub total: u64,
    /// Where compaction triggers, which is the line that actually matters:
    /// the window is not the budget, this is.
    pub compact_at: u64,
}

/// One file the agent has touched, and everything it did to it.
pub struct FileView {
    pub path: String,
    pub pane: Pane,
    /// Set once anything was written to it, so the tab can say so.
    pub changed: bool,
    /// Compaction dropped what the model had read of this file. The page is
    /// still worth showing; it is just no longer what the agent is holding.
    pub closed: bool,
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
///
/// Every request's own rate is kept as well as the sums. An average has already
/// forgotten which requests it was made of, so a median cannot be recovered
/// from one, and a median is the reading that survives a single cold start.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rates {
    prefill_tokens: u64,
    prefill_seconds: f64,
    decode_tokens: u64,
    decode_seconds: f64,
    /// One tokens-per-second reading per measured request, oldest first, capped
    /// at [`crate::totals::SAMPLES`] because the totals file carries these on.
    prefill_rates: Vec<f32>,
    decode_rates: Vec<f32>,
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

    /// The sums behind the averages, for the totals file to carry on with.
    pub fn prefill_sum(&self) -> (u64, f64) {
        (self.prefill_tokens, self.prefill_seconds)
    }

    pub fn decode_sum(&self) -> (u64, f64) {
        (self.decode_tokens, self.decode_seconds)
    }

    /// Every request's own rate, oldest first.
    pub fn prefill_rates(&self) -> &[f32] {
        &self.prefill_rates
    }

    pub fn decode_rates(&self) -> &[f32] {
        &self.decode_rates
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
                sample(&mut self.prefill_rates, usage.prefilled(), prefill);
            }
            let decode = at - first;
            if decode > 0.001 && usage.completion > 0 {
                self.decode_tokens += usage.completion;
                self.decode_seconds += decode;
                sample(&mut self.decode_rates, usage.completion, decode);
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

/// Record one request's rate, dropping the oldest once the ring is full. The
/// same bound the totals file uses, so what is measured is what is kept.
fn sample(rates: &mut Vec<f32>, tokens: u64, seconds: f64) {
    rates.push((tokens as f64 / seconds) as f32);
    if rates.len() > crate::totals::SAMPLES {
        rates.remove(0);
    }
}

/// A call in flight, kept so its end can be reported the way its start was.
///
/// The arguments come along because a call that fails is worth keeping and the
/// end frame does not carry them. Moved off the frame rather than cloned, so a
/// call that works costs nothing for this.
struct Open {
    kind: Kind,
    brief: String,
    args: noob_proto::Value,
}

/// A tool call that came back with an error, and what was sent to it.
///
/// Both halves are already on the wire and both were being rendered to a line
/// of the activity log and then dropped. Keeping them is what makes a debug
/// pane possible without touching the protocol.
#[derive(Clone, Debug, PartialEq)]
pub struct Failure {
    pub kind: Kind,
    /// The agent's own label for the call. Empty for a tool that sent none.
    pub brief: String,
    /// The class and code the system gave, as [`fault`] writes it.
    pub fault: String,
    pub message: String,
    /// The argument object, one line per field, rendered when it was recorded.
    pub args: Vec<String>,
}

/// One row of the debug pane: its text, how it reads, and which failure it
/// belongs to.
///
/// Built in this module rather than in the drawing because a click on the pane
/// is resolved by row number, and only the list that was drawn can say which
/// failure a row number means. Two lists would be two answers.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugRow {
    pub text: String,
    pub tone: Tone,
    /// `None` for the count at the top, which is not a failure and cannot open.
    pub failure: Option<usize>,
}

pub struct State {
    pub session: String,
    pub model: String,
    pub workspace: String,
    pub resumed: bool,

    pub output: Pane,
    pub activity: Pane,
    pub plan: Vec<Todo>,
    pub agents: Vec<AgentRow>,
    pub files: Vec<FileView>,
    pub open_file: usize,
    /// Which row the file explorer starts on, counted from the top of the list.
    /// Top-anchored, unlike a pane's scrollback: a list is read from its first
    /// entry down, and new files arrive at the end without moving the rest.
    pub file_scroll: usize,
    /// Where every pane that is a list rather than a transcript is scrolled to:
    /// PLAN, AGENTS, DEBUG and the three monitors. One place for all six, so a
    /// pane gains a scroll by being added to [`crate::view::scroll_extent`]
    /// rather than by growing a field of its own here.
    pub scrolls: crate::scroll::Scrolls,

    pub usage: Option<Usage>,
    pub prefilled: u64,
    pub generated: u64,
    /// Prompt tokens this session got out of the endpoint's cache. Nothing
    /// summed these before: `Usage` reports one request's cache at a time, so
    /// the only cache reading the window had was the last request's.
    pub cached_prefill: u64,
    pub requests: u32,
    /// What the last request alone cost, as against the session totals.
    pub last_prefill: u64,
    pub last_generated: u64,
    /// The largest single response this session. A total says how much came
    /// back; this says whether anything ever came back long.
    pub best_generated: u64,
    /// Every tool call started this session, working or not.
    pub tool_calls: u32,
    pub rates: Rates,

    /// Calls that failed, oldest first, bounded.
    pub failures: Vec<Failure>,
    /// Which failure is showing its arguments, by position in `failures`. One
    /// at a time: an argument block is several rows and two of them open at
    /// once pushes the rest of the list off the pane.
    pub open_failure: Option<usize>,

    /// The agent's own reading of how full it is. None until it says.
    pub context: Option<ContextFill>,

    /// A drag over one of the text panes, if there is one.
    pub selection: Option<crate::select::Selection>,

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
            output: Pane::new(6000),
            activity: Pane::new(4000),
            plan: Vec::new(),
            agents: Vec::new(),
            files: Vec::new(),
            open_file: 0,
            file_scroll: 0,
            scrolls: crate::scroll::Scrolls::default(),
            usage: None,
            prefilled: 0,
            generated: 0,
            cached_prefill: 0,
            requests: 0,
            last_prefill: 0,
            last_generated: 0,
            best_generated: 0,
            tool_calls: 0,
            rates: Rates::default(),
            failures: Vec::new(),
            open_failure: None,
            context: None,
            selection: None,
            turn: 0,
            phase: Phase::Starting,
            status: String::from("starting the agent"),
            open: HashMap::new(),
        }
    }

    /// What the human typed, echoed into the transcript so the conversation
    /// reads as a conversation.
    pub fn submitted(&mut self, text: &str) {
        self.output.blank_if_needed();
        self.output.say(format!("› {text}"), Tone::Bright);
        self.output.push(Line::new("", Tone::Body));
        self.phase = Phase::Thinking;
        self.status = String::from("thinking");
    }

    /// The file's view, creating it the first time it is mentioned. Opening it
    /// selects it, which is what makes the list follow the agent without anyone
    /// clicking.
    fn file_mut(&mut self, path: &str) -> &mut FileView {
        if let Some(index) = self.files.iter().position(|f| f.path == path) {
            self.show_file(index);
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
            closed: false,
        });
        self.show_file(self.files.len() - 1);
        self.files.last_mut().expect("just pushed")
    }

    /// Fold one frame in, untimed. The window always has a clock and uses
    /// [`State::apply_at`], so this has no caller in the build that ships and
    /// is not waiting for one.
    ///
    /// It stays because the tests are its point: a hundred of them fold a frame
    /// in and assert on what came out, and none of them is about time. Written
    /// through `apply_at` they would each carry a `None` that says nothing, and
    /// the one argument that matters would be the one nobody notices. The
    /// allowance below is what that costs.
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
            Event::TextDelta { d } => self.output.stream(&d, Tone::Body),
            Event::ReasoningDelta { d } => self.output.stream(&d, Tone::Dim),

            Event::ToolStart {
                call_id,
                name,
                brief,
                args,
            } => {
                let kind = Kind::of(&name);
                self.phase = Phase::Working;
                self.status = format!("{name} {brief}");
                self.tool_calls += 1;
                // The plan is a view, not a log line: the call carries the
                // whole updated checklist, so it replaces the last one.
                //
                // A `subagent` call deliberately does NOT open an agent row.
                // The call is the admission, not the child: it returns in
                // microseconds while the child runs for minutes, and it is
                // also how a cancel and a status poll are asked for. The
                // agent.* frames are the child's own lifecycle, and letting
                // both write rows showed every fan-out twice.
                if kind == Kind::Plan {
                    self.plan = read_todos(&args);
                }
                self.activity.say(
                    format!("{:>5}  {}", kind.tag(), subject(kind, &brief, &args)),
                    Tone::Call(kind),
                );
                self.open.insert(call_id, Open { kind, brief, args });
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
                match error {
                    None => self.activity.say(format!("       {summary}"), Tone::Good),
                    Some(error) => {
                        self.remember_failure(open, &error);
                        self.activity.say(format!("       {summary}"), Tone::Bad);
                        // The class and the number the system gave, when the
                        // failure was minted with them. `exit_status 3` is the
                        // whole answer often enough to be worth its own line.
                        self.activity
                            .say(format!("       {}", fault(&error)), Tone::Bad);
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
                        // What to do next, last, where the eye ends up.
                        if let Some(remedy) = error.remedy.as_deref() {
                            self.activity.say(format!("    -> {remedy}"), Tone::Bright);
                        }
                    }
                }
            }

            Event::FileOpen { path, lines, .. } => {
                let file = self.file_mut(&path);
                // Read again after a compaction dropped it: it is back in the
                // model's context, so the tab stops saying it is gone.
                file.closed = false;
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
                // Writing it puts it back in context, same as reading it.
                file.closed = false;
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
                    file.closed = true;
                    file.pane.say("left the context", Tone::Dim);
                }
            }

            Event::AgentSpawn {
                agent_id,
                prompt,
                tools,
            } => self.agents.push(AgentRow::new(agent_id, prompt, tools)),
            Event::AgentStateChanged {
                agent_id,
                state,
                detail,
            } => {
                // Exhaustive on purpose. A wildcard folded `Unknown` into
                // "queued", so a state a newer agent invented showed a child
                // that had already run as one that never started.
                let (word, tone) = match state {
                    noob_proto::AgentState::Done => ("done", Tone::Good),
                    noob_proto::AgentState::Failed => ("failed", Tone::Bad),
                    noob_proto::AgentState::Canceled => ("canceled", Tone::Dim),
                    noob_proto::AgentState::Running => ("running", Tone::Bright),
                    noob_proto::AgentState::Queued => ("queued", Tone::Dim),
                    noob_proto::AgentState::Unknown => ("unknown", Tone::Dim),
                };
                if let Some(agent) = self.agents.iter_mut().find(|a| a.label == agent_id) {
                    agent.state = word;
                    agent.tone = tone;
                    // How it ended replaces what it was last doing: at the end
                    // the reason is the only line worth the room.
                    if let Some(detail) = detail {
                        agent.last = detail;
                    }
                }
            }
            // A child's own output belongs to the child, not to the parent's
            // activity list: eight of them at once made that list unreadable
            // and buried the parent's own work under it.
            Event::AgentOutput { agent_id, line } => {
                match self.agents.iter_mut().find(|a| a.label == agent_id) {
                    Some(agent) => agent.last = line,
                    None => return false,
                }
            }

            Event::UsageReport { usage } => {
                self.prefilled += usage.prefilled();
                self.generated += usage.completion;
                self.cached_prefill += usage.cached_prompt;
                self.last_prefill = usage.prefilled();
                self.last_generated = usage.completion;
                self.best_generated = self.best_generated.max(usage.completion);
                self.requests += 1;
                self.usage = Some(usage);
            }

            Event::Note { line } => self.output.say(line, Tone::Dim),
            Event::Error { line } => {
                self.output.blank_if_needed();
                self.output.say(line, Tone::Bad);
            }

            // The agent's own reading of how full it is, which is a better
            // number than the last request's prompt: it moves at every
            // transcript boundary, including after a tool result mid-turn,
            // where usage only lands once per request.
            Event::Metrics { group, samples, .. } if group == "context" => {
                let value = |key: &str| {
                    samples
                        .iter()
                        .find(|s| s.key == key)
                        .map(|s| (s.value.max(0.0) as u64, s.max.unwrap_or(0.0).max(0.0) as u64))
                };
                match value("used") {
                    Some((used, total)) => {
                        self.context = Some(ContextFill {
                            used,
                            total,
                            compact_at: value("compact_at").map(|(at, _)| at).unwrap_or(0),
                        });
                    }
                    None => return false,
                }
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

    /// The scrollback a view is drawn from, for the views that have one.
    ///
    /// PLAN, AGENTS and the two monitors are built from lists and readings
    /// rather than from lines, so there is nothing there to select. Returning
    /// nothing for them is the honest answer and keeps selection off the views
    /// where it would mean guessing at a layout.
    pub fn pane_of(&self, view: crate::dock::View) -> Option<&Pane> {
        match view {
            crate::dock::View::Output => Some(&self.output),
            crate::dock::View::Activity => Some(&self.activity),
            crate::dock::View::Files => self.files.get(self.open_file).map(|file| &file.pane),
            _ => None,
        }
    }

    /// Show a file, dropping a selection that belonged to the one before it.
    ///
    /// A selection holds line numbers and the view it was made in, not the file.
    /// Left alone, one made in another file would band the same line numbers of
    /// this one and Ctrl-C would copy text nobody highlighted.
    pub fn show_file(&mut self, index: usize) -> bool {
        if index >= self.files.len() || index == self.open_file {
            return false;
        }
        self.open_file = index;
        if self.selection.map(|s| s.view) == Some(crate::dock::View::Files) {
            self.selection = None;
        }
        true
    }

    /// Where the explorer's scrollbar sits, or nothing when every file fits.
    pub fn files_thumb(&self, rows: usize) -> Option<(f32, f32)> {
        let heights = crate::view::file_heights(self.files.len());
        let back = text_geometry::scrollback_for(&heights, rows, self.file_scroll);
        text_geometry::thumb(&heights, rows, back)
    }

    /// Move the explorer list by `by` rows, down the list when `down`.
    ///
    /// Clamped so the last file stays on screen: a list scrolled into empty
    /// space says nothing about what is in it. Returns whether it moved, so a
    /// caller only redraws when it did.
    pub fn scroll_files(&mut self, by: usize, down: bool, rows: usize) -> bool {
        let most = text_geometry::max_scrollback(&crate::view::file_heights(self.files.len()), rows);
        let next = match down {
            true => (self.file_scroll + by).min(most),
            false => self.file_scroll.saturating_sub(by),
        };
        let moved = next != self.file_scroll;
        self.file_scroll = next;
        moved
    }

    /// Bring the row of the open file on screen.
    ///
    /// The agent moves this selection by touching a file, not the pointer, so a
    /// session that touches fifty files would otherwise leave the marked row
    /// scrolled off with nothing on screen saying which file the diff belongs
    /// to. Scrolls by the least it takes, so a list already showing the row is
    /// left where the reader put it.
    pub fn reveal_open_file(&mut self, rows: usize) -> bool {
        if rows == 0 || self.files.is_empty() {
            return false;
        }
        let most = text_geometry::max_scrollback(&crate::view::file_heights(self.files.len()), rows);
        let mut next = self.file_scroll.min(self.open_file);
        if self.open_file + 1 > next + rows {
            next = self.open_file + 1 - rows;
        }
        let next = next.min(most);
        let moved = next != self.file_scroll;
        self.file_scroll = next;
        moved
    }

    /// How much of the context window this session is holding, 0.0 to 1.0.
    ///
    /// The agent's own reading first, because it moves during a turn; the last
    /// request's prompt only as a fallback for a stream that never sent one.
    pub fn context_fraction(&self) -> f32 {
        if let Some(fill) = self.context.filter(|f| f.total > 0) {
            return (fill.used as f32 / fill.total as f32).clamp(0.0, 1.0);
        }
        match self.usage {
            Some(usage) if usage.context_total > 0 => {
                (usage.prompt as f32 / usage.context_total as f32).clamp(0.0, 1.0)
            }
            _ => 0.0,
        }
    }

    /// Keep a failed call, with the arguments its start frame carried.
    ///
    /// Bounded, because a loop that fails on every iteration would otherwise
    /// keep every one of them. The open row is held by position, so dropping
    /// the oldest has to move it or the pane would expand a different failure
    /// than the one that was clicked.
    fn remember_failure(&mut self, open: Option<Open>, error: &noob_proto::ToolError) {
        let (kind, brief, args) = match open {
            Some(open) => (open.kind, open.brief, args_lines(&open.args)),
            // An end whose start this window never saw. The failure is still
            // real and still worth counting, and saying there are no arguments
            // to show is truer than inventing some.
            None => (
                Kind::Other,
                String::new(),
                vec![String::from("this window never saw the call start")],
            ),
        };
        if self.failures.len() >= MAX_FAILURES {
            self.failures.remove(0);
            self.open_failure = match self.open_failure {
                None | Some(0) => None,
                Some(at) => Some(at - 1),
            };
        }
        self.failures.push(Failure {
            kind,
            brief,
            fault: fault(error),
            message: error.message.clone(),
            args,
        });
    }

    /// The debug pane, as one row per visual line.
    ///
    /// The count first, because that is the reading the pane is for. Then one
    /// row per failed call, and the arguments of the one that is open.
    pub fn debug_rows(&self) -> Vec<DebugRow> {
        let mut rows = vec![DebugRow {
            text: format!("failed calls  {}", self.failures.len()),
            tone: if self.failures.is_empty() {
                Tone::Dim
            } else {
                Tone::Bad
            },
            failure: None,
        }];
        if self.failures.is_empty() {
            rows.push(DebugRow {
                text: String::from("nothing has failed this session"),
                tone: Tone::Dim,
                failure: None,
            });
            return rows;
        }
        for (at, failure) in self.failures.iter().enumerate() {
            let open = self.open_failure == Some(at);
            // A plain `+` and `-` rather than a triangle: the mono font here is
            // whatever the system provides, and a glyph it lacks draws as
            // nothing, which is how a row would lose its marker entirely.
            let mark = if open { '-' } else { '+' };
            let subject = match failure.brief.is_empty() {
                true => failure.message.clone(),
                false => format!("{}  {}", failure.brief, failure.message),
            };
            rows.push(DebugRow {
                text: format!("{mark} {:>5}  {subject}", failure.kind.tag()),
                tone: Tone::Bad,
                failure: Some(at),
            });
            if !open {
                continue;
            }
            rows.push(DebugRow {
                text: format!("      {}", failure.fault),
                tone: Tone::Dim,
                failure: Some(at),
            });
            for line in &failure.args {
                rows.push(DebugRow {
                    text: format!("      {line}"),
                    tone: Tone::Body,
                    failure: Some(at),
                });
            }
        }
        rows
    }

    /// Open or close the failure the pane's row `row` belongs to. Returns
    /// whether anything changed, which is what tells the window to redraw.
    ///
    /// Resolved through [`State::debug_rows`], the same list the pane draws, so
    /// a click cannot land on a different failure than the one under it.
    pub fn toggle_failure(&mut self, row: usize) -> bool {
        let Some(at) = self.debug_rows().get(row).and_then(|row| row.failure) else {
            return false;
        };
        self.open_failure = if self.open_failure == Some(at) {
            None
        } else {
            Some(at)
        };
        true
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

/// What went wrong, as a class and a number rather than a sentence.
///
/// `exit_status 3` answers the question on its own often enough to earn its
/// own line above the message. A failure nobody classified says so, because
/// pretending to a class would be worse than admitting there is none.
fn fault(error: &noob_proto::ToolError) -> String {
    match error.code {
        Some(code) => format!("{} {code}", error.kind),
        None => error.kind.clone(),
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
/// Failures kept for the debug pane. A retry loop can fail dozens of times in a
/// minute, and the pane shows a handful of rows at once.
const MAX_FAILURES: usize = 100;

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

/// The argument object as lines, one per field.
///
/// Values are written on one line each: a shell command arrives as a string
/// with newlines in it, and a newline has no glyph, so it would draw as nothing
/// at all while still taking a column. Long ones are cut by the pane, which is
/// the only place that knows how wide it is.
fn args_lines(args: &noob_proto::Value) -> Vec<String> {
    let nothing = || vec![String::from("no arguments were sent")];
    match args {
        noob_proto::Value::Null => nothing(),
        noob_proto::Value::Object(fields) if fields.is_empty() => nothing(),
        noob_proto::Value::Object(fields) => fields
            .iter()
            .map(|(key, value)| format!("{key} = {}", one_line(value)))
            .collect(),
        other => vec![one_line(other)],
    }
}

/// One JSON value on one line. A string loses its quotes, because a path in
/// quotes is a path you cannot paste, and every control character becomes a
/// space.
fn one_line(value: &noob_proto::Value) -> String {
    let raw = match value {
        noob_proto::Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    raw.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Grouped digits, so a six figure token count can be read at a glance. Used by
/// every monitor reading that is a count of tokens.
pub fn thousands(n: u64) -> String {
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
        pane.visible(usize::MAX, 200)
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

        let lines = state.activity.visible(usize::MAX, 200);
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
    /// which is where they used to disappear. The list is built from the
    /// children's own frames: the `subagent` call is the admission, which
    /// returns in microseconds while the child runs for minutes.
    #[test]
    fn a_sub_agent_gets_a_row_of_its_own_that_resolves() {
        let mut state = State::new();
        state.apply(tool_start(
            "s",
            "subagent",
            serde_json::json!({"prompt": "search the web for elcensuradoweb.com"}),
        ));
        assert!(
            state.agents.is_empty(),
            "the admission is not the child: {:?}",
            state.agents
        );
        state.apply(Event::AgentSpawn {
            agent_id: "agent-1".into(),
            prompt: "search the web for elcensuradoweb.com".into(),
            tools: "web".into(),
        });
        assert_eq!(state.agents.len(), 1);
        assert_eq!(state.agents[0].state, "queued");
        assert_eq!(state.agents[0].tools, "web");
        assert!(state.agents[0].brief.contains("elcensuradoweb"));

        state.apply(Event::AgentStateChanged {
            agent_id: "agent-1".into(),
            state: noob_proto::AgentState::Running,
            detail: None,
        });
        assert_eq!(state.agents[0].state, "running");
        // Its own output is its news, and never the parent's activity list.
        let before = state.activity.visible(200, 200).len();
        state.apply(Event::AgentOutput {
            agent_id: "agent-1".into(),
            line: "* websearch search".into(),
        });
        assert_eq!(state.agents[0].last, "* websearch search");
        assert_eq!(state.activity.visible(200, 200).len(), before);

        state.apply(Event::AgentStateChanged {
            agent_id: "agent-1".into(),
            state: noob_proto::AgentState::Failed,
            detail: Some("needs the websearch CLI on PATH".into()),
        });
        assert_eq!(state.agents[0].state, "failed");
        assert_eq!(state.agents[0].tone, Tone::Bad);
        // How it ended replaces what it was last doing.
        assert_eq!(state.agents[0].last, "needs the websearch CLI on PATH");
    }

    /// The tail of a fan-out: a child cancelled before it ever ran still
    /// resolves, and output for a child this window never heard of is dropped
    /// rather than invented.
    #[test]
    fn a_fleet_resolves_every_child_it_was_told_about() {
        let mut state = State::new();
        for n in 1..=3 {
            state.apply(Event::AgentSpawn {
                agent_id: format!("agent-{n}"),
                prompt: format!("task {n}"),
                tools: "read".into(),
            });
        }
        assert_eq!(state.agents.len(), 3);
        state.apply(Event::AgentStateChanged {
            agent_id: "agent-3".into(),
            state: noob_proto::AgentState::Canceled,
            detail: Some("canceled before it started".into()),
        });
        assert_eq!(state.agents[2].state, "canceled");
        assert_eq!(state.agents[0].state, "queued", "one child, not all of them");
        assert!(!state.apply(Event::AgentOutput {
            agent_id: "agent-99".into(),
            line: "from nowhere".into(),
        }));
    }

    /// A state a newer agent invented must not read as one this build knows.
    /// Folding it into "queued" showed a child that had already run as one
    /// that never started.
    #[test]
    fn a_state_this_build_does_not_know_says_so() {
        let mut state = State::new();
        state.apply(Event::AgentSpawn {
            agent_id: "agent-1".into(),
            prompt: "look".into(),
            tools: "read".into(),
        });
        state.apply(Event::AgentStateChanged {
            agent_id: "agent-1".into(),
            state: noob_proto::AgentState::Running,
            detail: None,
        });
        state.apply(Event::AgentStateChanged {
            agent_id: "agent-1".into(),
            state: noob_proto::AgentState::Unknown,
            detail: None,
        });
        assert_eq!(state.agents[0].state, "unknown");
    }

    /// A failure carries its class, the number the system gave it, and what to
    /// do next. This is the row that used to read `error: exit code 1`.
    #[test]
    fn a_failed_call_shows_its_class_and_what_to_do_about_it() {
        let mut state = State::new();
        state.apply(tool_start("b", "bash", serde_json::json!({"cmd": "cargo build"})));
        state.apply(Event::ToolEnd {
            call_id: "b".into(),
            summary: "bash cargo build (2.0s, exit 127)".into(),
            elapsed_ms: 2000,
            error: Some(ToolError {
                kind: "exit_status".into(),
                code: Some(127),
                message: "cargo: command not found".into(),
                detail: None,
                remedy: Some("available here: python3 node".into()),
            }),
        });
        let shown: Vec<&str> = state
            .activity
            .visible(200, 200)
            .iter()
            .map(|line| line.text.trim())
            .collect();
        assert!(shown.contains(&"exit_status 127"), "{shown:?}");
        assert!(shown.contains(&"-> available here: python3 node"), "{shown:?}");
    }

    /// A failed call keeps what was sent to it. Both halves are already on the
    /// wire and both were being written to a line of the activity log and then
    /// dropped, which is why the debug pane needed no protocol change.
    #[test]
    fn a_failed_call_keeps_the_arguments_that_were_sent() {
        let mut state = State::new();
        state.apply(tool_start(
            "b",
            "bash",
            serde_json::json!({"cmd": "cargo build\n --release", "timeout": 30}),
        ));
        // One that works is not a failure and leaves nothing behind.
        state.apply(tool_start("r", "read", serde_json::json!({"path": "a.rs"})));
        state.apply(Event::ToolEnd {
            call_id: "r".into(),
            summary: "read 40 lines".into(),
            elapsed_ms: 3,
            error: None,
        });
        state.apply(Event::ToolEnd {
            call_id: "b".into(),
            summary: "bash cargo build".into(),
            elapsed_ms: 2000,
            error: Some(ToolError {
                kind: "exit_status".into(),
                code: Some(127),
                message: "cargo: command not found".into(),
                detail: None,
                remedy: None,
            }),
        });

        assert_eq!(state.failures.len(), 1);
        let failure = &state.failures[0];
        assert_eq!(failure.kind, Kind::Bash);
        assert_eq!(failure.fault, "exit_status 127");
        assert_eq!(failure.message, "cargo: command not found");
        // One line per field, unquoted, and the newline in the command is a
        // space: a newline has no glyph, so it would draw as nothing.
        assert_eq!(
            failure.args,
            vec![
                String::from("cmd = cargo build  --release"),
                String::from("timeout = 30"),
            ]
        );

        // An end with no start still counts, and says it has nothing to show.
        state.apply(Event::ToolEnd {
            call_id: "never-seen".into(),
            summary: "gone".into(),
            elapsed_ms: 1,
            error: Some(ToolError {
                kind: "internal".into(),
                code: None,
                message: "lost".into(),
                detail: None,
                remedy: None,
            }),
        });
        assert_eq!(state.failures.len(), 2);
        assert_eq!(state.failures[1].fault, "internal");
        assert!(state.failures[1].args[0].contains("never saw the call"));
    }

    /// A tool that sent no arguments says so rather than showing an empty block.
    #[test]
    fn a_call_with_no_arguments_says_there_were_none() {
        assert_eq!(
            args_lines(&serde_json::json!({})),
            vec![String::from("no arguments were sent")]
        );
        assert_eq!(
            args_lines(&noob_proto::Value::Null),
            vec![String::from("no arguments were sent")]
        );
        // Something that is not an object at all is still shown.
        assert_eq!(args_lines(&serde_json::json!([1, 2])), vec![String::from("[1,2]")]);
    }

    /// The count first, then a row per failure, and the arguments of the one
    /// that is open. Clicking the same row again closes it.
    #[test]
    fn the_debug_pane_counts_the_failures_and_opens_the_one_that_was_clicked() {
        let mut state = State::new();
        let rows = state.debug_rows();
        assert_eq!(rows[0].text, "failed calls  0");
        assert!(rows[1].text.contains("nothing has failed"));
        assert!(rows.iter().all(|row| row.failure.is_none()));
        assert!(!state.toggle_failure(1), "nothing to open");

        for (id, name) in [("a", "bash"), ("b", "read")] {
            state.apply(tool_start(id, name, serde_json::json!({"x": id})));
            state.apply(Event::ToolEnd {
                call_id: id.into(),
                summary: "no".into(),
                elapsed_ms: 1,
                error: Some(ToolError {
                    kind: "denied".into(),
                    code: None,
                    message: format!("{name} was refused"),
                    detail: None,
                    remedy: None,
                }),
            });
        }
        let rows = state.debug_rows();
        assert_eq!(rows[0].text, "failed calls  2");
        assert_eq!(rows.len(), 3, "closed, each failure is one row");
        assert!(rows[1].text.starts_with("+ "), "closed rows say so");
        assert_eq!(rows[1].failure, Some(0));
        assert_eq!(rows[2].failure, Some(1));

        // Open the second one: its rows appear under it and nowhere else.
        assert!(state.toggle_failure(2));
        let rows = state.debug_rows();
        assert_eq!(rows[2].failure, Some(1));
        assert!(rows[2].text.starts_with("- "), "an open row says so");
        assert!(rows[1].text.starts_with("+ "), "and the other stays closed");
        assert!(rows[3].text.contains("denied"), "{:?}", rows[3].text);
        assert!(rows[4].text.contains("x = b"), "{:?}", rows[4].text);
        assert!(rows.iter().skip(3).all(|row| row.failure == Some(1)));

        // The same row again closes it, and only one is ever open.
        assert!(state.toggle_failure(2));
        assert_eq!(state.debug_rows().len(), 3);
        assert!(state.toggle_failure(1));
        assert_eq!(state.open_failure, Some(0));
        // With a block open above it, the second failure's row has moved down
        // past that block, and the click follows the rows rather than the list.
        let rows = state.debug_rows();
        let moved = rows
            .iter()
            .position(|row| row.failure == Some(1))
            .expect("the second failure still has a row");
        assert_eq!(moved, 4, "{:?}", rows.iter().map(|r| &r.text).collect::<Vec<_>>());
        assert!(state.toggle_failure(moved));
        assert_eq!(state.open_failure, Some(1));
    }

    /// The list is bounded, and the row that is open moves with it: it is held
    /// by position, so dropping the oldest would otherwise expand a different
    /// failure than the one that was clicked.
    #[test]
    fn the_failure_list_is_bounded_and_the_open_row_follows_it() {
        let mut state = State::new();
        let fail = |state: &mut State, n: usize| {
            let id = format!("c{n}");
            state.apply(tool_start(&id, "bash", serde_json::json!({"n": n})));
            state.apply(Event::ToolEnd {
                call_id: id,
                summary: "no".into(),
                elapsed_ms: 1,
                error: Some(ToolError {
                    kind: "timeout".into(),
                    code: None,
                    message: format!("call {n} timed out"),
                    detail: None,
                    remedy: None,
                }),
            });
        };
        for n in 0..MAX_FAILURES {
            fail(&mut state, n);
        }
        assert_eq!(state.failures.len(), MAX_FAILURES);
        state.open_failure = Some(1);
        fail(&mut state, MAX_FAILURES);
        assert_eq!(state.failures.len(), MAX_FAILURES, "bounded");
        assert!(state.failures[0].message.contains("call 1"), "the oldest fell off");
        assert_eq!(state.open_failure, Some(0), "and the open one slid with it");
        // The one that was open falling off leaves nothing open, rather than
        // leaving a position pointing at whatever took its place.
        state.open_failure = Some(0);
        fail(&mut state, MAX_FAILURES + 1);
        assert_eq!(state.open_failure, None);
    }

    /// The agent's own reading of how full it is beats the last request's
    /// prompt: it moves during a turn, and it is the number compaction acts on.
    #[test]
    fn the_live_context_reading_outranks_the_last_request() {
        let mut state = State::new();
        state.apply(Event::UsageReport {
            usage: noob_proto::Usage {
                prompt: 1_000,
                cached_prompt: 0,
                completion: 10,
                context_total: 10_000,
            },
        });
        assert!((state.context_fraction() - 0.1).abs() < 0.001);
        assert!(state.apply(Event::Metrics {
            group: "context".into(),
            at_ms: 42,
            samples: vec![
                noob_proto::Sample {
                    key: "used".into(),
                    label: "context used".into(),
                    value: 8_000.0,
                    max: Some(10_000.0),
                    unit: Some("tokens".into()),
                },
                noob_proto::Sample {
                    key: "compact_at".into(),
                    label: "compacts at".into(),
                    value: 7_500.0,
                    max: Some(10_000.0),
                    unit: Some("tokens".into()),
                },
            ],
        }));
        assert!((state.context_fraction() - 0.8).abs() < 0.001);
        assert_eq!(state.context.unwrap().compact_at, 7_500);
        // A group this window does not draw is skipped, not guessed at.
        assert!(!state.apply(Event::Metrics {
            group: "weather".into(),
            at_ms: 43,
            samples: Vec::new(),
        }));
    }

    /// Live output from a running command scrolls under the call that made it,
    /// in the call's own colour, so two concurrent tools cannot be confused.
    #[test]
    fn a_running_command_scrolls_its_output_under_its_own_row() {
        let mut state = State::new();
        state.apply(tool_start("b", "bash", serde_json::json!({"cmd": "cargo build"})));
        for line in ["compiling noob", "compiling no0b"] {
            state.apply(Event::ToolProgress {
                call_id: "b".into(),
                line: line.into(),
            });
        }
        let shown: Vec<(String, Tone)> = state
            .activity
            .visible(200, 200)
            .iter()
            .map(|line| (line.text.trim().to_string(), line.tone))
            .collect();
        assert_eq!(shown[1].0, "compiling noob");
        assert_eq!(shown[1].1, Tone::Call(Kind::Bash), "{shown:?}");
        assert_eq!(shown[2].0, "compiling no0b");
    }

    /// Compaction ends a file's life in the model's context. The page stays
    /// readable; the tab stops claiming the agent is holding it, and reading
    /// it again puts it back.
    #[test]
    fn a_compacted_file_is_marked_as_gone_from_the_context() {
        let mut state = State::new();
        let open = |lines| Event::FileOpen {
            path: "src/main.rs".into(),
            lines,
            call_id: Some("r".into()),
        };
        state.apply(open(40));
        assert!(!state.files[0].closed);
        state.apply(Event::FileClose {
            path: "src/main.rs".into(),
            call_id: None,
        });
        assert!(state.files[0].closed);
        state.apply(open(40));
        assert!(
            !state.files[0].closed,
            "reading it again put it back in context"
        );
        state.apply(Event::FileClose {
            path: "src/main.rs".into(),
            call_id: None,
        });
        assert!(
            state.files[0]
                .pane
                .visible(20, 200)
                .iter()
                .any(|line| line.text.contains("left the context"))
        );
    }

    /// One row per file, selected by whatever the agent just touched, so the
    /// list follows it without anyone clicking.
    #[test]
    fn files_get_a_row_each_and_the_newest_is_selected() {
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

    fn with_files(count: usize) -> State {
        let mut state = State::new();
        for n in 0..count {
            state.apply(Event::FileOpen {
                path: format!("src/f{n}.rs"),
                lines: 1,
                call_id: None,
            });
        }
        state
    }

    /// Showing another file drops a selection made in the one before it. The
    /// selection holds line numbers and the view it was made in, so it would
    /// otherwise band the same line numbers of a different file.
    #[test]
    fn showing_another_file_drops_the_selection_from_the_last_one() {
        let mut state = with_files(3);
        state.selection = Some(crate::select::Selection::new(
            crate::dock::View::Files,
            crate::select::Spot::new(1, 0),
        ));
        assert!(state.show_file(0), "file 0 was not the one showing");
        assert_eq!(state.open_file, 0);
        assert!(state.selection.is_none(), "the old file's selection survived");

        // A selection somewhere else is none of this pane's business.
        state.selection = Some(crate::select::Selection::new(
            crate::dock::View::Output,
            crate::select::Spot::new(1, 0),
        ));
        assert!(state.show_file(2));
        assert!(state.selection.is_some(), "the transcript's selection was dropped");
        // Asking for the file already showing, or one that does not exist,
        // changes nothing at all.
        assert!(!state.show_file(2));
        assert!(!state.show_file(99));
        assert_eq!(state.open_file, 2);
    }

    /// The list scrolls in both directions and stops at both ends. Scrolling
    /// past the last file would show empty rows, which says nothing about what
    /// the agent has touched.
    #[test]
    fn the_file_list_scrolls_and_stops_at_both_ends() {
        let mut state = with_files(20);
        assert!(!state.scroll_files(3, false, 8), "already at the top");
        assert_eq!(state.file_scroll, 0);
        assert!(state.scroll_files(5, true, 8));
        assert_eq!(state.file_scroll, 5);
        // Twenty files in an eight row list leaves twelve rows to scroll.
        assert!(state.scroll_files(99, true, 8));
        assert_eq!(state.file_scroll, 12);
        assert!(!state.scroll_files(1, true, 8), "already at the bottom");
        assert!(state.scroll_files(99, false, 8));
        assert_eq!(state.file_scroll, 0);
        // A list that fits has nowhere to go and no thumb to say otherwise.
        let short = with_files(4);
        assert!(short.files_thumb(8).is_none());
        assert!(state.files_thumb(8).is_some(), "twenty files in eight rows");
    }

    /// The agent moves the selection by touching a file, so a list scrolled
    /// elsewhere has to come back to it: otherwise the marked row is off screen
    /// and nothing says which file the diff belongs to.
    #[test]
    fn the_list_comes_back_to_the_file_the_agent_touched() {
        let mut state = with_files(20);
        state.file_scroll = 0;
        state.open_file = 15;
        assert!(state.reveal_open_file(5), "row 15 is not in rows 0 to 4");
        assert_eq!(state.file_scroll, 11, "scrolled by the least it takes");
        // Already showing, so it is left where the reader put it.
        state.open_file = 13;
        assert!(!state.reveal_open_file(5));
        assert_eq!(state.file_scroll, 11);
        // And upwards, when the touched file is above the window.
        state.open_file = 2;
        assert!(state.reveal_open_file(5));
        assert_eq!(state.file_scroll, 2);
        // A list with nothing in it, and a pane with no room, are both no-ops
        // rather than a position nothing can be drawn at.
        assert!(!State::new().reveal_open_file(5));
        assert!(!state.reveal_open_file(0));
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
        assert_eq!(texts(&state.output), ["Hello there", "second line"]);
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
        let lines: Vec<_> = state.output.visible(usize::MAX, 200);
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

    /// The budget was one string in the title strip and is a set of monitor
    /// readings now, so this asserts the numbers rather than the sentence they
    /// used to be written into. Prefill is the prompt minus the cache, because
    /// summing raw prompt tokens counts work nobody did.
    #[test]
    fn the_budget_sums_prefill_and_cache_apart_and_keeps_the_longest_answer() {
        let mut state = State::new();
        for (prompt, cached, completion) in [(1000, 0, 10), (1200, 1000, 400), (1500, 1200, 90)] {
            state.apply(Event::UsageReport {
                usage: Usage {
                    prompt,
                    cached_prompt: cached,
                    completion,
                    context_total: 65536,
                },
            });
        }
        assert_eq!(state.prefilled, 1500, "1000 + 200 + 300");
        assert_eq!(state.cached_prefill, 2200, "nothing summed these before");
        assert_eq!(state.generated, 500);
        assert_eq!(state.best_generated, 400, "the longest single answer");
        assert_eq!(state.requests, 3);
        assert_eq!(state.last_prefill, 300);
    }

    /// Every call started, whether it worked or not: the reading is how much
    /// the agent asked for, not how much of it succeeded.
    #[test]
    fn every_tool_call_is_counted_once() {
        let mut state = State::new();
        assert_eq!(state.tool_calls, 0);
        for (id, name) in [("a", "read"), ("b", "bash"), ("c", "bash")] {
            state.apply(tool_start(id, name, serde_json::json!({})));
        }
        state.apply(Event::ToolEnd {
            call_id: "a".into(),
            summary: "ok".into(),
            elapsed_ms: 1,
            error: None,
        });
        assert_eq!(state.tool_calls, 3);
    }

    #[test]
    fn a_pane_keeps_only_its_last_lines() {
        let mut pane = Pane::new(8);
        for n in 0..40 {
            pane.say(format!("{n}"), Tone::Body);
        }
        assert_eq!(pane.visible(usize::MAX, 200).len(), 8);
        assert_eq!(pane.visible(3, 200).last().unwrap().text, "39");
    }

    /// The defect this whole change exists for. A pane following the live end
    /// used to hand the shaper as many logical lines as rows fit, so a line
    /// that wrapped overflowed the clip box, the newest rows were discarded,
    /// and no scroll position could reach them because forward stops at zero.
    #[test]
    fn the_end_of_a_wrapped_message_is_on_screen_at_the_live_end() {
        let mut pane = Pane::new(100);
        pane.say("short", Tone::Body);
        // Five rows in a fifty column box, so six rows of content in total.
        pane.say("x".repeat(250), Tone::Body);
        let (rows, cols) = (4, 50);

        let window = pane.window(rows, cols);
        assert_eq!(
            window.first + window.count,
            2,
            "the newest line has to be in the window"
        );
        let (top, height) = pane
            .band_of(rows, cols, 1)
            .expect("the long line is on screen");
        assert_eq!(
            top + height, rows,
            "its last row must be the last row of the pane, not clipped away"
        );
    }

    /// And the rows above it are reachable, a row at a time rather than a line
    /// at a time: a five row paragraph used to be one indivisible scroll step.
    #[test]
    fn scrolling_back_through_a_wrapped_line_moves_one_row_at_a_time() {
        let mut pane = Pane::new(100);
        pane.say("x".repeat(250), Tone::Body);
        let (rows, cols) = (2, 50);
        // Five rows of content in a two row pane leaves three to scroll through.
        assert_eq!(pane.window(rows, cols).skip, 3, "showing the last two rows");
        assert!(pane.scroll_back(1, rows, cols));
        assert_eq!(pane.window(rows, cols).skip, 2, "one row back, not one line");
        assert!(pane.scroll_back(99, rows, cols));
        assert_eq!(pane.scrollback, 3, "and it stops at the top of the line");
        assert_eq!(pane.window(rows, cols).skip, 0);
        assert!(!pane.scroll_back(1, rows, cols), "already at the oldest row");
    }

    /// A click on the second visual row of a wrapped line has to select text
    /// from past the wrap, not from the start of the line.
    #[test]
    fn a_row_of_a_wrapped_line_maps_past_the_wrap() {
        let mut pane = Pane::new(100);
        pane.say("x".repeat(120), Tone::Body);
        let (rows, cols) = (3, 50);
        assert_eq!(pane.spot_in(rows, cols, 0), Some((0, 0)));
        assert_eq!(
            pane.spot_in(rows, cols, 1),
            Some((0, 50)),
            "the second row starts fifty characters in"
        );
        assert_eq!(pane.spot_in(rows, cols, 2), Some((0, 100)));
        assert_eq!(
            pane.spot_in(rows, cols, 3),
            None,
            "and below the text is nothing, not the last character"
        );
    }

    /// The thumb has to know the pane is overflowing. Counting lines, four
    /// lines in a five row pane looked like it fitted while it was in fact
    /// twelve rows tall.
    #[test]
    fn the_thumb_appears_when_wrapped_content_overflows() {
        let mut pane = Pane::new(100);
        for _ in 0..4 {
            pane.say("x".repeat(150), Tone::Body);
        }
        assert!(
            pane.thumb(5, 50).is_some(),
            "twelve rows of content in a five row pane is an overflow"
        );
        assert!(
            pane.thumb(5, 600).is_none(),
            "and the same four lines in a wide pane fit, so no thumb"
        );
    }

    /// Streamed text grows the tail line in place, so its wrapped height moves
    /// without the line count changing. A cache keyed only on the count would
    /// keep reporting the old height forever.
    #[test]
    fn streaming_into_the_tail_line_remeasures_it() {
        let mut pane = Pane::new(100);
        pane.stream("short", Tone::Body);
        assert_eq!(pane.window(4, 20).skip, 0);
        pane.stream(&"y".repeat(80), Tone::Body);
        // The one line is now five rows tall in a twenty column box, so a two
        // row pane has to report three rows above it.
        assert_eq!(
            pane.window(2, 20).skip, 3,
            "the tail line was remeasured after it grew"
        );
    }

    #[test]
    fn scrolling_back_then_receiving_returns_to_the_live_end() {
        let mut pane = Pane::new(100);
        for n in 0..50 {
            pane.say(format!("{n}"), Tone::Body);
        }
        assert!(pane.scroll_back(10, 10, 200));
        assert_eq!(pane.visible(10, 200).last().unwrap().text, "39");
        pane.say("new", Tone::Body);
        assert_eq!(pane.scrollback, 0);
        assert_eq!(pane.visible(10, 200).last().unwrap().text, "new");
    }

    #[test]
    fn scrolling_stops_at_both_ends() {
        let mut pane = Pane::new(100);
        for n in 0..20 {
            pane.say(format!("{n}"), Tone::Body);
        }
        assert!(pane.scroll_back(999, 10, 200));
        assert_eq!(pane.scrollback, 10);
        assert!(!pane.scroll_back(1, 10, 200), "already at the oldest line");
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
        assert_eq!(pane.thumb(10, 200), None, "everything fits");
        assert_eq!(pane.thumb(20, 200), None);
        for n in 10..100 {
            pane.say(format!("{n}"), Tone::Body);
        }
        let (top, size) = pane.thumb(10, 200).expect("ninety lines do not fit in ten");
        assert!((size - 0.1).abs() < 0.01, "{size}");
        assert!((top - 0.9).abs() < 0.01, "at the live end: {top}");
        pane.scroll_back(90, 10, 200);
        let (top, _) = pane.thumb(10, 200).unwrap();
        assert_eq!(top, 0.0, "scrolled to the very top");
        // The thumb never runs off the end of its track.
        for rows in [1, 3, 7, 10, 99] {
            if let Some((top, size)) = pane.thumb(rows, 200) {
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
        let lines = state.files[0].pane.visible(usize::MAX, 200);
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
        assert!(pane.fence_before(10, 200).0.is_some(), "the block is still open");
        // The window is the lines at the end; `before` is everything above it,
        // so the closing fence has to be above the window to count as closed.
        pane.say("```", Tone::Body);
        pane.say("back to prose", Tone::Body);
        assert!(pane.fence_before(1, 200).0.is_none(), "and now it is closed");
        assert!(pane.fence_before(2, 200).0.is_some(), "the window still holds the fence");
        // What the human typed is not the model's Markdown, so it cannot open
        // a block that the model then has to close.
        let mut typed = Pane::new(100);
        typed.say("run ```this```", Tone::Bright);
        typed.say("prose", Tone::Body);
        assert!(typed.fence_before(1, 200).0.is_none());
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
