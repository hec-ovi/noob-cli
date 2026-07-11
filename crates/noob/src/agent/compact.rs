//! Compaction: the one sanctioned cache-prefix break, hardened for
//! multi-cycle survival on small local models (design record:
//! .research/context-compaction-survival).
//!
//! Ladder, cheapest first:
//! 1. PRUNE: replace old, large tool-result bodies in the middle with
//!    one-line placeholders (no LLM call, keeps the conversational
//!    skeleton). Adopted when it alone brings usage under the target.
//! 2. SUMMARIZE: the middle (which includes any previous summary, so facts
//!    merge instead of eroding) is LLM-summarized against the schema in
//!    prompts/compact.md, then VALIDATED deterministically: an empty or
//!    non-shrinking "summary" is retried once and then falls back to
//!    pruning or the hard drop, never spliced (a small summarizer fails in
//!    exactly these ways).
//! 3. HARD DROP: deterministic removal of the middle with a stub note,
//!    when the summarize request itself overflows or validation fails
//!    twice with nothing left to prune.
//!
//! Whatever path ran, a deterministic PINNED BLOCK is appended to the
//! spliced message: the original task, the files touched, and loaded skill
//! names, all assembled by the harness from ground truth, never by the
//! summarizer. A provider failure sets a backoff so a failing summarizer
//! is not retried on every subsequent round (the compression-loop trap).

use noob_provider::types::{Item, ProviderError, TurnRequest};

use super::{Agent, looks_like_context_overflow, prompt};
use crate::ui::Ui;

/// Tail kept verbatim, in estimated tokens: ~20k on a full-size context,
/// scaled down to a quarter of small windows so compaction still has a
/// middle to remove.
const TAIL_TOKENS: u64 = 20_000;

/// Prune is adopted when it alone gets estimated usage under this share of
/// the window; below it, a full summarize would rewrite the prefix for
/// little gain.
const PRUNE_TARGET_NUM: u64 = 3;
const PRUNE_TARGET_DEN: u64 = 5; // 60%

/// Tool results smaller than this stay verbatim: pruning them buys almost
/// nothing and every byte of skeleton helps the model.
const PRUNE_FLOOR: usize = 2 * 1024;

/// Ceiling on pinned list lines (paths), so the pin itself stays tiny.
const PIN_MAX_FILES: usize = 30;
/// The task pin keeps this many characters of the first user input.
const PIN_TASK_CHARS: usize = 500;

fn tail_budget(ctx_tokens: u64) -> u64 {
    TAIL_TOKENS.min(ctx_tokens / 4)
}

const SUMMARIZE_ASK: &str =
    "Summarize the conversation above following your instructions. Output only the summary.";

/// chars/4 token stand-in per transcript item, plus a small per-item
/// serialization overhead.
pub fn item_chars(item: &Item) -> usize {
    40 + match item {
        Item::User(text) => text.len(),
        Item::Assistant { text, tool_calls, raw_items } => {
            text.len()
                + tool_calls
                    .iter()
                    .map(|c| c.id.len() + c.name.len() + c.arguments.len())
                    .sum::<usize>()
                + raw_items.iter().map(|v| v.to_string().len()).sum::<usize>()
        }
        Item::ToolResult { call_id, content } => call_id.len() + content.len(),
    }
}

/// Deterministic summary validation (small summarizers fail in known ways).
enum SummaryCheck {
    Ok,
    /// Empty or not materially smaller than what it replaces: splicing it
    /// would wedge the session (the Gemini CLI failure catalog).
    Hard(&'static str),
    /// Usable but schema-poor: accepted with a warning, never retried.
    Soft,
}

fn validate_summary(summary: &str, middle_chars: usize) -> SummaryCheck {
    if summary.trim().is_empty() {
        return SummaryCheck::Hard("the summarizer returned nothing");
    }
    if summary.len() >= middle_chars / 2 {
        return SummaryCheck::Hard("the summary is not materially smaller than what it replaces");
    }
    let headers = [
        "## Goal",
        "## Key decisions",
        "## Files touched",
        "## Completed",
        "## In progress",
        "## Next steps",
    ];
    let found = headers.iter().filter(|h| summary.contains(*h)).count();
    if found < 2 {
        return SummaryCheck::Soft;
    }
    SummaryCheck::Ok
}

/// The original task, recovered deterministically: the first real user
/// input still in the transcript, or the `[task: ...]` pin a previous
/// cycle carried (so the pin survives any number of compactions).
fn find_task_pin(items: &[Item]) -> Option<String> {
    for item in items {
        let Item::User(text) = item else { continue };
        if let Some(line) = text.lines().find(|l| l.starts_with("[task: ")) {
            return line
                .strip_prefix("[task: ")
                .and_then(|rest| rest.strip_suffix(']'))
                .map(str::to_string);
        }
        if !text.starts_with('[') {
            let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let clipped: String = one_line.chars().take(PIN_TASK_CHARS).collect();
            return Some(clipped);
        }
    }
    None
}

/// Files named by a previous cycle's pin, so the list survives process
/// resumes where the in-memory seen-files registry starts empty.
fn find_files_pin(items: &[Item]) -> Vec<String> {
    for item in items {
        let Item::User(text) = item else { continue };
        for line in text.lines() {
            if let Some(rest) = line
                .strip_prefix("[files touched: ")
                .and_then(|r| r.strip_suffix(']'))
            {
                let rest = rest.split(" (+").next().unwrap_or(rest);
                return rest.split(", ").map(str::to_string).collect();
            }
        }
    }
    Vec::new()
}

/// A pruned copy of the middle: old, large tool-result bodies (never skill
/// loads; the pair structure always survives) replaced with a placeholder
/// naming the tool and the next move. Returns (items, chars saved).
fn pruned_middle(items: &[Item], cut: usize) -> (Vec<Item>, usize) {
    // Tool names by call id, so the placeholder can teach and skill loads
    // can be exempted (their bodies are the whole point of loading).
    let mut names: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for item in &items[..cut] {
        if let Item::Assistant { tool_calls, .. } = item {
            for call in tool_calls {
                names.insert(call.id.as_str(), call.name.as_str());
            }
        }
    }
    let mut saved = 0usize;
    let mut out: Vec<Item> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        if i >= cut {
            out.push(item.clone());
            continue;
        }
        match item {
            Item::ToolResult { call_id, content }
                if content.len() > PRUNE_FLOOR
                    && names.get(call_id.as_str()).copied() != Some("skill") =>
            {
                let name = names.get(call_id.as_str()).copied().unwrap_or("tool");
                let placeholder = format!(
                    "[an old {name} result ({} bytes) was removed to save context; \
                     re-run the tool if you need it]",
                    content.len()
                );
                saved += content.len().saturating_sub(placeholder.len());
                out.push(Item::ToolResult { call_id: call_id.clone(), content: placeholder });
            }
            other => out.push(other.clone()),
        }
    }
    (out, saved)
}

impl Agent {
    /// Compact the transcript. Returns true when the transcript changed.
    pub fn compact(&mut self, ui: &mut Ui) -> bool {
        // Walk back from the end until the tail holds the budget.
        let budget = tail_budget(self.ctx_tokens);
        let mut cut = self.items.len();
        let mut acc = 0u64;
        while cut > 0 {
            let c = (item_chars(&self.items[cut - 1]) / 4) as u64;
            if acc + c > budget {
                break;
            }
            acc += c;
            cut -= 1;
        }
        // Never split a call/result pair: a tail starting with tool results
        // pulls its assistant call message (and stays whole) into the tail.
        while cut > 0 && matches!(self.items.get(cut), Some(Item::ToolResult { .. })) {
            cut -= 1;
        }
        if cut < 2 {
            ui.note("nothing to compact yet");
            return false;
        }

        // Ladder step 1: pruning alone, when it frees enough. No LLM call,
        // no hallucination risk, the conversational skeleton survives.
        let (pruned, saved) = pruned_middle(&self.items, cut);
        let estimate = self.context_estimate();
        let target = self.ctx_tokens * PRUNE_TARGET_NUM / PRUNE_TARGET_DEN;
        if saved > 0 && estimate.saturating_sub((saved / 4) as u64) <= target {
            let pruned_count = pruned
                .iter()
                .zip(&self.items)
                .filter(|(a, b)| !items_eq(a, b))
                .count();
            self.adopt(pruned, ui);
            ui.note(&format!(
                "cache prefix reset: pruned {pruned_count} old tool results (no summary needed)"
            ));
            return true;
        }

        // Ladder step 2: schema'd summary of the (unpruned) middle, which
        // includes any previous [conversation summary] message, so cycles
        // merge instead of re-summarizing a summary alone.
        let mut middle: Vec<Item> = self.items[..cut].to_vec();
        middle.push(Item::User(SUMMARIZE_ASK.to_string()));
        let middle_chars: usize = self.items[..cut].iter().map(item_chars).sum();
        let req = TurnRequest {
            system: Some(prompt::COMPACT_MD.to_string()),
            items: middle,
            tools: vec![],
        };
        ui.note("compacting the conversation…");
        let mut summary: Option<String> = None;
        let mut hard_drop_reason: Option<String> = None;
        for attempt in 0..2 {
            let result = noob_provider::run_turn(
                &self.client,
                &self.config_dir,
                &self.ov,
                &req,
                &mut |_ev| {},
            );
            match result {
                Err(ProviderError::Interrupted) => {
                    ui.note("compaction canceled");
                    return false;
                }
                // The summarize request itself overflowed: deterministic
                // hard drop (retrying the same overflow is pointless).
                Err(ProviderError::Http { status: 400, ref body })
                    if looks_like_context_overflow(body) =>
                {
                    hard_drop_reason = Some("the context overflowed".to_string());
                    break;
                }
                Err(e) => {
                    // A transport-level failure must never destroy content,
                    // and must not be retried on every subsequent round
                    // (the compression-loop trap): back off until usage
                    // grows further.
                    ui.note(&format!("compaction failed: {e}; continuing uncompacted"));
                    self.compact_backoff = estimate + self.ctx_tokens / 20;
                    return false;
                }
                Ok(turn) => {
                    let text = turn.text.trim().to_string();
                    match validate_summary(&text, middle_chars) {
                        SummaryCheck::Ok => {
                            summary = Some(text);
                            break;
                        }
                        SummaryCheck::Soft => {
                            ui.note(
                                "the summary is missing sections; keeping it anyway \
                                 (the pinned facts below it are deterministic)",
                            );
                            summary = Some(text);
                            break;
                        }
                        SummaryCheck::Hard(reason) => {
                            if attempt == 0 {
                                ui.note(&format!("{reason}; retrying the summary once"));
                            } else {
                                hard_drop_reason = Some(reason.to_string());
                            }
                        }
                    }
                }
            }
        }

        let head_text = match (summary, hard_drop_reason) {
            (Some(s), _) => format!("[conversation summary]\n{s}"),
            (None, Some(reason)) => {
                // Ladder step 3a: an invalid summary but a prunable middle:
                // head-tail retention (prune everything prunable) beats
                // destroying the middle.
                if saved > 0 {
                    self.adopt(pruned, ui);
                    self.compact_backoff =
                        self.context_estimate() + self.ctx_tokens / 20;
                    ui.note(&format!(
                        "cache prefix reset: {reason}; pruned old tool results instead"
                    ));
                    return true;
                }
                // Ladder step 3b: nothing to prune; the stub note is the
                // only option left.
                format!(
                    "[earlier conversation dropped: {cut} items removed because {reason}]"
                )
            }
            (None, None) => unreachable!("the attempt loop always sets one of the two"),
        };

        // The pinned block: ground truth the summarizer never touches.
        let mut spliced = head_text;
        if let Some(task) = find_task_pin(&self.items) {
            spliced.push_str(&format!("\n[task: {task}]"));
        }
        // Files: this process's ground truth, merged with any previous
        // cycle's pin (the seen registry does not survive a resume).
        let mut files: Vec<String> = self
            .tool_ctx
            .seen
            .paths()
            .iter()
            .map(|p| crate::tools::display_path(&self.tool_ctx, p))
            .collect();
        for prev in find_files_pin(&self.items) {
            if !files.contains(&prev) {
                files.push(prev);
            }
        }
        files.sort();
        if !files.is_empty() {
            let more = files.len().saturating_sub(PIN_MAX_FILES);
            files.truncate(PIN_MAX_FILES);
            let suffix = if more > 0 { format!(" (+{more} more)") } else { String::new() };
            spliced.push_str(&format!("\n[files touched: {}{suffix}]", files.join(", ")));
        }
        // Deterministic re-listing (names only) so the model does not forget
        // what it loaded, even when the summarizer ignores its instructions
        // or the hard-drop path ran. Bodies are reloadable via the tool.
        let loaded = self.tool_ctx.loaded_skills.lock().unwrap().join(", ");
        if !loaded.is_empty() {
            spliced.push_str(&format!("\n[loaded skills: {loaded}]"));
        }
        // If the live skill set drifted from session start (an on-the-fly
        // /skills add or remove), pin the current set so it outlives the
        // summarized [skills updated] note: the frozen head index still lists
        // the original skills, and this is what keeps the model from offering
        // a removed one after a compaction.
        if let Some(current) = self.skills_drifted() {
            let listed = if current.is_empty() { "none".to_string() } else { current.join(", ") };
            spliced.push_str(&format!("\n[skills available now: {listed}]"));
        }

        let mut new_items = vec![Item::User(spliced)];
        new_items.extend_from_slice(&self.items[cut..]);
        self.adopt(new_items, ui);
        ui.note("cache prefix reset: compaction");
        true
    }

    /// Install a compacted transcript and reset everything the old one
    /// backed: the repeat detector (an identical call is now legitimate),
    /// the usage baseline, the failure backoff, and the session log.
    fn adopt(&mut self, items: Vec<Item>, _ui: &mut Ui) {
        self.items = items;
        self.recent_calls.clear();
        self.last_usage = None;
        self.chars_since_usage = self.items.iter().map(item_chars).sum();
        self.compact_backoff = 0;
        if let Some(s) = &mut self.session {
            s.log_reset(&self.items);
        }
    }
}

/// Structural equality for the prune count (Item does not derive PartialEq
/// because raw_items carries arbitrary JSON; compare what pruning changes).
fn items_eq(a: &Item, b: &Item) -> bool {
    match (a, b) {
        (
            Item::ToolResult { call_id: ia, content: ca },
            Item::ToolResult { call_id: ib, content: cb },
        ) => ia == ib && ca == cb,
        _ => true, // pruning only rewrites tool results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noob_provider::types::ToolCall;

    #[test]
    fn item_chars_counts_all_payload_fields() {
        let user = Item::User("abcd".into());
        assert_eq!(item_chars(&user), 44);
        let asst = Item::Assistant {
            text: "ab".into(),
            tool_calls: vec![ToolCall {
                id: "12".into(),
                name: "read".into(),
                arguments: "{}".into(),
            }],
            raw_items: vec![],
        };
        assert_eq!(item_chars(&asst), 40 + 2 + 2 + 4 + 2);
    }

    #[test]
    fn summary_validation_catches_the_small_model_failure_modes() {
        assert!(matches!(validate_summary("", 10_000), SummaryCheck::Hard(_)));
        assert!(matches!(validate_summary("   \n  ", 10_000), SummaryCheck::Hard(_)));
        // A "summary" as big as what it replaces would wedge the session.
        let inflated = "x".repeat(6_000);
        assert!(matches!(validate_summary(&inflated, 10_000), SummaryCheck::Hard(_)));
        // Schema-poor but small and non-empty: soft-accepted.
        assert!(matches!(validate_summary("did stuff", 10_000), SummaryCheck::Soft));
        let good = "## Goal\nfix the bug\n## Next steps\nrun tests";
        assert!(matches!(validate_summary(good, 10_000), SummaryCheck::Ok));
    }

    #[test]
    fn task_pin_prefers_real_input_and_survives_cycles() {
        // First real user input wins; bracket notes are skipped.
        let items = vec![
            Item::User("[interrupted]".into()),
            Item::User("fix the flaky test\nin ci".into()),
        ];
        assert_eq!(find_task_pin(&items).unwrap(), "fix the flaky test in ci");
        // After a compaction the pin line inside the summary is the source.
        let items = vec![Item::User(
            "[conversation summary]\nwork happened\n[task: fix the flaky test in ci]\n[files touched: a]"
                .into(),
        )];
        assert_eq!(find_task_pin(&items).unwrap(), "fix the flaky test in ci");
        assert!(find_task_pin(&[]).is_none());
        // A giant first input is clipped to the pin budget.
        let items = vec![Item::User("w".repeat(5_000))];
        assert_eq!(find_task_pin(&items).unwrap().chars().count(), 500);
    }

    fn result(id: &str, content: String) -> Item {
        Item::ToolResult { call_id: id.into(), content }
    }

    fn calls(pairs: &[(&str, &str)]) -> Item {
        Item::Assistant {
            text: String::new(),
            tool_calls: pairs
                .iter()
                .map(|(id, name)| ToolCall {
                    id: (*id).into(),
                    name: (*name).into(),
                    arguments: "{}".into(),
                })
                .collect(),
            raw_items: vec![],
        }
    }

    #[test]
    fn prune_replaces_only_old_fat_non_skill_results() {
        let items = vec![
            Item::User("go".into()),
            calls(&[("b1", "bash"), ("s1", "skill"), ("r1", "read")]),
            result("b1", "x".repeat(10_000)),  // fat bash: pruned
            result("s1", "y".repeat(10_000)),  // fat skill load: never pruned
            result("r1", "small".to_string()), // under the floor: kept
            calls(&[("b2", "bash")]),
            result("b2", "z".repeat(10_000)), // in the tail: kept
        ];
        let cut = 5; // the last call/result pair is the protected tail
        let (pruned, saved) = pruned_middle(&items, cut);
        assert!(saved > 9_000, "saved {saved}");
        match &pruned[2] {
            Item::ToolResult { content, .. } => {
                assert_eq!(
                    content,
                    "[an old bash result (10000 bytes) was removed to save context; \
                     re-run the tool if you need it]"
                );
            }
            other => panic!("wrong item {other:?}"),
        }
        match &pruned[3] {
            Item::ToolResult { content, .. } => {
                assert_eq!(content.len(), 10_000, "skill loads are never pruned");
            }
            other => panic!("wrong item {other:?}"),
        }
        match (&pruned[4], &pruned[6]) {
            (Item::ToolResult { content: small, .. }, Item::ToolResult { content: tail, .. }) => {
                assert_eq!(small, "small");
                assert_eq!(tail.len(), 10_000, "the tail is untouchable");
            }
            other => panic!("wrong items {other:?}"),
        }
        // The call/result pairing is intact: same ids, same order.
        assert_eq!(pruned.len(), items.len());
    }
}
