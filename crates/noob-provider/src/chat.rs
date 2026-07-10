//! Chat Completions adapter: SSE streaming, the delta assembler, and the
//! quirk matrix. Also keeps the non-streamed parser: a 200 answering
//! `application/json` to a `stream: true` request (several proxies and
//! older servers do this) is parsed as a single completion, not fed to the
//! SSE parser.

use serde_json::{Value, json};

use crate::assemble::Assembler;
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
    let url = format!("{}/chat/completions", ep.base_url);
    // Never any max_tokens-family key: output is never capped, and the mock
    // server fails any request carrying one. No parallel_tool_calls field
    // either: several OSS servers 400 on it.
    let mut body = json!({
        "model": ep.model,
        "messages": build_messages(req),
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    if !req.tools.is_empty() {
        body["tools"] = wire_tools(&req.tools);
    }

    let mut resp = client.post_json_stream(&url, &ep.api_key, &mut body)?;

    // Content-type guard: stream:true answered with a plain JSON completion.
    if resp.media_type() == "application/json" {
        let bytes = resp.read_to_end()?;
        let turn = parse_completion(&bytes)?;
        replay_turn(&turn, on);
        return Ok(turn);
    }

    let mut parser = SseParser::new();
    let mut asm = Assembler::new();
    let mut events = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    'read: loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            parser.finish(&mut events);
        } else {
            parser.feed(&buf[..n], &mut events);
        }
        for ev in events.drain(..) {
            if ev.data.trim() == "[DONE]" {
                break 'read;
            }
            let chunk: Value = serde_json::from_str(&ev.data).map_err(|e| {
                ProviderError::Wire(format!(
                    "invalid JSON in SSE data: {e}; first bytes: {}",
                    &ev.data.chars().take(120).collect::<String>()
                ))
            })?;
            asm.on_chunk(&chunk, on);
        }
        if n == 0 {
            break; // stream ended without [DONE]; finish() handles it
        }
    }
    // Consume the trailing bytes after [DONE] (normally just the chunked
    // terminator, already in flight) so the connection returns to ureq's
    // pool instead of being torn down every turn.
    resp.drain_for_reuse(std::time::Duration::from_millis(250));
    Ok(asm.finish(on))
}

/// Serialize the neutral transcript into chat messages. Reasoning is never
/// sent back (DeepSeek rejects it; llama.cpp templates re-inject thinking
/// themselves). Deterministic: byte-stable across identical inputs, which
/// is what the append-only prefix property rests on.
/// The Chat Completions wire shape of the tools array. Public so
/// `noob debug prompt --json` prints the exact serialized artifact the
/// budget tests measure, with no reimplementation drift.
pub fn wire_tools(tools: &[crate::types::ToolSpec]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                json!({"type": "function", "function": {
                    "name": t.name, "description": t.description,
                    "parameters": t.parameters}})
            })
            .collect(),
    )
}

fn build_messages(req: &TurnRequest) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(system) = &req.system {
        messages.push(json!({"role": "system", "content": system}));
    }
    for item in &req.items {
        match item {
            Item::User(text) => messages.push(json!({"role": "user", "content": text})),
            Item::Assistant { text, tool_calls, .. } => {
                let mut msg = json!({"role": "assistant"});
                msg["content"] = if text.is_empty() { Value::Null } else { json!(text) };
                if !tool_calls.is_empty() {
                    msg["tool_calls"] = Value::Array(
                        tool_calls
                            .iter()
                            .map(|c| {
                                json!({"id": c.id, "type": "function", "function": {
                                    "name": c.name, "arguments": c.arguments}})
                            })
                            .collect(),
                    );
                }
                messages.push(msg);
            }
            Item::ToolResult { call_id, content } => messages.push(json!({
                "role": "tool", "tool_call_id": call_id, "content": content
            })),
        }
    }
    messages
}

/// Emit the events a non-streamed completion would have produced, so the
/// caller sees one uniform event stream on the guard path too.
fn replay_turn(turn: &Turn, on: &mut dyn FnMut(Event)) {
    if let Some(r) = &turn.reasoning {
        on(Event::Reasoning(r.clone()));
    }
    if !turn.text.is_empty() {
        on(Event::Text(turn.text.clone()));
    }
    for (i, c) in turn.tool_calls.iter().enumerate() {
        on(Event::ToolCallStart { index: i as u32, id: c.id.clone(), name: c.name.clone() });
        on(Event::ToolArgsDelta { index: i as u32, delta: c.arguments.clone() });
    }
    if let Some(u) = turn.usage {
        on(Event::Usage(u));
    }
    on(Event::Done(turn.finish.clone()));
}

pub(crate) fn parse_completion(bytes: &[u8]) -> Result<Turn, ProviderError> {
    let v: Value = serde_json::from_slice(bytes)
        .map_err(|e| ProviderError::Wire(format!("invalid JSON completion: {e}")))?;
    let choice = v
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| ProviderError::Wire("completion has no choices[0]".to_string()))?;
    let msg = choice.get("message").cloned().unwrap_or(Value::Null);

    let text = msg
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // DeepSeek convention (llama.cpp, vLLM) and the OpenRouter variant.
    let reasoning = msg
        .get("reasoning_content")
        .or_else(|| msg.get("reasoning"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let mut tool_calls = Vec::new();
    if let Some(calls) = msg.get("tool_calls").and_then(Value::as_array) {
        for (i, call) in calls.iter().enumerate() {
            let f = call.get("function").cloned().unwrap_or(Value::Null);
            let arguments = match f.get("arguments") {
                Some(Value::String(s)) => s.clone(),
                // llama.cpp regression: arguments arrive as a JSON object.
                // Re-serialize canonically to a string.
                Some(obj @ Value::Object(_)) => obj.to_string(),
                _ => "{}".to_string(),
            };
            tool_calls.push(ToolCall {
                id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    // Some templates omit the id; a tool result needs one,
                    // and it must not collide across turns in a transcript.
                    .unwrap_or_else(|| crate::assemble::synth_call_id(i)),
                name: f
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                arguments,
            });
        }
    }

    let finish = match choice.get("finish_reason").and_then(Value::as_str) {
        Some("tool_calls") | Some("function_call") => Finish::ToolCalls,
        Some("length") => Finish::Length,
        Some("content_filter") => Finish::ContentFilter,
        _ if !tool_calls.is_empty() => Finish::ToolCalls,
        _ => Finish::Stop,
    };

    let usage = v.get("usage").and_then(|u| {
        Some(Usage {
            prompt_tokens: u.get("prompt_tokens")?.as_u64().unwrap_or(0),
            completion_tokens: u.get("completion_tokens").and_then(Value::as_u64).unwrap_or(0),
            cached_prompt_tokens: u
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
        })
    });

    Ok(Turn { text, reasoning, tool_calls, usage, finish, raw_items: Vec::new() })
}

#[cfg(test)]
mod tests {
    use super::{build_messages, parse_completion};
    use crate::types::{Finish, Item, ToolCall, TurnRequest};
    use serde_json::json;

    #[test]
    fn parses_plain_text_completion() {
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"hi"},
            "finish_reason":"stop"}],"usage":{"prompt_tokens":5,"completion_tokens":2}}"#;
        let turn = parse_completion(body.as_bytes()).unwrap();
        assert_eq!(turn.text, "hi");
        assert_eq!(turn.finish, Finish::Stop);
        assert_eq!(turn.usage.unwrap().prompt_tokens, 5);
    }

    #[test]
    fn arguments_object_reserialized_and_missing_id_synthesized() {
        let body = r#"{"choices":[{"message":{"content":null,"tool_calls":[
            {"function":{"name":"read","arguments":{"path":"a.txt"}}}]},
            "finish_reason":"tool_calls"}]}"#;
        let turn = parse_completion(body.as_bytes()).unwrap();
        assert_eq!(turn.finish, Finish::ToolCalls);
        assert!(turn.tool_calls[0].id.starts_with("call_"), "{}", turn.tool_calls[0].id);
        assert_eq!(turn.tool_calls[0].name, "read");
        let args: serde_json::Value = serde_json::from_str(&turn.tool_calls[0].arguments).unwrap();
        assert_eq!(args["path"], "a.txt");
        // Ids must never collide across turns within one transcript.
        let turn2 = parse_completion(body.as_bytes()).unwrap();
        assert_ne!(turn.tool_calls[0].id, turn2.tool_calls[0].id);
    }

    #[test]
    fn tool_calls_win_when_finish_reason_missing() {
        let body = r#"{"choices":[{"message":{"content":"","tool_calls":[
            {"id":"c1","function":{"name":"ls","arguments":"{}"}}]}}]}"#;
        let turn = parse_completion(body.as_bytes()).unwrap();
        assert_eq!(turn.finish, Finish::ToolCalls);
    }

    #[test]
    fn wire_error_on_garbage() {
        assert!(parse_completion(b"not json").is_err());
        assert!(parse_completion(br#"{"ok":true}"#).is_err());
    }

    #[test]
    fn transcript_serialization_matches_chat_wire_shape() {
        let req = TurnRequest {
            system: Some("be noob".into()),
            items: vec![
                Item::User("read a".into()),
                Item::Assistant {
                    text: String::new(),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "read".into(),
                        arguments: "{\"path\":\"a\"}".into(),
                    }],
                    raw_items: vec![],
                },
                Item::ToolResult { call_id: "c1".into(), content: "data".into() },
            ],
            tools: vec![],
        };
        let messages = build_messages(&req);
        assert_eq!(messages[0], json!({"role": "system", "content": "be noob"}));
        assert_eq!(messages[1], json!({"role": "user", "content": "read a"}));
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[2]["content"], serde_json::Value::Null);
        assert_eq!(messages[2]["tool_calls"][0]["id"], "c1");
        assert_eq!(messages[2]["tool_calls"][0]["function"]["arguments"], "{\"path\":\"a\"}");
        assert_eq!(
            messages[3],
            json!({"role": "tool", "tool_call_id": "c1", "content": "data"})
        );
    }

    #[test]
    fn serialization_is_deterministic() {
        let req = TurnRequest {
            system: Some("s".into()),
            items: vec![Item::User("u".into())],
            tools: vec![],
        };
        let a = serde_json::to_string(&build_messages(&req)).unwrap();
        let b = serde_json::to_string(&build_messages(&req)).unwrap();
        assert_eq!(a, b);
    }
}
