# docker/

The two-stage musl Dockerfile.

- `dev` stage: rust:alpine + build-base; the toolchain every `make` target
  runs in, so nothing gets installed on the host.
- `builder` stage: static release build (`+crt-static`).
- runtime stage: alpine + bash + git + ca-certificates and the `noob` binary.

Invariants: the runtime image contains zero state, config, or keys; the agent
works on the `/work` bind mount, never the container filesystem. The compose
file at the repo root points here.
