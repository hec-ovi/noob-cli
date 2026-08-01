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

/// How every pane of text in the window breaks its rows.
///
/// Prose is read in words, so a row ends at a blank and not in the middle of
/// one. The rule itself is `text-geometry`'s and is written down once there:
/// the heights a pane is scrolled by, the row a pointer lands on, the band that
/// is painted and the rows the renderer draws all pass this same value, which
/// is what makes the characters on a row and the characters a selection there
/// copies the same characters. The prompt is the one box that does not use it,
/// because its caret is placed by counting `row * cols + column`.
pub const PANE_WRAP: text_geometry::Break = text_geometry::Break::Word;

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub text: String,
    pub tone: Tone,
    /// The line this is in its file, for the gutter. Only file views set it.
    pub number: Option<u32>,
    /// How many characters at the front of this line are a subordinate column,
    /// drawn in the dim tone rather than the line's own. The activity list uses
    /// it for the clock reading in front of every row.
    ///
    /// Carried on the line rather than worked out from the text at draw time:
    /// the column is written by whoever pushed the line, and a drawing that
    /// sniffed for it would be a second rule about what a row looks like.
    pub gutter: usize,
    /// What this line looks like on screen, when that is not what was written.
    ///
    /// Only a Markdown pane sets it, and only for the lines whose marks it
    /// eats. Everything measured about a line is measured on this: its rows,
    /// the columns a selection on it holds, and the characters a copy returns.
    /// A reader selects the glyphs he can see, so `- **read** a file` is
    /// thirteen columns of `• read a file` and not seventeen of the source.
    shown: Option<String>,
    /// Where a fenced block stood in front of this line, so the pane can be
    /// drawn from any row of it without rescanning everything above.
    ///
    /// Remembered rather than worked out again, which is also what keeps the
    /// drawing and the measuring in step once the pane is full: the line that
    /// opened a block can fall off the top while its body is still on screen,
    /// and a scan of what is left would call that body prose.
    fence: crate::markdown::Fence,
}

impl Line {
    pub fn new(text: impl Into<String>, tone: Tone) -> Line {
        Line {
            text: text.into(),
            tone,
            number: None,
            gutter: 0,
            shown: None,
            fence: crate::markdown::Fence::default(),
        }
    }

    pub fn at(mut self, number: u32) -> Line {
        self.number = Some(number);
        self
    }

    /// A line whose first `gutter` characters are a subordinate column.
    pub fn with_gutter(mut self, gutter: usize) -> Line {
        self.gutter = gutter;
        self
    }

    /// The line split into that column and the rest of it, which is what the
    /// drawing needs to give the two of them different tones.
    pub fn split_gutter(&self) -> (&str, &str) {
        let at = self
            .text
            .char_indices()
            .nth(self.gutter)
            .map_or(self.text.len(), |(byte, _)| byte);
        self.text.split_at(at)
    }

    /// The text this line is drawn as, which is the text everything else about
    /// it is counted in.
    pub fn shown(&self) -> &str {
        self.shown.as_deref().unwrap_or(&self.text)
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
    /// Whether the body lines of this pane are Markdown, drawn as the thing
    /// they describe rather than as the marks that describe it.
    rendered: bool,
    /// Every line is one visual row, clipped by the painter, never wrapped.
    one_row: bool,
    /// Where a fenced block stands after the last line pushed, so the next one
    /// knows whether it is prose or code. Carried rather than rescanned: a full
    /// pane is thousands of lines and one arrives per token.
    fence: crate::markdown::Fence,
    /// The same, in front of the last line pushed. `stream` grows that line in
    /// place, and a line that grew has to be drawn again from the state it
    /// started in.
    tail_fence: crate::markdown::Fence,
}

/// The wrapped height of every line, and what it was computed for.
#[derive(Default)]
struct Heights {
    rows: Vec<usize>,
    cols: usize,
    stale: bool,
}

/// What a line reaches the screen as, and `None` when that is the text itself.
///
/// Only the model's prose is Markdown: what the human typed and what the
/// harness noted are shown as written, which is the same rule the drawing keeps
/// and the same one the fence is carried by. Advances `fence` either way, since
/// a fenced block runs across the lines rather than inside one and a pane that
/// renders nothing still has to know where the blocks are.
fn drawn(text: &str, tone: Tone, fence: &mut crate::markdown::Fence, render: bool) -> Option<String> {
    if tone != Tone::Body {
        return None;
    }
    if !render {
        fence.toggle(text);
        return None;
    }
    let shown = crate::markdown::shown(text, fence);
    (shown != text).then_some(shown)
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
            rendered: false,
            one_row: false,
            fence: crate::markdown::Fence::default(),
            tail_fence: crate::markdown::Fence::default(),
        }
    }

    /// A pane whose lines never wrap: each is one row however long it runs,
    /// clipped by the painter, like the file explorer clips its names. What
    /// a list needs - a row is an entry, and an entry is a row.
    pub fn clipped(mut self) -> Pane {
        self.one_row = true;
        self
    }

    /// Whether this pane's painter clips rather than wraps.
    pub fn one_row(&self) -> bool {
        self.one_row
    }

    /// Park the window on a top-anchored first visual row: how a surface
    /// with an offset of its own (the call popup) borrows this pane's row
    /// arithmetic.
    pub fn anchor_first(&mut self, first: usize, rows: usize, cols: usize) {
        let back = text_geometry::scrollback_for(&self.heights(cols), rows, first);
        self.scrollback = back;
    }

    /// Which line and wrapped row a visual row of the window shows, with no
    /// column resolution: what a band walk wants.
    pub fn spot_row(&self, rows: usize, cols: usize, row: usize) -> Option<(usize, usize)> {
        let w = self.window(rows, cols);
        let (line, wrapped) = text_geometry::row_at(&self.heights(cols), w, row)?;
        Some((self.dropped + line, wrapped))
    }

    /// A pane whose body lines are Markdown.
    ///
    /// It renders them as it pushes them, and everything it reports afterwards
    /// is about the rendering: how many rows a line takes, which character a
    /// row holds, and what a selection over it copies. The alternative was a
    /// map from a drawn column back to a source column, threaded through the
    /// hit test, the band and the clipboard, so that pointing at the last glyph
    /// of a bullet copied the four asterisks around it. A reader selects what
    /// he can see.
    pub fn rendered(mut self) -> Pane {
        self.rendered = true;
        self
    }

    pub fn push(&mut self, mut line: Line) {
        self.tail_fence = self.fence.clone();
        line.fence = self.fence.clone();
        line.shown = drawn(&line.text, line.tone, &mut self.fence, self.rendered);
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
            let grew = match self.lines.back_mut() {
                Some(last) if last.tone == tone => {
                    last.text.push_str(part);
                    true
                }
                _ => false,
            };
            if grew {
                self.redraw_tail();
            } else {
                self.push(Line::new(part, tone));
            }
        }
        // The tail line grew in place, which changes its wrapped height
        // without changing the line count.
        self.heights.borrow_mut().stale = true;
        self.scrollback = 0;
    }

    /// Draw the tail line again, from the fence state in front of it.
    ///
    /// A line that grew under `stream` is a line whose rendering moved with it:
    /// a half-written `**bold` is prose until its closing pair arrives, at
    /// which point four characters leave the screen.
    fn redraw_tail(&mut self) {
        let mut fence = self.tail_fence.clone();
        let render = self.rendered;
        if let Some(last) = self.lines.back_mut() {
            let shown = drawn(&last.text, last.tone, &mut fence, render);
            last.shown = shown;
        }
        self.fence = fence;
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
                // One buffer down the whole pane rather than one per line: a
                // full pane is thousands of lines and this runs whenever one
                // arrives.
                let mut wrapped = Vec::new();
                for line in &self.lines {
                    if self.one_row {
                        cache.rows.push(1);
                        continue;
                    }
                    text_geometry::rows_into(line.shown(), cols, PANE_WRAP, &mut wrapped);
                    cache.rows.push(wrapped.len());
                }
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

    /// Which line a visual row is showing, and which character of it sits
    /// `at` columns into that row. `None` for a row below the last line.
    ///
    /// A row does not start at `row * cols` characters into its line, because
    /// the pane wraps at blanks: the row a reader is pointing at starts
    /// wherever the words let it. Both halves come from `text-geometry`, the
    /// row from the cached heights and the characters on it from the line's own
    /// text, so this cannot drift from what the renderer drew.
    ///
    /// `at` is clamped to the row it landed on, so a pointer out in the empty
    /// space to the right of a short row takes that row's last character rather
    /// than reaching into the row below it.
    pub fn spot_in(&self, rows: usize, cols: usize, row: usize, at: usize) -> Option<(usize, usize)> {
        let w = self.window(rows, cols);
        let (line, wrapped) = text_geometry::row_at(&self.heights(cols), w, row)?;
        let span = *self.rows_of_line(self.dropped + line, cols).get(wrapped)?;
        Some((self.dropped + line, span.start + at.min(span.len())))
    }

    /// The visual rows one line is drawn as, by the rule the renderer draws it
    /// by. Empty for a line that has been evicted.
    ///
    /// Counted in the text that reaches the screen, which in a Markdown pane is
    /// shorter than the text that was written: a row of the source is not a row
    /// anybody can point at.
    pub fn rows_of_line(&self, absolute: usize, cols: usize) -> Vec<text_geometry::Row> {
        let Some(line) = self.line(absolute) else {
            return Vec::new();
        };
        if self.one_row {
            // One span: the row the wrap would have drawn first. What is on
            // screen is what a selection takes ("a reader selects what he
            // can see"); the whole entry lives on the popup, which selects.
            let mut rows = text_geometry::rows_in(line.shown(), cols, PANE_WRAP);
            rows.truncate(1);
            return rows;
        }
        text_geometry::rows_in(line.shown(), cols, PANE_WRAP)
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

    /// Swap one character of a line that has already been pushed, by absolute
    /// number.
    ///
    /// Only ever one single-byte character for another, and that is checked
    /// rather than assumed, which is why no height has to be measured again in
    /// a plain pane: the line is exactly as long as it was, so every cached
    /// wrapped height and every selection column still holds. It exists for the
    /// parallel mark, which is written into a row that was pushed before the
    /// window could know anything ran beside it.
    ///
    /// A Markdown pane is measured in what it draws, and a swapped character
    /// can be a mark, so there the line is drawn again from the fence state it
    /// carries and the heights go stale.
    fn mark(&mut self, absolute: usize, column: usize, glyph: char) {
        if !glyph.is_ascii() {
            return;
        }
        let Some(index) = absolute.checked_sub(self.dropped) else {
            return;
        };
        let Some(line) = self.lines.get_mut(index) else {
            return;
        };
        if !line.text.is_char_boundary(column) || !line.text.is_char_boundary(column + 1) {
            return;
        }
        line.text.replace_range(column..column + 1, glyph.encode_utf8(&mut [0u8; 4]));
        if !self.rendered {
            return;
        }
        let mut fence = line.fence.clone();
        let shown = drawn(&line.text, line.tone, &mut fence, true);
        line.shown = shown;
        self.heights.borrow_mut().stale = true;
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

    /// Put the window where a dragged scrollbar says, as a fraction of the
    /// whole travel: 0.0 is the oldest row still held, 1.0 the live end.
    /// The scrollback this pane keeps is counted from the end, so the
    /// fraction inverts here and nowhere else.
    pub fn scroll_to(&mut self, fraction: f32, rows: usize, cols: usize) -> bool {
        let most = text_geometry::max_scrollback(&self.heights(cols), rows);
        let back = ((1.0 - fraction.clamp(0.0, 1.0)) * most as f32).round() as usize;
        let moved = back != self.scrollback;
        self.scrollback = back;
        moved
    }

    /// Whether a fenced code block is open where this window starts.
    ///
    /// A transcript is drawn from a scrolling window, so a block that opened
    /// above it would otherwise render as prose. Each line carries the state in
    /// front of it, written down as it arrived: rescanning the lines above
    /// costs a walk of the whole pane every frame, and once the pane is full it
    /// gives a different answer from the one the line was measured with, since
    /// the line that opened the block can have fallen off the top.
    pub fn fence_before(&self, rows: usize, cols: usize) -> crate::markdown::Fence {
        let start = self.window(rows, cols).first;
        self.lines
            .get(start)
            .map_or_else(|| self.fence.clone(), |line| line.fence.clone())
    }

    /// Change one line's tone in place, keeping its text and so its
    /// geometry: how a call's row comes to say it failed without the pane
    /// gaining a row.
    pub fn retone(&mut self, absolute: usize, tone: Tone) {
        if let Some(index) = absolute.checked_sub(self.dropped)
            && let Some(line) = self.lines.get_mut(index)
        {
            line.tone = tone;
        }
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

pub struct AgentRow {
    /// The wire id (`agent-N`), the key every agent.* frame correlates by.
    pub label: String,
    /// The agent's number for a human: 1-based, in spawn order, stable for
    /// the whole session however many rows come and go around it. The list
    /// row reads `[N] Agent` and the output tab `[N] AGENT - OUTPUT`.
    pub ordinal: usize,
    pub brief: String,
    pub state: &'static str,
    pub tone: Tone,
    /// The tools this child was given, which is what says whether it can
    /// write. Empty when the frame did not carry one.
    pub tools: String,
    /// The last thing it said, live while it runs and its reason once it ends.
    /// A fleet of eight children is unreadable as eight names and no news.
    pub last: String,
    /// Everything it has said, one line per `agent.output` frame, bounded:
    /// what the `[N] AGENT - OUTPUT` tab shows.
    pub pane: Pane,
}

impl std::fmt::Debug for AgentRow {
    /// By hand because the pane is a scrollback, not a fact about the row:
    /// a test failure wants to say which children were listed, not print
    /// two thousand lines of one of them.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentRow")
            .field("label", &self.label)
            .field("ordinal", &self.ordinal)
            .field("state", &self.state)
            .field("last", &self.last)
            .finish_non_exhaustive()
    }
}

impl AgentRow {
    fn new(label: String, ordinal: usize, brief: String, tools: String) -> AgentRow {
        AgentRow {
            label,
            ordinal,
            brief,
            state: "queued",
            tone: Tone::Dim,
            tools,
            last: String::new(),
            pane: Pane::new(2000),
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
    /// One word for both busy phases, on purpose. Thinking and working are the
    /// endpoint generating and the agent running what it generated, and from
    /// outside they are the same thing: the model has the turn and you are
    /// waiting. The orb turns for both, so two words up here would have said
    /// something the one moving thing on screen does not.
    pub fn word(self) -> &'static str {
        match self {
            Phase::Starting => "STARTING",
            Phase::Ready => "READY",
            Phase::Thinking | Phase::Working => "INFERRING",
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
    /// at [`SAMPLES`].
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

/// How many per-request rate samples are kept, per phase.
///
/// A median needs the samples themselves: a running sum gives an average and
/// has already forgotten which requests it was made of. Bounded because a list
/// that grew one entry per request forever would be a session log, and the
/// newest samples are the ones that describe the machine as it is now.
const SAMPLES: usize = 512;

/// Record one request's rate, dropping the oldest once the ring is full.
fn sample(rates: &mut Vec<f32>, tokens: u64, seconds: f64) {
    rates.push((tokens as f64 / seconds) as f32);
    if rates.len() > SAMPLES {
        rates.remove(0);
    }
}

/// A call in flight, kept so its end can complete the record its start
/// frame opened without searching for it.
struct Open {
    /// The ordinal of its [`Call`], not a position in [`State::calls`]: that
    /// list is bounded and slides, and an index into it would come to mean a
    /// different call the moment the oldest fell off.
    call: usize,
}

/// What came back from a call that failed, kept whole.
///
/// The agent sends the tool's own result body as `detail` and only when the
/// call failed, so this is the one place the window ever sees what a tool
/// actually returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    /// The class and the number the system gave, already read into one line.
    pub fault: String,
    pub message: String,
    pub detail: Option<String>,
    pub remedy: Option<String>,
}

/// Everything the window ever learned about one tool call.
///
/// The activity list is text and stays text: this is the record beside it, so a
/// row can be clicked back into the call that wrote it. Bounded like every other
/// store here, and keyed to the **absolute** line numbers of the rows it wrote,
/// because the activity pane evicts from the front and a position in its deque
/// stops meaning the same row as soon as it does.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub call_id: String,
    pub kind: Kind,
    /// The tool the model asked for: `bash`, `skill`, `mcp_call`.
    pub name: String,
    pub brief: String,
    /// The particular thing that tool reached for. See [`invoked`].
    pub target: String,
    /// The arguments the model generated, exactly as they arrived.
    pub args: noob_proto::Value,
    pub turn: u32,
    /// When the start frame and the end frame reached this window, in the
    /// window's own monotonic seconds. `None` while nothing has timed the frame,
    /// which is every test that folds an event in without a clock.
    pub started: Option<f64>,
    pub ended: Option<f64>,
    /// How long the agent says it ran. Zero for a canned or cancelled outcome,
    /// so it is a label rather than the interval the overlap is read off.
    pub elapsed_ms: Option<u64>,
    pub summary: String,
    /// Whether an end frame has arrived for it. Its own flag rather than
    /// `ended.is_some()`: the arrival times are only there when something timed
    /// the frame, and "nobody was holding a clock" and "it has not come back"
    /// are two different things to say in the popup.
    pub done: bool,
    pub error: Option<Fault>,
    /// Whatever the call streamed while it ran. The only return value on the
    /// wire for a call that worked, and only for the tools that tap their own
    /// stdout.
    pub output: Vec<String>,
    /// Whether this call was in flight at the same time as another one inside
    /// the same turn.
    pub parallel: bool,
    /// The absolute activity line its one row was written to.
    pub line: usize,
}

/// One cell of the popup: a heading and the lines under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub label: &'static str,
    pub lines: Vec<String>,
    /// The lines are this window admitting it does not have the value, rather
    /// than the value. A blank cell would read as "the tool returned nothing",
    /// which is a different claim and usually a false one.
    pub absent: bool,
    pub tone: Tone,
}

impl Call {
    /// Whether an activity row belongs to this call: a call is one row.
    fn owns(&self, line: usize) -> bool {
        line == self.line
    }

    /// How long it ran, in whatever the window can honestly say.
    fn duration(&self) -> Option<f64> {
        match (self.started, self.ended) {
            (Some(start), Some(end)) if end >= start => Some(end - start),
            _ => None,
        }
    }

    /// The one line at the top of the popup: the tool, the thing it reached for,
    /// and whether it worked.
    pub fn heading(&self) -> String {
        let outcome = match (&self.error, self.done) {
            (Some(_), _) => "failed",
            (None, true) => "returned",
            (None, false) => "running",
        };
        format!("{} \u{25b8} {} \u{25b8} {outcome}", self.kind.tag(), self.target)
    }

    /// The popup's blocks, top to bottom: what was invoked, when, what the
    /// model generated, what came back, and - for a failure - the detail.
    ///
    /// Only the blocks with something to say: a filler block ("no detail")
    /// under every working call taught nothing, and a failure's message
    /// printed by three blocks at once read as three different failures.
    /// Every line appears exactly once, in the block it belongs to.
    pub fn cells(&self) -> Vec<Cell> {
        let mut out = vec![self.invoked_cell(), self.when_cell()];
        if let Some(generated) = self.generated_cell() {
            out.push(generated);
        }
        out.push(self.returned_cell());
        if let Some(detail) = self.detail_cell() {
            out.push(detail);
        }
        let mut seen: Vec<String> = Vec::new();
        for cell in &mut out {
            cell.lines.retain(|line| {
                let key = line.trim().to_string();
                let fresh = key.is_empty() || !seen.contains(&key);
                if fresh && !key.is_empty() {
                    seen.push(key);
                }
                fresh
            });
        }
        out.retain(|cell| !cell.lines.is_empty());
        out
    }

    fn invoked_cell(&self) -> Cell {
        let mut lines = vec![self.target.clone()];
        if !self.brief.trim().is_empty() && self.brief != self.target {
            lines.push(self.brief.clone());
        }
        // The skill's own file is computed agent side and written into the
        // result body, which reaches this window only when the call fails.
        if self.kind == Kind::Skill {
            lines.push(String::from(
                "the skill's own file is not on the wire: only its name is sent",
            ));
        }
        Cell {
            label: "INVOKED",
            lines,
            absent: false,
            tone: Tone::Bright,
        }
    }

    fn when_cell(&self) -> Cell {
        let mut lines = vec![format!("turn {}", self.turn)];
        if self.parallel {
            lines.push(String::from("ran beside another call in this turn"));
        }
        match self.duration() {
            Some(seconds) => lines.push(format!("{seconds:.2}s in this window")),
            None => lines.push(String::from("no arrival time was taken")),
        }
        if let Some(ms) = self.elapsed_ms {
            lines.push(format!("{ms} ms by the agent's own clock"));
        }
        Cell {
            label: "WHEN",
            lines,
            absent: false,
            tone: Tone::Dim,
        }
    }

    /// The arguments the model wrote, or nothing when it sent none: a block
    /// explaining its own absence taught nothing.
    fn generated_cell(&self) -> Option<Cell> {
        let empty = matches!(&self.args, noob_proto::Value::Null)
            || self.args.as_object().is_some_and(|o| o.is_empty());
        if empty {
            return None;
        }
        let text = serde_json::to_string_pretty(&self.args).unwrap_or_else(|_| self.args.to_string());
        Some(Cell {
            label: "GENERATED",
            lines: clipped(&text),
            absent: false,
            tone: Tone::Body,
        })
    }

    fn returned_cell(&self) -> Cell {
        let mut lines = Vec::new();
        if !self.summary.trim().is_empty() {
            lines.push(self.summary.clone());
        }
        let (rest, absent, tone) = match (&self.error, self.done) {
            // The failure's own story is the DETAIL block; this one keeps
            // only the summary and whatever the call printed before it died.
            (Some(_), _) => (self.output.clone(), false, Tone::Dim),
            (None, false) if !self.output.is_empty() => (self.output.clone(), false, Tone::Body),
            (None, false) => (vec![String::from("still running")], true, Tone::Dim),
            (None, true) if !self.output.is_empty() => (self.output.clone(), false, Tone::Body),
            // The wire carries a display summary and no result body for a call
            // that worked. Saying so is the whole of what this window can do
            // about it without a protocol change.
            (None, true) => (
                vec![String::from(
                    "the return value is not on the wire: a call that worked sends this summary and nothing more",
                )],
                true,
                Tone::Dim,
            ),
        };
        lines.extend(rest);
        Cell {
            label: "RETURNED",
            lines,
            absent,
            tone,
        }
    }

    /// The failure, whole and in one place: its class, its message, the body
    /// the tool sent, and what to do next. Nothing for a call that worked.
    fn detail_cell(&self) -> Option<Cell> {
        let error = self.error.as_ref()?;
        let mut lines = vec![error.fault.clone(), error.message.clone()];
        if let Some(detail) = error.detail.as_deref() {
            lines.extend(clipped(detail));
        }
        if let Some(remedy) = error.remedy.as_deref() {
            lines.push(format!("-> {remedy}"));
        }
        Some(Cell {
            label: "DETAIL",
            lines,
            absent: false,
            tone: Tone::Bad,
        })
    }
}

/// A block of text as lines, bounded, with a last line saying what was left out.
fn clipped(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = text.lines().take(CELL_LINES).map(str::to_string).collect();
    if text.lines().nth(CELL_LINES).is_some() {
        lines.push(String::from("\u{2026}"));
    }
    lines
}

pub struct State {
    pub session: String,
    pub model: String,
    pub workspace: String,
    pub resumed: bool,

    pub output: Pane,
    pub activity: Pane,
    /// Prompts submitted while a turn was running, waiting behind it in
    /// dispatch order. Not part of the transcript yet: each one echoes as a
    /// plain `› message` record at the `turn.start` that takes it, and until
    /// then the OUTPUT pane pins it to its bottom rows as a dim `[queued]`
    /// line that never scrolls away.
    pub queued: Vec<String>,
    pub plan: Vec<Todo>,
    /// The live fleet: queued and running children only. A child that
    /// finishes leaves the list in the same event that ends it; its report
    /// reaches the conversation through the parent's own turn.
    pub agents: Vec<AgentRow>,
    /// Children spawned this session, which is where an ordinal comes from:
    /// rows leave the list, so its length cannot number anything.
    agents_spawned: usize,
    /// The agent the `[N] AGENT - OUTPUT` tab is on, by ordinal. None when
    /// none was clicked, or when the one that was has finished; the shell
    /// hides the tab when this goes.
    pub shown_agent: Option<usize>,
    pub files: Vec<FileView>,
    pub open_file: usize,

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
    /// How many of those came back with an error. A count and nothing else:
    /// the pane that kept the failures themselves is gone, and what is left is
    /// the one figure that says whether this run is going well, read beside the
    /// total in the CONTEXT pane.
    ///
    /// Counted rather than measured off a list, so it cannot saturate: the list
    /// it replaces was bounded at a hundred and a run that failed more than
    /// that under-reported.
    pub failed_calls: u32,
    pub rates: Rates,

    /// The agent's own reading of how full it is. None until it says.
    pub context: Option<ContextFill>,


    pub turn: u32,
    pub phase: Phase,
    /// What is happening right now, in a few words, for the status bar.
    pub status: String,

    /// One record per tool call, oldest first, bounded at [`MAX_CALLS`]. The
    /// list beside the activity pane rather than inside it: the pane is text and
    /// stays text, and this is what a row of it can be clicked back into.
    pub calls: Vec<Call>,
    /// How many records have fallen off the front over the session's whole life,
    /// so a call can be named by an ordinal that never moves. Same reason
    /// [`Pane::dropped`] exists.
    calls_dropped: usize,
    /// Which call the popup is showing, by ordinal. `None` with the popup shut,
    /// and a call that has since been evicted simply stops resolving.
    pub open_call: Option<usize>,

    /// Where this window's own zero sits on the reader's clock, as seconds past
    /// local midnight. Every activity row is stamped with it plus the monotonic
    /// second its frame arrived at, which is the same reading the call record
    /// keeps for the popup rather than a second clock.
    ///
    /// `None` until someone says, which is every test that folds a frame in
    /// without a clock, and those rows carry no reading at all.
    pub day_zero: Option<u64>,

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
            // The one pane the model writes Markdown into, so the one pane that
            // is measured, banded and copied in what it draws.
            output: Pane::new(6000).rendered(),
            activity: Pane::new(4000).clipped(),
            queued: Vec::new(),
            plan: Vec::new(),
            agents: Vec::new(),
            agents_spawned: 0,
            shown_agent: None,
            files: Vec::new(),
            open_file: 0,
            usage: None,
            prefilled: 0,
            generated: 0,
            cached_prefill: 0,
            requests: 0,
            last_prefill: 0,
            last_generated: 0,
            best_generated: 0,
            tool_calls: 0,
            failed_calls: 0,
            rates: Rates::default(),
            context: None,
            turn: 0,
            phase: Phase::Starting,
            status: String::from("starting the agent"),
            calls: Vec::new(),
            calls_dropped: 0,
            open_call: None,
            day_zero: None,
            open: HashMap::new(),
        }
    }

    /// One call by its ordinal, or nothing when it has been evicted.
    pub fn call(&self, ordinal: usize) -> Option<&Call> {
        self.calls.get(ordinal.checked_sub(self.calls_dropped)?)
    }

    /// The call the popup is showing, if it is up and still held.
    pub fn popped(&self) -> Option<&Call> {
        self.call(self.open_call?)
    }

    /// Which call wrote an activity row, by that row's absolute number.
    ///
    /// Newest first, because a bounded list can hold two calls that once wrote
    /// the same row number only if one of them has been evicted, and the newer
    /// answer is the one still on screen.
    pub fn call_at_line(&self, line: usize) -> Option<usize> {
        self.calls
            .iter()
            .rposition(|call| call.owns(line))
            .map(|index| self.calls_dropped + index)
    }

    /// The clock reading to write in front of a row, and the two blanks that
    /// hold it off the tag. Empty when the frame was untimed or when nobody has
    /// said what time it is here, so an untimed session's rows are exactly the
    /// rows they always were.
    fn stamp(&self, at: Option<f64>) -> String {
        let (Some(zero), Some(at)) = (self.day_zero, at) else {
            return String::new();
        };
        format!("{}  ", clock(zero + at.max(0.0) as u64))
    }

    /// Write one row of the activity list, under the clock reading of the frame
    /// that caused it.
    ///
    /// The stamp goes in the row's own text rather than beside it because every
    /// row count, scroll window and selection column in that pane is measured
    /// off the stored line, the same reason the parallel mark is written in.
    /// The drawing gives it the dim tone off the gutter width.
    fn note(&mut self, stamp: &str, text: impl Into<String>, tone: Tone) {
        let gutter = stamp.chars().count();
        let line = format!("{stamp}{}", text.into());
        self.activity.push(Line::new(line, tone).with_gutter(gutter));
    }

    /// A line the window says in the activity list on its own behalf, timed the
    /// same way the frames around it are.
    ///
    /// The clipboard failing and a setting nobody recognizes are not calls and
    /// have no frame behind them, but they land in the same list, and a row
    /// with no reading in a column of readings looks like a row the clock
    /// missed rather than a row that never had one.
    pub fn noted(&mut self, at: Option<f64>, text: impl Into<String>, tone: Tone) {
        let stamp = self.stamp(at);
        self.note(&stamp, text, tone);
    }

    /// Keep a record, dropping the oldest once the list is full. Returns its
    /// ordinal.
    fn remember_call(&mut self, call: Call) -> usize {
        self.calls.push(call);
        while self.calls.len() > MAX_CALLS {
            self.calls.remove(0);
            self.calls_dropped += 1;
        }
        self.calls_dropped + self.calls.len() - 1
    }

    /// Say that a call ran beside another one, and put the mark in the gutter of
    /// the row it wrote.
    ///
    /// The mark is written into the text rather than drawn beside it because
    /// every row count, scroll window and selection column in that pane is
    /// measured off the stored line: a run added at draw time would put the
    /// drawn rows and the measured ones a character apart. It replaces a space
    /// with a bar, so the line is the length it always was.
    fn mark_parallel(&mut self, ordinal: usize) {
        let Some(index) = ordinal.checked_sub(self.calls_dropped) else {
            return;
        };
        let Some(call) = self.calls.get_mut(index) else {
            return;
        };
        if call.parallel {
            return;
        }
        call.parallel = true;
        let line = call.line;
        // Past the clock column, so the mark keeps standing in the blank
        // between the tag and the subject on a row that carries a reading.
        let column = PARALLEL_COLUMN + self.activity.line(line).map_or(0, |row| row.gutter);
        self.activity.mark(line, column, PARALLEL_MARK);
    }

    /// What the human typed, echoed into the transcript so the conversation
    /// reads as a conversation.
    pub fn submitted(&mut self, text: &str) {
        self.echo(text);
        self.phase = Phase::Thinking;
        self.status = String::from("thinking");
    }

    /// A prompt typed while the agent already had the turn. It waits in
    /// [`State::queued`] rather than entering the transcript: echoed at
    /// acceptance it would read as answered by whatever the running turn was
    /// already saying. The phase stays where it is, because nothing new is
    /// being thought about yet.
    pub fn enqueue(&mut self, text: &str) {
        self.queued.push(text.to_string());
    }

    /// How many of a panel's rows the queued messages pin to its bottom. At
    /// least one row always stays with the transcript, so a queue can crowd
    /// the conversation but never replace it.
    pub fn output_reserved(&self, rows: usize) -> usize {
        self.queued.len().min(rows.saturating_sub(1))
    }

    /// The `› message` record both submission paths write.
    fn echo(&mut self, text: &str) {
        self.output.blank_if_needed();
        self.output.say(format!("› {text}"), Tone::Bright);
        self.output.push(Line::new("", Tone::Body));
    }

    /// A /command the window ran itself, echoed the same way but never sent:
    /// no turn starts, so the phase does not move. The answer follows as the
    /// window's own lines.
    pub fn commanded(&mut self, text: &str) {
        self.output.blank_if_needed();
        self.output.say(format!("› {text}"), Tone::Bright);
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
                // Nothing is left to dispatch these; a [queued] row outliving
                // the agent would promise a turn that can never come.
                self.queued.clear();
            }
            Event::TurnStart { turn } => {
                self.turn = turn;
                // A turn that starts while messages wait is the front one's
                // turn: its pinned [queued] row becomes the plain `› message`
                // record, and the tag goes with it. Echoed here rather than
                // at acceptance, so a message can never read as answered by a
                // turn that never saw it.
                if !self.queued.is_empty() {
                    let text = self.queued.remove(0);
                    self.echo(&text);
                }
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
                // after the turn that owned it has ended. The row goes to the
                // failure colour and the reason waits on its popup, the same
                // shape every other ending has.
                let stragglers: Vec<usize> =
                    self.open.drain().map(|(_, open)| open.call).collect();
                for ordinal in stragglers {
                    let mut row = None;
                    if let Some(index) = ordinal.checked_sub(self.calls_dropped)
                        && let Some(call) = self.calls.get_mut(index)
                    {
                        // Ended, because the turn that owned it has: leaving it
                        // reading as still running is the one thing that is not
                        // true about it.
                        call.ended = at;
                        call.done = true;
                        call.summary = String::from("never reported back");
                        row = Some(call.line);
                    }
                    if let Some(row) = row {
                        self.activity.retone(row, Tone::Bad);
                    }
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
                // Taken before the push, so it is the number of the row about to
                // be written rather than the one after it.
                let line = self.activity.last();
                let stamp = self.stamp(at);
                // One row per call, ending in the chevron that says a row
                // opens: everything else about the call lives on the popup a
                // press on this row raises.
                self.note(
                    &stamp,
                    format!("{:>5}  {} \u{25b8}", kind.tag(), subject(kind, &brief, &args)),
                    Tone::Call(kind),
                );
                let ordinal = self.remember_call(Call {
                    call_id: call_id.clone(),
                    kind,
                    target: invoked(kind, &name, &args),
                    name,
                    brief: brief.clone(),
                    args,
                    turn: self.turn,
                    started: at,
                    ended: None,
                    elapsed_ms: None,
                    summary: String::new(),
                    done: false,
                    error: None,
                    output: Vec::new(),
                    parallel: false,
                    line,
                });
                // Every call still open started before this one and has not come
                // back, so this one and all of them were in flight together. The
                // turn takes care of itself: a turn that ends drains `open`, so
                // nothing here can be paired with a call from an earlier one.
                //
                // This is the whole of what the protocol allows. There is no
                // batch id, no ordinal and no agent-side timestamp on a tool
                // frame, so "these ran together" can only be read off what was
                // still open when a start frame arrived.
                if !self.open.is_empty() {
                    let together: Vec<usize> =
                        self.open.values().map(|open| open.call).chain([ordinal]).collect();
                    for other in together {
                        self.mark_parallel(other);
                    }
                }
                self.open.insert(call_id, Open { call: ordinal });
            }
            Event::ToolProgress { call_id, line } => {
                // The live stdout tap is the call's own news, never the
                // pane's: a websearch or a build printing every step buried
                // the list of calls under one call's chatter. It accumulates
                // on the popup's RETURNED block, bounded, and the status
                // line carries the latest of it while the call runs. A line
                // for a call this window never saw open changes nothing.
                let Some(ordinal) = self.open.get(&call_id).map(|o| o.call) else {
                    return false;
                };
                if let Some(index) = ordinal.checked_sub(self.calls_dropped)
                    && let Some(call) = self.calls.get_mut(index)
                {
                    if call.output.len() < CELL_LINES {
                        call.output.push(line.clone());
                    }
                    self.status = format!("{} {}", call.name, line);
                }
            }
            Event::ToolEnd {
                call_id,
                summary,
                elapsed_ms,
                error,
            } => {
                let ordinal = self.open.remove(&call_id).map(|open| open.call);
                // Counted whether or not this window saw the call start: an
                // end frame with an error on it is a call that failed either
                // way.
                if error.is_some() {
                    self.failed_calls += 1;
                }
                // How the call ended writes no rows: the summary, the fault
                // and the remedy are the popup's RETURNED and DETAIL blocks,
                // behind the one row the call already has. A failure recolors
                // that row, so the list still says at a glance which call to
                // press.
                let failed = error.map(|error| Fault {
                    fault: fault(&error),
                    message: error.message,
                    detail: error.detail,
                    remedy: error.remedy,
                });
                let mut failed_row = None;
                if let Some(index) = ordinal.and_then(|o| o.checked_sub(self.calls_dropped))
                    && let Some(call) = self.calls.get_mut(index)
                {
                    call.ended = at;
                    call.done = true;
                    call.elapsed_ms = Some(elapsed_ms);
                    call.summary = summary;
                    call.error = failed;
                    if call.error.is_some() {
                        failed_row = Some(call.line);
                    }
                }
                if let Some(row) = failed_row {
                    self.activity.retone(row, Tone::Bad);
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
            } => {
                self.agents_spawned += 1;
                self.agents
                    .push(AgentRow::new(agent_id, self.agents_spawned, prompt, tools));
            }
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
                // A finished child leaves the list in the same event that
                // ends it: the pane shows the live fleet, and the report
                // lands in the conversation through the parent's own turn.
                // Its output tab goes with it, having nothing left to watch.
                if matches!(
                    state,
                    noob_proto::AgentState::Done
                        | noob_proto::AgentState::Failed
                        | noob_proto::AgentState::Canceled
                ) {
                    if let Some(at) = self.agents.iter().position(|a| a.label == agent_id) {
                        if self.shown_agent == Some(self.agents[at].ordinal) {
                            self.shown_agent = None;
                        }
                        self.agents.remove(at);
                    }
                } else if let Some(agent) =
                    self.agents.iter_mut().find(|a| a.label == agent_id)
                {
                    agent.state = word;
                    agent.tone = tone;
                    if let Some(detail) = detail {
                        agent.last = detail;
                    }
                }
            }
            // A child's own output belongs to the child, not to the parent's
            // activity list: eight of them at once made that list unreadable
            // and buried the parent's own work under it. The row keeps the
            // last line for the list; the child's pane keeps all of them for
            // its output tab.
            Event::AgentOutput { agent_id, line } => {
                match self.agents.iter_mut().find(|a| a.label == agent_id) {
                    Some(agent) => {
                        agent.pane.say(line.clone(), Tone::Body);
                        agent.last = line;
                    }
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
            crate::dock::View::Agent => self.agent_shown().map(|agent| &agent.pane),
            _ => None,
        }
    }

    /// The agent the output tab is on, while it is still running.
    pub fn agent_shown(&self) -> Option<&AgentRow> {
        let ordinal = self.shown_agent?;
        self.agents.iter().find(|agent| agent.ordinal == ordinal)
    }

    /// The same row writable, for the wheel.
    pub fn agent_shown_mut(&mut self) -> Option<&mut AgentRow> {
        let ordinal = self.shown_agent?;
        self.agents.iter_mut().find(|agent| agent.ordinal == ordinal)
    }

    /// Point the output tab at one agent by its ordinal. Says whether
    /// anything changed, so a click on the row already showing costs no
    /// frame.
    pub fn show_agent(&mut self, ordinal: usize) -> bool {
        if !self.agents.iter().any(|agent| agent.ordinal == ordinal) {
            return false;
        }
        if self.shown_agent == Some(ordinal) {
            return false;
        }
        self.shown_agent = Some(ordinal);
        true
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
        true
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

/// The particular thing a call reached for, as against the tool that reached.
///
/// `bash` is only ever bash, so the tool's own name is the whole answer for most
/// of them. A skill call names a skill and an MCP call names a server and a tool
/// on it, and those two are the names worth reading rather than the word `skill`
/// nine times down a list. The skill's own file is not derivable here: it is
/// computed agent side and written into the result body, which reaches this
/// window only when the call fails.
fn invoked(kind: Kind, name: &str, args: &noob_proto::Value) -> String {
    let field = |key: &str| {
        args.get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
    };
    match kind {
        Kind::Skill => match field("name") {
            Some(skill) => format!("skill {skill}"),
            None => name.to_string(),
        },
        Kind::Mcp => match (field("server"), field("tool")) {
            (Some(server), Some(tool)) => format!("{server} \u{25b8} {tool}"),
            (Some(server), None) => server.to_string(),
            _ => name.to_string(),
        },
        _ => name.to_string(),
    }
}

const SUBJECT_CHARS: usize = 96;
const DIFF_LINES: usize = 60;
const MAX_FILES: usize = 40;
/// How many calls the window keeps the whole record of. Deep enough that a
/// failure is still there to be clicked several minutes after it scrolled past,
/// and bounded because a long session makes thousands of them.
const MAX_CALLS: usize = 300;
/// How many lines of any one popup cell are kept and shown. A build log is
/// thousands of lines and the popup is a box on a window.
const CELL_LINES: usize = 40;
/// Where in an activity row the parallel mark goes: the second of the two spaces
/// between the tag column and the subject, so the mark stands in its own column
/// down the whole list and every row is exactly as wide as it was.
const PARALLEL_COLUMN: usize = 6;
const PARALLEL_MARK: char = '|';

/// A clock reading, from seconds past midnight.
///
/// `HH:MM:SS` because a row of this list is compared against two things: the
/// row above it, which is often the same minute, and the clock the reader has
/// in front of him, which is why it is a time of day and not an age.
pub fn clock(day_second: u64) -> String {
    let second = day_second % 86_400;
    format!(
        "{:02}:{:02}:{:02}",
        second / 3600,
        (second % 3600) / 60,
        second % 60
    )
}

/// Seconds past midnight, read back off an `HH:MM:SS` reading.
///
/// `None` for anything else. The window is told what time it is by asking, and
/// an answer it cannot read leaves the rows with no clock at all rather than
/// with a reading nobody can trust.
pub fn day_second(reading: &str) -> Option<u64> {
    let fields: Vec<&str> = reading.trim().split(':').collect();
    let [hours, minutes, seconds] = fields[..] else {
        return None;
    };
    let value = |field: &str, limit: u64| -> Option<u64> {
        let digits = field.len() == 2 && field.bytes().all(|b| b.is_ascii_digit());
        digits.then(|| field.parse().ok()).flatten().filter(|n| *n < limit)
    };
    // A leap second is minute 59 second 60, and a clock that reports one is
    // right rather than broken.
    Some(value(hours, 24)? * 3600 + value(minutes, 60)? * 60 + value(seconds, 61)?)
}

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

    /// A transcript's thumb drags between its ends: 0.0 is the oldest row
    /// still held, 1.0 the live end it rests at.
    #[test]
    fn the_transcript_thumb_drags_between_its_ends() {
        let mut pane = Pane::new(100);
        for n in 0..30 {
            pane.say(format!("line {n}"), Tone::Body);
        }
        assert!(pane.scroll_to(0.0, 10, 80));
        assert_eq!(pane.window(10, 80).first, 0, "the top of the scrollback");
        assert!(pane.scroll_to(1.0, 10, 80));
        assert_eq!(pane.window(10, 80).first, 20, "back at the live end");
        assert!(!pane.scroll_to(1.0, 10, 80), "already there");
    }

    /// A prompt queued mid-turn stays out of the transcript until the turn
    /// that takes it starts, then echoes exactly once, in dispatch order.
    #[test]
    fn a_queued_prompt_echoes_at_the_turn_that_takes_it() {
        let mut state = State::new();
        state.submitted("first");
        state.apply(Event::TurnStart { turn: 1 });
        state.enqueue("second");
        assert_eq!(state.queued, vec!["second"]);
        assert!(
            !texts(&state.output).iter().any(|l| l.contains("second")),
            "queued must not enter the transcript at acceptance"
        );
        state.apply(Event::TurnEnd {
            turn: 1,
            interrupted: None,
        });
        state.apply(Event::TurnStart { turn: 2 });
        assert!(state.queued.is_empty());
        let echoes = texts(&state.output)
            .iter()
            .filter(|l| l.contains("› second"))
            .count();
        assert_eq!(echoes, 1, "one dispatch, one record");
    }

    /// The queue dies with the session: nothing is left to dispatch it, and a
    /// [queued] row outliving the agent would promise a turn that never comes.
    #[test]
    fn the_queue_dies_with_the_session() {
        let mut state = State::new();
        state.apply(Event::TurnStart { turn: 1 });
        state.enqueue("waiting");
        state.apply(Event::SessionEnd { id: String::new() });
        assert!(state.queued.is_empty());
    }

    /// The queue can crowd the transcript but never replace it: at least one
    /// row of the panel always stays with the conversation.
    #[test]
    fn the_reservation_leaves_the_transcript_a_row() {
        let mut state = State::new();
        for n in 0..10 {
            state.enqueue(&format!("m{n}"));
        }
        assert_eq!(state.output_reserved(30), 10);
        assert_eq!(state.output_reserved(5), 4);
        assert_eq!(state.output_reserved(0), 0);
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
        assert!(line.chars().count() <= SUBJECT_CHARS + 10, "{line}");
        assert!(
            line.ends_with("calc.py \u{25b8}"),
            "the end is what identifies it, then the press chevron: {line}"
        );
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
        // Every kind is distinguishable without color, too. The five were
        // started back to back and not one of them came back, so all five carry
        // the parallel mark in the column between the tag and the subject: see
        // `calls_that_overlap_are_marked_and_calls_that_do_not_are_not`.
        assert!(lines[0].text.contains("bash |rm -rf build"), "{:?}", lines[0]);
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

        // Its whole output is kept on the row's own pane for the output tab,
        // not only the last line.
        assert_eq!(state.agents[0].pane.visible(10, 100).len(), 1);

        // Finishing takes the row out of the list: the pane shows the live
        // fleet, and the child's report reaches the conversation through the
        // parent's own turn.
        state.apply(Event::AgentStateChanged {
            agent_id: "agent-1".into(),
            state: noob_proto::AgentState::Failed,
            detail: Some("needs the websearch CLI on PATH".into()),
        });
        assert!(state.agents.is_empty(), "{:?}", state.agents);
    }

    /// Clicking a row points the output tab at that agent; the agent
    /// finishing takes the choice away with the row, so the shell knows to
    /// close the tab rather than leave it over nothing.
    #[test]
    fn the_shown_agent_dies_with_its_row() {
        let mut state = State::new();
        for n in 1..=2 {
            state.apply(Event::AgentSpawn {
                agent_id: format!("agent-{n}"),
                prompt: format!("task {n}"),
                tools: "read".into(),
            });
        }
        assert_eq!(state.agents[1].ordinal, 2, "ordinals are spawn order");
        assert!(state.show_agent(2));
        assert!(!state.show_agent(2), "already showing");
        assert!(!state.show_agent(99), "no such agent");
        assert_eq!(state.agent_shown().map(|agent| agent.ordinal), Some(2));
        state.apply(Event::AgentStateChanged {
            agent_id: "agent-2".into(),
            state: noob_proto::AgentState::Done,
            detail: None,
        });
        assert_eq!(state.shown_agent, None);
        assert!(state.agent_shown().is_none());
        // The other child kept its row and its stable number.
        assert_eq!(state.agents.len(), 1);
        assert_eq!(state.agents[0].ordinal, 1);
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
        // A canceled child resolves by leaving; the others keep their rows.
        assert_eq!(state.agents.len(), 2);
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

    /// A failure keeps the pane to the call's one row and recolors it; the
    /// class, the number the system gave it and what to do next are the
    /// popup's DETAIL block, behind the row a press opens.
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
        let rows = state.activity.visible(200, 200);
        assert_eq!(rows.len(), 1, "one call, one row: {:?}", texts(&state.activity));
        assert_eq!(rows[0].tone, Tone::Bad, "the row itself says it failed");
        let call = state.call(0).expect("the record");
        let detail = call
            .cells()
            .into_iter()
            .find(|cell| cell.label == "DETAIL")
            .expect("the detail block");
        assert!(detail.lines.contains(&String::from("exit_status 127")), "{detail:?}");
        assert!(
            detail.lines.contains(&String::from("-> available here: python3 node")),
            "{detail:?}"
        );
    }

    /// Two calls in flight together are marked as such; two that took turns are
    /// not.
    ///
    /// There is no field on the wire that says "these ran in parallel": no batch
    /// id, no ordinal, no agent-side timestamp. What there is, is the set of
    /// calls still open when a start frame arrives, and that set is exact. So
    /// the rule under test is the whole of what the window can honestly claim.
    #[test]
    fn calls_that_overlap_are_marked_and_calls_that_do_not_are_not() {
        let end = |id: &str| Event::ToolEnd {
            call_id: id.into(),
            summary: format!("{id} done"),
            elapsed_ms: 5,
            error: None,
        };

        // A fan-out: two starts before either end.
        let mut fanned = State::new();
        fanned.apply(Event::TurnStart { turn: 1 });
        fanned.apply(tool_start("a", "read", serde_json::json!({"path": "a.rs"})));
        fanned.apply(tool_start("b", "read", serde_json::json!({"path": "b.rs"})));
        fanned.apply(end("a"));
        fanned.apply(end("b"));
        assert!(fanned.calls.iter().all(|call| call.parallel), "{:?}", fanned.calls);
        let marked: Vec<String> = texts(&fanned.activity)
            .into_iter()
            .filter(|line| line.contains('|'))
            .collect();
        assert_eq!(marked.len(), 2, "both rows carry the mark: {marked:?}");
        assert!(marked[0].contains("read |brief"), "{marked:?}");

        // One after the other, inside the same turn. Nothing overlapped.
        let mut queued = State::new();
        queued.apply(Event::TurnStart { turn: 1 });
        queued.apply(tool_start("a", "read", serde_json::json!({"path": "a.rs"})));
        queued.apply(end("a"));
        queued.apply(tool_start("b", "read", serde_json::json!({"path": "b.rs"})));
        queued.apply(end("b"));
        assert!(queued.calls.iter().all(|call| !call.parallel), "{:?}", queued.calls);
        assert!(
            !texts(&queued.activity).iter().any(|line| line.contains('|')),
            "{:?}",
            texts(&queued.activity)
        );

        // And a call left open by one turn cannot pair with the next turn's.
        let mut across = State::new();
        across.apply(Event::TurnStart { turn: 1 });
        across.apply(tool_start("a", "read", serde_json::json!({"path": "a.rs"})));
        across.apply(Event::TurnEnd {
            turn: 1,
            interrupted: None,
        });
        across.apply(Event::TurnStart { turn: 2 });
        across.apply(tool_start("b", "read", serde_json::json!({"path": "b.rs"})));
        assert!(across.calls.iter().all(|call| !call.parallel), "{:?}", across.calls);
    }

    /// The mark is written into the row rather than drawn beside it, so a marked
    /// row is exactly as long as an unmarked one.
    ///
    /// Every row count, scroll window and selection column in that pane is
    /// measured off the stored line. A mark that made a row one character
    /// longer would put the rows that are drawn and the rows that are measured
    /// out of step for the rest of the session.
    #[test]
    fn the_parallel_mark_costs_the_row_no_width() {
        let mut plain = State::new();
        plain.apply(tool_start("a", "read", serde_json::json!({"path": "a.rs"})));
        plain.apply(Event::ToolEnd {
            call_id: "a".into(),
            summary: "read 3 lines".into(),
            elapsed_ms: 1,
            error: None,
        });

        let mut fanned = State::new();
        fanned.apply(tool_start("a", "read", serde_json::json!({"path": "a.rs"})));
        fanned.apply(tool_start("b", "read", serde_json::json!({"path": "b.rs"})));

        let (before, after) = (&texts(&plain.activity)[0], &texts(&fanned.activity)[0]);
        assert_ne!(before, after, "the mark is there");
        assert_eq!(before.chars().count(), after.chars().count());
    }

    /// A row of the list knows which call wrote it, and so do the rows that call
    /// wrote when it ended.
    #[test]
    fn every_row_a_call_wrote_resolves_back_to_that_call() {
        let mut state = State::new();
        state.apply(tool_start("a", "read", serde_json::json!({"path": "a.rs"})));
        state.apply(Event::ToolEnd {
            call_id: "a".into(),
            summary: "read 3 lines".into(),
            elapsed_ms: 1,
            error: None,
        });
        state.apply(tool_start("b", "bash", serde_json::json!({"cmd": "ls"})));

        let first = state.call_at_line(0).expect("the first row is the read");
        assert_eq!(state.call(first).expect("held").call_id, "a");
        // A call is one row: its ending wrote no more of them.
        let second = state.call_at_line(1).expect("the second row is the bash");
        assert_eq!(state.call(second).expect("held").call_id, "b");
        assert_eq!(state.call_at_line(99), None, "a row nobody wrote");
    }

    /// A row of the list says when it happened, and still says what it is.
    ///
    /// The popup was given the timing and the row it opens from was left as a
    /// tag and a subject, which is item 42: the list is what is read, and a
    /// list of calls with no times on it cannot be lined up against anything.
    #[test]
    fn an_activity_row_carries_the_time_its_call_happened() {
        let mut state = State::new();
        // Half past two in the afternoon, on the reader's own clock.
        state.day_zero = Some(14 * 3600 + 30 * 60);
        state.apply_at(
            tool_start("a", "bash", serde_json::json!({"cmd": "cargo test"})),
            Some(7.0),
        );
        let row = &texts(&state.activity)[0];
        assert!(row.starts_with("14:30:07  "), "{row}");
        assert!(row.contains("bash"), "the tag survives: {row}");
        assert!(row.contains("cargo test"), "the subject survives: {row}");
        // The reading comes off the same arrival the record kept for the popup,
        // so the row and the popup cannot disagree about one call.
        assert_eq!(state.call(0).expect("kept").started, Some(7.0));
        // And the clock column is the row's dim part, not the call's color.
        let line = state.activity.line(0).expect("held");
        assert_eq!(line.tone, Tone::Call(Kind::Bash));
        assert_eq!(line.split_gutter().0, "14:30:07  ");
        assert!(line.split_gutter().1.starts_with(" bash"), "{line:?}");
    }

    /// Two calls a minute apart are two different readings on their own rows.
    #[test]
    fn two_calls_a_minute_apart_carry_two_different_times() {
        let mut state = State::new();
        state.day_zero = Some(9 * 3600);
        state.apply_at(
            tool_start("a", "read", serde_json::json!({"path": "a.rs"})),
            Some(0.0),
        );
        state.apply_at(
            Event::ToolEnd {
                call_id: "a".into(),
                summary: "read 3 lines".into(),
                elapsed_ms: 2,
                error: None,
            },
            Some(4.0),
        );
        state.apply_at(
            tool_start("b", "read", serde_json::json!({"path": "b.rs"})),
            Some(61.0),
        );
        let rows = texts(&state.activity);
        assert!(rows[0].starts_with("09:00:00  "), "{rows:?}");
        assert!(rows[1].starts_with("09:01:01  "), "{rows:?}");
        assert_ne!(&rows[0][..8], &rows[1][..8], "a minute apart, two clocks");
    }

    /// Progress belongs to its call, never to the pane: a running call's
    /// lines accumulate on its record for the popup, and progress for a call
    /// this window never saw open is dropped rather than invented a row.
    #[test]
    fn progress_lands_on_the_call_and_never_on_the_pane() {
        let mut state = State::new();
        state.day_zero = Some(60);
        assert!(!state.apply_at(
            Event::ToolProgress {
                call_id: "never opened".into(),
                line: "compiling no0b".into(),
            },
            Some(5.0),
        ));
        assert_eq!(state.activity.visible(200, 200).len(), 0, "no row was invented");

        // A note the window says for itself has no call behind it and is
        // stamped all the same, so the column runs down the whole list.
        state.noted(Some(70.0), "could not reach the clipboard", Tone::Bad);
        let line = state.activity.line(0).expect("held");
        assert_eq!(
            line.split_gutter(),
            ("00:02:10  ", "could not reach the clipboard")
        );

        // And anything pushed straight into the pane keeps the shape it was
        // given, clock column and all, which is none.
        state.activity.say("older than the clock", Tone::Dim);
        let line = state.activity.line(1).expect("held");
        assert_eq!(line.gutter, 0);
        assert_eq!(line.split_gutter(), ("", "older than the clock"));
    }

    /// The parallel mark stands in the blank between the tag and the subject,
    /// and the clock column in front of them moved that blank along.
    #[test]
    fn the_parallel_mark_lands_past_the_clock_column() {
        let mut state = State::new();
        state.day_zero = Some(0);
        state.apply_at(Event::TurnStart { turn: 1 }, Some(0.0));
        state.apply_at(
            tool_start("a", "read", serde_json::json!({"path": "a.rs"})),
            Some(1.0),
        );
        state.apply_at(
            tool_start("b", "read", serde_json::json!({"path": "b.rs"})),
            Some(2.0),
        );
        let rows = texts(&state.activity);
        assert!(rows[0].starts_with("00:00:01   read |brief"), "{rows:?}");
        assert!(rows[1].starts_with("00:00:02   read |brief"), "{rows:?}");
    }

    /// The clock is read off the one reading the window can trust, and anything
    /// else leaves it unset rather than guessed at.
    #[test]
    fn a_clock_reading_is_read_back_or_refused() {
        assert_eq!(day_second("00:00:00"), Some(0));
        assert_eq!(day_second("14:30:07\n"), Some(14 * 3600 + 30 * 60 + 7));
        // A leap second is a clock that is right, not one that is broken.
        assert_eq!(day_second("23:59:60"), Some(86_400));
        assert_eq!(day_second("24:00:00"), None);
        assert_eq!(day_second("9:00:00"), None);
        assert_eq!(day_second("+9:00:00"), None);
        assert_eq!(day_second("nine o'clock"), None);
        assert_eq!(day_second(""), None);
        assert_eq!(clock(14 * 3600 + 30 * 60 + 7), "14:30:07");
        // Past midnight the day starts over rather than reading 25:00:00.
        assert_eq!(clock(86_400 + 61), "00:01:01");
    }

    /// The popup of a failed call carries what the model generated and the body
    /// the toolkit sent back, because that pair is the whole reason to open it.
    #[test]
    fn the_popup_of_a_failed_call_carries_the_arguments_and_the_detail() {
        let mut state = State::new();
        state.apply(tool_start(
            "b",
            "bash",
            serde_json::json!({"cmd": "cargo build", "timeout": 30}),
        ));
        state.apply(Event::ToolEnd {
            call_id: "b".into(),
            summary: "bash cargo build (exit 127)".into(),
            elapsed_ms: 2000,
            error: Some(ToolError {
                kind: "exit_status".into(),
                code: Some(127),
                message: "cargo: command not found".into(),
                detail: Some("error: linker `cc` not found\n  note: install build-essential".into()),
                remedy: Some("available here: python3 node".into()),
            }),
        });
        let call = state.call(0).expect("the call was kept");
        let cells = call.cells();
        let cell = |label: &str| {
            cells
                .iter()
                .find(|cell| cell.label == label)
                .unwrap_or_else(|| panic!("no {label} cell"))
        };

        let generated = cell("GENERATED");
        assert!(!generated.absent);
        let text = generated.lines.join("\n");
        assert!(text.contains("cargo build"), "{text}");
        assert!(text.contains("timeout"), "{text}");

        // The failure's whole story is one block: class, message, the body
        // the tool sent, and what to do next, with nothing said twice.
        let detail = cell("DETAIL");
        assert!(!detail.absent);
        let text = detail.lines.join("\n");
        assert!(text.contains("exit_status 127"), "{text}");
        assert!(text.contains("cargo: command not found"), "{text}");
        assert!(text.contains("linker `cc` not found"), "{text}");
        assert!(text.contains("install build-essential"), "{text}");
        assert!(text.contains("available here: python3 node"), "{text}");
    }

    /// A call that worked has no return value on the wire. The cell says that in
    /// words instead of standing empty, because an empty cell reads as "the tool
    /// returned nothing" and that is a different claim.
    #[test]
    fn a_call_that_worked_says_its_return_is_not_on_the_wire() {
        let mut state = State::new();
        state.apply(tool_start("r", "read", serde_json::json!({"path": "a.rs"})));
        state.apply(Event::ToolEnd {
            call_id: "r".into(),
            summary: "read 40 lines".into(),
            elapsed_ms: 3,
            error: None,
        });
        let cells = state.call(0).expect("kept").cells();
        let returned = cells.iter().find(|c| c.label == "RETURNED").expect("a cell");
        assert!(returned.absent, "the window does not have the value");
        let text = returned.lines.join("\n");
        assert!(text.contains("read 40 lines"), "the summary is still shown: {text}");
        assert!(text.contains("not on the wire"), "{text}");
        assert!(!text.trim().is_empty());

        // No failure, no DETAIL block: a filler cell taught nothing.
        assert!(!cells.iter().any(|c| c.label == "DETAIL"), "{cells:?}");

        // A skill says the same about its own file, which is also never sent.
        let mut skilled = State::new();
        skilled.apply(tool_start("s", "skill", serde_json::json!({"name": "web-perf"})));
        let cells = skilled.call(0).expect("kept").cells();
        let invoked = cells.iter().find(|c| c.label == "INVOKED").expect("a cell");
        let text = invoked.lines.join("\n");
        assert!(text.contains("skill web-perf"), "{text}");
        assert!(text.contains("not on the wire"), "{text}");
    }

    /// What streamed out of a call is kept, because for a tool that taps its own
    /// stdout it is the only return value the wire ever carries.
    #[test]
    fn a_streaming_call_keeps_what_it_streamed_as_its_return() {
        let mut state = State::new();
        state.apply(tool_start("b", "bash", serde_json::json!({"cmd": "make"})));
        for line in ["cc -c main.c", "cc -c util.c", "ld -o app"] {
            state.apply(Event::ToolProgress {
                call_id: "b".into(),
                line: line.into(),
            });
        }
        state.apply(Event::ToolEnd {
            call_id: "b".into(),
            summary: "bash make (1.2s)".into(),
            elapsed_ms: 1200,
            error: None,
        });
        let cells = state.call(0).expect("kept").cells();
        let returned = cells.iter().find(|c| c.label == "RETURNED").expect("a cell");
        assert!(!returned.absent, "this one really did come back");
        let text = returned.lines.join("\n");
        assert!(text.contains("ld -o app"), "{text}");
    }

    /// The record list is bounded, and a call that has fallen off it stops
    /// resolving rather than answering with a neighbour's.
    #[test]
    fn the_call_records_are_bounded_and_the_oldest_stop_resolving() {
        let mut state = State::new();
        for n in 0..MAX_CALLS + 20 {
            state.apply(tool_start(&format!("c{n}"), "read", serde_json::json!({"path": "a.rs"})));
            state.apply(Event::ToolEnd {
                call_id: format!("c{n}"),
                summary: "read 1 line".into(),
                elapsed_ms: 1,
                error: None,
            });
        }
        assert_eq!(state.calls.len(), MAX_CALLS);
        assert_eq!(state.call(0), None, "the first twenty are gone");
        let last = state.call(MAX_CALLS + 19).expect("the newest is held");
        assert_eq!(last.call_id, format!("c{}", MAX_CALLS + 19));
    }

    /// Every call that came back with an error is counted, and nothing else is.
    ///
    /// This is what is left of the DEBUG pane: that pane kept the failures
    /// themselves, bounded at a hundred, and showed the length of the list as
    /// its count. The count is the part worth keeping, so it is a counter now
    /// and reads beside the total in the CONTEXT pane.
    #[test]
    fn a_call_that_came_back_with_an_error_is_counted() {
        let mut state = State::new();
        assert_eq!(state.failed_calls, 0);

        // One that works is not a failure.
        state.apply(tool_start("r", "read", serde_json::json!({"path": "a.rs"})));
        state.apply(Event::ToolEnd {
            call_id: "r".into(),
            summary: "read 40 lines".into(),
            elapsed_ms: 3,
            error: None,
        });
        assert_eq!(state.failed_calls, 0, "a call that worked is not a failure");

        state.apply(tool_start(
            "b",
            "bash",
            serde_json::json!({"cmd": "cargo build", "timeout": 30}),
        ));
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
        assert_eq!(state.failed_calls, 1);
        assert_eq!(state.tool_calls, 2, "both calls are still calls");

        // An end whose start this window never saw is still a call that failed.
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
        assert_eq!(state.failed_calls, 2);
    }

    /// The count does not saturate. The list it replaced was bounded at a
    /// hundred and dropped the oldest, so a run that failed more than that
    /// under-reported for the rest of the session.
    #[test]
    fn the_failed_count_is_not_bounded_the_way_the_old_list_was() {
        let mut state = State::new();
        for n in 0..250 {
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
        }
        assert_eq!(state.failed_calls, 250);
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

    /// Live output from a running command accumulates on the call for its
    /// popup and moves the status line; the pane keeps the call's one row,
    /// so two concurrent tools cannot bury the list under their chatter.
    #[test]
    fn a_running_command_keeps_its_output_on_its_own_record() {
        let mut state = State::new();
        state.apply(tool_start("b", "bash", serde_json::json!({"cmd": "cargo build"})));
        for line in ["compiling noob", "compiling no0b"] {
            state.apply(Event::ToolProgress {
                call_id: "b".into(),
                line: line.into(),
            });
        }
        assert_eq!(state.activity.visible(200, 200).len(), 1, "one call, one row");
        let call = state.call(0).expect("the record");
        assert_eq!(call.output, ["compiling noob", "compiling no0b"]);
        assert_eq!(state.status, "bash compiling no0b", "the latest line is the status");
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

    /// A failure says what broke on its popup, bounded. This is the same
    /// failure that used to render as a bare `exit code 1`.
    #[test]
    fn a_failure_shows_its_message_and_a_bounded_body() {
        let mut state = State::new();
        state.apply(tool_start(
            "x",
            "bash",
            serde_json::json!({"cmd": "cargo test"}),
        ));
        let detail = std::iter::once(String::from("exit code 1"))
            .chain((0..80).map(|n| format!("trace line {n}")))
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
        // The pane keeps the one recolored row; the popup's DETAIL block
        // carries the message and the bounded body, nothing said twice.
        assert_eq!(texts(&state.activity).len(), 1);
        let call = state.call(0).expect("the record");
        let detail = call
            .cells()
            .into_iter()
            .find(|cell| cell.label == "DETAIL")
            .expect("the detail block");
        assert!(
            detail.lines.iter().any(|l| l.contains("could not compile")),
            "{detail:?}"
        );
        assert!(detail.lines.iter().any(|l| l.contains("trace line 0")), "{detail:?}");
        assert!(detail.lines.iter().any(|l| l.contains('…')), "{detail:?}");
        assert!(
            detail.lines.len() <= CELL_LINES + 4,
            "the body is bounded: {}",
            detail.lines.len()
        );
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
        assert_eq!(state.headline(), "INFERRING");
        state.apply(tool_start(
            "p",
            "plan",
            serde_json::json!({"todos": [
                {"content": "one", "status": "completed"},
                {"content": "two", "status": "in_progress"},
            ]}),
        ));
        assert!(
            state.headline().starts_with("INFERRING"),
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

    /// The word and the orb say the same thing, which they can only do if the
    /// phase leaves READY the moment a turn starts and every phase the orb turns
    /// for writes one word.
    ///
    /// Both halves are asserted here because a window that said READY while the
    /// orb turned would be a bug in the phase rather than in the word, and the
    /// route in is not one event: a prompt sent from this window moves it, and
    /// so does the turn the agent reports having started.
    #[test]
    fn a_turn_leaves_ready_and_every_busy_phase_says_infering() {
        let mut state = State::new();
        state.apply(Event::SessionStart {
            id: "s1".into(),
            workspace: "/tmp".into(),
            model: "a-model".into(),
            resumed: false,
        });
        assert_eq!(state.phase, Phase::Ready);
        assert_eq!(state.headline(), "READY");
        assert!(!state.phase.busy());

        // Typed here: the window moves the phase itself rather than waiting for
        // the agent to report a turn, so the orb starts with the keystroke.
        state.submitted("go");
        assert_eq!(state.phase, Phase::Thinking);
        assert!(state.phase.busy());
        assert_eq!(state.headline(), "INFERRING");

        // Reported by the agent: the same phase from the other direction.
        let mut from_the_agent = State::new();
        from_the_agent.apply(Event::TurnStart { turn: 1 });
        assert_eq!(from_the_agent.phase, Phase::Thinking);

        state.apply(tool_start("b", "bash", serde_json::json!({"cmd": "ls"})));
        assert_eq!(state.phase, Phase::Working);
        assert!(state.phase.busy());
        assert!(state.headline().starts_with("INFERRING"), "{}", state.headline());

        // One word for every phase the orb turns for, and it is never the
        // resting one.
        for phase in [Phase::Thinking, Phase::Working] {
            assert!(phase.busy());
            assert_eq!(phase.word(), "INFERRING");
        }
        for phase in [Phase::Starting, Phase::Ready, Phase::Finished, Phase::Gone] {
            assert!(!phase.busy());
            assert_ne!(phase.word(), "INFERRING", "{phase:?}");
        }
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
        // The abandoned call's row goes to the failure colour and the reason
        // waits on its popup, the same shape every other ending has.
        let line = state.activity.line(0).expect("the call's row");
        assert_eq!(line.tone, Tone::Bad);
        let call = state.call(0).expect("the record");
        assert!(call.done, "the turn that owned it has ended");
        assert_eq!(call.summary, "never reported back");
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
        assert_eq!(pane.spot_in(rows, cols, 0, 0), Some((0, 0)));
        assert_eq!(
            pane.spot_in(rows, cols, 1, 0),
            Some((0, 50)),
            "the second row starts fifty characters in"
        );
        assert_eq!(pane.spot_in(rows, cols, 2, 0), Some((0, 100)));
        assert_eq!(
            pane.spot_in(rows, cols, 3, 0),
            None,
            "and below the text is nothing, not the last character"
        );
    }

    /// The same line with blanks in it: the pane breaks at the blank, so the
    /// row below starts where the words let it and not fifty characters in.
    /// The column is counted from the start of that row.
    #[test]
    fn a_row_of_a_line_wrapped_at_a_blank_starts_where_the_words_end() {
        let mut pane = Pane::new(100);
        // Two words of forty, so the break is at the blank at index 40 rather
        // than mid-word at fifty.
        let prose = format!("{} {}", "a".repeat(40), "b".repeat(40));
        pane.say(prose.clone(), Tone::Body);
        let (rows, cols) = (3, 50);
        assert_eq!(pane.spot_in(rows, cols, 0, 0), Some((0, 0)));
        assert_eq!(
            pane.spot_in(rows, cols, 1, 0),
            Some((0, 41)),
            "the second row starts past the blank the first one broke at"
        );
        assert_eq!(
            prose.chars().nth(41),
            Some('b'),
            "and that is the first character of the second word"
        );
        // A pointer out past the end of a row takes that row's last character,
        // not one from the row below it.
        assert_eq!(pane.spot_in(rows, cols, 0, 49), Some((0, 40)));
        assert_eq!(pane.spot_in(rows, cols, 1, 49), Some((0, 81)));
        assert_eq!(prose.chars().count(), 81);
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

    /// A Markdown pane is measured in what it draws, and what it draws moves
    /// while the line is still arriving: `**bold` is prose until the closing
    /// pair lands, at which point four characters leave the screen. The tail is
    /// drawn again on every chunk, so the rows and the columns move with it.
    #[test]
    fn a_streamed_markdown_line_is_measured_in_what_it_draws() {
        let mut pane = Pane::new(100).rendered();
        pane.stream("- **read", Tone::Body);
        let held = pane.line(0).expect("the tail is held");
        assert_eq!(held.shown(), "• **read", "an unclosed pair is text");
        pane.stream("** a file", Tone::Body);
        let held = pane.line(0).expect("the tail is held");
        assert_eq!(held.shown(), "• read a file");
        assert_eq!(pane.rows_of_line(0, 8).len(), 2, "measured in the thirteen drawn");
        // Every row of it is reachable, the last character included.
        assert_eq!(pane.spot_in(4, 8, 1, 99), Some((0, 13)));

        // What the human typed is not Markdown and keeps its marks.
        pane.stream("\n", Tone::Body);
        pane.stream("- **read** a file", Tone::Bright);
        let typed = pane.line(pane.last() - 1).expect("the typed line is held");
        assert_eq!(typed.shown(), typed.text);
    }

    /// A fenced block runs across lines, so a line inside one is code and keeps
    /// every mark in it, wherever the fence was opened.
    #[test]
    fn a_line_inside_a_fence_is_drawn_as_it_was_written() {
        let mut pane = Pane::new(100).rendered();
        for text in ["```python", "x = 1  # **not bold**", "```", "- **bold**"] {
            pane.say(text, Tone::Body);
        }
        assert_eq!(pane.line(1).unwrap().shown(), "x = 1  # **not bold**");
        assert_eq!(pane.line(3).unwrap().shown(), "• bold");
        // The fence line itself is structure and is drawn as the block it opens.
        assert_eq!(pane.line(0).unwrap().shown(), "── python ");
    }

    /// The line that opened a block can fall off the top while its body is
    /// still on screen. The body is still code, so it still keeps its marks,
    /// and the window still draws it as code: a pane that worked the fence out
    /// again from the lines it has left would call it prose, and then measure
    /// it as one thing and draw it as another.
    #[test]
    fn a_block_whose_fence_scrolled_away_is_still_code() {
        let mut pane = Pane::new(3).rendered();
        pane.say("```python", Tone::Body);
        for n in 0..3 {
            pane.say(format!("x = {n}  # **not bold**"), Tone::Body);
        }
        assert!(pane.line(0).is_none(), "the fence line has been evicted");
        let held = pane.line(3).expect("the last line is held");
        assert_eq!(held.shown(), "x = 2  # **not bold**");
        assert_eq!(held.shown(), held.text);
        assert!(
            pane.fence_before(3, 200).0.is_some(),
            "the window draws the block it is inside"
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
