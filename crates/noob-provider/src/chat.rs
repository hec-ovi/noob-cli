//! Chat Completions adapter, P0 skeleton: one non-streamed turn.
//! P1 turns on SSE streaming, the delta assembler, and the full quirk matrix.

use serde_json::{Value, json};

use crate::http::Client;
use crate::types::{Endpoint, Finish, ProviderError, ToolCall, Turn, Usage};

pub fn complete(client: &Client, ep: &Endpoint, messages: &[Value]) -> Result<Turn, ProviderError> {
    let url = format!("{}/chat/completions", ep.base_url);
    // Never any max_tokens-family key: output is never capped, and the mock
    // server fails any request carrying one.
    let body = json!({
        "model": ep.model,
        "messages": messages,
        "stream": false,
    });
    let (status, bytes) = client.post_json(&url, &ep.api_key, &body)?;
    if !(200..300).contains(&status) {
        return Err(ProviderError::Http {
            status,
            body: String::from_utf8_lossy(&bytes).into_owned(),
        });
    }
    parse_completion(&bytes)
}

fn parse_completion(bytes: &[u8]) -> Result<Turn, ProviderError> {
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
                    // Some templates omit the id; a tool result needs one.
                    .unwrap_or_else(|| format!("call_0_{i}")),
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

    Ok(Turn { text, reasoning, tool_calls, usage, finish })
}

#[cfg(test)]
mod tests {
    use super::parse_completion;
    use crate::types::Finish;

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
        assert_eq!(turn.tool_calls[0].id, "call_0_0");
        assert_eq!(turn.tool_calls[0].name, "read");
        let args: serde_json::Value = serde_json::from_str(&turn.tool_calls[0].arguments).unwrap();
        assert_eq!(args["path"], "a.txt");
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
}
