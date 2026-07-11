# noob/src/session

Append-only JSONL session persistence under `<config>/sessions/<id>.jsonl`.

Fresh IDs combine hexadecimal milliseconds, process ID, and a per-process serial. Resume validates IDs, skips corrupt lines, repairs dangling tool calls, applies reset records, and reconstructs a provider-valid transcript.

Append, flush, reset, and repair errors are returned to the agent. They are never ignored. On a live persistence failure the agent detaches the session, continues with its valid in-memory transcript, and warns once.

Sessions live in the config mount, not the image or project workspace.
