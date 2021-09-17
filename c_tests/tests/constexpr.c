// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   stderr:
//     ...
//     @.str = ...
//     ...
//     define internal void @__yk_compiled_trace_0(i32* %0) {
//       ...
//       ...getelementptr inbounds ([4 x i8], [4 x i8]* @.str...
//       ...

// Check that global variables inside constant expressions are copied and
// remapped.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

const volatile int global_int = 6;

__attribute__((noinline)) char foo(char *str) { return str[0]; }

int main(int argc, char **argv) {
  int res = 0;
  __yktrace_start_tracing(HW_TRACING, 0);
  res = foo("abc");
  NOOPT_VAL(res);
  void *tr = __yktrace_stop_tracing();
  assert(res == 97);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  res = 0;
  __yktrace_compiledtrace_exec(ct);
  assert(res == 97);

  return (EXIT_SUCCESS);
}
