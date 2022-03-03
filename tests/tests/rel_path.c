// Compiler:
// Run-time:
//   env-var: YKD_PRINT_JITSTATE=1
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   env-var: YKD_SERIALISE_COMPILATION=1
//   stderr:
//     ...
//     jit-state: enter-jit-code
//     ...

// Check that running a traced binary via a relative path works.

#include <assert.h>
#include <err.h>
#include <libgen.h>
#include <stdlib.h>
#include <unistd.h>
#include <yk.h>
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

  YkMT *mt = yk_mt_new();
  yk_mt_hot_threshold_set(mt, 0);
  YkLocation loc = yk_location_new();

  int i = 3;
  NOOPT_VAL(i);
  while (i > 0) {
    yk_mt_control_point(mt, &loc);
    i--;
  }

  assert(i == 0);
  yk_location_drop(loc);
  yk_mt_drop(mt);
  return (EXIT_SUCCESS);
}
