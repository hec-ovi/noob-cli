# crates/noob-provider/testdata

Captured wire transcripts for deterministic replay. Not compiled in; read
by the test suites only.

`sse/*.sse` are SSE byte transcripts. The llama.cpp files are real captures
from qwen3.6-35b-a3b (chat tool call, parallel tool calls, /v1/responses
function call); the OpenRouter file is synthesized to its documented shape
(comment keepalives, mid-stream in-band error).

`%%CHUNK%%` marks a TCP chunk boundary. The loader
(`noob_testkit::load_fixture_chunks`) removes the sentinel and NOTHING
else, so a sentinel may sit mid-line, mid-keyword, or mid-codepoint; the
fixtures deliberately do all three, which is the point of the format. A
file with sentinels inside a multibyte codepoint is not valid UTF-8 as a
whole; treat these files as bytes, never run text tools that re-encode
them.

Invariant: fixtures are never edited to make a test pass; they change only
when re-captured from a real server (note the source in the test that
replays them).
