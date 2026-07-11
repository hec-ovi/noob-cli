# noob/tests

End-to-end tests through the compiled binary. Tests use temporary config and work directories plus noob-testkit servers, then assert exit status, stdout and stderr, file effects, JSONL, session replay, and recorded wire data.

The mock automatically enforces transcript validity, byte-prefix stability, tool-array stability, and the absence of request output limits.

`e2e_ui.rs` has 24 tests using real pseudo-terminals for raw editing, dock rendering, queueing, confirmations, Markdown, reader loss, hard cancellation, and terminal restoration. Non-UI suites explicitly disable the dock when testing legacy plain interaction semantics.

`install_bundle.rs` crosses real Bash process boundaries with a fake Docker executable. It verifies image-build arguments, workspace and config mounts, websearch seeding, restore forwarding, API-key isolation, and safe overwrite behavior.

There are 522 offline tests and 8 ignored live tests. Live tests run serially through `./dev.sh smoke`; by default seven use the model endpoint on port 8090, and the websearch MCP test also uses the Streamable HTTP endpoint on port 8000. `NOOB_LIVE_BASE_URL` and `NOOB_LIVE_MCP_URL` override those endpoints. Offline tests do not contact external endpoints.
