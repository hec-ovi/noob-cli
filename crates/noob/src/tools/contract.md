# noob/src/tools

Built-in tools and their schemas. Core tools are read, write, edit, bash, grep, glob, ls, context, and plan. Skill, MCP, and subagent tools register only when available. Historical `todo` calls remain dispatchable for resumed transcripts, but only `plan` is registered.

## File operations

- `read` opens with `O_NONBLOCK`, checks the opened handle is a regular file, drains the whole file in chunks, hashes all bytes, and retains only the requested bounded page. It returns no line-number prefixes.
- `write` and `edit` require current read stamps before overwriting, reject symlink escapes in workspace mode, and publish through same-directory temp, fsync, and rename.
- `edit` tries exact bytes, trailing whitespace, typographic folding, uniform indentation, and CRLF-compatible views. Every stage rejects ambiguity.

## Processes and results

- `bash` merges stdout and stderr, continuously drains through a bounded head/tail buffer, and kills its process group on timeout or cancellation.
- `write`, `edit`, and `bash` take an OS lease on the workspace directory for the duration of one call. Leased calls do not overlap. Detached children wait for a bounded interval; a root call reports a conflict promptly. The advisory lease does not cover unmanaged or deliberately detached processes.
- `grep` and `glob` honor gitignore; `ls` lists explicitly and opens with a `<dir>:` header line so bare entry names always carry their base path.
- Retained tool results are bounded once before transcript insertion and include continuation instructions when clipped.
- `context` reports the same estimated use, configured total, and 75 percent compaction threshold as the agent loop.
- `plan` reports total elapsed time and records each completed item's elapsed time in its transcript-visible checklist.
- Tool operations continue to completion or cancellation even when only a bounded display/context result is retained.
- `ToolOutcome.canceled` is a structural flag, including for tools already running when interruption arrives.

Skills-directory write and edit targets require a one-use approval recorded by the agent and rechecked against the real execution target.
