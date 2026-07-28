#!/usr/bin/env python3
"""Measure what a change to the agent actually costs.

Each task runs the real binary against a live endpoint in a throwaway
workspace, then records four numbers: model round trips, tokens the server
actually computed, failed tool calls, and whether the task passed.

Nothing here mocks anything. The point is to compare noob against its own
previous self, so the only thing that must stay fixed between runs is the
task set and the model.

    ./bench/run.py                      # every task, 3 runs each
    ./bench/run.py --repeats 1          # one pass, for a quick check
    ./bench/run.py --task misspelled-dir
    ./bench/run.py --compare bench/results/baseline.json
"""

import argparse
import json
import os
import pathlib
import shutil
import statistics
import subprocess
import sys
import tempfile
import time

ROOT = pathlib.Path(__file__).resolve().parent
REPO = ROOT.parent
DEFAULT_BASE_URL = "http://localhost:8080/v1"
DEFAULT_MODEL = "qwen3.6-35b-a3b-q8"
TASK_TIMEOUT_S = 420

# Lower is better for every metric except `passed`.
METRICS = [
    "rounds", "prefilled", "generated", "context_end",
    "tool_calls", "tool_errors", "seconds",
]


def binary() -> str:
    override = os.environ.get("NOOB_BIN")
    if override:
        return override
    for candidate in ("target/release/noob", "target/debug/noob"):
        path = REPO / candidate
        if path.exists():
            return str(path)
    sys.exit("no noob binary; run `cargo build` or set NOOB_BIN")


def load_tasks(only: str | None) -> list[dict]:
    tasks = []
    for path in sorted((ROOT / "tasks").glob("*.json")):
        task = json.loads(path.read_text())
        if only in (None, task["id"]):
            tasks.append(task)
    if not tasks:
        sys.exit(f"no task matched {only!r}")
    return tasks


def build_workspace(task: dict, work: pathlib.Path) -> None:
    for entry in task.get("setup", []):
        target = work / entry["path"]
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(entry["content"])
        if entry.get("executable"):
            target.chmod(0o755)


def parse_events(stdout: str) -> tuple[int, int, int, str]:
    """Tool calls, failed tool calls, final transcript size, assistant text.

    `context_end` is the prompt size of the last request, which is how large
    the conversation grew. It is the context-economy number: `prefilled` says
    what the server had to compute, this says how much the agent dragged along.
    """
    calls = errors = context_end = 0
    text = []
    for line in stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        kind = event.get("t")
        if kind == "tool":
            calls += 1
        elif kind == "result" and event.get("err"):
            errors += 1
        elif kind == "text":
            text.append(event.get("d", ""))
        elif kind == "done":
            context_end = (event.get("usage") or {}).get("prompt", 0)
    return calls, errors, context_end, "".join(text)


def parse_session(config: pathlib.Path) -> tuple[int, int, int]:
    """Round trips, prefilled tokens, generated tokens, from the session log.

    One `usage` record is written per request, and `prefilled` already excludes
    what the server served from cache, so summing it describes work that
    actually happened rather than transcript size.
    """
    sessions = sorted((config / "sessions").glob("*.jsonl"), key=lambda p: p.stat().st_mtime)
    if not sessions:
        return 0, 0, 0
    rounds = prefilled = generated = 0
    for line in sessions[-1].read_text().splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("t") == "usage":
            rounds += 1
            prefilled += record.get("prefilled", 0)
            generated += record.get("generated", 0)
    return rounds, prefilled, generated


def check(task: dict, work: pathlib.Path, answer: str) -> bool:
    for rule in task.get("checks", []):
        kind = rule["kind"]
        if kind == "answer_contains":
            if rule["text"].lower() not in answer.lower():
                return False
        elif kind == "file_contains":
            path = work / rule["path"]
            if not path.exists() or rule["text"] not in path.read_text():
                return False
        elif kind == "file_lacks":
            path = work / rule["path"]
            if not path.exists() or rule["text"] in path.read_text():
                return False
        elif kind == "command_ok":
            done = subprocess.run(
                rule["cmd"], shell=True, cwd=work, capture_output=True, timeout=120
            )
            if done.returncode != 0:
                return False
        else:
            sys.exit(f"unknown check kind {kind!r}")
    return True


def run_once(task: dict, bin_path: str, base_url: str, model: str) -> dict:
    tmp = pathlib.Path(tempfile.mkdtemp(prefix="noob-bench-"))
    try:
        config, work = tmp / "config", tmp / "work"
        config.mkdir()
        work.mkdir()
        (config / ".env").write_text(f"NOOB_BASE_URL={base_url}\nNOOB_MODEL={model}\n")
        build_workspace(task, work)

        env = dict(os.environ)
        env.update({
            "NOOB_CONFIG_DIR": str(config),
            "NOOB_WEBSEARCH": "off",
            "NO_COLOR": "1",
        })
        started = time.monotonic()
        try:
            # `exec` persists nothing unless a session is named, and the
            # session log is where per-request usage lives. The id is fixed
            # because the config directory is already throwaway and per-run.
            done = subprocess.run(
                [bin_path, "exec", "--resume", "bench", "-p", task["prompt"], "--json"],
                cwd=work, env=env, capture_output=True, text=True, timeout=TASK_TIMEOUT_S,
            )
            timed_out = False
            stdout = done.stdout
        except subprocess.TimeoutExpired as expired:
            timed_out = True
            stdout = (expired.stdout or b"").decode("utf-8", "replace")
        seconds = time.monotonic() - started

        calls, errors, context_end, answer = parse_events(stdout)
        rounds, prefilled, generated = parse_session(config)
        return {
            "rounds": rounds,
            "prefilled": prefilled,
            "generated": generated,
            "context_end": context_end,
            "tool_calls": calls,
            "tool_errors": errors,
            "seconds": round(seconds, 1),
            "passed": (not timed_out) and check(task, work, answer),
        }
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


def summarize(runs: list[dict]) -> dict:
    """Median per metric. The local model is noisy; one run proves nothing."""
    out = {m: round(statistics.median(r[m] for r in runs), 1) for m in METRICS}
    out["passed"] = sum(1 for r in runs if r["passed"])
    out["runs"] = len(runs)
    return out


def render(results: dict, baseline: dict | None) -> None:
    head = (
        f"{'task':<22}{'pass':>7}{'rounds':>8}{'prefill':>9}{'gen':>8}"
        f"{'ctxend':>8}{'calls':>7}{'errs':>6}{'secs':>7}"
    )
    print(head)
    print("-" * len(head))
    for task_id, row in results["tasks"].items():
        line = (
            f"{task_id:<22}{str(row['passed']) + '/' + str(row['runs']):>7}"
            f"{row['rounds']:>8}{row['prefilled']:>9}{row['generated']:>8}"
            f"{row['context_end']:>8}{row['tool_calls']:>7}"
            f"{row['tool_errors']:>6}{row['seconds']:>7}"
        )
        if baseline and task_id in baseline.get("tasks", {}):
            was = baseline["tasks"][task_id]
            deltas = [
                f"{m} {row[m] - was[m]:+g}"
                for m in ("rounds", "context_end", "tool_errors")
                if row[m] != was[m]
            ]
            if deltas:
                line += "   " + ", ".join(deltas)
        print(line)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--task")
    parser.add_argument("--base-url", default=os.environ.get("NOOB_BASE_URL", DEFAULT_BASE_URL))
    parser.add_argument("--model", default=os.environ.get("NOOB_MODEL", DEFAULT_MODEL))
    parser.add_argument("--out")
    parser.add_argument("--compare")
    args = parser.parse_args()

    bin_path = binary()
    tasks = load_tasks(args.task)
    results = {"model": args.model, "binary": bin_path, "tasks": {}}

    for task in tasks:
        runs = []
        for n in range(args.repeats):
            print(f"  {task['id']} run {n + 1}/{args.repeats}", file=sys.stderr, flush=True)
            runs.append(run_once(task, bin_path, args.base_url, args.model))
        results["tasks"][task["id"]] = summarize(runs)

    baseline = json.loads(pathlib.Path(args.compare).read_text()) if args.compare else None
    render(results, baseline)

    if args.out:
        out = pathlib.Path(args.out)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(results, indent=2) + "\n")
        print(f"\nwrote {out}", file=sys.stderr)


if __name__ == "__main__":
    main()
