# Everything runs inside Docker; the host needs docker and nothing else.
# Cargo caches live in ./.cargo-home and ./target (both gitignored), so
# incremental builds survive across runs and stay user-owned.

SHELL := /bin/bash
export UID := $(shell id -u)
export GID := $(shell id -g)

DEV_IMG := noob-dev
BIN := target/x86_64-unknown-linux-musl/release/noob
RUN := docker run --rm --user $(UID):$(GID) \
	-e CARGO_HOME=/src/.cargo-home \
	-v $(CURDIR):/src -w /src $(DEV_IMG)

.PHONY: all test build smoke docker size-check dev-image clean

all: test

dev-image:
	docker build --target dev -t $(DEV_IMG) -f docker/Dockerfile .

# Offline test suite: unit + e2e against the in-process mock. The whole story;
# there is no CI.
test: dev-image
	$(RUN) cargo test --workspace

# Static release binary (same flags as the runtime image build).
build: dev-image
	$(RUN) env RUSTFLAGS="-C target-feature=+crt-static" \
		cargo build --release --locked --target x86_64-unknown-linux-musl

# Live smoke suite against local endpoints (qwen at :8090 etc). Opt-in.
smoke: dev-image
	docker run --rm --network host --user $(UID):$(GID) \
		-e CARGO_HOME=/src/.cargo-home -e NOOB_LIVE=1 \
		-v $(CURDIR):/src -w /src $(DEV_IMG) \
		cargo test --workspace -- --ignored

# The runtime image.
docker:
	docker build -t noob -f docker/Dockerfile .

# Footprint budgets from ARCHITECTURE.md: stripped binary <= 8 MB,
# runtime crate graph <= 45.
size-check: build
	@size=$$(stat -c%s $(BIN)); \
	echo "binary: $$size bytes"; \
	if [ $$size -gt 8388608 ]; then echo "FAIL: binary exceeds 8 MB"; exit 1; fi
	@crates=$$($(RUN) sh -c 'cargo tree -p noob -e normal --prefix none | sort -u | wc -l'); \
	echo "runtime crates: $$crates"; \
	if [ $$crates -gt 45 ]; then echo "FAIL: crate graph exceeds 45"; exit 1; fi

clean:
	rm -rf target .cargo-home
