# noob/src/agent

The turn loop (P2): build request, stream events, render, execute tool calls,
append results, repeat until a turn ends with no tool calls or a breaker
trips.

Owns: the scheduler (consecutive read-only calls run concurrently, any
mutating call is a sequential barrier, results always append in emission
order), doom-loop breakers, compaction at 75% of NOOB_CTX, prompt assembly,
and interrupt handling.

Invariants: the system prompt and tools array are frozen at session start;
every request is an exact prefix-extension of the previous one; the only
sanctioned prefix breaks are compaction and plan-mode entry or exit, each
logged.
