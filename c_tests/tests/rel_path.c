// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   stderr:
//   stderr:
//     ...
//     define internal void @__yk_compiled_trace_0(i32* %0) {
//        ...
//        store i32 2, i32* %0, align 4...
//        ...
//        ret void
//     }
//     ...

// Check that running a traced binary via a relative path works.

#include <assert.h>
#include <err.h>
#include <libgen.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <yk_testing.h>

int main(int argc, char **argv) {
  if (*argv[0] == '/') {
    // Reinvoke ourself with a relative path.
    char *base = basename(argv[0]);
    if (base == NULL)
      err(EXIT_FAILURE, "basename");

    char *dir = dirname(argv[0]);
    if (dir == NULL)
      err(EXIT_FAILURE, "dirname");
    if (chdir(dir) != 0)
      err(EXIT_FAILURE, "chdir");

    if (execl(base, base, NULL) == -1)
      err(EXIT_FAILURE, "execl");
    // NOREACH
  }

  int res = 0;
  __yktrace_start_tracing(HW_TRACING, 0);
  res = 2;
  NOOPT_VAL(res);
  void *tr = __yktrace_stop_tracing();
  assert(res == 2);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  res = 0;
  __yktrace_compiledtrace_exec(ct);
  assert(res == 2);

  return (EXIT_SUCCESS);
}
