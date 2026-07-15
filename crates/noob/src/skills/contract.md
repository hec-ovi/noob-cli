# noob/src/skills

SKILL.md discovery, parsing, resolver indexing, and user-driven installation.

Discovery order is project `.noob/skills`, `.claude/skills`, `.agents/skills`, then `<config>/skills`. First name wins, roots are sorted, and malformed entries warn without stopping discovery.

Required frontmatter is a lowercase name of at most 64 characters and a description of at most 1024 characters. The prompt receives a bounded name and description index. The `skill` tool loads the body, and `read` loads bundled files.

`/skills add` accepts a skill directory, bare `SKILL.md`, Git URL, or `owner/repo` GitHub shorthand (an existing local path wins over the shorthand reading). Frontmatter is validated first. Local content is copied into hidden staging and published with one rename. Git clone uses hidden staging, a process group, bounded diagnostics, a 120-second timeout, and interrupt checks. Partial installs never enter the discovery root.

`/skills remove` only deletes workspace-contained skill directories. Reload swaps the live index, updates tool registration if needed, and appends an in-band resolver correction.

Agent write/edit operations inside skills directories still require real-terminal confirmation. Headless and child modes deny them.
