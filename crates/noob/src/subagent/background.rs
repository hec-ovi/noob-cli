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
    /// Cheap display invalidation. The dock reads this on its streaming hot
    /// path and builds an allocated snapshot only after lifecycle state moves.
    revision: AtomicU64,
    concurrency: usize,
}

#[derive(Default)]
struct State {
    running: usize,
    jobs: Vec<Job>,
    workers: Vec<JoinHandle<()>>,
    stopping: bool,
    /// Failure/cancellation and the next human turn are ordered by the same
    /// mutex as admission. This closes the terminal-commit-to-drain race.
    spawn_paused: bool,
    /// Monotonic human-turn counter, bumped by `begin_human_turn` under this
    /// same mutex. Jobs are stamped with it at admission; only a failure or
    /// cancellation from the CURRENT epoch may (re-)arm `spawn_paused`, so a
    /// stale child failing (or being drained by `take_ready`) after the human
    /// already moved on cannot block the new turn's authorized spawns.
    epoch: u64,
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
    /// Human-turn epoch this job was admitted in (see `State::epoch`).
    epoch: u64,
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
    /// Active jobs whose cancellation was requested but whose worker has not
    /// yet committed the terminal result. Display-only: the interrupt note
    /// and the collapsed dock row say "stopping" instead of "running".
    pub stopping: usize,
    /// The longest-running active child, for a status digest with a real
    /// fleet elapsed instead of the poll call's own ~0s duration.
    pub oldest_active: Option<(String, Duration)>,
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
                revision: AtomicU64::new(0),
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
    pub(crate) fn submit_with(
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
        if state.spawn_paused {
            return ToolOutcome::err(
                "a background child failed or was canceled in this turn; do not spawn a \
                 replacement until the human gives a new instruction",
            );
        }
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
        let epoch = state.epoch;
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
            epoch,
        });
        self.inner.revision.fetch_add(1, Ordering::Release);
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

        // The acknowledgment carries the lifecycle contract at the exact
        // decision point: the orchestrating model reads this result when it
        // chooses its next move. Small local models were caught sleeping in
        // bash to "wait" for children; the contract makes the loop closure
        // explicit and deterministic.
        ToolOutcome::ok(
            json!({
                "job_id": id,
                "status": "running",
                "contract": "detached with one goal; the final report is delivered to you \
                             automatically as [background sub-agent result]; waiting, sleeping, \
                             polling, or listing files cannot fetch it, so do unrelated work or \
                             end this turn; spawn every sibling agent in one response; subagent \
                             {\"status\":true} gives one harmless snapshot; cancel only when the \
                             user asks or the job is obsolete; a failure does not authorize \
                             spawning a replacement",
            })
            .to_string(),
            format!("{id} started · {active} active"),
        )
    }

    /// Results move out exactly once. Removing a ready job drops all of its
    /// per-job state; the fixed pool workers remain for later submissions.
    pub fn take_ready(&self) -> Vec<ReadyResult> {
        let mut state = self.inner.state.lock().unwrap();
        let current_epoch = state.epoch;
        let mut ready = Vec::new();
        let mut rearm = false;
        let mut i = 0;
        while i < state.jobs.len() {
            if state.jobs[i].state != JobState::Ready {
                i += 1;
                continue;
            }
            let mut job = state.jobs.remove(i);
            let outcome = job.outcome.take().expect("a ready job has an outcome");
            // Only a failure/cancellation from the CURRENT human turn may
            // re-arm the replacement gate: a stale child's failure delivered
            // mid-turn must not block spawns the human already authorized.
            rearm |= (outcome.is_error || outcome.canceled) && job.epoch == current_epoch;
            ready.push(ReadyResult {
                id: job.id,
                outcome,
                elapsed: job.elapsed.unwrap_or_default(),
            });
        }
        if !ready.is_empty() {
            self.inner.revision.fetch_add(1, Ordering::Release);
        }
        if rearm {
            state.spawn_paused = true;
        }
        ready
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

    /// Allocation-free lifecycle version for the dock's render hot path.
    pub fn revision(&self) -> u64 {
        self.inner.revision.load(Ordering::Acquire)
    }

    /// A real human message is the only event that can authorize considering
    /// new work after a failure. Prompt policy still requires an explicit
    /// retry request; this gate prevents autonomous same-turn retry loops.
    /// Bumping the epoch under the same mutex as admission demotes every
    /// still-outstanding job to "stale": its later failure can no longer
    /// close the gate on the new turn (see `State::epoch`).
    pub(crate) fn begin_human_turn(&self) {
        let mut state = self.inner.state.lock().unwrap();
        state.epoch += 1;
        state.spawn_paused = false;
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
            let mut status = match job.state {
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
                    if snapshot
                        .oldest_active
                        .as_ref()
                        .is_none_or(|(_, oldest)| elapsed > *oldest)
                    {
                        snapshot.oldest_active = Some((job.id.clone(), elapsed));
                    }
                    "running"
                }
                JobState::Ready => {
                    snapshot.ready += 1;
                    "ready"
                }
            };
            if job.state != JobState::Ready && job.cancel.load(Ordering::SeqCst) {
                snapshot.stopping += 1;
                status = "canceling";
            }
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
        let mut state = self.inner.state.lock().unwrap();
        let Some(job) = state
            .jobs
            .iter_mut()
            .find(|job| job.id == id && job.state != JobState::Ready)
        else {
            return false;
        };
        job.cancel.store(true, Ordering::SeqCst);
        settle_queued_cancellation(job);
        state.spawn_paused = true;
        self.inner.revision.fetch_add(1, Ordering::Release);
        drop(state);
        self.inner.changed.notify_all();
        true
    }

    pub fn cancel_all(&self) -> usize {
        let mut state = self.inner.state.lock().unwrap();
        let mut count = 0;
        for job in state
            .jobs
            .iter_mut()
            .filter(|job| job.state != JobState::Ready)
        {
            count += 1;
            job.cancel.store(true, Ordering::SeqCst);
            settle_queued_cancellation(job);
        }
        if count > 0 {
            state.spawn_paused = true;
            self.inner.revision.fetch_add(1, Ordering::Release);
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
                    let current_epoch = state.epoch;
                    let job = &mut state.jobs[index];
                    job.state = JobState::Ready;
                    job.elapsed = Some(Duration::default());
                    job.outcome = Some(ToolOutcome::canceled_with(
                        "background sub-agent canceled before it started",
                    ));
                    job.runner.take();
                    // Epoch-gated like the terminal commit below: a stale
                    // job settled after the human moved on must not close
                    // the current turn's replacement gate.
                    if job.epoch == current_epoch {
                        state.spawn_paused = true;
                    }
                    inner.revision.fetch_add(1, Ordering::Release);
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
                inner.revision.fetch_add(1, Ordering::Release);
                break (
                    job.id.clone(),
                    job.started,
                    job.cancel.clone(),
                    job.runner.take().expect("a queued job has a runner"),
                );
            }
        };

        let (id, started, cancel, runner) = work;
        let mut outcome = crate::agent::sched::catch_unwind_silent(|| runner(cancel.clone()))
            .unwrap_or_else(|_| {
                ToolOutcome::err(
                    "the background sub-agent crashed; this is a noob bug, retry the task",
                )
            });
        let mut state = inner.state.lock().unwrap();
        state.running = state.running.saturating_sub(1);
        let current_epoch = state.epoch;
        let mut pause_spawning = false;
        if let Some(job) = state.jobs.iter_mut().find(|job| job.id == id) {
            // Linearization point: cancel() accepts only while the job is not
            // Ready and sets this flag under the same state lock. If it won
            // before this commit, the runner cannot contradict it with `ok`.
            if cancel.load(Ordering::SeqCst) && !outcome.canceled {
                outcome = ToolOutcome::canceled_with("background sub-agent canceled by user");
            }
            // Terminal commit closes admission only for the human turn the
            // job belongs to; a stale child failing mid-current-turn cannot
            // block spawns the new turn already authorized.
            pause_spawning = (outcome.is_error || outcome.canceled) && job.epoch == current_epoch;
            job.state = JobState::Ready;
            job.elapsed = Some(started.elapsed());
            job.outcome = Some(outcome);
            inner.revision.fetch_add(1, Ordering::Release);
        }
        if pause_spawning {
            state.spawn_paused = true;
        }
        drop(state);
        inner.changed.notify_all();
    }
}

/// Settle a job that has not been admitted to a worker. The caller holds the
/// hub state lock, so a worker can observe either Queued or Ready, never an
/// intermediate state and never the discarded runner.
fn settle_queued_cancellation(job: &mut Job) {
    if job.state != JobState::Queued {
        return;
    }
    job.state = JobState::Ready;
    job.elapsed = Some(Duration::default());
    job.outcome = Some(ToolOutcome::canceled_with(
        "background sub-agent canceled before it started",
    ));
    job.runner.take();
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
        let snapshot = hub.snapshot();
        assert_eq!(
            (snapshot.running, snapshot.queued, snapshot.ready),
            (1, 0, 1)
        );
        let canceled = hub.take_ready();
        assert_eq!(canceled.len(), 1);
        assert_eq!(canceled[0].id, "agent-2");
        assert!(canceled[0].outcome.canceled);
        assert!(hub.take_ready().is_empty());
        assert!(
            started_rx.try_recv().is_err(),
            "a canceled queued job must not run"
        );

        gate.store(true, Ordering::SeqCst);
        wait_until(|| hub.snapshot().ready == 1);

        let ready = hub.take_ready();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, "agent-1");
        assert!(!ready[0].outcome.is_error);
        assert!(
            hub.take_ready().is_empty(),
            "results must move out exactly once"
        );
        assert!(hub.shutdown().is_empty());
    }

    #[test]
    fn cancel_all_reports_stopping_and_snapshot_names_the_oldest_running_child() {
        let hub = BackgroundHub::new(2);
        for _ in 0..2 {
            hub.submit_with("stubborn".to_string(), move |cancel| {
                while !cancel.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                ToolOutcome::canceled()
            });
        }
        wait_until(|| hub.snapshot().running == 2);
        let snapshot = hub.snapshot();
        assert_eq!(snapshot.stopping, 0);
        assert!(
            snapshot.oldest_active.is_some(),
            "a running fleet must expose its longest-running child"
        );
        assert_eq!(hub.cancel_all(), 2);
        let snapshot = hub.snapshot();
        assert_eq!(
            snapshot.stopping, snapshot.active,
            "every still-active job is stopping after cancel_all"
        );
        assert!(
            snapshot
                .rows
                .iter()
                .filter(|row| !row.contains("· ready ·"))
                .all(|row| row.contains("· canceling ·")),
            "active rows must read canceling: {:?}",
            snapshot.rows
        );
        wait_until(|| hub.snapshot().ready == 2);
        let results = hub.take_ready();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|result| result.outcome.canceled));
    }

    #[test]
    fn cancel_all_settles_queued_jobs_before_a_worker_is_free() {
        let hub = BackgroundHub::new(1);
        let (started_tx, started_rx) = mpsc::channel();
        let gate = Arc::new(AtomicBool::new(false));

        let first_gate = gate.clone();
        let first_tx = started_tx.clone();
        hub.submit_with("running".to_string(), move |cancel| {
            first_tx.send("running").unwrap();
            while !first_gate.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            if cancel.load(Ordering::SeqCst) {
                ToolOutcome::canceled()
            } else {
                ToolOutcome::ok("unexpected", "done")
            }
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        for name in ["queued-one", "queued-two", "queued-three"] {
            let tx = started_tx.clone();
            hub.submit_with(name.to_string(), move |_| {
                tx.send("queued").unwrap();
                ToolOutcome::ok("unexpected", "done")
            });
        }

        assert_eq!(hub.cancel_all(), 4);
        let snapshot = hub.snapshot();
        assert_eq!(
            (snapshot.running, snapshot.queued, snapshot.ready),
            (1, 0, 3)
        );
        let ready = hub.take_ready();
        assert_eq!(
            ready
                .iter()
                .map(|result| result.id.as_str())
                .collect::<Vec<_>>(),
            ["agent-2", "agent-3", "agent-4"]
        );
        assert!(ready.iter().all(|result| result.outcome.canceled));
        assert!(hub.take_ready().is_empty());
        assert!(started_rx.try_recv().is_err());

        gate.store(true, Ordering::SeqCst);
        wait_until(|| hub.settled_ready());
        let running = hub.take_ready();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].id, "agent-1");
        assert!(running[0].outcome.canceled);
        assert!(hub.shutdown().is_empty());
    }

    #[test]
    fn accepted_running_cancel_wins_over_a_simultaneous_success() {
        let hub = BackgroundHub::new(1);
        let (started_tx, started_rx) = mpsc::channel();
        let release = Arc::new(AtomicBool::new(false));
        let runner_release = release.clone();
        hub.submit_with("cancel race".to_string(), move |_| {
            started_tx.send(()).unwrap();
            while !runner_release.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
            // Ignore the token deliberately to force the worker-commit race.
            ToolOutcome::ok("late success", "done")
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        assert!(hub.cancel("agent-1"));
        release.store(true, Ordering::SeqCst);
        wait_until(|| hub.settled_ready());
        let ready = hub.take_ready();
        assert_eq!(ready.len(), 1);
        assert!(ready[0].outcome.canceled);
        assert!(ready[0].outcome.is_error);
        assert!(!ready[0].outcome.content.contains("late success"));
        assert!(hub.shutdown().is_empty());
    }

    #[test]
    fn accepted_cancel_blocks_same_batch_replacement_before_delivery() {
        let hub = BackgroundHub::new(1);
        let (started_tx, started_rx) = mpsc::channel();
        hub.submit_with("original".into(), move |cancel| {
            started_tx.send(()).unwrap();
            while !cancel.load(Ordering::SeqCst) {
                std::thread::yield_now();
            }
            ToolOutcome::canceled()
        });
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();

        assert!(hub.cancel("agent-1"));
        let blocked = hub.submit_with("same batch replacement".into(), |_| {
            ToolOutcome::ok("unexpected", "done")
        });
        assert!(blocked.is_error);
        assert!(blocked.content.contains("new instruction"));

        wait_until(|| hub.settled_ready());
        assert!(hub.take_ready()[0].outcome.canceled);
        hub.begin_human_turn();
        let admitted = hub.submit_with("human retry".into(), |_| ToolOutcome::ok("done", "done"));
        assert!(!admitted.is_error);
        wait_until(|| hub.settled_ready());
        assert_eq!(hub.take_ready().len(), 1);
        assert!(hub.shutdown().is_empty());
    }

    #[test]
    fn terminal_failure_blocks_same_turn_replacement_until_human_input() {
        let hub = BackgroundHub::new(1);
        hub.submit_with("fails".into(), |_| ToolOutcome::err("failed"));
        wait_until(|| hub.settled_ready());
        // Terminal commit itself closes admission. Delivery is not the
        // linearization point and cannot leave a race window.
        let blocked = hub.submit_with("replacement".into(), |_| {
            ToolOutcome::ok("unexpected", "done")
        });
        assert!(blocked.is_error);
        assert!(blocked.content.contains("new instruction"));
        assert_eq!(hub.snapshot().active, 0);
        let failed = hub.take_ready();
        assert_eq!(failed.len(), 1);
        assert!(failed[0].outcome.is_error);

        hub.begin_human_turn();
        let admitted = hub.submit_with("new human work".into(), |_| {
            ToolOutcome::ok("finished", "done")
        });
        assert!(!admitted.is_error);
        wait_until(|| hub.settled_ready());
        assert_eq!(hub.take_ready().len(), 1);
        assert!(hub.shutdown().is_empty());
    }

    #[test]
    fn a_stale_epoch_failure_does_not_block_the_current_turns_spawns() {
        // F-58 regression: a child spawned in one human turn that fails while
        // a LATER turn is active used to close the replacement gate (both at
        // the worker's terminal commit and again in take_ready), blocking the
        // current turn's authorized spawns until yet another human message.
        let hub = BackgroundHub::new(1);
        let release = Arc::new(AtomicBool::new(false));
        let gate = release.clone();
        hub.begin_human_turn(); // epoch 1: the turn that spawned the child
        hub.submit_with("old turn child".into(), move |_| {
            while !gate.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(2));
            }
            ToolOutcome::err("stale failure")
        });
        wait_until(|| hub.snapshot().running == 1);

        hub.begin_human_turn(); // epoch 2: the human moved on; child still runs
        release.store(true, Ordering::SeqCst);
        wait_until(|| hub.settled_ready());
        // The stale terminal commit must not close the gate. Hold the
        // admitted current-turn job so it stays active across the drain.
        let hold = Arc::new(AtomicBool::new(false));
        let hold_gate = hold.clone();
        let admitted = hub.submit_with("current turn work".into(), move |cancel| {
            while !hold_gate.load(Ordering::SeqCst) && !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(2));
            }
            ToolOutcome::ok("a", "done")
        });
        assert!(!admitted.is_error, "{}", admitted.content);
        // ...and neither may draining the stale failure mid-current-turn.
        let drained = hub.take_ready();
        assert_eq!(drained.len(), 1);
        assert!(drained[0].outcome.is_error);
        let admitted = hub.submit_with("still current turn".into(), |_| {
            ToolOutcome::ok("b", "done")
        });
        assert!(
            !admitted.is_error,
            "delivery of a stale failure re-armed the gate: {}",
            admitted.content
        );
        hold.store(true, Ordering::SeqCst);
        wait_until(|| hub.snapshot().ready == 2);
        assert_eq!(hub.take_ready().len(), 2);
        // A failure from the CURRENT epoch still closes the gate.
        hub.submit_with("current turn failure".into(), |_| ToolOutcome::err("boom"));
        wait_until(|| hub.settled_ready());
        let blocked = hub.submit_with("replacement".into(), |_| {
            ToolOutcome::ok("unexpected", "done")
        });
        assert!(blocked.is_error);
        assert!(blocked.content.contains("new instruction"));
        assert_eq!(hub.shutdown().len(), 1);
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
    fn panicking_job_is_silent_and_releases_its_slot_and_becomes_a_result() {
        const CHILD: &str = "NOOB_TEST_SILENT_BACKGROUND_PANIC";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("panicking_job_is_silent_and_releases_its_slot_and_becomes_a_result")
                .arg("--nocapture")
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "child failed:\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let output = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!output.contains("background panic sentinel"), "{output}");
            return;
        }

        let hub = BackgroundHub::new(1);
        hub.submit_with("crash".to_string(), |_| panic!("background panic sentinel"));
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
        let ack = serde_json::from_str::<serde_json::Value>(&second.content).unwrap();
        assert_eq!(
            ack["job_id"], "agent-2",
            "the acknowledgment stays parseable"
        );
        assert_eq!(ack["status"], "running");
        assert!(
            ack["contract"]
                .as_str()
                .unwrap()
                .contains("delivered to you"),
            "the acknowledgment must carry the lifecycle contract: {ack}"
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
