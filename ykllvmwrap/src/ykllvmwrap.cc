// LLVM-related C++ code wrapped in the C ABI for calling from Rust.

#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif

#include "llvm/ExecutionEngine/Orc/ThreadSafeModule.h"
#include "llvm/Transforms/IPO/PassManagerBuilder.h"
#include <llvm/DebugInfo/Symbolize/Symbolize.h>
#include <llvm/ExecutionEngine/ExecutionEngine.h>
#include <llvm/ExecutionEngine/MCJIT.h>
#include <llvm/IR/IRBuilder.h>
#include <llvm/IR/LLVMContext.h>
#include <llvm/IR/LegacyPassManager.h>
#include <llvm/IR/Module.h>
#include <llvm/IR/Verifier.h>
#include <llvm/IRReader/IRReader.h>
#include <llvm/Support/SourceMgr.h>
#include <llvm/Support/TargetSelect.h>
#include <llvm/Transforms/Utils/ValueMapper.h>

#include <atomic>
#include <dlfcn.h>
#include <err.h>
#include <limits>
#include <link.h>
#include <mutex>
#include <stdlib.h>
#include <string.h>

#include "jitmodbuilder.cc"
#include "memman.cc"

using namespace llvm;
using namespace llvm::orc;
using namespace llvm::symbolize;
using namespace std;

extern "C" void __ykutil_get_llvmbc_section(void **res_addr, size_t *res_size);

// The bitcode module loaded from the .llvmbc section of the currently-running
// binary. This cannot be shared across threads and used concurrently without
// acquiring a lock, and since we do want to allow parallel compilation, each
// thread takes a copy of this module.
ThreadSafeModule GlobalAOTMod;

// Flag used to ensure that GlobalAOTMod is loaded only once.
once_flag GlobalAOTModLoaded;

// A copy of GlobalAOTMod for use by a single thread.
//
// A thread should never access this directly, but should instead go via
// getThreadAOTMod() which deals with the necessary lazy initialisation.
//
// PERF: Copying GlobalAOTMod is quite expensive (cloneToNewContext()
// serialises and deserializes). When a compilation thread dies, we should
// return its ThreadAOTMod to a pool and transfer ownership to the next thread
// that needs its own copy of GlobalAOTMod.
thread_local ThreadSafeModule ThreadAOTMod;

// A flag indicating whether GlobalAOTMod has been copied into the thread yet.
thread_local bool ThreadAOTModInitialized = false;

// Flag used to ensure that LLVM is initialised only once.
once_flag LLVMInitialised;

#ifndef NDEBUG
// Left trim (in-place) the character `C` from the string `S`.
void lTrim(string &S, const char C) {
  S.erase(0, std::min(S.find_first_not_of(C), S.size() - 1));
}

// Dumps an LLVM Value to a string and trims leading whitespace.
void dumpValueToString(Value *V, string &S) {
  raw_string_ostream RSO(S);
  V->print(RSO);
  lTrim(S, ' ');
}
#endif

enum DebugIR {
  AOT,
  JITPreOpt,
  JITPreOptSBS,
  JITPostOpt,
};

class DebugIRPrinter {
private:
  bitset<4> toPrint;

  const char *debugIRStr(DebugIR IR) {
    switch (IR) {
    case DebugIR::AOT:
      return "aot";
    case DebugIR::JITPreOpt:
      return "jit-pre-opt";
    case DebugIR::JITPreOptSBS:
      return "jit-pre-opt-sbs";
    case DebugIR::JITPostOpt:
      return "jit-post-opt";
    default:
      errx(EXIT_FAILURE, "unreachable");
    }
  }

public:
  DebugIRPrinter() {
    char *Env = std::getenv("YKD_PRINT_IR");
    char *Val;
    while ((Val = strsep(&Env, ",")) != nullptr) {
      if (strcmp(Val, "aot") == 0)
        toPrint.set(DebugIR::AOT);
      else if (strcmp(Val, "jit-pre-opt") == 0)
        toPrint.set(DebugIR::JITPreOpt);
#ifndef NDEBUG
      else if (strcmp(Val, "jit-pre-opt-sbs") == 0)
        toPrint.set(DebugIR::JITPreOptSBS);
#endif
      else if (strcmp(Val, "jit-post-opt") == 0)
        toPrint.set(DebugIR::JITPostOpt);
      else
        errx(EXIT_FAILURE, "invalid parameter for YKD_PRINT_IR: '%s'", Val);
    }
  }

#ifndef NDEBUG
  // Print a trace's instructions "side-by-side" with the instructions from
  // which they were derived in the AOT module.
  void printSBS(Module *AOTMod, Module *JITMod, ValueToValueMapTy &RevVMap) {
    assert(JITMod->size() == 1);
    Function *JITFunc = &*JITMod->begin();

    // Find the longest instruction from the JITMod so that we can align the
    // second column.
    size_t LongestJITLine = 0;
    for (auto &JITBlock : *JITFunc) {
      for (auto &JITInst : JITBlock) {
        string Line;
        dumpValueToString(&JITInst, Line);
        auto Len = Line.length();
        if (Len > LongestJITLine)
          LongestJITLine = Len;
      }
    }

    const string JITHeader = string("Trace");
    string Padding = string(LongestJITLine - JITHeader.length(), ' ');
    errs() << "\n\n--- Begin trace dump for " << JITFunc->getName() << " ---\n";
    errs() << JITHeader << Padding << "  | AOT\n";

    // Keep track of the AOT function we are currently in so that we can print
    // inlined function thresholds in the dumped trace.
    StringRef LastAOTFunc;
    const DebugLoc *LastDebugLoc = nullptr;
    for (auto &JITBlock : *JITFunc) {
      for (auto &JITInst : JITBlock) {
        auto V = RevVMap[&JITInst];
        if (V == nullptr) {
          // The instruction wasn't cloned from the AOTMod, so print it only in
          // the JIT column and carry on.
          std::string Line;
          dumpValueToString((Value *)&JITInst, Line);
          errs() << Line << "\n";
          continue;
        }
        Instruction *AOTInst = (Instruction *)&*V;
        assert(AOTInst != nullptr);
        Function *AOTFunc = AOTInst->getFunction();
        assert(AOTFunc != nullptr);
        StringRef AOTFuncName = AOTFunc->getName();
        if (AOTFuncName != LastAOTFunc) {
          // Print an inlining threshold.
          errs() << "# " << AOTFuncName << "()\n";
          LastAOTFunc = AOTFuncName;
        }
        const DebugLoc &ThisDebugLoc = JITInst.getDebugLoc();
        if (LastDebugLoc != &ThisDebugLoc) {
          string LocStr;
          raw_string_ostream RSO(LocStr);
          ThisDebugLoc.print(RSO);
          errs() << "# " << LocStr << "\n";
          LastAOTFunc = AOTFuncName;
        }
        string JITStr;
        dumpValueToString((Value *)&JITInst, JITStr);
        string Padding = string(LongestJITLine - JITStr.length(), ' ');
        string AOTStr;
        dumpValueToString((Value *)AOTInst, AOTStr);
        errs() << JITStr << Padding << "  |  " << AOTStr << "\n";
      }
    }
    errs() << "--- End trace dump for " << JITFunc->getName() << " ---\n";
  }
#endif

  void print(enum DebugIR IR, Module *M, Module *SBSAOTMod = nullptr,
             ValueToValueMapTy *SBSRevVMap = nullptr) {
    if (toPrint[IR]) {
      if (IR == DebugIR::JITPreOptSBS) {
#ifndef NDEBUG
        printSBS(SBSAOTMod, M, *SBSRevVMap);
#endif
      } else {
        // We print begin/end markers so that we can more test the IR at
        // specific stages in the JIT pipeline (by anchoring matches to the
        // begin/end markers).
        errs() << "--- Begin " << DebugIRPrinter::debugIRStr(IR) << " ---\n";
        M->dump();
        errs() << "--- End " << DebugIRPrinter::debugIRStr(IR) << " ---\n";
      }
    }
  }
};

// Initialise LLVM for JIT compilation. This must be executed exactly once.
void initLLVM(void *Unused) {
  InitializeNativeTarget();
  InitializeNativeTargetAsmPrinter();
  InitializeNativeTargetAsmParser();
}

extern "C" LLVMSymbolizer *__yk_llvmwrap_symbolizer_new() {
  return new LLVMSymbolizer;
}

extern "C" void __yk_llvmwrap_symbolizer_free(LLVMSymbolizer *Symbolizer) {
  delete Symbolizer;
}

// Finds the name of a code symbol from a virtual address.
// The caller is responsible for freeing the returned (heap-allocated) C string.
extern "C" char *
__yk_llvmwrap_symbolizer_find_code_sym(LLVMSymbolizer *Symbolizer,
                                       const char *Obj, uint64_t Off) {
  object::SectionedAddress Mod{Off, object::SectionedAddress::UndefSection};
  auto LineInfo = Symbolizer->symbolizeCode(Obj, Mod);
  if (auto Err = LineInfo.takeError()) {
    return NULL;
  }

  // PERF: get rid of heap allocation.
  return strdup(LineInfo->FunctionName.c_str());
}

// Load the GlobalAOTMod.
//
// This must only be called from getAOTMod() for correct synchronisation.
void loadAOTMod(void *Unused) {
  void *SecPtr;
  size_t SecSize;
  __ykutil_get_llvmbc_section(&SecPtr, &SecSize);
  auto Sf = StringRef((const char *)SecPtr, SecSize);
  auto Mb = MemoryBufferRef(Sf, "");
  SMDiagnostic Error;
  ThreadSafeContext AOTCtx = std::make_unique<LLVMContext>();
  auto M = parseIR(Mb, Error, *AOTCtx.getContext());
  if (!M)
    errx(EXIT_FAILURE, "Can't load module.");
  GlobalAOTMod = ThreadSafeModule(std::move(M), std::move(AOTCtx));
}

// Get a thread-safe handle on the LLVM module stored in the .llvmbc section of
// the binary. The module is loaded if we haven't yet done so.
ThreadSafeModule *getThreadAOTMod(void) {
  std::call_once(GlobalAOTModLoaded, loadAOTMod, nullptr);
  if (!ThreadAOTModInitialized) {
    ThreadAOTMod = cloneToNewContext(GlobalAOTMod);
    ThreadAOTModInitialized = true;
  }
  return &ThreadAOTMod;
}

// Compile a module in-memory and return a pointer to its function.
extern "C" void *compileModule(string TraceName, Module *M,
                               map<StringRef, uint64_t> GlobalMappings) {
  std::call_once(LLVMInitialised, initLLVM, nullptr);

  // FIXME Remember memman or allocated memory pointers so we can free the
  // latter when we're done with the trace.
  auto memman = new MemMan();

  auto MPtr = std::unique_ptr<Module>(M);
  string ErrStr;
  ExecutionEngine *EE =
      EngineBuilder(std::move(MPtr))
          .setEngineKind(EngineKind::JIT)
          .setMemoryManager(std::unique_ptr<MCJITMemoryManager>(memman))
          .setErrorStr(&ErrStr)
          .create();

  if (EE == nullptr)
    errx(EXIT_FAILURE, "Couldn't compile trace: %s", ErrStr.c_str());

  for (auto GM : GlobalMappings) {
    EE->addGlobalMapping(GM.first, GM.second);
  }

  EE->finalizeObject();
  if (EE->hasError())
    errx(EXIT_FAILURE, "Couldn't compile trace: %s",
         EE->getErrorMessage().c_str());

  return (void *)EE->getFunctionAddress(TraceName);
}

// Compile an IRTrace to executable code in memory.
//
// The trace to compile is passed in as two arrays of length Len. Then each
// (FuncName[I], BBs[I]) pair identifies the LLVM block at position `I` in the
// trace.
//
// Returns a pointer to the compiled function.
extern "C" void *__ykllvmwrap_irtrace_compile(char *FuncNames[], size_t BBs[],
                                              size_t Len, char *FAddrKeys[],
                                              size_t FAddrVals[],
                                              size_t FAddrLen) {
  DebugIRPrinter DIP;

  ThreadSafeModule *ThreadAOTMod = getThreadAOTMod();
  // Getting the module without acquiring the context lock is safe in this
  // instance since ThreadAOTMod is not shared between threads.
  Module *AOTMod = ThreadAOTMod->getModuleUnlocked();

  DIP.print(DebugIR::AOT, AOTMod);

  JITModBuilder JB;
  auto JITMod = JB.createModule(FuncNames, BBs, Len, AOTMod, FAddrKeys,
                                FAddrVals, FAddrLen);

  DIP.print(DebugIR::JITPreOpt, JITMod);
#ifndef NDEBUG
  DIP.print(DebugIR::JITPreOptSBS, JITMod, AOTMod, &JB.RevVMap);
#endif

  // The MCJIT code-gen does no optimisations itself, so we must do it
  // ourselves.
  PassManagerBuilder Builder;
  Builder.OptLevel = 2; // FIXME Make this user-tweakable.
  legacy::FunctionPassManager FPM(JITMod);
  Builder.populateFunctionPassManager(FPM);
  for (Function &F : *JITMod)
    FPM.run(F);

  DIP.print(DebugIR::JITPostOpt, JITMod);

  // Compile IR trace and return a pointer to its function.
  return compileModule(JB.TraceName, JITMod, JB.globalMappings);
}
