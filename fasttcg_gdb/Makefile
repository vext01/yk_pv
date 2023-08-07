CPPFLAGS=-I/usr/include/gdb
CFLAGS=-fPIC -g -Wextra

all: fasttcg_gdb.so

fasttcg_gdb.so: fasttcg_gdb.c
	${CC} ${CFLAGS} ${CPPFLAGS} -shared $< -o $@
