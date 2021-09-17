// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   stderr:
//     ...
//     define internal void @__yk_compiled_trace_0(...
//       ...
//       ...add nsw i32 3, 2...
//       ...
//       ret void
//     }
//     ...

// Check that basic trace compilation works.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

__attribute__((noinline)) int f(int a, int b) {
  int c = a + b;
  return c;
}

int main(int argc, char **argv) {
  int res = 0;
  __yktrace_start_tracing(HW_TRACING, 0);
  res = f(2, 3);
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
