# noob/src/session

Session persistence (P2): append-only JSONL transcripts under
`/config/sessions/<id>.jsonl`, one event per line. `exec --session <id>`
resumes by replaying the transcript.

Invariants: sessions live in the config mount, never in the image or the
workspace; files only append during a session; a resumed transcript must
serialize back byte-identically so the cache prefix survives resume.
