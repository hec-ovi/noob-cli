# noob/src/agent

The turn loop: build request, stream events, render, execute tool calls,
append results, repeat until a turn ends with no tool calls or a breaker
trips (cap: 50 rounds per user input).

Owns: the scheduler (`sched.rs`: consecutive read-only calls run
concurrently on scoped threads, cap 8; any mutating call is a sequential
barrier; results always append in emission order), doom-loop breakers
(identical call 3x within the last 12 intercepts; 4 consecutive errors
inject a nudge; 8 pause the REPL or abort headless runs), compaction
(`compact.rs`: at 75% of NOOB_CTX, middle summarized, head + ~20k-token
tail kept, call/result pairs never split, hard-drop only when the
summarize itself overflows), prompt assembly (`prompt.rs`, once per
session), and interrupt handling (partial turns discarded; parsed-but-
unexecuted calls get synthetic "canceled by user" results; the flag is
cleared so the session continues).

Invariants: the system prompt and tools array are frozen at session start;
every request is an exact prefix-extension of the previous one; the only
sanctioned prefix breaks are compaction and plan-mode entry or exit, each
logged as `cache prefix reset: <reason>`.
