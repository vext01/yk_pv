use llvm_sys::{LLVMOpcode, core::*};
use llvm_sys::orc2::LLVMOrcThreadSafeModuleRef;
use llvm_sys::prelude::{LLVMBasicBlockRef, LLVMModuleRef, LLVMTypeRef, LLVMValueRef};
use llvm_sys::target::{
    LLVMABISizeOfType, LLVMGetModuleDataLayout, LLVMOffsetOfElement, LLVMTargetDataRef,
};
use llvm_sys::{LLVMIntPredicate, LLVMTypeKind};
use std::{ffi::{CStr, c_char}, fmt};

pub struct ThreadSafeModule(LLVMOrcThreadSafeModuleRef);

impl ThreadSafeModule {
    pub fn aot_module() -> Self {
        let (data, len) = ykutil::obj::llvmbc_section();
        let aotmod = unsafe {
            LLVMGetThreadSafeModule(&BitcodeSection { data, len } as *const BitcodeSection)
        };
        Self(aotmod)
    }

    // FIXME: kill this by abstracting WithModuleDo.
    pub fn as_raw(&self) -> LLVMOrcThreadSafeModuleRef {
        self.0
    }
}

// Replicates struct of same name in `ykllvmwrap.cc`.
#[repr(C)]
pub struct BitcodeSection {
    pub data: *const u8,
    pub len: u64,
}

extern "C" {
    pub fn LLVMGetThreadSafeModule(bs: *const BitcodeSection) -> LLVMOrcThreadSafeModuleRef;
    pub fn __yktracec_collect_gep_offset(m: LLVMModuleRef, gep: LLVMValueRef) -> usize;
    pub fn __yktracec_get_raw_data_values(const_seq: LLVMValueRef, out_size: &mut usize) -> *const c_char;
    pub fn __yktracec_num_deopt_vars(cb: LLVMValueRef) -> usize;
    pub fn __yktracec_get_deopt_vars(cb: LLVMValueRef, fill: *const LLVMValueRef);
}

#[derive(Clone, Copy)]
pub struct TargetData(LLVMTargetDataRef);

impl TargetData {
    pub fn get(&self) -> LLVMTargetDataRef {
        self.0
    }
}

#[derive(Clone, Copy)]
pub struct Module(LLVMModuleRef);

impl Module {
    pub unsafe fn new(module: LLVMModuleRef) -> Self {
        Self(module)
    }

    pub fn target_data(&self) -> TargetData {
        TargetData(unsafe { LLVMGetModuleDataLayout(self.0) })
    }

    pub fn first_function(&self) -> Option<Value> {
        let f = unsafe { LLVMGetFirstFunction(self.0) };
        if !f.is_null() {
            Some(Value(f))
        } else {
            None
        }
    }

    pub fn get(&self) -> LLVMModuleRef {
        self.0
    }
}

#[derive(PartialEq, Debug, Eq, Clone, Copy, Hash)]
pub struct Type(LLVMTypeRef);
impl Type {
    pub fn get(&self) -> LLVMTypeRef {
        self.0
    }

    pub fn is_array_ty(&self) -> bool {
        self.kind() == LLVMTypeKind::LLVMArrayTypeKind
    }

    pub fn is_struct_ty(&self) -> bool {
        self.kind() == LLVMTypeKind::LLVMStructTypeKind
    }

    pub fn is_void_ty(&self) -> bool {
        self.kind() == LLVMTypeKind::LLVMVoidTypeKind
    }

    pub fn kind(&self) -> LLVMTypeKind {
        unsafe { LLVMGetTypeKind(self.0) }
    }

    pub fn is_integer(&self) -> bool {
        matches!(self.kind(), LLVMTypeKind::LLVMIntegerTypeKind)
    }

    pub fn get_int_width(&self) -> u32 {
        debug_assert!(self.is_integer());
        unsafe { LLVMGetIntTypeWidth(self.0) }
    }

    pub fn abi_size_in_bytes(&self, td: TargetData) -> usize {
        usize::try_from(unsafe { LLVMABISizeOfType(td.0, self.0) }).unwrap()
    }

    pub fn element_type(&self) -> Type {
        debug_assert!(self.is_array_ty());
        Type(unsafe { LLVMGetElementType(self.0) })
    }

    pub fn struct_field_offset(&self, target: TargetData, idx: usize) -> usize {
        debug_assert!(self.is_struct_ty());
        unsafe { LLVMOffsetOfElement(target.get(), self.0, idx.try_into().unwrap()) }
            .try_into()
            .unwrap()
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub struct BasicBlock(LLVMBasicBlockRef);
impl BasicBlock {
    pub fn next_basic_block(&self) -> Option<Self> {
        let b = unsafe { LLVMGetNextBasicBlock(self.0) };
        if !b.is_null() {
            Some(Self(b))
        } else {
            None
        }
    }

    pub fn first_instruction(&self) -> Option<Value> {
        let i = unsafe { LLVMGetFirstInstruction(self.0) };
        if !i.is_null() {
            Some(Value(i))
        } else {
            None
        }
    }

    pub fn iter_instructions(&self) -> BasicBlockIterator {
        BasicBlockIterator(self.first_instruction())
    }

    pub fn name(&self) -> &CStr {
        // Cast is safe, as in LLVM's type hierarchy a `BasicBlock` is a `Value`.
        let mut size: usize = 0; // FIXME: use maybeinit.
        unsafe {
            CStr::from_ptr(LLVMGetValueName2(
                self.0 as LLVMValueRef,
                &mut size as *mut usize,
            ))
        }
    }
}

/// Iterate over the instructions of a basic block.
pub struct BasicBlockIterator(Option<Value>);
impl Iterator for BasicBlockIterator {
    type Item = Value;
    fn next(&mut self) -> Option<Self::Item> {
        match self.0 {
            Some(b) => {
                let next = b.next_instruction();
                let ret = self.0.take();
                self.0 = next;
                ret
            }
            None => {
                // already exhausted
                None
            }
        }
    }
}

/// Iterate over the basic blocks of a function.
pub struct FunctionIterator(Option<BasicBlock>);
impl Iterator for FunctionIterator {
    type Item = BasicBlock;
    fn next(&mut self) -> Option<Self::Item> {
        match self.0 {
            Some(b) => {
                let next = b.next_basic_block();
                let ret = self.0.take();
                self.0 = next;
                ret
            }
            None => {
                // already exhausted
                None
            }
        }
    }
}

// #[derive(Copy, Clone, Debug)]
// pub enum InstrKind {
//     GEP,
//     Load,
//     Branch,
// }

#[derive(PartialEq, Eq, Hash, Clone, Copy)]
pub struct Value(LLVMValueRef);
impl Value {
    pub unsafe fn new(vref: LLVMValueRef) -> Self {
        Value(vref)
    }

    pub fn name(&self) -> &CStr {
        let mut size: usize = 0; // FIXME: use maybeinit and can we dedup name() in basicblock?
        unsafe { CStr::from_ptr(LLVMGetValueName2(self.0, &mut size as *mut usize)) }
    }

    pub fn get(&self) -> LLVMValueRef {
        self.0
    }

    pub fn called_value(&self) -> Value {
        debug_assert!(self.is_call()); // || self.is_invoke());
        Value(unsafe { LLVMGetCalledValue(self.0) })
    }

    pub fn is_vector(&self) -> bool {
        self.type_kind() == LLVMTypeKind::LLVMVectorTypeKind
    }

    pub fn is_instruction(&self) -> bool {
        unsafe { !LLVMIsAInstruction(self.0).is_null() }
    }

    pub fn is_nullptr_constant(&self) -> bool {
        unsafe { !LLVMIsAConstantPointerNull(self.0).is_null() }
    }

    pub fn is_inline_asm(&self) -> bool {
        unsafe { !LLVMIsAInlineAsm(self.0).is_null() }
    }

    pub fn is_alloca(&self) -> bool {
        unsafe { !LLVMIsAAllocaInst(self.0).is_null() }
    }

    pub fn is_call(&self) -> bool {
        unsafe { !LLVMIsACallInst(self.0).is_null() }
    }

    pub fn is_function(&self) -> bool {
        unsafe { !LLVMIsAFunction(self.0).is_null() }
    }

    pub fn is_intrinsic(&self) -> bool {
        unsafe { !LLVMIsAIntrinsicInst(self.0).is_null() }
    }

    pub fn inst_opcode(&self) -> LLVMOpcode {
        debug_assert!(self.is_instruction());
        unsafe {LLVMGetInstructionOpcode(self.0) }
    }

	pub fn intrinsic_id(&self) -> c_uint {
		unsafe { LLVMGetIntrinsicID(self.0) }
	}

    pub fn is_deoptimise_intrinsic(&self) -> bool {
        self.intrinsic_id() == lookup_intrinsic_id("llvm.experimental.deoptimize")
    }

    pub fn num_deopt_vars(&self) -> usize {
        unsafe { __yktracec_num_deopt_vars(self.0) }
    }

    pub fn get_deopt_vars(&self) -> Vec<Value> {
        let num = self.num_deopt_vars();
        let mut refs: Vec<LLVMValueRef> = Vec::with_capacity(num);
        unsafe { __yktracec_get_deopt_vars(self.0, refs.as_mut_ptr()) };
        unsafe { refs.set_len(num) };
        refs.into_iter().map(|r| unsafe { Value::new(r) }).collect()
    }

    pub fn allocated_type(&self) -> Type {
        debug_assert!(self.is_alloca());
        Type(unsafe { LLVMGetAllocatedType(self.0) })
    }

    pub fn is_gep(&self) -> bool {
        unsafe { !LLVMIsAGetElementPtrInst(self.0).is_null() }
    }

    pub fn is_branch(&self) -> bool {
        unsafe { !LLVMIsABranchInst(self.0).is_null() }
    }

    pub fn is_conditional_branch(&self) -> bool {
        debug_assert!(self.is_branch());
        unsafe { LLVMIsConditional(self.0) == 1 }
    }

    // FIXME: should be just `type()`.
    pub fn get_type(&self) -> Type {
        unsafe { Type(LLVMTypeOf(self.0)) }
    }

    pub fn abi_size_in_bytes(&self, td: TargetData) -> usize {
        self.get_type().abi_size_in_bytes(td)
    }

    // FIXME: rename to just "operand()"
    pub fn get_operand(&self, idx: usize) -> Value {
        debug_assert!(!unsafe { LLVMIsAUser(self.0).is_null() });
        let ret = unsafe { LLVMGetOperand(self.0, u32::try_from(idx).unwrap()) };
        debug_assert!(!ret.is_null());
        unsafe { Value::new(ret) }
    }

    pub fn num_operands(&self) -> usize {
        usize::try_from(unsafe { LLVMGetNumOperands(self.0) }).unwrap()
    }

    pub fn num_func_params(&self) -> usize {
        usize::try_from(unsafe { LLVMCountParams(self.0) }).unwrap()
    }

    pub fn func_param(&self, idx: usize) -> Value {
        debug_assert!(self.is_function());
        Value(unsafe { LLVMGetParam(self.0, idx.try_into().unwrap()) })
    }

    pub fn has_indices(&self) -> bool {
        unsafe {
            !LLVMIsAGetElementPtrInst(self.0).is_null()
                || !LLVMIsAExtractValueInst(self.0).is_null()
                || !LLVMIsAInsertValueInst(self.0).is_null()
        }
    }

    pub fn num_indices(&self) -> usize {
        debug_assert!(self.has_indices());
        usize::try_from(unsafe { LLVMGetNumIndices(self.0) }).unwrap()
    }

    pub fn index(&self, idx: usize) -> usize {
        debug_assert!(self.has_indices());
        usize::try_from(unsafe { *LLVMGetIndices(self.0).add(idx) }).unwrap()
    }

    pub fn element_type(&self) -> Type {
        self.get_type().element_type()
    }

    pub fn first_basic_block(&self) -> Option<BasicBlock> {
        debug_assert!(self.is_function());
        let b = unsafe { LLVMGetFirstBasicBlock(self.0) };
        if !b.is_null() {
            Some(BasicBlock(b))
        } else {
            None
        }
    }

    pub fn next_instruction(&self) -> Option<Self> {
        let i = unsafe { LLVMGetNextInstruction(self.0) };
        if !i.is_null() {
            return Some(Value(i));
        } else {
            None
        }
    }

    pub fn iter_basic_blocks(&self) -> FunctionIterator {
        debug_assert!(self.is_function());
        FunctionIterator(self.first_basic_block())
    }

    pub fn is_constant(&self) -> bool {
        unsafe { !LLVMIsAConstant(self.0).is_null() }
    }

    pub fn is_global(&self) -> bool {
        !unsafe { LLVMIsAGlobalVariable(self.0) }.is_null()
    }

    pub fn is_declaration(&self) -> bool {
        debug_assert!(self.is_global());
        (unsafe { LLVMIsDeclaration(self.0) }) == 1
    }

    pub fn is_constant_int(&self) -> bool {
        unsafe { !LLVMIsAConstantInt(self.0).is_null() }
    }

    pub fn constant_zext_value(&self) -> u64 {
        debug_assert!(self.is_constant_int());
        unsafe { LLVMConstIntGetZExtValue(self.0) }
    }

    pub fn compute_gep_offset(&self, m: Module) -> usize {
        debug_assert!(self.is_gep());
        unsafe { __yktracec_collect_gep_offset(m.get(), self.get()) }
    }

    pub fn is_constant_one(&self) -> bool {
        self.constant_zext_value() == 1
    }

    pub fn is_icmp(&self) -> bool {
        !unsafe { LLVMIsAICmpInst(self.0) }.is_null()
    }

    pub fn icmp_predicate(&self) -> LLVMIntPredicate {
        // FIXME: assert
        unsafe { LLVMGetICmpPredicate(self.0) }
    }

    pub fn static_alloca_size_in_bytes(&self, target: TargetData) -> usize {
        debug_assert!(self.is_alloca());
        let n_elems = if self.num_operands() == 1 {
            let op1 = self.get_operand(0);
            // We (and other parts of Yk, e.g. stackmaps) can't handle dynamically sized
            // stackframes.
            debug_assert!(op1.is_constant_int());
            op1.constant_zext_value()
        } else {
            1
        };
        let ty_size = u64::try_from(self.allocated_type().abi_size_in_bytes(target)).unwrap();
        usize::try_from(n_elems.checked_mul(ty_size).unwrap()).unwrap()
    }

    pub fn is_basic_block(&self) -> bool {
        unsafe { LLVMIsABasicBlock(self.0) }.is_null()
    }

    pub fn as_basic_block(&self) -> BasicBlock {
        //debug_assert!(self.is_basic_block());
        unsafe { BasicBlock(LLVMValueAsBasicBlock(self.0)) }
    }

    pub fn type_kind(&self) -> LLVMTypeKind {
        unsafe { LLVMGetTypeKind(self.get_type().get()) }
    }

    pub fn constant_initialiser(&self) -> Value {
        debug_assert!(self.is_constant());
        Value(unsafe { LLVMGetInitializer(self.0) })
    }

    pub fn is_constant_data_sequential(&self) -> bool {
        !unsafe { LLVMIsAConstantDataSequential(self.0) }.is_null()
    }

    pub fn raw_data_values(&self) -> (*const c_char, usize) {
        debug_assert!(self.is_constant_data_sequential());
        let mut size = 0;
        let ptr = unsafe { __yktracec_get_raw_data_values(self.0, &mut size) };
        (ptr, size)
    }

    // pub fn instr_kind(&self) -> InstrKind {
    //     debug_assert!(self.is_instruction());
    //     // FIXME: is there a way to avoid all these FFI calls?
    //     if !unsafe { LLVMIsAGetElementPtrInst(self.0) }.is_null() {
    //         InstrKind::GEP
    //     } else if !unsafe { LLVMIsALoadInst(self.0) }.is_null() {
    //         InstrKind::Load
    //     } else if !unsafe { LLVMIsABrInst(self.0) }.is_null() {
    //         InstrKind::Branch
    //     } else {
    //         todo!("Unknown instruction kind: {self:?}")
    //     }
    // }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", unsafe {
            CStr::from_ptr(LLVMPrintValueToString(self.0))
                .to_str()
                .unwrap()
                .trim()
        })
    }
}

use std::ffi::{CString, c_uint};
pub fn lookup_intrinsic_id(name: &str) -> c_uint {
    let name_c = CString::new(name).unwrap();
    unsafe { LLVMLookupIntrinsicID(name_c.as_ptr() as *const i8, name.len()) }
}
