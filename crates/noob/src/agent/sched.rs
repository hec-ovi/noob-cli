//! Batch scheduler: within one assistant tool batch, consecutive read-only
//! calls run concurrently on scoped threads (cap 8), consecutive subagent
//! calls form one fan-out group, and any other mutating call is a sequential
//! barrier executed alone, in order. Detached subagent admissions stay ordered
//! while the background hub runs their children concurrently; inline subagent
//! calls use the child cap directly.
//! Results always come back in emission order regardless of completion
//! order: parallelism where it is free, total determinism where it matters
//! (two edits to one file can never race).

use std::ops::Range;
use std::sync::Once;
#[cfg(test)]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::Value;

use noob_provider::http::INTERRUPTED;

use crate::tools::{self, ToolCtx, ToolOutcome};

const READ_CONCURRENCY: usize = 8;

#[cfg(test)]
static TEST_TASK_ADMISSIONS: AtomicU64 = AtomicU64::new(0);

/// One call, pre-processed by the loop: either execute (name, args) or
/// return a canned outcome (doom-loop intercept, unparseable arguments).
pub enum Planned {
    Run { name: String, args: Value },
    Canned(ToolOutcome),
}

/// Scheduling class of one planned call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Canned outcomes execute nothing; they join any group.
    Free,
    Read,
    /// Task calls: one fan-out group. Its execution mode depends on whether
    /// the surface has a detached hub.
    Task,
    /// Everything else mutates: alone, in order.
    Mutating,
}

fn kind(planned: &Planned) -> Kind {
    match planned {
        Planned::Canned(_) => Kind::Free,
        #[cfg(test)]
        Planned::Run { name, .. } if name == "__test_wait" => Kind::Read,
        #[cfg(test)]
        Planned::Run { name, .. } if name == "__test_panic" => Kind::Read,
        #[cfg(test)]
        Planned::Run { name, .. } if name == "__test_mutating_panic" => Kind::Mutating,
        #[cfg(test)]
        Planned::Run { name, .. } if name == "__test_task" => Kind::Task,
        Planned::Run { name, .. } if name == "subagent" => Kind::Task,
        Planned::Run { name, .. } if tools::is_read_only(name) => Kind::Read,
        Planned::Run { .. } => Kind::Mutating,
    }
}

/// One scheduled group: a contiguous range and the concurrency it runs at.
#[derive(Debug, PartialEq, Eq)]
struct Group {
    range: Range<usize>,
    kind: Kind,
}

/// Split a batch into contiguous groups: maximal runs of one concurrent
/// kind (Free items join whatever group they sit in), and single-item
/// groups for every mutating call. Pure, so the invariants are unit-tested
/// without spawning anything.
fn partition(kinds: &[Kind]) -> Vec<Group> {
    let mut groups = Vec::new();
    let n = kinds.len();
    let mut i = 0;
    while i < n {
        let mut group_kind: Option<Kind> = None;
        let mut j = i;
        while j < n {
            match kinds[j] {
                Kind::Free => {
                    j += 1;
                }
                Kind::Mutating => break,
                k => match group_kind {
                    None => {
                        group_kind = Some(k);
                        j += 1;
                    }
                    Some(g) if g == k => j += 1,
                    Some(_) => break,
                },
            }
        }
        if j == i {
            // kinds[i] is Mutating: alone, in order.
            groups.push(Group {
                range: i..i + 1,
                kind: Kind::Mutating,
            });
            i += 1;
        } else {
            // All-Free groups run at the read cap (nothing executes anyway).
            groups.push(Group {
                range: i..j,
                kind: group_kind.unwrap_or(Kind::Read),
            });
            i = j;
        }
    }
    groups
}

/// Execute a batch, returning outcomes in emission order. A Ctrl-C observed
/// between barriers or waves cancels every remaining call with a synthetic
/// "canceled by user" result: a mutation must never land AFTER the user
/// canceled, and every call id still gets exactly one result.
pub enum Progress<'a> {
    Started {
        index: usize,
    },
    Finished {
        index: usize,
        outcome: &'a ToolOutcome,
        elapsed: Option<Duration>,
    },
}

#[cfg(test)]
pub fn run_batch(ctx: &ToolCtx, batch: Vec<Planned>) -> Vec<ToolOutcome> {
    run_batch_with(ctx, batch, |_| {})
}

/// Execute a batch while reporting lifecycle transitions on the scheduler
/// thread. Starts are emitted immediately before execution; parallel finishes
/// are reported in real completion order. Returned outcomes remain in model
/// emission order, preserving the transcript and cache invariants.
pub fn run_batch_with(
    ctx: &ToolCtx,
    batch: Vec<Planned>,
    mut on_progress: impl FnMut(Progress<'_>),
) -> Vec<ToolOutcome> {
    let kinds: Vec<Kind> = batch.iter().map(kind).collect();
    let mut slots: Vec<Option<ToolOutcome>> = batch.iter().map(|_| None).collect();
    // Consume left to right so each Planned is moved exactly once.
    let mut batch: Vec<Option<Planned>> = batch.into_iter().map(Some).collect();
    for group in partition(&kinds) {
        if INTERRUPTED.load(Ordering::SeqCst) {
            for index in group.range.clone() {
                let outcome = ToolOutcome::canceled();
                on_progress(Progress::Finished {
                    index,
                    outcome: &outcome,
                    elapsed: None,
                });
                slots[index] = Some(outcome);
            }
            continue; // later groups get their synthetic results too
        }
        if group.kind == Kind::Mutating {
            let index = group.range.start;
            let planned = batch[index].take().unwrap();
            let started = if matches!(&planned, Planned::Run { .. }) {
                on_progress(Progress::Started { index });
                Some(Instant::now())
            } else {
                None
            };
            let outcome = catch_unwind_silent(|| execute(ctx, planned)).unwrap_or_else(|_| {
                ToolOutcome::err(
                    "the tool crashed while running; this is a noob bug, try a different approach",
                )
            });
            on_progress(Progress::Finished {
                index,
                outcome: &outcome,
                elapsed: started.map(|at| at.elapsed()),
            });
            slots[index] = Some(outcome);
            continue;
        }
        // Detached subagent calls are admissions or controls, not the child
        // work itself. Execute them in model emission order so job IDs are
        // deterministic and a cancel in the same batch cannot overtake a
        // spawn. The hub's bounded workers provide the actual concurrency.
        if group.kind == Kind::Task && detached_tasks(ctx) {
            for index in group.range.clone() {
                if INTERRUPTED.load(Ordering::SeqCst) {
                    let outcome = ToolOutcome::canceled();
                    on_progress(Progress::Finished {
                        index,
                        outcome: &outcome,
                        elapsed: None,
                    });
                    slots[index] = Some(outcome);
                    continue;
                }
                let planned = batch[index].take().unwrap();
                let started = if matches!(&planned, Planned::Run { .. }) {
                    on_progress(Progress::Started { index });
                    Some(Instant::now())
                } else {
                    None
                };
                let outcome = catch_unwind_silent(|| execute(ctx, planned)).unwrap_or_else(|_| {
                    ToolOutcome::err(
                        "the tool crashed while running; this is a noob bug, try a different \
                         approach",
                    )
                });
                on_progress(Progress::Finished {
                    index,
                    outcome: &outcome,
                    elapsed: started.map(|at| at.elapsed()),
                });
                slots[index] = Some(outcome);
            }
            continue;
        }
        let cap = match group.kind {
            Kind::Task => ctx.task_concurrency(),
            _ => READ_CONCURRENCY,
        };
        let group_items: Vec<(usize, Planned)> = group
            .range
            .clone()
            .map(|k| (k, batch[k].take().unwrap()))
            .collect();
        for wave in group_items.chunks(cap) {
            if INTERRUPTED.load(Ordering::SeqCst) {
                for (k, _) in wave {
                    let outcome = ToolOutcome::canceled();
                    on_progress(Progress::Finished {
                        index: *k,
                        outcome: &outcome,
                        elapsed: None,
                    });
                    slots[*k] = Some(outcome);
                }
                continue;
            }
            std::thread::scope(|scope| {
                let (done_tx, done_rx) = std::sync::mpsc::channel();
                let mut running = 0usize;
                for (index, planned) in wave {
                    match planned {
                        Planned::Canned(_) => {
                            let outcome = execute_ref(ctx, planned);
                            on_progress(Progress::Finished {
                                index: *index,
                                outcome: &outcome,
                                elapsed: None,
                            });
                            slots[*index] = Some(outcome);
                        }
                        Planned::Run { .. } => {
                            let started = Instant::now();
                            on_progress(Progress::Started { index: *index });
                            running += 1;
                            let tx = done_tx.clone();
                            scope.spawn(move || {
                                let outcome = catch_unwind_silent(|| execute_ref(ctx, planned))
                                    .unwrap_or_else(|_| {
                                        ToolOutcome::err(
                                            "the tool crashed while running; this is a noob bug, \
                                         try a different approach",
                                        )
                                    });
                                let _ = tx.send((*index, outcome, started.elapsed()));
                            });
                        }
                    }
                }
                drop(done_tx);
                for _ in 0..running {
                    let (index, outcome, elapsed) = done_rx
                        .recv()
                        .expect("every scoped tool sends one completion");
                    on_progress(Progress::Finished {
                        index,
                        outcome: &outcome,
                        elapsed: Some(elapsed),
                    });
                    slots[index] = Some(outcome);
                }
            });
        }
    }
    slots.into_iter().map(Option::unwrap).collect()
}

fn detached_tasks(ctx: &ToolCtx) -> bool {
    ctx.task
        .as_ref()
        .and_then(|task| task.background.as_ref())
        .is_some()
}

thread_local! {
    /// The process panic hook is global, but suppression must apply only to
    /// the worker whose panic is converted into a ToolOutcome.
    static SILENCE_CAUGHT_PANIC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Catch an unwind without letting the default hook write a red panic report
/// into the dock. A permanent hook wrapper avoids swapping the process-global
/// hook while other scheduler/background threads are running.
pub(crate) fn catch_unwind_silent<F, R>(f: F) -> std::thread::Result<R>
where
    F: FnOnce() -> R,
{
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !SILENCE_CAUGHT_PANIC.with(std::cell::Cell::get) {
                previous(info);
            }
        }));
    });

    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            SILENCE_CAUGHT_PANIC.with(|silent| silent.set(self.0));
        }
    }

    let previous = SILENCE_CAUGHT_PANIC.with(|silent| silent.replace(true));
    let restore = Restore(previous);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    drop(restore);
    result
}

fn execute(ctx: &ToolCtx, planned: Planned) -> ToolOutcome {
    match planned {
        Planned::Canned(out) => out,
        Planned::Run { name, args } => dispatch(ctx, &name, &args),
    }
}

fn execute_ref(ctx: &ToolCtx, planned: &Planned) -> ToolOutcome {
    match planned {
        Planned::Canned(out) => ToolOutcome {
            content: out.content.clone(),
            is_error: out.is_error,
            summary: out.summary.clone(),
            warning: out.warning.clone(),
            canceled: out.canceled,
        },
        Planned::Run { name, args } => dispatch(ctx, name, args),
    }
}

fn dispatch(ctx: &ToolCtx, name: &str, args: &Value) -> ToolOutcome {
    #[cfg(test)]
    if name == "__test_wait" {
        let millis = args.get("millis").and_then(Value::as_u64).unwrap_or(1);
        std::thread::sleep(std::time::Duration::from_millis(millis));
        return ToolOutcome::ok(millis.to_string(), format!("waited {millis}ms"));
    }
    #[cfg(test)]
    if name == "__test_task" {
        let millis = args.get("millis").and_then(Value::as_u64).unwrap_or(0);
        std::thread::sleep(std::time::Duration::from_millis(millis));
        if args.get("action").and_then(Value::as_str) == Some("control") {
            let admitted = TEST_TASK_ADMISSIONS.load(Ordering::SeqCst);
            return ToolOutcome::ok(
                format!("control-after-{admitted}"),
                "controlled detached task",
            );
        }
        let id = TEST_TASK_ADMISSIONS.fetch_add(1, Ordering::SeqCst) + 1;
        return ToolOutcome::ok(format!("agent-{id}"), "admitted detached task");
    }
    #[cfg(test)]
    if matches!(name, "__test_panic" | "__test_mutating_panic") {
        panic!("scheduler panic sentinel");
    }
    tools::dispatch(ctx, name, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn ctx() -> (tempfile::TempDir, ToolCtx) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        (
            tmp,
            ToolCtx::new(ws, crate::tools::guard::Sandbox::Container),
        )
    }

    fn bash(cmd: &str) -> Planned {
        Planned::Run {
            name: "bash".into(),
            args: json!({"cmd": cmd}),
        }
    }

    fn read(path: &str) -> Planned {
        Planned::Run {
            name: "read".into(),
            args: json!({"path": path}),
        }
    }

    fn test_task(action: &str, millis: u64) -> Planned {
        Planned::Run {
            name: "__test_task".into(),
            args: json!({"action": action, "millis": millis}),
        }
    }

    #[test]
    fn results_come_back_in_emission_order() {
        let (_t, ctx) = ctx();
        for name in ["a", "b", "c"] {
            std::fs::write(ctx.workspace.join(name), format!("content {name}\n")).unwrap();
        }
        let out = run_batch(&ctx, vec![read("c"), read("a"), read("b")]);
        assert!(out[0].content.contains("content c"));
        assert!(out[1].content.contains("content a"));
        assert!(out[2].content.contains("content b"));
    }

    #[test]
    fn read_only_group_genuinely_overlaps() {
        let (_t, ctx) = ctx();
        // A test-only read-class operation isolates scheduler timing without
        // teaching the real read tool to open FIFOs or other special files.
        let waits: Vec<Planned> = (0..6)
            .map(|_| Planned::Run {
                name: "__test_wait".into(),
                args: json!({"millis": 100}),
            })
            .collect();
        let started = Instant::now();
        let out = run_batch(&ctx, waits);
        assert_eq!(out.len(), 6);
        assert!(out.iter().all(|o| !o.is_error));
        assert!(
            started.elapsed() < Duration::from_millis(350),
            "six 100ms reads serialized: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn progress_reports_parallel_finishes_when_they_really_complete() {
        let (_t, ctx) = ctx();
        let batch = vec![
            Planned::Run {
                name: "__test_wait".into(),
                args: json!({"millis": 80}),
            },
            Planned::Run {
                name: "__test_wait".into(),
                args: json!({"millis": 5}),
            },
        ];
        let mut events = Vec::new();
        let mut durations = Vec::new();
        let out = run_batch_with(&ctx, batch, |event| match event {
            Progress::Started { index } => events.push(("start", index)),
            Progress::Finished { index, elapsed, .. } => {
                events.push(("done", index));
                durations.push(elapsed.expect("a real call has a duration"));
            }
        });
        assert_eq!(events[..2], [("start", 0), ("start", 1)]);
        assert_eq!(events[2..], [("done", 1), ("done", 0)]);
        assert_eq!(out[0].content, "80");
        assert_eq!(out[1].content, "5");
        assert!(durations.iter().all(|elapsed| !elapsed.is_zero()));
    }

    #[test]
    fn detached_task_admissions_and_controls_follow_emission_order() {
        TEST_TASK_ADMISSIONS.store(0, Ordering::SeqCst);
        let (_t, mut ctx) = ctx();
        let hub = crate::subagent::BackgroundHub::new(2);
        ctx.task = Some(crate::subagent::TaskCfg {
            depth: 0,
            concurrency: 2,
            max_turns: 10,
            wall_clock: Duration::from_secs(30),
            verbose: false,
            overrides: Default::default(),
            yolo: false,
            ancestor_skills: Vec::new(),
            background: Some(hub.clone()),
        });
        let mut events = Vec::new();
        let out = run_batch_with(
            &ctx,
            vec![
                test_task("spawn", 60),
                test_task("control", 0),
                test_task("spawn", 0),
            ],
            |event| match event {
                Progress::Started { index } => events.push(("start", index)),
                Progress::Finished { index, .. } => events.push(("done", index)),
            },
        );

        assert_eq!(
            events,
            vec![
                ("start", 0),
                ("done", 0),
                ("start", 1),
                ("done", 1),
                ("start", 2),
                ("done", 2),
            ]
        );
        assert_eq!(out[0].content, "agent-1");
        assert_eq!(out[1].content, "control-after-1");
        assert_eq!(out[2].content, "agent-2");
        assert!(hub.shutdown().is_empty());
    }

    #[test]
    fn caught_scheduler_panic_does_not_reach_the_process_hook() {
        const CHILD: &str = "NOOB_TEST_SILENT_SCHEDULER_PANIC";
        if std::env::var_os(CHILD).is_some() {
            let (_t, ctx) = ctx();
            let out = run_batch(
                &ctx,
                vec![Planned::Run {
                    name: "__test_panic".into(),
                    args: json!({}),
                }],
            );
            assert_eq!(out.len(), 1);
            assert!(out[0].is_error);
            assert!(out[0].content.contains("tool crashed"));
            return;
        }

        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("caught_scheduler_panic_does_not_reach_the_process_hook")
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
        assert!(!output.contains("scheduler panic sentinel"), "{output}");
    }

    #[test]
    fn mutating_tool_panic_becomes_one_typed_result() {
        let (_t, ctx) = ctx();
        let out = run_batch(
            &ctx,
            vec![Planned::Run {
                name: "__test_mutating_panic".into(),
                args: json!({}),
            }],
        );
        assert_eq!(out.len(), 1);
        assert!(out[0].is_error);
        assert!(out[0].content.contains("tool crashed"));
    }

    #[test]
    fn mutations_serialize_in_order() {
        let (_t, ctx) = ctx();
        let out = run_batch(
            &ctx,
            vec![
                bash("sleep 0.05; echo A >> log.txt"),
                bash("echo B >> log.txt"),
                bash("echo C >> log.txt"),
            ],
        );
        assert!(out.iter().all(|o| !o.is_error));
        let log = std::fs::read_to_string(ctx.workspace.join("log.txt")).unwrap();
        // Sequential barriers: A finished (with its sleep) before B started.
        assert_eq!(log, "A\nB\nC\n");
    }

    #[test]
    fn reads_run_concurrently_but_a_mutation_is_a_barrier() {
        let (_t, ctx) = ctx();
        std::fs::write(ctx.workspace.join("f"), "x\n").unwrap();
        let started = Instant::now();
        // read, read, bash(sleep .2), read: total should be ~0.2s series;
        // the point is it completes and stays ordered.
        let out = run_batch(
            &ctx,
            vec![
                read("f"),
                read("f"),
                bash("sleep 0.2; echo done"),
                read("f"),
            ],
        );
        assert!(out[2].content.contains("done"));
        assert!(started.elapsed() >= Duration::from_millis(200));
        assert!(!out[3].is_error);
    }

    #[test]
    fn canned_outcomes_slot_in_without_execution() {
        let (_t, ctx) = ctx();
        std::fs::write(ctx.workspace.join("f"), "x\n").unwrap();
        let out = run_batch(
            &ctx,
            vec![
                read("f"),
                Planned::Canned(ToolOutcome::err("repeated identical call")),
                read("f"),
            ],
        );
        assert!(!out[0].is_error);
        assert!(out[1].is_error);
        assert_eq!(out[1].content, "repeated identical call");
        assert!(!out[2].is_error);
    }

    #[test]
    fn partition_groups_match_the_locked_scheduling_semantics() {
        use Kind::*;
        let groups = |kinds: &[Kind]| -> Vec<(Range<usize>, Kind)> {
            partition(kinds)
                .into_iter()
                .map(|g| (g.range, g.kind))
                .collect()
        };
        // Reads group; a mutation is alone; tasks fan out together.
        assert_eq!(
            groups(&[Read, Read, Mutating, Task, Task, Task, Read]),
            vec![(0..2, Read), (2..3, Mutating), (3..6, Task), (6..7, Read)]
        );
        // Free (canned) items join whatever group surrounds them, including
        // a task group, and an all-free batch runs as one read-cap group.
        assert_eq!(groups(&[Task, Free, Task]), vec![(0..3, Task)]);
        assert_eq!(groups(&[Free, Free]), vec![(0..2, Read)]);
        // Free items before a mutation group together; the mutation stays alone.
        assert_eq!(
            groups(&[Free, Mutating, Free]),
            vec![(0..1, Read), (1..2, Mutating), (2..3, Read)]
        );
        // A read run and a task run never merge: their caps differ.
        assert_eq!(groups(&[Read, Task]), vec![(0..1, Read), (1..2, Task)]);
        assert_eq!(groups(&[]), vec![]);
    }

    #[test]
    fn task_calls_classify_as_task_and_everything_else_keeps_its_class() {
        let task = Planned::Run {
            name: "subagent".into(),
            args: json!({"prompt": "x"}),
        };
        assert_eq!(kind(&task), Kind::Task);
        assert_eq!(kind(&read("f")), Kind::Read);
        assert_eq!(kind(&bash("ls")), Kind::Mutating);
        assert_eq!(kind(&Planned::Canned(ToolOutcome::err("x"))), Kind::Free);
    }

    #[test]
    fn more_reads_than_the_cap_still_complete() {
        let (_t, ctx) = ctx();
        for i in 0..20 {
            std::fs::write(ctx.workspace.join(format!("f{i}")), "x\n").unwrap();
        }
        let out = run_batch(&ctx, (0..20).map(|i| read(&format!("f{i}"))).collect());
        assert_eq!(out.len(), 20);
        assert!(out.iter().all(|o| !o.is_error));
    }
}
