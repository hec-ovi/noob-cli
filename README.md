# noob-cli

noob-cli is a compact Rust agent for OpenAI-compatible model endpoints. It runs in an isolated Docker container against the current project directory, with persistent configuration and sessions stored outside the image.

The release binary is under 4 MB with 40 runtime crates. There is no async runtime or TUI framework.

## Install

The host needs Linux, Bash, Git, and a running Docker Engine available to your user. amd64 and arm64 are supported. Development from the checkout also needs the Docker Compose plugin. The first build needs network access to pull the Rust and Alpine images, Alpine runtime packages, and the pinned websearch package.

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

During a turn the input stays live: typing edits the next message, and Enter steers immediately by stopping the current parent turn and dispatching the accepted message. The dock keeps plan and agent status visible while output scrolls above it.

## Features

- Nine core tools: `read`, `write`, `edit`, `bash`, `grep`, `glob`, `ls`, `context`, and `plan`.
- Conditional SKILL.md, MCP, and self-spawned child-agent tools.
- Parallel read-only calls with sequential mutation barriers and actual lifecycle timing.
- Detached sub-agents in the interactive dock, including `tools: "all"` coding and web-search jobs. The original call receives a running acknowledgment, then one final report enters context. Tab shows bounded live child activity and `/agents` manages cancellation.
- A cross-process workspace lease around each individual child `write`, `edit`, or `bash` call. Leased calls do not overlap, while inference, file inspection, and MCP calls remain concurrent. A child waits for the lease for a bounded time; a parent mutation reports the active conflict promptly instead of blocking the conversation.
- Read-before-write stamps, atomic writes, deterministic edit fallbacks, and ambiguity rejection.
- JSONL sessions, newest-first discovery, `--resume latest`, on-screen replay, context compaction, cache-prefix checks, and repair of dangling calls or interrupted background jobs.
- Read-only plan mode through `/plan`, followed by `/go`.
- Lazy MCP over stdio and Streamable HTTP. Server schemas enter context only after connection, and `/mcp add` installs a server mid-session.
- Runtime skill discovery and atomic `/skills add`, `remove`, and `reload`.
- A default terminal dock with elapsed status, active tools, editable steering, confirmations, cancellation, Tab completion for slash commands, live in-place plan and agents panels, and reflow on terminal resize.
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

The **skill** is a `SKILL.md` in the config that tells the model when to search and which subcommand to reach for. The model runs `websearch` through `bash`, or through the MCP server when one is configured. The installer seeds both and enables the stdio server without a sidecar; the bundled skill is the standalone Bash fallback. From the checkout, turn on the same MCP config with:

```bash
cp config/mcp.websearch.example.json config/mcp.json
```

Point at an existing Streamable HTTP sidecar instead by setting its URL in `mcp.json`.

The pair composes with no hand-holding. In a live run, asked a plain research question, the model reached for `websearch web-search` and `websearch web-fetch` across several queries on its own and folded the results into sourced findings.

## 🧩 Skills: instructions the model runs

A skill is a `SKILL.md` the model activates and then carries out with the ordinary tools, so it adds a capability without adding code. Install one from a local path, a git URL, or an `owner/repo` GitHub shorthand with `/skills add` (`/skills add hec-ovi/research-skill` just works), list with `/skills`, and drop a workspace one with `/skills remove`.

The external [research-skill](https://github.com/hec-ovi/research-skill) shows the shape. Once installed, a plain research question drove the model to `write` a project-scoped `.research/` store (an `INDEX.md` and per-topic `FINDINGS.md`, each with a `sources` block), search through `websearch`, and `read` the store back on later lookups. It runs `read`, `write`, `bash`, and `websearch`, the same tools any turn uses.

## 📟 The dock up close

Three small things the persistent dock does while a turn streams above it.

**📋 Plan.** The `plan` tool is the live checklist the model and user both see. The active `[~]` box spins while work runs, and each completed action shows its elapsed time. Long lists show at most six steps windowed on the active one, plus one `… +N more` row with done and queued counts. A finished or canceled plan collapses to one timed line; cancellation uses the theme's red error style. `/clear-plan` replaces historical plan arguments and results with small placeholders while keeping provider-valid call/result pairs.

**👥 Agents.** Sub-agents detach after an immediate job acknowledgment, so the prompt becomes usable while they work. Use `tools: "all"` for coding, Bash, MCP, or web-search work. The dock keeps a current `[N] agents running` line beside a plan. Press Tab on an empty draft for persistent job details and recent activity, or use `/agents`. Each finished result is removed from its child instance, injected once into the parent context, and delivered without waiting for unrelated slow jobs.

**⌨️ Steering.** Type while a parent turn is running. Enter records the message with a `[steering]` marker, interrupts that turn, and dispatches the message on the next loop. Escape or Ctrl-C cancellation keeps unsubmitted text in the editor instead of firing it.

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
| `NOOB_SKILL_PATHS` | none | Colon-separated skill directories, each resolved against the workspace and registered as one resolver skill (so a `cli/SKILL.md` dispatcher is discovered without copying it into a skills root) | `.env`: `/skills reload`; environment: process start |
| `NOOB_ENV` | none | Comma-separated allowlist of extra environment variable names the host launcher forwards into the container (for a workflow's own variables) | process start (launcher) |

If startup autodetection selects an endpoint, that selection is fixed for the process. Restart noob to switch from an autodetected endpoint to a newly added `.env` URL. The launcher forwards a fixed set of `NOOB_*` and proxy variables plus any names listed in `NOOB_ENV`, and never forwards `NOOB_API_KEY`; put secrets in the mounted config `.env` and protect that directory with normal file permissions. `/skills reload` reloads skills; `/mcp add` and `/mcp remove` reload the MCP server set in place.

The model server needs one request slot for the parent plus `NOOB_TASK_CONCURRENCY` child slots. With the defaults, configure at least five slots and give every slot the same 131,072-token window as `NOOB_CTX`. The [companion llama.cpp stack](https://github.com/hec-ovi/llama-vulkan-strix) documents and validates the required total KV-cache arithmetic.

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
| System prompt | 581 |
| Tool schemas, 13 tools | 849 |
| noob total | 1,430 |
| Chat template and message framing added by the server | 511 |
| First request total | 1,941 |

The 511 is the model's own chat template (qwen3 re-wraps the tools in its `<tools>` block with tool-calling instructions), so it changes with the model and its tokenizer; noob never sends those bytes. llama.cpp caches the prefix, so the overhead is prefilled once per slot, not on every turn. Reproduce with `noob debug prompt --json` and the server's `/tokenize` endpoint.

## Output surfaces

- Interactive REPL: terminal dock, Markdown, live steering, and confirmations.
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

`./dev.sh test` runs the full offline suite in the dev container. `./dev.sh size-check` enforces an 8 MB static-binary limit and a 45-crate runtime limit. `./dev.sh smoke` runs the opt-in live model and web-search checks serially.

To use non-default live endpoints:

```bash
NOOB_LIVE_BASE_URL=http://localhost:8080/v1 \
NOOB_LIVE_MCP_URL=http://localhost:18000/mcp \
./dev.sh smoke
```

### Verified end to end

Beyond the offline suite, the stack was driven against the local qwen endpoint. A fresh session created and completed its own visible plan, wrote and verified a file, resumed in a new process, called the context tool, and accurately explained the prior work. The backing llama.cpp server was also exercised with five simultaneous uncapped requests, matching one parent plus four detached children.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the runtime design and [PLAN.md](PLAN.md) for verified release status. The terminal design was cross-checked against source snapshots from [Zero](https://github.com/Gitlawb/zero/tree/1af58828eb3c22567599c000736c913a290959d2) and [Codex](https://github.com/openai/codex/tree/5c19155cbd93bfa099016e7487259f61669823ff).

## License

[MIT](LICENSE). Copyright Hector Oviedo.
