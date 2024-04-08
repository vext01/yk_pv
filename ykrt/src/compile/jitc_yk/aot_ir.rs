//! Yk's AOT IR deserialiser.
//!
//! This module contains the data structures for the AOT IR and a parser to read it from its
//! serialised format.
//!
//! The AOT IR accurately reflects the structure and semantics of the AOT binary. As such, it must
//! not be mutated after the fact.
//!
//! The IR is index-centric, meaning that when one data structure refers to another, it is by a
//! numeric index into a backing vector (and not via a Rust reference). We chose to do it like this
//! because a) references can't easily be serialised and deserialised; and b) we didn't want to do
//! another pass over the IR to convert to another version of the data structures that uses
//! references.
//!
//! Each kind of index has a distinct Rust type so that it cannot be accidentally used in place of
//! an unrelated index. This is enforced by `TiVec`.
//!
//! At a high level, the AOT IR contains:
//!  - Functions (which contain basic blocks, which contain individual instructions).
//!  - Global variable declarations.
//!  - Function definitions/declarations.
//!  - Constant values.
//!  - Types, for use by all of the above.
//!
//! Throughout we use the term "definition" to mean something for which we have total IR knowledge
//! of, whereas a "declaration" is something compiled externally that we typically only know the
//! symbol name, address and type of.
//!
//! Elements of the IR can be converted to human-readable forms by calling `to_string()` on them.
//! This is used for testing, but can also be used for debugging.

use byteorder::{NativeEndian, ReadBytesExt};
use deku::prelude::*;
use std::{cell::RefCell, error::Error, ffi::CString, fs, path::PathBuf};
use typed_index_collections::TiVec;

/// A magic number that all bytecode payloads begin with.
const MAGIC: u32 = 0xedd5f00d;
/// The version of the bytecode format.
const FORMAT_VERSION: u32 = 0;

/// The symbol name of the control point function (after ykllvm has transformed it).
const CONTROL_POINT_NAME: &str = "__ykrt_control_point";
const STACKMAP_CALL_NAME: &str = "llvm.experimental.stackmap";
const LLVM_DEBUG_CALL_NAME: &str = "llvm.dbg.value";

// Generate common methods for index types.
macro_rules! index {
    ($struct:ident) => {
        impl $struct {
            #[allow(dead_code)] // FIXME: remove when constants and func args are implemented.
            pub(crate) fn new(v: usize) -> Self {
                Self(v)
            }
        }

        impl From<usize> for $struct {
            fn from(idx: usize) -> Self {
                Self(idx)
            }
        }

        impl From<$struct> for usize {
            fn from(s: $struct) -> usize {
                s.0
            }
        }
    };
}

/// An index into [Module::funcs].
#[deku_derive(DekuRead)]
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub(crate) struct FuncIdx(usize);
index!(FuncIdx);

/// An index into [Module::types].
#[deku_derive(DekuRead)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TypeIdx(usize);
index!(TypeIdx);

/// An index into [Func::bblocks].
#[deku_derive(DekuRead)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct BBlockIdx(usize);
index!(BBlockIdx);

/// An index into [BBlock::instrs].
#[deku_derive(DekuRead)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct InstrIdx(usize);
index!(InstrIdx);

/// An index into [Module::consts].
#[deku_derive(DekuRead)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ConstIdx(usize);
index!(ConstIdx);

/// An index into [Module::global_decls].
///
/// Note: these are "declarations" and not "definitions" because they all been AOT code-generated
/// already, and thus come "pre-initialised".
#[deku_derive(DekuRead)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct GlobalDeclIdx(usize);
index!(GlobalDeclIdx);

/// An index into [FuncType::arg_ty_idxs].
#[deku_derive(DekuRead)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ArgIdx(usize);
index!(ArgIdx);

/// Helper function for deku `map` attribute. It is necessary to write all the types out in full to
/// avoid type inference errors, so it's easier to have a single helper function rather than inline
/// this into each `map` attribute.
fn map_to_string(v: Vec<u8>) -> Result<String, DekuError> {
    if let Ok(x) = CString::from_vec_with_nul(v) {
        if let Ok(x) = x.into_string() {
            return Ok(x);
        }
    }
    Err(DekuError::Parse("Couldn't map string".to_owned()))
}

/// Helper function for deku `map` attribute. It is necessary to write all the types out in full to
/// avoid type inference errors, so it's easier to have a single helper function rather than inline
/// this into each `map` attribute.
fn map_to_tivec<I, T>(v: Vec<T>) -> Result<TiVec<I, T>, DekuError> {
    Ok(TiVec::from(v))
}

/// A trait for converting in-memory data-structures into a human-readable textual format.
///
/// This is analogous to [std::fmt::Display], but:
///   1. Takes a reference to a [Module] so that constructs that require lookups into the module's
///      tables from stringification have access to them.
///   2. Returns a [String], for ease of use.
pub(crate) trait AotIRDisplay {
    /// Return a human-readable string.
    fn to_string(&self, m: &Module) -> String;

    /// Print myself to stderr in human-readable form.
    ///
    /// This isn't used during normal operation of the system: it is provided as a debugging aid.
    #[allow(dead_code)]
    fn dump(&self, m: &Module) {
        eprintln!("{}", self.to_string(m));
    }
}

/// An instruction opcode.
#[deku_derive(DekuRead)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[deku(type = "u8")]
pub(crate) enum Opcode {
    Nop = 0,
    Load,
    Store,
    Alloca,
    Call,
    Br,
    CondBr,
    Icmp,
    BinaryOperator,
    Ret,
    InsertValue,
    PtrAdd,
    Add,
    Sub,
    Mul,
    Or,
    And,
    Xor,
    Shl,
    AShr,
    FAdd,
    FDiv,
    FMul,
    FRem,
    FSub,
    LShr,
    SDiv,
    SRem,
    UDiv,
    URem,
    Unimplemented = 255,
}

impl AotIRDisplay for Opcode {
    fn to_string(&self, _m: &Module) -> String {
        format!("{:?}", self).to_lowercase()
    }
}

/// Uniquely identifies an instruction within a [Module].
#[deku_derive(DekuRead)]
#[derive(Clone, Debug, Hash, Eq, PartialEq)]
pub(crate) struct InstructionID {
    #[deku(skip)] // computed after deserialisation.
    func_idx: FuncIdx,
    bb_idx: BBlockIdx,
    inst_idx: InstrIdx,
}

impl InstructionID {
    pub(crate) fn new(func_idx: FuncIdx, bb_idx: BBlockIdx, inst_idx: InstrIdx) -> Self {
        Self {
            func_idx,
            bb_idx,
            inst_idx,
        }
    }
}

/// Uniquely identifies a basic block within a [Func].
#[derive(Debug, PartialEq)]
pub(crate) struct BBlockId {
    func_idx: FuncIdx,
    bb_idx: BBlockIdx,
}

impl BBlockId {
    pub(crate) fn new(func_idx: FuncIdx, bb_idx: BBlockIdx) -> Self {
        Self { func_idx, bb_idx }
    }

    pub(crate) fn func_idx(&self) -> FuncIdx {
        self.func_idx
    }

    pub(crate) fn bb_idx(&self) -> BBlockIdx {
        self.bb_idx
    }

    pub(crate) fn is_entry(&self) -> bool {
        self.bb_idx == BBlockIdx(0)
    }
}

/// Predicates for use in numeric comparisons.
#[deku_derive(DekuRead)]
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
#[deku(type = "u8")]
pub(crate) enum Predicate {
    Equal = 0,
    NotEqual,
    UnsignedGreater,
    UnsignedGreaterEqual,
    UnsignedLess,
    UnsignedLessEqual,
    SignedGreater,
    SignedGreaterEqual,
    SignedLess,
    SignedLessEqual,
    // FIXME: add floating-point-specific predicates.
}

impl AotIRDisplay for Predicate {
    fn to_string(&self, _m: &Module) -> String {
        format!("{:?}", self)
    }
}

const OPKIND_CONST: u8 = 0;
const OPKIND_LOCAL_VARIABLE: u8 = 1;
const OPKIND_TYPE: u8 = 2;
const OPKIND_FUNC: u8 = 3;
const OPKIND_BLOCK: u8 = 4;
const OPKIND_ARG: u8 = 5;
const OPKIND_GLOBAL: u8 = 6;
const OPKIND_PREDICATE: u8 = 7;
const OPKIND_UNIMPLEMENTED: u8 = 255;

#[deku_derive(DekuRead)]
#[derive(Debug)]
#[deku(type = "u8")]
pub(crate) enum Operand {
    #[deku(id = "OPKIND_CONST")]
    Constant(ConstIdx),
    #[deku(id = "OPKIND_LOCAL_VARIABLE")]
    LocalVariable(InstructionID),
}

impl Operand {
    /// For a [Self::LocalVariable] operand return the instruction that defines the variable.
    ///
    /// Panics for other kinds of operand.
    ///
    /// OPT: This is expensive.
    pub(crate) fn to_instr<'a>(&self, aotmod: &'a Module) -> &'a Instruction {
        match self {
            Self::LocalVariable(iid) => {
                &aotmod.funcs[iid.func_idx].bblocks[iid.bb_idx].instrs[iid.inst_idx]
            }
            _ => panic!(),
        }
    }

    /// Returns the [Type] of the operand.
    pub(crate) fn type_<'a>(&self, m: &'a Module) -> &'a Type {
        match self {
            Self::LocalVariable(_) => {
                // The `unwrap` can't fail for a `LocalVariable`.
                self.to_instr(m).def_type(m).unwrap()
            }
            Self::Type(type_idx) => m.type_(*type_idx),
            _ => todo!(),
        }
    }

    /// Return the `InstructionID` of a local variable operand. Panics if called on other kinds of
    /// operands.
    pub(crate) fn to_instr_id(&self) -> InstructionID {
        match self {
            Self::LocalVariable(iid) => iid.clone(),
            _ => panic!(),
        }
    }
}

impl AotIRDisplay for Operand {
    fn to_string(&self, m: &Module) -> String {
        match self {
            Self::Constant(const_idx) => m.consts[*const_idx].to_string(m),
            Self::LocalVariable(iid) => {
                format!("${}_{}", usize::from(iid.bb_idx), usize::from(iid.inst_idx))
            }
            Self::Type(type_idx) => m.types[*type_idx].to_string(m),
            Self::Func(func_idx) => m.funcs[*func_idx].name.to_owned(),
            Self::BBlock(bb_idx) => format!("bb{}", usize::from(*bb_idx)),
            Self::Arg(arg_idx) => format!("$arg{}", usize::from(*arg_idx)),
            Self::Global(gd_idx) => m.global_decls[*gd_idx].to_string(m),
            Self::Predicate(p) => p.to_string(m),
            Self::Unimplemented(s) => format!("?op<{}>", s),
        }
    }
}

/// An instruction.
/// FIXME: tell deku
enum Instruction {
    Nop,
    Load(Operand), // instructions with one (or maybe two) fields can be tuple-like?
    Store {
        what: Operand,
        where_: Operand,
    }, // more involved instrs can be struct-like with named fields?
    Alloca(TypeIdx),
    Call {
        target: FuncDecl,
        first_arg: Operand,
        extra_args: ExtraArgIdx,
    },
    Br(BBlockIdx),
    // ... give all the other opcodes below the same treatment.
    // CondBr,
    // Icmp,
    // BinaryOperator,
    // Ret,
    // InsertValue,
    // PtrAdd,
    // Add,
    // Sub,
    // Mul,
    // Or,
    // And,
    // Xor,
    // Shl,
    // AShr,
    // FAdd,
    // FDiv,
    // FMul,
    // FRem,
    // FSub,
    // LShr,
    // SDiv,
    // SRem,
    // UDiv,
    // URem,
}

impl Instruction {
    /// Returns the operand at the specified index. Panics if the index is out of bounds.
    pub(crate) fn operand(&self, idx: usize) -> &Operand {
        &self.operands[idx]
    }

    /// Return a slice of the remaining operands, starting from the index `from` (inclusive).
    pub(crate) fn remaining_operands(&self, from: usize) -> &[Operand] {
        &self.operands[from..]
    }

    pub(crate) fn opcode(&self) -> Opcode {
        self.opcode
    }

    /// For a call instruction, return the callee.
    ///
    /// # Panics
    ///
    /// Panics if the instruction isn't a call instruction.
    pub(crate) fn callee(&self) -> FuncIdx {
        debug_assert!(matches!(self.opcode, Opcode::Call));
        let op = self.operand(0);
        match op {
            Operand::Func(func_idx) => *func_idx,
            _ => panic!(),
        }
    }

    pub(crate) fn type_idx(&self) -> TypeIdx {
        self.type_idx
    }

    /// Returns the [Type] of the local variable defined by this instruction or `None` if this
    /// instruction does not define a new local variable.
    pub(crate) fn def_type<'a>(&self, m: &'a Module) -> Option<&'a Type> {
        if m.instr_type(self) != &Type::Void {
            Some(m.type_(self.type_idx))
        } else {
            None
        }
    }

    pub(crate) fn is_store(&self) -> bool {
        self.opcode == Opcode::Store
    }

    pub(crate) fn is_ptr_add(&self) -> bool {
        self.opcode == Opcode::PtrAdd
    }

    pub(crate) fn is_mappable_call(&self, aot_mod: &Module) -> bool {
        if self.opcode == Opcode::Call {
            let op = &self.operands[0];
            match op {
                Operand::Func(func_idx) => !aot_mod.funcs[*func_idx].is_declaration(),
                _ => false,
            }
        } else {
            false
        }
    }

    pub(crate) fn is_control_point(&self, aot_mod: &Module) -> bool {
        if self.opcode == Opcode::Call {
            // Call instructions always have at least one operand (the callee), so this is safe.
            let op = &self.operands[0];
            match op {
                Operand::Func(func_idx) => {
                    return aot_mod.funcs[*func_idx].name == CONTROL_POINT_NAME;
                }
                _ => todo!(),
            }
        }
        false
    }

    pub(crate) fn is_stackmap_call(&self, aot_mod: &Module) -> bool {
        if self.opcode == Opcode::Call {
            // Call instructions always have at least one operand (the callee), so this is safe.
            let op = &self.operands[0];
            match op {
                Operand::Func(func_idx) => {
                    return aot_mod.funcs[*func_idx].name == STACKMAP_CALL_NAME;
                }
                _ => todo!(),
            }
        }
        false
    }

    pub(crate) fn is_debug_call(&self, aot_mod: &Module) -> bool {
        if self.opcode == Opcode::Call {
            // Call instructions always have at least one operand (the callee), so this is safe.
            let op = &self.operands[0];
            match op {
                Operand::Func(func_idx) => {
                    return aot_mod.funcs[*func_idx].name == LLVM_DEBUG_CALL_NAME;
                }
                _ => todo!(),
            }
        }
        false
    }

    /// Determine if two instructions in the (immutable) AOT IR are the same based on pointer
    /// identity.
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl AotIRDisplay for Instruction {
    fn to_string(&self, m: &Module) -> String {
        if self.opcode == Opcode::Unimplemented {
            debug_assert!(self.operands.len() == 1);
            if let Operand::Unimplemented(s) = &self.operands[0] {
                return format!("?inst<{}>", s);
            } else {
                // This would be an invalid serialisation.
                panic!();
            }
        }

        if !*m.var_names_computed.borrow() {
            m.compute_variable_names();
        }

        let mut ret = String::new();
        if let Some(_) = self.def_type(m) {
            let name = self.name.borrow();
            // The unwrap cannot fail, as we forced computation of variable names above.
            ret.push_str(&format!(
                "${}: {} = ",
                name.as_ref().unwrap(),
                m.instr_type(self).to_string(m)
            ));
        }
        ret.push_str(&self.opcode.to_string(m));
        if !self.operands.is_empty() {
            ret.push(' ');
        }
        let op_strs = self
            .operands
            .iter()
            .map(|o| o.to_string(m))
            .collect::<Vec<_>>();

        if self.opcode != Opcode::Call {
            ret.push_str(&op_strs.join(", "));
        } else {
            // Put parentheses around the call arguments.
            let mut itr = op_strs.into_iter();
            // unwrap safe: calls must have at least a callee operand.
            ret.push_str(&itr.next().unwrap());
            let rest = itr.collect::<Vec<_>>();
            ret.push('(');
            ret.push_str(&rest.join(", "));
            ret.push(')');
        }

        ret
    }
}

/// A basic block containing IR [Instruction]s.
#[deku_derive(DekuRead)]
#[derive(Debug)]
pub(crate) struct BBlock {
    #[deku(temp)]
    num_instrs: usize,
    #[deku(count = "num_instrs", map = "map_to_tivec")]
    pub(crate) instrs: TiVec<InstrIdx, Instruction>,
}

impl AotIRDisplay for BBlock {
    fn to_string(&self, m: &Module) -> String {
        let mut ret = String::new();
        for i in &self.instrs {
            ret.push_str(&format!("    {}\n", i.to_string(m)));
        }
        ret
    }
}

/// A function definition or declaration.
///
/// If the function was compiled by ykllvm as part of the interpreter binary, then we have IR for
/// the function body, and the function is said to be a *function definition*.
///
/// Conversely, if the function was *not* compiled by ykllvm as part of the interpreter binary (as
/// is the case for shared library functions), then we have no IR for the function body, and the
/// function is said to be a *function declaration*.
///
/// [Func::is_declaration()] can be used to determine if the [Func] is a definition or a
/// declaration.
#[deku_derive(DekuRead)]
#[derive(Debug)]
pub(crate) struct Func {
    #[deku(until = "|v: &u8| *v == 0", map = "map_to_string")]
    name: String,
    type_idx: TypeIdx,
    #[deku(temp)]
    num_bblocks: usize,
    #[deku(count = "num_bblocks", map = "map_to_tivec")]
    bblocks: TiVec<BBlockIdx, BBlock>,
}

impl Func {
    fn is_declaration(&self) -> bool {
        self.bblocks.is_empty()
    }

    /// Return the [BBlock] at the specified index.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of range.
    pub(crate) fn bblock(&self, bb_idx: BBlockIdx) -> &BBlock {
        &self.bblocks[bb_idx]
    }

    /// Return the name of the function.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Return the type index of the function.
    pub(crate) fn type_idx(&self) -> TypeIdx {
        self.type_idx
    }

    /// Return the [FuncType] for the function.
    ///
    /// # Panics
    ///
    /// Panics if the function's type isn't a [FuncType]. This cannot happen in well-formed IR.
    pub(crate) fn func_type<'a>(&self, m: &'a Module) -> &'a FuncType {
        if let Type::Func(ft) = m.type_(self.type_idx) {
            ft
        } else {
            panic!(); // IR is malformed.
        }
    }
}

impl AotIRDisplay for Func {
    fn to_string(&self, m: &Module) -> String {
        let ty = &m.types[self.type_idx];
        if let Type::Func(fty) = ty {
            let mut ret = format!(
                "func {}({}",
                self.name,
                fty.arg_ty_idxs
                    .iter()
                    .enumerate()
                    .map(|(i, t)| format!("$arg{}: {}", i, m.types[*t].to_string(m)))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if fty.is_vararg {
                ret.push_str(", ...");
            }
            ret.push(')');
            let ret_ty = &m.types[fty.ret_ty];
            if ret_ty != &Type::Void {
                ret.push_str(&format!(" -> {}", ret_ty.to_string(m)));
            }
            if self.is_declaration() {
                // declarations have no body, so print it as such.
                ret.push_str(";\n");
            } else {
                ret.push_str(" {\n");
                for (i, b) in self.bblocks.iter().enumerate() {
                    ret.push_str(&format!("  bb{}:\n{}", i, b.to_string(m)));
                }
                ret.push_str("}\n");
            }
            ret
        } else {
            unreachable!("{}", ty.to_string(m)); // Impossible for a function to not be of type `Func`.
        }
    }
}

/// Return the stringified constant integer obtained by interpreting `bytes` as `num-bits`-wide
/// constant integer.
///
/// FIXME: For now we just handle common integer types, but eventually we will need to
/// implement printing of aribitrarily-sized (in bits) integers. Consider using a bigint
/// library so we don't have to do it ourself?
///
/// This discussion may help:
/// https://rust-lang.zulipchat.com/#narrow/stream/122651-general/topic/.E2.9C.94.20Big.20Integer.20library.20with.20bit.20granularity/near/393733327
pub(crate) fn const_int_bytes_to_str(num_bits: u32, bytes: &[u8]) -> String {
    // All of the unwraps below are safe due to:
    debug_assert!(bytes.len() * 8 >= usize::try_from(num_bits).unwrap());

    let mut bytes = bytes;
    match num_bits {
        1 => format!("{}i1", bytes.read_i8().unwrap() & 1),
        8 => format!("{}i8", bytes.read_i8().unwrap()),
        16 => format!("{}i16", bytes.read_i16::<NativeEndian>().unwrap()),
        32 => format!("{}i32", bytes.read_i32::<NativeEndian>().unwrap()),
        64 => format!("{}i64", bytes.read_i64::<NativeEndian>().unwrap()),
        _ => todo!("{}", num_bits),
    }
}

/// A fixed-width integer type.
///
/// Note:
///   1. These integers range in size from 1..2^23 (inc.) bits. This is inherited [from LLVM's
///      integer type](https://llvm.org/docs/LangRef.html#integer-type).
///   2. Signedness is not specified. Interpretation of the bit pattern is delegated to operations
///      upon the integer.
#[deku_derive(DekuRead)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct IntegerType {
    num_bits: u32,
}

impl IntegerType {
    /// Create a new integer type with the specified number of bits.
    #[cfg(test)]
    pub(crate) fn new(num_bits: u32) -> Self {
        debug_assert!(num_bits > 0 && num_bits <= 0x800000);
        Self { num_bits }
    }

    /// Return the number of bits (1..2^23 (inc.)) this integer spans.
    pub(crate) fn num_bits(&self) -> u32 {
        debug_assert!(self.num_bits > 0 && self.num_bits <= 0x800000);
        self.num_bits
    }

    /// Return the number of bytes required to store this integer type.
    ///
    /// Padding for alignment is not included.
    #[cfg(test)]
    pub(crate) fn byte_size(&self) -> usize {
        let bits = self.num_bits();
        let mut ret = bits / 8;
        // If it wasn't an exactly byte-sized thing, round up to the next byte.
        if bits % 8 != 0 {
            ret += 1;
        }
        usize::try_from(ret).unwrap()
    }

    /// Format a constant integer value that is of the type described by `self`.
    fn const_to_str(&self, c: &Constant) -> String {
        const_int_bytes_to_str(self.num_bits, c.bytes())
    }
}

impl AotIRDisplay for IntegerType {
    fn to_string(&self, _m: &Module) -> String {
        format!("i{}", self.num_bits)
    }
}

#[deku_derive(DekuRead)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FuncType {
    /// The number of formal arguments the function takes.
    #[deku(temp)]
    num_args: usize,
    /// Type indices for the function's formal arguments.
    #[deku(count = "num_args")]
    arg_ty_idxs: Vec<TypeIdx>,
    /// Type index of the function's return type.
    ret_ty: TypeIdx,
    /// Is the function vararg?
    is_vararg: bool,
}

impl FuncType {
    #[cfg(test)]
    fn new(arg_ty_idxs: Vec<TypeIdx>, ret_ty_idx: TypeIdx, is_vararg: bool) -> Self {
        Self {
            arg_ty_idxs,
            ret_ty: ret_ty_idx,
            is_vararg,
        }
    }

    pub(crate) fn arg_ty_idxs(&self) -> &[TypeIdx] {
        &self.arg_ty_idxs
    }

    pub(crate) fn ret_ty(&self) -> TypeIdx {
        self.ret_ty
    }

    pub(crate) fn is_vararg(&self) -> bool {
        self.is_vararg
    }
}

impl AotIRDisplay for FuncType {
    fn to_string(&self, m: &Module) -> String {
        let mut args = self
            .arg_ty_idxs
            .iter()
            .map(|t| m.types[*t].to_string(m))
            .collect::<Vec<_>>();
        if self.is_vararg() {
            args.push("...".to_owned());
        }
        let rty = m.type_(self.ret_ty);
        let args_s = args.join(", ");
        if rty != &Type::Void {
            format!("func({}) -> {}", args_s, rty.to_string(m))
        } else {
            format!("func({})", args_s)
        }
    }
}

#[deku_derive(DekuRead)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StructType {
    /// The number of fields the struct has.
    #[deku(temp)]
    num_fields: usize,
    /// The types of the fields.
    #[deku(count = "num_fields")]
    field_ty_idxs: Vec<TypeIdx>,
    /// The bit offsets of the fields (taking into account any required padding for alignment).
    #[deku(count = "num_fields")]
    field_bit_offs: Vec<usize>,
}

impl StructType {
    /// Returns the type index of the specified field index.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds.
    pub(crate) fn field_type_idx(&self, idx: usize) -> TypeIdx {
        self.field_ty_idxs[idx]
    }

    /// Returns the byte offset of the specified field index.
    ///
    /// # Panics
    ///
    /// Panics if the field is not byte-aligned or the index is out of bounds.
    pub(crate) fn field_byte_off(&self, idx: usize) -> usize {
        let bit_off = self.field_bit_offs[idx];
        if bit_off % 8 != 0 {
            todo!();
        }
        bit_off / 8
    }

    /// Returns the number of fields in the struct.
    pub(crate) fn num_fields(&self) -> usize {
        self.field_ty_idxs.len()
    }
}

impl AotIRDisplay for StructType {
    fn to_string(&self, m: &Module) -> String {
        let mut s = String::from("{");
        s.push_str(
            &self
                .field_ty_idxs
                .iter()
                .enumerate()
                .map(|(i, ti)| format!("{}: {}", self.field_bit_offs[i], m.types[*ti].to_string(m)))
                .collect::<Vec<_>>()
                .join(", "),
        );
        s.push('}');
        s
    }
}

const TYKIND_VOID: u8 = 0;
const TYKIND_INTEGER: u8 = 1;
const TYKIND_PTR: u8 = 2;
const TYKIND_FUNC: u8 = 3;
const TYKIND_STRUCT: u8 = 4;
const TYKIND_UNIMPLEMENTED: u8 = 255;

/// A type.
#[deku_derive(DekuRead)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[deku(type = "u8")]
pub(crate) enum Type {
    #[deku(id = "TYKIND_VOID")]
    Void,
    #[deku(id = "TYKIND_INTEGER")]
    Integer(IntegerType),
    #[deku(id = "TYKIND_PTR")]
    Ptr,
    #[deku(id = "TYKIND_FUNC")]
    Func(FuncType),
    #[deku(id = "TYKIND_STRUCT")]
    Struct(StructType),
    #[deku(id = "TYKIND_UNIMPLEMENTED")]
    Unimplemented(#[deku(until = "|v: &u8| *v == 0", map = "map_to_string")] String),
}

impl Type {
    fn const_to_str(&self, c: &Constant) -> String {
        match self {
            Self::Void => "void".to_owned(),
            Self::Integer(it) => it.const_to_str(c),
            Self::Ptr => {
                // FIXME: write a stringifier for constant pointers.
                "const_ptr".to_owned()
            }
            Self::Func(_) => unreachable!(), // No such thing as a constant function in our IR.
            Self::Struct(_) => {
                // FIXME: write a stringifier for constant structs.
                "const_struct".to_owned()
            }
            Self::Unimplemented(s) => format!("?cst<{}>", s),
        }
    }
}

impl AotIRDisplay for Type {
    fn to_string(&self, m: &Module) -> String {
        match self {
            Self::Void => "void".to_owned(),
            Self::Integer(i) => i.to_string(m),
            Self::Ptr => "ptr".to_owned(),
            Self::Func(ft) => ft.to_string(m),
            Self::Struct(st) => st.to_string(m),
            Self::Unimplemented(s) => format!("?ty<{}>", s),
        }
    }
}

/// A constant.
#[deku_derive(DekuRead)]
#[derive(Debug)]
pub(crate) struct Constant {
    type_idx: TypeIdx,
    #[deku(temp)]
    num_bytes: usize,
    #[deku(count = "num_bytes")]
    bytes: Vec<u8>,
}

impl Constant {
    /// Return a byte slice of the constant's value.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Return the type index of the constant.
    pub(crate) fn type_idx(&self) -> TypeIdx {
        self.type_idx
    }
}

impl AotIRDisplay for Constant {
    fn to_string(&self, m: &Module) -> String {
        m.types[self.type_idx].const_to_str(self)
    }
}

/// A global variable declaration, identified by its symbol name.
///
/// Since the AOT IR doesn't capture the initialisers of global variables (externally compiled or
/// otherwise), all global variables are considered *declarations*.
#[deku_derive(DekuRead)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GlobalDecl {
    is_threadlocal: bool,
    #[deku(until = "|v: &u8| *v == 0", map = "map_to_string")]
    name: String,
}

impl AotIRDisplay for GlobalDecl {
    fn to_string(&self, _m: &Module) -> String {
        format!("GlobalDecl({}, tls={})", self.name, self.is_threadlocal)
    }
}

impl GlobalDecl {
    pub(crate) fn is_threadlocal(&self) -> bool {
        self.is_threadlocal
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }
}

/// An AOT IR module.
///
/// This is the top-level container for the AOT IR.
///
/// A module is platform dependent, as type sizes and alignment are baked-in.
#[deku_derive(DekuRead)]
#[derive(Debug, Default)]
pub(crate) struct Module {
    #[deku(assert = "*magic == MAGIC", temp)]
    magic: u32,
    #[deku(assert = "*version == FORMAT_VERSION")]
    version: u32,
    #[deku(temp)]
    num_funcs: usize,
    #[deku(count = "num_funcs", map = "map_to_tivec")]
    funcs: TiVec<FuncIdx, Func>,
    #[deku(temp)]
    num_consts: usize,
    #[deku(count = "num_consts", map = "map_to_tivec")]
    consts: TiVec<ConstIdx, Constant>,
    #[deku(temp)]
    num_global_decls: usize,
    #[deku(count = "num_global_decls", map = "map_to_tivec")]
    global_decls: TiVec<GlobalDeclIdx, GlobalDecl>,
    #[deku(temp)]
    num_types: usize,
    #[deku(count = "num_types", map = "map_to_tivec")]
    types: TiVec<TypeIdx, Type>,
    /// Have local variable names been computed?
    ///
    /// Names are computed on-demand when an instruction is printed for the first time.
    #[deku(skip)]
    var_names_computed: RefCell<bool>,
}

impl Module {
    /// Compute the names of all local variables, for use when stringifying.
    fn compute_variable_names(&self) {
        debug_assert!(!*self.var_names_computed.borrow());
        // Note that because the on-disk IR is conceptually immutable, so we don't have to worry
        // about keeping the names up to date.
        for f in &self.funcs {
            for (bb_idx, bb) in f.bblocks.iter().enumerate() {
                for (inst_idx, inst) in bb.instrs.iter().enumerate() {
                    if let Some(_) = inst.def_type(self) {
                        *inst.name.borrow_mut() = Some(format!("{}_{}", bb_idx, inst_idx));
                    }
                }
            }
        }
        *self.var_names_computed.borrow_mut() = true;
    }

    /// Find a function by its name.
    ///
    /// # Panics
    ///
    /// Panics if no function exists with that name.
    pub(crate) fn func_idx(&self, find_func: &str) -> FuncIdx {
        // OPT: create a cache in the Module.
        self.funcs
            .iter()
            .enumerate()
            .find(|(_, f)| f.name == find_func)
            .map(|(f_idx, _)| FuncIdx(f_idx))
            .unwrap()
    }

    /// Return the block uniquely identified (in this module) by the specified [BBlockId].
    pub(crate) fn bblock(&self, bid: &BBlockId) -> &BBlock {
        self.funcs[bid.func_idx].bblock(bid.bb_idx)
    }

    /// Fill in the function index of local variable operands of instructions.
    ///
    /// FIXME: It may be possible to do this as we deserialise, instead of after the fact:
    /// https://github.com/sharksforarms/deku/issues/363
    fn compute_local_operand_func_indices(&mut self) {
        for (f_idx, f) in self.funcs.iter_mut().enumerate() {
            for bb in &mut f.bblocks {
                for inst in &mut bb.instrs {
                    for op in &mut inst.operands {
                        if let Operand::LocalVariable(ref mut iid) = op {
                            iid.func_idx = FuncIdx(f_idx);
                        }
                    }
                }
            }
        }
    }

    /// Get the type of the instruction.
    ///
    /// It is UB to pass an `instr` that is not from the `Module` referenced by `self`.
    pub(crate) fn instr_type(&self, instr: &Instruction) -> &Type {
        &self.types[instr.type_idx]
    }

    pub(crate) fn constant(&self, co: &ConstIdx) -> &Constant {
        &self.consts[*co]
    }

    pub(crate) fn const_type(&self, c: &Constant) -> &Type {
        &self.types[c.type_idx]
    }

    /// Lookup a type by its index.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds.
    pub(crate) fn type_(&self, idx: TypeIdx) -> &Type {
        &self.types[idx]
    }

    /// Lookup a function by its index.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds.
    pub(crate) fn func(&self, idx: FuncIdx) -> &Func {
        &self.funcs[idx]
    }

    /// Lookup a global variable declaration by its index.
    ///
    /// # Panics
    ///
    /// Panics if the index is out of bounds.
    pub(crate) fn global_decl(&self, idx: GlobalDeclIdx) -> &GlobalDecl {
        &self.global_decls[idx]
    }

    /// Return the number of global variable declarations.
    pub(crate) fn global_decls_len(&self) -> usize {
        self.global_decls.len()
    }

    pub(crate) fn to_str(&self) -> String {
        let mut ret = String::new();
        ret.push_str(&format!("# IR format version: {}\n", self.version));
        ret.push_str(&format!("# Num funcs: {}\n", self.funcs.len()));
        ret.push_str(&format!("# Num consts: {}\n", self.consts.len()));
        ret.push_str(&format!(
            "# Num global decls: {}\n",
            self.global_decls.len()
        ));
        ret.push_str(&format!("# Num types: {}\n", self.types.len()));

        for func in &self.funcs {
            ret.push_str(&format!("\n{}", func.to_string(self)));
        }
        ret
    }

    #[allow(dead_code)]
    pub(crate) fn dump(&self) {
        eprintln!("{}", self.to_str());
    }
}

/// Deserialise an AOT module from the slice `data`.
pub(crate) fn deserialise_module(data: &[u8]) -> Result<Module, Box<dyn Error>> {
    let ((_, _), mut modu) = Module::from_bytes((data, 0))?;
    modu.compute_local_operand_func_indices();
    Ok(modu)
}

/// Deserialise and print IR from an on-disk file.
///
/// Used for support tooling (in turn used by tests too).
pub fn print_from_file(path: &PathBuf) -> Result<(), Box<dyn Error>> {
    let data = fs::read(path)?;
    let ir = deserialise_module(&data)?;
    println!("{}", ir.to_str());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;
    use num_traits::{PrimInt, ToBytes};
    use std::mem;

    #[cfg(target_pointer_width = "64")]
    fn write_native_usize(d: &mut Vec<u8>, v: usize) {
        d.write_u64::<NativeEndian>(v as u64).unwrap(); // `as` is safe: u64 == usize
    }

    fn write_str(d: &mut Vec<u8>, s: &str) {
        d.extend(s.as_bytes());
        d.push(0); // null terminator.
    }

    /// Note that this test only checks valid IR encodings and not for valid IR semantics. For
    /// example, nonsensical instruction arguments (incorrect arg counts, incorrect arg types etc.)
    /// are not checked.
    ///
    /// FIXME: implement an IR validator for this purpose.
    #[test]
    fn deser_and_display() {
        let mut data = Vec::new();

        // HEADER
        // magic:
        data.write_u32::<NativeEndian>(MAGIC).unwrap();
        // version:
        data.write_u32::<NativeEndian>(FORMAT_VERSION).unwrap();

        // num_funcs:
        write_native_usize(&mut data, 2);

        // FUNCTION 0
        // name:
        write_str(&mut data, "foo");
        // type_idx:
        write_native_usize(&mut data, 4);
        // num_bblocks:
        write_native_usize(&mut data, 2);

        // BLOCK 0
        // num_instrs:
        write_native_usize(&mut data, 3);

        // INSTRUCTION 0
        // type_idx:
        write_native_usize(&mut data, 2);
        // opcode:
        data.write_u8(Opcode::Alloca as u8).unwrap();
        // num_operands:
        data.write_u32::<NativeEndian>(1).unwrap();
        // OPERAND 0
        // operand_kind:
        data.write_u8(OPKIND_CONST).unwrap();
        // const_idx
        write_native_usize(&mut data, 0);

        // INSTRUCTION 1
        // type_idx:
        write_native_usize(&mut data, 0);
        // opcode:
        data.write_u8(Opcode::Nop as u8).unwrap();
        // num_operands:
        data.write_u32::<NativeEndian>(0).unwrap();

        // INSTRUCTION 2
        // type_idx:
        write_native_usize(&mut data, 0);
        // opcode:
        data.write_u8(Opcode::CondBr as u8).unwrap();
        // num_operands:
        data.write_u32::<NativeEndian>(3).unwrap();
        // OPERAND 0
        // operand_kind:
        data.write_u8(OPKIND_LOCAL_VARIABLE as u8).unwrap();
        // bb_idx:
        write_native_usize(&mut data, 0);
        // inst_idx:
        write_native_usize(&mut data, 0);
        // OPERAND 1
        // operand_kind:
        data.write_u8(OPKIND_BLOCK as u8).unwrap();
        // bb_idx:
        write_native_usize(&mut data, 0);
        // OPERAND 2
        // operand_kind:
        data.write_u8(OPKIND_BLOCK as u8).unwrap();
        // bb_idx:
        write_native_usize(&mut data, 1);

        // BLOCK 1
        // num_instrs:
        write_native_usize(&mut data, 6);

        // INSTRUCTION 0
        // type_idx:
        write_native_usize(&mut data, 0);
        // opcode:
        data.write_u8(Opcode::Unimplemented as u8).unwrap();
        // num_operands:
        data.write_u32::<NativeEndian>(1).unwrap();
        // OPERAND 0
        // operand_kind:
        data.write_u8(OPKIND_UNIMPLEMENTED as u8).unwrap();
        // unimplemented description:
        write_str(&mut data, "%3 = some_llvm_instruction ...");

        // INSTRUCTION 1
        // type_idx:
        write_native_usize(&mut data, 2);
        // opcode:
        data.write_u8(Opcode::PtrAdd as u8).unwrap();
        // num_operands:
        data.write_u32::<NativeEndian>(1).unwrap();
        // OPERAND 0
        // operand_kind:
        data.write_u8(OPKIND_CONST as u8).unwrap();
        // const_idx:
        write_native_usize(&mut data, 1);

        // INSTRUCTION 2
        // type_idx:
        write_native_usize(&mut data, 2);
        // opcode:
        data.write_u8(Opcode::Alloca as u8).unwrap();
        // num_operands:
        data.write_u32::<NativeEndian>(2).unwrap();
        // OPERAND 0
        // operand_kind:
        data.write_u8(OPKIND_TYPE).unwrap();
        // type_idx:
        write_native_usize(&mut data, 3);
        // OPERAND 1
        // operand_kind:
        data.write_u8(OPKIND_CONST as u8).unwrap();
        // const_idx:
        write_native_usize(&mut data, 2);

        // INSTRUCTION 3
        // type_idx:
        write_native_usize(&mut data, 2);
        // opcode:
        data.write_u8(Opcode::Call as u8).unwrap();
        // num_operands:
        data.write_u32::<NativeEndian>(3).unwrap();
        // OPERAND 0
        // operand_kind:
        data.write_u8(OPKIND_FUNC).unwrap();
        // func_idx:
        write_native_usize(&mut data, 1);
        // OPERAND 1
        // operand_kind:
        data.write_u8(OPKIND_CONST).unwrap();
        // const_idx:
        write_native_usize(&mut data, 2);
        // OPERAND 2
        // operand_kind:
        data.write_u8(OPKIND_CONST).unwrap();
        // const_idx:
        write_native_usize(&mut data, 2);

        // INSTRUCTION 4
        // type_idx:
        write_native_usize(&mut data, 0);
        // opcode:
        data.write_u8(Opcode::Br as u8).unwrap();
        // num_operands:
        data.write_u32::<NativeEndian>(0).unwrap();

        // INSTRUCTION 5
        // type_idx:
        write_native_usize(&mut data, 6);
        // opcode:
        data.write_u8(Opcode::Nop as u8).unwrap();
        // num_operands:
        data.write_u32::<NativeEndian>(0).unwrap();

        // FUNCTION 1
        // name:
        write_str(&mut data, "bar");
        // type_idx:
        write_native_usize(&mut data, 5);
        // num_bblocks:
        write_native_usize(&mut data, 0);

        // CONSTANTS
        // num_consts:
        write_native_usize(&mut data, 3);

        // CONSTANT 0
        // type_idx:
        write_native_usize(&mut data, 1);
        // num_bytes:
        write_native_usize(&mut data, 0);

        // CONSTANT 1
        // type_idx:
        write_native_usize(&mut data, 3);
        // num_bytes:
        write_native_usize(&mut data, 4);
        // bytes:
        data.write_u32::<NativeEndian>(u32::MAX).unwrap();

        // CONSTANT 2
        // type_idx:
        write_native_usize(&mut data, 3);
        // num_bytes:
        write_native_usize(&mut data, 4);
        // bytes:
        data.write_u32::<NativeEndian>(50).unwrap();

        // GLOBAL DECLS
        // num_global_decls:
        write_native_usize(&mut data, 1);

        // GLOBAL DECL 1
        // is_threadlocal:
        let _ = data.write_u8(0);
        // name:
        write_str(&mut data, "aaa");

        // TYPES
        // num_types:
        write_native_usize(&mut data, 7);

        // TYPE 0
        // type_kind:
        data.write_u8(TYKIND_VOID).unwrap();

        // TYPE 1
        // type_kind:
        data.write_u8(TYKIND_UNIMPLEMENTED).unwrap();
        // unimplemented description:
        write_str(&mut data, "a_type");

        // TYPE 2
        // type_kind:
        data.write_u8(TYKIND_PTR).unwrap();

        // TYPE 3
        // type_kind:
        data.write_u8(TYKIND_INTEGER).unwrap();
        // num_bits:
        data.write_u32::<NativeEndian>(32).unwrap();

        // TYPE 4
        // type_kind:
        data.write_u8(TYKIND_FUNC).unwrap();
        // num_args:
        write_native_usize(&mut data, 2);
        // arg_ty_idxs:
        write_native_usize(&mut data, 2);
        write_native_usize(&mut data, 3);
        // ret_ty:
        write_native_usize(&mut data, 3);
        // is_vararg:
        data.write_u8(0).unwrap();

        // TYPE 5
        // type_kind:
        data.write_u8(TYKIND_FUNC).unwrap();
        // num_args:
        write_native_usize(&mut data, 0);
        // ret_ty:
        write_native_usize(&mut data, 0);
        // is_vararg:
        data.write_u8(0).unwrap();

        // TYPE 6
        // type_kind:
        data.write_u8(TYKIND_STRUCT).unwrap();
        // num_fields:
        write_native_usize(&mut data, 2);
        // field_ty_idxs[0]:
        write_native_usize(&mut data, 2);
        // field_ty_idxs[1]:
        write_native_usize(&mut data, 3);
        // field_bit_offs[0]:
        write_native_usize(&mut data, 0);
        // field_bit_offs[0]:
        write_native_usize(&mut data, 24);

        let test_mod = deserialise_module(data.as_slice()).unwrap();
        let string_mod = test_mod.to_str();

        println!("{}", string_mod);
        let expect = "\
# IR format version: 0
# Num funcs: 2
# Num consts: 3
# Num global decls: 1
# Num types: 7

func foo($arg0: ptr, $arg1: i32) -> i32 {
  bb0:
    $0_0: ptr = alloca ?cst<a_type>
    nop
    condbr $0_0, bb0, bb1
  bb1:
    ?inst<%3 = some_llvm_instruction ...>
    $1_1: ptr = ptradd -1i32
    $1_2: ptr = alloca i32, 50i32
    $1_3: ptr = call bar(50i32, 50i32)
    br
    $1_5: {0: ptr, 24: i32} = nop
}

func bar();
";
        assert_eq!(expect, string_mod);
    }

    #[test]
    fn string_deser() {
        let check = |s: &str| {
            assert_eq!(
                &map_to_string(CString::new(s).unwrap().into_bytes_with_nul()).unwrap(),
                s
            );
        };
        check("testing");
        check("the quick brown fox jumped over the lazy dog");
        check("");
        check("         ");
    }

    #[test]
    fn const_int_strings() {
        // Check (in an endian neutral manner) that a `num-bits`-sized integer of value `num`, when
        // converted to a constant IR integer, then stringified, results in the string `expect`.
        //
        // When `num` has a bit size greater than `num_bits` the most significant bits of `num` are
        // treated as undefined: they can be any value as IR stringification will ignore them.
        fn check<T: ToBytes + PrimInt>(num_bits: u32, num: T, expect: &str) {
            assert!(mem::size_of::<T>() * 8 >= usize::try_from(num_bits).unwrap());

            // Get a byte-vector for `num`.
            let bytes = ToBytes::to_ne_bytes(&num).as_ref().to_vec();

            // Construct an IR constant and check it stringifies ok.
            let it = IntegerType { num_bits };
            let c = Constant {
                type_idx: TypeIdx::new(0),
                bytes,
            };
            assert_eq!(it.const_to_str(&c), expect);
        }

        check(1, 1u8, "1i1");
        check(1, 0u8, "0i1");
        check(1, 254u8, "0i1");
        check(1, 255u8, "1i1");
        check(1, 254u64, "0i1");
        check(1, 255u64, "1i1");

        check(16, 0u16, "0i16");
        check(16, u16::MAX, "-1i16");
        check(16, 12345u16, "12345i16");
        check(16, 12345u64, "12345i16");
        check(16, i16::MIN as u16, &format!("{}i16", i16::MIN));
        check(16, i16::MIN as u64, &format!("{}i16", i16::MIN));

        check(32, 0u32, "0i32");
        check(32, u32::MAX, "-1i32");
        check(32, 12345u32, "12345i32");
        check(32, 12345u64, "12345i32");
        check(32, i32::MIN as u32, &format!("{}i32", i32::MIN));
        check(32, i32::MIN as u64, &format!("{}i32", i32::MIN));

        check(64, 0u64, "0i64");
        check(64, u64::MAX, "-1i64");
        check(64, 12345678u64, "12345678i64");
        check(64, i64::MIN as u64, &format!("{}i64", i64::MIN));
    }

    #[test]
    fn integer_type_sizes() {
        for i in 1..8 {
            assert_eq!(IntegerType::new(i).byte_size(), 1);
        }
        for i in 9..16 {
            assert_eq!(IntegerType::new(i).byte_size(), 2);
        }
        assert_eq!(IntegerType::new(127).byte_size(), 16);
        assert_eq!(IntegerType::new(128).byte_size(), 16);
        assert_eq!(IntegerType::new(129).byte_size(), 17);
    }

    #[test]
    fn stringify_func_types() {
        let mut m = Module::default();

        let i8_tyidx = TypeIdx::new(m.types.len());
        m.types.push(Type::Integer(IntegerType { num_bits: 8 }));
        let void_tyidx = TypeIdx::new(m.types.len());
        m.types.push(Type::Void);

        let fty = Type::Func(FuncType::new(vec![i8_tyidx], i8_tyidx, false));
        assert_eq!(&fty.to_string(&m), "func(i8) -> i8");

        let fty = Type::Func(FuncType::new(vec![i8_tyidx], i8_tyidx, true));
        assert_eq!(&fty.to_string(&m), "func(i8, ...) -> i8");

        let fty = Type::Func(FuncType::new(vec![], i8_tyidx, false));
        assert_eq!(&fty.to_string(&m), "func() -> i8");

        let fty = Type::Func(FuncType::new(vec![], void_tyidx, false));
        assert_eq!(&fty.to_string(&m), "func()");
    }
}
