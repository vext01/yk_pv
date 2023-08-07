#![allow(dead_code)]
#![allow(unused_imports)]

use super::{FastTCG, Local};
use crate::frame::llvmbridge::{Module, Value};
use dynasmrt::{
    dynasm, x64::Rq, AssemblyOffset, DynasmApi, DynasmLabelApi, ExecutableBuffer, Register,
};
use iced_x86;
use llvm_sys::LLVMIntPredicate;
use std::{
    collections::HashMap,
    error::Error,
    ffi::{CString, c_void},
    fmt::{self, Debug, Formatter},
    slice,
    sync::LazyLock,
};
use yksmp::{LiveVar, Location};

/// Argument registers as defined by the X86_64 SysV ABI.
static ARG_REGS: [Rq; 6] = [Rq::RDI, Rq::RSI, Rq::RDX, Rq::RCX, Rq::R8, Rq::R9];
static REG64_SIZE: usize = 8;

static X86_64_REG_SIZE: usize = 8;

/// Work registers, i.e. the registers we use temproarily (where possible) for operands to
/// intermediate computations.
///
/// We choose callee-save registers so that we don't have to worry about storing/restoring them
/// when we do a function call to external code.
static WR0: Rq = Rq::R12;
static WR1: Rq = Rq::R13;
static WR2: Rq = Rq::R14;

static DEOPT_FUNC: LazyLock<CString> = LazyLock::new(|| CString::new("__llvm_deoptimize").unwrap());

impl FastTCG {
    /// Note that there is no correspoinding `emit_epilogue()`. This is because the only way out of
    /// JITted code is via deoptimisation, which will rewrite the whole stack anyway.
    ///
    /// Returns the offset at which to insert the stack allocation later.
    pub(super) fn emit_prologue(&mut self) -> AssemblyOffset {
        // Start a frame for the JITted code.
        dynasm!(self.asm
            ; push rbp
            ; mov rbp, rsp
        );

        // Emit a dummy stack allocation instruction that we will patch later when we know how big
        // the frame needs to be. This instruction must be patched with a same-sized instruction,
        // so we make sure to use an explicit size qualifier.
        let alloc_off = self.asm.offset();
        dynasm!(self.asm
            ; sub rsp, DWORD 0 // This will be patched later when we know the size of the frame.
        );

        // Spill each argument into a stack slot and track it in our locals mapping.
        let num_args = self.jitfunc.num_func_params();
        debug_assert!(num_args <= ARG_REGS.len());

        for i in 0..num_args {
            self.reg_into_new_local(self.jitfunc.func_param(i), ARG_REGS[i]);
        }

        alloc_off
    }

    fn load_local(&mut self, reg: Rq, value: Value) {
        let l = &self.locals[&value]; //self.local(value);
        let foff = i32::try_from(l.frame_off).unwrap();

        // We use `movzx` where possible to avoid partial register stalls.
        match value.abi_size_in_bytes(self.target) {
            1 => dynasm!(self.asm; movzx Rq(reg.code()), BYTE [rbp - foff]),
            2 => dynasm!(self.asm; movzx Rq(reg.code()), WORD [rbp - foff]),
            4 => dynasm!(self.asm; mov Rd(reg.code()), [rbp - foff]),
            8 => dynasm!(self.asm; mov Rq(reg.code()), [rbp - foff]),
            _ => todo!(), // Can it even happen? write tests.
        }
    }

    /// Allocates a new local variable in the JITted code frame.
    ///
    /// This is platform specific because the frame offset depends on if the stack grows up or down.
    ///
    /// FIXME: Merge with store_local?
    fn new_local(&mut self, value: Value) -> Local {
        debug_assert!(!self.locals.contains_key(&value));

        let size = value.get_type().abi_size_in_bytes(self.target);
        self.asp += i32::try_from(size).unwrap();
        let l = Local::new(self.asp);
        self.locals.insert(value, l);
        self.locals[&value]
    }

    fn store_local(&mut self, l: &Local, reg: Rq, size_in_bytes: usize) {
        debug_assert!(l.frame_off <= self.asp);
        match size_in_bytes {
            8 => dynasm!(self.asm ; mov [rbp - l.frame_off], Rq(reg.code())),
            4 => dynasm!(self.asm ; mov [rbp - l.frame_off], Rd(reg.code())),
            2 => dynasm!(self.asm ; mov [rbp - l.frame_off], Rw(reg.code())),
            1 => dynasm!(self.asm ; mov [rbp - l.frame_off], Rb(reg.code())),
            _ => todo!("{}", size_in_bytes),
        }
    }

    fn reg_into_new_local(&mut self, inst: Value, reg: Rq) {
        let l = self.new_local(inst);
        self.store_local(&l, reg, inst.abi_size_in_bytes(self.target));
    }

    pub(super) fn codegen_gep_inst(&mut self, inst: Value) {
        self.value_into_reg(WR0, inst.get_operand(0));

        let off = inst.compute_gep_offset(self.jitmod);
        // If fits in 32-bit can do:
        //dynasm!(self.asm; add Rq(WR0.code()), off.try_into().unwrap());
        dynasm!(self.asm
            ; mov Rq(WR1.code()), QWORD off as i64 // intentional as
            ; add Rq(WR0.code()), Rq(WR1.code())
        );
        self.reg_into_new_local(inst, WR0);
    }

    pub(super) fn codegen_load_inst(&mut self, inst: Value) {
        self.value_into_reg(WR0, inst.get_operand(0));
        // FIXME: assumes the thing fits in a register.
        let size = inst.abi_size_in_bytes(self.target);
        debug_assert!(size <= REG64_SIZE);
        match size {
            8 => dynasm!(self.asm ; mov Rq(WR0.code()), [Rq(WR0.code())]),
            4 => dynasm!(self.asm ; mov Rd(WR0.code()), [Rq(WR0.code())]),
            2 => dynasm!(self.asm ; movzx Rd(WR0.code()), WORD [Rq(WR0.code())]),
            1 => dynasm!(self.asm ; movzx Rq(WR0.code()), BYTE [Rq(WR0.code())]),
            _ => todo!("{}", size),
        };
        self.reg_into_new_local(inst, WR0);
    }

    pub(super) fn codegen_branch_inst(&mut self, inst: Value) {
        if inst.is_conditional_branch() {
            // Note the reversed operand order from what you might expect.
            let f_label = self.label_for_block(inst.get_operand(1).as_basic_block());
            let t_label = self.label_for_block(inst.get_operand(2).as_basic_block());
            self.value_into_reg(WR0, inst.get_operand(0));
            dynasm!(self.asm
                ; test Rb(WR0.code()), 1
                ; nop
                ; jz =>f_label
                ; jmp =>t_label
            )
        } else {
            let label = self.label_for_block(inst.get_operand(0).as_basic_block());
            dynasm!(self.asm; jmp =>label);
        }
    }

    pub(super) fn codegen_store_inst(&mut self, inst: Value) {
        let op0 = inst.get_operand(0);
        self.value_into_reg(WR0, op0);
        self.value_into_reg(WR1, inst.get_operand(1));
        let size = op0.abi_size_in_bytes(self.target);
        match size {
            8 => dynasm!(self.asm; mov [Rq(WR1.code())], Rq(WR0.code())),
            4 => dynasm!(self.asm; mov [Rq(WR1.code())], Rd(WR0.code())),
            2 => dynasm!(self.asm; mov [Rq(WR1.code())], Rw(WR0.code())),
            1 => dynasm!(self.asm; mov [Rq(WR1.code())], Rb(WR0.code())),
            _ => todo!(),
        }
    }

    pub(super) fn codegen_add_inst(&mut self, inst: Value) {
        let op0 = inst.get_operand(0);
        self.value_into_reg(WR0, op0);
        self.value_into_reg(WR1, inst.get_operand(1));
        match op0.abi_size_in_bytes(self.target) {
            8 => dynasm!(self.asm; add Rq(WR0.code()), Rq(WR1.code())),
            4 => dynasm!(self.asm; add Rd(WR0.code()), Rd(WR1.code())),
            1 => dynasm!(self.asm; add Rb(WR0.code()), Rb(WR1.code())),
            _ => todo!("{}", op0.abi_size_in_bytes(self.target)),
        }
        self.reg_into_new_local(inst, WR0);
    }

    pub(super) fn codegen_sub_inst(&mut self, inst: Value) {
        let op0 = inst.get_operand(0);
        self.value_into_reg(WR0, op0);
        self.value_into_reg(WR1, inst.get_operand(1));
        match op0.abi_size_in_bytes(self.target) {
            8 => dynasm!(self.asm; sub Rq(WR0.code()), Rq(WR1.code())),
            _ => todo!("{}", op0.abi_size_in_bytes(self.target)),
        }
        self.reg_into_new_local(inst, WR0);
    }

    pub(super) fn codegen_shl_inst(&mut self, inst: Value) {
        let op0 = inst.get_operand(0);
        if op0.get_type().is_integer() {
            self.value_into_reg(WR0, inst.get_operand(0));
            self.value_into_reg(WR1, inst.get_operand(1));
            dynasm!(self.asm
                ; shlx Rq(WR0.code()), Rq(WR0.code()), Rq(WR1.code())
            );
            self.reg_into_new_local(inst, WR0);
        } else {
            todo!();
        }
    }

    pub(super) fn codegen_select_inst(&mut self, inst: Value) {
        let cond = inst.get_operand(0);
        let t_val = inst.get_operand(1);
        let f_val = inst.get_operand(2);

        // We only handle scalar select for now.
        if cond.is_vector() {
            todo!();
        }

        // WR1 will contain the selected value.
        self.value_into_reg(WR0, cond);
        self.value_into_reg(WR1, t_val);
        self.value_into_reg(WR2, f_val);
        dynasm!(self.asm
            ; cmp Rb(WR0.code()), 1
            ; cmovne Rq(WR1.code()), Rq(WR2.code())
        );
        self.reg_into_new_local(inst, WR1);
    }

    pub fn const_into_reg(&mut self, reg: Rq, cv: u64) {
        dynasm!(self.asm
            ; mov Rq(reg.code()), QWORD cv as i64 // `as` intentional.
        )
    }

    pub fn global_into_reg(&mut self, reg: Rq, global: Value) {
        let vaddr = if !global.is_declaration() {
            if global.is_constant() {
                let init = global.constant_initialiser();
                if init.is_constant_data_sequential() {
                    let (src_ptr, size) = init.raw_data_values();
                    // FIXME: this is very dumb.
                    let dest_ptr = unsafe { libc::malloc(size) };
                    assert!(!dest_ptr.is_null());
                    unsafe { libc::memcpy(dest_ptr, src_ptr as *const c_void, size) };
                    dest_ptr as usize
                } else {
                    todo!();
                }
            } else {
                todo!();
            }
        } else {
            // It's an external symbol allocated and initialised elsewhere.
            use ykutil::addr::symbol_vaddr;
            symbol_vaddr(global.name()).unwrap()
        };
        let vaddr = i64::try_from(vaddr).unwrap();
        dynasm!(self.asm; mov Rq(reg.code()), QWORD vaddr);
    }

    pub fn value_into_reg(&mut self, reg: Rq, v: Value) {
        if v.abi_size_in_bytes(self.target) > X86_64_REG_SIZE {
            panic!("Doesn't fit in a register! {:?}", v);
        }

        if v.is_global() {
            self.global_into_reg(reg, v);
        } else if v.is_constant() {
            if v.is_constant_int() {
                self.const_into_reg(reg, v.constant_zext_value());
            } else if v.is_nullptr_constant() {
                self.const_into_reg(reg, 0);
            }
        } else {
            self.load_local(reg, v);
        }
    }

    pub (super) fn codegen_ptrtoint_inst(&mut self, inst: Value) {
        self.value_into_reg(WR0, inst.get_operand(0));
        self.reg_into_new_local(inst, WR0);
    }

    pub(super) fn codegen_icmp_inst(&mut self, inst: Value) {
        self.value_into_reg(WR0, inst.get_operand(0));
        self.value_into_reg(WR1, inst.get_operand(1));

        dynasm!(self.asm
            ; cmp Rq(WR0.code()), Rq(WR1.code())
        );
        match inst.icmp_predicate() {
            LLVMIntPredicate::LLVMIntSLT => dynasm!(self.asm; setl Rb(WR0.code())),
            LLVMIntPredicate::LLVMIntSGT => dynasm!(self.asm; setg Rb(WR0.code())),
            LLVMIntPredicate::LLVMIntULT => dynasm!(self.asm; setb Rb(WR0.code())),
            LLVMIntPredicate::LLVMIntUGT => dynasm!(self.asm; seta Rb(WR0.code())),
            LLVMIntPredicate::LLVMIntEQ => dynasm!(self.asm; sete Rb(WR0.code())),
            _ => todo!("{:?}", inst.icmp_predicate()),
        }

        debug_assert_eq!(inst.abi_size_in_bytes(self.target), 1);
        self.reg_into_new_local(inst, WR0);
    }

    pub(super) fn codegen_alloca_inst(&mut self, inst: Value) {
        let alloc_size = inst.static_alloca_size_in_bytes(self.target);

        // Reserve space for the allocation.
        self.asp += i32::try_from(alloc_size).unwrap();

        // Store a pointer to what we just allocated.
        dynasm!(self.asm
            ; lea Rq(WR1.code()), [rbp - self.asp]
        );
        self.reg_into_new_local(inst, WR1);
    }

    pub(super) fn codegen_intrinsic_inst(&mut self, inst: Value) {
        let intr = inst.called_value();
        if intr.is_deoptimise_intrinsic() {
            self.codegen_deopt_inst(inst);
        } else {
            todo!("{:?}", inst);
        }

        if !inst.get_type().is_void_ty() {
            self.new_local(inst);
        }
    }

    pub fn codegen_deopt_inst(&mut self, inst: Value) {
        // First compute the stackmap entry for the deopt.
        let vars = inst.get_deopt_vars();
        let mut rec = Vec::new();
        // Ugh. push a random location that is skipped in deopt. FML.
        rec.push(LiveVar::new(Vec::new()));
        for var in vars {
            let l = self.locals[&var];
            let rbp = 6; // FIXME
            let locs = vec![Location::Indirect(rbp, -l.frame_off, u16::try_from(var.abi_size_in_bytes(self.target)).unwrap())];
            let livevar = LiveVar::new(locs);
            rec.push(livevar);
        }

        // Emit the call to __llvm_deoptimize.
        let mut args = Vec::new();
        for i in 0..(inst.num_operands() - 1 - inst.num_deopt_vars()) { // list is the call target.
            args.push(inst.get_operand(i));
        }
        let target = inst.called_value();

        self.emit_call(target, args);

        // Write the stackmap keyed by the asm offset AFTER the call.
        self.stackmaps.get_mut().insert(u64::try_from(self.asm.offset().0).unwrap(), rec);
    }

    fn get_ext_fn_addr(&self, func: Value) -> Option<usize> {
        if let Some(va) = self.global_mappings.get(&func.get())  {
            Some(*va as usize)
        } else {
            use ykutil::addr::symbol_vaddr;
            let mut fname = func.name();
            if fname.to_str().unwrap().starts_with("llvm.experimental.deoptimize") {
                fname = &DEOPT_FUNC;
            }
            symbol_vaddr(fname)
        }
    }

    fn emit_call(&mut self, target: Value, args: Vec<Value>) {
        if args.len() > ARG_REGS.len() {
            todo!("needs spill: {} args", args.len());
        }

        for i in 0..args.len() {
            let reg = ARG_REGS[i];
            self.value_into_reg(reg, args[i]);
        }

        // XXX edd, why isn't it in global_mappings?
        let va = self.get_ext_fn_addr(target).unwrap();

        // Note: The stack is already aligned prior to this call sequence.
        dynasm!(self.asm
            ; mov Rq(WR0.code()), QWORD va as i64
            ; xor rax, rax
            ; call Rq(WR0.code())
        );
    }

    pub(super) fn codegen_sext_inst(&mut self, inst: Value) {
        let op0 = inst.get_operand(0);
        self.value_into_reg(WR0, op0);

        // We can sign extend into a 64-bit register and it is safe to truncate it if it is
        // smaller.
        let size = op0.abi_size_in_bytes(self.target);
        match size {
            1 => dynasm!(self.asm; movsx Rq(WR0.code()), Rb(WR0.code())),
            2 => dynasm!(self.asm; movsx Rq(WR0.code()), Rw(WR0.code())),
            4 => dynasm!(self.asm; movsxd Rq(WR0.code()), Rd(WR0.code())),
            8 => {}, // NOP
            _ => todo!("{}", size),
        }

        self.reg_into_new_local(inst, WR0);
    }

    pub(super) fn codegen_call_inst(&mut self, inst: Value) {
        // rbp-0xc4d
        if inst.is_intrinsic() {
            self.codegen_intrinsic_inst(inst)
        } else {
            let mut args = Vec::new();
            for i in 0..(inst.num_operands() - 1) { // last is the call target.
                args.push(inst.get_operand(i));
            }

            let target = inst.called_value();
            if target.is_inline_asm() {
                return; // FIXME
            }
            self.emit_call(target, args);

            if !inst.get_type().is_void_ty() {
                self.reg_into_new_local(inst, Rq::RAX);
            }
        }
    }

    pub(super) fn codegen_ret_inst(&mut self, _inst: Value) {
        dynasm!(self.asm; ret);
    }
}
