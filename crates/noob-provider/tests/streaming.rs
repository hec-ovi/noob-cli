//! Streaming-path behavior matrix against the mock server: content-type
//! guard, retry/backoff, Retry-After, the no-retry-after-content rule, the
//! 400 field-strip compat, mid-stream interrupts, and the named invariant
//! tests `hot_reload_env` and `no_output_cap`.

use std::time::{Duration, Instant};

use noob_provider::http::{Client, RetryPolicy, Timeouts};
use noob_provider::types::{
    ApiStyle, Endpoint, Event, Finish, Item, Overrides, ProviderError, ToolSpec, TurnRequest,
};
use noob_provider::{chat, responses, run_turn};
use noob_testkit::{MockServer, RawStep, chat_stream_datas, chunked_sse_response, sse_headers};
use serde_json::json;

fn endpoint(server: &MockServer, style: ApiStyle) -> Endpoint {
    Endpoint {
        base_url: server.base_url(),
        api_key: String::new(),
        model: "m".to_string(),
        style,
    }
}

fn user_turn(text: &str) -> TurnRequest {
    TurnRequest { system: None, items: vec![Item::User(text.to_string())], tools: vec![] }
}

fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        delays: vec![Duration::from_millis(5); 3],
        jitter: false,
    }
}

#[test]
fn streamed_text_arrives_as_ordered_deltas() {
    let server = MockServer::start();
    server.enqueue_stream_completion("the quick brown fox");
    let client = Client::new(Timeouts::default());

    let mut deltas: Vec<String> = Vec::new();
    let turn = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &user_turn("go"),
        &mut |e| {
            if let Event::Text(t) = e {
                deltas.push(t);
            }
        },
    )
    .unwrap();

    assert_eq!(turn.text, "the quick brown fox");
    assert_eq!(deltas.concat(), "the quick brown fox");
    assert!(deltas.len() > 1, "must stream in pieces, got {deltas:?}");
    assert_eq!(turn.finish, Finish::Stop);
    assert!(turn.usage.is_some());
    server.assert_clean();
}

/// A 200 with application/json answering a stream:true request is a whole
/// completion, not an SSE stream (several proxies and older servers).
#[test]
fn json_answer_to_streamed_request_is_parsed_not_fed_to_sse() {
    let server = MockServer::start();
    server.enqueue_completion("plain json answer");
    let client = Client::new(Timeouts::default());

    let mut events = Vec::new();
    let turn = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &user_turn("go"),
        &mut |e| events.push(e),
    )
    .unwrap();

    assert_eq!(turn.text, "plain json answer");
    // The guard path still produces the uniform event stream.
    assert!(events.iter().any(|e| matches!(e, Event::Text(t) if t == "plain json answer")));
    assert!(events.iter().any(|e| matches!(e, Event::Done(Finish::Stop))));
    server.assert_clean();
}

#[test]
fn whole_json_completion_is_not_length_capped() {
    let server = MockServer::start();
    let answer = "x".repeat(8 * 1024 * 1024 + 1);
    server.enqueue_completion(&answer);
    let client = Client::new(Timeouts::default());

    let turn = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &user_turn("go"),
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(turn.text.len(), answer.len());
    assert_eq!(turn.text, answer);
    server.assert_clean();
}

#[test]
fn retryable_5xx_retries_then_succeeds() {
    let server = MockServer::start();
    server.enqueue_json(500, json!({"error": "transient"}));
    server.enqueue_json(503, json!({"error": "still transient"}));
    server.enqueue_stream_completion("third time lucky");
    let client = Client::with_retry(Timeouts::default(), fast_retry());

    let turn = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &user_turn("go"),
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(turn.text, "third time lucky");
    assert_eq!(server.recorded().len(), 3);
    server.assert_clean();
}

#[test]
fn retry_exhaustion_surfaces_the_http_error() {
    let server = MockServer::start();
    for _ in 0..4 {
        server.enqueue_json(502, json!({"error": "dead upstream"}));
    }
    let client = Client::with_retry(Timeouts::default(), fast_retry());

    let err = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &user_turn("go"),
        &mut |_| {},
    )
    .unwrap_err();

    match err {
        ProviderError::Http { status, body } => {
            assert_eq!(status, 502);
            assert!(body.contains("dead upstream"));
        }
        other => panic!("expected Http, got {other:?}"),
    }
    // Initial attempt + one per backoff slot, and no more.
    assert_eq!(server.recorded().len(), 4);
    server.assert_clean();
}

#[test]
fn retry_after_header_wins_over_the_backoff_schedule() {
    let server = MockServer::start();
    let body = br#"{"error":"slow down"}"#;
    let mut resp = format!(
        "HTTP/1.1 429 MOCK\r\ncontent-type: application/json\r\nretry-after: 1\r\ncontent-length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    resp.extend_from_slice(body);
    server.enqueue_raw(vec![RawStep::Bytes(resp)]);
    server.enqueue_stream_completion("after the pause");
    // Schedule says 5 ms; the header says 1 s. The header must win.
    let client = Client::with_retry(Timeouts::default(), fast_retry());

    let start = Instant::now();
    let turn = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &user_turn("go"),
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(turn.text, "after the pause");
    assert!(start.elapsed() >= Duration::from_millis(900), "took {:?}", start.elapsed());
    assert_eq!(server.recorded().len(), 2);
    server.assert_clean();
}

/// Once content has streamed, a stall is a turn error, never a silent
/// retry: a retry after output would duplicate it.
#[test]
fn no_retry_after_the_first_content_byte() {
    let server = MockServer::start();
    let delta = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"par\"}}]}\n\n";
    server.enqueue_raw(vec![
        RawStep::Bytes(sse_headers()),
        RawStep::Bytes(delta.as_bytes().to_vec()),
        RawStep::SleepMs(5_000),
    ]);
    let client = Client::with_retry(
        Timeouts {
            connect: Duration::from_secs(5),
            first_byte: Duration::from_secs(10),
            idle: Duration::from_secs(1),
        },
        fast_retry(),
    );

    let mut text = String::new();
    let err = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &user_turn("go"),
        &mut |e| {
            if let Event::Text(t) = e {
                text.push_str(&t);
            }
        },
    )
    .unwrap_err();

    assert!(
        matches!(err, ProviderError::Timeout(_)),
        "expected a typed timeout, got {err:?}"
    );
    assert_eq!(text, "par", "partial output was delivered before the stall");
    assert_eq!(server.recorded().len(), 1, "mid-stream death must not retry");
    server.assert_clean();
}

/// Reactive compat: a 400 naming a strippable field we sent gets one
/// immediate retry without it, and the client remembers for its lifetime.
#[test]
fn compat_400_strips_the_named_field_and_remembers() {
    let server = MockServer::start();
    server.enqueue_json(400, json!({"error": {
        "message": "Unknown parameter: 'stream_options'", "type": "invalid_request_error"}}));
    server.enqueue_stream_completion("works without it");
    server.enqueue_stream_completion("second turn");
    let client = Client::new(Timeouts::default());

    let ep = endpoint(&server, ApiStyle::Chat);
    let turn = chat::stream(&client, &ep, &user_turn("go"), &mut |_| {}).unwrap();
    assert_eq!(turn.text, "works without it");
    let turn2 = chat::stream(&client, &ep, &user_turn("go"), &mut |_| {}).unwrap();
    assert_eq!(turn2.text, "second turn");

    let recorded = server.recorded();
    assert_eq!(recorded.len(), 3);
    assert!(
        recorded[0].json().unwrap().get("stream_options").is_some(),
        "first attempt sends the field"
    );
    assert!(
        recorded[1].json().unwrap().get("stream_options").is_none(),
        "the strip-retry drops it"
    );
    assert!(
        recorded[2].json().unwrap().get("stream_options").is_none(),
        "the client remembers for its lifetime"
    );
    server.assert_clean();
}

/// A 400 that does NOT name anything strippable surfaces immediately.
#[test]
fn ordinary_400_is_not_retried() {
    let server = MockServer::start();
    server.enqueue_json(400, json!({"error": "model not found"}));
    let client = Client::new(Timeouts::default());

    let err = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &user_turn("go"),
        &mut |_| {},
    )
    .unwrap_err();

    assert!(matches!(err, ProviderError::Http { status: 400, .. }), "{err:?}");
    assert_eq!(server.recorded().len(), 1);
    server.assert_clean();
}

#[test]
fn interrupt_mid_stream_aborts_with_partial_output() {
    let server = MockServer::start();
    let delta = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"begin\"}}]}\n\n";
    server.enqueue_raw(vec![
        RawStep::Bytes(sse_headers()),
        RawStep::Bytes(delta.as_bytes().to_vec()),
        RawStep::SleepMs(10_000),
    ]);
    let client = Client::new(Timeouts::default());
    let ctl = client.ctl();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        ctl.interrupt();
    });

    let start = Instant::now();
    let mut text = String::new();
    let err = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &user_turn("go"),
        &mut |e| {
            if let Event::Text(t) = e {
                text.push_str(&t);
            }
        },
    )
    .unwrap_err();

    assert!(matches!(err, ProviderError::Interrupted), "{err:?}");
    assert_eq!(text, "begin");
    assert!(start.elapsed() < Duration::from_millis(2600), "took {:?}", start.elapsed());
    server.assert_clean();
}

/// Chunked SSE (the framing real servers use) with the connection kept
/// alive: consuming past [DONE] must return the connection to the pool, so
/// two turns share one TCP connection instead of paying a handshake each.
#[test]
fn chat_stream_reuses_the_connection_across_turns() {
    let server = MockServer::start();
    for text in ["turn one", "turn two"] {
        let datas = chat_stream_datas(text);
        let refs: Vec<&str> = datas.iter().map(String::as_str).collect();
        server.enqueue_raw_keepalive(vec![RawStep::Bytes(chunked_sse_response(&refs))]);
    }
    let client = Client::new(Timeouts::default());
    let ep = endpoint(&server, ApiStyle::Chat);

    let t1 = chat::stream(&client, &ep, &user_turn("go"), &mut |_| {}).unwrap();
    let t2 = chat::stream(&client, &ep, &user_turn("go"), &mut |_| {}).unwrap();

    assert_eq!(t1.text, "turn one");
    assert_eq!(t2.text, "turn two");
    assert_eq!(server.recorded().len(), 2);
    assert_eq!(server.connection_count(), 1, "keep-alive must reuse the connection");
    server.assert_clean();
}

/// When the first body bytes arrive in the same TCP segment as the headers
/// (KV-cached prompts answer fast; TLS coalesces records), the idle clock
/// must still engage: a mid-stream stall trips Idle on the idle budget,
/// not FirstByte on the much larger first-byte budget.
#[test]
fn idle_clock_engages_when_body_arrives_with_the_headers() {
    let server = MockServer::start();
    let mut first_write = sse_headers();
    first_write.extend_from_slice(
        b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"par\"}}]}\n\n",
    );
    server.enqueue_raw(vec![RawStep::Bytes(first_write), RawStep::SleepMs(6_000)]);
    let client = Client::with_retry(
        Timeouts {
            connect: Duration::from_secs(5),
            // Larger than the whole scripted stall: if the phase never
            // leaves AwaitFirstByte, nothing trips and the test fails.
            first_byte: Duration::from_secs(30),
            idle: Duration::from_secs(1),
        },
        RetryPolicy::none(),
    );

    let start = Instant::now();
    let err = chat::stream(
        &client,
        &endpoint(&server, ApiStyle::Chat),
        &user_turn("go"),
        &mut |_| {},
    )
    .unwrap_err();

    assert!(
        matches!(err, ProviderError::Timeout(noob_provider::types::TimeoutKind::Idle)),
        "expected Idle, got {err:?}"
    );
    assert!(start.elapsed() < Duration::from_secs(4), "took {:?}", start.elapsed());
    server.assert_clean();
}

/// Responses adapter end to end over the mock, including the request body
/// invariants (store:false, streamed, flattened tools, replayed transcript).
#[test]
fn responses_round_trip_with_tool_result_replay() {
    let server = MockServer::start();
    server.enqueue_sse(&[
        r#"{"type":"response.created","response":{"id":"r1"}}"#,
        r#"{"type":"response.output_item.added","item":{"id":"m1","type":"message","role":"assistant","content":[]}}"#,
        r#"{"type":"response.output_text.delta","item_id":"m1","delta":"file says hi"}"#,
        r#"{"type":"response.completed","response":{"status":"completed","output":[{"id":"m1","type":"message","role":"assistant","content":[{"type":"output_text","text":"file says hi"}]}],"usage":{"input_tokens":50,"output_tokens":4,"input_tokens_details":{"cached_tokens":40}}}}"#,
    ]);
    let client = Client::new(Timeouts::default());

    let raw_call = json!({"id": "fc_1", "type": "function_call",
        "call_id": "call_1", "name": "read", "arguments": "{\"path\":\"x\"}"});
    let req = TurnRequest {
        system: Some("sys".to_string()),
        items: vec![
            Item::User("read x".to_string()),
            Item::Assistant {
                text: String::new(),
                tool_calls: vec![],
                raw_items: vec![raw_call.clone()],
            },
            Item::ToolResult { call_id: "call_1".to_string(), content: "hi".to_string() },
        ],
        tools: vec![ToolSpec {
            name: "read".to_string(),
            description: "read".to_string(),
            parameters: json!({"type": "object"}),
        }],
    };
    let turn = responses::stream(
        &client,
        &endpoint(&server, ApiStyle::Responses),
        &req,
        &mut |_| {},
    )
    .unwrap();

    assert_eq!(turn.text, "file says hi");
    assert_eq!(turn.finish, Finish::Stop);
    assert_eq!(turn.usage.unwrap().cached_prompt_tokens, 40);

    let body = server.recorded()[0].json().unwrap();
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["input"][1], raw_call, "captured wire item replayed verbatim");
    assert_eq!(body["input"][2]["type"], "function_call_output");
    server.assert_clean();
}

/// Named invariant test: `.env` is re-read on every request, so a key
/// rotation applies to the very next call with no restart.
#[test]
fn hot_reload_env() {
    let server = MockServer::start();
    server.enqueue_stream_completion("first");
    server.enqueue_stream_completion("second");
    let dir = tempfile::tempdir().unwrap();
    let env_path = dir.path().join(".env");
    std::fs::write(
        &env_path,
        format!("NOOB_BASE_URL={}\nNOOB_API_KEY=key-one\n", server.base_url()),
    )
    .unwrap();
    let client = Client::new(Timeouts::default());
    let ov = Overrides::default();

    run_turn(&client, dir.path(), &ov, &user_turn("go"), &mut |_| {}).unwrap();
    std::fs::write(
        &env_path,
        format!("NOOB_BASE_URL={}\nNOOB_API_KEY=key-two\n", server.base_url()),
    )
    .unwrap();
    run_turn(&client, dir.path(), &ov, &user_turn("go"), &mut |_| {}).unwrap();

    let recorded = server.recorded();
    assert_eq!(recorded[0].header("authorization"), Some("Bearer key-one"));
    assert_eq!(recorded[1].header("authorization"), Some("Bearer key-two"));
    server.assert_clean();
}

/// Named invariant test: no request from either adapter ever carries a
/// max_tokens-family key, with tools, without tools, on any turn.
#[test]
fn no_output_cap() {
    let server = MockServer::start();
    server.enqueue_stream_completion("chat answer");
    server.enqueue_sse(&[
        r#"{"type":"response.output_text.delta","item_id":"m1","delta":"resp answer"}"#,
        r#"{"type":"response.completed","response":{"status":"completed","output":[]}}"#,
    ]);
    let client = Client::new(Timeouts::default());

    let mut req = user_turn("go");
    req.tools = vec![ToolSpec {
        name: "bash".to_string(),
        description: "run a command".to_string(),
        parameters: json!({"type": "object",
            "properties": {"command": {"type": "string"}}}),
    }];
    chat::stream(&client, &endpoint(&server, ApiStyle::Chat), &req, &mut |_| {}).unwrap();
    responses::stream(&client, &endpoint(&server, ApiStyle::Responses), &req, &mut |_| {})
        .unwrap();

    // The mock's automatic scan is the real teeth; make the intent explicit
    // here too by scanning the recorded bodies ourselves.
    for rec in server.recorded() {
        let body_text = String::from_utf8_lossy(&rec.body).to_ascii_lowercase();
        assert!(!body_text.contains("max_tokens"), "{body_text}");
        assert!(!body_text.contains("max_completion_tokens"), "{body_text}");
        assert!(!body_text.contains("max_output_tokens"), "{body_text}");
    }
    server.assert_clean();
}
