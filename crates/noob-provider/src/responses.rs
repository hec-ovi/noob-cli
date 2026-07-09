//! Responses API adapter. Stateless full-input replay: `store: false`
//! always, no `previous_response_id` ever, prior turns replayed from their
//! captured wire items so reasoning survives byte-identical.
//!
//! Event routing is on the payload `type` (with the SSE `event:` field as
//! fallback). Unknown event types are ignored: the vocabulary grows, and a
//! new event must never crash the client.

use serde_json::{Value, json};

use crate::http::Client;
use crate::sse::SseParser;
use crate::types::{
    Endpoint, Event, Finish, Item, ProviderError, ToolCall, Turn, TurnRequest, Usage,
};

pub fn stream(
    client: &Client,
    ep: &Endpoint,
    req: &TurnRequest,
    on: &mut dyn FnMut(Event),
) -> Result<Turn, ProviderError> {
    let url = format!("{}/responses", ep.base_url);
    let mut body = build_body(ep, req);
    let mut resp = client.post_json_stream(&url, &ep.api_key, &mut body)?;

    // Same guard as chat: a JSON 200 to a stream:true request is a whole
    // response object, not an SSE stream.
    if resp.media_type() == "application/json" {
        let bytes = resp.read_to_end()?;
        let v: Value = serde_json::from_slice(&bytes)
            .map_err(|e| ProviderError::Wire(format!("invalid JSON response: {e}")))?;
        let mut state = State::default();
        state.on_completed_response(&v, on);
        return Ok(state.finish(on));
    }

    let mut parser = SseParser::new();
    let mut state = State::default();
    let mut events = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            parser.finish(&mut events);
        } else {
            parser.feed(&buf[..n], &mut events);
        }
        for ev in events.drain(..) {
            if ev.data.trim() == "[DONE]" {
                continue; // some servers append the chat sentinel; harmless
            }
            let payload: Value = serde_json::from_str(&ev.data).map_err(|e| {
                ProviderError::Wire(format!(
                    "invalid JSON in SSE data: {e}; first bytes: {}",
                    &ev.data.chars().take(120).collect::<String>()
                ))
            })?;
            let kind = payload
                .get("type")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or(ev.event);
            state.on_event(kind.as_deref().unwrap_or(""), &payload, on);
        }
        if n == 0 {
            break; // Responses streams end at EOF after response.completed
        }
    }
    Ok(state.finish(on))
}

/// The full request body. Stateless replay: `store: false` always, never
/// `previous_response_id`. Only api.openai.com gets the encrypted-reasoning
/// include (other servers 400 on it, and only OpenAI returns it anyway).
fn build_body(ep: &Endpoint, req: &TurnRequest) -> Value {
    let mut body = json!({
        "model": ep.model,
        "input": build_input(req),
        "store": false,
        "stream": true,
    });
    if let Some(system) = &req.system {
        body["instructions"] = json!(system);
    }
    if !req.tools.is_empty() {
        body["tools"] = Value::Array(
            req.tools
                .iter()
                .map(|t| {
                    json!({"type": "function", "name": t.name,
                        "description": t.description, "parameters": t.parameters})
                })
                .collect(),
        );
    }
    if ep.base_url.contains("api.openai.com") {
        body["include"] = json!(["reasoning.encrypted_content"]);
    }
    body
}

/// Serialize the neutral transcript into Responses `input` items.
fn build_input(req: &TurnRequest) -> Vec<Value> {
    let mut input = Vec::new();
    for item in &req.items {
        match item {
            Item::User(text) => {
                input.push(json!({"type": "message", "role": "user", "content": text}));
            }
            Item::Assistant { text, tool_calls, raw_items } => {
                if !raw_items.is_empty() {
                    // Verbatim replay preserves reasoning items and byte
                    // identity (the append-only prefix property).
                    input.extend(raw_items.iter().cloned());
                } else {
                    // A turn captured on the chat shape (style switched
                    // mid-session): reconstruct equivalent items.
                    if !text.is_empty() {
                        input.push(json!({
                            "type": "message", "role": "assistant", "content": text
                        }));
                    }
                    for c in tool_calls {
                        input.push(json!({"type": "function_call", "call_id": c.id,
                            "name": c.name, "arguments": c.arguments}));
                    }
                }
            }
            Item::ToolResult { call_id, content } => {
                input.push(json!({
                    "type": "function_call_output", "call_id": call_id, "output": content
                }));
            }
        }
    }
    input
}

#[derive(Default)]
struct State {
    text: String,
    reasoning: String,
    /// (item_id, call) in added order.
    calls: Vec<(String, ToolCall)>,
    raw_items: Vec<Value>,
    /// raw_items came from response.completed's output array (authoritative).
    raw_from_completed: bool,
    usage: Option<Usage>,
    finish: Option<Finish>,
}

impl State {
    fn on_event(&mut self, kind: &str, payload: &Value, on: &mut dyn FnMut(Event)) {
        match kind {
            "response.output_item.added" => {
                let Some(item) = payload.get("item") else { return };
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    let item_id = item
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let call = ToolCall {
                        id: item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("call_resp_{}", self.calls.len())),
                        name: item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        arguments: item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    };
                    on(Event::ToolCallStart {
                        index: self.calls.len() as u32,
                        id: call.id.clone(),
                        name: call.name.clone(),
                    });
                    self.calls.push((item_id, call));
                }
            }
            "response.function_call_arguments.delta" => {
                let delta = payload.get("delta").and_then(Value::as_str).unwrap_or_default();
                if let Some(pos) = self.call_pos(payload) {
                    self.calls[pos].1.arguments.push_str(delta);
                    on(Event::ToolArgsDelta { index: pos as u32, delta: delta.to_string() });
                }
            }
            "response.function_call_arguments.done" => {
                // Authoritative full arguments; replaces accumulated deltas.
                if let Some(args) = payload.get("arguments").and_then(Value::as_str) {
                    if let Some(pos) = self.call_pos(payload) {
                        self.calls[pos].1.arguments = args.to_string();
                    }
                }
            }
            "response.output_text.delta" => {
                let delta = payload.get("delta").and_then(Value::as_str).unwrap_or_default();
                if !delta.is_empty() {
                    self.text.push_str(delta);
                    on(Event::Text(delta.to_string()));
                }
            }
            "response.reasoning_text.delta" | "response.reasoning_summary_text.delta" => {
                let delta = payload.get("delta").and_then(Value::as_str).unwrap_or_default();
                if !delta.is_empty() {
                    self.reasoning.push_str(delta);
                    on(Event::Reasoning(delta.to_string()));
                }
            }
            "response.output_item.done" => {
                let Some(item) = payload.get("item") else { return };
                if !self.raw_from_completed {
                    self.raw_items.push(item.clone());
                }
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    // Completed item carries the final arguments.
                    if let Some(args) = item.get("arguments").and_then(Value::as_str) {
                        let item_id = item.get("id").and_then(Value::as_str).unwrap_or_default();
                        if let Some((_, call)) =
                            self.calls.iter_mut().find(|(iid, _)| iid == item_id)
                        {
                            call.arguments = args.to_string();
                        }
                    }
                }
            }
            "response.completed" => {
                let Some(response) = payload.get("response") else { return };
                self.on_completed_response(response, on);
            }
            "response.failed" => {
                let msg = payload
                    .pointer("/response/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("the server reported the response failed");
                self.finish = Some(Finish::Error(msg.to_string()));
            }
            "response.incomplete" => {
                let reason = payload
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                self.finish = Some(if reason == "content_filter" {
                    Finish::ContentFilter
                } else {
                    Finish::Error(format!("response incomplete: {reason}"))
                });
            }
            "error" => {
                let msg = payload
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("the server sent an in-band error event");
                self.finish = Some(Finish::Error(msg.to_string()));
            }
            // Unknown event types are ignored by design.
            _ => {}
        }
    }

    /// Resolve which call an arguments event belongs to. `item_id` when
    /// present; otherwise the last opened call.
    fn call_pos(&self, payload: &Value) -> Option<usize> {
        if let Some(item_id) = payload.get("item_id").and_then(Value::as_str) {
            if let Some(pos) = self.calls.iter().position(|(iid, _)| iid == item_id) {
                return Some(pos);
            }
        }
        self.calls.len().checked_sub(1)
    }

    /// Absorb a complete response object (from `response.completed` or the
    /// content-type-guard path).
    fn on_completed_response(&mut self, response: &Value, on: &mut dyn FnMut(Event)) {
        if let Some(output) = response.get("output").and_then(Value::as_array) {
            // Authoritative full output: replaces per-item captures.
            self.raw_items = output.to_vec();
            self.raw_from_completed = true;
            for item in output {
                match item.get("type").and_then(Value::as_str).unwrap_or("") {
                    "function_call" => {
                        let item_id =
                            item.get("id").and_then(Value::as_str).unwrap_or_default();
                        let known = self.calls.iter().any(|(iid, _)| iid == item_id);
                        if !known {
                            // Whole call only in the final response object.
                            let call = ToolCall {
                                id: item
                                    .get("call_id")
                                    .and_then(Value::as_str)
                                    .filter(|s| !s.is_empty())
                                    .map(str::to_string)
                                    .unwrap_or_else(|| {
                                        format!("call_resp_{}", self.calls.len())
                                    }),
                                name: item
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                                arguments: item
                                    .get("arguments")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                            };
                            on(Event::ToolCallStart {
                                index: self.calls.len() as u32,
                                id: call.id.clone(),
                                name: call.name.clone(),
                            });
                            on(Event::ToolArgsDelta {
                                index: self.calls.len() as u32,
                                delta: call.arguments.clone(),
                            });
                            self.calls.push((item_id.to_string(), call));
                        }
                    }
                    "message" => {
                        if self.text.is_empty() {
                            if let Some(parts) = item.get("content").and_then(Value::as_array) {
                                for part in parts {
                                    if let Some(t) =
                                        part.get("text").and_then(Value::as_str)
                                    {
                                        self.text.push_str(t);
                                    }
                                }
                                if !self.text.is_empty() {
                                    on(Event::Text(self.text.clone()));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if let Some(u) = response.get("usage").filter(|u| u.is_object()) {
            let usage = Usage {
                prompt_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
                completion_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
                cached_prompt_tokens: u
                    .pointer("/input_tokens_details/cached_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
            };
            self.usage = Some(usage);
            on(Event::Usage(usage));
        }
        if self.finish.is_none() {
            // The status matters on the content-type-guard path, where the
            // whole response object (possibly failed or incomplete) arrives
            // as one JSON document and no response.failed event ever fires.
            self.finish = Some(match response.get("status").and_then(Value::as_str) {
                Some("failed") => Finish::Error(
                    response
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("the server reported the response failed")
                        .to_string(),
                ),
                Some("incomplete") => {
                    let reason = response
                        .pointer("/incomplete_details/reason")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    if reason == "content_filter" {
                        Finish::ContentFilter
                    } else {
                        Finish::Error(format!("response incomplete: {reason}"))
                    }
                }
                _ if self.calls.is_empty() => Finish::Stop,
                _ => Finish::ToolCalls,
            });
        }
    }

    fn finish(mut self, on: &mut dyn FnMut(Event)) -> Turn {
        // A server that omitted call_id got a synthesized ToolCall.id; the
        // captured raw item must carry the same id, or the replayed
        // transcript pairs a function_call_output with no function_call.
        for item in &mut self.raw_items {
            if item.get("type").and_then(Value::as_str) != Some("function_call") {
                continue;
            }
            let missing = item
                .get("call_id")
                .and_then(Value::as_str)
                .map(str::is_empty)
                .unwrap_or(true);
            if missing {
                let item_id = item.get("id").and_then(Value::as_str).unwrap_or_default();
                if let Some((_, call)) = self.calls.iter().find(|(iid, _)| iid == item_id) {
                    item["call_id"] = Value::String(call.id.clone());
                }
            }
        }
        let tool_calls: Vec<ToolCall> = self
            .calls
            .drain(..)
            .map(|(_, mut c)| {
                c.arguments = crate::assemble::repair_args(&c.arguments);
                c
            })
            .collect();
        let finish = self.finish.unwrap_or_else(|| {
            // The stream ended without a terminal event. A lenient server
            // that just stops after finishing its calls is tolerated, but
            // only if every call's arguments actually parse: a truncated
            // call must surface as an error, never execute half-written.
            let calls_complete = !tool_calls.is_empty()
                && tool_calls
                    .iter()
                    .all(|c| serde_json::from_str::<Value>(&c.arguments).is_ok());
            if calls_complete {
                Finish::ToolCalls
            } else {
                Finish::Error("the stream ended before the response completed".to_string())
            }
        });
        on(Event::Done(finish.clone()));
        Turn {
            text: self.text,
            reasoning: if self.reasoning.is_empty() { None } else { Some(self.reasoning) },
            tool_calls,
            usage: self.usage,
            finish,
            raw_items: self.raw_items,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn drive(events: &[(&str, Value)]) -> (Turn, Vec<Event>) {
        let mut out = Vec::new();
        let mut state = State::default();
        for (kind, payload) in events {
            state.on_event(kind, payload, &mut |e| out.push(e));
        }
        let turn = state.finish(&mut |e| out.push(e));
        (turn, out)
    }

    #[test]
    fn function_call_session_start_deltas_done_completed() {
        let (turn, events) = drive(&[
            ("response.created", json!({"type": "response.created"})),
            (
                "response.output_item.added",
                json!({"item": {"id": "fc_1", "type": "function_call",
                    "call_id": "call_1", "name": "read", "arguments": ""}}),
            ),
            (
                "response.function_call_arguments.delta",
                json!({"item_id": "fc_1", "delta": "{\"path\":"}),
            ),
            (
                "response.function_call_arguments.delta",
                json!({"item_id": "fc_1", "delta": "\"a\"}"}),
            ),
            (
                "response.output_item.done",
                json!({"item": {"id": "fc_1", "type": "function_call", "status": "completed",
                    "call_id": "call_1", "name": "read", "arguments": "{\"path\":\"a\"}"}}),
            ),
            (
                "response.completed",
                json!({"response": {"status": "completed",
                    "output": [{"id": "fc_1", "type": "function_call", "status": "completed",
                        "call_id": "call_1", "name": "read", "arguments": "{\"path\":\"a\"}"}],
                    "usage": {"input_tokens": 100, "output_tokens": 9,
                        "input_tokens_details": {"cached_tokens": 80}}}}),
            ),
        ]);
        assert_eq!(turn.finish, Finish::ToolCalls);
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].id, "call_1");
        assert_eq!(turn.tool_calls[0].arguments, "{\"path\":\"a\"}");
        assert_eq!(turn.usage.unwrap().cached_prompt_tokens, 80);
        // raw_items are the completed output array, replayable verbatim.
        assert_eq!(turn.raw_items.len(), 1);
        assert_eq!(turn.raw_items[0]["call_id"], "call_1");
        assert!(matches!(events[0], Event::ToolCallStart { .. }));
    }

    #[test]
    fn text_stream_and_usage() {
        let (turn, _) = drive(&[
            ("response.output_text.delta", json!({"item_id": "m1", "delta": "hel"})),
            ("response.output_text.delta", json!({"item_id": "m1", "delta": "lo"})),
            (
                "response.completed",
                json!({"response": {"output": [{"id": "m1", "type": "message",
                    "content": [{"type": "output_text", "text": "hello"}]}],
                    "usage": {"input_tokens": 5, "output_tokens": 2}}}),
            ),
        ]);
        assert_eq!(turn.text, "hello");
        assert_eq!(turn.finish, Finish::Stop);
    }

    #[test]
    fn reasoning_deltas_and_verbatim_reasoning_replay() {
        let (turn, _) = drive(&[
            ("response.reasoning_text.delta", json!({"delta": "think"})),
            (
                "response.completed",
                json!({"response": {"output": [
                    {"id": "rs_1", "type": "reasoning",
                     "encrypted_content": "opaque-bytes", "summary": []},
                    {"id": "m1", "type": "message",
                     "content": [{"type": "output_text", "text": "hi"}]}]}}),
            ),
        ]);
        assert_eq!(turn.reasoning.as_deref(), Some("think"));
        // The reasoning item survives verbatim for the next request.
        assert_eq!(turn.raw_items[0]["type"], "reasoning");
        assert_eq!(turn.raw_items[0]["encrypted_content"], "opaque-bytes");
    }

    #[test]
    fn arguments_done_is_authoritative_over_deltas() {
        let (turn, _) = drive(&[
            (
                "response.output_item.added",
                json!({"item": {"id": "fc_1", "type": "function_call",
                    "call_id": "c", "name": "read", "arguments": ""}}),
            ),
            (
                "response.function_call_arguments.delta",
                json!({"item_id": "fc_1", "delta": "{\"partial"}),
            ),
            (
                "response.function_call_arguments.done",
                json!({"item_id": "fc_1", "arguments": "{\"path\":\"full\"}"}),
            ),
        ]);
        assert_eq!(turn.tool_calls[0].arguments, "{\"path\":\"full\"}");
    }

    #[test]
    fn failed_and_incomplete_and_error_events() {
        let (turn, _) = drive(&[(
            "response.failed",
            json!({"response": {"error": {"message": "boom"}}}),
        )]);
        assert!(matches!(turn.finish, Finish::Error(ref m) if m == "boom"));

        let (turn, _) = drive(&[(
            "response.incomplete",
            json!({"response": {"incomplete_details": {"reason": "content_filter"}}}),
        )]);
        assert_eq!(turn.finish, Finish::ContentFilter);

        let (turn, _) = drive(&[("error", json!({"message": "in-band"}))]);
        assert!(matches!(turn.finish, Finish::Error(ref m) if m == "in-band"));
    }

    #[test]
    fn unknown_event_types_ignored() {
        let (turn, _) = drive(&[
            ("response.new_shiny_thing", json!({"whatever": true})),
            ("response.output_text.delta", json!({"delta": "ok"})),
        ]);
        assert_eq!(turn.text, "ok");
    }

    #[test]
    fn stream_death_before_completed_is_a_typed_finish() {
        let (turn, _) = drive(&[("response.output_text.delta", json!({"delta": "par"}))]);
        assert!(matches!(turn.finish, Finish::Error(_)));
        assert_eq!(turn.text, "par");
    }

    #[test]
    fn truncated_call_without_terminal_event_is_an_error_not_toolcalls() {
        // Stream dies mid-arguments: the half-written call must never be
        // reported as a valid ToolCalls turn.
        let (turn, _) = drive(&[
            (
                "response.output_item.added",
                json!({"item": {"id": "fc_1", "type": "function_call",
                    "call_id": "c1", "name": "read", "arguments": ""}}),
            ),
            (
                "response.function_call_arguments.delta",
                json!({"item_id": "fc_1", "delta": "{\"path\":\"/wo"}),
            ),
        ]);
        assert!(matches!(turn.finish, Finish::Error(_)), "finish: {:?}", turn.finish);

        // A lenient server that stops after COMPLETE calls (arguments
        // parse) but never sends response.completed is tolerated.
        let (turn, _) = drive(&[
            (
                "response.output_item.added",
                json!({"item": {"id": "fc_1", "type": "function_call",
                    "call_id": "c1", "name": "read", "arguments": ""}}),
            ),
            (
                "response.function_call_arguments.done",
                json!({"item_id": "fc_1", "arguments": "{\"path\":\"/work\"}"}),
            ),
        ]);
        assert_eq!(turn.finish, Finish::ToolCalls);
    }

    #[test]
    fn encrypted_reasoning_include_only_for_openai() {
        let req = TurnRequest {
            system: None,
            items: vec![Item::User("hi".into())],
            tools: vec![],
        };
        let openai = Endpoint {
            base_url: "https://api.openai.com/v1".into(),
            api_key: String::new(),
            model: "m".into(),
            style: crate::types::ApiStyle::Responses,
        };
        let body = build_body(&openai, &req);
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);

        let local = Endpoint { base_url: "http://localhost:8090/v1".into(), ..openai };
        let body = build_body(&local, &req);
        assert!(body.get("include").is_none(), "include only for api.openai.com");
    }

    #[test]
    fn guard_path_failed_and_incomplete_statuses_are_typed() {
        // The content-type-guard path feeds a whole response object in;
        // its status must not be mistaken for success.
        let mut state = State::default();
        state.on_completed_response(
            &json!({"status": "failed", "error": {"message": "boom"}, "output": []}),
            &mut |_| {},
        );
        let turn = state.finish(&mut |_| {});
        assert!(matches!(turn.finish, Finish::Error(ref m) if m == "boom"));

        let mut state = State::default();
        state.on_completed_response(
            &json!({"status": "incomplete",
                "incomplete_details": {"reason": "content_filter"}, "output": []}),
            &mut |_| {},
        );
        let turn = state.finish(&mut |_| {});
        assert_eq!(turn.finish, Finish::ContentFilter);
    }

    #[test]
    fn synthesized_call_id_is_patched_into_the_captured_raw_item() {
        // Server omits call_id: the ToolCall gets a synthesized id, and the
        // captured raw item must carry the SAME id or the replayed
        // transcript pairs a function_call_output with no function_call.
        let (turn, _) = drive(&[
            (
                "response.output_item.added",
                json!({"item": {"id": "fc_1", "type": "function_call",
                    "name": "read", "arguments": ""}}),
            ),
            (
                "response.function_call_arguments.delta",
                json!({"item_id": "fc_1", "delta": "{}"}),
            ),
            (
                "response.completed",
                json!({"response": {"status": "completed", "output": [
                    {"id": "fc_1", "type": "function_call",
                     "name": "read", "arguments": "{}"}]}}),
            ),
        ]);
        let id = &turn.tool_calls[0].id;
        assert!(id.starts_with("call_resp_"), "{id}");
        assert_eq!(turn.raw_items[0]["call_id"], json!(id.as_str()));
    }

    #[test]
    fn input_serialization_replays_raw_items_verbatim() {
        let raw = vec![
            json!({"id": "rs_1", "type": "reasoning", "encrypted_content": "blob"}),
            json!({"id": "fc_1", "type": "function_call", "call_id": "c1",
                "name": "read", "arguments": "{}"}),
        ];
        let req = TurnRequest {
            system: Some("sys".into()),
            items: vec![
                Item::User("go".into()),
                Item::Assistant {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "read".into(),
                        arguments: "{}".into(),
                    }],
                    raw_items: raw.clone(),
                },
                Item::ToolResult { call_id: "c1".into(), content: "out".into() },
            ],
            tools: vec![],
        };
        let input = build_input(&req);
        assert_eq!(input[0], json!({"type": "message", "role": "user", "content": "go"}));
        assert_eq!(input[1], raw[0]);
        assert_eq!(input[2], raw[1]);
        assert_eq!(
            input[3],
            json!({"type": "function_call_output", "call_id": "c1", "output": "out"})
        );
    }

    #[test]
    fn input_serialization_reconstructs_when_no_raw_items() {
        let req = TurnRequest {
            system: None,
            items: vec![Item::Assistant {
                text: "did it".into(),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "bash".into(),
                    arguments: "{\"command\":\"ls\"}".into(),
                }],
                raw_items: vec![],
            }],
            tools: vec![],
        };
        let input = build_input(&req);
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "assistant");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "c1");
    }
}
