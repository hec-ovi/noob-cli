# noob/src/agent

The inference and tool loop. A user input can run at most 50 rounds.

Each round borrows the system prompt, items, and active schemas through `TurnRequestRef`, streams one provider turn, validates its finish state, persists the completed assistant item, schedules tool calls, appends one result per call, and repeats.

## Scheduler

- Consecutive read-only calls run concurrently on scoped threads, cap 8.
- Detached subagent admissions and controls execute in emission order; the hub runs admitted children concurrently under the child cap. Inline child calls retain bounded fan-out.
- Other mutations are sequential barriers.
- Starts are reported immediately before execution.
- Parallel finishes are reported in actual completion order.
- Returned outcomes and transcript results remain in model emission order.

Canceled outcomes are structural. They are removed from the repeated-call window and never inferred from result text.

## Modes and breakers

Plan mode sends only the read-only tool set and refuses hallucinated mutations. `/go` restores the full set. Identical calls repeated three times within twelve are intercepted; four consecutive errors add a nudge; eight ask the terminal user or abort headless execution.

Compaction prunes old large tool results first, then summarizes and validates the middle if needed. Call/result pairs stay intact, and harness-built pins preserve task, files, loaded skills, and active background job IDs.

The default interactive dock detaches every sub-agent. Original calls receive immediate acknowledgments. The main agent can process ordinary human turns while several children run, and child completion does not interrupt an active parent turn. The owner thread injects each terminal result exactly once as a synthetic user item without waiting for unrelated jobs. When all coalesced ready results succeeded, parent inference starts only if the human is not composing a message; their submitted turn otherwise integrates the packets first. Any error or cancellation in the drained batch remains visible and durable without invoking parent inference. Cancel acceptance and terminal child failure close a same-turn replacement gate until the next human input. Headless and classic-prompt calls remain inline. File-tool mutations contend for the cross-process workspace lease. Bash remains concurrent for builds, tests, and exploration; the system contract directs source changes through write/edit. Background jobs and the foreground plan are independent state machines and dock regions; they may coexist, and agent lifecycle is never represented as plan items.

`/clear-plan` explicitly resets the cache prefix by replacing plan arguments and results with placeholders. It preserves call/result structure and persists the replacement as a session reset.

## Persistence and approvals

Every item append checks the session result. A failed session detaches, leaves the in-memory transcript valid, and produces one ordered warning. Skill-write approvals are counted, batch-scoped, and cleared after completion or cancellation.

The system prompt and tool array remain byte-stable except for explicit plan transitions and first live skill-tool registration. Requests remain byte-prefix extensions except at those transitions, compaction, and explicit `/clear-plan` cleanup. Tests enforce the prefix rule.
