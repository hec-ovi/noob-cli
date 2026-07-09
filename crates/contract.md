# crates/

The cargo workspace.

- `noob` the binary: argv dispatch, agent loop, tools, UI.
- `noob-provider` transcript in, events out; both OpenAI wire shapes; the
  only crate allowed to depend on ureq (test-enforced from P2).
- `noob-testkit` dev-only mock OpenAI server with automatic wire assertions;
  never a runtime dependency.

Dependency direction: `noob -> noob-provider`. Nothing depends on the testkit
except dev-dependencies.
