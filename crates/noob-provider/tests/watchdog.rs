//! P0 risk gate: prove the 1 s tick-read watchdog on the custom ureq
//! transport. Each test scripts raw bytes and sleeps on the mock server and
//! asserts the typed outcome plus rough wall-clock bounds.

use std::time::{Duration, Instant};

use noob_provider::http::{Client, RetryPolicy, Timeouts};
use noob_provider::types::{ProviderError, TimeoutKind};
use noob_testkit::{MockServer, RawStep, http_response};
use serde_json::json;

fn short_timeouts() -> Timeouts {
    Timeouts {
        connect: Duration::from_secs(5),
        first_byte: Duration::from_secs(3),
        idle: Duration::from_secs(2),
    }
}

fn body_steps(payload: &[u8], split_sleeps: &[(usize, u64)]) -> Vec<RawStep> {
    // Interleave payload slices with sleeps at the given byte offsets.
    let mut steps = vec![RawStep::Bytes(http_response(200, Some(payload.len())))];
    let mut start = 0;
    for &(offset, sleep_ms) in split_sleeps {
        steps.push(RawStep::Bytes(payload[start..offset].to_vec()));
        steps.push(RawStep::SleepMs(sleep_ms));
        start = offset;
    }
    steps.push(RawStep::Bytes(payload[start..].to_vec()));
    steps
}

/// The feasibility proof: reads resume across multiple 1 s socket-timeout
/// ticks and the body arrives intact.
#[test]
fn reads_resume_across_ticks() {
    let server = MockServer::start();
    let payload = br#"{"ok":true,"n":12345}"#;
    // Two stalls of 1.5 s each: every read crosses at least one tick.
    server.enqueue_raw(body_steps(payload, &[(5, 1500), (12, 1500)]));

    let client = Client::new(short_timeouts());
    let (status, bytes) = client
        .post_json(&server.url("/x"), "", &json!({}))
        .expect("dripped body must arrive intact");
    assert_eq!(status, 200);
    assert_eq!(bytes, payload);
}

/// Silence between body bytes beyond the idle budget trips a typed Idle
/// timeout, and does so promptly.
#[test]
fn idle_stall_trips_within_budget() {
    let server = MockServer::start();
    let payload = br#"{"ok":true}"#;
    // 2 bytes, then a stall far beyond idle=2s.
    server.enqueue_raw(body_steps(payload, &[(2, 8000)]));

    let client = Client::new(short_timeouts());
    let start = Instant::now();
    let err = client
        .post_json(&server.url("/x"), "", &json!({}))
        .unwrap_err();
    let elapsed = start.elapsed();
    assert!(
        matches!(err, ProviderError::Timeout(TimeoutKind::Idle)),
        "expected Idle timeout, got {err:?}"
    );
    assert!(elapsed < Duration::from_secs(6), "took {elapsed:?}");
}

/// No response bytes at all (server accepts, says nothing): FirstByte trip.
#[test]
fn header_silence_trips_first_byte() {
    let server = MockServer::start();
    server.enqueue_raw(vec![RawStep::SleepMs(8000)]);

    let client = Client::new(short_timeouts());
    let start = Instant::now();
    let err = client
        .post_json(&server.url("/x"), "", &json!({}))
        .unwrap_err();
    assert!(
        matches!(err, ProviderError::Timeout(TimeoutKind::FirstByte)),
        "expected FirstByte timeout, got {err:?}"
    );
    assert!(start.elapsed() < Duration::from_secs(6));
}

/// Headers arrive, then silence before the first body byte: still FirstByte,
/// not Idle. This is the llama.cpp prompt-processing window.
#[test]
fn body_silence_after_headers_trips_first_byte() {
    let server = MockServer::start();
    server.enqueue_raw(vec![
        RawStep::Bytes(http_response(200, Some(10))),
        RawStep::SleepMs(8000),
    ]);

    let client = Client::new(short_timeouts());
    let err = client
        .post_json(&server.url("/x"), "", &json!({}))
        .unwrap_err();
    assert!(
        matches!(err, ProviderError::Timeout(TimeoutKind::FirstByte)),
        "expected FirstByte timeout, got {err:?}"
    );
}

/// The idle clock must NOT run before the first body byte: a gap between
/// headers and body longer than the idle budget (but inside the first-byte
/// budget) succeeds.
#[test]
fn idle_clock_starts_only_at_first_body_byte() {
    let server = MockServer::start();
    let payload = br#"{"ok":1}"#;
    server.enqueue_raw(vec![
        RawStep::Bytes(http_response(200, Some(payload.len()))),
        // 2.5 s gap: longer than idle (2 s), shorter than first_byte (3 s).
        RawStep::SleepMs(2500),
        RawStep::Bytes(payload.to_vec()),
    ]);

    let client = Client::new(short_timeouts());
    let (status, bytes) = client
        .post_json(&server.url("/x"), "", &json!({}))
        .expect("gap before first body byte must fall under first_byte, not idle");
    assert_eq!(status, 200);
    assert_eq!(bytes, payload);
}

/// An interrupt from another thread aborts a silent request within about one
/// tick. This is the Ctrl-C responsiveness guarantee.
#[test]
fn interrupt_aborts_within_a_tick() {
    let server = MockServer::start();
    server.enqueue_raw(vec![
        RawStep::Bytes(http_response(200, Some(10))),
        RawStep::SleepMs(8000),
    ]);

    let client = Client::new(Timeouts {
        connect: Duration::from_secs(5),
        first_byte: Duration::from_secs(30),
        idle: Duration::from_secs(30),
    });
    let ctl = client.ctl();
    let start = Instant::now();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        ctl.interrupt();
    });
    let err = client
        .post_json(&server.url("/x"), "", &json!({}))
        .unwrap_err();
    let elapsed = start.elapsed();
    assert!(
        matches!(err, ProviderError::Interrupted),
        "expected Interrupted, got {err:?}"
    );
    assert!(elapsed < Duration::from_millis(2600), "took {elapsed:?}");
}

/// Nobody listening: typed connect error naming the URL. Connect errors are
/// retryable, so this uses a no-retry policy to assert the raw outcome.
#[test]
fn connection_refused_is_a_typed_connect_error() {
    let client = Client::with_retry(short_timeouts(), RetryPolicy::none());
    let err = client
        .post_json("http://127.0.0.1:9/x", "", &json!({}))
        .unwrap_err();
    match err {
        ProviderError::Connect(msg) => assert!(msg.contains("127.0.0.1:9"), "{msg}"),
        ProviderError::Timeout(TimeoutKind::Connect) => {}
        other => panic!("expected Connect error, got {other:?}"),
    }
}
