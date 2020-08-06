//! Helpers for generating assembler code with dynasm.

/// Emits a 'mem <- reg'  assembler instruction using the desired size qualifier.
macro_rules! asm_mem_reg {
    ($dasm: expr, $size: expr, $op: expr, $mem: expr, $reg: expr) => {
        match $size {
            1 => {
                dynasm!($dasm
                    ; $op BYTE $mem, Rb($reg)
                );
            }
            2 => {
                dynasm!($dasm
                    ; $op WORD $mem, Rw($reg)
                );
            },
            4 => {
                dynasm!($dasm
                    ; $op DWORD $mem, Rd($reg)
                );
            },
            8 => {
                dynasm!($dasm
                    ; $op QWORD $mem, Rq($reg)
                );
            }
            _ => panic!("Invalid size operand: {}", $size),
        }
    }
}

/// Emits a 'reg <- mem'  assembler instruction using the desired size qualifier.
macro_rules! asm_reg_mem {
    ($dasm: expr, $size: expr, $op: expr, $reg: expr, $mem: expr) => {
        match $size {
            1 => {
                dynasm!($dasm
                    ; $op Rb($reg), BYTE $mem
                );
            }
            2 => {
                dynasm!($dasm
                    ; $op Rw($reg), WORD $mem
                );
            },
            4 => {
                dynasm!($dasm
                    ; $op Rd($reg), DWORD $mem
                );
            },
            8 => {
                dynasm!($dasm
                    ; $op Rq($reg), QWORD $mem
                );
            }
            _ => panic!("Invalid size operand: {}", $size),
        }
    }
}

/// Emits a 'reg <- reg'  assembler instruction using the desired size qualifier.
macro_rules! asm_reg_reg {
    ($dasm: expr, $size: expr, $op: expr, $dest_reg: expr, $src_reg: expr) => {
        match $size {
            1 => {
                dynasm!($dasm
                    ; $op Rb($dest_reg), Rb($src_reg)
                );
            }
            2 => {
                dynasm!($dasm
                    ; $op Rw($dest_reg), Rw($src_reg)
                );
            },
            4 => {
                dynasm!($dasm
                    ; $op Rd($dest_reg), Rd($src_reg)
                );
            },
            8 => {
                dynasm!($dasm
                    ; $op Rq($dest_reg), Rq($src_reg)
                );
            }
            _ => panic!("Invalid size operand: {}", $size),
        }
    }
}

/// Emits a 'reg <- const'  assembler instruction using the desired size qualifier.
/// This currently has no arm for `$size = 8` as some operations, e.g. ADD don't allow imm64 source
/// operands and that would cause a (compile-time) dynasm error if we had the `$size = 8` arm.
macro_rules! asm_reg_const32 {
    ($dasm: expr, $size: expr, $op: expr, $reg: expr, $const: expr) => {
        match $size {
            1 => {
                dynasm!($dasm
                    ; $op Rb($reg), BYTE $const as i8
                );
            },
            2 => {
                dynasm!($dasm
                    ; $op Rw($reg), WORD $const as i16
                );
            },
            4 => {
                dynasm!($dasm
                    ; $op Rd($reg), DWORD $const
                );
            },
            _ => panic!("Invalid size operand: {}", $size),
        }
    }
}
