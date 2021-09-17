// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   stderr:
//     ...
//     define internal void @__yk_compiled_trace_0(...
//       ...
//       store i32 333, i32* %...
//       ...
//       ret void
//     }
//     ...

// Check that tracing function calls in sequence works.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

__attribute__((noinline)) int f(a) { return a; }

int main(int argc, char **argv) {
  int res = 0;
  __yktrace_start_tracing(HW_TRACING, 0);
  int a = f(111);
  int b = f(222);
  res = a + b;
  NOOPT_VAL(res);
  void *tr = __yktrace_stop_tracing();
  assert(res == 333);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  res = 0;
  a = 0;
  b = 0;
  __yktrace_compiledtrace_exec(ct);
  printf("%d\n", res);
  assert(res == 333);

  return (EXIT_SUCCESS);
}
