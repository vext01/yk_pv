// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   env-var: YKD_SERIALISE_COMPILATION=1
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
#include <yk.h>
#include <yk_testing.h>

int main(int argc, char **argv) {
  YkMT *mt = yk_mt_new();
  yk_hot_threshold_set(mt, 0);
  YkLocation loc = yk_location_new();

  int ch = '1';
  NOOPT_VAL(ch);
  while (ch != '3') {
    yk_control_point(mt, &loc);
    // Note that sometimes the compiler will make this a call to putc(3).
    putchar(ch);
    ch++;
  }

  yk_location_drop(loc);
  yk_mt_drop(mt);
  return (EXIT_SUCCESS);
}
