//! Blocking HTTP transport with a 1 s tick-read watchdog.
//!
//! P0 risk gate, resolved: ureq's stock TcpTransport maps a socket read
//! timeout to a hard `Error::Timeout`, so it cannot resume across a tick.
//! We therefore ship the fallback named in ARCHITECTURE.md as the primary:
//! a custom transport on ureq's `unversioned` connector API (pinned =3.3.0)
//! that owns the TcpStream, reads with a fixed 1 s socket timeout, and
//! treats a timed-out read as a tick, not an error. Each tick checks the
//! interrupt flag, the first-byte deadline, and the idle deadline.
//!
//! Deadline model (three phases, driven by the caller):
//! - AwaitHeaders: from request send until response headers, budget = first_byte.
//! - AwaitFirstByte: from headers until the first body byte, budget = first_byte.
//!   llama.cpp legitimately spends minutes of silence here on a 131k-ctx
//!   prompt, which is why header bytes must not start the idle clock.
//! - Streaming: between body bytes, budget = idle, reset on every read.

use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ureq::Agent;
use ureq::config::Config;
use ureq::unversioned::resolver::DefaultResolver;
use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, LazyBuffers, NextTimeout, RustlsConnector, Transport,
};

use crate::types::{ProviderError, TimeoutKind};

/// Process-wide interrupt flag. noob's SIGINT handler sets it; the watchdog
/// checks it once per tick, which is what makes Ctrl-C responsive within
/// about a second even while the server is silent.
pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);

const TICK: Duration = Duration::from_secs(1);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    pub connect: Duration,
    pub first_byte: Duration,
    pub idle: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Timeouts {
            connect: Duration::from_secs(10),
            first_byte: Duration::from_secs(300),
            idle: Duration::from_secs(90),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trip {
    Interrupted,
    FirstByte,
    Idle,
    /// The server accepted the connection but stopped reading the request.
    SendStall,
}

#[derive(Debug)]
enum Phase {
    Idle,
    AwaitHeaders { deadline: Instant },
    AwaitFirstByte { deadline: Instant, idle: Duration },
    Streaming { deadline: Instant, idle: Duration },
}

#[derive(Debug)]
struct WdInner {
    local_interrupt: AtomicBool,
    phase: Mutex<Phase>,
    trip: Mutex<Option<Trip>>,
}

/// Shared between the caller (phase transitions) and the transport (ticks).
#[derive(Clone, Debug)]
pub struct WatchdogCtl {
    inner: Arc<WdInner>,
}

impl WatchdogCtl {
    fn new() -> Self {
        WatchdogCtl {
            inner: Arc::new(WdInner {
                local_interrupt: AtomicBool::new(false),
                phase: Mutex::new(Phase::Idle),
                trip: Mutex::new(None),
            }),
        }
    }

    /// Arm once per logical request, BEFORE the retry loop: clears the trip
    /// and any stale interrupt. Kept separate from `begin` so an interrupt
    /// arriving during a between-attempts backoff sleep is not silently
    /// cleared by the next attempt.
    fn arm(&self) {
        *self.inner.trip.lock().unwrap() = None;
        self.inner.local_interrupt.store(false, Ordering::SeqCst);
    }

    /// Start the deadline clock for one attempt.
    fn begin(&self, t: &Timeouts) {
        *self.inner.phase.lock().unwrap() = Phase::AwaitHeaders {
            deadline: Instant::now() + t.first_byte,
        };
    }

    fn interrupted(&self) -> bool {
        INTERRUPTED.load(Ordering::SeqCst) || self.inner.local_interrupt.load(Ordering::SeqCst)
    }

    fn body_started(&self, t: &Timeouts) {
        *self.inner.phase.lock().unwrap() = Phase::AwaitFirstByte {
            deadline: Instant::now() + t.first_byte,
            idle: t.idle,
        };
    }

    /// Short grace window for post-stream drain reads: whatever phase was
    /// active, allow at most `window` of further silence.
    fn begin_drain(&self, window: Duration) {
        *self.inner.phase.lock().unwrap() = Phase::Streaming {
            deadline: Instant::now() + window,
            idle: window,
        };
    }

    fn finish(&self) {
        *self.inner.phase.lock().unwrap() = Phase::Idle;
    }

    /// Abort the in-flight request from another thread (REPL Ctrl-C path,
    /// tests). Effective within one tick.
    pub fn interrupt(&self) {
        self.inner.local_interrupt.store(true, Ordering::SeqCst);
    }

    fn on_bytes(&self) {
        let mut phase = self.inner.phase.lock().unwrap();
        match *phase {
            Phase::AwaitFirstByte { idle, .. } | Phase::Streaming { idle, .. } => {
                *phase = Phase::Streaming { deadline: Instant::now() + idle, idle };
            }
            // Header bytes never advance the phase; the caller flips to
            // AwaitFirstByte once ureq hands back the response head.
            Phase::AwaitHeaders { .. } | Phase::Idle => {}
        }
    }

    fn check(&self) -> Option<Trip> {
        let trip = if self.interrupted() {
            Some(Trip::Interrupted)
        } else {
            let now = Instant::now();
            match *self.inner.phase.lock().unwrap() {
                Phase::AwaitHeaders { deadline } | Phase::AwaitFirstByte { deadline, .. }
                    if now >= deadline =>
                {
                    Some(Trip::FirstByte)
                }
                Phase::Streaming { deadline, .. } if now >= deadline => Some(Trip::Idle),
                _ => None,
            }
        };
        if trip.is_some() {
            *self.inner.trip.lock().unwrap() = trip;
        }
        trip
    }

    fn set_trip(&self, trip: Trip) {
        *self.inner.trip.lock().unwrap() = Some(trip);
    }

    fn take_trip(&self) -> Option<Trip> {
        self.inner.trip.lock().unwrap().take()
    }
}

fn trip_io_error(trip: Trip) -> io::Error {
    io::Error::other(format!("noob watchdog trip: {trip:?}"))
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

pub struct TickTransport {
    stream: TcpStream,
    buffers: LazyBuffers,
    ctl: WatchdogCtl,
}

impl std::fmt::Debug for TickTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TickTransport")
            .field("addr", &self.stream.peer_addr().ok())
            .finish()
    }
}

impl Transport for TickTransport {
    fn buffers(&mut self) -> &mut dyn Buffers {
        &mut self.buffers
    }

    fn transmit_output(&mut self, amount: usize, _timeout: NextTimeout) -> Result<(), ureq::Error> {
        let output = &self.buffers.output()[..amount];
        self.stream.write_all(output).map_err(|e| {
            if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) {
                // Record the phase so the error maps to a send-stall remedy,
                // never to "timed out connecting" (the connect succeeded).
                self.ctl.set_trip(Trip::SendStall);
                ureq::Error::Io(io::Error::new(io::ErrorKind::TimedOut, "request send stalled"))
            } else {
                ureq::Error::Io(e)
            }
        })
    }

    fn await_input(&mut self, _timeout: NextTimeout) -> Result<bool, ureq::Error> {
        // The socket read timeout is fixed at 1 s (set at connect); ureq's own
        // per-phase timeouts are unset, so the watchdog is the only clock.
        loop {
            if let Some(trip) = self.ctl.check() {
                return Err(ureq::Error::Io(trip_io_error(trip)));
            }
            let buf = self.buffers.input_append_buf();
            match self.stream.read(buf) {
                Ok(n) => {
                    if n > 0 {
                        self.ctl.on_bytes();
                    }
                    self.buffers.input_appended(n);
                    return Ok(n > 0);
                }
                // A timed-out read is a tick, not an error: loop and re-check.
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(ureq::Error::Io(e)),
            }
        }
    }

    fn is_open(&mut self) -> bool {
        probe_open(&mut self.stream).unwrap_or(false)
    }
}

fn probe_open(stream: &mut TcpStream) -> io::Result<bool> {
    stream.set_nonblocking(true)?;
    let mut buf = [0u8];
    let open = match stream.read(&mut buf) {
        Err(e) if e.kind() == io::ErrorKind::WouldBlock => true,
        // Unsolicited bytes or a closed/broken socket: do not reuse.
        Ok(_) | Err(_) => false,
    };
    stream.set_nonblocking(false)?;
    Ok(open)
}

#[derive(Debug)]
pub struct TickTcpConnector {
    ctl: WatchdogCtl,
    connect_timeout: Duration,
}

impl<In: Transport> Connector<In> for TickTcpConnector {
    type Out = TickTransport;

    fn connect(
        &self,
        details: &ConnectionDetails,
        _chained: Option<In>,
    ) -> Result<Option<Self::Out>, ureq::Error> {
        let mut last: Option<io::Error> = None;
        for addr in &details.addrs {
            match TcpStream::connect_timeout(addr, self.connect_timeout) {
                Ok(stream) => {
                    stream.set_nodelay(true)?;
                    stream.set_read_timeout(Some(TICK))?;
                    stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
                    let buffers = LazyBuffers::new(
                        details.config.input_buffer_size(),
                        details.config.output_buffer_size(),
                    );
                    return Ok(Some(TickTransport {
                        stream,
                        buffers,
                        ctl: self.ctl.clone(),
                    }));
                }
                Err(e) => last = Some(e),
            }
        }
        Err(ureq::Error::Io(last.unwrap_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no address resolved")
        })))
    }
}

// ---------------------------------------------------------------------------
// Retry policy and reactive compat
// ---------------------------------------------------------------------------

/// Backoff schedule for pre-content retries. One initial attempt plus one
/// retry per delay entry. Full jitter: each sleep is uniform in (0, delay].
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub delays: Vec<Duration>,
    pub jitter: bool,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            delays: vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ],
            jitter: true,
        }
    }
}

impl RetryPolicy {
    /// No retries at all (watchdog tests, doctor probes).
    pub fn none() -> Self {
        RetryPolicy { delays: Vec::new(), jitter: false }
    }
}

/// A retry happens only before the first streamed content byte:
/// connect/TLS failures and these statuses. Mid-stream death after content
/// surfaces as a turn error; a silent retry would duplicate output.
fn retryable_status(status: u16) -> bool {
    matches!(status, 408 | 425 | 429) || (500..600).contains(&status)
}

fn retryable_error(e: &ProviderError) -> bool {
    matches!(
        e,
        ProviderError::Connect(_) | ProviderError::Timeout(TimeoutKind::Connect)
    )
}

/// `Retry-After: <seconds>` honored up to 60 s; the HTTP-date form is ignored.
fn retry_after(resp: &ureq::http::Response<ureq::Body>) -> Option<Duration> {
    let secs: u64 = resp.headers().get("retry-after")?.to_str().ok()?.trim().parse().ok()?;
    Some(Duration::from_secs(secs.min(60)))
}

/// Fields a 400-and-strip compat retry may remove. Core fields are never
/// eligible: stripping them cannot produce the request the caller meant.
const COMPAT_STRIPPABLE: &[&str] = &["stream_options", "store", "include", "parallel_tool_calls"];

/// If the 400 body NAMES a strippable top-level field we sent, return it.
/// "Names" means quoted like a field (`"stream_options"`, `'store'`,
/// backticks): servers that reject a field quote it (OpenAI `Unknown
/// parameter: 'x'`, pydantic `["body","x"]`, llama.cpp `"x"`). A bare
/// substring match would false-trigger on English prose ("must include",
/// "cannot be stored") and permanently strip a field the server accepts.
fn compat_field(body: &serde_json::Value, error_body: &str) -> Option<String> {
    let map = body.as_object()?;
    COMPAT_STRIPPABLE
        .iter()
        .copied()
        .find(|f| {
            map.contains_key(*f)
                && [format!("\"{f}\""), format!("'{f}'"), format!("`{f}`")]
                    .iter()
                    .any(|quoted| error_body.contains(quoted.as_str()))
        })
        .map(str::to_string)
}

/// Uniform jitter in (0, max]. Seeded from the OS via RandomState; no rand
/// crate for one sleep.
fn jittered(max: Duration) -> Duration {
    use std::hash::{BuildHasher, Hasher};
    let r = std::collections::hash_map::RandomState::new().build_hasher().finish();
    let ms = max.as_millis().max(1) as u64;
    Duration::from_millis(r % ms + 1)
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// One blocking HTTP client with a persistent connection pool. One in-flight
/// request at a time; the watchdog state is per-client.
pub struct Client {
    agent: Agent,
    ctl: WatchdogCtl,
    timeouts: Timeouts,
    retry: RetryPolicy,
    /// Fields the endpoint 400ed on, remembered for this client's lifetime
    /// only (one client per process; no persisted quirk registry).
    compat_stripped: Mutex<HashSet<String>>,
}

impl Client {
    pub fn new(timeouts: Timeouts) -> Client {
        Client::with_retry(timeouts, RetryPolicy::default())
    }

    pub fn with_retry(timeouts: Timeouts, retry: RetryPolicy) -> Client {
        let ctl = WatchdogCtl::new();
        let config = Config::builder()
            // We surface non-2xx as data, not errors: callers need the body
            // for compat handling and error messages.
            .http_status_as_error(false)
            // ureq's Config::default() picks up HTTP_PROXY/HTTPS_PROXY/
            // ALL_PROXY from the environment, and with a proxy configured it
            // hands our connector zero resolved addresses (proxying is the
            // proxy connector's job, which we deliberately do not chain).
            // noob talks only to the configured endpoints, so proxies are
            // explicitly off; without this line an exported proxy var breaks
            // every request with a misleading error.
            .proxy(None)
            // Bound getaddrinfo so a dead DNS server cannot hang a request;
            // with a timeout set ureq resolves on a helper thread.
            .timeout_resolve(Some(Duration::from_secs(5)))
            .build();
        let connector = ()
            .chain(TickTcpConnector {
                ctl: ctl.clone(),
                connect_timeout: timeouts.connect,
            })
            .chain(RustlsConnector::default());
        let agent = Agent::with_parts(config, connector, DefaultResolver::default());
        Client { agent, ctl, timeouts, retry, compat_stripped: Mutex::new(HashSet::new()) }
    }

    /// Watchdog handle, for wiring an interrupt source other than SIGINT.
    pub fn ctl(&self) -> WatchdogCtl {
        self.ctl.clone()
    }

    pub fn timeouts(&self) -> Timeouts {
        self.timeouts
    }

    /// POST a JSON body, read the whole response through the watchdog.
    /// Returns (status, body bytes). Non-streamed convenience: same retry
    /// and compat behavior as the streaming path.
    pub fn post_json(
        &self,
        url: &str,
        api_key: &str,
        body: &serde_json::Value,
    ) -> Result<(u16, Vec<u8>), ProviderError> {
        let mut body = body.clone();
        match self.post_json_stream(url, api_key, &mut body) {
            Ok(mut stream) => {
                let status = stream.status();
                let bytes = stream.read_to_end()?;
                Ok((status, bytes))
            }
            // Callers of this convenience get non-2xx as data, like before.
            Err(ProviderError::Http { status, body }) => Ok((status, body.into_bytes())),
            Err(e) => Err(e),
        }
    }

    /// POST a JSON body and hand back the response as a watchdog-guarded
    /// streaming reader. Everything that may retry happens in here, BEFORE
    /// the first content byte reaches the caller:
    /// - connect/TLS errors and 408/425/429/5xx: backoff per the retry
    ///   policy, `Retry-After` honored up to 60 s;
    /// - a 400 naming a strippable top-level field we sent (in practice
    ///   `stream_options`): field removed from `body`, remembered for the
    ///   process lifetime, one immediate retry.
    /// Non-2xx after all that surfaces as `ProviderError::Http` with the
    /// (bounded) body already read.
    pub fn post_json_stream(
        &self,
        url: &str,
        api_key: &str,
        body: &mut serde_json::Value,
    ) -> Result<StreamBody, ProviderError> {
        let auth: Vec<(String, String)> = if api_key.is_empty() {
            Vec::new()
        } else {
            vec![("authorization".to_string(), format!("Bearer {api_key}"))]
        };
        self.post_json_stream_with(url, &auth, body)
    }

    /// Like `post_json_stream`, but with arbitrary extra request headers
    /// (the MCP Streamable HTTP transport needs Accept, MCP-Protocol-Version
    /// and Mcp-Session-Id). Content-type is always set here.
    pub fn post_json_stream_with(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &mut serde_json::Value,
    ) -> Result<StreamBody, ProviderError> {
        // Drop fields this client already learned the endpoint rejects.
        if let Some(map) = body.as_object_mut() {
            let known = self.compat_stripped.lock().unwrap();
            map.retain(|k, _| !known.contains(k));
        }

        self.ctl.arm();
        let mut backoff = self.retry.delays.iter();
        let mut compat_retried = false;
        loop {
            self.ctl.begin(&self.timeouts);
            let sent = (|| {
                let mut req = self
                    .agent
                    .post(url)
                    .header("content-type", "application/json");
                for (name, value) in headers {
                    req = req.header(name, value);
                }
                req.send(body.to_string().as_str())
                    .map_err(|e| map_ureq_error(&self.ctl, url, e))
            })();

            let resp = match sent {
                Ok(resp) => resp,
                Err(e) => {
                    self.ctl.finish();
                    if retryable_error(&e) {
                        if let Some(&delay) = backoff.next() {
                            // Same full jitter as the status branch: a fleet
                            // reconnecting to a restarted server must not
                            // stampede in lockstep.
                            self.sleep_interruptible(if self.retry.jitter {
                                jittered(delay)
                            } else {
                                delay
                            })?;
                            continue;
                        }
                    }
                    return Err(e);
                }
            };

            let status = resp.status().as_u16();
            if (200..300).contains(&status) {
                // Headers are in; from here the first-byte budget covers the
                // silent prompt-processing window before body byte one.
                self.ctl.body_started(&self.timeouts);
                return Ok(StreamBody::new(self.ctl.clone(), url, resp));
            }

            // Error statuses: read the (bounded) body through the watchdog,
            // then decide between compat strip, backoff, and surfacing.
            let wait = retry_after(&resp);
            self.ctl.body_started(&self.timeouts);
            let mut stream = StreamBody::new(self.ctl.clone(), url, resp);
            let text = match stream.read_to_end() {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(e) => return Err(e),
            };
            drop(stream);

            if status == 400 && !compat_retried {
                if let Some(field) = compat_field(body, &text) {
                    self.compat_stripped.lock().unwrap().insert(field.clone());
                    if let Some(map) = body.as_object_mut() {
                        map.remove(&field);
                    }
                    compat_retried = true;
                    continue; // immediate, does not consume a backoff slot
                }
            }
            if retryable_status(status) {
                if let Some(&delay) = backoff.next() {
                    self.sleep_interruptible(wait.unwrap_or_else(|| {
                        if self.retry.jitter { jittered(delay) } else { delay }
                    }))?;
                    continue;
                }
            }
            return Err(ProviderError::Http { status, body: text });
        }
    }

    /// Backoff sleep in 50 ms slices so Ctrl-C lands between attempts too.
    fn sleep_interruptible(&self, total: Duration) -> Result<(), ProviderError> {
        let deadline = Instant::now() + total;
        loop {
            if self.ctl.interrupted() {
                return Err(ProviderError::Interrupted);
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(());
            }
            std::thread::sleep((deadline - now).min(Duration::from_millis(50)));
        }
    }
}

/// A streaming response body. Every read goes through the tick-read
/// watchdog: interrupts and stalls surface as typed errors within about a
/// second. Dropping it returns the watchdog to `Idle`.
pub struct StreamBody {
    ctl: WatchdogCtl,
    url: String,
    status: u16,
    content_type: String,
    /// Response headers, names lowercased (MCP captures Mcp-Session-Id).
    headers: Vec<(String, String)>,
    reader: ureq::BodyReader<'static>,
}

/// Bound on non-SSE whole-body reads (error bodies, the content-type-guard
/// path). SSE streams are unbounded by design; completions are not.
const MAX_WHOLE_BODY: u64 = 8 * 1024 * 1024;

impl StreamBody {
    fn new(ctl: WatchdogCtl, url: &str, resp: ureq::http::Response<ureq::Body>) -> StreamBody {
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        let headers = resp
            .headers()
            .iter()
            .filter_map(|(n, v)| {
                v.to_str()
                    .ok()
                    .map(|v| (n.as_str().to_ascii_lowercase(), v.to_string()))
            })
            .collect();
        let (_, body) = resp.into_parts();
        StreamBody {
            ctl,
            url: url.to_string(),
            status,
            content_type,
            headers,
            reader: body.into_reader(),
        }
    }

    pub fn status(&self) -> u16 {
        self.status
    }

    /// One response header by (case-insensitive) name.
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }

    /// Lowercased media type without parameters, e.g. `application/json`.
    pub fn media_type(&self) -> String {
        self.content_type
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
    }

    /// Read the next chunk of body bytes; 0 means a clean end of stream.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, ProviderError> {
        match self.reader.read(buf) {
            Ok(n) => {
                if n > 0 {
                    // Body bytes that arrived in the same socket read as the
                    // headers are served from ureq's buffer without touching
                    // the transport, so the transport-level on_bytes never
                    // fires for them; delivery to the caller is the reliable
                    // progress signal that moves the watchdog to the idle
                    // clock (and keeps resetting it).
                    self.ctl.on_bytes();
                }
                Ok(n)
            }
            Err(e) => Err(map_ureq_error(&self.ctl, &self.url, ureq::Error::Io(e))),
        }
    }

    /// Consume the residual bytes after a logical end-of-stream marker
    /// (chat's `[DONE]`) so ureq sees the body reach its real end and
    /// returns the connection to the pool; stopping short strands it and
    /// every turn pays a fresh handshake. Bounded by `window` and a small
    /// byte cap: on a server that misbehaves past the marker we just give
    /// up and the connection is not reused, same as not draining.
    pub fn drain_for_reuse(&mut self, window: Duration) {
        self.ctl.begin_drain(window);
        let mut buf = [0u8; 4096];
        let mut total = 0usize;
        loop {
            match self.reader.read(&mut buf) {
                Ok(0) => return,
                Ok(n) => {
                    total += n;
                    if total > 16 * 1024 {
                        return;
                    }
                }
                Err(_) => {
                    // A trip here (or any IO error) only costs the reuse.
                    self.ctl.take_trip();
                    return;
                }
            }
        }
    }

    /// Read the whole remaining body (bounded; not for SSE streams).
    pub fn read_to_end(&mut self) -> Result<Vec<u8>, ProviderError> {
        let mut bytes = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            let n = self.read(&mut buf)?;
            if n == 0 {
                return Ok(bytes);
            }
            bytes.extend_from_slice(&buf[..n]);
            if bytes.len() as u64 > MAX_WHOLE_BODY {
                return Err(ProviderError::Wire(format!(
                    "response body exceeded {} bytes without ending",
                    MAX_WHOLE_BODY
                )));
            }
        }
    }
}

impl Drop for StreamBody {
    fn drop(&mut self) {
        self.ctl.finish();
    }
}

/// One-shot GET with a short overall timeout: true when the URL answers with
/// any HTTP response at all. Used only for the loopback endpoint-autodetect
/// probes at session start; never called against remote hosts.
pub fn probe(url: &str, timeout: Duration) -> bool {
    let config = Config::builder()
        .http_status_as_error(false)
        .proxy(None)
        .timeout_global(Some(timeout))
        .build();
    config.new_agent().get(url).call().is_ok()
}

fn map_ureq_error(ctl: &WatchdogCtl, url: &str, e: ureq::Error) -> ProviderError {
    if let Some(trip) = ctl.take_trip() {
        return match trip {
            Trip::Interrupted => ProviderError::Interrupted,
            Trip::FirstByte => ProviderError::Timeout(TimeoutKind::FirstByte),
            Trip::Idle => ProviderError::Timeout(TimeoutKind::Idle),
            Trip::SendStall => ProviderError::Timeout(TimeoutKind::Send),
        };
    }
    match e {
        ureq::Error::Io(io) if io.kind() == io::ErrorKind::TimedOut => {
            ProviderError::Timeout(TimeoutKind::Connect)
        }
        ureq::Error::Io(io) => {
            ProviderError::Connect(format!("{io}; is the server at {url} running?"))
        }
        other => ProviderError::Connect(format!("{other}; check NOOB_BASE_URL ({url})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn retry_set_is_exactly_the_spec_list() {
        for s in [408, 425, 429, 500, 502, 503, 599] {
            assert!(retryable_status(s), "{s} must be retryable");
        }
        for s in [200, 400, 401, 403, 404, 409, 418, 422] {
            assert!(!retryable_status(s), "{s} must not be retryable");
        }
    }

    #[test]
    fn default_policy_is_three_attempts_1_2_4() {
        let p = RetryPolicy::default();
        assert_eq!(
            p.delays,
            vec![
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4)
            ]
        );
        assert!(p.jitter);
    }

    #[test]
    fn compat_field_matches_only_quoted_field_names_we_sent() {
        let body = json!({"model": "m", "messages": [],
            "stream_options": {"include_usage": true}, "store": false});
        // The real formats: OpenAI single quotes, pydantic loc arrays,
        // llama.cpp double quotes, backticks.
        for err in [
            r#"Unknown parameter: 'stream_options'"#,
            r#"{"detail":[{"loc":["body","stream_options"],"msg":"extra"}]}"#,
            r#"Unrecognized member: "stream_options""#,
            "unexpected field `stream_options`",
        ] {
            assert_eq!(
                compat_field(&body, err),
                Some("stream_options".to_string()),
                "err: {err}"
            );
        }
        // English prose containing a field name as a word must NOT strip:
        // a false strip is remembered for the whole client lifetime.
        assert_eq!(compat_field(&body, "input must include at least one item"), None);
        assert_eq!(compat_field(&body, "this response cannot be stored"), None);
        assert_eq!(compat_field(&body, "unknown field: stream_options"), None, "unquoted");
        // Named but never sent: nothing to strip.
        assert_eq!(compat_field(&body, "'include' is not supported"), None);
        // Core fields are never eligible even if the error quotes them.
        assert_eq!(compat_field(&body, r#"invalid value for "messages""#), None);
    }

    #[test]
    fn jitter_stays_in_range_and_is_nonzero() {
        for _ in 0..200 {
            let d = jittered(Duration::from_millis(100));
            assert!(d > Duration::ZERO && d <= Duration::from_millis(100), "{d:?}");
        }
    }
}
