<h1 align="center">noob-CLI</h1>

<p align="center">
  <strong>An agentic CLI that assumes you know nothing: one small static binary in a Docker sandbox, pointed at whatever local model you already run.</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/status-in%20development-orange" alt="Status" />
  <img src="https://img.shields.io/badge/Rust-1.96%20%2F%20edition%202024-DEA584?logo=rust&logoColor=white" alt="Rust" />
  <img src="https://img.shields.io/badge/binary-3.4%20MB%20static%20musl-blue" alt="Binary size" />
  <img src="https://img.shields.io/badge/image-41.2%20MB-2496ED?logo=docker&logoColor=white" alt="Image size" />
</p>

<p align="center">
  <img src="https://img.shields.io/badge/tests-372%20offline%20%2B%208%20live-brightgreen" alt="Tests" />
  <img src="https://img.shields.io/badge/async%20runtime-none-success" alt="No async runtime" />
  <img src="https://img.shields.io/badge/runtime%20crates-40%20of%2045%20budget-blueviolet" alt="Crate count" />
  <img src="https://img.shields.io/badge/APIs-Chat%20Completions%20%2B%20Responses-7B3FA0" alt="Both OpenAI wire shapes" />
  <img src="https://img.shields.io/badge/license-MIT-green" alt="License" />
</p>

The name is the design goal: you should be able to use this knowing nothing. `docker compose run` lands in a working chat. No wizard, no learning curve, no host install beyond Docker itself.

## What this is

A lightweight general-purpose agentic CLI in Rust. Not a coding tool that happens to chat: an agent that reads and writes files, runs commands, loads skills, and calls MCP servers to get whatever task done, on whatever folder you mount. The binary lives inside a Docker container (that container is the sandbox), works on your files through a bind mount at `/work`, and speaks both OpenAI wire shapes, Chat Completions and Responses, against any base URL: llama.cpp, vLLM, Ollama, LM Studio, OpenAI, OpenRouter. Small local models (qwen-class through llama.cpp) are the first-class target, so every design choice optimizes for a tiny prompt budget, byte-stable cache prefixes, and error messages that tell the model what to do next.

Why another one? Most lean harnesses pick one wire shape (Codex CLI is Responses-only, OpenCode is Chat-first); noob speaks both against any URL, and small local models are the primary target rather than a fallback. The design comes from studying Pi, OpenCode, Codex CLI, Hermes Agent, Agent Zero, Zerostack, zot, Zap, and Zero; no code was copied from any of them. The full survey is in [docs/RESEARCH.md](docs/RESEARCH.md).

## What works today

P0 through P6 are done plus most of P7's hardening: noob is a working agent with skills, MCP, plan mode, and sub-agents, in active development. Working right now:

- `docker compose run --rm noob` opens a chat; `... noob exec -p "your prompt"` is the one-shot. Live-tested against qwen3.6 through llama.cpp: it reads files, edits them, runs commands, and reports back. `noob doctor` checks the setup and prints a one-line fix for anything broken. Leave with `/quit`, or just type `exit`.
- Seven core tools: read, write, edit, bash, grep, glob, ls. Three more register only when they can work: `skill` (when skills exist), `mcp_connect`/`mcp_call` (when mcp.json has servers), `task` (sub-agents, below the recursion ceiling). Consecutive read-only calls from one turn run in parallel; any mutation is a strict barrier, so two edits can never race.
- SKILL.md skills per the [agentskills.io](https://agentskills.io) standard, discovered from `.noob/skills/`, `.claude/skills/`, `.agents/skills/` in your project and from `/config/skills/`. Only a capped one-line-per-skill index sits in the prompt (the resolver, in the gbrain thin-harness fat-skills sense); bodies load on demand as tool results, capped at 24 KiB, and loaded skills are re-listed by name after compaction, across session resumes too. The agent can never author skills: any write into a skills directory asks you first, at a real terminal only; piped and headless runs are denied.
- MCP client (protocol 2025-11-25, tools only), lazy to the bone: nothing connects at startup, one line in the prompt names the servers, `mcp_connect` fetches a catalog as a tool result, `mcp_call` validates args against the cached schema before anything touches the wire. stdio servers get per-call kill-on-timeout on both the read and write side and respawn transparently; Streamable HTTP keeps the session id, re-initializes once on 404, and has an absolute per-call deadline a keepalive trickle cannot dodge. Everything a server sends comes back capped and wrapped in untrusted-content delimiters.
- Plan mode: `/plan` (or `--plan`) shrinks the tool set to read-only exploration, structurally: mutating schemas are absent from the request, so they cannot tempt a small model, and the dispatcher refuses hallucinated mutations anyway. `/go` approves the plan and restores the full set; `exec --plan` prints the plan and exits, and resuming the session executes it, which is a review-then-approve flow for wrappers.
- Sub-agents: the binary spawns itself. `task` fans out scoped children (read-only by default) with fresh contexts; one JSON result line per child comes back, nothing else enters the parent transcript. Caps enforced on both sides: 4 concurrent, 25 turns, 300 s wall clock (the parent kills the whole process group), recursion depth 2.
- The edit tool is exact string replace with a deterministic fallback ladder (trailing whitespace, typographic characters, uniform indent shift, CRLF files) and hard ambiguity rejection. A failed edit returns the actual file region so the model can fix its next attempt; files changed on disk behind the model's back are refused (hash check-and-set).
- Append-only prompt discipline, byte-exact: turn 3 of a live session shows a 97% cached-prompt share on llama.cpp's own counters. The mock server fails any test where a request is not a byte-prefix extension of the previous one.
- Sessions persist as JSONL under `/config/sessions/`; `exec --session <id>` resumes across processes (a session killed mid-tool-run is healed on resume). `exec --json` emits one JSONL event per loop step for wrappers.
- Compaction built to survive heavy sessions (the design notes live in the repo's research store): at 75% of the window a ladder runs, cheapest first. Old fat tool results become one-line placeholders with no LLM call when that alone frees enough; otherwise the middle is summarized against a fixed section schema and the result is validated before splicing (an empty or non-shrinking summary is retried once, then pruned or hard-dropped, never trusted). Every spliced message carries a pinned block the harness builds from ground truth: the original task, files touched, loaded skills. Pins merge across cycles and process resumes instead of eroding through summary-of-summary drift.
- Doom-loop breakers for small models: repeated identical calls are intercepted, four consecutive tool errors inject a course correction, eight stop the run.
- Endpoint autodetect: with an empty config, noob probes llama.cpp/Ollama/LM Studio/vLLM on localhost and uses the first that answers.
- The system prompt plus all tool schemas fit in 1,500 tokens, enforced by tests against both tiktoken and the live qwen tokenizer.
- Both OpenAI wire shapes (Chat Completions + Responses) against any base URL; SSE parsing that survives every TCP split; retries with backoff; hot `.env` reload on every request; Ctrl-C responsive within a second at every point, including mid-tool-batch and mid-fan-out (pending calls are canceled, children are killed, the session stays valid).
- 372 offline tests via `./dev.sh test`, 8 live smokes via `./dev.sh smoke` (serialized: parallel live tests would evict each other's llama.cpp cache slots).

What remains before v0.1 is the live all-terrain gauntlet (PLAN.md, Testing) and release packaging; the roadmap below marks exactly what exists.

## Quickstart

```bash
git clone https://github.com/hec-ovi/noob-cli.git
cd noob-cli
cp config/.env.example config/.env   # edit NOOB_BASE_URL if your endpoint differs
./dev.sh exec "say hi"               # or: docker compose run --rm noob exec -p "say hi"
```

The first run builds the image (musl static build, all inside Docker). An empty config works too: noob probes the usual localhost ports (llama.cpp :8090/:8080, Ollama :11434, LM Studio :1234, vLLM :8000) and uses the first endpoint that answers.

## Config

One flat `.env` in the bind-mounted config dir. All keys optional, everything commented in [config/.env.example](config/.env.example). The file is re-read on every request: change the model or rotate a key mid-session and the next call uses it. Keys never enter the process environment, so shell commands and (later) child agents cannot read them.

| Key | Default | What it does |
|---|---|---|
| `NOOB_BASE_URL` | localhost autodetect | OpenAI-compatible `/v1` base URL |
| `NOOB_API_KEY` | empty | API key, if the endpoint wants one; local servers usually accept anything |
| `NOOB_MODEL` | `default` | Model name as the endpoint knows it |
| `NOOB_API_STYLE` | by host | `chat` or `responses`; `api.openai.com` defaults to responses, everything else to chat |
| `NOOB_CTX` | `131072` | Context window in tokens; compaction starts at 75% |
| `NOOB_SANDBOX` | by `/.dockerenv` | `container` (unrestricted tools) or `workspace` (writes stay inside the project) |
| `NOOB_TASK_CONCURRENCY` | `4` | Concurrent sub-agent cap (P6) |
| `NOOB_TASK_MAX_TURNS` | `25` | Per-sub-agent turn cap (P6) |

## How it fits together

```mermaid
flowchart LR
    subgraph host["Your machine (needs docker, nothing else)"]
        WS["project folder"]
        CFG["config/.env<br>re-read every request"]
        EP["llama.cpp :8090 · vLLM · Ollama · LM Studio<br>or any OpenAI-compatible base URL"]
    end
    subgraph box["Docker sandbox · alpine · 41.2 MB · non-root"]
        NOOB["noob<br>static binary, 3.4 MB"]
        PROV["noob-provider<br>HTTP · SSE · wire adapters<br>sole owner of ureq"]
        NOOB --> PROV
    end
    WS == "bind mount /work" ==> NOOB
    CFG -- "bind mount /config" --> NOOB
    PROV -- "Chat Completions · Responses API<br>SSE streamed, retried, watchdog-guarded" --> EP
```

Two shipped crates plus a dev-only one: `noob` (the binary: argv dispatch, agent loop, tools, UI) depends on `noob-provider` (transcript in, events out, the only crate allowed to touch the network); `noob-testkit` is the hand-rolled mock OpenAI server the e2e suite runs against, never a runtime dependency. The compose file uses `network_mode: host` so the container reaches model servers on host loopback, and runs as your UID so files written to `/work` are never root-owned.

## Design rules

The opinionated bits. Where a rule says "test-enforced" that is literal: the mock server checks it on every request.

| Rule | Why |
|---|---|
| No request ever carries a `max_tokens`-family key; there is no config knob for one (test-enforced) | A capped response truncates mid-answer and breaks structured output; length is shaped by instructions, never by a ceiling |
| Append-only prompt: every request is an exact prefix extension of the previous one (test-enforced) | llama.cpp prefix KV reuse and provider prompt caching make turn N+1 cost only the new suffix |
| System prompt budget locked at 1,500 tokens total fixed overhead (about 1.1% of a 131k context) | Small models lose the thread in long prompts; the budget is measured on the shipped artifact |
| Edit is exact string replace with a deterministic fallback ladder, no similarity-score fuzzing, ever | A fuzzy match can corrupt a file silently; a rejection comes back with the actual file region so the model can retry correctly |
| The container is the sandbox | No permission-rule DSL: isolation comes from Docker; outside a container the binary falls back to a restricted workspace mode |
| No telemetry: the binary talks only to the configured endpoints | No update checks, no phoning home; only `noob-provider` may touch the network stack, checked against the crate graph |
| State lives in the mounts, never in the image | The image contains zero config, keys, or sessions; `docker rmi` loses nothing of yours |
| No async runtime, no tokio, no clap, no serde derive; ureq pinned at 3.3.0 | 28 runtime crates against a hard budget of 45; one inference at a time needs threads, not a reactor |

## Roadmap

| Phase | Scope | Status |
|---|---|---|
| P0 scaffold | Workspace, contracts, Docker build, mock OpenAI server, `exec` skeleton, watchdog | done |
| P1 provider layer | SSE streaming, Responses adapter, tool-call parsing, retry/backoff | done |
| P2 core loop + tools | Interactive REPL, the 7 file/shell tools, agent loop, system prompt, compaction, sessions, endpoint autodetect | done |
| P3 skills | SKILL.md discovery with progressive disclosure ([agentskills.io](https://agentskills.io) standard) | done |
| P4 MCP client | stdio + Streamable HTTP transports, lazy connect | done |
| P5 plan mode | Read-only exploration, explicit `/go` approval | done |
| P6 multi-agent | Self-spawning sub-agents, parallel fan-out, concurrency and turn caps | done |
| P7 hardening + release | `doctor` and compaction hardening are in; live gauntlet, integrations, v0.1 (static binary + image) remain | in progress |

Everything on the v0.1 feature list is built: seven file/shell tools with parallel calls, SKILL.md skills, an MCP client, plan mode, sub-agents the binary spawns from itself, and a headless JSONL surface (`exec --json --session`) built to be driven by other CLI agents, not just humans. What stands between here and the tag is the gauntlet.

## Development

Everything runs inside Docker; nothing is installed on the host. `./dev.sh` is the task runner (plain bash; a thin Makefile delegates to it if you prefer `make test`). Every folder ships a `contract.md` stating its purpose, interface, and invariants, and nothing about the rest of the system. There is no CI: `./dev.sh test` is the whole story.

| Target | What it does |
|---|---|
| `./dev.sh test` | The offline suite: 372 unit + e2e tests against the in-process mock servers, run in a dev container |
| `./dev.sh build` | The static musl release binary |
| `./dev.sh docker` | The runtime image |
| `./dev.sh repl` / `./dev.sh exec "..."` | Compose with your uid:gid passed explicitly, so files under `/work` keep your ownership |
| `./dev.sh smoke` | Live suite against local endpoints (opt-in, `NOOB_LIVE=1`) |
| `./dev.sh size-check` | Fails if the binary exceeds 8 MB or the runtime crate graph exceeds 45 |

The v0.1 exit bar is an all-terrain live gauntlet (PLAN.md, Testing): hard multi-step prompts, several tool calls per inference, session close/resume cycles with recall checks, interrupts at the nastiest points, and a chaos pass of random SIGINTs. The whole thing runs against a small local model, and it is driven agent-to-agent through the headless surface, so being operable by another agent is proven by construction.

## License

[MIT](LICENSE). Built by Hector Oviedo ([hec-ovi](https://github.com/hec-ovi)).
