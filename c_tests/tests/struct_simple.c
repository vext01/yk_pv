// ignore: https://github.com/ykjit/yk/issues/409
// Compiler:
// Run-time:

// Check that we can handle struct field accesses.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

struct s {
  int x;
};

int main(int argc, char **argv) {
  struct s s1 = {argc};
  int y = 0;
  __yktrace_start_tracing(HW_TRACING, 0);
  NOOPT_VAL(s1);
  y = s1.x;
  NOOPT_VAL(y);
  void *tr = __yktrace_stop_tracing();
  assert(y == 1);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  y = 0;
  __yktrace_compiledtrace_exec(ct);
  assert(y == 1);

  return (EXIT_SUCCESS);
}
