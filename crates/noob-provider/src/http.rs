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

    fn begin(&self, t: &Timeouts) {
        *self.inner.trip.lock().unwrap() = None;
        self.inner.local_interrupt.store(false, Ordering::SeqCst);
        *self.inner.phase.lock().unwrap() = Phase::AwaitHeaders {
            deadline: Instant::now() + t.first_byte,
        };
    }

    fn body_started(&self, t: &Timeouts) {
        *self.inner.phase.lock().unwrap() = Phase::AwaitFirstByte {
            deadline: Instant::now() + t.first_byte,
            idle: t.idle,
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
        let trip = if INTERRUPTED.load(Ordering::SeqCst)
            || self.inner.local_interrupt.load(Ordering::SeqCst)
        {
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
                ureq::Error::Io(io::Error::new(io::ErrorKind::TimedOut, "write timed out"))
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
// Client
// ---------------------------------------------------------------------------

/// One blocking HTTP client with a persistent connection pool. One in-flight
/// request at a time; the watchdog state is per-client.
pub struct Client {
    agent: Agent,
    ctl: WatchdogCtl,
    timeouts: Timeouts,
}

impl Client {
    pub fn new(timeouts: Timeouts) -> Client {
        let ctl = WatchdogCtl::new();
        let config = Config::builder()
            // We surface non-2xx as data, not errors: callers need the body
            // for compat handling and error messages.
            .http_status_as_error(false)
            .build();
        let connector = ()
            .chain(TickTcpConnector {
                ctl: ctl.clone(),
                connect_timeout: timeouts.connect,
            })
            .chain(RustlsConnector::default());
        let agent = Agent::with_parts(config, connector, DefaultResolver::default());
        Client { agent, ctl, timeouts }
    }

    /// Watchdog handle, for wiring an interrupt source other than SIGINT.
    pub fn ctl(&self) -> WatchdogCtl {
        self.ctl.clone()
    }

    pub fn timeouts(&self) -> Timeouts {
        self.timeouts
    }

    /// POST a JSON body, read the whole response through the watchdog.
    /// Returns (status, body bytes). P1 replaces whole-body reads with
    /// chunked SSE streaming over the same transport.
    pub fn post_json(
        &self,
        url: &str,
        api_key: &str,
        body: &serde_json::Value,
    ) -> Result<(u16, Vec<u8>), ProviderError> {
        self.ctl.begin(&self.timeouts);
        let result = (|| {
            let mut req = self
                .agent
                .post(url)
                .header("content-type", "application/json");
            if !api_key.is_empty() {
                req = req.header("authorization", &format!("Bearer {api_key}"));
            }
            let mut resp = req
                .send(body.to_string().as_str())
                .map_err(|e| self.map_error(url, e))?;
            let status = resp.status().as_u16();
            // Headers are in; from here the first-byte budget covers the
            // silent prompt-processing window before body byte one.
            self.ctl.body_started(&self.timeouts);
            let mut bytes = Vec::new();
            resp.body_mut()
                .as_reader()
                .read_to_end(&mut bytes)
                .map_err(|e| self.map_error(url, ureq::Error::Io(e)))?;
            Ok((status, bytes))
        })();
        self.ctl.finish();
        result
    }

    fn map_error(&self, url: &str, e: ureq::Error) -> ProviderError {
        if let Some(trip) = self.ctl.take_trip() {
            return match trip {
                Trip::Interrupted => ProviderError::Interrupted,
                Trip::FirstByte => ProviderError::Timeout(TimeoutKind::FirstByte),
                Trip::Idle => ProviderError::Timeout(TimeoutKind::Idle),
            };
        }
        match e {
            ureq::Error::Io(io) if io.kind() == io::ErrorKind::TimedOut => {
                ProviderError::Timeout(TimeoutKind::Connect)
            }
            ureq::Error::Io(io) => ProviderError::Connect(format!(
                "{io}; is the server at {url} running?"
            )),
            other => ProviderError::Connect(format!("{other}; check NOOB_BASE_URL ({url})")),
        }
    }
}
