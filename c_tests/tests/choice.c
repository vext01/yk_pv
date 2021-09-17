// Compiler:
// Run-time:

// Check that tracing a cascading "if...else if...else" works.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

__attribute__((noinline)) int f(int x) {
  if (x == 0)
    return 30;
  else if (x == 1)
    return 47;
  else
    return 52;
}

int main(int argc, char **argv) {
  int res = 0;
  __yktrace_start_tracing(HW_TRACING, 0);
  NOOPT_VAL(argc);
  res = f(argc);
  NOOPT_VAL(res);
  void *tr = __yktrace_stop_tracing();
  assert(res == 47);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  res = 0;
  __yktrace_compiledtrace_exec(ct);
  assert(res == 47);

  return (EXIT_SUCCESS);
}
