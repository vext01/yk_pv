// Compiler:
// Run-time:
//   env-var: YKD_PRINT_IR=jit-pre-opt
//   stderr:
//     ...
//     define internal %OutputStruct @__yk_compiled_trace_0(%OutputStruct %0) {
//        ...
//     }
//     ...
#include <assert.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <yk.h>
#include <yk_testing.h>

// The sole mutable memory cell of the interpreter.
int mem = 3;

// The bytecodes accepted by the interpreter.
#define NOP 0
#define DEC 1
#define RESTART_IF_NOT_ZERO 2
#define EXIT 3

int
main(int argc, char **argv)
{
  // A hard-coded program to execute.
  int prog[] = {0, 0, 1, 2, 0, 3};
  // The program counter (FIXME: also serving as a location ID for now).
  int pc = 0;

  // interpreter loop.
  while (true) {
    control_point(pc);

    int bc = prog[pc];
    printf("%d\n", pc);
    switch (bc) {
      case NOP:
        pc++;
        break;
      case DEC:
        mem--;
        pc++;
        break;
      case RESTART_IF_NOT_ZERO:
        if (mem > 0)
          pc = 0;
        else
          pc++;
        break;
      case EXIT:
        goto done;
      default:
        abort(); // unreachable.
    }
  }

done:
  assert(mem == 0);
  assert(pc == 5);
  assert(0);
  return (EXIT_SUCCESS);
}
