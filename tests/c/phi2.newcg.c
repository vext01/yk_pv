// ignore-if: test $YK_JIT_COMPILER != "yk" -o "$YKB_TRACER" = "swt"
// Run-time:
//   env-var: YKD_LOG_IR=-:aot,jit-pre-opt
//   env-var: YKD_LOG_JITSTATE=-
//   env-var: YKD_LOG_STATS=/dev/null
//   stderr:
//     jitstate: start-tracing
//     i=4, val=2
//     jitstate: stop-tracing
//     --- Begin aot ---
//     ...
//     %{{14_0}}: i32 = phi bb{{bb13}} -> 2i32, bb{{bb12}} -> 1i32
//     ...
//     --- End aot ---
//     --- Begin jit-pre-opt ---
//     ...
//     %{{15}}: i32 = 2i32
//     ...
//     --- End jit-pre-opt ---
//     i=3, val=2
//     jitstate: enter-jit-code
//     i=2, val=2
//     i=1, val=2
//     jitstate: deoptimise

// Check that PHI nodes JIT properly.

#include <stdio.h>
#include <stdlib.h>
#include <yk.h>
#include <yk_testing.h>

bool test_compiled_event(YkCStats stats) {
  return stats.traces_compiled_ok == 1;
}

int main(int argc, char **argv) {
  YkMT *mt = yk_mt_new(NULL);
  yk_mt_hot_threshold_set(mt, 0);
  YkLocation loc = yk_location_new();

  int val = 0;
  int cond = -1;
  int i = 4;
  NOOPT_VAL(loc);
  NOOPT_VAL(val);
  NOOPT_VAL(i);
  while (i > 0) {
    yk_mt_control_point(mt, &loc);
    if (i == 3) {
      __ykstats_wait_until(mt, test_compiled_event);
    }
    NOOPT_VAL(cond);
    if (cond > 0) {
      val = 1;
    } else {
      val = 2;
    }
    fprintf(stderr, "i=%d, val=%d\n", i, val);
    i--;
  }
  yk_location_drop(loc);
  yk_mt_drop(mt);
  return (EXIT_SUCCESS);
}
