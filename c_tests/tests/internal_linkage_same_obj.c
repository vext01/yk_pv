// Compiler:
// Run-time:

// Check that we can call a static function with internal linkage from the same
// compilation unit.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

static int call_me(int x) {
  if (x == 5)
    return x;
  else {
    // The recursion will cause a call to be emitted in the trace.
    return call_me(x + 1);
  }
}

int main(int argc, char **argv) {
  int res = 0;
  __yktrace_start_tracing(HW_TRACING, 0);
  NOOPT_VAL(argc);
  // At higher optimisation levels LLVM realises that this call can be
  // completely removed. Hence we only structurally test a couple of lower opt
  // levels.
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
