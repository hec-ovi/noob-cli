# crates/noob/prompts

`base.md` (system prompt) and `compact.md` (compaction summarizer prompt),
compiled in via include_str!. Lands in P2.

Invariants: base prompt <= 500 tokens, budget-tested against both tiktoken
o200k and the live endpoint tokenizer on the shipped artifact. No word,
sentence, or length caps anywhere in the text; output is shaped by content
instructions only.
