# noob/src/tools

Built-in tools and their schemas. Core tools are read, write, edit, bash, grep, glob, and ls. Skill, MCP, and task tools register only when available.

## File operations

- `read` opens with `O_NONBLOCK`, checks the opened handle is a regular file, drains the whole file in chunks, hashes all bytes, and retains only the requested bounded page. It returns no line-number prefixes.
- `write` and `edit` require current read stamps before overwriting, reject symlink escapes in workspace mode, and publish through same-directory temp, fsync, and rename.
- `edit` tries exact bytes, trailing whitespace, typographic folding, uniform indentation, and CRLF-compatible views. Every stage rejects ambiguity.

## Processes and results

- `bash` merges stdout and stderr, continuously drains through a bounded head/tail buffer, and kills its process group on timeout or cancellation.
- `grep` and `glob` honor gitignore; `ls` lists explicitly.
- Retained tool results are bounded once before transcript insertion and include continuation instructions when clipped.
- Tool operations continue to completion or cancellation even when only a bounded display/context result is retained.
- `ToolOutcome.canceled` is a structural flag, including for tools already running when interruption arrives.

Skills-directory write and edit targets require a one-use approval recorded by the agent and rechecked against the real execution target.
