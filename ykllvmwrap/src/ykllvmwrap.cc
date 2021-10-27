// LLVM-related C++ code wrapped in the C ABI for calling from Rust.

#ifndef _GNU_SOURCE
#define _GNU_SOURCE
#endif

#include "llvm/ExecutionEngine/Orc/ThreadSafeModule.h"
#include "llvm/IR/AssemblyAnnotationWriter.h"
#include "llvm/Support/FormattedStream.h"
#include "llvm/Transforms/IPO/PassManagerBuilder.h"
#include <llvm/DebugInfo/Symbolize/Symbolize.h>
#include <llvm/ExecutionEngine/ExecutionEngine.h>
#include <llvm/ExecutionEngine/MCJIT.h>
#include <llvm/ExecutionEngine/RTDyldMemoryManager.h>
#include <llvm/IR/DebugInfo.h>
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
#include <sys/mman.h>

#define TRACE_FUNC_PREFIX "__yk_compiled_trace_"
#define YKTRACE_STOP "__yktrace_stop_tracing"
#define YK_NEW_CONTROL_POINT "yk_new_control_point"
#define YK_CONTROL_POINT_ARG_IDX 1

using namespace llvm;
using namespace llvm::orc;
using namespace llvm::symbolize;
using namespace std;

struct AllocMem {
  uint8_t *Ptr;
  uintptr_t Size;
};

class MemMan : public RTDyldMemoryManager {
public:
  MemMan();
  ~MemMan() override;

  uint8_t *allocateCodeSection(uintptr_t Size, unsigned Alignment,
                               unsigned SectionID,
                               StringRef SectionName) override;

  uint8_t *allocateDataSection(uintptr_t Size, unsigned Alignment,
                               unsigned SectionID, StringRef SectionName,
                               bool isReadOnly) override;

  bool finalizeMemory(std::string *ErrMsg) override;
  void freeMemory();

private:
  std::vector<AllocMem> code;
  std::vector<AllocMem> data;
};

MemMan::MemMan() {}
MemMan::~MemMan() {}

uint8_t *alloc_mem(uintptr_t Size, unsigned Alignment,
                   std::vector<AllocMem> *Vec) {
  uintptr_t RequiredSize = Alignment * ((Size + Alignment - 1) / Alignment + 1);
  auto Ptr = (unsigned char *)mmap(0, RequiredSize, PROT_WRITE,
                                   MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
  assert(Ptr != MAP_FAILED);
  Vec->push_back({Ptr, RequiredSize});
  return Ptr;
}

uint8_t *MemMan::allocateCodeSection(uintptr_t Size, unsigned Alignment,
                                     unsigned SectionID,
                                     StringRef SectionName) {
  return alloc_mem(Size, Alignment, &code);
}

uint8_t *MemMan::allocateDataSection(uintptr_t Size, unsigned Alignment,
                                     unsigned SectionID, StringRef SectionName,
                                     bool isReadOnly) {
  return alloc_mem(Size, Alignment, &data);
}

bool MemMan::finalizeMemory(std::string *ErrMsg) {
  for (const AllocMem &Value : code) {
    if (mprotect(Value.Ptr, Value.Size, PROT_READ | PROT_EXEC) == -1) {
      errx(EXIT_FAILURE, "Can't make allocated memory executable.");
    }
  }
  return true;
}

void MemMan::freeMemory() {
  for (const AllocMem &Value : code) {
    if (munmap(Value.Ptr, Value.Size) == -1) {
      errx(EXIT_FAILURE, "Failed to unmap memory.");
    }
  }
  for (const AllocMem &Value : data) {
    if (munmap(Value.Ptr, Value.Size) == -1) {
      errx(EXIT_FAILURE, "Failed to unmap memory.");
    }
  }
}

// An atomic counter used to issue compiled traces with unique names.
atomic<uint64_t> NextTraceIdx(0);
uint64_t getNewTraceIdx() {
  uint64_t TraceIdx = NextTraceIdx.fetch_add(1, memory_order_relaxed);
  assert(TraceIdx != numeric_limits<uint64_t>::max());
  return TraceIdx;
}

// Dump an error message and an LLVM value to stderr and exit with failure.
void dumpValueAndExit(const char *Msg, Value *V) {
  errs() << Msg << ": ";
  V->dump();
  exit(EXIT_FAILURE);
}

// A function name and basic block index pair that identifies a block in the
// AOT LLVM IR.
struct IRBlock {
  // A non-null pointer to the function name.
  char *FuncName;
  // The index of the block in the parent LLVM function.
  size_t BBIdx;
};

// Describes the software or hardware trace to be compiled using LLVM.
class InputTrace {
private:
  // An ordered array of function names. Each non-null element describes the
  // function part of a (function, block) pair that identifies an LLVM
  // BasicBlock. A null element represents unmappable code in the trace.
  char **FuncNames;
  // An ordered array of basic block indices. Each element corresponds with
  // an element (at the same index) in the above `FuncNames` array to make a
  // (function, block) pair that identifies an LLVM BasicBlock.
  size_t *BBs;
  // The length of the `FuncNames` and `BBs` arrays.
  size_t Len;

public:
  InputTrace(char **FuncNames, size_t *BBs, size_t Len)
      : FuncNames(FuncNames), BBs(BBs), Len(Len) {}
  size_t Length() { return Len; }

  // Returns the optional IRBlock at index `Idx` in the trace. No value is
  // returned if element at `Idx` was unmappable. It is undefined behaviour to
  // invoke this method with an out-of-bounds `Idx`.
  const Optional<IRBlock> operator[](size_t Idx) {
    assert(Idx < Len);
    char *FuncName = FuncNames[Idx];
    if (FuncName == nullptr) {
      return Optional<IRBlock>();
    } else {
      return Optional<IRBlock>(IRBlock{FuncName, BBs[Idx]});
    }
  }

  // The same as `operator[]`, but for scenarios where you are certain that the
  // element at position `Idx` cannot be unmappable.
  const IRBlock getUnchecked(size_t Idx) {
    assert(Idx < Len);
    char *FuncName = FuncNames[Idx];
    assert(FuncName != nullptr);
    return IRBlock{FuncName, BBs[Idx]};
  }
};

// Function virtual addresses observed in the input trace.
// Maps a function symbol name to a virtual address.
class FuncAddrs {
  map<string, void *> Map;

public:
  FuncAddrs(char **FuncNames, void **VAddrs, size_t Len) {
    for (size_t I = 0; I < Len; I++) {
      Map.insert({FuncNames[I], VAddrs[I]});
    }
  }

  // Lookup the address of the specified function name or return nullptr on
  // failure.
  void *operator[](const char *FuncName) {
    auto It = Map.find(FuncName);
    if (It == Map.end())
      return nullptr; // Not found.
    return It->second;
  }
};

/// Get the `Value` of the `YkCtrlPointVars` struct by looking it up inside the
/// arguments of the new control point.
Value *getYkCtrlPointVarsStruct(Module *AOTMod, InputTrace &InpTrace) {
  Function *F = AOTMod->getFunction(YK_NEW_CONTROL_POINT);
  assert(F != nullptr);
  User *CallSite = F->user_back();
  CallInst *CI = cast<CallInst>(CallSite);
  return CI->getArgOperand(YK_CONTROL_POINT_ARG_IDX);
}

class JITModBuilder {
  // Global variables/functions that were copied over and need to be
  // initialised.
  vector<GlobalVariable *> cloned_globals;
  // The module being traced.
  Module *AOTMod;
  // The new module that is being build.
  Module *JITMod;
  // A pointer to the call to YK_NEW_CONTROL_POINT in the AOT module (once
  // encountered). When this changes from NULL to non-NULL, then we start
  // copying instructions from the AOT module into the JIT module.
  Instruction *NewControlPointCall = nullptr;
  // Stack of inlined calls, required to resume at the correct place in the
  // caller.
  std::vector<tuple<size_t, CallInst *>> InlinedCalls;
  // Instruction at which to continue after an a call.
  Optional<tuple<size_t, CallInst *>> ResumeAfter;
  // Depth of nested calls when outlining a recursive function.
  size_t RecCallDepth = 0;
  // Signifies a hole (for which we have no IR) in the trace.
  bool ExpectUnmappable = false;
  // The JITMod's builder.
  llvm::IRBuilder<> Builder;
  // Dead values to recursively delete upon finalisation of the JITMod. This is
  // required because it's not safe to recursively delete values in the middle
  // of creating the JIT module. We don't know if any of those values might be
  // required later in the trace.
  vector<Value *> DeleteDeadOnFinalise;

  // Information about the trace we are compiling.
  InputTrace InpTrace;
  // Function virtual addresses discovered from the input trace.
  FuncAddrs FAddrs;

  // A stack of BasicBlocks. Each time we enter a new call frame, we push the
  // first basic block to the stack. Following a branch to another basic block
  // updates the most recently pushed block. This is required for selecting the
  // correct incoming value when tracing a PHI node.
  vector<BasicBlock *> LastCompletedBlocks;

  // Since a trace starts tracing after the control point but ends before it,
  // we need to map the values inserted into the `YkCtrlPointVars` (appearing
  // before the control point) to the extracted values (appearing after the
  // control point). This map helps to match inserted values to their
  // corresponding extracted values using their index in the struct.
  std::map<uint64_t, Value *> InsertValueMap;

  Value *getMappedValue(Value *V) {
    if (VMap.find(V) != VMap.end()) {
      return VMap[V];
    }
    assert(isa<Constant>(V));
    return V;
  }

  // Returns true if the given function exists on the call stack, which means
  // this is a recursive call.
  bool isRecursiveCall(Function *F) {
    for (auto Tup : InlinedCalls) {
      CallInst *CInst = get<1>(Tup);
      if (CInst->getCalledFunction() == F) {
        return true;
      }
    }
    return false;
  }

  // Add an external declaration for the given function to JITMod.
  void declareFunction(Function *F) {
    assert(JITMod->getFunction(F->getName()) == nullptr);
    auto DeclFunc = llvm::Function::Create(F->getFunctionType(),
                                           GlobalValue::ExternalLinkage,
                                           F->getName(), JITMod);
    VMap[F] = DeclFunc;
  }

  // Find the machine code corresponding to the given AOT IR function and
  // ensure there's a mapping from its name to that machine code.
  void addGlobalMappingForFunction(Function *CF) {
    StringRef CFName = CF->getName();
    void *FAddr = FAddrs[CFName.data()];
    assert(FAddr != nullptr);
    globalMappings.insert({CF, FAddr});
  }

  void handleCallInst(CallInst *CI, Function *CF, size_t &CurInstrIdx) {
    if (CF == nullptr || CF->isDeclaration()) {
      // The definition of the callee is external to AOTMod. We still
      // need to declare it locally if we have not done so yet.
      if (CF != nullptr && VMap.find(CF) == VMap.end()) {
        declareFunction(CF);
      }
      if (RecCallDepth == 0) {
        copyInstruction(&Builder, (Instruction *)&*CI);
      }
      // We should expect an "unmappable hole" in the trace. This is
      // where the trace followed a call into external code for which we
      // have no IR, and thus we cannot map blocks for.
      ExpectUnmappable = true;
      ResumeAfter = make_tuple(CurInstrIdx, CI);
    } else {
      LastCompletedBlocks.push_back(nullptr);
      if (RecCallDepth > 0) {
        // When outlining a recursive function, we need to count all other
        // function calls so we know when we left the recusion.
        RecCallDepth += 1;
        InlinedCalls.push_back(make_tuple(CurInstrIdx, CI));
        return;
      }
      // If this is a recursive call that has been inlined, remove the
      // inlined code and turn it into a normal call.
      if (isRecursiveCall(CF)) {
        if (VMap.find(CF) == VMap.end()) {
          declareFunction(CF);
          addGlobalMappingForFunction(CF);
        }
        copyInstruction(&Builder, CI);
        InlinedCalls.push_back(make_tuple(CurInstrIdx, CI));
        RecCallDepth = 1;
        return;
      }
      // This is neither recursion nor an external call, so keep it inlined.
      InlinedCalls.push_back(make_tuple(CurInstrIdx, CI));
      // Remap function arguments to the variables passed in by the caller.
      for (unsigned int i = 0; i < CI->arg_size(); i++) {
        Value *Var = CI->getArgOperand(i);
        Value *Arg = CF->getArg(i);
        // Check the operand for things we need to remap, e.g. globals.
        handleOperand(Var);
        // If the operand has already been cloned into JITMod then we
        // need to use the cloned value in the VMap.
        VMap[Arg] = getMappedValue(Var);
      }
    }
  }

  void handleReturnInst(Instruction *I) {
    ResumeAfter = InlinedCalls.back();
    InlinedCalls.pop_back();
    LastCompletedBlocks.pop_back();
    if (RecCallDepth > 0) {
      RecCallDepth -= 1;
      return;
    }
    // Replace the return variable of the call with its return value.
    // Since the return value will have already been copied over to the
    // JITModule, make sure we look up the copy.
    auto OldRetVal = ((ReturnInst *)&*I)->getReturnValue();
    if (OldRetVal != nullptr) {
      assert(ResumeAfter.hasValue());
      VMap[get<1>(ResumeAfter.getValue())] = getMappedValue(OldRetVal);
    }
  }

  void handlePHINode(Instruction *I, BasicBlock *BB) {
    Value *V = ((PHINode *)&*I)->getIncomingValueForBlock(BB);
    VMap[&*I] = getMappedValue(V);
  }

  Function *createJITFunc(Value *TraceInputs, Type *RetTy) {
    // Compute a name for the trace.
    uint64_t TraceIdx = getNewTraceIdx();
    TraceName = string(TRACE_FUNC_PREFIX) + to_string(TraceIdx);

    // Create the function.
    std::vector<Type *> InputTypes;
    InputTypes.push_back(TraceInputs->getType());
    llvm::FunctionType *FType =
        llvm::FunctionType::get(RetTy, InputTypes, false);
    llvm::Function *JITFunc = llvm::Function::Create(
        FType, Function::InternalLinkage, TraceName, JITMod);
    JITFunc->setCallingConv(CallingConv::C);

    return JITFunc;
  }

  // Delete the dead value `V` from its parent, also deleting any dependencies
  // of `V` (i.e. operands) which then become dead.
  void deleteDeadTransitive(Value *V) {
    assert(V->user_empty()); // The value should be dead.
    vector<Value *> Work;
    Work.push_back(V);
    while (!Work.empty()) {
      Value *V = Work.back();
      Work.pop_back();
      // Remove `V` (an instruction or a global variable) from its parent
      // container. If any of the operands of `V` have a sole use, then they
      // will become dead and can also be deleted too.
      if (isa<Instruction>(V)) {
        Instruction *I = cast<Instruction>(V);
        for (auto &Op : I->operands()) {
          if (Op->hasOneUser()) {
            Work.push_back(&*Op);
          }
        }
        I->eraseFromParent();
      } else if (isa<GlobalVariable>(V)) {
        GlobalVariable *G = cast<GlobalVariable>(V);
        for (auto &Op : G->operands()) {
          if (Op->hasOneUser()) {
            Work.push_back(&*Op);
          }
        }
        // Be sure to remove this global variable from `cloned_globals` too, so
        // that we don't try to add an initialiser later in `finalise()`.
        erase_if(cloned_globals, [G, this](GlobalVariable *CG) {
          assert(VMap.find(CG) != VMap.end());
          return G == VMap[CG];
        });
        G->eraseFromParent();
      } else {
        dumpValueAndExit("Unexpected Value", V);
      }
    }
  }

public:
  // Store virtual addresses for called functions.
  std::map<GlobalValue *, void *> globalMappings;
  // The function name of this trace.
  string TraceName;
  // Mapping from AOT instructions to JIT instructions.
  ValueToValueMapTy VMap;

  // OPT: https://github.com/ykjit/yk/issues/419
  JITModBuilder(Module *AOTMod, char *FuncNames[], size_t BBs[],
                size_t TraceLen, char *FAddrKeys[], void *FAddrVals[],
                size_t FAddrLen)
      : Builder(AOTMod->getContext()), InpTrace(FuncNames, BBs, TraceLen),
        FAddrs(FAddrKeys, FAddrVals, FAddrLen) {
    this->AOTMod = AOTMod;

    JITMod = new Module("", AOTMod->getContext());
  }

  // Generate the JIT module.
  Module *createModule() {
    LLVMContext &JITContext = JITMod->getContext();
    // Find the trace inputs.
    Value *TraceInputs = getYkCtrlPointVarsStruct(AOTMod, InpTrace);

    // Get new control point call.
    Function *F = AOTMod->getFunction(YK_NEW_CONTROL_POINT);
    User *CallSite = F->user_back();
    CallInst *CPCI = cast<CallInst>(CallSite);
    Type *OutputStructTy = CPCI->getType();

    // When assembling a trace, we start collecting instructions below the
    // control point and finish above it. This means that alloca'd variables
    // become undefined (as they are defined outside of the trace) and thus
    // need to be remapped to the input of the compiled trace. SSA values
    // remain correct as phi nodes at the beginning of the trace automatically
    // select the appropriate input value.
    //
    // For example, once patched, a typical interpreter loop will look like
    // this:
    //
    // ```
    // bb0:
    //   %a = alloca  // Stack variable
    //   store 0, %a
    //   %b = 1       // Register variable
    //   br %bb1
    //
    // bb1:
    //   %b1 = phi [%b, %bb0], [%binc, %bb1]
    //   %s = new YkCtrlPointVars
    //
    //   insertvalue %s, %a, 0
    //   insertvalue %s, %b1, 1           // traces end here
    //   %s2 = call yk_new_control_point(%s)
    //   %anew = extractvalue %s, 0       // traces start here
    //   %bnew = extractvalue %s, 1
    //
    //   %aload = load %anew
    //   %ainc = add 1, %aload
    //   store %ainc, %a
    //   %binc = add 1, %bnew
    //   br %bb1
    // ```
    //
    // There are two trace inputs (`%a` and `%b1`) and two trace outputs
    // (`%anew` and `%bnew`). `%a` and `%anew` correspond to the same
    // high-level variable, and so do `%b1` and `%bnew`. When assembling a
    // trace from the above IR, it would look like this:
    //
    // ```
    // void compiled_trace(%YkCtrlPointVars %s) {
    //   %anew = extractvalue %s, 0     // traces start here
    //   %bnew = extractvalue %s, 1
    //
    //   %aload = load %anew
    //   %ainc = add 1, %aload
    //   store %ainc, %a                // %a is undefined
    //   %binc = add 1, %bnew
    //   %b1 = phi(bb0: %b, bb1: %binc)
    //   %s = new struct
    //
    //   insertvalue %s, %a, 0
    //   insertvalue %s, %b1, 1         // traces end here
    //   br %bb0
    // }
    // ```
    //
    // Here `%a` is undefined because we didn't trace its allocation. Instead
    // it needs to be extracted from the `YkCtrlPointVars`, which means we need
    // to replace `%a` with `%anew` in the store instruction. The other value
    // `%b` doesn't have this problem, since the PHI node already makes sure it
    // selects the correct SSA value `%binc`.
    Value *OutS = CPCI->getArgOperand(1);
    while (isa<InsertValueInst>(OutS)) {
      InsertValueInst *IVI = cast<InsertValueInst>(OutS);
      if (!isa<PHINode>(IVI->getInsertedValueOperand())) {
        InsertValueMap[*IVI->idx_begin()] = IVI->getInsertedValueOperand();
      }
      OutS = IVI->getAggregateOperand();
    }

    // Create function to store compiled trace.
    Function *JITFunc = createJITFunc(TraceInputs, CPCI->getType());

    // Remap control point return value.
    VMap[CPCI] = JITFunc->getArg(0);

    // Map the YkCtrlPointVars struct used inside the trace to the argument of
    // the compiled trace function.
    VMap[TraceInputs] = JITFunc->getArg(0);

    // Create entry block and setup builder.
    auto DstBB = BasicBlock::Create(JITContext, "", JITFunc);
    Builder.SetInsertPoint(DstBB);

    LastCompletedBlocks.push_back(nullptr);
    BasicBlock *NextCompletedBlock = nullptr;

    // Iterate over the trace and stitch together all traced blocks.
    for (size_t Idx = 0; Idx < InpTrace.Length(); Idx++) {
      Optional<IRBlock> MaybeIB = InpTrace[Idx];
      if (ExpectUnmappable && !MaybeIB.hasValue()) {
        ExpectUnmappable = false;
        continue;
      }
      assert(MaybeIB.hasValue());
      IRBlock IB = MaybeIB.getValue();

      // Get a traced function so we can extract blocks from it.
      Function *F = AOTMod->getFunction(IB.FuncName);
      if (!F)
        errx(EXIT_FAILURE, "can't find function %s", IB.FuncName);

      if (F->getName() == YK_NEW_CONTROL_POINT) {
        continue;
      }

      // Skip to the correct block.
      auto It = F->begin();
      std::advance(It, IB.BBIdx);
      BasicBlock *BB = &*It;

      assert(LastCompletedBlocks.size() >= 1);
      LastCompletedBlocks.back() = NextCompletedBlock;
      NextCompletedBlock = BB;

      // Iterate over all instructions within this block and copy them over
      // to our new module.
      for (size_t CurInstrIdx = 0; CurInstrIdx < BB->size(); CurInstrIdx++) {
        // If we've returned from a call, skip ahead to the instruction where
        // we left off.
        if (ResumeAfter.hasValue() != 0) {
          CurInstrIdx = std::get<0>(ResumeAfter.getValue()) + 1;
          ResumeAfter.reset();
        }
        auto I = BB->begin();
        std::advance(I, CurInstrIdx);
        assert(I != BB->end());

        // Skip calls to debug intrinsics (e.g. @llvm.dbg.value). We don't
        // currently handle debug info and these "pseudo-calls" cause our blocks
        // to be prematurely terminated.
        if (isa<DbgInfoIntrinsic>(I))
          continue;

        if (isa<CallInst>(I)) {
          CallInst *CI = cast<CallInst>(I);
          Function *CF = CI->getCalledFunction();
          if (CF == nullptr) {
            if (NewControlPointCall == nullptr) {
              continue;
            }
            // The target isn't statically known, so we can't inline the
            // callee.
            if (!isa<InlineAsm>(CI->getCalledOperand())) {
              // Look ahead in the trace to find the callee so we can
              // map the arguments if we are inlining the call.
              Optional<IRBlock> MaybeNextIB = InpTrace[Idx + 1];
              if (MaybeNextIB.hasValue()) {
                CF = AOTMod->getFunction(MaybeNextIB.getValue().FuncName);
              } else {
                CF = nullptr;
              }
              // FIXME Don't inline indirect calls unless promoted.
              handleCallInst(CI, CF, CurInstrIdx);
              break;
            }
          } else if (CF->getName() == YK_NEW_CONTROL_POINT) {
            if (NewControlPointCall == nullptr) {
              NewControlPointCall = &*CI;
            } else {
              VMap[CI] = getMappedValue(CI->getArgOperand(1));
              ResumeAfter = make_tuple(CurInstrIdx, CI);
              break;
            }
            continue;
          } else if (CF->getName() == YKTRACE_STOP) {
            finalise(AOTMod, &Builder);
            return JITMod;
          } else if (NewControlPointCall != nullptr) {
            handleCallInst(CI, CF, CurInstrIdx);
            break;
          }
        }

        // We don't start copying instructions into the JIT module until we've
        // seen the call to YK_NEW_CONTROL_POINT.
        if (NewControlPointCall == nullptr)
          continue;

        if (isa<IndirectBrInst>(I)) {
          // FIXME Replace all potential CFG divergence with guards.
          //
          // It isn't necessary to copy the indirect branch into the `JITMod`
          // as the successor block is known from the trace. However, naively
          // not copying the branch would lead to dangling references in the IR
          // because the `address` operand typically (indirectly) references
          // AOT block addresses not present in the `JITMod`. Therefore we also
          // remove the IR instruction which defines the `address` operand and
          // anything which also becomes dead as a result (recursively).
          Value *FirstOp = I->getOperand(0);
          assert(VMap.find(FirstOp) != VMap.end());
          DeleteDeadOnFinalise.push_back(VMap[FirstOp]);
          continue;
        }

        if ((isa<BranchInst>(I)) || isa<SwitchInst>(I)) {
          // FIXME Replace all potential CFG divergence with guards.
          continue;
        }

        if (isa<ReturnInst>(I)) {
          handleReturnInst(&*I);
          break;
        }

        if (RecCallDepth > 0) {
          // We are currently ignoring an inlined function.
          continue;
        }

        if (isa<PHINode>(I)) {
          assert(LastCompletedBlocks.size() >= 1);
          handlePHINode(&*I, LastCompletedBlocks.back());
          continue;
        }

        // If execution reaches here, then the instruction I is to be copied
        // into JITMod.
        copyInstruction(&Builder, (Instruction *)&*I);

        // Perform the remapping described by InsertValueMap. See comments
        // above.
        if (isa<ExtractValueInst>(I)) {
          ExtractValueInst *EVI = cast<ExtractValueInst>(I);
          if (EVI->getAggregateOperand()->getType() == OutputStructTy) {
            Value *IV = InsertValueMap[*EVI->idx_begin()];
            VMap[IV] = getMappedValue(EVI);
          }
        }
      }
    }

    Builder.CreateRet(VMap[CPCI]);
    finalise(AOTMod, &Builder);
    return JITMod;
  }

  void handleOperand(Value *Op) {
    if (VMap.find(Op) == VMap.end()) {
      // The operand is undefined in JITMod.
      Type *OpTy = Op->getType();

      // Variables allocated outside of the traced section must be passed into
      // the trace and thus must already have a mapping.
      assert(!isa<llvm::AllocaInst>(Op));

      if (isa<ConstantExpr>(Op)) {
        // A `ConstantExpr` may contain operands that require remapping, e.g.
        // global variables. Iterate over all operands and recursively call
        // `handleOperand` on them, then generate a new `ConstantExpr` with
        // the remapped operands.
        ConstantExpr *CExpr = cast<ConstantExpr>(Op);
        std::vector<Constant *> NewCEOps;
        for (unsigned CEOpIdx = 0; CEOpIdx < CExpr->getNumOperands();
             CEOpIdx++) {
          Value *CEOp = CExpr->getOperand(CEOpIdx);
          handleOperand(CEOp);
          NewCEOps.push_back(cast<Constant>(getMappedValue(CEOp)));
        }
        Constant *NewCExpr = CExpr->getWithOperands(NewCEOps);
        VMap[CExpr] = NewCExpr;
      } else if (isa<GlobalVariable>(Op)) {
        // If there's a reference to a GlobalVariable, copy it over to the
        // new module.
        GlobalVariable *OldGV = cast<GlobalVariable>(Op);
        // Global variable is a constant so just copy it into the trace.
        // We don't need to check if this global already exists, since
        // we're skipping any operand that's already been cloned into
        // the VMap.
        GlobalVariable *GV = new GlobalVariable(
            *JITMod, OldGV->getValueType(), OldGV->isConstant(),
            OldGV->getLinkage(), (Constant *)nullptr, OldGV->getName(),
            (GlobalVariable *)nullptr, OldGV->getThreadLocalMode(),
            OldGV->getType()->getAddressSpace());
        VMap[OldGV] = GV;
        if (OldGV->isConstant()) {
          GV->copyAttributesFrom(&*OldGV);
          cloned_globals.push_back(OldGV);
        }
      } else if ((isa<Constant>(Op)) || (isa<InlineAsm>(Op))) {
        if (isa<Function>(Op)) {
          // We are storing a function pointer in a variable, so we need to
          // redeclare the function in the JITModule in case it gets called.
          declareFunction(cast<Function>(Op));
        }
        // Constants and inline asm don't need to be mapped.
      } else if (Op == NewControlPointCall) {
        // The value generated by NewControlPointCall is the thread tracer.
        // At some optimisation levels, this gets stored in an alloca'd
        // stack space. Since we've stripped the instruction that
        // generates that value (from the JIT module), we have to make a
        // dummy stack slot to keep LLVM happy.
        Value *NullVal = Constant::getNullValue(OpTy);
        VMap[Op] = NullVal;
      } else {
        dumpValueAndExit("don't know how to handle operand", Op);
      }
    }
  }

  void copyInstruction(IRBuilder<> *Builder, Instruction *I) {
    // Before copying an instruction, we have to scan the instruction's
    // operands checking that each is defined in JITMod.
    for (unsigned OpIdx = 0; OpIdx < I->getNumOperands(); OpIdx++) {
      Value *Op = I->getOperand(OpIdx);
      handleOperand(Op);
    }

    // Shortly we will copy the instruction into the JIT module. We start by
    // cloning the instruction.
    auto NewInst = &*I->clone();

    // Since the instruction operands still reference values from the AOT
    // module, we must remap them to point to new values in the JIT module.
    llvm::RemapInstruction(NewInst, VMap, RF_NoModuleLevelChanges);
    VMap[&*I] = NewInst;

    // Copy over any debugging metadata required by the instruction.
    llvm::SmallVector<std::pair<unsigned, llvm::MDNode *>, 1> metadataList;
    I->getAllMetadata(metadataList);
    for (auto MD : metadataList) {
      NewInst->setMetadata(
          MD.first, MapMetadata(MD.second, VMap, llvm::RF_MoveDistinctMDs));
    }

    // And finally insert the new instruction into the JIT module.
    Builder->Insert(NewInst);
  }

  // Finalise the JITModule by adding a return instruction and initialising
  // global variables.
  void finalise(Module *AOTMod, IRBuilder<> *Builder) {
    // Now that we've seen all possible uses of values in the JITMod, we can
    // delete the values we've marked dead (and possibly their dependencies if
    // they too turn out to be dead).
    for (auto &V : DeleteDeadOnFinalise)
      deleteDeadTransitive(V);

    // Fix initialisers/referrers for copied global variables.
    // FIXME Do we also need to copy Linkage, MetaData, Comdat?
    for (GlobalVariable *G : cloned_globals) {
      GlobalVariable *NewGV = cast<GlobalVariable>(VMap[G]);
      if (G->isDeclaration())
        continue;

      if (G->hasInitializer())
        NewGV->setInitializer(MapValue(G->getInitializer(), VMap));
    }

    // Ensure that the JITModule has a `!llvm.dbg.cu`.
    // This code is borrowed from LLVM's `cloneFunction()` implementation.
    // OPT: Is there a faster way than scanning the whole module?
    DebugInfoFinder DIFinder;
    DIFinder.processModule(*AOTMod);
    if (DIFinder.compile_unit_count()) {
      auto *NMD = JITMod->getOrInsertNamedMetadata("llvm.dbg.cu");
      SmallPtrSet<const void *, 8> Visited;
      for (auto *Operand : NMD->operands())
        Visited.insert(Operand);
      for (auto *Unit : DIFinder.compile_units())
        if (Visited.insert(Unit).second)
          NMD->addOperand(Unit);
    }
  }
};

// An annotator for `Module::print()` which adds debug location lines.
class DebugAnnotationWriter : public AssemblyAnnotationWriter {
  string LastLineInfo;

public:
  void emitInstructionAnnot(const Instruction *I, formatted_raw_ostream &OS) {
    const DebugLoc &DL = I->getDebugLoc();
    string LineInfo;
    raw_string_ostream RSO(LineInfo);
    DL.print(RSO);
    if ((!LineInfo.empty()) && (LineInfo != LastLineInfo)) {
      string FuncName = "<unknown-func>";
      const MDNode *Scope = DL.getInlinedAtScope();
      if (auto *SP = getDISubprogram(Scope))
        FuncName.assign(SP->getName().data());
      OS << "  ; " << FuncName << "() " << LineInfo << "\n";
      LastLineInfo = LineInfo;
    }
  }
};

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
  JITPostOpt,
};

class DebugIRPrinter {
private:
  bitset<3> toPrint;

  const char *debugIRStr(DebugIR IR) {
    switch (IR) {
    case DebugIR::AOT:
      return "aot";
    case DebugIR::JITPreOpt:
      return "jit-pre-opt";
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
      else if (strcmp(Val, "jit-post-opt") == 0)
        toPrint.set(DebugIR::JITPostOpt);
      else
        errx(EXIT_FAILURE, "invalid parameter for YKD_PRINT_IR: '%s'", Val);
    }
  }

  void print(enum DebugIR IR, Module *M) {
    if (toPrint[IR]) {
      string PrintMode = debugIRStr(IR);
      errs() << "--- Begin " << PrintMode << " ---\n";
      DebugAnnotationWriter DAW;
      M->print(errs(), &DAW);
      errs() << "--- End " << PrintMode << " ---\n";
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
                               map<GlobalValue *, void *> GlobalMappings) {
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
                                              void *FAddrVals[],
                                              size_t FAddrLen) {
  DebugIRPrinter DIP;

  ThreadSafeModule *ThreadAOTMod = getThreadAOTMod();
  // Getting the module without acquiring the context lock is safe in this
  // instance since ThreadAOTMod is not shared between threads.
  Module *AOTMod = ThreadAOTMod->getModuleUnlocked();

  DIP.print(DebugIR::AOT, AOTMod);

  JITModBuilder JB(AOTMod, FuncNames, BBs, Len, FAddrKeys, FAddrVals, FAddrLen);
  auto JITMod = JB.createModule();
  DIP.print(DebugIR::JITPreOpt, JITMod);
#ifndef NDEBUG
  llvm::verifyModule(*JITMod, &llvm::errs());
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
