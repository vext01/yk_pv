//! A wrapper around llvm::symbolizer::LLVMSymbolizer.

use libc::free;
use std::{
    ffi::{c_void, CStr, CString},
    ops::Drop,
    os::raw::c_char,
    path::Path,
};

extern "C" {
    fn __yk_llvmwrap_symbolizer_new() -> *mut c_void;
    fn __yk_llvmwrap_symbolizer_free(symbolizer: *mut c_void);
    fn __yk_llvmwrap_symbolizer_find_code_sym(
        symbolizer: *mut c_void,
        obj: *const c_char,
        off: u64,
    ) -> *mut c_char;
}

pub struct Symbolizer(*mut c_void);

impl Symbolizer {
    pub fn new() -> Self {
        Self(unsafe { __yk_llvmwrap_symbolizer_new() })
    }

    /// Returns the name of the symbol at byte offset `off` in the object file `obj`,
    /// or `None` if the symbol couldn't be found.
    pub fn find_code_sym(&self, obj: &Path, off: u64) -> Option<CString> {
        let obj_c = CString::new(obj.to_str().unwrap()).unwrap();
        let ptr = unsafe {
            __yk_llvmwrap_symbolizer_find_code_sym(self.0, obj_c.as_ptr() as *const i8, off)
        };
        if ptr.is_null() {
            None
        } else {
            let sym = unsafe { CStr::from_ptr(ptr) };
            if sym.to_bytes() == b"<invalid>" {
                return None;
            }
            // We can't take ownership of a heap-allocated C string, so we copy it and free the old
            // one.
            let ret = CString::from(sym);
            unsafe { free(ptr as *mut _) };
            Some(ret)
        }
    }
}

impl Drop for Symbolizer {
    fn drop(&mut self) {
        unsafe { __yk_llvmwrap_symbolizer_free(self.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::Symbolizer;
    use ykutil::addr::code_vaddr_to_off;

    extern "C" {
        fn getuid();
    }

    #[inline(never)]
    fn symbolize_me_mangled() {}

    // This function has a different signature to the one above to prevent LLVM from merging the
    // functions (and their symbols) when optimising in --release mode.
    #[inline(never)]
    #[no_mangle]
    fn symbolize_me_unmangled() -> u8 {
        1
    }

    #[test]
    fn find_code_sym_mangled() {
        let f_vaddr = symbolize_me_mangled as *const fn() as *const _ as usize;
        let (obj, f_off) = code_vaddr_to_off(f_vaddr).unwrap();
        let s = Symbolizer::new();
        let sym = s.find_code_sym(&obj, f_off).unwrap();
        let sym = sym.to_str().unwrap();
        // The symbol will be suffixed with an auto-generated module name, e.g.:
        // ykllvmwrap::symbolizer::tests::symbolize_me_mangled::hc7a76ddceae6f9c4
        assert!(sym.starts_with("ykllvmwrap::symbolizer::tests::symbolize_me_mangled::"));
        let elems = sym.split("::");
        assert_eq!(elems.count(), 5);
    }

    #[test]
    fn find_code_sym_unmangled() {
        let f_vaddr = symbolize_me_unmangled as *const fn() as *const _ as usize;
        let (obj, f_off) = code_vaddr_to_off(f_vaddr).unwrap();
        let s = Symbolizer::new();
        let sym = s.find_code_sym(&obj, f_off).unwrap();
        assert_eq!(sym.to_str().unwrap(), "symbolize_me_unmangled");
    }

    #[test]
    fn find_code_sym_libc() {
        let f_vaddr = getuid as *const fn() as *const _ as usize;
        let (obj, f_off) = code_vaddr_to_off(f_vaddr).unwrap();
        let s = Symbolizer::new();
        let sym = s.find_code_sym(&obj, f_off).unwrap();
        assert_eq!(sym.to_str().unwrap(), "getuid");
    }
}
