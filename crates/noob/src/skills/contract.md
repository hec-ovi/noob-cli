# noob/src/skills

SKILL.md discovery and the L1 resolver index (agentskills.io standard).
Four discovery paths at session start, first hit per name wins:
`/work/.noob/skills`, `/work/.claude/skills`, `/work/.agents/skills`,
`/config/skills`; alphabetical within a root, so discovery is
deterministic. Frontmatter (required `name` <= 64 chars lowercase+digits+
hyphens, `description` <= 1024 chars) is parsed by the hand-rolled scanner
(plain scalars, quoted strings, `|`/`>` blocks, nested keys ignored);
malformed skills are skipped with a stderr warning, never a crash.

Disclosure (GBrain resolver / thin-harness-fat-skills pattern: the index
is the dispatcher, descriptions are the triggers, bodies cost zero tokens
until loaded): L1 is the `# Skills (resolver)` section in the prompt (one
line per skill, description clipped at 200 chars, section capped at 4,000
chars ~ 1,000 tokens, overflow degrades to name-only lines then a count
note). L2 is the `skill` tool (tools/skill.rs) returning `body_of()`: the
byte-exact SKILL.md suffix after the frontmatter. L3 is ordinary `read` of
bundled files.

Invariants: the prompt head never mutates when a skill loads; skill bodies
are untrusted input and never granted authority; the agent never authors
skills (write/edit into `**/skills/**` is confirmation-gated in the agent
loop, headless denies); `body_of` is lenient at call time (an unparseable
file degrades to the whole text, it never errors).
