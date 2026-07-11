# docker/

The three-stage musl Dockerfile.

- `dev` stage: rust:alpine plus build-base, jq, Bash, and Git; the toolchain
  every `./dev.sh` target runs in, so nothing gets installed on the host.
- `builder` stage: static release build (`+crt-static`).
- `runtime` stage: Alpine, Bash, Git, CA certificates, Python 3, uv, the pinned
  `websearch-skill` tool environment, and the `noob` binary.

Invariants: the runtime image contains zero state, config, or keys; the agent
works on the `/work` bind mount, never the container filesystem. The compose
file and installed host launcher bind `/work` and `/config`. The builder selects
the static musl target for Docker `amd64` or `arm64`.
