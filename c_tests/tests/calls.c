// Compiler:
// Run-time:

// Check that basic trace compilation works.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

__attribute__((noinline)) int f() {
  int b = 2;
  return b;
}

int main(int argc, char **argv) {
  int res = 0;
  __yktrace_start_tracing(HW_TRACING, 0);
  res = f();
  NOOPT_VAL(res);
  void *tr = __yktrace_stop_tracing();
  assert(res == 2);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  res = 0;
  __yktrace_compiledtrace_exec(ct);
  assert(res == 2);

  return (EXIT_SUCCESS);
}
