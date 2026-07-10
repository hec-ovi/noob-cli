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

case "${1:-test}" in
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
  # The runtime image.
  docker)
    docker build -t noob -f docker/Dockerfile .
    ;;
  # Interactive REPL / one-shot through compose, with the caller's uid:gid
  # passed explicitly (compose only sees UID/GID when the shell exports them,
  # which most shells do not).
  repl)
    docker compose run --rm --user "$UIDGID" noob
    ;;
  exec)
    shift
    docker compose run --rm --user "$UIDGID" noob exec -p "${1:?usage: ./dev.sh exec \"prompt\"}"
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
  *)
    echo "usage: ./dev.sh {test|build|smoke|docker|repl|exec \"prompt\"|size-check|clean}" >&2
    exit 2
    ;;
esac
