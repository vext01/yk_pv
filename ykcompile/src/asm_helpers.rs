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

/// Emits a 'reg <- const'  assembler instruction using the desired size qualifier.
macro_rules! asm_reg_const {
    ($dasm: expr, $size: expr, $op: expr, $reg: expr, $const: expr) => {
        match $size {
            1 => {
                dynasm!($dasm
                    ; $op Rb($reg), BYTE $const
                );
            }
            2 => {
                dynasm!($dasm
                    ; $op Rw($reg), WORD $const
                );
            },
            4 => {
                dynasm!($dasm
                    ; $op Rd($reg), DWORD $const
                );
            },
            8 => {
                dynasm!($dasm
                    ; $op Rq($reg), QWORD $const
                );
            }
            _ => panic!("Invalid size operand: {}", $size),
        }
    }
}
