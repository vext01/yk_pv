use libc::size_t;
use std::convert::TryFrom;
use std::default::Default;
use std::ffi::{c_void, CString};
use std::os::raw::c_char;
use std::ptr;

use ykbh::SIRInterpreter;
use ykcompile::{TraceCompiler, REG_POOL};
use ykpack::{self, CguHash, Local, LocalDecl, TyIndex};
use yktrace::sir::{self, SirTrace, SIR};
use yktrace::tir::TirTrace;

#[no_mangle]
unsafe extern "C" fn __ykshim_sirtrace_len(sir_trace: *mut c_void) -> size_t {
    let trace = &mut *(sir_trace as *mut SirTrace);
    trace.len()
}

#[no_mangle]
unsafe extern "C" fn __ykshim_tirtrace_new(sir_trace: *mut c_void) -> *mut c_void {
    let sir_trace = &mut *(sir_trace as *mut SirTrace);
    Box::into_raw(Box::new(TirTrace::new(&SIR, &*sir_trace).unwrap())) as *mut c_void
}

#[no_mangle]
unsafe extern "C" fn __ykshim_tirtrace_len(tir_trace: *mut c_void) -> size_t {
    Box::from_raw(tir_trace as *mut TirTrace).len()
}

#[no_mangle]
unsafe extern "C" fn __ykshim_tirtrace_display(tir_trace: *mut c_void) -> *mut c_char {
    let tt = Box::from_raw(tir_trace as *mut TirTrace);
    let st = CString::new(format!("{}", tt)).unwrap();
    CString::into_raw(st)
}

#[no_mangle]
unsafe extern "C" fn __ykshim_body_ret_ty(
    sym: *mut c_char,
    ret_cgu: *mut CguHash,
    ret_idx: *mut TyIndex,
) {
    let sym = CString::from_raw(sym);
    let rv = usize::try_from(sir::RETURN_LOCAL.0).unwrap();
    let tyid = SIR.body(&sym.to_str().unwrap()).unwrap().local_decls[rv].ty;
    *ret_cgu = tyid.0;
    *ret_idx = tyid.1;
}

#[no_mangle]
unsafe extern "C" fn __ykshim_tracecompiler_default() -> *mut c_void {
    let tc = Box::new(TraceCompiler::new(Default::default(), Default::default()));
    Box::into_raw(tc) as *mut c_void
}

#[no_mangle]
unsafe extern "C" fn __ykshim_tracecompiler_drop(comp: *mut c_void) {
    Box::from_raw(comp as *mut TraceCompiler);
}

#[no_mangle]
unsafe extern "C" fn __ykshim_tracecompiler_insert_decl(
    tc: *mut c_void,
    local: Local,
    local_ty_cgu: CguHash,
    local_ty_index: TyIndex,
    referenced: bool,
) {
    let tc = &mut *(tc as *mut TraceCompiler);
    tc.local_decls.insert(
        local,
        LocalDecl {
            ty: (local_ty_cgu, local_ty_index),
            referenced,
        },
    );
}

/// Returns a string describing the register allocation of the specified local.
#[no_mangle]
unsafe extern "C" fn __ykshim_tracecompiler_local_to_location_str(
    tc: *mut c_void,
    local: u32,
) -> *mut c_char {
    let tc = &mut *(tc as *mut TraceCompiler);
    let rstr = format!("{:?}", tc.local_to_location(Local(local)));
    CString::new(rstr.as_str()).unwrap().into_raw()
}

#[no_mangle]
unsafe extern "C" fn __ykshim_tracecompiler_local_dead(tc: *mut c_void, local: u32) {
    let tc = &mut *(tc as *mut TraceCompiler);
    tc.local_dead(&Local(local)).unwrap();
}

#[no_mangle]
unsafe extern "C" fn __ykshim_tracecompiler_find_sym(sym: *mut c_char) -> *mut c_void {
    TraceCompiler::find_symbol(CString::from_raw(sym).to_str().unwrap())
        .unwrap_or_else(|_| ptr::null_mut())
}

#[no_mangle]
unsafe extern "C" fn __yktest_interpret_body(body_name: *mut c_char, icx: *mut u8) {
    let body = SIR
        .body(CString::from_raw(body_name).to_str().unwrap())
        .unwrap();
    let mut si = SIRInterpreter::new(&*body);
    si.set_trace_inputs(icx);
    si.interpret(body);
}

#[no_mangle]
unsafe extern "C" fn __yktest_reg_pool_size() -> usize {
    REG_POOL.len()
}
