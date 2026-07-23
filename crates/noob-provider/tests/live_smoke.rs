//! Live smoke against local endpoints. Opt-in: `./dev.sh smoke` runs
//! `cargo test -- --ignored` with host networking (`./dev.sh smoke`).
//!
//! This is PLAN's top P1 risk gate: qwen tool-calling through the
//! llama.cpp jinja template, driven by the real adapters, both wire
//! shapes, including the tool-result replay leg.

use noob_provider::http::{Client, Timeouts};
use noob_provider::types::{
    ApiStyle, Endpoint, Event, Finish, Item, ToolCall, ToolSpec, TurnRequest,
};
use noob_provider::{chat, responses};

fn live_endpoint(style: ApiStyle) -> Endpoint {
    let base_url = std::env::var("NOOB_LIVE_BASE_URL")
        .unwrap_or_else(|_| "http://localhost:8090/v1".to_string());
    Endpoint {
        base_url,
        api_key: "noauth".to_string(),
        model: std::env::var("NOOB_LIVE_MODEL").unwrap_or_else(|_| "qwen3.6-35b-a3b".to_string()),
        style,
    }
}

fn read_tool() -> ToolSpec {
    ToolSpec {
        name: "read".to_string(),
        description: "Read a file from disk".to_string(),
        parameters: serde_json::json!({"type": "object",
            "properties": {"path": {"type": "string", "description": "absolute path"}},
            "required": ["path"]}),
    }
}

fn first_leg() -> TurnRequest {
    TurnRequest {
        system: Some("You are noob, a coding agent. Use the provided tools to act.".to_string()),
        items: vec![Item::User("Read the file /work/hello.txt".to_string())],
        tools: vec![read_tool()],
    }
}

fn second_leg(call: &ToolCall, raw_items: Vec<serde_json::Value>) -> TurnRequest {
    let mut req = first_leg();
    req.items.push(Item::Assistant {
        text: String::new(),
        tool_calls: vec![call.clone()],
        raw_items,
    });
    req.items.push(Item::ToolResult {
        call_id: call.id.clone(),
        content: "hello from noob\n".to_string(),
    });
    req
}

/// Chat shape: streamed tool call out, tool result back through the jinja
/// template, final answer built on the result.
#[test]
#[ignore = "live: requires qwen at :8090 (./dev.sh smoke)"]
fn live_chat_toolcall_roundtrip() {
    let client = Client::new(Timeouts::default());
    let ep = live_endpoint(ApiStyle::Chat);

    let mut saw_args_delta = false;
    let turn = chat::stream(&client, &ep, &first_leg(), &mut |e| {
        if matches!(e, Event::ToolArgsDelta { .. }) {
            saw_args_delta = true;
        }
    })
    .expect("first leg against live qwen");
    assert_eq!(turn.finish, Finish::ToolCalls, "turn: {turn:?}");
    assert_eq!(turn.tool_calls.len(), 1, "turn: {turn:?}");
    let call = &turn.tool_calls[0];
    assert_eq!(call.name, "read");
    assert!(!call.id.is_empty());
    assert!(saw_args_delta, "llama.cpp streams argument fragments");
    let args: serde_json::Value =
        serde_json::from_str(&call.arguments).expect("arguments must be valid JSON after finish()");
    assert_eq!(args["path"], "/work/hello.txt");

    let turn2 = chat::stream(&client, &ep, &second_leg(call, vec![]), &mut |_| {})
        .expect("tool-result replay through the jinja template");
    assert_eq!(turn2.finish, Finish::Stop, "turn2: {turn2:?}");
    assert!(
        turn2.text.to_lowercase().contains("hello"),
        "the answer should quote the file: {}",
        turn2.text
    );
    assert!(
        turn2.usage.is_some(),
        "usage chunk expected with include_usage"
    );
}

/// Parallel tool calls in one inference (indexes 0 and 1, distinct ids).
#[test]
#[ignore = "live: requires qwen at :8090 (./dev.sh smoke)"]
fn live_chat_parallel_toolcalls() {
    let client = Client::new(Timeouts::default());
    let ep = live_endpoint(ApiStyle::Chat);
    let req = TurnRequest {
        system: Some(
            "You are noob, a coding agent. Use the provided tools to act. When several \
             independent reads are needed, issue all the tool calls in one turn."
                .to_string(),
        ),
        items: vec![Item::User(
            "Read both /work/a.txt and /work/b.txt".to_string(),
        )],
        tools: vec![read_tool()],
    };

    let turn = chat::stream(&client, &ep, &req, &mut |_| {}).expect("live parallel calls");
    assert_eq!(turn.finish, Finish::ToolCalls, "turn: {turn:?}");
    assert_eq!(
        turn.tool_calls.len(),
        2,
        "expected both reads in one turn: {turn:?}"
    );
    let ids: std::collections::HashSet<_> = turn.tool_calls.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids.len(), 2, "ids must be distinct");
    for c in &turn.tool_calls {
        assert!(serde_json::from_str::<serde_json::Value>(&c.arguments).is_ok());
    }
}

/// Responses shape against the same server (this llama.cpp build serves
/// /v1/responses): function call out, function_call_output back, final
/// answer; raw_items replayed verbatim.
#[test]
#[ignore = "live: requires qwen at :8090 (./dev.sh smoke)"]
fn live_responses_toolcall_roundtrip() {
    let client = Client::new(Timeouts::default());
    let ep = live_endpoint(ApiStyle::Responses);

    let turn = responses::stream(&client, &ep, &first_leg(), &mut |_| {})
        .expect("first leg against live /v1/responses");
    assert_eq!(turn.finish, Finish::ToolCalls, "turn: {turn:?}");
    assert_eq!(turn.tool_calls.len(), 1);
    let call = &turn.tool_calls[0];
    assert_eq!(call.name, "read");
    let args: serde_json::Value = serde_json::from_str(&call.arguments).unwrap();
    assert_eq!(args["path"], "/work/hello.txt");
    assert!(
        !turn.raw_items.is_empty(),
        "completed output captured for replay"
    );

    let turn2 = responses::stream(
        &client,
        &ep,
        &second_leg(call, turn.raw_items.clone()),
        &mut |_| {},
    )
    .expect("function_call_output replay");
    assert_eq!(turn2.finish, Finish::Stop, "turn2: {turn2:?}");
    assert!(
        turn2.text.to_lowercase().contains("hello"),
        "the answer should quote the file: {}",
        turn2.text
    );
}
