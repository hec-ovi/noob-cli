#!/usr/bin/env bash
# Task runner. Everything runs inside Docker: the host needs docker and a
# shell, nothing else (not even make; the Makefile just delegates here).
# Cargo caches live in ./.cargo-home and ./target (gitignored, user-owned).
set -euo pipefail
cd "$(dirname "$0")"
ROOT="$PWD"

DEV_IMG=noob-dev
case "$(uname -m)" in
  x86_64) RUST_TARGET=x86_64-unknown-linux-musl ;;
  aarch64|arm64) RUST_TARGET=aarch64-unknown-linux-musl ;;
  *) echo "unsupported architecture: $(uname -m)" >&2; exit 2 ;;
esac
BIN="target/$RUST_TARGET/release/noob"
UIDGID="$(id -u):$(id -g)"
RUN=(docker run --rm --user "$UIDGID" -e CARGO_HOME=/src/.cargo-home
     -v "$PWD":/src -w /src "$DEV_IMG")

dev_image() {
  docker build --build-arg "TARGETARCH=${RUST_TARGET%%-*}" --target dev \
    -t "$DEV_IMG" -f docker/Dockerfile .
}

# Open the interactive agent: build the runtime image (cached, so fast when
# nothing changed) and run it through compose in one step, forwarding any noob
# flags (e.g. --resume <id>). Compose passes the caller's uid:gid so files
# under /work are never root-owned on the host.
open_agent() {
  local workspace="${NOOB_WORKSPACE:-${WORKSPACE:-$ROOT/workspace}}"
  mkdir -p "$workspace"
  workspace="$(cd "$workspace" && pwd -P)"
  NOOB_WORKSPACE="$workspace" docker compose run --build --rm --user "$UIDGID" noob "$@"
}

case "${1:-}" in
  # Offline suite: unit + e2e against the in-process mock. The whole story;
  # there is no CI.
  test)
    dev_image
    # Never share native test artifacts with a host Rust toolchain. A glibc
    # binary under target/debug is not runnable in the Alpine dev container,
    # and Cargo can otherwise mistake it for a fresh artifact.
    "${RUN[@]}" env CARGO_TARGET_DIR=/src/target/docker-dev cargo test --workspace
    ;;
  # Static release binary (same flags as the runtime image build).
  build)
    dev_image
    "${RUN[@]}" env RUSTFLAGS="-C target-feature=+crt-static" \
      sh -c 'rustup target add "$1" && cargo build --release --locked --target "$1"' \
      sh "$RUST_TARGET"
    ;;
  # Live smoke suite against local endpoints (qwen at :8090 etc). Opt-in.
  # Serialized: parallel live tests share one llama-server and evict each
  # other's KV-cache slots, which flakes the cached-share assertions.
  smoke)
    dev_image
    docker run --rm --network host --user "$UIDGID" \
      -e CARGO_HOME=/src/.cargo-home -e NOOB_LIVE=1 \
      -e CARGO_TARGET_DIR=/src/target/docker-dev \
      -e NOOB_LIVE_BASE_URL -e NOOB_LIVE_MCP_URL \
      -v "$PWD":/src -w /src "$DEV_IMG" \
      cargo test --workspace -- --ignored --test-threads=1
    ;;
  # Build the runtime image without running it.
  docker)
    docker build --build-arg "TARGETARCH=${RUST_TARGET%%-*}" -t noob -f docker/Dockerfile .
    ;;
  install)
    shift
    exec ./install.sh "$@"
    ;;
  # One-shot headless run through compose.
  exec)
    shift
    open_agent exec -p "${1:?usage: ./dev.sh exec \"prompt\"}"
    ;;
  # Footprint budgets from ARCHITECTURE.md: stripped binary <= 8 MiB,
  # runtime crate graph <= 45.
  size-check)
    "$0" build
    size=$(stat -c%s "$BIN")
    echo "binary: $size bytes"
    [ "$size" -le 8388608 ] || { echo "FAIL: binary exceeds 8 MiB"; exit 1; }
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
    echo "  ./dev.sh --resume <id>      resume a saved session"
    echo "  ./dev.sh --plan | --yolo    any noob flag is forwarded to the agent"
    echo "  ./dev.sh install|test|build|docker|exec \"prompt\"|smoke|size-check|clean"
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
