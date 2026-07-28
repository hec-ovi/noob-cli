# bench

Measures whether a change to the agent made it better or worse. Not a
comparison against other tools: it compares noob to its own previous self, on a
fixed task set, against a fixed model.

## Running it

Needs a live endpoint. Defaults to `http://localhost:8080/v1`.

```bash
./bench/run.py                                   # every task, 3 runs each
./bench/run.py --repeats 1                       # quick pass
./bench/run.py --task misspelled-dir             # one task
./bench/run.py --out bench/results/today.json
./bench/run.py --compare bench/results/baseline.json
```

`--compare` prints the change per task next to each row. Set `NOOB_BIN` to
point at a specific binary, otherwise the release build is used if present and
the debug build otherwise.

Each run gets a throwaway config directory and workspace, so nothing touches
your real sessions and no task can see another task's files.

## What is recorded

| Column | Meaning |
|---|---|
| `pass` | How many runs satisfied the task's checks |
| `rounds` | Requests sent to the model. The cost of thrashing shows up here first |
| `prefill` | Prompt tokens the server actually computed, excluding anything served from its cache |
| `gen` | Tokens generated |
| `calls` | Tool calls made |
| `errs` | Tool calls that failed |
| `secs` | Wall clock |

Every column except `pass` is better when lower.

`prefill` deliberately excludes cached prompt tokens. Every request re-sends the
whole transcript, so summing raw prompt tokens would grow with the square of
the conversation and describe work nobody did.

## Why three runs

The local model is sampled, so a single run says nothing. Each metric is the
median across runs. A one-run pass is for checking the harness itself, not for
recording a baseline.

## Tasks

Each file in `tasks/` carries a `why` field stating what it is for. A task that
cannot explain what it measures should be deleted rather than kept for
coverage.

Checks are deliberately blunt: does the answer contain the expected string,
does a file contain or lack some text, does a command exit zero. A task whose
pass condition needs interpretation is a task whose result cannot be trusted.

## Using it

Checkpoints that touch the agent record a before and after. A change that
raises rounds or tokens without a stated reason does not ship. Baselines live
in `results/` and are committed, because a baseline nobody can reproduce is
just a number in a commit message.
