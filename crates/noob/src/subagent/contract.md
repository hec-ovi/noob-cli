# noob/src/subagent

The subagent tool spawns `current_exe() child` with one JSON task on stdin. The child gets a fresh context, sends progress to stderr, and writes exactly one JSON result line to stdout.

The default interactive dock detaches every call through a session-scoped hub. Each original call receives one running-job result immediately. Final output later enters exactly once as a framed, untrusted synthetic user report, never as a second result for the call ID. A fixed FIFO worker pool enforces the child cap, the pending queue is bounded, and `/agents` supports per-job cancellation. Any ready result is deliverable without waiting for unrelated jobs. Background workers never touch the parent transcript, session file, provider instance, UI, or input loop.

The main agent can process ordinary human turns while multiple children remain in flight. Child completion does not interrupt an active parent turn. At the idle prompt, a typed draft or submitted message wins the race with a ready result and integrates that result before the human text in the ordinary turn. Successful ready batches otherwise trigger one automatic continuation; any error or cancellation in a drained batch leaves the prompt idle.

Cancel acceptance and terminal failure or cancellation close the replacement gate under the hub state lock. A later spawn in the same model batch or automatic turn is rejected until a new human input begins. The terminal packet for the failed or canceled job is still delivered exactly once. Background jobs and the foreground plan are independent state machines that may coexist; agent lifecycle is never represented as plan items.

The child profiles are:

- `tools: "read-only"`: `read`, `grep`, `glob`, `ls`, `context`, and `skill` when one is available.
- `tools: "web"`: the read-only set plus `mcp_connect` and `mcp_call`. It requires one unique configured name after case-folding and removing `-` and `_` from `websearch`, filters MCP access to that server, and exposes no Bash, write, edit, plan, or subagent tool.
- `tools: "all"`: the full registered tool set. A dock child remains a structural leaf.

Loaded ancestor skills are excluded from descendant discovery. A completed web child needs at least two distinct `mcp_call` IDs followed by server-originated MCP results. Local validation, connection, and transport failures do not count; server-declared tool errors do. If the first completion has fewer, the same child receives one corrective evidence instruction using only its remaining original round budget. Exhausted budget or a second unsupported completion produces an error; aborted or interrupted runs are not retried.

Full-tool children can use Bash, MCP, web search, and file tools. Each child `write` or `edit` call takes the workspace directory lease, so file-tool mutations do not overlap while inference, Bash, and read-only work continue concurrently. A child waits for the lease for a bounded interval; a root write/edit reports a conflict instead of waiting behind a child. Shell mutations are outside the lease guarantee. The lease covers one file-tool call, not a whole child task, and ends when that call returns. Exec, piped, classic-prompt, and nested-child calls remain inline. On cancellation or parent death, Linux cleanup signals the child process group and kills descendants still attached to it before reaping. Shutdown also cancels, reaps, and joins every child.

Defaults are four concurrent children, 25 inference rounds, and 300 seconds. Dock children are structural leaves, so the visible root fleet cannot multiply or oversubscribe the slots validated by `noob doctor`. Inline surfaces retain recursion depth two, with one-at-a-time nested delegation at depth 1. Children default to read-only tools and have no TTY.

The parent starts the wall clock before sending input. Child stdin is nonblocking, checks cancellation and deadline while writing, and closes on completion. The child result is read without an output-length cap. The dock retains a bounded recent progress window for the persistent Tab view while stderr continues to drain. Timeout and cancellation kill and reap the process group.

These are execution budgets, not model output-token limits.
