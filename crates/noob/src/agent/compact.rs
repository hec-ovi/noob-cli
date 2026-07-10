//! Compaction: the one sanctioned cache-prefix break. At 75% of NOOB_CTX
//! (checked before each request) the middle of the transcript is
//! LLM-summarized into one spliced user message, keeping the system head
//! and the most recent ~20k tokens intact and never splitting a
//! call/result pair. If the summarization itself overflows, fall back to a
//! deterministic hard drop with a stub note.

use noob_provider::types::{Item, ProviderError, TurnRequest};

use super::{Agent, looks_like_context_overflow, prompt};
use crate::ui::Ui;

/// Tail kept verbatim, in estimated tokens: ~20k on a full-size context,
/// scaled down to a quarter of small windows so compaction still has a
/// middle to remove.
const TAIL_TOKENS: u64 = 20_000;

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

        let mut middle: Vec<Item> = self.items[..cut].to_vec();
        middle.push(Item::User(SUMMARIZE_ASK.to_string()));
        let req = TurnRequest {
            system: Some(prompt::COMPACT_MD.to_string()),
            items: middle,
            tools: vec![],
        };
        ui.note("compacting the conversation…");
        let result = noob_provider::run_turn(
            &self.client,
            &self.config_dir,
            &self.ov,
            &req,
            &mut |_ev| {},
        );

        let mut head_text = match result {
            Ok(turn) if !turn.text.trim().is_empty() => {
                format!("[conversation summary]\n{}", turn.text.trim())
            }
            Err(ProviderError::Interrupted) => {
                ui.note("compaction canceled");
                return false;
            }
            // The summarization request itself overflowed: deterministic
            // hard drop of the middle, with a stub the model can see.
            Err(ProviderError::Http { status: 400, ref body })
                if looks_like_context_overflow(body) =>
            {
                format!(
                    "[earlier conversation dropped: {cut} items removed because the \
                     context overflowed]"
                )
            }
            Ok(_) | Err(_) => {
                // A failed summarize must never destroy content.
                if let Err(e) = &result {
                    ui.note(&format!("compaction failed: {e}; continuing uncompacted"));
                } else {
                    ui.note("compaction produced an empty summary; continuing uncompacted");
                }
                return false;
            }
        };
        // Deterministic re-listing (names only) so the model does not forget
        // what it loaded, even when the summarizer ignores its instructions
        // or the hard-drop path ran. Bodies are reloadable via the tool.
        let loaded = self.tool_ctx.loaded_skills.lock().unwrap().join(", ");
        if !loaded.is_empty() {
            head_text.push_str(&format!("\n[loaded skills: {loaded}]"));
        }

        let mut new_items = vec![Item::User(head_text)];
        new_items.extend_from_slice(&self.items[cut..]);
        self.items = new_items;
        // The compacted context invalidates the repeat detector: an
        // identical call is now legitimate (re-loading a skill whose body
        // was summarized away is exactly the sanctioned move).
        self.recent_calls.clear();
        // The old usage numbers describe a transcript that no longer
        // exists; fall back to the chars/4 estimate until fresh usage lands.
        self.last_usage = None;
        self.chars_since_usage = self.items.iter().map(item_chars).sum();
        if let Some(s) = &mut self.session {
            s.log_reset(&self.items);
        }
        ui.note("cache prefix reset: compaction");
        true
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
}
