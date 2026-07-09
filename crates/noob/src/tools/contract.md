# noob/src/tools

The built-in tool set (P2): read, write, edit, bash, grep, glob, ls, plus a
registry. Each tool is a pure fn(args) -> ToolResult with no knowledge of the
agent loop.

Key rules: read returns plain text with no line numbers; edit is exact string
replace with a deterministic fallback ladder and hard ambiguity rejection,
guarded by read-before-edit and hash check-and-set; writes are atomic
(temp file + rename). Every result is truncated once at emission with a
marker that names the next action, then byte-frozen.
