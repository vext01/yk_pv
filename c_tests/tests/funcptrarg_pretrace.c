// Compiler:
// Run-time:

// Test that indirect calls are only copied to the JITModule after we have seen
// `start_tracing`. Since indirect calls are handled before our regular
// are-we-tracing-yet check, and require an additional check, it makes sense to
// test for this here.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

int bar(size_t (*func)(const char *)) {
  int pre = func("abc");

  int res;
  __yktrace_start_tracing(HW_TRACING, 0);
  res = func("abc");
  NOOPT_VAL(res);
  void *tr = __yktrace_stop_tracing();
  assert(res == 3);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);
  res = 0;
  __yktrace_compiledtrace_exec(ct);
  assert(res == 3);

  assert(pre == 3);
  return res;
}

int main(int argc, char **argv) {
  int res = 0;
  res = bar(strlen);
  assert(res == 3);

  return (EXIT_SUCCESS);
}
