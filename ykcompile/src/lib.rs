#![feature(proc_macro_hygiene)]
#![feature(test)]

#[macro_use]
extern crate dynasm;
extern crate dynasmrt;
extern crate test;

use std::collections::HashMap;
use std::mem;

use yktrace::tir::{
    BinOp, Constant, ConstantInt, Local, Operand, Rvalue, SignedInt, Statement, TirOp, TirTrace,
    UnsignedInt,
};

use dynasmrt::DynasmApi;

enum DynasmConst {
    I32(i32),
    I64(i64),
}

impl From<&ConstantInt> for DynasmConst {
    fn from(ci: &ConstantInt) -> Self {
        match ci {
            ConstantInt::UnsignedInt(ui) => {
                // Unsigned to signed casts deliberate. We care only for the bit-representation.
                match ui {
                    UnsignedInt::U8(u) => Self::I32(*u as i32),
                    UnsignedInt::U16(u) => Self::I32(*u as i32),
                    UnsignedInt::U32(u) => Self::I32(*u as i32),
                    UnsignedInt::U64(u) => Self::I64(*u as i64),
                    UnsignedInt::Usize(u) => {
                        #[cfg(target_pointer_width = "64")]
                        {
                            Self::I64(*u as i64)
                        }
                        #[cfg(target_pointer_width = "32")]
                        Self::I64(*u as i32)
                    }
                    UnsignedInt::U128(_) => panic!("dynasm can't deal with 128-bit constants"),
                }
            }
            ConstantInt::SignedInt(si) => match si {
                SignedInt::I8(i) => Self::I32(*i as i32),
                SignedInt::I16(i) => Self::I32(*i as i32),
                SignedInt::I32(i) => Self::I32(*i as i32),
                SignedInt::I64(i) => Self::I64(*i as i64),
                SignedInt::Isize(i) => {
                    #[cfg(target_pointer_width = "64")]
                    {
                        Self::I64(*i as i64)
                    }
                    #[cfg(target_pointer_width = "32")]
                    Self::I64(*i as i32)
                }
                SignedInt::I128(_) => panic!("dynasm can't deal with 128-bit constants"),
            },
        }
    }
}

/// A compiled SIRTrace.
pub struct CompiledTrace {
    /// A compiled trace.
    mc: dynasmrt::ExecutableBuffer,
}

impl CompiledTrace {
    pub fn execute(&self) -> u64 {
        // For now a compiled trace always returns whatever has been left in register RAX. We also
        // assume for now that this will be a `u64`.
        let func: fn() -> u64 = unsafe { mem::transmute(self.mc.ptr(dynasmrt::AssemblyOffset(0))) };
        func()
    }
}

/// The `TraceCompiler` takes a `SIRTrace` and compiles it to machine code. Returns a `CompiledTrace`.
pub struct TraceCompiler {
    asm: dynasmrt::x64::Assembler,
    /// Contains the list of currently available registers.
    available_regs: Vec<u8>,
    /// Maps locals to their assigned registers.
    assigned_regs: HashMap<u32, u8>,
}

impl TraceCompiler {
    fn local_to_reg(&mut self, l: u32) -> u8 {
        // This is a really dumb register allocator, which runs out of available registers after 7
        // locals. We can do better than this by using StorageLive/StorageDead from the MIR to free
        // up registers again, and allocate additional locals on the stack. Though, ultimately we
        // probably want to implement a proper register allocator, e.g. linear scan.
        if l == 0 {
            0
        } else {
            let reg = self
                .available_regs
                .pop()
                .expect("Can't allocate more than 7 locals yet!");
            *self.assigned_regs.entry(l).or_insert(reg)
        }
    }

    /// Move constant `c` of type `usize` into local `a`.
    pub fn mov_local_usize(&mut self, local: u32, cnst: usize) {
        let reg = self.local_to_reg(local);
        dynasm!(self.asm
            ; mov Rq(reg), cnst as i32
        );
    }

    /// Move constant `c` of type `u8` into local `a`.
    pub fn mov_local_u8(&mut self, local: u32, cnst: u8) {
        let reg = self.local_to_reg(local);
        dynasm!(self.asm
            ; mov Rq(reg), cnst as i32
        );
    }

    /// Move local `var2` into local `var1`.
    fn mov_local_local(&mut self, l1: u32, l2: u32) {
        let lreg = self.local_to_reg(l1);
        let rreg = self.local_to_reg(l2);
        dynasm!(self.asm
            ; mov Rq(lreg), Rq(rreg)
        );
    }

    fn nop(&mut self) {
        dynasm!(self.asm
            ; nop
        );
    }

    fn c_mov_int(&mut self, local: u32, constant: &ConstantInt) {
        let reg = self.local_to_reg(local);
        let val = match constant {
            ConstantInt::UnsignedInt(UnsignedInt::U8(i)) => *i as i64,
            ConstantInt::UnsignedInt(UnsignedInt::Usize(i)) => *i as i64,
            e => todo!("SignedInt, etc: {}", e),
        };
        dynasm!(self.asm
            ; mov Rq(reg), QWORD val
        );
    }

    fn c_mov_bool(&mut self, local: u32, b: bool) {
        let reg = self.local_to_reg(local);
        dynasm!(self.asm
            ; mov Rq(reg), QWORD b as i64
        );
    }

    // FIXME only adds.
    fn c_binop(&mut self, _op: BinOp, dest: Local, opnd1: &Operand, opnd2: &Operand) {
        let r_dest = self.local_to_reg(dest.0);

        match (opnd1, opnd2) {
            (Operand::Place(p1), Operand::Place(p2)) => {
                let r1 = self.local_to_reg(Local::from(p1).0);
                let r2 = self.local_to_reg(Local::from(p2).0);

                dynasm!(self.asm
                    ; mov Rq(r_dest), Rq(r1)
                    ; add Rq(r_dest), Rq(r2))
            }
            (Operand::Place(p), Operand::Constant(Constant::Int(ci)))
            | (Operand::Constant(Constant::Int(ci)), Operand::Place(p)) => {
                let r = self.local_to_reg(Local::from(p).0);

                match DynasmConst::from(ci) {
                    DynasmConst::I32(i) => dynasm!(self.asm
                            ; mov Rd(r_dest), i
                            ; add Rd(r_dest), Rd(r)),
                    DynasmConst::I64(i) => dynasm!(self.asm
                            ; mov Rq(r_dest), QWORD i
                            ; add Rq(r_dest), Rq(r)),
                }
            }
            _ => todo!("unimplemented operand for binary operation"),
        }
    }

    fn statement(&mut self, stmt: &Statement) {
        match stmt {
            Statement::Assign(l, r) => {
                let local = Local::from(l);
                match r {
                    Rvalue::Use(Operand::Place(p)) => self.mov_local_local(local.0, p.local.0),
                    Rvalue::Use(Operand::Constant(c)) => match c {
                        Constant::Int(ci) => self.c_mov_int(local.0, ci),
                        Constant::Bool(b) => self.c_mov_bool(local.0, *b),
                        c => todo!("Not implemented: {}", c),
                    },
                    // FIXME checked binops are not yet checked.
                    Rvalue::BinaryOp(op, opnd1, opnd2)
                    | Rvalue::CheckedBinaryOp(op, opnd1, opnd2) => {
                        self.c_binop(*op, l.local, opnd1, opnd2)
                    }
                    unimpl => todo!("Not implemented: {:?}", unimpl),
                };
            }
            Statement::Return => {}
            Statement::Nop => {}
            Statement::Unimplemented(mir_stmt) => todo!("Can't compile: {}", mir_stmt),
        }
    }

    fn finish(mut self) -> dynasmrt::ExecutableBuffer {
        dynasm!(self.asm
            ; ret
        );
        self.asm.finalize().unwrap()
    }

    pub fn compile(tt: TirTrace) -> CompiledTrace {
        // Set available registers to R11-R8, RDX, RCX
        let regs = vec![11, 10, 9, 8, 2, 1];
        let assembler = dynasmrt::x64::Assembler::new().unwrap();
        let mut tc = TraceCompiler {
            asm: assembler,
            available_regs: regs,
            assigned_regs: HashMap::new(),
        };
        for i in 0..tt.len() {
            let t = tt.op(i);
            match t {
                TirOp::Statement(st) => tc.statement(st),
                TirOp::Guard(_) => tc.nop(), // FIXME Implement guards.
            }
        }
        CompiledTrace { mc: tc.finish() }
    }
}

#[cfg(test)]
mod tests {

    use super::TraceCompiler;
    use yktrace::tir::TirTrace;
    use yktrace::{start_tracing, TracingKind};

    #[inline(never)]
    fn simple() -> u8 {
        let x = 13;
        x
    }

    #[test]
    pub(crate) fn test_simple() {
        let th = start_tracing(Some(TracingKind::HardwareTracing));
        simple();
        let sir_trace = th.stop_tracing().unwrap();
        let tir_trace = TirTrace::new(&*sir_trace).unwrap();
        let ct = TraceCompiler::compile(tir_trace);
        assert_eq!(ct.execute(), 13);
    }

    #[inline(never)]
    fn add(x: u64, y: u64) -> u64 {
        x + y
    }

    #[test]
    pub(crate) fn test_binop() {
        let th = start_tracing(Some(TracingKind::HardwareTracing));
        add(10, 20);
        let sir_trace = th.stop_tracing().unwrap();
        let tir_trace = TirTrace::new(&*sir_trace).unwrap();
        let ct = TraceCompiler::compile(tir_trace);
        assert_eq!(ct.execute(), 30);
    }
}
