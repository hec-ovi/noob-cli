# noob/tests

End-to-end tests through the compiled binary: each test spawns
env!("CARGO_BIN_EXE_noob") with NOOB_CONFIG_DIR at a temp dir whose .env
targets a noob-testkit mock server, then asserts stdout, exit codes, file
side effects, and the recorded wire bytes.

Every test must end with the mock's `assert_clean()`, which surfaces the
automatic wire assertions (prefix stability, no output caps, transcript
validity).

Live smoke tests are `#[ignore]` and run via `make smoke` (NOOB_LIVE=1)
against local endpoints; the offline suite never touches the network.
