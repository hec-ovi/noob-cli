# noob-cli

noob-cli is a compact Rust agent for OpenAI-compatible model endpoints. It runs in an isolated Docker container against the current project directory, with persistent configuration and sessions stored outside the image.

The release binary is under 4 MB with 40 runtime crates. There is no async runtime or TUI framework.

## Install

The host needs Linux, Bash, and Docker Engine. amd64 and arm64 are supported.

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

The installed command mounts the current directory at `/work`, mounts `${XDG_CONFIG_HOME:-$HOME/.config}/noob` at `/config`, uses the caller's UID and GID, and removes the container when the command exits.

Resume a saved session:

```bash
noob --resume <session>
```

`--resume` is the canonical recovery flag; `--restore` and `--session` are aliases. On an interactive resume noob redisplays the prior conversation, and resuming an unknown id prints `no saved session <id>; starting fresh`. The exit line prints the session ID and the `noob --resume <id>` command that reopens it.

Installer options:

```text
./install.sh [--prefix <dir>] [--force]
```

`NOOB_INSTALL_PREFIX`, `NOOB_CONFIG_HOME`, `NOOB_WORKSPACE`, and `NOOB_IMAGE` override the install prefix, persisted config directory, mounted workspace, and runtime image.

## Run from the checkout

For development, or without installing the host command:

```bash
./dev.sh
WORKSPACE=/absolute/path/to/project ./dev.sh
./dev.sh exec "inspect the project and run its tests"
```

With no configured base URL, noob probes supported localhost ports. To pin an endpoint, copy and edit the example:

```bash
cp config/.env.example config/.env
```

## Commands

```text
noob [--model <name>] [--base-url <url>] [--resume <id>] [--plan] [--verbose] [--yolo]
noob exec -p "<prompt>" [--json] [--resume <id>] [--plan] [--verbose] [--model <name>] [--base-url <url>] [--yolo]
noob doctor
noob --version
```

Interactive commands:

| Command | Action |
|---|---|
| `/plan` | Enter read-only plan mode |
| `/go` | Approve the plan and restore the full tool set |
| `/status` | Show endpoint, usage, session, skills, and MCP state |
| `/compact` | Compact the current session |
| `/skills` | List skills |
| `/skills add <path-or-git-url>` | Install and reload one skill |
| `/skills remove <name>` | Remove a workspace-installed skill |
| `/skills reload` | Run discovery again |
| `/quit`, `exit`, or `quit` | Leave the REPL |

During a turn, typing edits the next draft. Enter queues one message. Escape twice within five seconds cancels, while Ctrl-C cancels immediately. A second Ctrl-C while cancellation is in progress restores the terminal and exits with status 130. Cancellation returns queued messages to the editor.

## Features

- Eight core tools: `read`, `write`, `edit`, `bash`, `grep`, `glob`, `ls`, and `todo` (a live `[x]`/`[~]`/`[ ]` checklist the model updates as it works, with no approval step).
- Conditional SKILL.md, MCP, and self-spawned child-agent tools.
- Parallel read-only calls with sequential mutation barriers and actual lifecycle timing.
- A live agents panel for `subagent` fan-out: one checklist of the parallel sub-agents with running or done status, a one-line result each, and the concurrency cap (`NOOB_TASK_CONCURRENCY`).
- Read-before-write stamps, atomic writes, deterministic edit fallbacks, and ambiguity rejection.
- JSONL sessions, `--resume` with on-screen replay of the prior conversation, context compaction, cache-prefix checks, and dangling-call repair.
- Read-only plan mode through `/plan`, followed by `/go`.
- Lazy MCP over stdio and Streamable HTTP. Server schemas enter context only after connection.
- Runtime skill discovery and atomic `/skills add`, `remove`, and `reload`.
- A default terminal dock with elapsed status, active tools, editable typeahead, queueing, confirmations, cancellation, Tab completion for slash commands, live in-place plan and agents panels, and reflow on terminal resize.
- Interactive Markdown for headings, emphasis, lists, fenced code, JSON, and width-aware tables.
- Matrix, ocean, amber, and violet display themes.

## Python and web search

The runtime image contains Python 3, uv, and `websearch-skill==0.1.0` in an isolated uv tool environment. The `websearch` command supports both standalone and MCP use:

```bash
websearch web-search "query"
websearch web-fetch "https://example.com/page"
websearch arxiv "paper topic"
websearch github "repository topic" --language Rust
websearch mcp
```

The installer enables the stdio MCP server without starting a sidecar. The bundled skill provides the standalone Bash fallback. When running from the checkout, enable the same MCP configuration with:

```bash
cp config/mcp.websearch.example.json config/mcp.json
```

An existing Streamable HTTP sidecar can instead be configured by setting its URL in `mcp.json`.

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

If startup autodetection selects an endpoint, that selection is fixed for the process. Restart noob to switch from an autodetected endpoint to a newly added `.env` URL. The launcher forwards a fixed set of `NOOB_*` and proxy variables plus any names listed in `NOOB_ENV`, and never forwards `NOOB_API_KEY`; put secrets in the mounted config `.env` and protect that directory with normal file permissions. `/skills reload` and a new process reload skills and MCP configuration respectively.

Display variables can be set in the shell or the checkout's root `.env` for Compose:

| Key | Default | Meaning |
|---|---|---|
| `NOOB_DOCK` | `1` | Set `0` for the classic prompt editor |
| `NOOB_RAW` | `1` | Set `0` for cooked input |
| `NOOB_THEME` | `matrix` | `matrix`, `ocean`, `amber`, or `violet` |
| `COLORTERM` | `truecolor` in Docker | Terminal color capability |
| `NO_COLOR` | unset | Disable color while keeping structure and status |

## Output surfaces

- Interactive REPL: terminal dock, Markdown, typeahead, queueing, and confirmations.
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

Beyond the offline suite, the interactive stack was driven against a local qwen model. The core tools, `todo` plans that are laid out and executed in the same turn, `subagent` fan-out, `/skills add`, use, and `remove`, `--resume` with on-screen replay, plan mode, cancellation, and resize reflow all behave as described.

The external [research-skill](https://github.com/hec-ovi/research-skill) was installed with `/skills add <git-url>` and exercised with a plain research question and no extra prompting. The model built a project-scoped `.research/` store on its own: an `INDEX.md` and per-topic `FINDINGS.md` files carrying a `sources` block, by driving the bundled `websearch` tool across several searches and fetches. So an installed skill and the web-search tooling compose without hand-holding.

See [ARCHITECTURE.md](ARCHITECTURE.md) for the runtime design and [PLAN.md](PLAN.md) for verified release status. The terminal design was cross-checked against current [Zero](https://github.com/Gitlawb/zero) and [Codex](https://github.com/openai/codex) source.

## License

[MIT](LICENSE). Copyright Hector Oviedo.
