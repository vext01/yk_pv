// Run-time:
//   env-var: YKD_SERIALISE_COMPILATION=1
//   env-var: YKD_PRINT_JITSTATE=1

// The smallest program we can JIT and see something happen.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk.h>
#include <yk_testing.h>

int main(int argc, char **argv) {
  YkMT *mt = yk_mt_new(NULL);
  yk_mt_hot_threshold_set(mt, 0);
  YkLocation loc = yk_location_new();

  // try `int` to get infinite loop. I think the prolog is wrong when loading
  // args smaller than reg-sized.
  int i = 0, x = 0;
  NOOPT_VAL(loc);
  NOOPT_VAL(i);
  NOOPT_VAL(x);
  for (i = 0; i < 10; i++) {
    yk_mt_control_point(mt, &loc);
    //puts("it works!");
    putchar(0x41 + i);
    putchar('\n');
    fflush(stdout);
    x++;
  }
  printf("x=%d\n", x);
  yk_location_drop(loc);
  yk_mt_drop(mt);
  return (EXIT_SUCCESS);
}
