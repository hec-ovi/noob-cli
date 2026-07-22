# noob-cli

noob-cli is a compact Rust agent for OpenAI-compatible model endpoints. It runs in an isolated Docker container against the current project directory, with persistent configuration and sessions stored outside the image.

The static release binary is 4,330,368 bytes (4.13 MiB) with 40 runtime crates. There is no async runtime or TUI framework.

## Showcase

Recorded against a live qwen3.6-35b-a3b endpoint. Idle waits are sped up; the interactions themselves play close to real time.

The `context` tool reports token use on demand:

![noob answering with its own context use through the context tool](docs/media/showcase-context.gif)

Install a skill straight from a GitHub repo, hand it a research task, and keep talking while the detached sub-agent works. Tab opens its live view, here the sub-agent running web search in the background:

![Installing the research skill, then a sub-agent web-searching while the prompt stays live](docs/media/showcase-skills-agents.gif)

Ask for a three-step plan, then queue two follow-up messages while it builds. The plan finishes on its own and the queued messages dispatch in order, the first one right after the plan completes:

![A plan building three files while two typed messages wait queued, then dispatch in order](docs/media/showcase-plan-queue.gif)

## Install

You need Docker and Git. Everything else, including the Rust toolchain, lives inside the container, so the first build pulls the base images and packages and needs network access. Tested on Linux (amd64 and arm64); running from the checkout with `./dev.sh` also wants the Docker Compose plugin.

```bash
git clone https://github.com/hec-ovi/noob-cli.git
cd noob-cli
./install.sh
```

The installer builds `noob:local`, installs `~/.local/bin/noob`, and seeds the web-search skill plus its lazy stdio MCP configuration under `~/.config/noob`. It refuses to replace an unrelated `noob` command unless `--force` is passed.

Add `~/.local/bin` to `PATH` if your shell does not already include it, then run:

```bash
cd /path/to/project
noob
```

The installed command mounts the directory where you run it at `/work`. For disposable work, keep it separate from a source checkout:

```bash
mkdir -p ~/noob-workspace
cd ~/noob-workspace
noob
```

It also mounts `${XDG_CONFIG_HOME:-$HOME/.config}/noob` at `/config`, uses the caller's UID and GID, and removes the container when the command exits.

Resume a saved session:

```bash
noob sessions
noob --resume latest
# or: noob --resume <session-id>
```

`noob sessions` lists saved sessions newest first. `--resume latest` selects the newest one without copying its ID. `--resume` is the canonical recovery flag; `--restore` and `--session` are aliases. On an interactive resume noob redisplays the prior conversation, and resuming an unknown id prints `no saved session <id>; starting fresh`. The exit line prints the session ID and the exact command that reopens it.

Installer options:

```text
./install.sh [--prefix <dir>] [--force]
```

`NOOB_INSTALL_PREFIX`, `NOOB_CONFIG_HOME`, `NOOB_WORKSPACE`, and `NOOB_IMAGE` override the install prefix, persisted config directory, mounted workspace, and runtime image.

## Run from the checkout

For development, or without installing the host command, the default agent mount is the ignored `workspace/` directory in this checkout:

```bash
./dev.sh
NOOB_WORKSPACE=/absolute/path/to/project ./dev.sh
NOOB_WORKSPACE="$PWD" ./dev.sh exec "inspect the project and run its tests"
```

`./dev.sh` creates the default `workspace/` directory before mounting it at `/work`, so generated projects do not land in the noob-cli source tree.

With no configured base URL, noob probes supported localhost ports. To pin an endpoint, copy and edit the example:

```bash
cp config/.env.example config/.env
```

## Commands

```text
noob [--model <name>] [--base-url <url>] [--resume <id>] [--plan] [--verbose] [--yolo]
noob exec -p "<prompt>" [--json] [--resume <id>] [--plan] [--verbose] [--model <name>] [--base-url <url>] [--yolo]
noob sessions
noob doctor
noob --version
```

Interactive commands:

| Command | Action |
|---|---|
| `/plan` | Enter read-only plan mode |
| `/clear-plan` | Redact prior plan payloads from the active context |
| `/go` | Approve the plan and restore the full tool set |
| `/status` | Show endpoint, usage, session, skills, and MCP state |
| `/context` | Show context use and the automatic-compaction threshold |
| `/sessions` | List saved sessions newest first |
| `/agents` | List background sub-agents |
| `/agents cancel <agent-N\|all>` | Cancel and reap detached work |
| `/config` | Show, set, or unset non-secret `.env` settings |
| `/compact` | Compact the current session |
| `/skills` | List skills |
| `/skills add <path\|git-url\|owner/repo>` | Install and reload one skill (`owner/repo` reads from GitHub, like `npx skills add`) |
| `/skills remove <name>` | Remove a workspace-installed skill |
| `/skills reload` | Run discovery again |
| `/mcp` | List configured MCP servers and their connection state |
| `/mcp add <name> <url\|command...>` | Install an MCP server on the fly (persisted to `.noob/mcp.json`) |
| `/mcp remove <name>` | Drop a project-installed MCP server |
| `/mcp connect <name>` | Connect now and print the server's tool catalog |
| `/quit`, `exit`, or `quit` | Leave the REPL |

During a turn the input stays live: typing edits the next message, and Enter queues it without touching the running turn. The queued message waits as a normal `› message` row with a green `[queued]` tag above the input and dispatches in order once the turn finishes, landing in the transcript as a plain `› message` line. Only double-Escape (or Ctrl-C) stops a turn. The dock keeps plan and agent status pinned inside the input frame, in-turn and at the idle prompt alike, while output scrolls above it.

## Features

- Nine core tools: `read`, `write`, `edit`, `bash`, `grep`, `glob`, `ls`, `context`, and `plan`.
- Conditional SKILL.md, MCP, and self-spawned child-agent tools.
- Parallel read-only calls with sequential mutation barriers and actual lifecycle timing.
- Detached sub-agents in the interactive dock. The original call receives a running acknowledgment, then one final report enters context exactly once. A model response that only spawns agents ends its turn right after the acknowledgments, and status polling is answered once per input before the turn is closed for it, so the prompt frees seconds after a spawn instead of sitting behind a waiting loop. The prompt remains usable for ordinary main-agent work while several children run, and a child completion never interrupts an active parent turn. Tab shows bounded live child activity; both the user (`/agents cancel`) and the model (`subagent {"cancel":"agent-N"}`) can cancel a job, and double-Escape stops the whole fleet while a queued message and Ctrl-C leave it running. An accepted cancellation or terminal child failure blocks same-turn replacement spawns until the next human instruction.
- Three child tool profiles: the default `tools: "read-only"` for local inspection, `tools: "web"` for local inspection plus one unambiguous web-search MCP server, and `tools: "all"` for the full registered tool set. Web children cannot run Bash, mutate files, change the plan, or delegate. Dock children are leaves in every profile.
- A cross-process workspace lease around each `write` or `edit` call. File-tool mutations do not overlap, while inference, Bash, file inspection, and MCP calls remain concurrent. A child waits for the lease for a bounded time; a parent file mutation reports the active conflict promptly instead of blocking the conversation. Shell commands that mutate files are outside this guarantee, so the agent contract reserves Bash for builds, tests, and exploration.
- Read-before-write stamps, atomic writes, deterministic edit fallbacks, and ambiguity rejection.
- JSONL sessions, newest-first discovery, `--resume latest`, on-screen replay, context compaction, cache-prefix checks, and repair of dangling calls or interrupted background jobs.
- Read-only plan mode through `/plan`, followed by `/go`.
- Lazy MCP over stdio and Streamable HTTP. Server schemas enter context only after connection, and `/mcp add` installs a server mid-session.
- Runtime skill discovery and atomic `/skills add`, `remove`, and `reload`.
- A default terminal dock with elapsed status, active tools, mid-turn message queueing, confirmations, cancellation, Tab completion for slash commands, persistent in-place plan and agents panels that stay animated between turns, and single-write batched repaints (no flicker while output streams).

Known issue (minor edge case): resizing the terminal window is still unstable. The dock resets the viewport and repaints, but repeated resizes leave stale idle frames and blank gaps in the scrollback history. Pending a proper fix; avoid resizing mid-demo.
- Interactive Markdown for headings, emphasis, lists, fenced code, JSON, and width-aware tables.
- Matrix, ocean, amber, and violet display themes.

## 🔎 Web search: a skill and a tool

Web search reaches the model as a **skill plus a tool**, not a built-in.

The **tool** is `websearch`, a small Python package ([`websearch-skill`](https://github.com/hec-ovi/websearch-skill), pinned and installed in its own uv tool environment inside the runtime image). It ships a CLI and a stdio MCP server:

```bash
websearch web-search "query"
websearch web-fetch "https://example.com/page"
websearch arxiv "paper topic"
websearch github "repository topic" --language Rust
websearch mcp
```

The tool takes an optional egress proxy, off by default: set `WEBSEARCH_PROXY` to a proxy URL (`socks5h://user:pass@host:1080`), to `nordvpn`, or to `off`. The `nordvpn` shorthand builds the SOCKS5 URL from the `NORDVPN_USER` and `NORDVPN_PASS` service credentials, with `NORDVPN_HOST` selecting a server. The launcher forwards these four variables into the container, so exporting them before running `noob` is all it takes.

The **skill** is a `SKILL.md` in the config that tells the model when to search and which subcommand to reach for. The model runs `websearch` through `bash`, or through the MCP server when one is configured. The installer seeds both and enables the stdio server without a sidecar; the bundled skill is the standalone Bash fallback. From the checkout, turn on the same MCP config with:

```bash
cp config/mcp.websearch.example.json config/mcp.json
```

Point at an existing Streamable HTTP sidecar instead by setting its URL in `mcp.json`.

The opt-in live test gives qwen a research prompt with the MCP server configured, then asserts that the JSON event stream contains `mcp_connect` and `mcp_call` operations targeting `websearch`.

## 🧩 Skills: instructions the model runs

A skill is a `SKILL.md` the model activates and then carries out with the ordinary tools, so it adds a capability without adding code. Install one from a local path, a git URL, or an `owner/repo` GitHub shorthand with `/skills add` (`/skills add hec-ovi/research-skill` just works), list with `/skills`, and drop a workspace one with `/skills remove`.

The external [research-skill](https://github.com/hec-ovi/research-skill) shows the shape. With one unambiguous `websearch` MCP server configured, noob recognizes that skill's investigation brief and enforces `tools: "web"`, even if a small model requested `"all"`. That child can inspect local files and gather web evidence, but cannot run Bash, write files, change the plan, or spawn another agent. It returns the complete synthesis; the main agent validates it and alone updates the project-scoped `.research/` store. A completed web report is accepted only after at least two distinct `mcp_call` operations reached the server and returned server-originated results. Without that MCP match, the parent can use `tools: "all"` with the standalone `websearch` command when the task needs it.

## 📟 The dock up close

Three small things the persistent dock does while a turn streams above it.

**📋 Plan.** The `plan` tool is the live checklist the model and user both see. The active `[~]` box spins while work runs, and each completed action shows its elapsed time. Long lists show at most six steps windowed on the active one, plus one `… +N more` row with done and queued counts. A finished plan collapses to one timed line and moves into the chat history at turn end instead of staying stuck to the input; canceling a turn leaves an unfinished plan pinned in its actual state. The unfinished checklist stays pinned above the input across turns and at the idle prompt, updating in place instead of re-printing into the transcript, and the active step keeps spinning between turns (a step delegated to a still-running sub-agent stays visibly alive while the parent waits at the prompt). `/clear-plan` unpins it and replaces historical plan arguments and results with small placeholders while keeping provider-valid call/result pairs.

**👥 Agents.** Sub-agents detach after an immediate job acknowledgment, so the prompt becomes usable while they work. Use `tools: "read-only"` for inspection, `tools: "web"` for nonmutating MCP research, and `tools: "all"` for coding or shell work. Background jobs and the foreground plan are independent state machines that may coexist; the dock renders separate regions, and agent lifecycle is never copied into plan steps. Press Tab on an empty draft for persistent job details and recent activity, or use `/agents`. Double-Escape, during a turn or at the idle prompt, cancels every running agent after a visible confirmation hint; a lone Ctrl-C stops only the parent turn; a typed message stops nothing, it just queues. Each terminal result is removed from its child instance and injected once into the parent context. A message already being composed wins the completion race and receives ready reports before its own text in the ordinary turn. A failed or canceled report, including one coalesced with a success, leaves the prompt idle instead of invoking parent inference. Cancellation and failure also reject autonomous replacement spawns until a new human turn begins.

**⌨️ Queueing.** Type while a parent turn is running. Enter queues the message and leaves the turn, its tools, the plan, and every sub-agent untouched; it waits as a normal `› message` row with a green `[queued]` tag above the input, then dispatches as the next turn once the current one finishes and shows up in the history as a plain `› message` line. Escape or Ctrl-C cancellation hands queued and unsubmitted text back to the editor instead of firing it.

**⎋ Cancel.** Escape twice within five seconds cancels a running turn; Ctrl-C cancels at once. A second Ctrl-C during cancellation restores the terminal and exits with status 130.

## Configuration

The mounted config directory contains `.env`, optional `AGENTS.md`, `mcp.json`, global `skills/`, and `sessions/`.

| Key | Default | Meaning | Reload |
|---|---|---|---|
| `NOOB_BASE_URL` | localhost autodetect | OpenAI-compatible `/v1` base URL | `.env`: each request; CLI, environment, or autodetect: process |
| `NOOB_API_KEY` | empty | API key from `.env` only | each request |
| `NOOB_MODEL` | `default` | Endpoint model name | `.env`: each request; CLI or environment: process |
| `NOOB_API_STYLE` | by host | `chat` or `responses` | `.env`: each request; environment: process |
| `NOOB_AUTODETECT` | enabled | Set `0` to disable loopback probing | process start |
| `NOOB_CTX` | `131072` | Context window used for accounting | process start |
| `NOOB_SANDBOX` | container detection | `container` or `workspace` | process start |
| `NOOB_TASK_CONCURRENCY` | `4` | Concurrent child limit | process start |
| `NOOB_TASK_MAX_TURNS` | `25` | Child inference-round limit | process start |
| `NOOB_TASK_WALL_CLOCK_S` | `300` | Child wall-clock limit | process start |
| `NOOB_TOOL_CAPS` | enabled | Set `0` (or `off`) to lift every tool-output truncation cap: read, bash, grep, glob/ls, skill, and MCP results flow through whole | process start |
| `NOOB_SKILL_PATHS` | none | Colon-separated skill directories, each resolved against the workspace and registered as one resolver skill (so a `cli/SKILL.md` dispatcher is discovered without copying it into a skills root) | `.env`: `/skills reload`; environment: process start |
| `NOOB_ENV` | none | Comma-separated allowlist of extra environment variable names the host launcher forwards into the container (for a workflow's own variables) | process start (launcher) |

If startup autodetection selects an endpoint, that selection is fixed for the process. Restart noob to switch from an autodetected endpoint to a newly added `.env` URL. The launcher forwards a fixed set of `NOOB_*` and proxy variables plus any names listed in `NOOB_ENV`, and never forwards `NOOB_API_KEY`; put secrets in the mounted config `.env` and protect that directory with normal file permissions. `/skills reload` reloads skills; `/mcp add` and `/mcp remove` reload the MCP server set in place.

The model server needs one request slot for the parent plus `NOOB_TASK_CONCURRENCY` child slots to keep all of them generating at once. With the defaults, configure at least five slots. For llama.cpp, `--parallel` controls the `total_slots` reported by `GET /props`; set `--ctx-size` and the KV-cache configuration so the reported `n_ctx` is at least `NOOB_CTX` while those slots are active. `noob doctor` performs that read-only capacity check and also reports disabled tool-calling capabilities. See the current [llama.cpp server documentation](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md) and the [companion stack](https://github.com/hec-ovi/llama-vulkan-strix) for the deployment arithmetic.

`/context` (and `/status`, and the model-callable `context` tool) shows the estimated use, configured total, and 75 percent automatic-compaction threshold. When compaction runs, the terminal states whether the configured threshold, an endpoint overflow, or a length finish triggered it, then reports whether old tool output was pruned or the older conversation was summarized. Provider failures include the failed stage or HTTP status and a concrete next check.

`/config list` shows the effective non-secret settings and their file. `/config set ctx 65536` and `/config unset ctx` update that file atomically. Endpoint, model, and API-style edits apply on the next request unless a CLI flag or exported variable overrides them. Context and child-agent budget edits need a restart. API keys are intentionally not accepted by `/config`; edit the mounted `.env` so a secret does not enter terminal history.

Display variables can be set in the shell or the checkout's root `.env` for Compose:

| Key | Default | Meaning |
|---|---|---|
| `NOOB_DOCK` | `1` | Set `0` for the classic prompt editor |
| `NOOB_RAW` | `1` | Set `0` for cooked input |
| `NOOB_THEME` | `matrix` | `matrix`, `ocean`, `amber`, or `violet` |
| `COLORTERM` | `truecolor` in Docker | Terminal color capability |
| `NO_COLOR` | unset | Disable color while keeping structure and status |

## Prompt budget

The fixed first-request overhead is small and locked. `noob debug prompt --json` prints the exact system prompt and tool schemas the binary sends, and a budget test keeps that artifact under 1,500 tokens (o200k tokenizer) with every tool, a skill, and an MCP server registered.

Measured on the stock install (websearch skill and MCP server, all 13 tools) against qwen3.6-35b-a3b on llama.cpp:

| Piece | Tokens |
|---|---|
| System prompt | 585 |
| Tool schemas, 13 tools | 902 |
| noob total | 1,487 |
| Chat template and message framing added by the server | 530 |
| First request total | 2,017 |

The server-side framing figure is the model's own chat template (qwen3 re-wraps the tools in its `<tools>` block with tool-calling instructions), so it changes with the model and its tokenizer; noob never sends those bytes. llama.cpp caches the prefix, so the overhead is prefilled once per slot, not on every turn. Reproduce with `noob debug prompt --json` and the server's `/tokenize` endpoint.

## Output surfaces

- Interactive REPL: terminal dock, Markdown, mid-turn queueing, and confirmations.
- `exec`: assistant text on stdout and progress on stderr.
- `exec --json`: one JSON object per event.
- `child`: one JSON result line on stdout and progress on stderr.

Formatting never changes requests, transcripts, sessions, or cache-prefix bytes.

## Development and verification

```bash
./dev.sh test
./dev.sh size-check
./dev.sh docker
./dev.sh smoke
```

`./dev.sh test` runs the full offline suite in the dev container. `./dev.sh size-check` enforces an 8 MiB static-binary limit and a 45-crate runtime limit. `./dev.sh smoke` runs the opt-in live model and web-search checks serially.

To use non-default live endpoints:

```bash
NOOB_LIVE_BASE_URL=http://localhost:8080/v1 \
NOOB_LIVE_MCP_URL=http://localhost:18000/mcp \
./dev.sh smoke
```

### Verified end to end

Beyond the offline suite, the stack was driven against the local qwen endpoint. A fresh session created and completed its own visible plan, wrote and verified a file, resumed in a new process, called the context tool, and accurately explained the prior work. The backing llama.cpp server was also exercised with five simultaneous uncapped requests, matching one parent plus four detached children.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the runtime design.

## License

[MIT](LICENSE). Copyright Hector Oviedo.
