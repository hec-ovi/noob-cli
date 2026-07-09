//! Replay captured SSE byte transcripts (testdata/sse/*.sse) through the
//! real adapters, both purely (parser + assembler) and end to end (mock
//! server + Client + adapter). The %%CHUNK%% sentinels reproduce the
//! nastiest TCP splits deterministically: mid-`data:`, mid-`event:`,
//! mid-JSON-key, and mid-multibyte-codepoint.

use noob_provider::assemble::Assembler;
use noob_provider::chat;
use noob_provider::http::{Client, Timeouts};
use noob_provider::responses;
use noob_provider::sse::SseParser;
use noob_provider::types::{ApiStyle, Endpoint, Event, Finish, Item, ToolSpec, TurnRequest};
use noob_testkit::{MockServer, RawStep, load_fixture_chunks, sse_headers};

fn fixture(name: &str) -> Vec<Vec<u8>> {
    load_fixture_chunks(format!("{}/testdata/sse/{name}", env!("CARGO_MANIFEST_DIR")))
}

fn endpoint(server: &MockServer, style: ApiStyle) -> Endpoint {
    Endpoint {
        base_url: server.base_url(),
        api_key: String::new(),
        model: "qwen3.6-35b-a3b".to_string(),
        style,
    }
}

/// Serve the fixture chunks as one SSE response, each chunk its own write
/// with a small pause, so the client-side reads see the splits.
fn enqueue_fixture(server: &MockServer, name: &str) {
    let mut steps = vec![RawStep::Bytes(sse_headers())];
    for chunk in fixture(name) {
        steps.push(RawStep::Bytes(chunk));
        steps.push(RawStep::SleepMs(3));
    }
    server.enqueue_raw(steps);
}

fn read_request() -> TurnRequest {
    TurnRequest {
        system: Some("You are noob, a coding agent.".to_string()),
        items: vec![Item::User("Read the file /work/hello.txt".to_string())],
        tools: vec![ToolSpec {
            name: "read".to_string(),
            description: "Read a file from disk".to_string(),
            parameters: serde_json::json!({"type": "object",
                "properties": {"path": {"type": "string"}}, "required": ["path"]}),
        }],
    }
}

/// Real llama.cpp qwen capture: single streamed tool call, id in the first
/// delta, string argument fragments, usage chunk, [DONE].
#[test]
fn llamacpp_toolcall_fixture_end_to_end() {
    let server = MockServer::start();
    enqueue_fixture(&server, "llamacpp-qwen-toolcall.sse");
    let client = Client::new(Timeouts::default());

    let mut events = Vec::new();
    let turn = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &read_request(),
        &mut |e| events.push(e),
    )
    .unwrap();

    assert_eq!(turn.finish, Finish::ToolCalls);
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].id, "FJL6JI5993wALojEE60X2auup8PCLaui");
    assert_eq!(turn.tool_calls[0].name, "read");
    let args: serde_json::Value =
        serde_json::from_str(&turn.tool_calls[0].arguments).unwrap();
    assert_eq!(args["path"], "/work/hello.txt");
    let usage = turn.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 354);
    assert_eq!(usage.completion_tokens, 27);

    // The request wire shape: streamed, usage requested, tools nested,
    // never a cap key (assert_clean checks that one).
    let recorded = server.recorded();
    let body = recorded[0].json().unwrap();
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["function"]["name"], "read");
    assert!(body.get("parallel_tool_calls").is_none());
    server.assert_clean();
}

/// Real llama.cpp qwen capture: two tool calls in one turn (index 0 and 1),
/// distinct ids, both assembled in emission order.
#[test]
fn llamacpp_parallel_calls_fixture_end_to_end() {
    let server = MockServer::start();
    enqueue_fixture(&server, "llamacpp-qwen-parallel.sse");
    let client = Client::new(Timeouts::default());

    let mut starts = 0;
    let turn = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &read_request(),
        &mut |e| {
            if matches!(e, Event::ToolCallStart { .. }) {
                starts += 1;
            }
        },
    )
    .unwrap();

    assert_eq!(starts, 2);
    assert_eq!(turn.finish, Finish::ToolCalls);
    assert_eq!(turn.tool_calls.len(), 2);
    assert_eq!(turn.tool_calls[0].id, "wbRDiG7ivFr7xh7KEd9jTCwJnMWUFChi");
    assert_eq!(turn.tool_calls[1].id, "Epz3kcn9OOKO0OSnFztaT6BxkpNc9aXs");
    let a: serde_json::Value = serde_json::from_str(&turn.tool_calls[0].arguments).unwrap();
    let b: serde_json::Value = serde_json::from_str(&turn.tool_calls[1].arguments).unwrap();
    assert_eq!(a["path"], "/work/a.txt");
    assert_eq!(b["path"], "/work/b.txt");
    server.assert_clean();
}

/// Real llama.cpp /v1/responses capture: typed events, arguments deltas by
/// item_id, completed output captured verbatim for replay.
#[test]
fn llamacpp_responses_fixture_end_to_end() {
    let server = MockServer::start();
    enqueue_fixture(&server, "llamacpp-responses-toolcall.sse");
    let client = Client::new(Timeouts::default());

    let turn = responses::stream(
        &client,
        &endpoint(&server, ApiStyle::Responses),
        &read_request(),
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(turn.finish, Finish::ToolCalls);
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].id, "call_fLwarxp8xhBvVdMF1RdPmETSKQG2d3IJ");
    assert_eq!(turn.tool_calls[0].name, "read");
    assert_eq!(turn.tool_calls[0].arguments, "{\"path\":\"/work/hello.txt\"}");
    // raw_items are the authoritative completed output, replayable verbatim.
    assert_eq!(turn.raw_items.len(), 1);
    assert_eq!(turn.raw_items[0]["type"], "function_call");
    let usage = turn.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 293);

    let recorded = server.recorded();
    assert_eq!(recorded[0].path, "/v1/responses");
    let body = recorded[0].json().unwrap();
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["instructions"], "You are noob, a coding agent.");
    // Flattened tool shape, and no include off api.openai.com.
    assert_eq!(body["tools"][0]["name"], "read");
    assert!(body["tools"][0].get("function").is_none());
    assert!(body.get("include").is_none());
    server.assert_clean();
}

/// OpenRouter-shape stream: comment keepalives, multibyte content split
/// mid-codepoint across TCP chunks, then a mid-stream in-band error.
#[test]
fn openrouter_keepalive_and_midstream_error_fixture() {
    let server = MockServer::start();
    enqueue_fixture(&server, "openrouter-keepalive-error.sse");
    let client = Client::new(Timeouts::default());

    let mut text = String::new();
    let turn = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &read_request(),
        &mut |e| {
            if let Event::Text(t) = e {
                text.push_str(&t);
            }
        },
    )
    .unwrap();

    // The multibyte content reassembled exactly despite the codepoint splits.
    assert_eq!(text, "caf\u{00e9} \u{1F980} ok");
    assert_eq!(turn.text, text);
    assert!(
        matches!(turn.finish, Finish::Error(ref m) if m.contains("Provider returned error")),
        "finish: {:?}",
        turn.finish
    );
    server.assert_clean();
}

/// Parser + assembler property: re-splitting the real tool-call transcript
/// at EVERY byte offset yields the identical assembled turn.
#[test]
fn toolcall_fixture_resplit_at_every_byte_offset() {
    let full: Vec<u8> = fixture("llamacpp-qwen-toolcall.sse").concat();

    let assemble = |chunks: &[&[u8]]| {
        let mut parser = SseParser::new();
        let mut events = Vec::new();
        for c in chunks {
            parser.feed(c, &mut events);
        }
        parser.finish(&mut events);
        let mut asm = Assembler::new();
        for ev in &events {
            if ev.data.trim() == "[DONE]" {
                break;
            }
            let chunk: serde_json::Value = serde_json::from_str(&ev.data)
                .unwrap_or_else(|e| panic!("bad JSON in event {ev:?}: {e}"));
            asm.on_chunk(&chunk, &mut |_| {});
        }
        asm.finish(&mut |_| {})
    };

    let want = assemble(&[&full]);
    assert_eq!(want.tool_calls.len(), 1, "sanity: fixture assembles");
    for cut in 0..=full.len() {
        let got = assemble(&[&full[..cut], &full[cut..]]);
        assert_eq!(got.tool_calls.len(), 1, "cut at {cut}");
        assert_eq!(got.tool_calls[0].arguments, want.tool_calls[0].arguments, "cut at {cut}");
        assert_eq!(got.tool_calls[0].id, want.tool_calls[0].id, "cut at {cut}");
        assert_eq!(got.usage, want.usage, "cut at {cut}");
        assert_eq!(got.finish, want.finish, "cut at {cut}");
    }
}
