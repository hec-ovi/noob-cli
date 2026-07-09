# Dev stage: the toolchain `make test` and `make build` run in, so nothing
# gets installed on the host. build-base carries the C compiler ring needs.
FROM rust:1.96-alpine AS dev
RUN apk add --no-cache build-base jq

# Builder: static musl release binary.
FROM dev AS builder
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# The base image disables crt-static (for proc-macros, which we do not have);
# re-enable it so the binary is fully static.
ENV RUSTFLAGS="-C target-feature=+crt-static"
RUN cargo build --release --locked --target x86_64-unknown-linux-musl

# Runtime: the sandbox the agent lives in. bash for the bash tool, git with
# system roots for https clones; noob itself uses compiled-in webpki roots.
# No config, no state, no keys in the image, ever.
FROM alpine:3.22
RUN apk add --no-cache bash git ca-certificates
COPY --from=builder /src/target/x86_64-unknown-linux-musl/release/noob /usr/local/bin/noob
ENV NOOB_SANDBOX=container
WORKDIR /work
ENTRYPOINT ["noob"]
