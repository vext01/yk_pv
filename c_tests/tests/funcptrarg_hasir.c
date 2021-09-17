// Compiler:
// Run-time:

// Test indirect calls where we have IR for the callee.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

__attribute__((noinline)) int foo(int a) { return a + 1; }

int bar(int (*func)(int)) {
  int a = func(3);
  return a;
}

int main(int argc, char **argv) {
  int z = 0;
  __yktrace_start_tracing(HW_TRACING, 0);
  z = bar(foo);
  NOOPT_VAL(z);
  void *tr = __yktrace_stop_tracing();
  assert(z == 4);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  z = 0;
  __yktrace_compiledtrace_exec(ct);
  assert(z == 4);

  return (EXIT_SUCCESS);
}
