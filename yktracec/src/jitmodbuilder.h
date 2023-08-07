#ifndef __JITMODBUILDER_H
#define __JITMODBUILDER_H

#include "llvm/IR/GlobalValue.h"
#include "llvm/IR/Module.h"
#include <map>

// An unaligned virtual address.
#define YK_INVALID_ALIGNED_VADDR 0x1

using namespace llvm;

struct GenJITModResult {
  Module *JITMod;
  std::string TraceName;
  std::map<GlobalValue *, void *> GlobalMappings;
  void *LiveAOTVars;
  size_t NumGuards;
};

struct GenJITModResult createModule(Module *AOTMod, char *FuncNames[],
                                    size_t BBs[], size_t TraceLen,
                                    char *FAddrKeys[], void *FAddrVals[],
                                    size_t FAddrLen);
#ifdef YK_TESTING
struct GenJITModResult createModuleForTraceCompilerTests(
    Module *AOTMod, char *FuncNames[], size_t BBs[], size_t TraceLen,
    char *FAddrKeys[], void *FAddrVals[], size_t FAddrLen);
#endif // YK_TESTING
#endif
