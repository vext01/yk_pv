// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   stderr:
//     ...
//     define internal void @__yk_compiled_trace_0(i32* %0) {
//       ...
//       %2 = load i32, i32* %0, align 4...
//       %3 = icmp eq i32 %2, 0...
//       ...
//       store i32 3, i32* %0, align 4...
//       ret void
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
  if (cond) {
    res = 2;
    cond = 3;
  } else {
    res = 4;
  }
  NOOPT_VAL(res);
  void *tr = __yktrace_stop_tracing();

  assert(cond == 3);
  assert(res == 2);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  res = 0;
  __yktrace_compiledtrace_exec(ct);
  assert(res == 3);

  return (EXIT_SUCCESS);
}
