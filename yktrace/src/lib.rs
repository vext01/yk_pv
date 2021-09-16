//! Utilities for collecting and decoding traces.

#![feature(thread_local_const_init)]

mod errors;
use libc::c_void;
use libffi;
use std::{
    cell::RefCell,
    collections::HashMap,
    default::Default,
    ffi::{CStr, CString},
    ptr,
};
mod hwt;

pub use errors::InvalidTraceError;
pub use hwt::mapper::BlockMap;

thread_local! {
    // FIXME: talk about inputs.
    // When `Some`, contains the `ThreadTracer` for the current thread. When `None`, the current
    // thread is not being traced.
    //
    // We hide the `ThreadTracer` in a thread local (rather than returning it to the consumer of
    // yk). This ensures that the `ThreadTracer` itself cannot appear in traces.
    pub static THREAD_TRACER: RefCell<Option<(ThreadTracer, Vec<*const c_void>)>> = RefCell::new(None);
}

/// The different ways by which we can collect a trace.
#[derive(Clone, Copy)]
pub enum TracingKind {
    /// Software tracing.
    SoftwareTracing,
    /// Hardware tracing via a branch tracer (e.g. Intel PT).
    HardwareTracing,
}

impl Default for TracingKind {
    /// Returns the default tracing kind.
    fn default() -> Self {
        // FIXME this should query the hardware for a suitable hardware tracer and failing that
        // fall back on software tracing.
        TracingKind::HardwareTracing
    }
}

/// A globally unique block ID for an LLVM IR block.
#[derive(Debug, Eq, PartialEq)]
pub struct IRBlock {
    /// The name of the function containing the block.
    ///
    /// PERF: Use a string pool to avoid duplicated function names in traces.
    func_name: CString,
    /// The index of the block within the function.
    ///
    /// The special value `usize::MAX` indicates unmappable code.
    bb: usize,
}

impl IRBlock {
    pub fn func_name(&self) -> &CStr {
        &self.func_name.as_c_str()
    }

    pub fn bb(&self) -> usize {
        self.bb
    }

    /// Returns an IRBlock whose `bb` field indicates unmappable code.
    pub fn unmappable() -> Self {
        Self {
            func_name: CString::new("").unwrap(),
            bb: usize::MAX,
        }
    }

    /// Determines whether `self` represents unmappable code.
    pub fn is_unmappable(&self) -> bool {
        self.bb == usize::MAX
    }
}

/// An LLVM IR trace.
pub struct IRTrace {
    /// The blocks of the trace.
    blocks: Vec<IRBlock>,
    /// Function addresses discovered dynamically via the trace. symbol-name -> address.
    faddrs: HashMap<CString, *const c_void>,
    /// FIXME
    pub inputs: Vec<*const c_void>,
}

unsafe impl Send for IRTrace {}
unsafe impl Sync for IRTrace {}

impl IRTrace {
    pub(crate) fn new(
        blocks: Vec<IRBlock>,
        faddrs: HashMap<CString, *const c_void>,
        inputs: Vec<*const c_void>,
    ) -> Self {
        debug_assert!(blocks.len() < usize::MAX);
        Self {
            blocks,
            faddrs,
            inputs,
        }
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    // Get the block at the specified position.
    // Returns None if there is no block at this index. Returns Some(None) if the block at that
    // index couldn't be mapped.
    pub fn get(&self, idx: usize) -> Option<Option<&IRBlock>> {
        // usize::MAX is a reserved index.
        debug_assert_ne!(idx, usize::MAX);
        self.blocks
            .get(idx)
            .map(|b| if b.is_unmappable() { None } else { Some(b) })
    }

    pub fn compile(&self) -> CompiledTrace {
        let len = self.len();
        let mut func_names = Vec::with_capacity(len);
        let mut bbs = Vec::with_capacity(len);
        for blk in &self.blocks {
            if blk.is_unmappable() {
                // The block was unmappable. Indicate this with a null function name.
                func_names.push(ptr::null());
                bbs.push(0);
            } else {
                func_names.push(blk.func_name().as_ptr());
                bbs.push(blk.bb());
            }
        }

        let mut faddr_keys = Vec::new();
        let mut faddr_vals = Vec::new();
        for k in self.faddrs.iter() {
            faddr_keys.push(k.0.as_ptr());
            faddr_vals.push(*k.1);
        }

        let code_ptr = unsafe {
            ykllvmwrap::__ykllvmwrap_irtrace_compile(
                func_names.as_ptr(),
                bbs.as_ptr(),
                len,
                faddr_keys.as_ptr(),
                faddr_vals.as_ptr(),
                faddr_keys.len(),
            )
        };
        assert_ne!(code_ptr, ptr::null());

        //// Make a libffi calling interface (CIF) so that we can call the JITted code with the
        //// correct (dynamic) signature later.
        let num_inputs = self.inputs.len();
        let mut cif_arg_types = Vec::new();
        cif_arg_types.resize(num_inputs, libffi::middle::Type::pointer());

        CompiledTrace {
            code_ptr: libffi::middle::CodePtr(code_ptr as *mut c_void),
            cif: libffi::middle::Cif::new(cif_arg_types, libffi::middle::Type::void()),
            args: self.inputs.clone(),
        }
    }
}

/// Binary executable trace code.
#[repr(C)]
pub struct CompiledTrace {
    code_ptr: libffi::middle::CodePtr,
    cif: libffi::middle::Cif,
    args: Vec<*const c_void>,
}

impl CompiledTrace {
    pub fn execute(&self) {
        let args = self
            .args
            .iter()
            .map(|a| libffi::middle::arg(a))
            .collect::<Vec<_>>();
        unsafe { self.cif.call::<()>(self.code_ptr, &args) };
    }
}

unsafe impl Send for CompiledTrace {}
unsafe impl Sync for CompiledTrace {}

/// Represents a thread which is currently tracing.
pub struct ThreadTracer {
    /// The tracing implementation.
    t_impl: Box<dyn ThreadTracerImpl>,
}

impl ThreadTracer {
    /// Stops tracing on the current thread, returning a IR trace on success.
    pub fn stop_tracing(
        mut self,
        inputs: Vec<*const c_void>,
    ) -> Result<IRTrace, InvalidTraceError> {
        let trace = self.t_impl.stop_tracing(inputs);
        if let Ok(inner) = &trace {
            if inner.len() == 0 {
                return Err(InvalidTraceError::EmptyTrace);
            }
        }
        trace
    }
}

// An generic interface which tracing backends must fulfill.
trait ThreadTracerImpl {
    /// Stops tracing on the current thread, returning the IR trace on success.
    fn stop_tracing(&mut self, inputs: Vec<*const c_void>) -> Result<IRTrace, InvalidTraceError>;
}

/// Start tracing on the current thread using the specified tracing kind.
/// Each thread can have at most one active tracer; calling `start_tracing()` on a thread where
/// there is already an active tracer leads to undefined behaviour.
pub fn start_tracing(kind: TracingKind, inputs: Vec<*const c_void>) {
    let tt = match kind {
        TracingKind::SoftwareTracing => todo!(),
        TracingKind::HardwareTracing => hwt::start_tracing(),
    };
    THREAD_TRACER.with(|tl| *tl.borrow_mut() = Some((tt, inputs)));
}

/// Stop tracing on the current thread. Calling this when the current thread is not already tracing
/// leads to undefined behaviour.
pub fn stop_tracing() -> Result<IRTrace, InvalidTraceError> {
    THREAD_TRACER.with(|tt| {
        let tt_info = tt.borrow_mut().take().unwrap();
        tt_info.0.stop_tracing(tt_info.1)
    })
}
