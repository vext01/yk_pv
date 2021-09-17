// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=aot
//   stderr:
//     ...
//     define dso_local i32 @main...
//       ...
//       ...phi...
//       ...
//       call void (i64, i64, ...) @__yktrace_start_tracing(...
//       ...
//       }
//       ...

// Check that we can handle struct field accesses where the field is
// initialised via a phi node.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

struct s {
  int x;
};

int main(int argc, char **argv) {
  int z = 5;
  struct s s1;
  s1.x = argc || z; // Creates a phi node.
  int y = 0;
  __yktrace_start_tracing(HW_TRACING, 0);
  NOOPT_VAL(s1);
  y = s1.x;
  NOOPT_VAL(y);
  void *tr = __yktrace_stop_tracing();
  assert(y == 1);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  y = 0;
  __yktrace_compiledtrace_exec(ct);
  assert(y == 1);

  return (EXIT_SUCCESS);
}
