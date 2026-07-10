#!/usr/bin/env bash
# Task runner. Everything runs inside Docker: the host needs docker and a
# shell, nothing else (not even make; the Makefile just delegates here).
# Cargo caches live in ./.cargo-home and ./target (gitignored, user-owned).
set -euo pipefail
cd "$(dirname "$0")"

DEV_IMG=noob-dev
BIN=target/x86_64-unknown-linux-musl/release/noob
UIDGID="$(id -u):$(id -g)"
RUN=(docker run --rm --user "$UIDGID" -e CARGO_HOME=/src/.cargo-home
     -v "$PWD":/src -w /src "$DEV_IMG")

dev_image() { docker build --target dev -t "$DEV_IMG" -f docker/Dockerfile .; }

# Open the interactive agent: build the runtime image (cached, so fast when
# nothing changed) and run it through compose in one step, forwarding any noob
# flags (e.g. --session <id>). Compose passes the caller's uid:gid so files
# under /work are never root-owned on the host.
open_agent() { docker compose run --build --rm --user "$UIDGID" noob "$@"; }

case "${1:-}" in
  # Offline suite: unit + e2e against the in-process mock. The whole story;
  # there is no CI.
  test)
    dev_image
    "${RUN[@]}" cargo test --workspace
    ;;
  # Static release binary (same flags as the runtime image build).
  build)
    dev_image
    "${RUN[@]}" env RUSTFLAGS="-C target-feature=+crt-static" \
      cargo build --release --locked --target x86_64-unknown-linux-musl
    ;;
  # Live smoke suite against local endpoints (qwen at :8090 etc). Opt-in.
  # Serialized: parallel live tests share one llama-server and evict each
  # other's KV-cache slots, which flakes the cached-share assertions.
  smoke)
    dev_image
    docker run --rm --network host --user "$UIDGID" \
      -e CARGO_HOME=/src/.cargo-home -e NOOB_LIVE=1 \
      -v "$PWD":/src -w /src "$DEV_IMG" \
      cargo test --workspace -- --ignored --test-threads=1
    ;;
  # Build the runtime image without running it.
  docker)
    docker build -t noob -f docker/Dockerfile .
    ;;
  # One-shot headless run through compose.
  exec)
    shift
    open_agent exec -p "${1:?usage: ./dev.sh exec \"prompt\"}"
    ;;
  # Footprint budgets from ARCHITECTURE.md: stripped binary <= 8 MB,
  # runtime crate graph <= 45.
  size-check)
    "$0" build
    size=$(stat -c%s "$BIN")
    echo "binary: $size bytes"
    [ "$size" -le 8388608 ] || { echo "FAIL: binary exceeds 8 MB"; exit 1; }
    crates=$("${RUN[@]}" sh -c \
      'cargo tree -p noob -e normal --prefix none | sed "s/ (\*)$//" | sort -u | wc -l')
    echo "runtime crates: $crates"
    [ "$crates" -le 45 ] || { echo "FAIL: crate graph exceeds 45"; exit 1; }
    ;;
  clean)
    rm -rf target .cargo-home
    ;;
  -h|--help|help)
    echo "usage:"
    echo "  ./dev.sh                    open the agent"
    echo "  ./dev.sh --session <id>     resume a saved session"
    echo "  ./dev.sh --plan | --yolo    any noob flag is forwarded to the agent"
    echo "  ./dev.sh test|build|docker|exec \"prompt\"|smoke|size-check|clean"
    ;;
  # Bare `./dev.sh`, or leading-dash noob flags (--session, --plan, ...): open
  # the agent. `repl` is kept as a silent alias for old muscle memory.
  ""|-*)
    open_agent "$@"
    ;;
  repl)
    shift
    open_agent "$@"
    ;;
  *)
    echo "./dev.sh: unknown command '${1}'; run ./dev.sh --help" >&2
    exit 2
    ;;
esac
