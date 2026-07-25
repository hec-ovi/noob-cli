---
name: coding
description: >-
  Changing code that already exists: a feature, a bug fix, a refactor, a test, or
  running a project's checks.
---

# Changing code that already exists

## Before writing anything

Read the file you are about to change, then one neighbour that does the same kind of job.
Copy what you find there: naming, error handling, how failures are returned, how tests are
laid out, how much the code comments itself. A change that reads like the code around it is
the one a reviewer accepts.

A library exists only if this project already declares it. Before you import anything, look
in the manifest: `Cargo.toml`, `package.json`, `pyproject.toml`, `requirements.txt`,
`go.mod`, `Gemfile`, `composer.json`, `build.gradle`. Popular is not the same as installed.
If it is not declared, use what the standard library gives you, or say plainly that the
change needs a new dependency and let the person decide. The same goes for a command you
plan to run: check it is on PATH before you build a plan around it.

Look for the project's own instructions too (`AGENTS.md`, `CLAUDE.md`, `CONTRIBUTING.md`,
the README). They outrank your defaults.

## While writing

Make the smallest change that does the whole job. Touching unrelated lines, reformatting a
file you happened to open, or renaming things on the way past all cost the reader time and
hide the actual change.

Prefer editing a file over rewriting it. A rewrite regenerates every byte, including the
bytes that were already correct, and it destroys anything you did not know was there.

Comment what the next reader cannot work out from the code: why this way, what breaks
otherwise, which constraint forced it. Do not narrate what the line already says, and do
not leave a note about what you did; that belongs in your reply, not in the file.

Never write a key, token, or password into a file, a log line, or an error message.

## Before saying it is done

Compiling is not running. Parsing is not running. Type-checking is not running. Run the
thing: execute the test, call the endpoint, invoke the command, load the page. Code that
passes every static check still crashes on the first line of real work, and the only way to
know is to make it work.

Find the project's real commands instead of guessing them: the README, `Makefile`,
`justfile`, `dev.sh`, the `scripts` block in `package.json`, `tox.ini`, `noxfile.py`, or a
workflow file under `.github/` (read it for the commands, do not try to run CI). Run the
tests and the linter the project actually uses.

If a behavioural change has no test covering it, add one, next to the tests that already
exist and in their style. If the project has no test setup at all, say so rather than
inventing a framework it does not use.

Then report what you ran and what came back. If a check did not run, name it and say why.
An unrun check reported as passing is worse than no check.

## Do not

- Commit or push unless you were asked to.
- Add a dependency for something the standard library already does.
- Leave a stub, a `TODO`, or a silently swallowed error behind as if the work were finished.
- Claim a result you did not observe.
