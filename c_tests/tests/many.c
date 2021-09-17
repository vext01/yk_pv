// Compiler:
// Run-time:

// Check that compiling and running multiple traces in sequence works.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

void trace(void) {
  __yktrace_start_tracing(HW_TRACING, 0);
  CLOBBER_MEM();
  int res = 1 + 1;
  CLOBBER_MEM();
  void *tr = __yktrace_stop_tracing();
  assert(res == 2);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  __yktrace_compiledtrace_exec(ct);
}

int main(int argc, char **argv) {
  for (int i = 0; i < 3; i++)
    trace();

  return (EXIT_SUCCESS);
}
