// Exporting parts of the LLVM C++ API not present in the LLVM C API.

#![allow(clippy::new_without_default)]

// FIXME: C++ exceptions may unwind over the Rust FFI?
// https://github.com/ykjit/yk/issues/426

use libc::{c_void, size_t};
use std::os::raw::c_char;

extern "C" {
    pub fn __ykllvmwrap_irtrace_compile(
        func_names: *const *const c_char,
        bbs: *const size_t,
        trace_len: size_t,
        faddr_keys: *const *const c_char,
        faddr_vals: *const *const c_void,
        faddr_len: size_t,
        llvmbc_data: *const u8,
        llvmbc_len: size_t,
    ) -> *const c_void;

    #[cfg(feature = "yk_testing")]
    pub fn __ykllvmwrap_irtrace_compile_for_tc_tests(
        func_names: *const *const c_char,
        bbs: *const size_t,
        trace_len: size_t,
        faddr_keys: *const *const c_char,
        faddr_vals: *const *const c_void,
        faddr_len: size_t,
        llvmbc_data: *const u8,
        llvmbc_len: size_t,
    ) -> *const c_void;

    pub fn __ykllvmwrap_find_bbmaps(
        bitcode_data: *const u8,
        bitcode_len: usize,
        start_addr: *mut size_t,
        end_addr: *mut size_t,
    ) -> bool;
}
