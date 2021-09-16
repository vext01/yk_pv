// ignore: https://github.com/ykjit/yk/issues/409
// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   stderr:
//     ...
//     define internal void @__yk_compiled_trace_0(i32* %0, i32* %1) {
//       %3 = load i32, i32* %1, align 4, !tbaa !0
//       %4 = shl nsw i32 %3, 3
//       %5 = icmp ult i32 %4, 3
//       %6 = add nsw i32 %4, -2
//       %7 = tail call i32 @fib(i32 %6, i32* %1)...
//       %8 = add nsw i32 %4, -1
//       %9 = tail call i32 @fib(i32 %8, i32* %1)...
//       %10 = add nsw i32 %9, %7
//       store i32 %10, i32* %1, align 4, !tbaa !0
//       store i32 %10, i32* %0, align 4, !tbaa !0
//       ret void
//     }
//     ...

// Check that recursive function calls are not unrolled.

#include <assert.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk_testing.h>

__attribute__((noinline)) int fib(int num, int *tcp) {
  if (num == 0)
    return 0;
  if (num == 1)
    return 1;
  if (num == 2)
    return 1;
  int a = fib(num - 2, tcp);
  int b = fib(num - 1, tcp);
  int c = a + b;
  *tcp = c; // Prevent tail call optimisation.
  return c;
}

int main(int argc, char **argv) {
  int res = 0;
  printf("XXX: %p %p\n", &argc, &res);
  __yktrace_start_tracing(HW_TRACING, 0);
  res = fib(argc * 8, &argc);
  void *tr = __yktrace_stop_tracing();
  assert(res == 21);

  void *ct = __yktrace_irtrace_compile(tr);
  __yktrace_drop_irtrace(tr);

  res = 0;
  argc = 1;
  __yktrace_compiledtrace_exec(ct);
  assert(res == 21);

  return (EXIT_SUCCESS);
}
