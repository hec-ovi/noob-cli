# noob-cli: project plan

An extremely lightweight, friendly agentic CLI. Rust, single static binary, built to live inside a Docker sandbox and work on a mounted host folder. Provider-agnostic (Chat Completions and Responses APIs), skills, MCP, plan mode, parallel tool calls, and dynamic multi-agent workflows. Ideas are learned from the best existing harnesses (Pi, OpenCode, Codex CLI, Hermes Agent, Agent Zero, and the small Rust/Go ones); no code is copied from any of them.

## Hard requirements

- Zero-friction UX, no learning curve: plug it in and it works. `docker compose up` (or one `docker run` line) must land in a working chat with no wizard and no mandatory config step. Auto-detect local OpenAI-compatible endpoints (llama.cpp, vLLM, Ollama, LM Studio) and sane-default everything else; a commented `.env.example` with obvious variable names covers the rest. Only familiar conventions (AGENTS.md, SKILL.md skills, the usual slash commands, .mcp.json-style config); no invented concepts a user must study first. A `doctor` command diagnoses setup and prints the one-line fix; every error message states its remedy.
- Rust, single static binary (musl target), instant startup, tiny memory footprint.
- Docker-first: development and runtime both happen in containers; nothing gets installed on the host. The agent is sandbox-native: it assumes it runs inside a container and treats that as its natural habitat.
- Works on a bind-mounted host folder (default `/work`), never inside the container filesystem or a named volume.
- Config lives in a bind-mounted directory containing an easy `.env`. Keys are read lazily on each API request, never cached at startup, so editing `.env` on the host applies on the next call with no container restart. Container env vars are not used for secrets (they freeze at `docker run`).
- Provider-agnostic: speaks both OpenAI Chat Completions and Responses APIs against any base URL (llama.cpp, vLLM, OpenAI, OpenRouter, etc.). Anthropic-style APIs can come later behind the same trait.
- Minimal instruction overhead: the system prompt has a measured token budget (target well under 1k tokens; the exact number gets locked in the design phase). Small local models are first-class citizens.
- File tools: read, write, edit (exact string replace), grep, glob, ls, bash. Multiple tool calls in a single inference, executed in parallel where independent.
- Skills: SKILL.md standard with progressive disclosure (only name + description in context until a skill is actually used).
- MCP client: stdio and streamable HTTP transports, tools first.
- Plan mode: read-only exploration, then an approved plan before writes.
- Dynamic multi-agent workflows: the agent can spawn scoped sub-agents with fresh contexts at runtime, fan them out in parallel, collect each result as a single message, and keep child context out of the parent. Budget and concurrency caps are enforced.
- Every folder in the repo ships a `contract.md`: what the folder does, its interface, and nothing about the rest of the system. Contracts are agnostic and isolated.
- Every behavioral change ships tests that run locally. No CI pipelines.

## Environment facts (verified 2026-07-09)

- Local model for live testing: `qwen3.6-35b-a3b` (35B MoE, Q4_K_XL, thinking disabled) served by llama.cpp Vulkan at `http://localhost:8090/v1`, 131072 ctx, key `noauth`. Verified reachable.
- A Responses API endpoint is also available locally when needed: the `vllm-qwen` stack serves `/v1/responses`.
- websearch MCP server (from `websearch-skill`) runs at `http://localhost:8000`.
- Container pattern to follow (from `pi-gemma`): `network_mode: host` so the container reaches host-loopback services, repo mounted at `/work` as the agent cwd, a config dir bind-mounted for global settings, `run --rm` for headless one-shots and interactive TTY for the REPL.
- Repo: `github.com/hec-ovi/noob-cli` (SSH remote), commits on `main`.
- Integration targets: `websearch-skill` (MCP) and `telegram-bot-skill` (expose noob-cli over Telegram).

## Process

1. **Research**: three parallel investigations covering (a) minimal harnesses: Pi, Zap, zot, Zerostack, QQCode, (b) OpenCode and Codex CLI, (c) Hermes Agent, Agent Zero, multi-agent patterns, the SKILL.md standard, and the current MCP spec. Findings land in the local research store and get distilled into `docs/RESEARCH.md` (committed).
2. **Design** (multi-agent workflow): four independent architecture proposals under different lenses (minimal footprint, provider abstraction, agentic loop quality, extensibility and UX), scored by a judge panel, synthesized into `ARCHITECTURE.md` plus the initial `contract.md` set. Design locks: async runtime choice, HTTP client, TUI vs plain REPL, crate layout, exact system prompt budget.
3. **Build** in the phases below. A phase is done when its tests pass locally. Commits are small and frequent (conventional style), pushed to `origin main` as they land.

## Build phases

- **P0 scaffold**: cargo workspace, per-crate `contract.md`, Dockerfile (musl builder stage, minimal runtime stage), `compose.yml` with the `/work` + config mounts, task runner, e2e test harness with an in-process mock OpenAI server.
- **P1 provider layer**: Chat Completions adapter with SSE streaming and tool-call parsing, Responses API adapter behind the same trait, parallel tool-call support, retry/backoff, lazy `.env` resolution per request.
- **P2 core loop + tools**: the agent loop, the file/shell tool set, tool result truncation, the minimal system prompt (token-counted in tests), context compaction.
- **P3 skills**: SKILL.md discovery and progressive disclosure, compatible with the existing skills ecosystem.
- **P4 MCP client**: stdio + streamable HTTP, config file, live test against the local websearch MCP.
- **P5 plan mode**: read-only tool gating and the plan approval flow.
- **P6 multi-agent**: dynamic sub-agent spawning, parallel fan-out, result collection, budget and concurrency caps.
- **P7 hardening + release**: live e2e suite against local qwen, integrations (websearch MCP, telegram-bot-skill), README, repo description and topics, v0.1 release (static binary + docker image).

## Testing

- End-to-end through the real binary: tests spawn the compiled CLI against an in-process mock OpenAI server (both API shapes), asserting on transcripts and file side effects.
- A live smoke suite (opt-in flag) runs against the local qwen endpoint at `:8090`.
- Unit tests where logic is intricate (streaming parsers, patch application, context accounting).
- An all-terrain live gauntlet, grown with each phase and complete at P7 (the v0.1 exit bar): hard multi-step prompts; several tool calls in a single inference; create, read, and edit files through the tools; repeated session close/resume/close/restore cycles with recall checks (ask about content from before the resume and expect the right answer); a real websearch call through the MCP client; interrupts at the nastiest points (mid-stream generation, mid tool execution such as during a file write) followed by a clean next turn; and a chaos pass sending SIGINT at random moments. Pass criteria: nothing wedges, transcripts stay API-valid, sessions always reopen, no partial file is ever left behind. The whole gauntlet runs against the small local model (qwen at `:8090`): if the harness only works with frontier models it has failed its design goal. And it is driven agent-to-agent: the gauntlet scripts speak to noob exclusively through the headless surface (`exec --json --session`, JSONL events, exit codes), the same way another CLI agent would drive it, so being operable by another agent is proven by construction, not claimed.

## Risks and open questions

- Tool-calling quality of qwen 3.6 MoE through llama-server jinja templates: validate in P1, first thing.
- llama.cpp does not serve the Responses API; the Responses adapter is validated against the mock server and the local vLLM stack.
- TUI vs plain REPL: lightweight-first bias, decided in the design phase.
- Windows is out of scope; Linux (and macOS via Docker) only.
