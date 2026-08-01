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

# Run box contract checks: each argument is a tests/contract.py validating what
# crosses that box's boundary against its schema/. Skipped rather than failed
# where jsonschema is absent, so a missing dev-only Python package cannot block
# a build.
run_contracts() {
  for contract in "$@"; do
    [ -f "$contract" ] || continue
    if python3 -c 'import jsonschema' 2>/dev/null; then
      python3 "$contract" || return 1
    else
      echo "skipped $contract (pip install jsonschema to run it)"
    fi
  done
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
  # Live smoke suite against local endpoints (llama.cpp at :8080 etc). Opt-in.
  # Serialized: parallel live tests share one llama-server and evict each
  # other's KV-cache slots, which flakes the cached-share assertions.
  smoke)
    dev_image
    docker run --rm --network host --user "$UIDGID" \
      -e CARGO_HOME=/src/.cargo-home \
      -e CARGO_TARGET_DIR=/src/target/docker-dev \
      -e NOOB_LIVE_BASE_URL -e NOOB_LIVE_MODEL -e NOOB_LIVE_MCP_URL \
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
  gui)
    # NO0B, from its own workspace. Built on the host rather than in the
    # container: it needs a GPU, a compositor and a window, none of which the
    # CLI's build image has or should have. The folder is still gui/clippy; the
    # package and the binary are no0b.
    shift
    cargo build --release --manifest-path gui/Cargo.toml
    NOOB_BIN="${NOOB_BIN:-$PWD/target/release/noob}" \
      exec gui/target/release/no0b "$@"
    ;;
  gui-test)
    cargo test --manifest-path gui/Cargo.toml
    cargo clippy --manifest-path gui/Cargo.toml --all-targets -- -D warnings
    # Layer contract tests: the Rust tests above prove behavior, these prove
    # that what crosses a layer boundary matches its declared schema.
    run_contracts gui/layers/*/tests/contract.py
    ;;
  gui-check)
    cargo build --release --manifest-path gui/Cargo.toml
    size=$(stat -c%s gui/target/release/no0b)
    echo "no0b binary: $size bytes"
    [ "$size" -le 41943040 ] || { echo "FAIL: NO0B exceeds 40 MiB"; exit 1; }
    crates=$(cargo tree --manifest-path gui/Cargo.toml -p no0b -e normal \
      --prefix none | sed 's/ (\*)$//' | awk '{print $1}' | sort -u | grep -c .)
    echo "no0b runtime crates: $crates"
    [ "$crates" -le 400 ] || { echo "FAIL: NO0B crate graph exceeds 400"; exit 1; }
    ;;
  check-macos)
    # Type-check every crate of both workspaces for aarch64-apple-darwin.
    # The mac binaries are built by the release workflow on real runners;
    # this catches unix-only code at the desk. Needs the target's std and a
    # darwin-capable C compiler for ring's build script; zig serves as one.
    # Skipped rather than failed where zig is absent, so a machine without
    # it still runs everything else.
    if ! command -v zig >/dev/null 2>&1; then
      echo "skipped check-macos (install zig to type-check the mac target)"
      exit 0
    fi
    rustup target add aarch64-apple-darwin >/dev/null 2>&1 || true
    wrap=$(mktemp -d)
    trap 'rm -rf "$wrap"' EXIT
    cat > "$wrap/cc" <<'WRAP'
#!/bin/sh
# zig as darwin cc: drop the clang target flags zig cannot parse and pin
# the zig triple instead.
args=""
skip=0
for a in "$@"; do
  if [ "$skip" = 1 ]; then skip=0; continue; fi
  case "$a" in
    --target=*) continue ;;
    -arch) skip=1; continue ;;
    -mmacosx-version-min=*) continue ;;
    *) args="$args \"$a\"" ;;
  esac
done
eval exec zig cc -target aarch64-macos $args
WRAP
    printf '#!/bin/sh\nexec zig ar "$@"\n' > "$wrap/ar"
    chmod +x "$wrap/cc" "$wrap/ar"
    export CC_aarch64_apple_darwin="$wrap/cc" AR_aarch64_apple_darwin="$wrap/ar"
    cargo check --workspace --target aarch64-apple-darwin
    cargo check --manifest-path gui/Cargo.toml --workspace --target aarch64-apple-darwin
    echo "check-macos: both workspaces type-check for aarch64-apple-darwin"
    ;;
  # Every suite and gate in one pass: the CLI tests, the NO0B tests and
  # clippy, every box contract in the tree, both footprint gates. Stops at
  # the first failure; the last line is the verdict.
  test-all)
    step() { "$0" "$1" || { echo "test-all: FAIL at $1"; exit 1; }; }
    step test
    step gui-test
    # git finds new boxes without a list to maintain; -o adds not-yet-tracked
    # contract runners, --exclude-standard keeps target/ and workspace/ out.
    mapfile -t contracts < <(git ls-files -co --exclude-standard '*/tests/contract.py' | sort)
    run_contracts "${contracts[@]}" || { echo "test-all: FAIL at contracts"; exit 1; }
    step size-check
    step gui-check
    step check-macos
    echo "test-all: PASS"
    ;;
  gui-package|gui-install)
    # Both routes stage the same directory, because they were two copies of the
    # same list once and the copies disagreed: a glob matched the icon and the
    # launcher but not the icon's small variant, and the install died on a file
    # nobody had noticed was missing. `gui/data` is copied whole now, so the
    # staged set cannot drift from what is in the repository.
    #
    # Not a Flatpak or an AppImage: the binary links the system's C library and
    # loads the machine's own GPU and display libraries at runtime, and a driver
    # is exactly what a bundle cannot ship.
    command=$1
    shift
    version=$(awk -F'"' '/^version/ {print $2; exit}' gui/Cargo.toml)
    arch=$(uname -m)
    stage="gui/target/package/no0b-$version-$arch-linux"
    cargo build --release --manifest-path gui/Cargo.toml
    rm -rf "$stage"
    install -d "$stage"
    install -m 0755 gui/target/release/no0b "$stage/no0b"
    install -m 0755 gui/data/install.sh "$stage/install.sh"
    for asset in gui/data/io.github.hec_ovi.NO0B*; do
        install -m 0644 "$asset" "$stage/"
    done
    if [ "$command" = gui-install ]; then
        exec bash "$stage/install.sh" "$@"
    fi
    tarball="$(dirname "$stage")/$(basename "$stage").tar.gz"
    tar -czf "$tarball" -C "$(dirname "$stage")" "$(basename "$stage")"
    echo "$tarball"
    echo "$(stat -c%s "$tarball") bytes"
    ;;
  clean)
    rm -rf target .cargo-home gui/target
    ;;
  -h|--help|help)
    echo "usage:"
    echo "  ./dev.sh                    open the agent"
    echo "  ./dev.sh --resume <id>      resume a saved session"
    echo "  ./dev.sh --plan | --yolo    any noob flag is forwarded to the agent"
    echo "  ./dev.sh install|test|build|docker|exec \"prompt\"|smoke|size-check|clean"
    echo "  ./dev.sh test-all            every suite and gate in one pass"
    echo "  ./dev.sh check-macos         type-check both workspaces for the mac target"
    echo "  ./dev.sh gui [workspace]     open NO0B, the GPU front end"
    echo "  ./dev.sh gui-test|gui-check  NO0B tests and its own size gate"
    echo "  ./dev.sh gui-install         install NO0B under ~/.local"
    echo "  ./dev.sh gui-package         build the release tarball"
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
