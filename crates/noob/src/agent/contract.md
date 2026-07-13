# noob/src/agent

The inference and tool loop. A user input can run at most 50 rounds.

Each round borrows the system prompt, items, and active schemas through `TurnRequestRef`, streams one provider turn, validates its finish state, persists the completed assistant item, schedules tool calls, appends one result per call, and repeats.

## Scheduler

- Consecutive read-only calls run concurrently on scoped threads, cap 8.
- Consecutive subagent calls form one fan-out group under child concurrency.
- Other mutations are sequential barriers.
- Starts are reported immediately before execution.
- Parallel finishes are reported in actual completion order.
- Returned outcomes and transcript results remain in model emission order.

Canceled outcomes are structural. They are removed from the repeated-call window and never inferred from result text.

## Modes and breakers

Plan mode sends only the read-only tool set and refuses hallucinated mutations. `/go` restores the full set. Identical calls repeated three times within twelve are intercepted; four consecutive errors add a nudge; eight ask the terminal user or abort headless execution.

Compaction prunes old large tool results first, then summarizes and validates the middle if needed. Call/result pairs stay intact, and harness-built pins preserve task, files, loaded skills, and active background job IDs.

The default interactive dock detaches every sub-agent, including `tools: "all"` jobs. Original calls receive immediate acknowledgments. The owner thread injects each terminal result once as a synthetic user item and continues as soon as any job is ready, without waiting for unrelated jobs. Headless and classic-prompt calls remain inline. Each workspace-mutating child tool call contends for the tools layer's cross-process lease, so leased calls do not overlap.

`/clear-plan` explicitly resets the cache prefix by replacing plan arguments and results with placeholders. It preserves call/result structure and persists the replacement as a session reset.

## Persistence and approvals

Every item append checks the session result. A failed session detaches, leaves the in-memory transcript valid, and produces one ordered warning. Skill-write approvals are counted, batch-scoped, and cleared after completion or cancellation.

The system prompt and tool array remain byte-stable except for explicit plan transitions and first live skill-tool registration. Requests remain byte-prefix extensions except at those transitions, compaction, and explicit `/clear-plan` cleanup. Tests enforce the prefix rule.
