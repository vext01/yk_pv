// Compiler:
// Run-time:

// Check that basic tracing works.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

int main(int argc, char **argv) {
  __yktrace_start_tracing(HW_TRACING, 0);
  CLOBBER_MEM();
  void *tr = __yktrace_stop_tracing();
  assert(__yktrace_irtrace_len(tr) == 1);

  char *func_name = NULL;
  size_t bb = 0;
  __yktrace_irtrace_get(tr, 0, &func_name, &bb);
  assert(strcmp(func_name, "main") == 0);
  assert(bb == 0);

  __yktrace_drop_irtrace(tr);

  return (EXIT_SUCCESS);
}
