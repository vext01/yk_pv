// Compiler:
// Run-time:

// Check that trace compilation works in the non-entry block.
//
// Since LLVM allocas typically appear in the entry block of a function, we
// will miss the allocas if tracing starts in a later block.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

int main(int argc, char **argv) {
  // Causes the traced block to NOT be the entry block.
  if (argc == -1)
    abort();

  int res;
  __yktrace_start_tracing(HW_TRACING, 0);
  // Causes both a load and a store to things defined outside the trace.
  NOOPT_VAL(argc);
  res = 1 + argc;
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
