//! Session-scoped detached sub-agent jobs. Workers own only child processes
//! and this hub's state; the parent Agent remains the sole owner of its
//! transcript, session log, provider requests, and UI.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::json;

use super::{RunCfg, TaskRequest, run_task};
use crate::tools::ToolOutcome;

const MAX_JOBS: usize = 64;
const PROGRESS_KEEP_BYTES: usize = 2 * 1024;
const PROGRESS_KEEP_LINES: usize = 12;
type Runner = Box<dyn FnOnce(Arc<AtomicBool>) -> ToolOutcome + Send + 'static>;

#[derive(Clone, Default)]
pub(super) struct ProgressLog(Arc<Mutex<String>>);

impl ProgressLog {
    pub(super) fn push(&self, bytes: &[u8]) {
        let mut text = self.0.lock().unwrap();
        text.push_str(&String::from_utf8_lossy(bytes));
        if text.len() > PROGRESS_KEEP_BYTES {
            let mut cut = text.len() - PROGRESS_KEEP_BYTES;
            while !text.is_char_boundary(cut) {
                cut += 1;
            }
            text.drain(..cut);
        }
    }

    fn summary(&self) -> String {
        let text = self.0.lock().unwrap();
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        prompt_slice(&flat)
    }

    /// Recent child stderr as logical display lines. This is deliberately
    /// separate from the full terminal result: the dock may show enough live
    /// tool activity to be useful, while the completed report still moves to
    /// the parent unchanged through `ReadyResult`.
    fn recent_lines(&self) -> Vec<String> {
        let text = self.0.lock().unwrap();
        let mut lines = text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .rev()
            .take(PROGRESS_KEEP_LINES)
            .map(str::to_string)
            .collect::<Vec<_>>();
        lines.reverse();
        lines
    }
}

#[derive(Clone)]
pub struct BackgroundHub {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for BackgroundHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackgroundHub")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

struct Inner {
    state: Mutex<State>,
    changed: Condvar,
    next_id: AtomicU64,
    concurrency: usize,
}

#[derive(Default)]
struct State {
    running: usize,
    jobs: Vec<Job>,
    workers: Vec<JoinHandle<()>>,
    stopping: bool,
}

struct Job {
    id: String,
    prompt: String,
    state: JobState,
    started: Instant,
    cancel: Arc<AtomicBool>,
    outcome: Option<ToolOutcome>,
    elapsed: Option<Duration>,
    runner: Option<Runner>,
    progress: ProgressLog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JobState {
    Queued,
    Running,
    Ready,
}

pub struct ReadyResult {
    pub id: String,
    pub outcome: ToolOutcome,
    pub elapsed: Duration,
}

/// Bounded display-only progress for one undelivered child. `lines` retains
/// chronological order within the most recent portion of that child's stderr.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JobProgressSnapshot {
    pub id: String,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JobsSnapshot {
    pub active: usize,
    pub queued: usize,
    pub running: usize,
    pub ready: usize,
    pub active_ids: Vec<String>,
    pub undelivered_ids: Vec<String>,
    pub rows: Vec<String>,
    /// Jobs with non-empty recent stderr. Each per-job line list is bounded;
    /// terminal output is never sourced from or limited by this field.
    pub recent_progress: Vec<JobProgressSnapshot>,
}

impl BackgroundHub {
    pub fn new(concurrency: usize) -> BackgroundHub {
        let concurrency = concurrency.max(1);
        let hub = BackgroundHub {
            inner: Arc::new(Inner {
                state: Mutex::new(State::default()),
                changed: Condvar::new(),
                next_id: AtomicU64::new(1),
                concurrency,
            }),
        };
        let mut workers = Vec::with_capacity(concurrency);
        for ordinal in 1..=concurrency {
            let inner = hub.inner.clone();
            workers.push(
                std::thread::Builder::new()
                    .name(format!("noob-agent-worker-{ordinal}"))
                    .spawn(move || worker_loop(inner))
                    .expect("spawn the bounded background worker pool"),
            );
        }
        hub.inner.state.lock().unwrap().workers = workers;
        hub
    }

    pub(super) fn submit(&self, mut cfg: RunCfg, request: TaskRequest) -> ToolOutcome {
        let prompt = request.prompt.clone();
        let progress = ProgressLog::default();
        cfg.progress = Some(progress.clone());
        self.enqueue(prompt, progress, move |cancel| {
            run_task(&cfg, &request, || cancel.load(Ordering::SeqCst))
        })
    }

    #[cfg(test)]
    fn submit_with(
        &self,
        prompt: String,
        runner: impl FnOnce(Arc<AtomicBool>) -> ToolOutcome + Send + 'static,
    ) -> ToolOutcome {
        self.enqueue(prompt, ProgressLog::default(), runner)
    }

    fn enqueue(
        &self,
        prompt: String,
        progress: ProgressLog,
        runner: impl FnOnce(Arc<AtomicBool>) -> ToolOutcome + Send + 'static,
    ) -> ToolOutcome {
        let mut state = self.inner.state.lock().unwrap();
        if state.stopping {
            return ToolOutcome::err(
                "background sub-agents are shutting down; retry in a new session",
            );
        }
        if state.jobs.len() >= MAX_JOBS {
            return ToolOutcome::err(format!(
                "{MAX_JOBS} background sub-agents are already queued, running, or awaiting delivery; wait for results before starting more"
            ));
        }
        let ordinal = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let id = format!("agent-{ordinal}");
        state.jobs.push(Job {
            id: id.clone(),
            prompt,
            state: JobState::Queued,
            started: Instant::now(),
            cancel: Arc::new(AtomicBool::new(false)),
            outcome: None,
            elapsed: None,
            runner: Some(Box::new(runner)),
            progress,
        });
        // This summary is display metadata only; the tool-result JSON remains
        // byte-compatible. The panel uses it after a successful acknowledgment
        // so its compact count includes jobs from earlier batches and turns,
        // rather than merely the calls in the current tool group.
        let active = state
            .jobs
            .iter()
            .filter(|job| matches!(job.state, JobState::Queued | JobState::Running))
            .count();
        drop(state);
        self.inner.changed.notify_all();

        ToolOutcome::ok(
            json!({"job_id": id, "status": "running"}).to_string(),
            format!("{id} started · {active} active"),
        )
    }

    /// Results move out exactly once. Removing a ready job drops all of its
    /// per-job state; the fixed pool workers remain for later submissions.
    pub fn take_ready(&self) -> Vec<ReadyResult> {
        {
            let mut state = self.inner.state.lock().unwrap();
            let mut ready = Vec::new();
            let mut i = 0;
            while i < state.jobs.len() {
                if state.jobs[i].state != JobState::Ready {
                    i += 1;
                    continue;
                }
                let mut job = state.jobs.remove(i);
                let outcome = job.outcome.take().expect("a ready job has an outcome");
                ready.push(ReadyResult {
                    id: job.id,
                    outcome,
                    elapsed: job.elapsed.unwrap_or_default(),
                });
            }
            ready
        }
    }

    /// Wake the idle parent as soon as any terminal result is deliverable.
    /// Jobs submitted in separate model rounds are not one fan-out group, so a
    /// slow or wedged job must not hold an unrelated completed report hostage.
    /// `take_ready` coalesces everything ready at the instant the owner drains
    /// the hub, avoiding needless duplicate continuations without global
    /// all-jobs coupling.
    pub fn settled_ready(&self) -> bool {
        let state = self.inner.state.lock().unwrap();
        state.jobs.iter().any(|job| job.state == JobState::Ready)
    }

    pub fn snapshot(&self) -> JobsSnapshot {
        let state = self.inner.state.lock().unwrap();
        let mut snapshot = JobsSnapshot::default();
        for job in &state.jobs {
            snapshot.undelivered_ids.push(job.id.clone());
            let elapsed = match job.state {
                JobState::Queued => Duration::default(),
                JobState::Running => job.started.elapsed(),
                JobState::Ready => job.elapsed.unwrap_or_default(),
            };
            let status = match job.state {
                JobState::Queued => {
                    snapshot.queued += 1;
                    snapshot.active += 1;
                    snapshot.active_ids.push(job.id.clone());
                    "queued"
                }
                JobState::Running => {
                    snapshot.running += 1;
                    snapshot.active += 1;
                    snapshot.active_ids.push(job.id.clone());
                    "running"
                }
                JobState::Ready => {
                    snapshot.ready += 1;
                    "ready"
                }
            };
            let mut row = format!(
                "{} · {status} · {} · {}",
                job.id,
                crate::ui::elapsed_label(elapsed),
                prompt_slice(&job.prompt),
            );
            let progress = job.progress.summary();
            if !progress.is_empty() {
                row.push_str(" · ");
                row.push_str(&progress);
            }
            snapshot.rows.push(row);
            let lines = job.progress.recent_lines();
            if !lines.is_empty() {
                snapshot.recent_progress.push(JobProgressSnapshot {
                    id: job.id.clone(),
                    lines,
                });
            }
        }
        snapshot
    }

    pub fn cancel(&self, id: &str) -> bool {
        let state = self.inner.state.lock().unwrap();
        let Some(job) = state
            .jobs
            .iter()
            .find(|job| job.id == id && job.state != JobState::Ready)
        else {
            return false;
        };
        job.cancel.store(true, Ordering::SeqCst);
        drop(state);
        self.inner.changed.notify_all();
        true
    }

    pub fn cancel_all(&self) -> usize {
        let state = self.inner.state.lock().unwrap();
        let active = state.jobs.iter().filter(|job| job.state != JobState::Ready);
        let count = active.clone().count();
        for job in active {
            job.cancel.store(true, Ordering::SeqCst);
        }
        drop(state);
        self.inner.changed.notify_all();
        count
    }

    /// Cancel every queued/running child and wait until each worker has killed
    /// and reaped its process group. The terminal results remain drainable.
    pub fn shutdown(&self) -> Vec<ReadyResult> {
        self.cancel_all();
        let handles = {
            let mut state = self.inner.state.lock().unwrap();
            state.stopping = true;
            std::mem::take(&mut state.workers)
        };
        self.inner.changed.notify_all();
        for handle in handles {
            let _ = handle.join();
        }
        self.take_ready()
    }

    pub fn raise_next_id(&self, next: u64) {
        let mut current = self.inner.next_id.load(Ordering::SeqCst);
        while current < next {
            match self.inner.next_id.compare_exchange(
                current,
                next,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
    }
}

fn worker_loop(inner: Arc<Inner>) {
    loop {
        let work = {
            let mut state = inner.state.lock().unwrap();
            loop {
                let queued = state
                    .jobs
                    .iter()
                    .position(|job| job.state == JobState::Queued);
                let Some(index) = queued else {
                    if state.stopping {
                        return;
                    }
                    state = inner.changed.wait(state).unwrap();
                    continue;
                };
                if state.jobs[index].cancel.load(Ordering::SeqCst) {
                    let job = &mut state.jobs[index];
                    job.state = JobState::Ready;
                    job.elapsed = Some(Duration::default());
                    job.outcome = Some(ToolOutcome::canceled_with(
                        "background sub-agent canceled before it started",
                    ));
                    job.runner.take();
                    inner.changed.notify_all();
                    continue;
                }
                if state.running >= inner.concurrency {
                    state = inner.changed.wait(state).unwrap();
                    continue;
                }
                state.running += 1;
                let job = &mut state.jobs[index];
                job.state = JobState::Running;
                job.started = Instant::now();
                break (
                    job.id.clone(),
                    job.started,
                    job.cancel.clone(),
                    job.runner.take().expect("a queued job has a runner"),
                );
            }
        };

        let (id, started, cancel, runner) = work;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runner(cancel)))
            .unwrap_or_else(|_| {
                ToolOutcome::err(
                    "the background sub-agent crashed; this is a noob bug, retry the task",
                )
            });
        let mut state = inner.state.lock().unwrap();
        state.running = state.running.saturating_sub(1);
        if let Some(job) = state.jobs.iter_mut().find(|job| job.id == id) {
            job.state = JobState::Ready;
            job.elapsed = Some(started.elapsed());
            job.outcome = Some(outcome);
        }
        drop(state);
        inner.changed.notify_all();
    }
}

fn prompt_slice(prompt: &str) -> String {
    let flat = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= 72 {
        return flat;
    }
    let head: String = flat.chars().take(35).collect();
    let count = flat.chars().count();
    let tail: String = flat.chars().skip(count - 35).collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn wait_until(mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !predicate() {
            assert!(Instant::now() < deadline, "condition did not become true");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn cap_queue_cancel_and_exact_once_drain() {
        let hub = BackgroundHub::new(1);
        let (started_tx, started_rx) = mpsc::channel();
        let gate = Arc::new(AtomicBool::new(false));

        for name in ["alpha", "beta"] {
            let tx = started_tx.clone();
            let release = gate.clone();
            let label = name.to_string();
            hub.submit_with(label.clone(), move |cancel| {
                tx.send(label.clone()).unwrap();
                while !release.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                if cancel.load(Ordering::SeqCst) {
                    ToolOutcome::canceled()
                } else {
                    ToolOutcome::ok(format!("{label} result"), "done")
                }
            });
        }

        assert_eq!(
            started_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "alpha"
        );
        let snapshot = hub.snapshot();
        assert_eq!((snapshot.running, snapshot.queued), (1, 1));
        assert!(hub.cancel("agent-2"));
        gate.store(true, Ordering::SeqCst);
        wait_until(|| hub.snapshot().ready == 2);

        let ready = hub.take_ready();
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].id, "agent-1");
        assert_eq!(ready[1].id, "agent-2");
        assert!(!ready[0].outcome.is_error);
        assert!(ready[1].outcome.canceled);
        assert!(
            hub.take_ready().is_empty(),
            "results must move out exactly once"
        );
        assert!(
            started_rx.try_recv().is_err(),
            "a canceled queued job must not run"
        );
    }

    #[test]
    fn shutdown_cancels_and_joins_running_workers() {
        let hub = BackgroundHub::new(2);
        let (started_tx, started_rx) = mpsc::channel();
        for name in ["one", "two"] {
            let tx = started_tx.clone();
            hub.submit_with(name.to_string(), move |cancel| {
                tx.send(()).unwrap();
                while !cancel.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                ToolOutcome::canceled()
            });
        }
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let started = Instant::now();
        let results = hub.shutdown();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result.outcome.canceled));
        assert_eq!(hub.snapshot(), JobsSnapshot::default());
    }

    #[test]
    fn panicking_job_releases_its_slot_and_becomes_a_result() {
        let hub = BackgroundHub::new(1);
        hub.submit_with("crash".to_string(), |_| panic!("boom"));
        hub.submit_with("next".to_string(), |_| ToolOutcome::ok("finished", "done"));
        wait_until(|| hub.snapshot().ready == 2);
        let results = hub.take_ready();
        assert_eq!(results.len(), 2);
        assert!(results[0].outcome.is_error);
        assert!(results[0].outcome.content.contains("crashed"));
        assert_eq!(results[1].outcome.content, "finished");
    }

    #[test]
    fn one_ready_job_is_deliverable_while_an_unrelated_job_keeps_running() {
        let hub = BackgroundHub::new(2);
        let gate = Arc::new(AtomicBool::new(false));
        let slow_gate = gate.clone();
        hub.submit_with("fast".to_string(), |_| {
            ToolOutcome::ok("fast result", "done")
        });
        hub.submit_with("slow".to_string(), move |cancel| {
            while !slow_gate.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            ToolOutcome::ok("slow result", "done")
        });

        wait_until(|| {
            let snapshot = hub.snapshot();
            snapshot.ready == 1 && snapshot.running == 1
        });
        assert!(
            hub.settled_ready(),
            "a completed report must wake the parent independently"
        );
        let ready = hub.take_ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "agent-1");
        assert!(
            !hub.settled_ready(),
            "the running job is not yet deliverable"
        );

        gate.store(true, Ordering::SeqCst);
        wait_until(|| hub.settled_ready());
        let ready = hub.take_ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "agent-2");
        assert!(hub.shutdown().is_empty());
    }

    #[test]
    fn acknowledgment_summary_counts_all_active_hub_jobs() {
        let hub = BackgroundHub::new(1);
        let gate = Arc::new(AtomicBool::new(false));
        let first_gate = gate.clone();
        let first = hub.submit_with("first".to_string(), move |cancel| {
            while !first_gate.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            ToolOutcome::canceled()
        });
        let second_gate = gate.clone();
        let second = hub.submit_with("second".to_string(), move |cancel| {
            while !second_gate.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            ToolOutcome::canceled()
        });

        assert_eq!(first.summary, "agent-1 started · 1 active");
        assert_eq!(second.summary, "agent-2 started · 2 active");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&second.content).unwrap(),
            json!({"job_id":"agent-2","status":"running"}),
            "the transcript acknowledgment stays compatible"
        );
        gate.store(true, Ordering::SeqCst);
        assert_eq!(hub.shutdown().len(), 2);
    }

    #[test]
    fn pending_queue_is_bounded() {
        let hub = BackgroundHub::new(1);
        let gate = Arc::new(AtomicBool::new(false));
        for ordinal in 0..MAX_JOBS {
            let release = gate.clone();
            let outcome = hub.submit_with(format!("job {ordinal}"), move |cancel| {
                while !release.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                ToolOutcome::canceled()
            });
            assert!(!outcome.is_error, "job {ordinal}: {}", outcome.content);
        }
        let overflow = hub.submit_with("overflow".to_string(), |_| ToolOutcome::canceled());
        assert!(overflow.is_error);
        assert!(overflow.content.contains("64 background sub-agents"));
        gate.store(true, Ordering::SeqCst);
        let results = hub.shutdown();
        assert_eq!(results.len(), MAX_JOBS);
    }

    #[test]
    fn snapshot_includes_bounded_live_child_progress() {
        let hub = BackgroundHub::new(1);
        let progress = ProgressLog::default();
        let gate = Arc::new(AtomicBool::new(false));
        let release = gate.clone();
        hub.enqueue("inspect files".to_string(), progress.clone(), move |_| {
            while !release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            ToolOutcome::ok("done", "done")
        });
        wait_until(|| hub.snapshot().running == 1);
        progress.push(b"read src/main.rs\noutput: found entry point\n");
        let snapshot = hub.snapshot();
        let row = snapshot.rows.join("\n");
        assert!(row.contains("read src/main.rs"), "{row}");
        assert!(row.contains("output: found entry point"), "{row}");
        assert_eq!(
            snapshot.recent_progress,
            vec![JobProgressSnapshot {
                id: "agent-1".to_string(),
                lines: vec![
                    "read src/main.rs".to_string(),
                    "output: found entry point".to_string(),
                ],
            }],
            "the Tab view needs child calls and their outputs as separate rows"
        );
        gate.store(true, Ordering::SeqCst);
        wait_until(|| hub.settled_ready());
        hub.take_ready();
        assert!(hub.shutdown().is_empty());
    }

    #[test]
    fn snapshot_progress_keeps_only_the_latest_complete_line_window() {
        let hub = BackgroundHub::new(1);
        let progress = ProgressLog::default();
        let gate = Arc::new(AtomicBool::new(false));
        let release = gate.clone();
        let final_output = "result ".repeat(PROGRESS_KEEP_BYTES);
        let runner_output = final_output.clone();
        hub.enqueue("many steps".to_string(), progress.clone(), move |_| {
            while !release.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            ToolOutcome::ok(runner_output, "done")
        });
        wait_until(|| hub.snapshot().running == 1);
        for ordinal in 0..(PROGRESS_KEEP_LINES + 3) {
            progress.push(format!("* tool event {ordinal}\n").as_bytes());
        }

        let details = hub.snapshot().recent_progress;
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].lines.len(), PROGRESS_KEEP_LINES);
        assert_eq!(details[0].lines.first().unwrap(), "* tool event 3");
        assert_eq!(details[0].lines.last().unwrap(), "* tool event 14");

        gate.store(true, Ordering::SeqCst);
        wait_until(|| hub.settled_ready());
        assert_eq!(
            hub.take_ready()[0].outcome.content,
            final_output,
            "the bounded display snapshot must not cap the injected final result"
        );
        assert!(hub.shutdown().is_empty());
    }
}
