// ignore-if: test $YK_JIT_COMPILER != "yk" -o "$YKB_TRACER" = "swt"
// Run-time:
//   env-var: YKD_LOG_IR=-:aot,jit-pre-opt
//   env-var: YKD_SERIALISE_COMPILATION=1
//   env-var: YKD_LOG_JITSTATE=-
//   stderr:
//     jitstate: start-tracing
//     4
//     foo-if
//     bar
//     jitstate: stop-tracing
//     --- Begin aot ---
//     ...
//     func main(%arg0: i32, %arg1: ptr) -> i32 {
//     ...
//     --- End aot ---
//     --- Begin jit-pre-opt ---
//     ...
//     %{{1}}: i8 = sgt %{{2}}, 1i32
//     ...
//     %{{3}}: i64 = call @fwrite(%{{4}}, 4i64, 1i64, %{{5}})
//     ...
//     --- End jit-pre-opt ---
//     3
//     foo-if
//     bar
//     jitstate: enter-jit-code
//     2
//     foo-if
//     bar
//     1
//     jitstate: deoptimise
//     foo-else
//     bar
//     0
//     exit

// Test deoptimisation with multiple nested calls.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk.h>
#include <yk_testing.h>

void foo(int i) {
  if (i > 1) {
    fputs("foo-if\n", stderr);
  } else {
    fputs("foo-else\n", stderr);
  }
}

void bar(int i) {
  foo(i);
  fputs("bar\n", stderr);
}

int main(int argc, char **argv) {
  YkMT *mt = yk_mt_new(NULL);
  yk_mt_hot_threshold_set(mt, 0);
  YkLocation loc = yk_location_new();

  int res = 9998;
  int i = 4;
  NOOPT_VAL(loc);
  NOOPT_VAL(res);
  NOOPT_VAL(i);
  while (i > 0) {
    yk_mt_control_point(mt, &loc);
    fprintf(stderr, "%d\n", i);
    bar(i);
    res += 2;
    i--;
  }
  fprintf(stderr, "%d\n", i);
  fprintf(stderr, "exit\n");
  NOOPT_VAL(res);
  yk_location_drop(loc);
  yk_mt_drop(mt);
  return (EXIT_SUCCESS);
}
