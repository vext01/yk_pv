// Compiler:
// Run-time:

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

int global_int = 12;

__attribute__((noinline)) int foo(int num) {
  global_int = num;
  return global_int;
}

int main(int argc, char **argv) {
  int res = 0;
  __yktrace_start_tracing(HW_TRACING, 0);
  res = foo(2);
  NOOPT_VAL(res);
  void *tr = __yktrace_stop_tracing();
  assert(res == 2);
  assert(global_int == 2);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  res = 0;
  global_int = 12;
  __yktrace_compiledtrace_exec(ct);
  assert(res == 2);
  assert(global_int == 2);

  return (EXIT_SUCCESS);
}
