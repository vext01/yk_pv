// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   stderr:
//     ...
//     define internal %OutputStruct @__yk_compiled_trace_0(...
//        ...
//        %%9 = add nsw i32 %%8, 2...
//        ...
//     }
//     ...

// Check that basic trace compilation works.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>
#include <yk.h>

void _yk_test(int i, int res) {
  if (i == 0)
    assert(res == 2);
}

int main(int argc, char **argv) {
  int res = 0;
  int loc = 0;
  int i = 3;
  NOOPT_VAL(res);
  NOOPT_VAL(i);
  while (i>0) {
    control_point(loc);
    res += 2;
    i--;
  }
  NOOPT_VAL(res);
  assert(res == 8);

  return (EXIT_SUCCESS);
}
