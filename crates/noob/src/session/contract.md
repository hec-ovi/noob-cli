# noob/src/session

Append-only JSONL session persistence under `<config>/sessions/<id>.jsonl`.

Fresh IDs combine hexadecimal milliseconds, process ID, and a per-process serial. `noob sessions` and `/sessions` list valid files newest first; `--resume latest` selects the newest. Resume validates IDs, skips corrupt records with one warning and a bounded count, repairs dangling tool calls and unfinished acknowledged background jobs, applies reset records, and reconstructs a provider-valid transcript.

Append, flush, reset, and repair errors are returned to the agent. They are never ignored. On a live persistence failure the agent detaches the session, continues with its valid in-memory transcript, and warns once.

Compaction and explicit plan cleanup use reset records, so resume reconstructs the current active context rather than superseded payloads.

Sessions live in the config mount, not the image or project workspace.
