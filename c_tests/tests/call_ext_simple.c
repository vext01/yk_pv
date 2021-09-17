// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   stderr:
//     ...
//     ...call i32 @putc...
//     ...
//     declare i32 @putc...
//     ...
//   stdout:
//     12

// Check that calling an external function works.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

int main(int argc, char **argv) {
  int ch = '1';
  __yktrace_start_tracing(HW_TRACING, 0);
  NOOPT_VAL(ch);
  // Note that sometimes the compiler will make this a call to putc(3).
  putchar(ch);
  CLOBBER_MEM();
  void *tr = __yktrace_stop_tracing();

  ch = '2';
  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  __yktrace_compiledtrace_exec(ct);

  return (EXIT_SUCCESS);
}
