//! Chat Completions delta assembler: a state machine keyed by
//! `tool_calls[].index`, hardened against the quirk matrix in
//! ARCHITECTURE.md. Pure function of parsed chunk JSON: fixture-replayable
//! with zero mocks.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use crate::types::{Event, Finish, ToolCall, Turn, Usage};

/// Session-unique sequence for synthesized call ids. Some llama.cpp
/// templates omit the id, but a tool result requires `tool_call_id`, and
/// ids must not collide across turns within one transcript.
static SYNTH_SEQ: AtomicU64 = AtomicU64::new(0);

/// `call_<seq>_<index>`: unique for the process lifetime, so no two turns
/// in one transcript can collide. Shared by every path that has to invent
/// an id (streamed deltas, non-streamed completions).
pub(crate) fn synth_call_id(index: usize) -> String {
    let seq = SYNTH_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("call_{seq}_{index}")
}

#[derive(Debug, Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug, Default)]
pub struct Assembler {
    text: String,
    reasoning: String,
    /// In order of first appearance; position = our emission index.
    calls: Vec<(u64, PartialCall)>,
    /// The wire index of the most recently opened call, for deltas that
    /// arrive without an `index` (some proxies, older Azure/Mistral).
    last_wire_index: Option<u64>,
    usage: Option<Usage>,
    finish_reason: Option<String>,
    error: Option<String>,
}

impl Assembler {
    pub fn new() -> Assembler {
        Assembler::default()
    }

    /// Absorb one parsed SSE chunk, emitting events as they materialize.
    pub fn on_chunk(&mut self, chunk: &Value, on: &mut dyn FnMut(Event)) {
        // In-band mid-stream error payloads (OpenRouter emits these after
        // streaming starts): a turn error, never a panic.
        if let Some(err) = chunk.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| err.to_string());
            self.error = Some(msg);
            return;
        }

        // Usage arrives on the final chunk with stream_options.include_usage;
        // OpenAI sends `"usage": null` on every chunk before it.
        if let Some(u) = chunk.get("usage").filter(|u| u.is_object()) {
            let usage = Usage {
                prompt_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
                completion_tokens: u
                    .get("completion_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                cached_prompt_tokens: u
                    .get("prompt_tokens_details")
                    .and_then(|d| d.get("cached_tokens"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            };
            self.usage = Some(usage);
            on(Event::Usage(usage));
        }

        let Some(choice) = chunk.get("choices").and_then(|c| c.get(0)) else {
            return; // usage-only final chunk has "choices": []
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish_reason = Some(reason.to_string());
        }
        if let Some(delta) = choice.get("delta") {
            self.on_message(delta, true, on);
        }
        // Some backends put the whole assembled message in a final non-delta
        // `choices[].message`; accepted at any point.
        if let Some(msg) = choice.get("message") {
            self.on_message(msg, false, on);
        }
    }

    fn on_message(&mut self, msg: &Value, is_delta: bool, on: &mut dyn FnMut(Event)) {
        if let Some(t) = msg.get("content").and_then(Value::as_str) {
            if is_delta {
                if !t.is_empty() {
                    self.text.push_str(t);
                    on(Event::Text(t.to_string()));
                }
            } else if self.text.is_empty() && !t.is_empty() {
                // A full message repeating already-streamed text is ignored.
                self.text.push_str(t);
                on(Event::Text(t.to_string()));
            }
        }
        // DeepSeek convention (llama.cpp, vLLM) and the OpenRouter variant.
        if let Some(r) = msg
            .get("reasoning_content")
            .or_else(|| msg.get("reasoning"))
            .and_then(Value::as_str)
            .filter(|r| !r.is_empty())
        {
            self.reasoning.push_str(r);
            on(Event::Reasoning(r.to_string()));
        }
        if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
            for (i, entry) in calls.iter().enumerate() {
                // A non-delta message's array has no index fields; its
                // position IS the index (parse_completion semantics).
                let positional = if is_delta { None } else { Some(i as u64) };
                self.on_tool_delta(entry, positional, on);
            }
        }
    }

    /// Which call does an entry belong to? Explicit `index` wins; a
    /// non-delta array position is authoritative; an index-less delta with
    /// a NEW distinct id opens a new call (merging it into the last open
    /// call would concatenate two calls' arguments into invalid JSON and
    /// silently drop the second call); otherwise the most recently opened
    /// call, or index 0 when none is open.
    fn attribute(&self, entry: &Value, positional: Option<u64>) -> u64 {
        if let Some(i) = entry.get("index").and_then(Value::as_u64) {
            return i;
        }
        if let Some(i) = positional {
            return i;
        }
        if let Some(id) = entry
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            if let Some((w, _)) = self.calls.iter().find(|(_, c)| c.id == id) {
                return *w;
            }
            // A distinct new id is a new call (an open call always has an
            // id: real or synthesized at open).
            return self.calls.iter().map(|(w, _)| w + 1).max().unwrap_or(0);
        }
        self.last_wire_index.unwrap_or(0)
    }

    fn on_tool_delta(&mut self, entry: &Value, positional: Option<u64>, on: &mut dyn FnMut(Event)) {
        let wire_index = self.attribute(entry, positional);
        self.last_wire_index = Some(wire_index);

        let known = self.calls.iter().position(|(w, _)| *w == wire_index);
        let pos = match known {
            Some(pos) => pos,
            None => {
                self.calls.push((wire_index, PartialCall::default()));
                self.calls.len() - 1
            }
        };
        let started = known.is_some();
        let call = &mut self.calls[pos].1;

        // Repeated id/name in every delta: ignore after first.
        if call.id.is_empty()
            && let Some(id) = entry
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        {
            call.id = id.to_string();
        }
        let f = entry.get("function");
        if call.name.is_empty()
            && let Some(name) = f
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
        {
            call.name = name.to_string();
        }
        if !started {
            // Absent or empty id: synthesize one; a tool result requires it.
            if call.id.is_empty() {
                call.id = synth_call_id(wire_index as usize);
            }
            on(Event::ToolCallStart {
                index: pos as u32,
                id: call.id.clone(),
                name: call.name.clone(),
            });
        }

        match f.and_then(|f| f.get("arguments")) {
            Some(Value::String(s)) if !s.is_empty() => {
                call.arguments.push_str(s);
                on(Event::ToolArgsDelta {
                    index: pos as u32,
                    delta: s.clone(),
                });
            }
            // arguments as a JSON object instead of a string (live llama.cpp
            // regression, ggml-org/llama.cpp#20198): re-serialize canonically.
            Some(obj @ Value::Object(_)) => {
                let s = obj.to_string();
                call.arguments.push_str(&s);
                on(Event::ToolArgsDelta {
                    index: pos as u32,
                    delta: s,
                });
            }
            _ => {}
        }
    }

    /// Close the stream: validate and mechanically repair tool-call args,
    /// map the finish reason, emit `Done`. Called whether or not the server
    /// sent `[DONE]`; every started call leaves here with an id, a name, and
    /// arguments (possibly still invalid JSON, which the agent loop turns
    /// into an error tool result so the model can self-correct).
    pub fn finish(mut self, on: &mut dyn FnMut(Event)) -> Turn {
        let tool_calls: Vec<ToolCall> = self
            .calls
            .drain(..)
            .map(|(_, c)| ToolCall {
                id: c.id,
                name: c.name,
                arguments: repair_args(&c.arguments),
            })
            .collect();

        let finish = if let Some(msg) = self.error {
            Finish::Error(msg)
        } else {
            match self.finish_reason.as_deref() {
                Some("tool_calls") | Some("function_call") => Finish::ToolCalls,
                // Output is never capped, so Length means the context is
                // full: compaction territory, not retry territory.
                Some("length") => Finish::Length,
                Some("content_filter") => Finish::ContentFilter,
                _ if !tool_calls.is_empty() => Finish::ToolCalls,
                _ => Finish::Stop,
            }
        };
        on(Event::Done(finish.clone()));

        Turn {
            text: self.text,
            reasoning: if self.reasoning.is_empty() {
                None
            } else {
                Some(self.reasoning)
            },
            tool_calls,
            usage: self.usage,
            finish,
            raw_items: Vec::new(),
        }
    }
}

/// Empty args become `{}`; args that do not parse as JSON get exactly one
/// mechanical repair (strip markdown fences, trim). Still-invalid args are
/// returned as-is for the loop to reject with a useful error.
pub fn repair_args(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "{}".to_string();
    }
    if serde_json::from_str::<Value>(trimmed).is_ok() {
        return trimmed.to_string();
    }
    let repaired = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    if serde_json::from_str::<Value>(repaired).is_ok() {
        return repaired.to_string();
    }
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn drive(chunks: &[Value]) -> (Turn, Vec<Event>) {
        let mut events = Vec::new();
        let mut asm = Assembler::new();
        for c in chunks {
            asm.on_chunk(c, &mut |e| events.push(e));
        }
        let turn = asm.finish(&mut |e| events.push(e));
        (turn, events)
    }

    fn delta(v: Value) -> Value {
        json!({"choices": [{"index": 0, "delta": v}]})
    }

    #[test]
    fn standard_flow_id_and_name_first_then_arg_fragments() {
        let (turn, events) = drive(&[
            delta(json!({"role": "assistant", "content": null})),
            delta(
                json!({"tool_calls": [{"index": 0, "id": "c1", "type": "function",
                "function": {"name": "read", "arguments": "{"}}]}),
            ),
            delta(json!({"tool_calls": [{"index": 0,
                "function": {"arguments": "\"path\":\"a\"}"}}]})),
            json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
        ]);
        assert_eq!(turn.finish, Finish::ToolCalls);
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].id, "c1");
        assert_eq!(turn.tool_calls[0].name, "read");
        assert_eq!(turn.tool_calls[0].arguments, "{\"path\":\"a\"}");
        assert!(matches!(events[0], Event::ToolCallStart { ref id, .. } if id == "c1"));
    }

    #[test]
    fn missing_index_attributes_to_last_open_call() {
        let (turn, _) = drive(&[
            delta(json!({"tool_calls": [{"index": 0, "id": "c1",
                "function": {"name": "read", "arguments": "{\"a\""}}]})),
            // No index: belongs to c1.
            delta(json!({"tool_calls": [{"function": {"arguments": ":1}"}}]})),
        ]);
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].arguments, "{\"a\":1}");
    }

    #[test]
    fn missing_index_with_no_open_call_opens_index_zero() {
        let (turn, _) = drive(&[delta(json!({"tool_calls": [
            {"id": "c1", "function": {"name": "ls", "arguments": "{}"}}]}))]);
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "ls");
    }

    #[test]
    fn absent_id_synthesized_and_unique_across_turns() {
        let (t1, _) = drive(&[delta(json!({"tool_calls": [
            {"index": 0, "function": {"name": "a", "arguments": "{}"}}]}))]);
        let (t2, _) = drive(&[delta(json!({"tool_calls": [
            {"index": 0, "function": {"name": "b", "arguments": "{}"}}]}))]);
        assert!(!t1.tool_calls[0].id.is_empty());
        assert!(!t2.tool_calls[0].id.is_empty());
        assert_ne!(t1.tool_calls[0].id, t2.tool_calls[0].id);
    }

    #[test]
    fn repeated_id_and_name_in_every_delta_ignored_after_first() {
        let (turn, events) = drive(&[
            delta(json!({"tool_calls": [{"index": 0, "id": "c1",
                "function": {"name": "read", "arguments": "{\"p\""}}]})),
            delta(json!({"tool_calls": [{"index": 0, "id": "c1",
                "function": {"name": "read", "arguments": ":2}"}}]})),
        ]);
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].arguments, "{\"p\":2}");
        let starts = events
            .iter()
            .filter(|e| matches!(e, Event::ToolCallStart { .. }))
            .count();
        assert_eq!(starts, 1);
    }

    #[test]
    fn arguments_object_reserialized_canonically() {
        let (turn, _) = drive(&[delta(json!({"tool_calls": [{"index": 0, "id": "c1",
            "function": {"name": "read", "arguments": {"path": "a.txt"}}}]}))]);
        let parsed: Value = serde_json::from_str(&turn.tool_calls[0].arguments).unwrap();
        assert_eq!(parsed["path"], "a.txt");
    }

    #[test]
    fn whole_call_in_final_non_delta_message() {
        let (turn, _) = drive(&[
            json!({"choices": [{"index": 0, "finish_reason": "tool_calls",
            "message": {"role": "assistant", "content": null, "tool_calls": [
                {"id": "c9", "type": "function",
                 "function": {"name": "bash", "arguments": "{\"command\":\"ls\"}"}}]}}]}),
        ]);
        assert_eq!(turn.finish, Finish::ToolCalls);
        assert_eq!(turn.tool_calls[0].id, "c9");
        assert_eq!(turn.tool_calls[0].arguments, "{\"command\":\"ls\"}");
    }

    #[test]
    fn text_interleaved_with_concurrent_indexes_preserves_order() {
        let (turn, events) = drive(&[
            delta(json!({"content": "let me "})),
            delta(json!({"tool_calls": [{"index": 0, "id": "a",
                "function": {"name": "read", "arguments": "{\"p\":\"x\"}"}}]})),
            delta(json!({"tool_calls": [{"index": 1, "id": "b",
                "function": {"name": "read", "arguments": "{\"p\":\"y\"}"}}]})),
            delta(json!({"content": "look"})),
        ]);
        assert_eq!(turn.text, "let me look");
        assert_eq!(turn.tool_calls.len(), 2);
        assert_eq!(turn.tool_calls[0].id, "a");
        assert_eq!(turn.tool_calls[1].id, "b");
        // Arrival order: text, start a, start b, text.
        let kinds: Vec<u8> = events
            .iter()
            .map(|e| match e {
                Event::Text(_) => 0,
                Event::ToolCallStart { .. } => 1,
                Event::ToolArgsDelta { .. } => 2,
                _ => 9,
            })
            .filter(|k| *k < 9)
            .collect();
        assert_eq!(kinds, vec![0, 1, 2, 1, 2, 0]);
    }

    #[test]
    fn final_message_with_multiple_indexless_calls_stays_separate() {
        // Azure/proxy quirk: the whole tool_calls array arrives in a final
        // non-delta message, whose entries carry no index fields. Position
        // is authoritative; the calls must never merge.
        let (turn, events) = drive(&[json!({"choices": [{"index": 0,
        "finish_reason": "tool_calls",
        "message": {"tool_calls": [
            {"id": "c1", "function": {"name": "read", "arguments": "{\"path\":\"a\"}"}},
            {"id": "c2", "function": {"name": "bash", "arguments": "{\"command\":\"ls\"}"}}
        ]}}]})]);
        assert_eq!(turn.tool_calls.len(), 2);
        assert_eq!(turn.tool_calls[0].id, "c1");
        assert_eq!(turn.tool_calls[0].arguments, "{\"path\":\"a\"}");
        assert_eq!(turn.tool_calls[1].id, "c2");
        assert_eq!(turn.tool_calls[1].name, "bash");
        assert_eq!(turn.tool_calls[1].arguments, "{\"command\":\"ls\"}");
        let starts = events
            .iter()
            .filter(|e| matches!(e, Event::ToolCallStart { .. }))
            .count();
        assert_eq!(starts, 2);
    }

    #[test]
    fn indexless_deltas_with_distinct_ids_open_separate_calls() {
        let (turn, _) = drive(&[
            delta(json!({"tool_calls": [{"id": "c1",
                "function": {"name": "read", "arguments": "{\"p\":1}"}}]})),
            delta(json!({"tool_calls": [{"id": "c2",
                "function": {"name": "bash", "arguments": "{\"c\":2}"}}]})),
        ]);
        assert_eq!(turn.tool_calls.len(), 2);
        assert_eq!(turn.tool_calls[0].arguments, "{\"p\":1}");
        assert_eq!(turn.tool_calls[1].arguments, "{\"c\":2}");
    }

    #[test]
    fn indexless_idless_fragments_still_merge_into_the_open_call() {
        let (turn, _) = drive(&[
            delta(json!({"tool_calls": [{"id": "c1",
                "function": {"name": "read", "arguments": "{\"a\""}}]})),
            // Continuation fragments: no index, no id. Belong to c1.
            delta(json!({"tool_calls": [{"function": {"arguments": ":1}"}}]})),
        ]);
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].arguments, "{\"a\":1}");
    }

    #[test]
    fn midstream_error_payload_becomes_done_error() {
        let (turn, _) = drive(&[
            delta(json!({"content": "part"})),
            json!({"error": {"message": "upstream fell over", "code": 502}}),
        ]);
        assert_eq!(turn.text, "part");
        assert!(matches!(turn.finish, Finish::Error(ref m) if m.contains("upstream fell over")));
    }

    #[test]
    fn reasoning_content_and_reasoning_variants_both_stream() {
        let (turn, _) = drive(&[
            delta(json!({"reasoning_content": "hmm "})),
            delta(json!({"reasoning": "ok"})),
            delta(json!({"content": "hi"})),
        ]);
        assert_eq!(turn.reasoning.as_deref(), Some("hmm ok"));
        assert_eq!(turn.text, "hi");
    }

    #[test]
    fn finish_reason_mapping() {
        for (reason, want) in [
            ("stop", Finish::Stop),
            ("length", Finish::Length),
            ("content_filter", Finish::ContentFilter),
            ("function_call", Finish::ToolCalls),
        ] {
            let (turn, _) = drive(&[json!({"choices": [
                {"index": 0, "delta": {}, "finish_reason": reason}]})]);
            assert_eq!(turn.finish, want, "reason {reason}");
        }
    }

    #[test]
    fn usage_null_ignored_object_parsed() {
        let (turn, _) = drive(&[
            json!({"choices": [{"index": 0, "delta": {"content": "x"}}], "usage": null}),
            json!({"choices": [], "usage": {"prompt_tokens": 100, "completion_tokens": 7,
                "prompt_tokens_details": {"cached_tokens": 90}}}),
        ]);
        let u = turn.usage.unwrap();
        assert_eq!(u.prompt_tokens, 100);
        assert_eq!(u.completion_tokens, 7);
        assert_eq!(u.cached_prompt_tokens, 90);
    }

    #[test]
    fn full_message_does_not_duplicate_streamed_text() {
        let (turn, _) = drive(&[
            delta(json!({"content": "hello"})),
            json!({"choices": [{"index": 0, "finish_reason": "stop",
                "message": {"role": "assistant", "content": "hello"}}]}),
        ]);
        assert_eq!(turn.text, "hello");
    }

    #[test]
    fn repair_args_ladder() {
        assert_eq!(repair_args(""), "{}");
        assert_eq!(repair_args("  "), "{}");
        assert_eq!(repair_args("{\"a\":1}"), "{\"a\":1}");
        assert_eq!(repair_args("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(repair_args("```\n{}\n```"), "{}");
        // Still invalid after one repair: returned as-is for the loop.
        assert_eq!(repair_args("{broken"), "{broken");
    }

    #[test]
    fn empty_args_finish_as_empty_object() {
        let (turn, _) = drive(&[delta(json!({"tool_calls": [
            {"index": 0, "id": "c1", "function": {"name": "list"}}]}))]);
        assert_eq!(turn.tool_calls[0].arguments, "{}");
    }
}
