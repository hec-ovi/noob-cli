# crates/noob/prompts

`base.md` (system prompt layer 1) and `compact.md` (compaction summarizer
prompt), compiled in via include_str!.

Invariants:
- base.md <= 500 tokens, budget-tested on the shipped artifact through
  `noob debug prompt --json` (tiktoken o200k offline; llama-server /tokenize
  live). Budget numbers live in one const block in the budget test.
- No word, sentence, or length caps anywhere in either text; output is
  shaped by content instructions only. The budget test lints for forbidden
  cap phrasing.
- base.md is frozen at session start (cache prefix discipline); editing it
  ships a new binary, there is no runtime override.
