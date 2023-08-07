use crate::frame::llvmbridge::{BasicBlock, Module, TargetData, Value};
use dynasmrt::{
    self, dynasm, AssemblyOffset, DynamicLabel, DynasmApi, DynasmLabelApi, ExecutableBuffer,
};
use llvm_sys::{LLVMOpcode, core::*, prelude::*};
use std::{
    cell::Cell,
    collections::HashMap,
    ffi::{c_char, c_void},
    fs::File,
    io, slice,
    sync::{Arc, Mutex},
};
use yksmp::LiveVar;
use tempfile;

// FIXME: yeuch. nasty hack to hold code alive.
static FASTTCG_CODE: Mutex<Vec<ExecutableBuffer>> = Mutex::new(Vec::new());

// FIXME: yeuch. nasty hack to hold tempfiles open.
static FASTTCG_TEMPS: Mutex<Vec<tempfile::NamedTempFile>> = Mutex::new(Vec::new());

#[cfg(not(target_arch = "x86_64"))]
error!("unsupported FastTCG architecture");

#[cfg(target_arch = "x86_64")]
mod x86_64;

mod gdb;

#[derive(Debug, Clone, Copy)]
struct Local {
    frame_off: i32,
}

impl Local {
    fn new(frame_off: i32) -> Self {
        Self { frame_off }
    }
}

#[cfg(debug_assertions)]
struct TracePrinter<'a> {
    buf: &'a ExecutableBuffer,
    debug_lines: &'a HashMap<AssemblyOffset, Vec<String>>,
    // vaddr -> line_no
    line_map: Cell<HashMap<usize, usize>>,
    output: &'a mut dyn io::Write,
    line_num: usize,
}

#[cfg(debug_assertions)]
impl<'a> TracePrinter<'a> {
    fn new(
        buf: &'a ExecutableBuffer,
        #[cfg(debug_assertions)]
        debug_lines: &'a HashMap<AssemblyOffset, Vec<String>>,
        output: &'a mut dyn io::Write,
    ) -> Self {
        Self {
            buf,
            debug_lines,
            line_map: Cell::new(HashMap::new()),
            output,
            line_num: 1,
        }
    }

    fn append_line(&mut self, line: String, vaddr: Option<usize>) {
        if let Some(vaddr) = vaddr {
            self.line_map.get_mut().insert(vaddr, self.line_num);
        }
        write!(self.output, "{}\n", line).unwrap();
        self.line_num += 1;
    }

    fn print(&mut self) -> HashMap<usize, usize> {
        self.append_line("--- Begin fasttcg output ---".into(), None);
        let len = self.buf.len();
        let bptr = self.buf.ptr(AssemblyOffset(0));
        let code = unsafe { slice::from_raw_parts(bptr, len) };
        self.target_print(code, bptr as u64, len);
        self.append_line("--- End fasttcg output ---".into(), None);
        self.line_map.take()
    }

    #[cfg(target_arch = "x86_64")]
    fn target_print(&mut self, code: &[u8], start_vaddr: u64, len: usize) {
        let mut dis = iced_x86::Decoder::with_ip(64, code, start_vaddr, 0);

        let mut remain = len;
        while remain != 0 {
            let off = len - remain;
            if let Some(lines) = self.debug_lines.get(&AssemblyOffset(off)) {
                for line in lines {
                    self.append_line(format!("; {line}"), Some(dis.ip() as usize));
                }
            }
            let inst = dis.decode();
            // self.append_line(
            //     format!("{:08x} {:08x}: {}", off, inst.ip(), inst),
            //     Some(usize::try_from(inst.ip()).unwrap()),
            // );
            remain = remain.checked_sub(inst.len()).unwrap();
        }
    }
}

// FIXME: hack to get a breakpoint.
// #[inline(never)]
// #[no_mangle]
// pub extern "C" fn xxx() {}

/// The Fast Trace Code Generator.
pub struct FastTCG {
    jitmod: Module,
    jitfunc: Value,
    #[cfg(target_arch = "x86_64")]
    asm: dynasmrt::x64::Assembler,
    /// Abstract stack pointer, as a relative offset from `RBP`. The higher this number, the larger
    /// the JITted code's stack. That means that even on a host were the stack grows down, this
    /// value grows up.
    asp: i32,
    /// Maps each LLVM local variable to a size and frame offset in the JITted code.
    ///
    /// Note that the codegen is dumb and *all* variables are spilled to the stack: there is no
    /// register allocation.
    locals: HashMap<Value, Local>,
    /// LLVM target information.
    target: TargetData,
    /// Debug line info. // FIXME guard?
    #[cfg(debug_assertions)]
    debug_lines: Cell<HashMap<AssemblyOffset, Vec<String>>>,
    block_labels: HashMap<BasicBlock, DynamicLabel>,
    global_mappings: HashMap<LLVMValueRef, *const c_void>,
    // return address -> vars
    stackmaps: Cell<HashMap<u64, Vec<LiveVar>>>,
}

impl FastTCG {
    pub fn new(jitmod: Module, global_mappings: HashMap<LLVMValueRef, *const c_void>) -> FastTCG {
        #[cfg(target_arch = "x86_64")]
        // XXX: assert function looks right and there is only one.
        let jitfunc = jitmod.first_function().unwrap();
        let asm = dynasmrt::x64::Assembler::new().unwrap();
        let target = jitmod.target_data();
        #[cfg(debug_assertions)]
        let debug_lines = Cell::new(HashMap::new());
        let block_labels = HashMap::new();
        Self {
            jitmod,
            jitfunc,
            asm,
            asp: 0,
            locals: HashMap::new(),
            target,
            #[cfg(debug_assertions)]
            debug_lines,
            block_labels,
            global_mappings,
            stackmaps: Cell::new(HashMap::new()),
        }
    }

    pub fn label_for_block(&mut self, bb: BasicBlock) -> DynamicLabel {
        *self
            .block_labels
            .entry(bb)
            .or_insert_with(|| self.asm.new_dynamic_label())
    }

    pub fn codegen(mut self) -> (*const c_void, HashMap<u64, Vec<LiveVar>>) {
        // XXX: assert function looks right and there is only one.
        #[cfg(debug_assertions)]
        self.add_debug_line(self.asm.offset(), "prologue".to_owned());

        // dynasm!(self.asm
        //     ; mov rax, QWORD xxx as _
        //     ; call rax
        // );
        let alloc_off = self.emit_prologue();


        for bb in self.jitfunc.iter_basic_blocks() {
            #[cfg(debug_assertions)]
            {
                self.add_debug_line(
                    self.asm.offset(),
                    format!("{}: ", bb.name().to_str().unwrap().trim()),
                );
            }
            let bl = self.label_for_block(bb);
            self.asm.dynamic_label(bl);
            for inst in bb.iter_instructions() {
                self.codegen_inst(inst);
            }
        }

        // Now we know the size of the stack frame, add the instruction to allocate it.
        // XXX X64, move
        let mut patchup = self.asm.alter_uncommitted();
        patchup.goto(alloc_off);
        let rem = self.asp % 16;
        if rem != 0 {
            self.asp += 16 - rem;
        }
        dynasm!(patchup; sub rsp, DWORD self.asp);

        #[cfg(debug_assertions)]
        let debug_lines = self.debug_lines.take(); // `self` about to be consumed.
        let stackmaps = self.stackmaps.take();

        self.asm.commit().unwrap();
        let buf = self.asm.finalize().unwrap();

        // Patch up stackmap return addresses now we know the load adress of the code.
        let mut new_stackmaps = HashMap::new();
        let load_addr = buf.ptr(AssemblyOffset(0)) as u64;
        for (k, v) in stackmaps {
            new_stackmaps.insert(k + load_addr, v);
        }

        // XXX added `debug_assertions` at last minute and broke gdb integration.
        #[cfg(debug_assertions)]
        let (line_map, path) = {
            let mut debug_src = tempfile::NamedTempFile::new().unwrap();
            let path = debug_src.path().to_owned(); // FIXME, can kill?
            let mut printer = TracePrinter::new(&buf, &debug_lines, &mut debug_src);
            let line_map = printer.print();
            FASTTCG_TEMPS.lock().unwrap().push(debug_src);
            (line_map, path)
        };

        let ptr = buf.ptr(AssemblyOffset(0));
        let len = buf.len();

        // FIXME: Stop the code from being freed. Gross hack.
        FASTTCG_CODE.lock().unwrap().push(buf);

        // Tell gdb about it.
        #[cfg(debug_assertions)]
        gdb::register_jitted_code(ptr as *const c_void, len, &line_map, &path);

        (ptr as *const c_void, new_stackmaps) // FIXME: ideally we'd execute the trace using dynasm's safe interface
    }

    #[cfg(debug_assertions)]
    fn add_debug_line(&mut self, off: AssemblyOffset, line: String) {
        self.debug_lines
            .get_mut()
            .entry(off)
            .or_default()
            .push(line);
    }

    fn codegen_inst(&mut self, inst: Value) {
        debug_assert!(inst.is_instruction());

        #[cfg(debug_assertions)]
        self.add_debug_line(self.asm.offset(), format!("{:?}", inst));

        match inst.inst_opcode() {
            LLVMOpcode::LLVMGetElementPtr => self.codegen_gep_inst(inst),
            LLVMOpcode::LLVMLoad => self.codegen_load_inst(inst),
            LLVMOpcode::LLVMBr => self.codegen_branch_inst(inst),
            LLVMOpcode::LLVMStore => self.codegen_store_inst(inst),
            LLVMOpcode::LLVMAdd => self.codegen_add_inst(inst),
            LLVMOpcode::LLVMSub => self.codegen_sub_inst(inst),
            LLVMOpcode::LLVMSelect => self.codegen_select_inst(inst),
            LLVMOpcode::LLVMICmp => self.codegen_icmp_inst(inst),
            LLVMOpcode::LLVMAlloca => self.codegen_alloca_inst(inst),
            LLVMOpcode::LLVMRet=> self.codegen_ret_inst(inst),
            LLVMOpcode::LLVMCall => self.codegen_call_inst(inst),
            LLVMOpcode::LLVMSExt => self.codegen_sext_inst(inst),
            LLVMOpcode::LLVMPtrToInt => self.codegen_ptrtoint_inst(inst),
            LLVMOpcode::LLVMShl => self.codegen_shl_inst(inst),
            _ => todo!("unknown instruction: {:?}", inst.inst_opcode()),
        }
    }
}
