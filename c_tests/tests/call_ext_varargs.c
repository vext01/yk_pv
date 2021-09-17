// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   stderr:
//    ...
//    ...call i32 (i8*, ...) @printf...
//    ...
//    declare i32 @printf(...
//    ...
//   stdout:
//     abc123
//     abc101112

// Check that calling an external function works.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

int main(int argc, char **argv) {
  int x = 1;
  __yktrace_start_tracing(HW_TRACING, 0);
  NOOPT_VAL(x);
  printf("abc%d%d%d\n", x, x + 1, x + 2);
  CLOBBER_MEM();
  void *tr = __yktrace_stop_tracing();

  x = 10;
  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  __yktrace_compiledtrace_exec(ct);

  return (EXIT_SUCCESS);
}
