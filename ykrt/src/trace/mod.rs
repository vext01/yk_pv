//! Utilities for collecting and decoding traces.

#![allow(clippy::len_without_is_empty)]
#![allow(clippy::new_without_default)]
#![allow(clippy::missing_safety_doc)]

mod errors;
use libc::c_void;
use llvm_sys::prelude::{LLVMModuleRef, LLVMValueRef};
//use llvm_sys::{error::{LLVMCreateStringError, LLVMErrorRef}, orc2::LLVMOrcThreadSafeModuleWithModuleDo};
#[cfg(unix)]
use std::os::unix::io::AsRawFd;
use std::{
    collections::HashMap,
    env,
    error::Error,
    ffi::{c_char, c_int, CStr, CString},
    fmt, ptr,
    sync::Arc,
};
pub mod hwt;
use tempfile::NamedTempFile;
use yksmp::{LiveVar, StackMapParser};
use ykutil::obj::llvmbc_section;

use crate::{
    fasttcg::FastTCG,
    frame::llvmbridge::{Module, Value},
    mt::MT,
};
pub use errors::InvalidTraceError;

/// A globally unique block ID for an LLVM IR block.
#[derive(Debug, Eq, PartialEq)]
pub enum IRBlock {
    /// A sucessfully mapped block.
    Mapped {
        /// The name of the function containing the block.
        ///
        /// PERF: Use a string pool to avoid duplicated function names in traces.
        func_name: CString,
        /// The index of the block within the function.
        ///
        /// The special value `usize::MAX` indicates unmappable code.
        bb: usize,
    },
    /// One or more machine blocks that could not be mapped.
    ///
    /// This usually means that the blocks were compiled outside of ykllvm.
    Unmappable {
        /// The change to the stack depth as a result of executing the unmappable region.
        stack_adjust: isize,
    },
}

impl IRBlock {
    pub fn new_mapped(func_name: CString, bb: usize) -> Self {
        Self::Mapped { func_name, bb }
    }

    pub fn new_unmappable(stack_adjust: isize) -> Self {
        Self::Unmappable { stack_adjust }
    }

    /// If `self` is a mapped block, return the function name, otherwise panic.
    pub fn func_name(&self) -> &CStr {
        if let Self::Mapped { func_name, .. } = self {
            func_name.as_c_str()
        } else {
            panic!();
        }
    }

    /// If `self` is a mapped block, return the basic block index, otherwise panic.
    pub fn bb(&self) -> usize {
        if let Self::Mapped { bb, .. } = self {
            *bb
        } else {
            panic!();
        }
    }

    /// Determines whether `self` represents unmappable code.
    pub fn is_unmappable(&self) -> bool {
        matches!(self, Self::Unmappable { .. })
    }

    /// If `self` is an unmappable region, return the stack adjustment value, otherwise panic.
    pub fn stack_adjust(&self) -> isize {
        if let Self::Unmappable { stack_adjust } = self {
            *stack_adjust
        } else {
            panic!();
        }
    }

    pub fn stack_adjust_mut(&mut self) -> &mut isize {
        if let Self::Unmappable { stack_adjust } = self {
            stack_adjust
        } else {
            panic!();
        }
    }
}

/// An LLVM IR trace.
pub struct IRTrace {
    /// The blocks of the trace.
    blocks: Vec<IRBlock>,
    /// Function addresses discovered dynamically via the trace. symbol-name -> address.
    faddrs: HashMap<CString, *const c_void>,
}

impl IRTrace {
    pub fn new(blocks: Vec<IRBlock>, faddrs: HashMap<CString, *const c_void>) -> Self {
        debug_assert!(blocks.len() < usize::MAX);
        Self { blocks, faddrs }
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    fn encode_trace(&self) -> (Vec<*const i8>, Vec<usize>, usize) {
        let trace_len = self.len();
        let mut func_names = Vec::with_capacity(trace_len);
        let mut bbs = Vec::with_capacity(trace_len);
        for blk in &self.blocks {
            if blk.is_unmappable() {
                // The block was unmappable. Indicate this with a null function name and the block
                // index encodes the stack adjustment value.
                func_names.push(ptr::null());
                // Subtle cast from `isize` to `usize`. `as` is used deliberately here to preserve
                // the exact bit pattern. The consumer on the other side of the FFI knows to
                // reverse this.
                bbs.push(blk.stack_adjust() as usize);
            } else {
                func_names.push(blk.func_name().as_ptr());
                bbs.push(blk.bb());
            }
        }
        (func_names, bbs, trace_len)
    }

    // If necessary, create a temporary file for us to write the trace's debugging "source code"
    // into. Elsewhere, the JIT module will have `DebugLoc`s inserted into it which will point to
    // lines in this temporary file.
    //
    // If the `YKD_TRACE_DEBUGINFO` environment variable is set to "1", then this function returns
    // a `NamedTempFile`, a non-negative file descriptor, and a path to the file.
    //
    // If the `YKD_TRACE_DEBUGINFO` environment variable is *not* set to "1", then no file is
    // created and this function returns `(None, -1, ptr::null())`.
    #[cfg(unix)]
    fn create_debuginfo_temp_file() -> (Option<NamedTempFile>, c_int, *const c_char) {
        let mut di_tmp = None;
        let mut di_fd = -1;
        let mut di_tmpname_c = ptr::null() as *const c_char;
        if let Ok(di_val) = env::var("YKD_TRACE_DEBUGINFO") {
            if di_val == "1" {
                let tmp = NamedTempFile::new().unwrap();
                di_tmpname_c = tmp.path().to_str().unwrap().as_ptr() as *const c_char;
                di_fd = tmp.as_raw_fd();
                di_tmp = Some(tmp);
            }
        }
        (di_tmp, di_fd, di_tmpname_c)
    }

    // pub fn compile_fast(&self) -> Result<(*const c_void, Option<NamedTempFile>), Box<dyn Error>> {
    //     use crate::frame::llvmbridge::ThreadSafeModule;
    //     let aot_tsm = ThreadSafeModule::aot_module();
    //     unsafe { LLVMOrcThreadSafeModuleWithModuleDo(aot_tsm.as_raw(), do_compile_fast, self as *const Self as *mut c_void) };
    //     todo!()
    // }

    pub fn compile(
        &self,
        use_fasttcg: bool,
    ) -> Result<(*const c_void, Option<NamedTempFile>), Box<dyn Error>> {
        let (func_names, bbs, trace_len) = self.encode_trace();

        let mut faddr_keys = Vec::new();
        let mut faddr_vals = Vec::new();
        for k in self.faddrs.iter() {
            faddr_keys.push(k.0.as_ptr());
            faddr_vals.push(*k.1);
        }

        let (llvmbc_data, llvmbc_len) = llvmbc_section();
        let (di_tmp, di_fd, di_tmpname_c) = Self::create_debuginfo_temp_file();

        let ret = if use_fasttcg {
            // Generate the JIT module and compile it with FastTCG.
            let genres = unsafe {
                yktracec::__yktracec_irtrace_generate_jitmod(
                    func_names.as_ptr(),
                    bbs.as_ptr(),
                    trace_len,
                    faddr_keys.as_ptr(),
                    faddr_vals.as_ptr(),
                    faddr_keys.len(),
                    llvmbc_data,
                    llvmbc_len,
                    di_fd,
                    di_tmpname_c,
                )
            };
            // Get all the required data into Rust data structures.
            let jitmod = unsafe { Module::new(genres.jitmod as LLVMModuleRef) };
            let global_keys = unsafe {
                slice::from_raw_parts(genres.global_mappings_keys, genres.global_mappings_len)
            };
            let global_vals = unsafe {
                slice::from_raw_parts(genres.global_mappings_vals, genres.global_mappings_len)
            };
            let mut global_mappings = HashMap::new();
            for i in 0..genres.global_mappings_len {
                let key = global_keys[i] as LLVMValueRef;
                global_mappings.insert(key, global_vals[i]);
            }

            // Generate code!
            let tcg = FastTCG::new(jitmod, global_mappings);
            let (code, stackmaps) = tcg.codegen();

            let smbox = Box::new(stackmaps);

            // FIXME: Emulate Lukas' gross hack from compileModule().
            use std::mem;
            let yuck = unsafe { libc::calloc(5, mem::size_of::<usize>()) } as *mut usize;
            assert_ne!(yuck, ptr::null_mut());
            unsafe {
                *yuck = code as usize;
                *yuck.add(1) = Box::into_raw(smbox) as usize; // stackmap data (abused).
                *yuck.add(2) = 0; // Stackmap data size (unused for fasttcg).
                *yuck.add(3) = genres.aot_vars as usize; // LiveAOTVals.
                *yuck.add(4) = genres.guard_count; // GuardCount
            }
            yuck as *const c_void
        } else {
            // Generate the JIT module and compile it with LLVM.
            unsafe {
                yktracec::__yktracec_irtrace_compile(
                    func_names.as_ptr(),
                    bbs.as_ptr(),
                    trace_len,
                    faddr_keys.as_ptr(),
                    faddr_vals.as_ptr(),
                    faddr_keys.len(),
                    llvmbc_data,
                    llvmbc_len,
                    di_fd,
                    di_tmpname_c,
                )
            }
        };
        if ret.is_null() {
            Err("Could not compile trace.".into())
        } else {
            Ok((ret, di_tmp))
        }
    }

    #[cfg(feature = "yk_testing")]
    pub unsafe fn compile_for_tc_tests(&self, llvmbc_data: *const u8, llvmbc_len: u64) {
        let (func_names, bbs, trace_len) = self.encode_trace();
        let (_di_tmp, di_fd, di_tmpname_c) = Self::create_debuginfo_temp_file();

        // These would only need to be populated if we were to load the resulting compiled code
        // into the address space, which for trace compiler tests, we don't.
        let faddr_keys = Vec::new();
        let faddr_vals = Vec::new();

        let ret = yktracec::__yktracec_irtrace_compile_for_tc_tests(
            func_names.as_ptr(),
            bbs.as_ptr(),
            trace_len,
            faddr_keys.as_ptr(),
            faddr_vals.as_ptr(),
            faddr_keys.len(),
            llvmbc_data,
            llvmbc_len,
            di_fd,
            di_tmpname_c,
        );
        assert_ne!(ret, ptr::null());
    }
}

struct SendSyncConstPtr<T>(*const T);
unsafe impl<T> Send for SendSyncConstPtr<T> {}
unsafe impl<T> Sync for SendSyncConstPtr<T> {}

struct Guard {
    failed: u32,
    code: Option<SendSyncConstPtr<c_void>>,
}

/// A trace compiled into machine code. Note that these are passed around as raw pointers and
/// potentially referenced by multiple threads so, once created, instances of this struct can only
/// be updated if a lock is held or a field is atomic.
pub struct CompiledTrace {
    pub mt: Arc<MT>,
    /// A function which when called, executes the compiled trace.
    ///
    /// The argument to the function is a pointer to a struct containing the live variables at the
    /// control point. The exact definition of this struct is not known to Rust: the struct is
    /// generated at interpreter compile-time by ykllvm.
    entry: SendSyncConstPtr<c_void>,
    /// Parsed stackmap of this trace. We only need to read this once, and can then use it to
    /// lookup stackmap information for each guard failure as needed.
    pub smap: HashMap<u64, Vec<LiveVar>>,
    /// Pointer to heap allocated live AOT values.
    aotvals: SendSyncConstPtr<c_void>,
    /// List of guards containing hotness counts or compiled side traces.
    guards: Vec<Option<Guard>>,
    /// If requested, a temporary file containing the "source code" for the trace, to be shown in
    /// debuggers when stepping over the JITted code.
    ///
    /// (rustc incorrectly identifies this field as dead code. Although it isn't being "used", the
    /// act of storing it is preventing the deletion of the file via its `Drop`)
    #[allow(dead_code)]
    di_tmpfile: Option<NamedTempFile>,
}

use std::slice;
impl CompiledTrace {
    /// Create a `CompiledTrace` from a pointer to an array containing: the pointer to the compiled
    /// trace, the pointer to the stackmap and the size of the stackmap, and the pointer to the
    /// live AOT values.
    pub fn new(mt: Arc<MT>, data: *const c_void, di_tmpfile: Option<NamedTempFile>) -> Self {
        let slice = unsafe { slice::from_raw_parts(data as *const usize, 5) };
        let funcptr = slice[0] as *const c_void;
        let smptr = slice[1] as *const c_void;
        let smsize = slice[2];
        let aotvals = slice[3] as *mut c_void;
        let guardcount = slice[4] as usize;

        let fast_tcg = if let Ok(v) = env::var("YKD_USE_FASTTCG") {
            v == "1"
        } else {
            false
        };

        // FIXME: shouldn't be needed once fasttcg can write a stackmap.
        let smap = if !fast_tcg {
            // Parse the stackmap of this trace and cache it. The original data allocated by memman.cc
            // is now no longer needed and can be freed.
            let smslice = unsafe { slice::from_raw_parts(smptr as *mut u8, smsize) };
            let smap = StackMapParser::parse(smslice).unwrap();
            unsafe { libc::munmap(smptr as *mut c_void, smsize) };
            smap
        } else {
            let smap = unsafe { Box::from_raw(smptr as *mut _)};
            Box::into_inner(smap)
        };

        // We heap allocated this array in yktracec to pass the data here. Now that we've
        // extracted it we no longer need to keep the array around.
        unsafe { libc::free(data as *mut c_void) };
        Self {
            mt,
            entry: SendSyncConstPtr(funcptr),
            smap,
            aotvals: SendSyncConstPtr(aotvals),
            di_tmpfile,
            guards: Vec::with_capacity(guardcount),
        }
    }

    #[cfg(any(test, feature = "yk_testing"))]
    #[doc(hidden)]
    /// Create a `CompiledTrace` with null contents. This is unsafe and only intended for testing
    /// purposes where a `CompiledTrace` instance is required, but cannot sensibly be constructed
    /// without overwhelming the test. The resulting instance must not be inspected or executed.
    pub unsafe fn new_null(mt: Arc<MT>) -> Self {
        Self {
            mt,
            entry: SendSyncConstPtr(std::ptr::null()),
            smap: HashMap::new(),
            aotvals: SendSyncConstPtr(std::ptr::null()),
            di_tmpfile: None,
            guards: Vec::new(),
        }
    }

    pub fn aotvals(&self) -> *const c_void {
        self.aotvals.0
    }

    pub fn entry(&self) -> *const c_void {
        self.entry.0
    }
}

impl Drop for CompiledTrace {
    fn drop(&mut self) {
        // The memory holding the AOT live values needs to live as long as the trace. Now that we
        // no longer need the trace, this can be freed too.
        unsafe { libc::free(self.aotvals.0 as *mut c_void) };
    }
}

impl fmt::Debug for CompiledTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CompiledTrace {{ ... }}")
    }
}

/// A tracer is an object which can start / stop collecting traces. It may have its own
/// configuration, but that is dependent on the concrete tracer itself.
pub trait Tracer: Send + Sync {
    /// Start collecting a trace of the current thread.
    fn start_collector(self: Arc<Self>) -> Result<Box<dyn ThreadTracer>, Box<dyn Error>>;
}

/// Represents a thread which is currently tracing.
pub trait ThreadTracer {
    /// Stop collecting a trace of the current thread.
    fn stop_collector(self: Box<Self>) -> Result<Box<dyn UnmappedTrace>, InvalidTraceError>;
}

pub fn default_tracer_for_platform() -> Result<Arc<dyn Tracer>, Box<dyn Error>> {
    Ok(Arc::new(hwt::HWTracer::new()?))
}

pub trait UnmappedTrace: Send {
    fn map(self: Box<Self>, tracer: Arc<dyn Tracer>) -> Result<IRTrace, InvalidTraceError>;
}
