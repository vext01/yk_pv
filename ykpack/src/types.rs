// Copyright 2019 King's College London.
// Created by the Software Development Team <http://soft-dev.org/>.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Types for the Yorick intermediate language.

use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};

pub type CrateHash = u64;
pub type DefIndex = u32;
pub type BasicBlockIndex = u32;
pub type LocalIndex = u32;
pub type TyIndex = u32;
pub type FieldIndex = u32;

/// rmp-serde serialisable 128-bit numeric types, to work around:
/// https://github.com/3Hren/msgpack-rust/issues/169
macro_rules! new_ser128 {
    ($n: ident, $t: ty) => {
        #[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
        pub struct $n {
            hi: u64,
            lo: u64,
        }

        impl $n {
            pub fn new(val: $t) -> Self {
                Self {
                    hi: (val >> 64) as u64,
                    lo: val as u64,
                }
            }

            pub fn val(&self) -> $t {
                (self.hi as $t) << 64 | self.lo as $t
            }
        }

        impl Display for $n {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, "{}({})", stringify!($n), self.val())
            }
        }
    };
}

new_ser128!(SerU128, u128);
new_ser128!(SerI128, i128);

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone, Copy)]
pub struct Local {
    idx: LocalIndex,
    ty: TyIndex,
}

impl Local {
    pub fn new(idx: LocalIndex, ty: TyIndex) -> Self {
        Self { idx, ty }
    }

    pub fn idx(&self) -> LocalIndex {
        self.idx
    }

    pub fn ty(&self) -> TyIndex {
        self.ty
    }
}

impl Display for Local {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "${}: t{}", self.idx, self.ty)
    }
}

/// A mirror of the compiler's notion of a "definition ID".
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct DefId {
    pub crate_hash: CrateHash,
    pub def_idx: DefIndex,
}

impl DefId {
    pub fn new(crate_hash: CrateHash, def_idx: DefIndex) -> Self {
        Self {
            crate_hash,
            def_idx,
        }
    }
}

impl Display for DefId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "DefId({}, {})", self.crate_hash, self.def_idx)
    }
}

/// A tracing IR pack.
/// Each TIR instance maps to exactly one MIR instance.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct Tir {
    pub def_id: DefId,
    pub item_path_str: String,
    pub blocks: Vec<BasicBlock>,
}

impl Tir {
    pub fn new(def_id: DefId, item_path_str: String, blocks: Vec<BasicBlock>) -> Self {
        Self {
            def_id,
            item_path_str,
            blocks,
        }
    }
}

impl Display for Tir {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "[Begin TIR for {}]", self.item_path_str)?;
        writeln!(f, "    {}:", self.def_id)?;
        let mut block_strs = Vec::new();
        for (i, b) in self.blocks.iter().enumerate() {
            block_strs.push(format!("    bb{}:\n{}", i, b));
        }
        println!("{:?}", block_strs);
        writeln!(f, "{}", block_strs.join("\n"))?;
        writeln!(f, "[End TIR for {}]", self.item_path_str)?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct BasicBlock {
    pub stmts: Vec<Statement>,
    pub term: Terminator,
}

impl BasicBlock {
    pub fn new(stmts: Vec<Statement>, term: Terminator) -> Self {
        Self { stmts, term }
    }
}

impl Display for BasicBlock {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for s in self.stmts.iter() {
            write!(f, "        {}\n", s)?;
        }
        write!(f, "        {}", self.term)
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum Statement {
    /// Do nothing.
    Nop,
    /// An assignment to a local variable.
    Assign(Local, Rvalue),
    /// Store into the memory.
    Store(Local, Operand),
    /// Any unimplemented lowering maps to this variant.
    /// The string inside is the stringified MIR statement.
    Unimplemented(String),
}

impl Display for Statement {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Statement::Nop => write!(f, "nop"),
            Statement::Assign(l, r) => write!(f, "{} = {}", l, r),
            Statement::Store(ptr, val) => write!(f, "store({}, {})", ptr, val),
            Statement::Unimplemented(mir_stmt) => write!(f, "unimplemented_stmt: {}", mir_stmt),
        }
    }
}

/// The right-hand side of an assignment.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum Rvalue {
    /// Another local variable.
    Local(Local),
    /// A constant value.
    Constant(Constant),
    /// Get a pointer to a field.
    GetField(Local, FieldIndex),
    /// Load a value of specified type from a pointer.
    Load(Local),
    /// Nullary, Unary and Binary Ops.
    BinaryOp(BinOp, Operand, Operand),
    /// Allocate space for the specified type on the stack and return a pointer to it.
    Alloca(TyIndex),
}

impl Display for Rvalue {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Rvalue::Local(l) => write!(f, "{}", l),
            Rvalue::Constant(c) => write!(f, "{}", c),
            Rvalue::GetField(ptr, fidx) => write!(f, "get_field({}, {})", ptr, fidx),
            Rvalue::Load(l) => write!(f, "load({})", l),
            Rvalue::BinaryOp(oper, o1, o2) => write!(f, "{}({}, {})", oper, o1, o2),
            Rvalue::Alloca(t) => write!(f, "alloca({})", t),
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum Operand {
    Local(Local),
    Constant(Constant),
}

impl Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Operand::Local(l) => write!(f, "{}", l),
            Operand::Constant(c) => write!(f, "{}", c),
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum Constant {
    Int(ConstantInt),
    Unimplemented,
}

impl Display for Constant {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Constant::Int(i) => write!(f, "{}", i),
            Constant::Unimplemented => write!(f, "Unimplemented Constant"),
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum ConstantInt {
    UnsignedInt(UnsignedInt),
    SignedInt(SignedInt),
}

impl ConstantInt {
    // When constructing constant integers, truncation is deliberate.
    pub fn u8_from_bits(bits: u128) -> Self {
        ConstantInt::UnsignedInt(UnsignedInt::U8(bits as u8))
    }

    pub fn u16_from_bits(bits: u128) -> Self {
        ConstantInt::UnsignedInt(UnsignedInt::U16(bits as u16))
    }

    pub fn u32_from_bits(bits: u128) -> Self {
        ConstantInt::UnsignedInt(UnsignedInt::U32(bits as u32))
    }

    pub fn u64_from_bits(bits: u128) -> Self {
        ConstantInt::UnsignedInt(UnsignedInt::U64(bits as u64))
    }

    pub fn u128_from_bits(bits: u128) -> Self {
        ConstantInt::UnsignedInt(UnsignedInt::U128(SerU128::new(bits)))
    }

    pub fn usize_from_bits(bits: u128) -> Self {
        ConstantInt::UnsignedInt(UnsignedInt::Usize(bits as usize))
    }

    pub fn i8_from_bits(bits: u128) -> Self {
        ConstantInt::SignedInt(SignedInt::I8(bits as i8))
    }

    pub fn i16_from_bits(bits: u128) -> Self {
        ConstantInt::SignedInt(SignedInt::I16(bits as i16))
    }

    pub fn i32_from_bits(bits: u128) -> Self {
        ConstantInt::SignedInt(SignedInt::I32(bits as i32))
    }

    pub fn i64_from_bits(bits: u128) -> Self {
        ConstantInt::SignedInt(SignedInt::I64(bits as i64))
    }

    pub fn i128_from_bits(bits: u128) -> Self {
        ConstantInt::SignedInt(SignedInt::I128(SerI128::new(bits as i128)))
    }

    pub fn isize_from_bits(bits: u128) -> Self {
        ConstantInt::SignedInt(SignedInt::Isize(bits as isize))
    }
}

impl Display for ConstantInt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ConstantInt::UnsignedInt(u) => write!(f, "{}", u),
            ConstantInt::SignedInt(s) => write!(f, "{}", s),
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum UnsignedInt {
    Usize(usize),
    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),
    U128(SerU128),
}

impl Display for UnsignedInt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum SignedInt {
    Isize(isize),
    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),
    I128(SerI128),
}

impl Display for SignedInt {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// A call target.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum CallOperand {
    /// A statically known function identified by its DefId.
    Fn(DefId),
    /// An unknown or unhandled callable.
    Unknown, // FIXME -- Find out what else. Closures jump to mind.
}

impl Display for CallOperand {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CallOperand::Fn(def_id) => write!(f, "{}", def_id),
            CallOperand::Unknown => write!(f, "unknown"),
        }
    }
}

/// A basic block terminator.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum Terminator {
    Goto(BasicBlockIndex),
    SwitchInt {
        local: Local,
        values: Vec<SerU128>,
        target_bbs: Vec<BasicBlockIndex>,
    },
    Resume,
    Abort,
    Return,
    Unreachable,
    Drop {
        target_bb: BasicBlockIndex,
        unwind_bb: Option<BasicBlockIndex>,
    },
    DropAndReplace {
        target_bb: BasicBlockIndex,
        unwind_bb: Option<BasicBlockIndex>,
    },
    Call {
        operand: CallOperand,
        cleanup_bb: Option<BasicBlockIndex>,
        ret_bb: Option<BasicBlockIndex>,
    },
    Assert {
        target_bb: BasicBlockIndex,
        cleanup_bb: Option<BasicBlockIndex>,
    },
    Unimplemented, // FIXME will eventually disappear.
}

impl Display for Terminator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Terminator::Goto(bb) => write!(f, "goto bb{}", bb),
            Terminator::SwitchInt {
                local,
                values,
                target_bbs,
            } => write!(
                f,
                "switch_int local={}, vals=[{}], targets=[{}]",
                local,
                values
                    .iter()
                    .map(|b| format!("{}", b))
                    .collect::<Vec<String>>()
                    .join(", "),
                target_bbs
                    .iter()
                    .map(|b| format!("{}", b))
                    .collect::<Vec<String>>()
                    .join(", ")
            ),
            Terminator::Resume => write!(f, "resume"),
            Terminator::Abort => write!(f, "abort"),
            Terminator::Return => write!(f, "return"),
            Terminator::Unreachable => write!(f, "unreachable"),
            Terminator::Drop {
                target_bb,
                unwind_bb,
            } => write!(
                f,
                "drop target=bb{}, unwind={}",
                target_bb,
                opt_bb_as_str(unwind_bb)
            ),
            Terminator::DropAndReplace {
                target_bb,
                unwind_bb,
            } => write!(
                f,
                "drop_and_replace target=bb{}, unwind={}",
                target_bb,
                opt_bb_as_str(unwind_bb)
            ),
            Terminator::Call {
                operand,
                cleanup_bb,
                ret_bb,
            } => write!(
                f,
                "call target={}, cleanup={}, return_to={}",
                operand,
                opt_bb_as_str(cleanup_bb),
                opt_bb_as_str(ret_bb)
            ),
            Terminator::Assert {
                target_bb,
                cleanup_bb,
            } => write!(
                f,
                "assert target=bb{}, cleanup={}",
                target_bb,
                opt_bb_as_str(cleanup_bb)
            ),
            Terminator::Unimplemented => write!(f, "unimplemented"),
        }
    }
}

fn opt_bb_as_str(opt_bb: &Option<BasicBlockIndex>) -> String {
    match opt_bb {
        Some(bb) => format!("bb{}", bb),
        _ => String::from("none"),
    }
}

/// Binary operations.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    BitXor,
    BitAnd,
    BitOr,
    Shl,
    Shr,
    Eq,
    Lt,
    Le,
    Ne,
    Ge,
    Gt,
    Offset,
}

impl Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            BinOp::Add => "add",
            BinOp::Sub => "sub",
            BinOp::Mul => "mul",
            BinOp::Div => "div",
            BinOp::Rem => "rem",
            BinOp::BitXor => "bit_xor",
            BinOp::BitAnd => "bit_and",
            BinOp::BitOr => "bit_or",
            BinOp::Shl => "shl",
            BinOp::Shr => "shr",
            BinOp::Eq => "eq",
            BinOp::Lt => "lt",
            BinOp::Le => "le",
            BinOp::Ne => "ne",
            BinOp::Ge => "ge",
            BinOp::Gt => "gt",
            BinOp::Offset => "offset",
        };
        write!(f, "{}", s)
    }
}

/// The top-level pack type.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum Pack {
    Tir(Tir),
}

impl Display for Pack {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let Pack::Tir(tir) = self;
        write!(f, "{}", tir)
    }
}

#[cfg(test)]
mod tests {
    use super::{SerI128, SerU128};

    #[test]
    fn seru128_round_trip() {
        let val: u128 = std::u128::MAX - 427819;
        assert_eq!(SerU128::new(val).val(), val);
    }

    #[test]
    fn seri128_round_trip() {
        let val = std::i128::MIN + 77;
        assert_eq!(SerI128::new(val).val(), val);
    }
}
