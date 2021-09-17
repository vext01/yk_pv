// Compiler:
// Run-time:

// Check that inlining a function with a void return type works.
//
// FIXME An optimising compiler can remove all of the code between start/stop
// tracing.

#include <assert.h>
#include <stdlib.h>
#include <yk_testing.h>

void __attribute__((noinline)) f() { return; }

int main(int argc, char **argv) {
  __yktrace_start_tracing(HW_TRACING, 0);
  f();
  void *tr = __yktrace_stop_tracing();

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  __yktrace_compiledtrace_exec(ct);

  return (EXIT_SUCCESS);
}
