// ignore: https://github.com/ykjit/yk/issues/409
// Compiler:
// Run-time:

// Check that we can call a function without IR from another object (.o) file.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

extern int call_me(int);

int main(int argc, char **argv) {
  int res = 0;
  __yktrace_start_tracing(HW_TRACING, 0);
  NOOPT_VAL(argc);
  res = call_me(argc);
  NOOPT_VAL(res);
  void *tr = __yktrace_stop_tracing();
  assert(res == 5);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  res = 0;
  __yktrace_compiledtrace_exec(ct);
  assert(res == 5);

  return (EXIT_SUCCESS);
}
