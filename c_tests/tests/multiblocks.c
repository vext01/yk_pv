// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   stderr:
//     ...
//     define internal void @__yk_compiled_trace_0(...
//       ...
//       ...= icmp...
//       ...
//       store i32 3, i32* %...
//       ...
//     }
//     ...

// Check that basic trace compilation works.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

int main(int argc, char **argv) {
  int cond = argc;
  __yktrace_start_tracing(HW_TRACING, 0);
  NOOPT_VAL(cond);
  int res = 0;
  if (cond == 1) {
    res = 2;
    cond = 3;
  } else {
    res = 4;
  }
  NOOPT_VAL(res);
  NOOPT_VAL(cond);
  void *tr = __yktrace_stop_tracing();

  assert(cond == 3);
  assert(res == 2);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  res = 0;
  cond = argc;
  __yktrace_compiledtrace_exec(ct);
  printf("XX: %d %d\n", cond, res);
  assert(cond == 3);
  assert(res == 2);

  return (EXIT_SUCCESS);
}
