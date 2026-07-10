# noob-cli ARCHITECTURE.md

## Goal

noob-cli is an extremely lightweight general-purpose agentic CLI: one static Rust binary that lives in a Docker sandbox, works on a bind-mounted `/work` folder, speaks both OpenAI Chat Completions and Responses APIs against any base URL, and ships skills, MCP, plan mode, parallel tool calls, and self-spawned sub-agents.
Small local models (qwen-class through llama.cpp) are the first-class target: every choice below optimizes for byte-stable prompt prefixes, a tiny measured prompt budget, wire-quirk tolerance, and error messages that teach the model its next move.

## Synthesis basis and resolved judge disagreements

The base is the ruthless-minimalist proposal (ranked first by judges 1 and 2). The wire-protocol proposal's provider layer is adopted nearly wholesale as the spec for the provider crate (endorsed by all three judges). The agent-loop-quality edit engine, scheduling semantics, and truncation policy are grafted in, as is the extensibility lens's headless integration surface. Disagreements resolved:

- **Winning base**: ruthless-minimalist (2 of 3 judges); judge 3's wire-protocol pick is honored by adopting its provider layer, fixtures, and named tests wholesale inside the minimalist skeleton.
- **Crate layout**: 2 shipped crates + 1 dev-only testkit in a workspace, not the winner's single crate; this keeps PLAN P0's workspace requirement (judge 3's objection), enables the cargo-metadata egress test (judges 1-3 graft), and stays at minimalist weight (judge 1 suggested "single-crate-or-2-crate").
- **HTTP stack**: blocking ureq (judges 1 and 2) over hyper+tokio (judge 3's winner); the one hole judges found in the blocking story (first-byte vs idle timeout, dead Ctrl-C during prompt processing) is fixed with a tick-read watchdog, which judge 2 confirms is implementable on ureq.
- **Plan-mode gating**: structural schema removal (minimalist, grafted by judge 3) over wire-protocol's dispatch gating; all three judges noted visible write/edit schemas tempt qwen-class models into multi-minute rejected round-trips, which costs more than one cache bust.
- **Plan approval**: explicit `/go` from the user (extensibility lens); both automatic triggers were judge-criticized (minimalist's "turn ends with no tool calls" misfires, agent-loop's `submit_plan` dead-ends when the model prints its plan as text).
- **MCP tool shape**: two fixed tools `mcp_connect` + `mcp_call` (agent-loop, grafted by judge 3); minimalist's per-tool schema registration reinvents the eager dump and busts the cache, and the single overloaded `mcp` tool's presence-of-field discriminator is a small-model trap (judges 2 and 3).
- **Token counting**: offline tiktoken o200k ceilings as a conservative gate plus live llama-server `/tokenize` checks against the real qwen tokenizer, both measured on the shipped artifact via `noob debug prompt --json`; this replaces both the chars/3.5 heuristic (judge 1 objection) and trusting o200k alone (judge 2 objection).
- **bash truncation direction**: tail-heavy 8 KiB head + 16 KiB tail (agent-loop, grafted by judge 3) over the winner's middle-truncated 64 KiB (too large, judge 3) and wire-protocol's head-heavy split (verdicts live at the tail).

## Locked decisions

| # | Decision | Choice | Why |
|---|---|---|---|
| 1 | Crate layout | Cargo workspace: `noob` (bin) + `noob-provider` + `noob-testkit` (dev-only) | Keeps PLAN's workspace, enables the cargo-metadata egress test, near-minimalist weight; extraction of more crates later is mechanical |
| 2 | Async runtime | None; blocking IO + `std::thread` | One inference at a time; threads cover tool fan-out and children; tokio would be the largest shipped subtree for work we do not have |
| 3 | HTTP client + TLS | ureq 3.x (default-features off) + rustls with the ring provider + compiled-in webpki-roots | Cleanest musl story (no cmake, no system cert store); feature wiring pinned first thing in P0 |
| 4 | Timeouts | connect 10 s, first-byte 300 s, inter-chunk idle 90 s, via a 1 s tick-read watchdog | A single read timeout either kills legitimate 131k-ctx llama.cpp prompt processing or never catches a stalled stream (all judges) |
| 5 | UI | Opt-in termios line editor at an interactive tty (boxed prompt, real editing); cooked `read_line` when piped; no ratatui, no rustyline | Zero-dep editor over libc, restored by three hooks; TUIs add a redraw bug class; piped/exec/child stay byte-identical |
| 6 | Tool set | read, write, edit, bash, grep, glob, ls + skill + mcp_connect/mcp_call + task | PLAN's seven plus fixed infra tools that keep the tools array byte-stable for the whole session |
| 7 | Edit engine | Exact string replace + 3-stage ladder (trailing whitespace, unicode punctuation, uniform indent shift), hard ambiguity rejection, fnv1a64 check-and-set | Indent shift is the dominant qwen edit failure; no similarity-score fuzzing ever (silent corruption is unrecoverable) |
| 8 | Parallel tool semantics | Consecutive read-only calls run concurrently (cap 8); any mutating call is a sequential barrier; results appended in emission order | Fixes the winner's unguarded all-N-concurrent race (judge 2: disqualifying if unfixed) |
| 9 | System prompt budget | base <= 500 tokens, env block <= 60, full tool schema array <= 940, total fixed overhead <= 1,500 | Enforced offline (tiktoken o200k) and live (llama-server /tokenize) on the shipped artifact |
| 10 | api_style | Host `api.openai.com` -> responses; anything else -> chat; `NOOB_API_STYLE` override | The field-proven convention (Zerostack, Pi); the entire compat matrix, no probe requests |
| 11 | Config | `/config` bind mount; flat `.env` re-read on every request; `mcp.json`; no TOML/YAML crates | Hot reload for free; secrets never enter the process environment; model switch is a text edit |
| 12 | Skills | 4 discovery paths, agentskills.io frontmatter, 3-level disclosure, body delivered as a tool result (24 KiB cap) | The prompt head never mutates; ecosystem-compatible (`.claude/skills/`, `.agents/skills/`) |
| 13 | MCP | Two fixed tools `mcp_connect`/`mcp_call`; stdio + Streamable HTTP through noob-provider's http/sse modules | Tools array never changes mid-session; lazy to the bone (Zap); egress invariant holds mechanically |
| 14 | Plan mode | Structural removal of mutating schemas + injected user-role mode message; explicit `/go` approval | Absent schemas cannot tempt small models; one accepted cache bust on entry and exit |
| 15 | Multi-agent | Self-spawn via `current_exe() child`; JSON task on stdin, one JSON result line on stdout, progress on stderr; caps 4 concurrent / 25 turns / 300 s / depth 2 | The process boundary is the context boundary; argv+pipes survive being wrapped by anything |
| 16 | Docker | 2-stage alpine musl build; runtime = alpine + bash + git + ca-certificates; non-root compose user; `network_mode: host` | Non-root fixes root-owned bind-mount files (judge 2: real daily pain); host network reaches llama.cpp :8090 and MCP :8000 |
| 17 | Headless surface | `noob exec -p ... [--json] [--session <id>]` JSONL event protocol; `noob debug prompt --json`; `noob doctor` | The concrete integration surface telegram-bot-skill needs (PLAN names it); budget tests measure the shipped artifact |
| 18 | Onboarding | No wizard; localhost endpoint autodetect when unconfigured; committed commented `.env.example` | PLAN hard requirement: compose run lands in a working chat with no mandatory config step |

## Folder tree

Every folder ships a `contract.md`: purpose, public interface, invariants, nothing about siblings.

```
noob-cli/
├── contract.md              # repo: what noob-cli is, how to build/test/run; "no CI, cargo test is the whole story"
├── Cargo.toml               # workspace manifest + release profile (opt-level="s", lto="fat", panic="abort", strip)
├── Makefile                 # build, test, smoke (NOOB_LIVE=1), docker, size-check
├── config/
│   └── contract.md          # committed .env.example (commented) and default compose mount target; never real secrets
├── docker/
│   └── contract.md          # Dockerfile (2-stage musl) + compose.yml; the image contains zero state, config, or keys
├── docs/
│   └── contract.md          # RESEARCH.md, ARCHITECTURE.md; design record, not runtime input
└── crates/
    ├── contract.md          # crate map; dependency direction noob -> noob-provider; noob-testkit is dev-only
    ├── noob/
    │   ├── contract.md      # the binary; argv dispatch only: repl | exec | child | doctor | debug | --version
    │   ├── prompts/         # contract.md: base.md + compact.md, compiled in via include_str!, budget-tested
    │   ├── src/
    │   │   ├── agent/       # contract.md: turn loop, scheduler, doom-loop guard, compaction, prompt assembly, interrupts
    │   │   ├── tools/       # contract.md: 7 built-ins + registry; pure fn(args) -> ToolResult; truncation policy; no loop knowledge
    │   │   ├── skills/      # contract.md: SKILL.md discovery (4 paths), frontmatter micro-parser, skill tool
    │   │   ├── mcp/         # contract.md: JSON-RPC framing, stdio + Streamable HTTP (HTTP via noob-provider), lazy connect
    │   │   ├── task/        # contract.md: task tool, child stdin/stdout protocol, caps
    │   │   ├── config/      # contract.md: precedence, endpoint autodetect, mcp.json, hot-reload semantics
    │   │   ├── session/     # contract.md: append-only JSONL transcripts under /config/sessions/; resume
    │   │   └── ui/          # contract.md: REPL input (cooked when piped, opt-in termios line editor at a tty), streaming stdout writer, ANSI, y/N prompts, JSONL event emitter
    │   └── tests/           # contract.md: e2e through the compiled binary vs noob-testkit; live smoke opt-in (NOOB_LIVE=1)
    ├── noob-provider/
    │   ├── contract.md      # transcript in, event stream out; both wire shapes; sole owner of ureq; .env parser; no fs/tool knowledge
    │   └── src/             # types.rs http.rs sse.rs chat.rs responses.rs assemble.rs retry.rs envfile.rs
    └── noob-testkit/
        ├── contract.md      # dev-only; hand-rolled mock OpenAI server (both shapes), scripted turns, request recorder, automatic assertions
        └── testdata/sse/    # SSE byte transcripts with %%CHUNK%% split sentinels (provenance in its contract.md)
```

## Dependencies

Direct runtime deps, the whole list:

| Crate | Features | Why |
|---|---|---|
| ureq | default-features off; rustls (ring provider) + webpki-roots | Blocking HTTP/1.1, streaming body reader, per-call timeouts; only `noob-provider` may depend on it (test-enforced) |
| serde_json (+serde, NO derive) | - | Tolerant Value-based wire parsing, which heterogeneous servers demand anyway; skipping derive drops syn/quote/proc-macro2 |
| regex | - | grep tool correctness |
| ignore (brings globset, walkdir) | parallel-walk off | Gitignore-aware grep/glob; matches inside node_modules/target poison a 131k window faster than anything else (judge 2 graft) |
| libc | - | ~15-line SIGINT handler setting an AtomicBool, plus kill(-pgid) for bash/MCP timeout enforcement |

Dev-dependencies only: tiktoken-rs (o200k_base, budget tests), tempfile. Hand-rolled instead of crates: argv dispatch (~80 lines), .env parser (~60 lines), YAML frontmatter scanner (~120 lines), JSON-RPC framing (~150 lines), SSE parser (~150 lines), fnv1a64 (~10 lines), backoff jitter (xorshift from SystemTime nanos). Explicitly rejected: tokio, hyper, reqwest, clap, rustyline, toml, any YAML crate, dotenvy, any eventsource crate, anyhow/thiserror, rand, chrono, tracing, crc32fast.

Footprint budgets, enforced by `make size-check`: runtime crate graph <= 45, stripped musl binary <= 8 MB, idle RSS <= 25 MB, cold start to first prompt < 50 ms.

## Provider layer (crates/noob-provider)

The wire-protocol proposal's provider design, ported to blocking IO.

### Trait and types

```rust
pub trait Provider {
    /// Blocking; invokes `on` for each stream event; returns the assembled assistant turn.
    fn stream(&self, req: &TurnRequest, on: &mut dyn FnMut(Event)) -> Result<Turn, ProviderError>;
}
pub enum Event {
    Text(String), Reasoning(String),
    ToolCallStart { index: u32, id: String, name: String },
    ToolArgsDelta { index: u32, delta: String },
    Usage(Usage),                    // prompt, completion, cached_prompt
    Done(Finish),                    // Stop | ToolCalls | Length | ContentFilter | Error
}
pub struct Turn { pub text: String, pub reasoning: Option<Reasoning>, pub tool_calls: Vec<ToolCall>, pub usage: Option<Usage>, pub finish: Finish }
```

`ProviderError` is a hand-rolled enum (typed wire errors, never stringly). Adapters are pure functions of (transcript, SSE bytes): fixture-replayable with zero mocks. Each adapter has a `finish()` pass because several backends end streams without `[DONE]` or without closing tool-call state: every started call must have complete, parseable args; `ToolCallDone` is synthesized if the server never signaled it.

### HTTP transport and timeouts

One in-flight request per agent; keep-alive to localhost via a persistent ureq agent. Three timeouts: connect 10 s; first-byte 300 s (llama.cpp prompt processing on a 131k-ctx request legitimately takes minutes before token one); inter-chunk idle 90 s. Mechanism: the response body is read through a 1 s tick loop (socket read timeout 1 s; a timeout tick is not an error). Each tick checks the SIGINT flag, the first-byte deadline, and the idle deadline (reset on every received byte); tripping any of the three closes the socket and surfaces the corresponding typed error. This makes Ctrl-C responsive within 1 s even during minutes of silent prompt processing. If ureq's body reader cannot resume across a timeout tick, the fallback is a custom ureq transport that owns the TcpStream and implements the tick loop below ureq; this is verified in P0 alongside the feature wiring.

### SSE parser (sse.rs), owned and byte-exact

~150 lines plus exhaustive table tests: incremental UTF-8 decoding (TCP chunks split multibyte codepoints; trailing incomplete sequences buffered across reads); event framing across chunk boundaries; multiple `data:` lines per event joined with `\n`; optional space after colon; CRLF and LF; BOM stripped on first chunk; comment lines (`:` prefix) ignored (OpenRouter's `: OPENROUTER PROCESSING` keepalives crash naive parsers); `event:` field captured (Responses routes on it); `retry:`/`id:` tolerated and dropped; `data: [DONE]` terminates chat streams and its absence is handled by `finish()`. Content-type guard: a 200 with `application/json` on a `stream: true` request is parsed as a single non-streamed completion, not fed to the SSE parser. The same parser serves MCP Streamable HTTP; there is exactly one SSE parser in the binary.

### Chat Completions adapter

Request: POST `{base}/chat/completions`, `stream: true`, `stream_options: {"include_usage": true}`. Never any of `max_tokens`/`max_completion_tokens`/`max_output_tokens`; there is no config knob for one. No `parallel_tool_calls` field sent (several OSS servers 400 on it). Reactive compat: if a 400 names a top-level field we sent (in practice `stream_options`), strip it, retry once, remember for process lifetime only; no persisted quirk registry, no startup probe.

Delta assembler (assemble.rs), a state machine keyed by `tool_calls[].index`, hardened against the full quirk matrix:
- Standard flow: first delta for an index carries id + name; later deltas append `function.arguments` fragments.
- Missing `index` (some proxies, older Azure/Mistral): attribute to the most recently opened call; if none open, open index 0.
- Absent or empty `id` (llama.cpp with some templates): synthesize `call_<turn>_<index>`; a tool result requires `tool_call_id`, so an id must always exist.
- Repeated id/name in every delta: ignore after first.
- `arguments` as a JSON object instead of a string (live llama.cpp regression, ggml-org/llama.cpp issue #20198): re-serialize canonically to a string. Same tolerance for non-streamed responses.
- Whole call in one delta, or the whole `tool_calls` array only in a final non-delta `choices[].message`: accepted at any point.
- Text deltas interleaved with tool-call deltas, and multiple concurrent indexes, preserved in arrival order.
- In-band mid-stream `error` payloads (OpenRouter emits these after streaming starts): Done(Error) with the message, never a panic.
- finish_reason mapping: `tool_calls`/`function_call` -> ToolCalls; `stop` -> Stop; `length` -> Length. Since output is never capped, Length means the context is full: it triggers compaction, not retry.
- `finish()` validation: every open call's args must parse as JSON; one mechanical repair (strip markdown fences, trim); if still invalid the call is dispatched as an error tool result ("arguments were not valid JSON: <parse error>; re-issue the call") so the model self-corrects.

Reasoning: accept `delta.reasoning_content` (DeepSeek convention, llama.cpp, vLLM) and `delta.reasoning` (OpenRouter), emit Reasoning events, keep in the transcript for display, never serialize back in subsequent requests (DeepSeek rejects it; llama.cpp templates re-inject thinking themselves). No `resend_reasoning` option in v0.1 (judge 1: speculative).

Serialization back: ToolCall items become an assistant message with a `tool_calls` array (args always a string); each ToolResult becomes `{"role":"tool","tool_call_id":...,"content":...}` in the same index order the calls were emitted.

### Responses API adapter

Request: `instructions` = the system prompt (byte-identical across turns), `input` = item array (`message`, `function_call`, `function_call_output`, `reasoning` items replayed verbatim from their captured wire form), `store: false` always (stateless full-input replay preserves append-only and works on vLLM), `stream: true`. Against `api.openai.com` additionally `include: ["reasoning.encrypted_content"]`. No `previous_response_id` ever.

Event routing on the SSE `event:` field: `response.output_item.added` (type function_call -> ToolCallStart), `response.function_call_arguments.delta`/`.done`, `response.output_text.delta`, `response.reasoning_text.delta` and `response.reasoning_summary_text.delta` -> Reasoning, `response.output_item.done` (capture completed reasoning items verbatim), `response.completed` -> Usage + Done, `response.failed`/`response.incomplete`/`error` -> Done(Error). Unknown event types are ignored (the vocabulary grows; a new event must never crash the client). Validated against the mock and the local vllm-qwen `/v1/responses` (llama.cpp does not serve it).

### Retry and backoff

Retries only before the first streamed content byte: connect/TLS errors, 408, 425, 429, 5xx. 3 attempts, 1 s / 2 s / 4 s with full jitter, `Retry-After` honored up to 60 s. Mid-stream death after content surfaces as a turn error the user retries; silent mid-stream retry would duplicate output. Other 4xx surface immediately (except the one-shot compat field-strip retry).

### Append-only cache discipline (tested invariant)

The system prompt and tools array are frozen at session start; the environment block (date, cwd, model) is computed once at session start, never per request, or every day rollover would bust the cache mid-session. Messages only append; tool results are truncated once at emission and byte-frozen. Every request is an exact prefix-extension of the previous one. Sanctioned prefix breaks, exactly two, each logging `cache prefix reset: <reason>`: compaction, and plan-mode entry/exit. Payoff: llama.cpp prefix KV reuse makes turn N+1 cost only the new suffix; OpenAI/OpenRouter prompt caching hits at Codex-reported rates. Usage events (including `cached_tokens`) feed `/status`; 0% cache hits on turn 5 is a live serializer-bug alarm.

## Config and secrets (crates/noob/src/config + noob-provider/src/envfile.rs)

Config dir: `/config` in the container (bind mount), `NOOB_CONFIG_DIR` override, fallback `~/.config/noob` outside Docker.

```
/config/
├── .env          # KEY=VALUE; # comments; optional quotes; no interpolation
├── mcp.json      # MCP servers
├── AGENTS.md     # optional global instructions
├── skills/       # global skills
└── sessions/     # per-session JSONL transcripts (state lives here, never in the image)
```

`.env` keys: `NOOB_BASE_URL`, `NOOB_API_KEY`, `NOOB_MODEL`, `NOOB_API_STYLE`, `NOOB_CTX` (default 131072), `NOOB_TASK_CONCURRENCY`, `NOOB_TASK_MAX_TURNS`, `NOOB_SANDBOX`.

Lazy mechanics: the ~60-line parser in `noob-provider::envfile` opens and parses `/config/.env` inside every request build, resolves the needed keys, and drops the parse. Nothing is cached, so editing `.env` on the host applies on the very next API call (hot reload with no restart, test-enforced by the named `hot_reload_env` e2e). Secrets never enter the process environment, so the bash tool's subprocesses, MCP stdio servers, and child agents cannot read them by accident. Only the config-dir `.env` is read, never `/work/.env`, which belongs to the user's project. A parse error keeps the previous good values and prints one warning line.

Precedence (highest wins): CLI flag (`--model`, `--base-url`) > process env var (non-secret settings only) > `/config/.env`. No project-local env overlay in v0.1.

Endpoint autodetect (PLAN zero-friction requirement): when `NOOB_BASE_URL` is unset, probe localhost candidates in order with a 500 ms connect timeout and `GET /v1/models`: `:8090`, `:8080` (llama.cpp), `:11434/v1` (Ollama), `:1234/v1` (LM Studio), `:8000/v1` (vLLM). First responder wins; one line printed naming it. Loopback only, only when unconfigured; never a remote call.

`noob doctor`: checks config dir presence and writability, `.env` parse, endpoint reachability (`GET {base}/models`, 2 s timeout), api_style sanity, `mcp.json` parse, `/work` writability; each failure prints one line stating the fix. Every error message in the binary states its remedy (PLAN requirement).

## System prompt (crates/noob/prompts + agent/prompt assembly)

Assembled once per session in fixed order (order is a cache invariant):
1. `prompts/base.md` via `include_str!`: identity ("noob, a coding agent running inside a sandbox container, cwd /work"), edit discipline (read before edit; make `old` unique with 3-8 lines of context; prefer edit over write), parallel-call encouragement (batch independent reads in one message), verification norm (run tests/build after changes). No word/sentence/length caps anywhere in the text; output is shaped by content instruction only. Budget <= 500 tokens.
2. Environment block: cwd, platform, date, model, sandbox flag; computed once at session start. <= 60 tokens.
3. AGENTS.md: `/config/AGENTS.md` (global) then `/work/AGENTS.md` (project), each hard-capped at 16 KiB with a truncation notice.
4. Skills index: a `# Skills (resolver)` section opening with the dispatcher instruction ("Match the task against these skills. Load a matching skill with the skill tool and follow it before acting; if two match, load both."), then one `- name: description` line per discovered skill (description clipped at 200 chars); section capped at 1,000 tokens; overflow skills get name-only lines, then a count note. The index is the resolver (GBrain RESOLVER.md / thin-harness-fat-skills pattern): descriptions are the triggers, bodies cost zero tokens until loaded.
5. One line naming configured MCP servers when `mcp.json` has any: `MCP servers (use mcp_connect): websearch, fs`.

Plan mode never touches the head: it is an injected user-role message plus a tools-array change (see Plan mode).

Locked budget: layers 1+2 <= 560 tokens; serialized tool schema array (all registered tools including skill, mcp_connect, mcp_call, task) <= 940 tokens; total fixed first-request overhead <= 1,500 tokens, about 1.1% of qwen's 131k ctx. Enforcement is threefold: (a) an offline test runs the real binary's `noob debug prompt --json` and tokenizes with tiktoken o200k as a conservative ceiling; (b) the live smoke suite POSTs the assembled prompt to llama-server `/tokenize` and asserts the same ceilings against the real qwen tokenizer; (c) a lint asserts the prompt text contains no forbidden cap phrasing (`in N words`, `max N sentences`, "keep it brief") and the request JSON contains no `max_tokens`-family key. Budget numbers live in one `const` block so raising them is a visible diff.

## Agent loop (crates/noob/src/agent)

Turn machine: build request -> stream events -> render -> execute tool calls -> append results -> repeat until a turn ends with no tool calls or a breaker trips.

**Scheduling**: within one assistant tool batch, calls are partitioned in emission order. Consecutive read-only calls (read, grep, glob, ls, skill, mcp_connect) run concurrently on `std::thread::scope` (cap 8). Any mutating call (write, edit, bash, mcp_call, task) is a sequential barrier executed alone in order, except that multiple `task` calls in one batch are recognized as one fan-out group and run concurrently up to the child cap. Results are always appended in emission order, one tool message per call id, regardless of completion order: parallelism where it is free, total determinism where it matters (two edits to one file can never race).

**Doom-loop breakers** (all thresholds locked): identical (tool, canonical-args-hash) 3 times within the last 12 calls intercepts execution and returns "repeated identical call; the result will not change; take a different approach". Edit-retry escalation: the 2nd failed edit on the same (path, old) returns the actual file region (up to 40 lines) around the best anchor so the error contains the ground truth; the 3rd suggests re-read then region rewrite. 4 consecutive tool errors of any kind injects a course-correct nudge; 8 pauses and asks the user (REPL) or aborts with a structured error (exec). Turn cap: 50 inference rounds per user input.

**Interrupts**: first Ctrl-C during a stream aborts the HTTP request via the watchdog (responsive within 1 s even during prompt processing). A partial turn interrupted mid-stream is discarded entirely (never committed, so the prefix stays replayable) and a user note `[interrupted]` is appended. If the turn had completed and tool calls were parsed but not yet executed, each receives a synthetic tool result "canceled by user" so the transcript stays API-valid for the next request. Second Ctrl-C hard-exits.

**Compaction** (the sanctioned prefix break): context usage is estimated from the last server-reported usage plus chars/4 for the delta since; at 75% of `NOOB_CTX` the middle of the transcript is LLM-summarized with `prompts/compact.md` into one spliced message, keeping the system head and the most recent ~20k tokens intact and never splitting a call/result pair. The summary preserves the task statement, decisions, files touched, unresolved errors, and re-lists loaded-skill names (names only) so the model does not forget what it loaded. A provider context-overflow 400 or `finish_reason: length` triggers one emergency compaction and retry; if the summarization request itself overflows, fall back to deterministic hard-drop of oldest turns with a stub note.

P7 amendment (design record: `.research/context-compaction-survival`): compaction is a ladder. Old large tool results in the middle are first replaced with one-line placeholders; when that alone brings usage under 60% no summarize call happens at all. Otherwise the middle (including any previous summary, so cycles merge) is summarized against a fixed section schema (prompts/compact.md) and validated deterministically before splicing: empty or non-shrinking output retries once, then prunes or hard-drops. Every spliced message ends with a harness-built pinned block ([task], [files touched], [loaded skills]) recovered from ground truth and carried verbatim across cycles and resumes. A transport failure sets a backoff so a failing summarizer is not re-invoked every round.

**Session log**: every session appends JSONL events to `/config/sessions/<id>.jsonl` (unix-millis hex id) from P2. `noob exec --session <id>` resumes by replaying the transcript.

## Tools (crates/noob/src/tools)

Registered set is decided at session start and stays byte-stable: 7 core tools always; `skill` only when at least one skill is discovered; `mcp_connect`/`mcp_call` only when `mcp.json` has servers; `task` from P6, absent at max depth. Descriptions are <= 20 words each.

```jsonc
read        {"path": "string", "offset": "int?", "limit": "int?"}     // plain text, NO line numbers
write       {"path": "string", "content": "string"}
edit        {"path": "string", "old": "string", "new": "string", "all": "bool?"}
bash        {"cmd": "string", "timeout_s": "int?"}                     // bash -c, merged output, default 120s max 600s, kill process group on expiry
grep        {"pattern": "string", "path": "string?", "glob": "string?", "ignore_case": "bool?"}
glob        {"pattern": "string"}                                      // paths, mtime-sorted, newest first
ls          {"path": "string?"}
skill       {"name": "string"}
mcp_connect {"server": "string"}
mcp_call    {"server": "string", "tool": "string", "args": "object"}
task        {"prompt": "string", "tools": "\"read-only\"|\"all\"?", "max_turns": "int?"}
```

No line numbers anywhere: `read` returns raw lines with a one-line header (`src/main.rs lines 1-500 of 1240`). Line-number prefixes are the most common contaminant of small-model `old` strings; the failure mode is removed at the source. grep and glob are gitignore-aware via the `ignore` crate.

**Edit engine**: exact string replace with a deterministic ladder and hard ambiguity rejection.
1. Exact byte match. Exactly 1 occurrence: apply. More than 1 without `all: true`: reject with "old matched N locations; add surrounding lines to make it unique". Zero: descend.
2. Stage A: per-line trailing whitespace stripped on both sides.
3. Stage B: typographic normalization (smart quotes, unicode dashes, NBSP -> ASCII) on a shadow view with a byte-offset map back to the original; matching on the shadow, splicing on the original bytes; `new` spliced verbatim.
4. Stage C, uniform indent shift: if `old`'s lines match a contiguous region modulo one constant leading-whitespace delta, accept and re-indent `new` by the same delta (the dominant qwen-class failure, fixed losslessly).
5. Every stage independently enforces uniqueness; a stage finding multiple candidates rejects and does not fall through. No similarity-score fuzzing (no Levenshtein thresholds), ever.
6. Failure teaching: when the whole ladder misses, the error locates the closest region by anchor-line match and returns a character-level diff of that region against `old`. Goal: 2-retry convergence because the error contains the ground truth. Which fallback stage fired is logged into the tool result so the model learns.

Check-and-set staleness: `read` records `(path, len, fnv1a64)` in a session registry; successful write/edit updates it. `edit` on a never-read path is rejected ("read the file first"). `write`/`edit` on a path whose current hash differs from the recorded one fails with "file changed on disk since your last read; re-read it". Atomicity: temp file in the same directory, fsync, rename over target, mode preserved; no partial write is ever visible. Hashedit (CRC line tags) stays the documented post-v0.1 upgrade behind the same tool name.

**Truncation** (applied once at emission, then byte-frozen in the transcript):

| Tool | Cap | Shape |
|---|---|---|
| read | 500 lines default, 500 chars per line, 40 KiB hard | header states total line count for paging with offset/limit |
| bash | 24 KiB: head 8 KiB + tail 16 KiB | tail-heavy; compilers and test runners put the verdict last |
| grep | 100 matches or 16 KiB | always ends with the total count: "312 matches, showing 100; narrow the pattern or add a glob" |
| glob / ls | 200 entries | count note |
| skill | 24 KiB | pointer to read the remainder with `read` |
| mcp_call | 20 KiB head+tail | wrapped in untrusted-content delimiters |

Every truncation marker names the next action, and marker phrasing is frozen in golden tests: error text is API surface for small models.

## Skills (crates/noob/src/skills)

Discovery at session start, first hit per name wins: `/work/.noob/skills/`, `/work/.claude/skills/`, `/work/.agents/skills/`, `/config/skills/`. Each candidate is `<dir>/<skill>/SKILL.md` per agentskills.io: frontmatter with required `name` (<= 64 chars, lowercase+hyphens) and `description` (<= 1024 chars), parsed by the hand-rolled ~120-line scanner (plain scalars, quoted strings, `|`/`>` blocks); malformed skills are skipped with a stderr warning, never a crash.

Disclosure: L1 is the one-line-per-skill index in the prompt (capped, see System prompt). L2 is the `skill {name}` tool returning the SKILL.md body (frontmatter stripped) plus the skill's directory path as a tool result, capped at 24 KiB, with the standard's ~5k-token recommendation echoed as a warning on oversize bodies. L3 is ordinary `read` of bundled files. Skill bodies are untrusted input and never granted authority.

Loaded-skill names are tracked and re-listed (names only) after compaction. The agent never authors skills in v0.1; as defense in depth, any write/edit whose real target resolves inside a `**/skills/**` directory requires confirmation in every mode including auto (agent-created skills are persistent injection vectors; in headless/child contexts where there is no TTY, and for any REPL not attached to a real terminal, this ask degrades to deny). This is a guardrail against the write/edit tools authoring skills, not a sandbox boundary: bash is unrestricted (the container is the wall). The gate has two halves so a symlink created earlier in the same tool batch cannot route a write past it: the loop asks at plan time and records the confirmed real target; write/edit re-resolve the real target at execution time and refuse it unless it was the confirmed one.

## MCP client (crates/noob/src/mcp)

Config `/config/mcp.json`, merged with `/work/.noob/mcp.json` (project wins per name):

```json
{ "servers": {
    "websearch": { "url": "http://localhost:8000", "timeout_s": 30 },
    "fs":        { "command": "some-mcp-server", "args": ["--flag"], "timeout_s": 30 }
} }
```

`url` -> Streamable HTTP; `command` -> stdio; both on one entry is a config error. Protocol 2025-11-25, tools only in v0.1.

Lazy to the bone: startup connects nothing; the prompt carries one line of server names. `mcp_connect {server}` performs initialize + tools/list and returns a compact catalog (tool name, first 150 chars of description, parameter sketch) as a tool result wrapped in untrusted-content delimiters; schemas never enter the head and the tools array never changes, so the cache prefix survives MCP use entirely. `mcp_call {server, tool, args}` validates `args` client-side against the cached schema before sending; a validation miss returns the expected schema snippet to the model instead of a wire error. Calling an unconnected server returns "connect first with mcp_connect".

Transports: stdio is a child process with newline-delimited JSON-RPC, one reader thread, `mpsc::recv_timeout` for per-call timeouts (default 30 s, per-server override), kill-on-timeout of the process group so a wedged server can never block the loop; a dead server is respawned transparently on the next call. Streamable HTTP is POST with `Accept: application/json, text/event-stream`, both response types handled, `MCP-Session-Id` captured and replayed with one re-initialize on 404, `MCP-Protocol-Version: 2025-11-25` on every request. All MCP HTTP goes through `noob_provider`'s http and sse modules, which is what keeps the cargo-metadata egress test true.

## Plan mode (agent + tools registry)

Entered via `noob --plan`, `noob exec --plan`, or `/plan` in the REPL. Implementation is structural: the tools array sent to the model contains only read, grep, glob, ls, and skill. No bash (a read-only bash cannot be cheaply verified), no MCP, no task, no write/edit. Absent schemas cannot be called, cost zero prompt tokens, and do not tempt small models; as defense in depth the dispatcher also refuses any mutating call while in plan mode. Entry appends a user-role message: `[plan mode] Explore read-only, then present a numbered implementation plan.` and is one accepted cache bust (the tools array changed).

The plan is plain assistant text; there is no plan tool and no end-of-turn heuristic. Approval is explicit: `/go` in the REPL restores the full tool set (the second accepted bust) and appends "Plan approved. Execute it."; the loop continues in the same session. `noob exec --plan -p "..."` prints the plan and exits 0; `noob exec --session <id> -p "go"` continues it, which gives telegram-bot-skill a review-then-approve flow from a phone.

## Multi-agent (crates/noob/src/task)

The binary spawns itself: parent runs `current_exe() child` with env `NOOB_DEPTH=n+1`. The task payload goes to the child's stdin as one JSON object `{"prompt", "tools": "read-only"|"all", "max_turns"}`. The child builds a fresh context (base prompt + AGENTS.md, no parent history, no skills index unless it uses `skill` itself), runs its own loop, resolves `/config/.env` lazily itself (key rotation applies to running children), streams progress to stderr (parent relays as dim `[task] ...` lines under `--verbose`, otherwise discards), and writes exactly one JSON line to stdout: `{"status": "ok"|"error", "result": "...", "turns": N, "usage": {...}}`. The fd separation makes single-message result return a mechanical guarantee. Only `result` enters the parent transcript, as the `task` call's tool result.

`task` defaults to `tools: "read-only"` (children are research agents unless the model explicitly asks for a mutating child). Caps, enforced by both parent and child: concurrency `NOOB_TASK_CONCURRENCY` (default 4; fan-out beyond it queues), per-child turn cap `NOOB_TASK_MAX_TURNS` (default 25; child exits with status error at the cap), per-child wall clock 300 s (parent kills the process group on expiry), recursion depth max 2 via `NOOB_DEPTH` (the child's `task` tool is simply not registered at max depth). Children have no TTY, so any ask-gated action (skills-dir writes) degrades to deny. Turn and wall-clock caps bound spend; they are loop budgets, never output-token caps, and no request ever carries a max_tokens-family field. No sockets, no shared memory, no scratch files: argv + stdin + stdout is the whole IPC surface, and tests spawn children exactly the way the parent does.

## UI and headless surface (crates/noob/src/ui)

Interactive REPL: at a terminal `noob` runs an opt-in zero-dependency termios line editor (a boxed green prompt with real editing: insert, backspace across a multibyte char, word and line kills, cursor moves, bracketed paste), restored to cooked on every exit by three hooks (RAII guard, panic hook, and the SIGINT handler before `_exit(130)`, since release is `panic = abort`); piped or headless it falls back to cooked `read_line`, byte-identical. It streams model text raw to stdout as it arrives (no markdown rendering; small local models produce cleaner plain text when nothing reformats them), renders reasoning deltas dim, and renders tool activity as single dim ANSI lines: `* read src/main.rs (312 lines)`, `* bash cargo test (4.1s, exit 0)`. `IsTerminal` disables ANSI when piped. Slash commands, the complete v0.1 set: `/plan`, `/go`, `/status` (usage, cache-hit tokens, endpoint), `/compact`, `/quit`. Approval prompts are one-line y/N questions. The termios editor landed in v0.2.1; arrow-key history follows in v0.2.2.

Headless: `noob exec -p "<prompt>" [--json] [--session <id>]`. Default prints the final assistant text to stdout, progress to stderr, nonzero exit on failure. `--json` emits one JSONL event per loop step (`{"t":"text","d":...}`, `{"t":"tool","name":...,"args":...}`, `{"t":"result","id":...,"err":...}`, `{"t":"done","usage":...}`) so wrappers stream progress; `--session` persists and resumes a JSONL session under `/config/sessions/`. This is the telegram-bot-skill surface: streaming multi-turn chats with zero daemon.

Debug surface: `noob debug prompt --json` prints the exact assembled system prompt and tools array the binary would send, so budget tests measure the shipped artifact rather than a reimplementation.

## Docker and sandbox (docker/)

Dockerfile, two stages:
1. `rust:alpine` + `musl-dev` (C compiler for ring), `cargo build --release --locked --target x86_64-unknown-linux-musl`, strip.
2. `alpine:3.22` + `apk add --no-cache bash git ca-certificates` (the bash tool needs a real shell; git needs system roots for https clones; noob itself uses compiled-in webpki-roots), binary at `/usr/local/bin/noob`, `ENV NOOB_SANDBOX=container`, `WORKDIR /work`, `ENTRYPOINT ["noob"]`. No config, no state, no keys in the image, ever. Image lands around 40 MB.

```yaml
services:
  noob:
    build: { context: ., dockerfile: docker/Dockerfile }
    network_mode: host          # reaches llama.cpp :8090 and websearch MCP :8000 on host loopback
    working_dir: /work
    user: "${UID:-1000}:${GID:-1000}"   # files written to /work are never root-owned on the host
    volumes:
      - ${WORKSPACE:-.}:/work
      - ${NOOB_CONFIG:-./config}:/config
    stdin_open: true
    tty: true
```

`docker compose run --rm noob` is the REPL; `docker compose run --rm noob exec -p "..."` is a one-shot. The committed `config/.env.example` plus endpoint autodetect means an empty config still lands in a working chat.

Sandbox levels, two states total, no permission-rule DSL in v0.1 (the container is the wall; permission matrices without OS isolation are a false wall): inside a container (detected via `/.dockerenv` or `NOOB_SANDBOX=container`) tools run unrestricted except the skills-dir write gate. Outside a container the binary starts in workspace mode: write/edit refuse paths resolving outside the cwd tree (paths canonicalized; symlink escapes rejected), bash prints a one-time "no sandbox, commands run on your host" warning; `--yolo` lifts it.

Egress invariant: the binary makes network calls only to the configured base URL, configured MCP endpoints, and the loopback autodetect probes; no telemetry, no update checks, no title generation. Mechanically enforced: a test parses `cargo metadata` and fails if any crate other than `noob-provider` depends on ureq, and a mock e2e asserts zero requests to any URL other than the configured base.

## Testing strategy

No CI, ever; `make test` (offline) and `NOOB_LIVE=1 make smoke` (live) are the whole story, stated in the root contract.md.

**Mock harness (noob-testkit)**: a hand-rolled HTTP/1.1 server on `std::net::TcpListener` (~300 lines; it serves only our own client) exposing `/v1/chat/completions` and `/v1/responses`. Tests enqueue scripted turns (canned SSE byte responses or request-assertion closures); the harness records every raw request body. Three assertions run automatically on EVERY request so every future e2e inherits the invariants for free: (1) prefix stability, each request's serialized message/input array is an exact JSON-byte prefix of the next (compaction and plan transitions declare expected breaks); (2) no `max_tokens`/`max_completion_tokens`/`max_output_tokens` key anywhere; (3) transcript validity, every tool_call id paired with exactly one result, in order.

**SSE fixtures**: `testdata/sse/*.sse` are byte transcripts with `%%CHUNK%%` sentinels marking TCP chunk boundaries, so replay reproduces pathological splits deterministically, including one fixture splitting a multibyte codepoint and one splitting mid-`data:` line. The llama.cpp ones (qwen tool call, parallel calls, Responses function-call session) are captured real transcripts; the OpenRouter one (keepalive comments + mid-stream in-band error) is synthesized to OpenRouter's documented stream shape, to be replaced with a real capture when an OpenRouter key is on hand (each fixture's provenance is stated in testdata/contract.md).

**e2e** (crates/noob/tests): spawn the compiled binary (`env!("CARGO_BIN_EXE_noob")`) with `NOOB_CONFIG_DIR` at a temp dir whose `.env` targets the mock, cwd in a temp workspace; assert stdout, JSONL session content, recorded request bodies, and file side effects. Both API shapes run the same scenario matrix. Named tests locked now: `hot_reload_env` (rewrite `.env` between turns, assert the Authorization header changes), `cache_prefix` (3-turn prefix property), `no_output_cap`, `parallel_calls` (concurrent read batch, results in emission order), `mutate_barrier` (edit+bash serialize), `plan_gate` (mutating schemas absent in plan mode, restored after /go), `child_fanout` (three task calls, concurrency cap observed via mock timestamps), `doom_loop`, `compaction` (inflated mock usage forces the trigger; asserts summary shape and skill re-listing), `exec_json` (JSONL event stream shape), `session_resume`, `skills_gate` (write into a skills dir requires confirmation), `egress` (cargo-metadata + zero foreign requests).

**Unit tests** where logic is intricate: SSE parser table tests (every case in the parser section, driven by fixtures re-split at every byte offset), chat assembler quirk tests (every quirk-matrix case), edit ladder per stage plus ambiguity and staleness rejection, truncation goldens (marker phrasing frozen), .env parser, frontmatter scanner, JSON-RPC framing, compat 400-strip.

**Token budget**: offline test runs `noob debug prompt --json` and tokenizes with tiktoken o200k (dev-dep) against the locked ceilings, plus the forbidden-phrase lint; the live suite closes the loop with llama-server `/tokenize` against the real qwen tokenizer.

**Live smoke** (opt-in, `NOOB_LIVE=1`) against qwen3.6-35b-a3b at `:8090`: (1) edit round-trip (read, edit, verify file); (2) parallel tool calls in one turn; (3) a 3-turn cache check asserting llama.cpp `timings.prompt_n` on turn 3 is approximately suffix-sized, proving prefix reuse end to end; (4) `/tokenize` budget checks; (5) skill load and use (P3+); (6) a real call through the websearch MCP at `:8000` (P4+); (7) one Responses-shape turn against the local vllm-qwen stack. Item 1 runs first in P1: it is the gate on PLAN's top risk (qwen tool-calling through llama.cpp jinja).

**Size budget**: `make size-check` fails on crate graph > 45, stripped binary > 8 MB.

## Phase mapping P0-P7 and cut lines

- **P0 scaffold**: workspace (noob, noob-provider, noob-testkit) + full contract.md set, Dockerfile, compose.yml, Makefile, `config/.env.example`, mock harness skeleton serving both endpoints with the three automatic assertions wired (they exist before any provider code), `exec` skeleton. Two risk gates retired here: ureq/rustls/ring feature wiring on musl, and the 1 s tick-read watchdog feasibility on ureq's body reader (fallback: custom transport). Done = an e2e runs the real binary against the mock.
- **P1 provider**: both adapters complete with the full quirk matrix, SSE parser + fixtures, dual timeouts + interrupt plumbing, retry/backoff, compat strip-retry, lazy `.env`. Named tests `hot_reload_env` and `no_output_cap` land here. Live smoke item 1 runs vs qwen (PLAN's top risk, first thing) and one Responses turn vs vLLM.
- **P2 core loop + tools**: agent loop, 7 tools, edit ladder + check-and-set, truncation goldens, scheduler (read-concurrent/mutate-barrier), doom-loop breakers, system prompt assembly + budget tests + `debug prompt`, compaction, session JSONL, cooked REPL, `exec --json --session`, endpoint autodetect. `cache_prefix`, `parallel_calls`, `mutate_barrier` e2e land here.
- **P3 skills**: 4 discovery paths, frontmatter scanner, skill tool, capped index, post-compaction re-listing, skills-dir write gate. Ecosystem-path tests plus live skill load.
- **P4 MCP**: both transports through noob-provider's http/sse, mcp.json merge, mcp_connect/mcp_call with client-side validation, per-call kill-on-timeout. Live test vs websearch MCP at `:8000`. (P5 has no dependency on P4 and can land first if P4 stalls on live testing.)
- **P5 plan mode**: structural gating, injected mode message, `/go` flow, `exec --plan`. `plan_gate` e2e. Small phase; the registry machinery exists from P2.
- **P6 multi-agent**: `child` subcommand, task tool, fan-out group scheduling, caps, ask-degrades-to-deny in children. `child_fanout` e2e including a cap-exhaustion case.
- **P7 hardening + release**: `noob doctor`, full live suite, integrations (websearch MCP recipe, telegram-bot-skill via `exec --json --session` with the exact compose invocation documented), README, repo description and topics per repo-visibility policy, size budgets, v0.1 tag: static binary + docker image.

**Cut from v0.1** (each with its upgrade path recorded here): CRC hashedit (check-and-set covers the safety property; hashedit slots behind the same tool name), Anthropic API adapter (the neutral Turn/Event types and per-shape adapters accommodate it), ratatui or any TUI, history (v0.2.2; the termios line editor itself landed in v0.2.1), markdown rendering, permission glob-rule DSL (the container is the wall), first-run wizard (autodetect + .env.example instead), agent-authored skills (deliberately never without an explicit user gate), MCP resources and prompts, `resend_reasoning` and any persisted compat registry (process-lifetime cache only), background bash, LSP anything, token-level child budgets (turn + wall-clock caps instead), network-isolated compose profile (documented as future work), secret scanner, aarch64 image (P7 stretch), Windows.

