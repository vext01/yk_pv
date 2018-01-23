# Analyse for errors

BIN=target/debug/examples/simple_example
VALGRIND=valgrind --error-exitcode=1

all: valgrind

.PHONY: valgrind

${BIN}:
	cargo build --examples

valgrind: ${BIN}
	${VALGRIND} --track-fds=yes --leak-check=full ${BIN}
