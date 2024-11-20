//! This module adds some basic well-formedness checks to the JIT IR. These are intended both to
//! help debugging incorrectly formed IR and to provide guarantees about what IR different stages
//! of the compiler can expect.
//!
//! Specifically, after calling [Module::assert_well_formed] one can safely assume:
//!
//!   * [super::BinOpInst]s:
//!       * Have left and right hand side operands with the same [Ty]s.
//!       * Have left and right hand side operands compatible with the operation in question.
//!   * [super::DirectCallInst]s pass the correct number of arguments to a [super::FuncTy] and each
//!     of those arguments has the correct [super::Ty].
//!   * [super::FPExtInst]s:
//!       * Have an float-typed source operand.
//!       * Have a float-type as the destination type operand.
//!       * Have a destination type operand strictly larger than the type of the source operand.
//!   * [super::GuardInst]s:
//!       * Have a `cond` whose type is [super::Ty::Integer(1)] (i.e. an `i1`).
//!       * If `cond` references a constant, that constant matches the guard's `expect` attribute.
//!   * [super::ICmpInst]s left and right hand side operands have the same [Ty]s.
//!   * [super::SIToFPInst]s:
//!       * Have an integer-typed source operand.
//!       * Have a float-type as the destination type operand.
//!       * Have a destination type operand at least as big as the type of the source operand.
//!   * [Const::Int]s cannot use more bits than the corresponding [Ty::Integer] type.
//!   * [super::Inst] operands refer to values which have been previously defined.

use super::{BinOp, BinOpInst, Const, GuardInst, Inst, InstIdx, Module, Operand, Ty};
use std::collections::{hash_map, HashMap};
use yksmp::Location;

impl Module {
    pub(crate) fn assert_well_formed(&self) {
        if !self.root_entry_vars.is_empty() {
            if self.root_entry_vars.len() != self.loop_jump_vars.len() {
                panic!("Loop start/end variables have different lengths.");
            }
        } else if self.loop_start_vars.len() != self.loop_jump_vars.len() {
            panic!("Loop start/end variables have different lengths.");
        }

        let mut last_inst = None;
        // The set of locations we've seen trace inputs loaded from.
        let mut seen_ti_locs: HashMap<&Location, (InstIdx, &Inst)> = HashMap::new();

        for (iidx, inst) in self.iter_skipping_insts() {
            inst.map_operand_locals(self, &mut |x| {
                if let Inst::Tombstone = self.inst_raw(x) {
                    panic!(
                        "Instruction at position {iidx} uses undefined value (%{x})\n  {}",
                        self.inst_no_copies(iidx).display(iidx, self)
                    );
                }
            });
            match inst {
                Inst::BinOp(BinOpInst { lhs, binop, rhs }) => {
                    let lhs_tyidx = lhs.unpack(self).tyidx(self);
                    if lhs_tyidx != rhs.unpack(self).tyidx(self) {
                        panic!(
                            "Instruction at position {iidx} has different types on lhs and rhs\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self)
                        );
                    }
                    match binop {
                        BinOp::Add
                        | BinOp::Sub
                        | BinOp::Mul
                        | BinOp::Or
                        | BinOp::And
                        | BinOp::Xor
                        | BinOp::Shl
                        | BinOp::AShr
                        | BinOp::LShr
                        | BinOp::SDiv
                        | BinOp::SRem
                        | BinOp::UDiv
                        | BinOp::URem => {
                            if matches!(self.type_(lhs_tyidx), Ty::Float(_)) {
                                panic!(
                                    "Integer binop at position {iidx} operates on float operands\n  {}",
                                    self.inst_no_copies(iidx).display(iidx, self)
                                );
                            }
                        }
                        BinOp::FAdd | BinOp::FDiv | BinOp::FMul | BinOp::FRem | BinOp::FSub => {
                            if !matches!(self.type_(lhs_tyidx), Ty::Float(_)) {
                                panic!(
                                    "Float binop at position {iidx} operates on integer operands\n  {}",
                                    self.inst_no_copies(iidx).display(iidx, self)
                                );
                            }
                        }
                    }
                }
                Inst::Call(x) => {
                    // Check number of parameters/arguments.
                    let fdecl = self.func_decl(x.target());
                    let Ty::Func(fty) = self.type_(fdecl.tyidx()) else {
                        panic!()
                    };
                    if x.num_args() < fty.num_params() {
                        panic!(
                            "Instruction at position {iidx} passing too few arguments:\n  {}",
                            inst.display(iidx, self)
                        );
                    }
                    if x.num_args() > fty.num_params() && !fty.is_vararg() {
                        panic!(
                            "Instruction at position {iidx} passing too many arguments:\n  {}",
                            inst.display(iidx, self)
                        );
                    }

                    // Check parameter/argument types.
                    for (j, (par_ty, arg_ty)) in fty
                        .param_tys()
                        .iter()
                        .zip(x.iter_args_idx().map(|x| self.arg(x).tyidx(self)))
                        .enumerate()
                    {
                        if *par_ty != arg_ty {
                            panic!("Instruction at position {iidx} passing argument {j} of wrong type ({}, but should be {})\n  {}",
                                self.type_(arg_ty).display(self),
                                self.type_(*par_ty).display(self),
                                inst.display(iidx, self));
                        }
                    }
                }
                Inst::Guard(gi @ GuardInst { cond, .. }) => {
                    let cond = cond.unpack(self);
                    let tyidx = cond.tyidx(self);
                    let Ty::Integer(1) = self.type_(tyidx) else {
                        panic!(
                            "Guard at position {iidx} does not have 'cond' of type 'i1'\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self)
                        )
                    };
                    // Check that each live variable appears at most once.
                    //
                    // If the same variable appears more than once, and the guard is side-traced,
                    // then this would lead to distinct variables with the same storage location in
                    // the side-trace and this would upset the register allocator, which assumes
                    // this doesn't happen.
                    use std::collections::HashSet;
                    let mut seen_lvars = HashSet::new();
                    for (_, o) in gi.guard_info(self).live_vars() {
                        if let Operand::Var(oidx) = o.unpack(self) {
                            if seen_lvars.contains(&oidx) {
                                panic!("Guard at position {iidx} has a safepoint that passes the same live variable '%{}' multiple times:\n  {}", oidx, inst.display(iidx, self));
                            }
                            seen_lvars.insert(oidx);
                        }
                    }
                    if let Operand::Const(x) = cond {
                        let Const::Int(_, _v) = self.const_(x) else {
                            unreachable!()
                        };
                        // FIXME: We currently need to break this check due to side-traces being
                        // unfinished and needing to deopt back to the normal interpreter at the
                        // end.
                        // if (*expect && *v == 0) || (!*expect && *v == 1) {
                        //     panic!(
                        //         "Guard at position {iidx} references a constant that is at odds with the guard itself\n  {}",
                        //         self.inst_no_copies(iidx).display(iidx, self)
                        //     );
                        // }
                    }
                }
                Inst::ICmp(x) => {
                    if x.lhs(self).tyidx(self) != x.rhs(self).tyidx(self) {
                        panic!(
                            "Instruction at position {iidx} has different types on lhs and rhs\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self)
                        );
                    }
                }
                Inst::SExt(x) => {
                    let Ty::Integer(val_bitsize) = self.type_(x.val(self).tyidx(self)) else {
                        panic!();
                    };
                    let Ty::Integer(dest_bitsize) = self.type_(x.dest_tyidx()) else {
                        panic!();
                    };
                    if val_bitsize >= dest_bitsize {
                        panic!(
                            "Instruction at position {iidx} trying to sign extend from an equal-or-larger-than integer type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self)
                        );
                    }
                }
                Inst::ZExt(x) => {
                    let val_ty = self.type_(x.val(self).tyidx(self));
                    if !matches!(val_ty, Ty::Integer(_)) && !matches!(val_ty, Ty::Ptr) {
                        panic!(
                            "Instruction at position {iidx} trying to zero extend from a non-integer-or-ptr type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self)
                        );
                    }
                    let val_bitsize = val_ty.bit_size().unwrap();

                    let dest_ty = self.type_(x.dest_tyidx());
                    if !matches!(dest_ty, Ty::Integer(_)) && !matches!(dest_ty, Ty::Ptr) {
                        panic!(
                            "Instruction at position {iidx} trying to zero extend to a non-integer-or-ptr type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self)
                        );
                    }
                    let dest_bitsize = dest_ty.bit_size().unwrap();

                    // FIXME: strictly this should be >= to be in line with LLVM semantics, but the
                    // way we lower LLVM `ptrtoint` to `zext` means that pointer to integer
                    // conversions (i.e. converting from and to the same pointer-size thing) comes
                    // through here. Maybe it was a bad idea to piggy back on `zext` after all.
                    if val_bitsize > dest_bitsize {
                        panic!(
                            "Instruction at position {iidx} trying to zero extend to a smaller integer type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self)
                        );
                    }
                }
                Inst::BitCast(x) => {
                    let val_ty = self.type_(x.val(self).tyidx(self));
                    let dest_ty = self.type_(x.dest_tyidx());
                    // LLVM semantics: "If the source type is a pointer, the destination type must
                    // also be a pointer of the same size"
                    if matches!(val_ty, Ty::Ptr) && !matches!(dest_ty, Ty::Ptr) {
                        panic!(
                            "Instruction at position {iidx} trying to bitcast from a pointer type to a non-pointer type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self)
                        );
                    }
                    // LLVM semantics: "The bit sizes of [source] value and the destination type
                    // must be identical"
                    let val_bitsize = val_ty.bit_size().unwrap();
                    let dest_bitsize = dest_ty.bit_size().unwrap();
                    if val_bitsize != dest_bitsize {
                        panic!(
                            "Instruction at position {iidx} trying to bitcast to a differently-sized type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self)
                        );
                    }
                }
                Inst::Trunc(x) => {
                    let Ty::Integer(val_bitsize) = self.type_(x.val(self).tyidx(self)) else {
                        panic!("Instruction at position {iidx} trying to convert from a non-integer type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self));
                    };
                    let Ty::Integer(dest_bitsize) = self.type_(x.dest_tyidx()) else {
                        panic!("Instruction at position {iidx} trying to convert to a non-integer type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self));
                    };
                    if dest_bitsize >= val_bitsize {
                        panic!(
                            "Instruction at position {iidx} trying to truncate to an equal-or-larger-than type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self)
                        );
                    }
                }
                Inst::SIToFP(x) => {
                    let from_type = self.type_(x.val(self).tyidx(self));
                    let to_type = self.type_(x.dest_tyidx());

                    if !matches!(from_type, Ty::Integer(_)) {
                        panic!("Instruction at position {iidx} trying to convert a non-integer type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self));
                    }
                    if !matches!(to_type, Ty::Float(_)) {
                        panic!("Instruction at position {iidx} trying to convert to a non-float type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self));
                    }
                    if to_type.byte_size() < from_type.byte_size() {
                        panic!("Instruction at position {iidx} trying to convert to a smaller-sized float\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self));
                    }
                }
                Inst::FPExt(x) => {
                    let from_type = self.type_(x.val(self).tyidx(self));
                    let to_type = self.type_(x.dest_tyidx());
                    if !matches!(from_type, Ty::Float(_)) {
                        panic!("Instruction at position {iidx} trying to extend from a non-float type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self));
                    }
                    if !matches!(to_type, Ty::Float(_)) {
                        panic!("Instruction at position {iidx} trying to extend to a non-float type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self));
                    }
                    if to_type.byte_size() <= from_type.byte_size() {
                        panic!("Instruction at position {iidx} trying to extend to a smaller-sized float\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self));
                    }
                }
                Inst::FPToSI(x) => {
                    let from_type = self.type_(x.val(self).tyidx(self));
                    let to_type = self.type_(x.dest_tyidx());

                    if !matches!(from_type, Ty::Float(_)) {
                        panic!("Instruction at position {iidx} trying to convert a non-float type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self));
                    }
                    if !matches!(to_type, Ty::Integer(_)) {
                        panic!("Instruction at position {iidx} trying to convert to a non-integer type\n  {}",
                            self.inst_no_copies(iidx).display(iidx, self));
                    }
                }
                Inst::LoadTraceInput(tii) => {
                    // Check load_ti instructions appear first in the trace.
                    if let Some(i) = last_inst {
                        if !matches!(i, Inst::LoadTraceInput(_)) {
                            panic!("LoadTraceInput instruction may only appear at the beginning of a trace or after another LoadTraceInput instruction\n  {}",
                                self.inst_no_copies(iidx).display(iidx, self));
                        }
                    }
                    // Now check that no other load_ti instruction has already loaded from that
                    // same location. There should be no such aliases.
                    let loc = &self.tilocs[usize::try_from(tii.locidx()).unwrap()];
                    if !matches!(loc, Location::Constant(_)) {
                        match seen_ti_locs.entry(&loc) {
                            hash_map::Entry::Occupied(o) => {
                                let (other_iidx, other_inst) = o.get();
                                panic!(
                                    "load_ti instructions load from the same location:\n  {}\n  {}",
                                    other_inst.display(*other_iidx, self),
                                    inst.display(iidx, self)
                                );
                            }
                            hash_map::Entry::Vacant(v) => {
                                v.insert((iidx, inst));
                            }
                        }
                    }
                }
                _ => (),
            }
            last_inst = Some(*inst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BinOp, BinOpInst, Const, Inst, Module, Operand};

    #[should_panic(expected = "Instruction at position 0 passing too few arguments")]
    #[test]
    fn too_few_args() {
        Module::from_str(
            "
              func_decl f(i32)
              entry:
                call @f()
            ",
        );
    }

    #[should_panic(expected = "Instruction at position 0 passing too few arguments")]
    #[test]
    fn too_few_args2() {
        Module::from_str(
            "
              func_decl f(i32, ...)
              entry:
                call @f()
            ",
        );
    }

    #[should_panic(expected = "Instruction at position 1 passing too many arguments")]
    #[test]
    fn too_many_args() {
        Module::from_str(
            "
              func_decl f()
              entry:
                %0: i8 = load_ti 0
                call @f(%0)
            ",
        );
    }

    #[test]
    fn var_args() {
        Module::from_str(
            "
              func_decl f(...)
              entry:
                %0: i8 = load_ti 0
                call @f(%0)
            ",
        );
    }

    #[should_panic(
        expected = "Instruction at position 1 passing argument 0 of wrong type (i8, but should be i32)"
    )]
    #[test]
    fn cg_call_bad_arg_type() {
        Module::from_str(
            "
              func_decl f(i32) -> i32
              entry:
                %0: i8 = load_ti 0
                %1: i32 = call @f(%0)
            ",
        );
    }

    #[should_panic(expected = "Instruction at position 0 has different types on lhs and rhs")]
    #[test]
    fn cg_add_wrong_types() {
        // The parser will reject a binop with a result type different from either operand, so to
        // get the test we want, we can't use the parser.
        let mut m = Module::new(0, 0).unwrap();
        let c1 = m.insert_const(Const::Int(m.int1_tyidx(), 0)).unwrap();
        let c2 = m.insert_const(Const::Int(m.int8_tyidx(), 0)).unwrap();
        m.push(Inst::BinOp(BinOpInst::new(
            Operand::Const(c1),
            BinOp::Add,
            Operand::Const(c2),
        )))
        .unwrap();
        m.assert_well_formed();
    }

    #[test]
    #[should_panic(expected = "Instruction at position 2 has different types on lhs and rhs")]
    fn cg_icmp_diff_types() {
        Module::from_str(
            "
              entry:
                %0: i8 = load_ti 0
                %1: i64 = load_ti 1
                %2: i1 = eq %0, %1
            ",
        );
    }

    #[test]
    #[should_panic(
        expected = "Instruction at position 1 trying to sign extend from an equal-or-larger-than integer type"
    )]
    fn sign_extend_wrong_size() {
        Module::from_str(
            "
              entry:
                %0: i8 = load_ti 0
                %1: i8 = sext %0
            ",
        );
    }

    #[test]
    #[should_panic(
        expected = "Instruction at position 1 trying to zero extend to a smaller integer type"
    )]
    fn zero_extend_wrong_size() {
        Module::from_str(
            "
              entry:
                %0: i16 = load_ti 0
                %1: i8 = zext %0
            ",
        );
    }

    #[test]
    #[should_panic(
        expected = "Instruction at position 1 trying to zero extend to a non-integer-or-ptr type"
    )]
    fn zero_extend_to_wrong_type() {
        Module::from_str(
            "
              entry:
                %0: i16 = load_ti 0
                %1: float = zext %0
            ",
        );
    }

    #[test]
    #[should_panic(
        expected = "Instruction at position 1 trying to zero extend from a non-integer-or-ptr type"
    )]
    fn zero_extend_from_wrong_type() {
        Module::from_str(
            "
              entry:
                %0: float = load_ti 0
                %1: i64 = zext %0
            ",
        );
    }

    #[test]
    #[should_panic(
        expected = "Instruction at position 1 trying to convert from a non-integer type"
    )]
    fn trunc_from_non_int() {
        Module::from_str(
            "
              entry:
                %0: float = load_ti 0
                %1: i8 = trunc %0
            ",
        );
    }

    #[test]
    #[should_panic(expected = "Instruction at position 1 trying to convert to a non-integer type")]
    fn trunc_to_non_int() {
        Module::from_str(
            "
              entry:
                %0: i64 = load_ti 0
                %1: float = trunc %0
            ",
        );
    }

    #[test]
    #[should_panic(
        expected = "Instruction at position 1 trying to truncate to an equal-or-larger-than type"
    )]
    fn trunc_to_larger() {
        Module::from_str(
            "
              entry:
                %0: i8 = load_ti 0
                %1: i16 = trunc %0
            ",
        );
    }

    #[test]
    #[should_panic(expected = "Guard at position 1 does not have 'cond' of type 'i1'")]
    fn guard_i1() {
        Module::from_str(
            "
              entry:
                %0: i8 = load_ti 0
                guard true, %0, []
            ",
        );
    }

    #[test]
    #[should_panic(expected = "Instruction at position 1 trying to convert a non-integer type")]
    fn si_to_fp_from_non_int() {
        Module::from_str(
            "
              entry:
                %0: float = load_ti 0
                %1: float = si_to_fp %0
            ",
        );
    }

    #[test]
    #[should_panic(expected = "Instruction at position 1 trying to convert to a non-float type")]
    fn si_to_fp_to_non_float() {
        Module::from_str(
            "
              entry:
                %0: i32 = load_ti 0
                %1: i64 = si_to_fp %0
            ",
        );
    }

    #[test]
    #[should_panic(
        expected = "Instruction at position 1 trying to convert to a smaller-sized float"
    )]
    fn si_to_fp_smaller() {
        Module::from_str(
            "
              entry:
                %0: i64 = load_ti 0
                %1: float = si_to_fp %0
            ",
        );
    }

    #[test]
    #[should_panic(
        expected = "Instruction at position 1 trying to extend to a smaller-sized float"
    )]
    fn fp_ext_smaller() {
        Module::from_str(
            "
              entry:
                %0: double = load_ti 0
                %1: float = fp_ext %0
            ",
        );
    }

    #[test]
    #[should_panic(expected = "Instruction at position 1 trying to extend from a non-float type")]
    fn fp_ext_from_non_float() {
        Module::from_str(
            "
              entry:
                %0: i32 = load_ti 0
                %1: double = fp_ext %0
            ",
        );
    }

    #[test]
    #[should_panic(expected = "Instruction at position 1 trying to extend to a non-float type")]
    fn fp_ext_to_non_float() {
        Module::from_str(
            "
              entry:
                %0: float = load_ti 0
                %1: i64 = fp_ext %0
            ",
        );
    }

    #[test]
    #[should_panic(expected = "Instruction at position 1 trying to convert to a non-integer type")]
    fn fp_to_si_to_non_int() {
        Module::from_str(
            "
              entry:
                %0: float = load_ti 0
                %1: float = fp_to_si %0
            ",
        );
    }

    #[test]
    #[should_panic(expected = "Instruction at position 1 trying to convert a non-float type")]
    fn fp_to_si_from_non_float() {
        Module::from_str(
            "
              entry:
                %0: i32 = load_ti 0
                %1: i32 = fp_to_si %0
            ",
        );
    }

    #[test]
    #[should_panic(expected = "Integer binop at position 1 operates on float operands")]
    fn int_binop_with_float_opnds() {
        Module::from_str(
            "
              entry:
                %0: float = load_ti 0
                %1: float = add %0, %0
            ",
        );
    }

    #[test]
    #[should_panic(expected = "Float binop at position 1 operates on integer operands")]
    fn float_binop_with_int_opnds() {
        Module::from_str(
            "
              entry:
                %0: i32 = load_ti 0
                %1: i32 = fadd %0, %0
            ",
        );
    }

    #[test]
    #[should_panic(
        expected = "LoadTraceInput instruction may only appear at the beginning of a trace or after another LoadTraceInput instruction"
    )]
    fn load_ti_invalid() {
        Module::from_str(
            "
              entry:
                %0: i32 = load_ti 0
                %1: i32 = add %0, %0
                %2: i32 = load_ti 1
            ",
        );
    }

    #[test]
    #[should_panic(
        expected = "Instruction at position 1 trying to bitcast from a pointer type to a non-pointer type"
    )]
    fn bitcast_invalid_ptrcast() {
        Module::from_str(
            "
              entry:
                %0: ptr = load_ti 0
                %1: i64 = bitcast %0
            ",
        );
    }

    #[test]
    #[should_panic(
        expected = "Instruction at position 1 trying to bitcast to a differently-sized type"
    )]
    fn bitcast_invalid_bitsize() {
        Module::from_str(
            "
              entry:
                %0: i32 = load_ti 0
                %1: i64 = bitcast %0
            ",
        );
    }

    // FIXME: test would look like this, but the below (valid, I believe) guard doesn't parse for
    // some reason.
    //
    // #[test]
    // #[should_panic(
    //     expected = "xxx"
    // )]
    // fn guard_with_dup_lives() {
    //     Module::from_str(
    //         "
    //           entry:
    //             %0: i1 = load_ti 0
    //             %1: i32 = load_ti 1
    //             guard true, %0, [0:%0_0: %1, 0:%0_1: %1]
    //         ",
    //     );
    // }
}
