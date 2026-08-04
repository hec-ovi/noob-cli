# Demo script

One take that shows the four things a screenshot cannot: the plan ticking
itself off, messages queued while the agent works, a fleet of detached
sub-agents, and the activity strip. The raw take runs about 12 minutes and cuts
to 90 seconds.

Every screen line quoted here was played through the real binary at 0.11.6
against a local llama.cpp endpoint (qwen3.6-35b-a3b). The timings are what that
run measured, so you know where the dead air is before you sit down to record.

## Before you roll

1. **Install what you are demoing.** The installed `noob` is whatever was
   packaged last, not what is in the tree.

   ```bash
   noob --version                 # compare against Cargo.toml
   ./dev.sh build && ./dev.sh package
   sudo apt install ./dist/noob_amd64.deb
   ```

2. **Model server up.** `noob doctor` prints the endpoint and
   `commands folder-locked (landlock abi 8)`.

3. **Name the model.** With `NOOB_MODEL` unset the header says `default`.
   Once, then quit:

   ```
   /config set model qwen3.6-35b-a3b-heretic-q8
   ```

4. **Terminal at 120x40 or wider**, dark background, font large enough to read
   on a phone (16 to 18pt). The dock repaints in place, and a narrow window
   clips the pinned rows that carry the whole story.

5. **Record from a short path.** Activity lines print the file each tool
   touched, and a long path truncates mid-line.

   ```bash
   rm -rf ~/demo && mkdir ~/demo && cd ~/demo
   ```

6. **Paste the long prompts, do not type them.** The input line scrolls
   sideways past the window width, which reads badly on camera.

7. Notifications off, scrollback cleared, recorder rolling before you launch
   noob: the greeting is the opening shot.

## The take

Each scene builds on the last: an empty folder becomes a small project, the
project gets planned against, then a fleet writes its tests.

### 0. Cold start (10s)

Type `noob`.

```
noob 0.11.6 · http://localhost:8080/v1 · qwen3.6-35b-a3b-heretic-q8 · context 131.1k · session 19fccac0224-61d4-0-483b3cfc
type a task; /plan /clear-plan /go /status /context /sessions /agents /config /compact /skills /mcp /quit
```

Say: one static binary, about 4.3 MiB, no async runtime and no TUI framework,
talking to a model running on this machine.

Two-second beat: type `/a` and stop. The line completes itself and says what it
does: `/a  /agents  list or cancel background sub-agents`.

### 1. The plan, ticking (90s, no dead air)

Paste:

```
build a tiny task tracker in python: tasks.py with add/list/done commands, storage.py with json persistence, and a README. Run it once to prove it works.
```

At about four seconds the checklist pins itself above the prompt and stays
there for the rest of the turn, without being asked for:

```
* plan:  0/4 done · 0.0s
plan (0/4 done): · 0.0s
[ ] Create storage.py with JSON persistence
[ ] Create tasks.py with add/list/done commands
[ ] Create README.md
[ ] Run tests to prove it works
```

Measured: step one at 13.5s, step two at 16.3s, step three at 9.6s, the whole
plan at 42.8s, each step rewriting the block in place with its own elapsed
time.

```
plan (3/4 done): · 42.8s
[x] Create storage.py with JSON persistence · 13.5s
[x] Create tasks.py with add/list/done commands · 16.3s
[x] Create README.md with usage instructions · 9.6s
[ ] Run the tracker end-to-end to prove it works
```

Underneath it, every tool call lands as its own line, first the call, then what
it did:

```
* write  storage.py (1138 bytes) · 0.0s
* read   tasks.py (41 of 41 lines) · 0.0s
* edited tasks.py (1 replacement) · 0.0s
* bash   python3 tasks.py add "buy milk" (1.0s, exit 0)
commands are folder-locked to the workspace (landlock)
```

And the frame around it all:

```
── ▪▪▪▪▪▪      Working 41s ─────────────────────────── 6.5k prefilled · 1.5k generated ──
```

Point at: the comet sweeping, the elapsed clock, the token counters growing
during the turn rather than after it, and the folder-lock notice the first time
the agent runs a command. Every command it types can read the machine and write
only inside this folder, enforced by the kernel, not by a prompt.

`[ ]` waiting, `[x]` done, `[~]` running. Whether you see `[~]` is the model's
call: it can mark a step in progress, or go straight to done. Past six steps
the block collapses into `… +4 more steps · 3 done · 1 queued`.

A failed command keeps its own line and brings the last eight lines of the
error with it, which is worth catching if it happens:

```
* bash python3 tasks.py list (0.1s, exit 1) · AttributeError: 'str' object has no attribute 'read_bytes'
```

### 2. Queued messages (record inside scene 1)

While the plan is still running, type and press Enter:

```
also add a Makefile with a run target
```

Nothing is interrupted. It pins as a dim row under the plan, and the bottom
rule counts it:

```
› also add a Makefile with a run target [queued]
── 1 queued · Esc Esc to cancel ──────────────────────────────────────────
```

Queue a second one:

```
and a short README section listing what each file does
```

When the turn ends the first queued message dispatches by itself, its
`[queued]` row disappears, and the plain `› also add a Makefile with a run
target` record enters the transcript as though you had just typed it. The
second one goes in after that turn.

Point at: you are never blocked. Typing is always live, Enter always queues,
and nothing you typed is lost or half-applied.

Optional five-second beat: press Esc once and the bottom rule turns into
`press ESC again to cancel`. Any other key, or five seconds, disarms it. Two
taps actually stop the turn.

### 3. Plan mode: read-only until you approve (2 to 3 min)

Type `/plan`:

```
plan mode: read-only tools until /go (cache prefix reset)
```

Then paste:

```
read the files here, then plan a run.py entry point that runs any of them by name, plus a tests folder
```

Write, edit and bash are not merely refused, they are absent from the request,
so the model cannot try. It reads the files and answers with a numbered plan.
The frame carries `· plan` while it works, and the idle rule after it lands
reads `── plan ──── 3.8k prefilled · 1.0k generated ──`.

Measured: three `* read` calls, plan text streaming from about 25s, complete
near 100s. It ends on its own line: `That's the plan. Ready to implement when
you /go.`

Type `/go`:

```
plan approved: full tools restored (cache prefix reset)
```

The full tool set comes back and it executes the plan it just showed you.

Say: the same context, two tool sets. Plan mode is not a politeness layer, the
mutating tools are structurally absent until you approve.

### 4. The fleet (3 min)

Paste:

```
spawn three detached sub-agents in parallel: one writes tests for tasks.py, one writes tests for storage.py, one writes tests for run.py. Each with edge cases. Answer with one short line.
```

Measured: about 20 to 40 seconds of thinking, then one `* subagent` line and
the fleet row appears above the prompt:

```
[3] agents running (Tab to view)
```

**Press Tab with an empty prompt line.** Tab on a non-empty line completes a
slash command instead, and it does nothing before the first agent exists. The
row expands into the panel, `[~]` running, `[ ]` queued, `[x]` ready, each row
carrying its live progress tail. Press Tab again to collapse it.

Queue another message while they work, so the fleet panel, a `[queued]` row and
the running frame are all on screen at once. That frame is your thumbnail.

Each result comes back as one line into the conversation:

```
agent-1 ok · 10.4s · Done. Created tests/test_tasks.py with 9 cases…
agent-2 ok · 10.5s · Done. Created tests/test_storage.py with 6 cases…
```

Say: each sub-agent is this same binary re-executed as `noob child`, its own
process, its own context, killed with its parent. The parent never blocks on
them, results land at the next round boundary. Concurrency is four by default,
and depth is capped at two so a fleet cannot multiply behind your back.

Then the controls: `/agents` lists them (`no background sub-agents` once their
results have landed), `/agents cancel agent-2` stops one, `/agents cancel all`
stops the rest. Esc Esc stops the turn and the fleet together.

### 5. Numbers and exit (30s)

```
/context
/status
```

`/context` prints one line, the same report the model gets when it calls the
`context` tool on itself:

```
context: ~12.4k / 131.1k tokens (9%); automatic compaction starts near 98.3k (75%)
```

`/status` prints endpoint, model, context, last-turn usage with cached prompt
tokens (the prompt cache, visibly working), skills, MCP servers and the session
file.

Then Ctrl-D:

```
session 19fccac0224-61d4-0-483b3cfc saved · resume with: noob --resume 19fccac0224-61d4-0-483b3cfc
```

Closing shot: `noob --resume latest`. The conversation redisplays and you carry
on typing.

## If it goes sideways

- **No checklist in scene 1.** The model answered without calling the plan
  tool. Since 0.11.6 the shipped prompt says when to call it and this holds on
  a 35B local model, but if you have replaced `AGENTS.md` with your own text
  you took that instruction out; put it back, or say "use the plan tool to
  track your steps" in front of the task.
- **Tab does nothing.** The prompt line must be empty and at least one agent
  must exist. Before that, Tab completes slash commands.
- **The agent asks a question mid-turn.** The bottom rule becomes
  `Enter confirms · Ctrl-C cancels all`. Type `y`, press Enter.
- **A turn stalls past two minutes.** Esc Esc, then retype.
- **The greeting says `default`.** `NOOB_MODEL` is unset, see setup step 3.
- **Sub-agent rows look verbose.** When the websearch CLI is on PATH, children
  are handed a runtime preamble, and the panel shows the head of it before the
  task text. Nothing is wrong; it is the child's real prompt.

## Cutting notes

- Scene 1 is the spine. If only one scene survives the cut, it is the plan
  ticking with the activity strip under it.
- Dead air is where the model thinks: 15 to 40 seconds before the first tool
  call of a turn, and up to a minute while sub-agents run. Speed those 4x to
  8x and keep the comet visible so it reads as live rather than frozen.
- Keep in real time: a queued row appearing, a plan step flipping to `[x]`, the
  fleet panel opening. Those three moments are the product.
- The prose plan in scene 3 is a long markdown block. Scroll it once at reading
  speed, then cut to `/go`.
- Nothing on screen needs redaction: no keys, no paths outside `~/demo`. Check
  the greeting anyway, it carries your endpoint URL.

## The 90-second cut

1. Greeting, three seconds.
2. Prompt pasted, checklist appears, steps flip to `[x]` (speed the gaps).
3. A message typed mid-turn, the `[queued]` row, the counter on the rule.
4. `/plan`, one line of the numbered plan, `/go`, tools coming back.
5. The fleet panel open with three agents running and a queued row underneath.
6. Ctrl-D, the resume line, `noob --resume latest` bringing it all back.
