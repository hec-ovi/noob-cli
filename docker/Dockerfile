# Dev stage: the toolchain `./dev.sh test` and `./dev.sh build` run in, so
# nothing gets installed on the host. build-base carries the C compiler ring
# needs; bash and git are what the bash-tool tests exercise (the runtime
# stage ships the same pair).
FROM rust:1.96-alpine AS dev
RUN apk add --no-cache build-base jq bash git

# Builder: static musl release binary.
FROM dev AS builder
ARG TARGETARCH
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# The base image disables crt-static (for proc-macros, which we do not have);
# re-enable it so the binary is fully static.
ENV RUSTFLAGS="-C target-feature=+crt-static"
RUN case "${TARGETARCH:-$(uname -m)}" in \
      amd64|x86_64) rust_target=x86_64-unknown-linux-musl ;; \
      arm64|aarch64) rust_target=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported Docker architecture: ${TARGETARCH:-$(uname -m)}" >&2; exit 2 ;; \
    esac \
    && rustup target add "$rust_target" \
    && cargo build --release --locked --target "$rust_target" \
    && install -D -m 0755 "target/$rust_target/release/noob" /out/noob

# Runtime: the sandbox the agent lives in. Python and uv make Python-backed
# skills usable without touching the host. websearch-skill is pinned and
# installed into its own uv tool environment, exposing both the standalone
# `websearch` CLI and `websearch mcp` from the same package.
# No config, state, cache, or keys are baked into the image.
#
# WEBSEARCH_ENV_FILE moves that tool's dotenv off its working directory. It
# reads `.env` from the cwd and exports every key it finds, and the agent's cwd
# is /work, the user's own project. Verified against 0.2.5: a project .env
# naming a dead proxy made every search fail closed, while the same container
# with no .env searched fine. A project's own secrets have no business in the
# environment of a process that opens sockets, and the sanctioned places to
# configure this tool are the launcher's forwarded variables and /config.
#
# nodejs is here because its absence was expensive, not for completeness. Asked
# to write a browser game, the model tried `node -e` to check its own
# JavaScript, got "not found", and then spent twelve rounds reimplementing the
# game's pathfinding in Python to verify it by simulation: 43,412 generated
# tokens against 19,053 for the same task in a sandbox that had node. A
# language runtime for the language the agent is writing pays for itself in one
# avoided round.
FROM alpine:3.22 AS runtime
RUN apk add --no-cache bash git ca-certificates python3 uv nodejs
ARG WEBSEARCH_SKILL_VERSION=0.2.5
RUN UV_CACHE_DIR=/tmp/uv-cache \
    UV_TOOL_DIR=/opt/noob/uv-tools \
    UV_TOOL_BIN_DIR=/usr/local/bin \
    uv tool install --python /usr/bin/python3 "websearch-skill==${WEBSEARCH_SKILL_VERSION}" \
    && rm -rf /tmp/uv-cache \
    && mkdir -p /tmp/noob-home \
    && chmod 1777 /tmp/noob-home
COPY --from=builder /out/noob /usr/local/bin/noob
ENV NOOB_SANDBOX=container \
    NOOB_DOCK=1 \
    NOOB_RAW=1 \
    NOOB_THEME=matrix \
    COLORTERM=truecolor \
    HOME=/tmp/noob-home \
    UV_CACHE_DIR=/config/cache/uv \
    WEBSEARCH_ENV_FILE=/config/websearch.env \
    PYTHONDONTWRITEBYTECODE=1
WORKDIR /work
ENTRYPOINT ["noob"]
