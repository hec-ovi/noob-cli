# noob/src/skills

SKILL.md discovery and progressive disclosure (P3), per the agentskills.io
standard. Four discovery paths, first hit per name wins: `/work/.noob/skills`,
`/work/.claude/skills`, `/work/.agents/skills`, `/config/skills`.

Disclosure levels: one index line per skill in the prompt; the `skill` tool
returns the body as a tool result (24 KiB cap); bundled files are read with
the ordinary read tool.

Invariants: the prompt head never mutates when a skill loads; skill bodies are
untrusted input and never granted authority; the agent never authors skills,
and any write into a skills directory requires explicit confirmation.
