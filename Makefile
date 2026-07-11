# Thin wrapper for make users; ./dev.sh is the real task runner and the only
# requirement is docker plus a shell.

.PHONY: all test build smoke docker install repl size-check clean

all: test

test build smoke docker install repl size-check clean:
	./dev.sh $@
