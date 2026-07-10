//! Batch scheduler: within one assistant tool batch, consecutive read-only
//! calls run concurrently on scoped threads (cap 8); any mutating call is a
//! sequential barrier executed alone, in order. Results always come back in
//! emission order regardless of completion order: parallelism where it is
//! free, total determinism where it matters (two edits to one file can
//! never race).

use std::sync::atomic::Ordering;

use serde_json::Value;

use noob_provider::http::INTERRUPTED;

use crate::tools::{self, ToolCtx, ToolOutcome};

const READ_CONCURRENCY: usize = 8;

/// One call, pre-processed by the loop: either execute (name, args) or
/// return a canned outcome (doom-loop intercept, unparseable arguments).
pub enum Planned {
    Run { name: String, args: Value },
    Canned(ToolOutcome),
}

impl Planned {
    fn read_only(&self) -> bool {
        match self {
            // Canned outcomes execute nothing; they join any group.
            Planned::Canned(_) => true,
            Planned::Run { name, .. } => tools::is_read_only(name),
        }
    }
}

/// Execute a batch, returning outcomes in emission order. A Ctrl-C observed
/// between barriers or waves cancels every remaining call with a synthetic
/// "canceled by user" result: a mutation must never land AFTER the user
/// canceled, and every call id still gets exactly one result.
pub fn run_batch(ctx: &ToolCtx, batch: Vec<Planned>) -> Vec<ToolOutcome> {
    let mut slots: Vec<Option<ToolOutcome>> = batch.iter().map(|_| None).collect();
    let mut i = 0;
    let n = batch.len();
    // Consume left to right so each Planned is moved exactly once.
    let mut batch: Vec<Option<Planned>> = batch.into_iter().map(Some).collect();
    while i < n {
        if INTERRUPTED.load(Ordering::SeqCst) {
            for slot in slots.iter_mut().skip(i) {
                *slot = Some(ToolOutcome::canceled());
            }
            break;
        }
        let mut j = i;
        while j < n && batch[j].as_ref().unwrap().read_only() {
            j += 1;
        }
        if j == i {
            // A mutating call: alone, in order.
            let planned = batch[i].take().unwrap();
            slots[i] = Some(execute(ctx, planned));
            i += 1;
            continue;
        }
        // The read-only group [i, j), in waves of READ_CONCURRENCY.
        let group: Vec<(usize, Planned)> =
            (i..j).map(|k| (k, batch[k].take().unwrap())).collect();
        for wave in group.chunks(READ_CONCURRENCY) {
            if INTERRUPTED.load(Ordering::SeqCst) {
                for (k, _) in wave {
                    slots[*k] = Some(ToolOutcome::canceled());
                }
                continue;
            }
            // chunks() can't move out; rebind by reference and execute via
            // scoped threads writing into disjoint slots.
            std::thread::scope(|scope| {
                let handles: Vec<_> = wave
                    .iter()
                    .map(|(k, planned)| (*k, scope.spawn(move || execute_ref(ctx, planned))))
                    .collect();
                for (k, handle) in handles {
                    slots[k] = Some(handle.join().unwrap_or_else(|_| {
                        ToolOutcome::err(
                            "the tool crashed while running; this is a noob bug, \
                             try a different approach",
                        )
                    }));
                }
            });
        }
        i = j;
    }
    slots.into_iter().map(Option::unwrap).collect()
}

fn execute(ctx: &ToolCtx, planned: Planned) -> ToolOutcome {
    match planned {
        Planned::Canned(out) => out,
        Planned::Run { name, args } => tools::dispatch(ctx, &name, &args),
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
        Planned::Run { name, args } => tools::dispatch(ctx, name, args),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn ctx() -> (tempfile::TempDir, ToolCtx) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        (tmp, ToolCtx::new(ws, crate::tools::guard::Sandbox::Container))
    }

    fn bash(cmd: &str) -> Planned {
        Planned::Run { name: "bash".into(), args: json!({"cmd": cmd}) }
    }

    fn read(path: &str) -> Planned {
        Planned::Run { name: "read".into(), args: json!({"path": path}) }
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
        // 6 FIFOs; each read blocks on open until a writer arrives. The
        // writer services them in REVERSE emission order, so a serialized
        // scheduler would wedge on the first read forever; only a concurrent
        // group lets the writer reach the last FIFO while the first is
        // still waiting.
        let n = 6;
        for i in 0..n {
            let p = ctx.workspace.join(format!("fifo{i}"));
            let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
        }
        let ws = ctx.workspace.clone();
        let writer = std::thread::spawn(move || {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            for i in (0..n).rev() {
                let p = ws.join(format!("fifo{i}"));
                let deadline = Instant::now() + Duration::from_secs(5);
                // Non-blocking open fails with ENXIO until a reader is
                // present; a bounded retry turns "not concurrent" into a
                // clean panic instead of a hung test.
                let mut file = loop {
                    match std::fs::OpenOptions::new()
                        .write(true)
                        .custom_flags(libc::O_NONBLOCK)
                        .open(&p)
                    {
                        Ok(f) => break f,
                        Err(_) if Instant::now() < deadline => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("reads did not run concurrently: fifo{i}: {e}"),
                    }
                };
                file.write_all(b"data\n").unwrap();
            }
        });
        let out = run_batch(&ctx, (0..n).map(|i| read(&format!("fifo{i}"))).collect());
        writer.join().unwrap();
        assert_eq!(out.len(), n);
        assert!(out.iter().all(|o| !o.is_error), "{:?}", out[0].content);
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
            vec![read("f"), read("f"), bash("sleep 0.2; echo done"), read("f")],
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
