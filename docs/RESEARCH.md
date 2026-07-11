# Research: what the existing harnesses teach noob-cli

Distilled 2026-07-09 from a three-track investigation of the agentic CLI field (full findings with all sources live in the local research store; this file keeps only what shapes noob-cli's design).

## Current terminal cross-check

The 2026-07-11 interface audit re-checked official upstream source rather than relying on the older survey snapshot:

- [Zero at commit `1af5882`](https://github.com/Gitlawb/zero/tree/1af58828eb3c22567599c000736c913a290959d2) keeps elapsed in-flight status and queue state in its TUI model, and has dedicated streaming Markdown, fenced-code, and table rendering in [`assistant_markdown.go`](https://github.com/Gitlawb/zero/blob/1af58828eb3c22567599c000736c913a290959d2/internal/tui/assistant_markdown.go).
- [Codex at commit `5c19155`](https://github.com/openai/codex/tree/5c19155cbd93bfa099016e7487259f61669823ff) uses a live status row above the composer with elapsed time, interrupt hints, and width-aware details in [`status_indicator_widget.rs`](https://github.com/openai/codex/blob/5c19155cbd93bfa099016e7487259f61669823ff/codex-rs/tui/src/status_indicator_widget.rs).

Those patterns informed the persistent status/composer separation and width-aware Markdown work. noob keeps its own zero-dependency ANSI implementation and ordered semantic event channel.

## Websearch runtime cross-check

The 2026-07-11 packaging audit used the current [websearch-skill source at commit `dcbbdf7`](https://github.com/hec-ovi/websearch-skill/tree/dcbbdf786527345c32894c52767fd72a6ea44c92), its [installation documentation](https://github.com/hec-ovi/websearch-skill/blob/dcbbdf786527345c32894c52767fd72a6ea44c92/docs/INSTALL.md), and the published [PyPI 0.1.0 package](https://pypi.org/project/websearch-skill/).

One Python package exposes both the standalone `websearch` commands and the stdio `websearch mcp` server. That makes a second sidecar unnecessary for the default install: the runtime image installs the pinned package in a uv tool environment, the bundled skill can call the standalone CLI through Bash, and the seeded MCP configuration launches the same package lazily over stdio. Alpine 3.22 supplies [Python 3](https://pkgs.alpinelinux.org/package/v3.22/main/x86_64/python3) and [uv](https://pkgs.alpinelinux.org/package/v3.22/community/x86_64/uv) from its package repositories.

## The field at a glance

| Tool | Language | Scale | One key trait |
|---|---|---|---|
| Pi (earendil-works/pi) | TypeScript | ~69k stars | Sub-1k-token system prompt, 4 core tools, everything else is extensions |
| OpenCode (anomalyco/opencode) | TypeScript (+Zig TUI core) | v1.17.x, daily releases | Agents as markdown config, plan/build split, Chat Completions reference |
| Codex CLI (openai/codex) | Rust workspace | v0.144.x | Single multitool binary, OS sandboxes, Responses API only since Feb 2026 |
| Zerostack | Rust | ~1.4k stars, 26 MB binary | Dual edit engine incl. CRC-32 line-hash check-and-set |
| zot | Go | ~289 stars | Subprocess swarm, JSON-RPC extensions, both API shapes |
| Zap | Rust | young | Skill injection, lazy MCP, Docker container sandbox with --network none |
| Hermes Agent (NousResearch) | Python | ~212k stars | Self-improving skills (agent writes its own SKILL.md files) |
| Agent Zero | Python | ~18k stars | Docker-native runtime split, superior/subordinate agents with fresh contexts |
| Zero (Gitlawb/zero) | Go | ~1k stars | Stream-JSON headless protocol, incremental write-root grants, bubblewrap+seccomp helper binary |

Attribution notes: Pi's repo is earendil-works/pi. QQCode (Python, stale since 2026-01) is not a reference here. Agent Zero is by Jan Tomasek, unrelated to the OpenCode team. OpenCode's canonical repo is anomalyco/opencode (SST rebranded to Anomaly).

## What we take from each

**Pi**: the minimal-harness bar. System prompt plus tool definitions under 1,000 tokens; four core tools (read, write, edit, bash) with optional read-only extras off by default; exact-string-replace editing with a fuzzy fallback (trailing whitespace, smart quotes, Unicode dashes); no line numbers in edits. Session files as JSONL trees (id/parentId) enabling branching in one file. Compaction that keeps ~20k recent tokens and LLM-summarizes the rest. A `compat` field that auto-detects OpenAI-compatible quirks per base URL.

**Codex CLI**: the Rust single-binary bar. One zero-dependency multitool binary with subcommands (exec, mcp-server, resume). Append-only prompt discipline: every request is an exact prefix-extension of the last one, which yields 80-90% prompt-cache hits and a near-linear (not quadratic) loop. Sandbox modes (read-only, workspace-write, full) kept orthogonal to approval policy (untrusted, on-request, never), with denial detection and escalation. Skills with progressive disclosure capped at ~2% of context. AGENTS.md with an explicit size cap (32 KiB).

**OpenCode**: agents as pure config. An agent is a markdown file with YAML frontmatter: mode (primary/subagent), model, permissions, tools, prompt body. Plan/build as two primary agents where plan simply sets edit/bash to deny/ask. A `task` tool taking subagent_type with fresh child context. Permissions as allow/ask/deny with per-command glob rules. Skills discovery that also reads `.claude/skills/` and `.agents/skills/` for ecosystem compatibility. Parallel tool calls encouraged by the prompt, not a scheduler.

**Zerostack**: the deterministic edit engine. Two selectable edit systems: aider-style SEARCH/REPLACE with fuzzy fallback, and `hashedit`, where reads annotate each line with an 8-char CRC-32 tag and edits reference those hashes under check-and-set, so stale edits fail loudly instead of corrupting files. Also: doom-loop detection (block identical tool calls repeated 3+ times), five permission modes, api_style switch (Responses for api.openai.com, Chat Completions for custom base URLs). Proof that ~16-17k LoC and ~16 MB RAM is achievable.

**zot**: single-binary multi-agent. `/swarm` spawns subagents as separate subprocesses of the same binary, each with its own loop and persistent session file, surviving restart. Extensions in any language over subprocess JSON-RPC. Layered system prompt (replace base with SYSTEM.md, append AGENTS.md). Reasoning-level flag mapping to per-provider thinking budgets.

**Zap**: context frugality. Skill injection so a greeting costs ~31 tokens while a full task assembles ~1.8k base plus task-relevant skills. Lazy MCP: server schemas stay out of context entirely until the model actually connects to a server mid-turn. The only surveyed tool with a Docker-native sandbox mode (container wrap with --network none). Casual-turn detection. A pre-transmission secret scanner.

**Hermes Agent**: the self-improvement loop, and its danger. Skills auto-created after complex tasks, error recoveries, and user corrections via a `skill_manage` tool plus a background review pass; a `/learn` command distills skills from directories, URLs, or past conversations. Its security audit (4 critical, 9 high, default allow-all, agent-created skills as persistent injection vectors) is the strongest argument that agent-authored skills must be gated and default-deny.

**Zero (Gitlawb/zero)**: the ops surface. Added late to the survey (same org as OpenClaude, part of Gitlawb's agent stack; Go, created 2026-05, very active). A documented stream-JSON stdin/stdout protocol for headless runs (`zero exec --input-format stream-json --output-format stream-json`), which is exactly the integration surface a Telegram bridge or another agent needs. Incremental write-root grants (`--add-dir`) instead of whole-filesystem access. Instruction files capped at 8 KiB each / 32 KiB total, injected general-to-specific from git root down to cwd. A `doctor` command for setup/key/connectivity checks and provider autodetection for Ollama/LM Studio. The delegation section only enters the system prompt when subagents are actually configured. Notably it speaks the Responses API only against the ChatGPT codex backend, not generically, so full dual-API support stays a noob-cli differentiator.

**Agent Zero**: the framework/execution runtime split, so agent-installed packages cannot destabilize the harness. Volume-mount only user data, never application code. Prompts as user-editable markdown fragments assembled at runtime, per-tool prompt files included. `call_subordinate` spawns a child with a fresh context cloned from parent config plus a profile; only the child's return value reaches the parent. MCP client calls run in disposable workers with timeouts so a wedged server cannot block the loop; resource payloads capped. A skill-reattachment token budget re-injects loaded skills after compaction.

## Standards to target

- **SKILL.md / Agent Skills** (agentskills.io, Anthropic open standard since 2025-12-18): folder with SKILL.md, YAML frontmatter, required `name` (max 64 chars, lowercase+hyphens) and `description` (max 1024 chars). Progressive disclosure in three levels: name+description only (~100 tokens/skill) at startup, full body on match (recommended under ~5k tokens), bundled files only when referenced. Adopted by Codex, OpenCode, Pi, zot, Hermes, Agent Zero, Cursor, Gemini CLI and more; this is the portable format.
- **MCP 2025-11-25**: implement stdio and Streamable HTTP transports only (standalone HTTP+SSE is deprecated; SSE survives only inside Streamable HTTP, and a client MUST accept both application/json and text/event-stream responses to POST). Tools first; surface resources and prompts. Handle MCP-Session-Id (404 re-init), MCP-Protocol-Version headers. Treat tool descriptions as untrusted input.
- **Both API shapes**: Codex removed Chat Completions (Feb 2026, Responses-only); OpenCode is Chat-Completions-first. Pi, zot, and Zerostack speak both, with the recurring pattern of an api_style/compat switch keyed on base URL (Responses for api.openai.com, Chat Completions default for custom endpoints). Supporting both is table stakes among the lean tools and a differentiator against Codex for local/OSS endpoints.

## Design bets these findings support

1. Tiny prompt, few tools: Pi proves under 1k tokens works; Zap proves dynamic assembly beats a fixed prompt for small models.
2. String-replace editing as primary, with fuzzy fallback; Zerostack's CRC hashedit as the deterministic upgrade path. Avoid unified diffs and line numbers (V4A only works because OpenAI trained for it).
3. Append-only prompt layout for cache hits (Codex's 80-90%).
4. Multi-agent via self-spawn: the single binary spawns itself as subprocess children with fresh, scoped contexts (zot swarm + Agent Zero subordinate model + Claude Code Task semantics: child sees only its task message, returns one message, capped in concurrency and budget).
5. Skills per the open standard, discovered also from `.claude/skills/` and `.agents/skills/`; loaded via a skill tool; never auto-written by the agent without explicit user approval (Hermes lesson).
6. Lazy MCP (Zap): schema loading deferred until a server is actually used; stdio + Streamable HTTP only; disposable workers with timeouts (Agent Zero).
7. Docker-native by design: the sandbox is the container (Agent Zero split, Zap container mode); mount the workspace at /work and config separately; never store state in the image.
8. Permissions: allow/ask/deny with glob rules (OpenCode), orthogonal to sandbox level (Codex), plus doom-loop detection (Zerostack).
9. Config as files read lazily (hot reload for free), agents/modes as markdown with frontmatter (OpenCode), prompts as editable files (Agent Zero, zot).
10. Headless-first integration surface: a documented stream-JSON stdin/stdout protocol and meaningful exit codes (Zero), so bridges like telegram-bot-skill drive the agent without scraping a TUI. Onboarding ergonomics: a doctor-style connectivity check and local-endpoint autodetection (Zero).
11. Prompt sections that pay rent: subagent/delegation instructions enter the system prompt only when subagents are configured (Zero); instruction files capped per file and in total (Zero 8/32 KiB, Codex 32 KiB).

## What we deliberately avoid

- Fixed multi-thousand-token system prompts (the ~4k-token baseline Zap measures against).
- Eager MCP schema dumps (13-18k tokens per server, Pi's measured numbers).
- No OS-level isolation with permissions as the only wall (OpenCode's gap).
- Silent network calls of any kind: OpenCode's Grok-titles telemetry incident is the cautionary tale. noob-cli talks only to the configured endpoints.
- Agent-authored skills without a gate (Hermes audit).
- Responses-only or Chat-Completions-only lock-in (Codex vs OpenCode; we do both).
- LSP servers in-loop by default (OpenCode docs themselves warn of the overhead).
- 1GB-RAM TUI stacks; we target Zerostack-class footprint (tens of MB).
