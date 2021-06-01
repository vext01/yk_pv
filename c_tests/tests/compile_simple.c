// Compiler:
// Run-time:

// Check that basic trace compilation works.
// FIXME An optimising compiler can remove all of the code between start/stop
// tracing.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

int main(int argc, char **argv) {
  void *tt = __yktrace_start_tracing(HW_TRACING);
  int res = 1 + 1;
  void *tr = __yktrace_stop_tracing(tt);

  assert(res == 2);

  void *ptr = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  void (*func)() = (void (*)())ptr;
  func();

  return (EXIT_SUCCESS);
}