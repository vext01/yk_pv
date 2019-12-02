//! Types for the Yorick intermediate language.

use core::yk::SirLoc;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display};

pub type CrateHash = u64;
pub type DefIndex = u32;
pub type BasicBlockIndex = u32;
pub type StatementIndex = usize;
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
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($n), self.val())
            }
        }
    };
}

new_ser128!(SerU128, u128);
new_ser128!(SerI128, i128);

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone, Copy, Hash)]
pub struct Local(pub LocalIndex);

impl Display for Local {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "${}", self.0)
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone, Hash)]
pub struct Place {
    pub base: PlaceBase,
    pub projections: Vec<PlaceProjection>,
}

impl Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.projections.is_empty() {
            write!(f, "{}", self.base)?;
        } else {
            write!(f, "({})", self.base)?;
            for p in &self.projections {
                write!(f, "{}", p)?;
            }
        }
        Ok(())
    }
}

impl From<Local> for Place {
    fn from(l: Local) -> Self {
        Self {
            base: PlaceBase::Local(l),
            projections: Vec::new(),
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone, Hash)]
pub enum PlaceBase {
    Local(Local),
    Static, // FIXME not implemented
}

impl Display for PlaceBase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(l) => write!(f, "{}", l),
            Self::Static => write!(f, "Static"),
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone, Hash)]
pub enum PlaceProjection {
    Field(FieldIndex),
    Unimplemented,
}

impl Display for PlaceProjection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(fld) => write!(f, ".{}", fld),
            Self::Unimplemented => write!(f, "!"),
        }
    }
}

/// A mirror of the compiler's notion of a "definition ID".
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone, Hash)]
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

    /// Creates a DefId from an SirLoc, discarding the block index.
    pub fn from_sir_loc(loc: &SirLoc) -> Self {
        Self::new(loc.crate_hash(), loc.def_idx())
    }
}

impl Display for DefId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DefId({}, {})", self.crate_hash, self.def_idx)
    }
}

/// Bits in the `flags` bitfield in `Body`.
pub mod bodyflags {
    pub const TRACE_HEAD: u8 = 1;
    pub const TRACE_TAIL: u8 = 1 << 1;
}

/// A tracing IR pack.
/// Each Body maps to exactly one MIR Body.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct Body {
    pub def_id: DefId,
    pub def_path_str: String,
    pub blocks: Vec<BasicBlock>,
    /// The number of arguments to the function.
    pub num_args: usize,
    /// The number of local variables used by the function, including the return value and
    /// arguments.
    pub num_locals: usize,
    pub flags: u8,
}

impl Display for Body {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "[Begin SIR for {}]", self.def_path_str)?;
        writeln!(f, "    {}:", self.def_id)?;
        let mut block_strs = Vec::new();
        for (i, b) in self.blocks.iter().enumerate() {
            block_strs.push(format!("    bb{}:\n{}", i, b));
        }
        writeln!(f, "{}", block_strs.join("\n"))?;
        writeln!(f, "[End SIR for {}]", self.def_path_str)?;
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    /// An assignment.
    Assign(Place, Rvalue),
    /// Any unimplemented lowering maps to this variant.
    /// The string inside is the stringified MIR statement.
    Unimplemented(String),
}

impl Display for Statement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Statement::Nop => write!(f, "nop"),
            Statement::Assign(l, r) => write!(f, "{} = {}", l, r),
            Statement::Unimplemented(mir_stmt) => write!(f, "unimplemented_stmt: {}", mir_stmt),
        }
    }
}

/// The right-hand side of an assignment.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum Rvalue {
    Use(Operand),
    BinaryOp(BinOp, Operand, Operand),
    CheckedBinaryOp(BinOp, Operand, Operand),
    Unimplemented,
}

impl Display for Rvalue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Use(p) => write!(f, "{}", p),
            Self::BinaryOp(op, oper1, oper2) => write!(f, "{}({}, {})", op, oper1, oper2),
            Self::CheckedBinaryOp(op, oper1, oper2) => {
                write!(f, "checked_{}({}, {})", op, oper1, oper2)
            }
            Self::Unimplemented => write!(f, "unimplemented rvalue"),
        }
    }
}

impl From<Local> for Rvalue {
    fn from(l: Local) -> Self {
        Self::Use(Operand::from(l))
    }
}

/// Unlike in MIR, we don't track move/copy semantics in operands.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum Operand {
    Place(Place),
    Constant(Constant),
}

impl Display for Operand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operand::Place(p) => write!(f, "{}", p),
            Operand::Constant(c) => write!(f, "{}", c),
        }
    }
}

impl From<Local> for Operand {
    fn from(l: Local) -> Self {
        Operand::Place(Place::from(l))
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum Constant {
    Int(ConstantInt),
    Unimplemented,
}

impl Display for Constant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

impl From<bool> for ConstantInt {
    fn from(b: bool) -> Self {
        if b {
            ConstantInt::UnsignedInt(UnsignedInt::Usize(1))
        } else {
            ConstantInt::UnsignedInt(UnsignedInt::Usize(0))
        }
    }
}

/// Generate a method that constructs a ConstantInt variant from bits in u128 form.
/// This can't be used to generate methods for 128-bit integers due to SerU128/SerI128.
macro_rules! const_int_from_bits {
    ($fn_name: ident, $rs_t: ident, $yk_t: ident, $yk_variant: ident) => {
        pub fn $fn_name(bits: u128) -> Self {
            ConstantInt::$yk_t($yk_t::$yk_variant(bits as $rs_t))
        }
    };
}

impl ConstantInt {
    const_int_from_bits!(u8_from_bits, u8, UnsignedInt, U8);
    const_int_from_bits!(u16_from_bits, u16, UnsignedInt, U16);
    const_int_from_bits!(u32_from_bits, u32, UnsignedInt, U32);
    const_int_from_bits!(u64_from_bits, u64, UnsignedInt, U64);
    const_int_from_bits!(usize_from_bits, usize, UnsignedInt, Usize);

    pub fn u128_from_bits(bits: u128) -> Self {
        ConstantInt::UnsignedInt(UnsignedInt::U128(SerU128::new(bits)))
    }

    const_int_from_bits!(i8_from_bits, i8, SignedInt, I8);
    const_int_from_bits!(i16_from_bits, i16, SignedInt, I16);
    const_int_from_bits!(i32_from_bits, i32, SignedInt, I32);
    const_int_from_bits!(i64_from_bits, i64, SignedInt, I64);
    const_int_from_bits!(isize_from_bits, isize, SignedInt, Isize);

    pub fn i128_from_bits(bits: u128) -> Self {
        ConstantInt::SignedInt(SignedInt::I128(SerI128::new(bits as i128)))
    }
}

impl Display for ConstantInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// A call target.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum CallOperand {
    /// A statically known function identified by its DefId.
    /// A pair: the definition ID and the binary symbol name, if known. If the callee doeesn't have
    /// all of its type parameters instantiated, then there will be no symbol.
    Fn(DefId, Option<String>),
    /// A dynamic call via a vtable,
    /// A pair like the Fn variant.
    Virtual(DefId, Option<String>),
    /// An unknown or unhandled callable.
    Unknown, // FIXME -- Find out what else. Closures jump to mind.
}

fn sym_name_str(maybe_name: Option<&str>) -> &str {
    maybe_name.map_or_else(|| "<unknown>", |n| n)
}

impl Display for CallOperand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallOperand::Fn(def_id, sym_name) => write!(
                f,
                "Fn(def_id={}, sym_name={})",
                def_id,
                sym_name_str(sym_name.as_ref().map(|s| &**s)),
            ),
            CallOperand::Virtual(def_id, sym_name) => write!(
                f,
                "Virtual(def_id={}, sym_name={})",
                def_id,
                sym_name_str(sym_name.as_ref().map(|s| &**s)),
            ),
            CallOperand::Unknown => write!(f, "unknown"),
        }
    }
}

/// A basic block terminator.
/// Note that we assume an the abort strategy, so there are no unwind or cleanup edges present.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum Terminator {
    Goto(BasicBlockIndex),
    SwitchInt {
        discr: Place,
        values: Vec<SerU128>,
        target_bbs: Vec<BasicBlockIndex>,
        otherwise_bb: BasicBlockIndex,
    },
    Return,
    Unreachable,
    Drop {
        location: Place,
        target_bb: BasicBlockIndex,
    },
    DropAndReplace {
        location: Place,
        target_bb: BasicBlockIndex,
        value: Operand,
    },
    Call {
        operand: CallOperand,
        ret_bb: Option<BasicBlockIndex>,
    },
    /// The value in `cond` must equal to `expected` to advance to `target_bb`.
    Assert {
        cond: Place,
        expected: bool,
        target_bb: BasicBlockIndex,
    },
    Unimplemented(String), // FIXME will eventually disappear.
}

impl Display for Terminator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Terminator::Goto(bb) => write!(f, "goto bb{}", bb),
            Terminator::SwitchInt {
                discr,
                values,
                target_bbs,
                otherwise_bb,
            } => write!(
                f,
                "switch_int local={}, vals=[{}], targets=[{}], otherwise={}",
                discr,
                values
                    .iter()
                    .map(|b| format!("{}", b))
                    .collect::<Vec<String>>()
                    .join(", "),
                target_bbs
                    .iter()
                    .map(|b| format!("{}", b))
                    .collect::<Vec<String>>()
                    .join(", "),
                otherwise_bb
            ),
            Terminator::Return => write!(f, "return"),
            Terminator::Unreachable => write!(f, "unreachable"),
            Terminator::Drop {
                location,
                target_bb,
            } => write!(f, "drop loc={}, target=bb{}", target_bb, location,),
            Terminator::DropAndReplace {
                location,
                value,
                target_bb,
            } => write!(
                f,
                "drop_and_replace loc={}, value={}, target=bb{}",
                location, value, target_bb,
            ),
            Terminator::Call { operand, ret_bb } => write!(
                f,
                "call operand={}, ret_bb={}",
                operand,
                opt_bb_as_str(ret_bb)
            ),
            Terminator::Assert {
                cond,
                target_bb,
                expected,
            } => write!(
                f,
                "assert cond={}, expected={}, target=bb{}",
                cond, target_bb, expected
            ),
            Terminator::Unimplemented(s) => write!(f, "unimplemented: {}", s),
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

/// A debugging entry, mapping a DefId to its definition path string.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub struct SirDebug {
    def_id: DefId,
    def_path: String,
}

impl SirDebug {
    pub fn new(def_id: DefId, def_path: String) -> Self {
        Self { def_id, def_path }
    }

    pub fn def_id(&self) -> &DefId {
        &self.def_id
    }

    pub fn def_path(&self) -> &str {
        &self.def_path
    }
}

/// The top-level pack type.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Clone)]
pub enum Pack {
    Body(Body),
    Debug(SirDebug),
}

impl Display for SirDebug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<debug: def_id={}, def_path_str={}>",
            self.def_id, self.def_path
        )
    }
}

impl Display for Pack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pack::Body(sir) => write!(f, "{}", sir),
            Pack::Debug(dbg) => write!(f, "{}", dbg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConstantInt, SerI128, SerU128, SignedInt, UnsignedInt};

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

    #[test]
    fn const_u8_from_bits() {
        let v = 233;
        let cst = ConstantInt::u8_from_bits(v as u128);
        assert_eq!(cst, ConstantInt::UnsignedInt(UnsignedInt::U8(v)));
    }

    #[test]
    fn const_i32_from_bits() {
        let v = -42i32;
        let cst = ConstantInt::i32_from_bits(v as u128);
        assert_eq!(cst, ConstantInt::SignedInt(SignedInt::I32(v)));
    }

    #[test]
    fn const_u64_from_bits() {
        let v = std::u64::MAX;
        let cst = ConstantInt::u64_from_bits(v as u128);
        assert_eq!(cst, ConstantInt::UnsignedInt(UnsignedInt::U64(v)));
    }

    #[test]
    fn const_i128_from_bits() {
        let v = -100001i128;
        let cst = ConstantInt::i128_from_bits(v as u128);
        match &cst {
            ConstantInt::SignedInt(SignedInt::I128(seri128)) => assert_eq!(seri128.val(), v),
            _ => panic!(),
        }
    }
}
