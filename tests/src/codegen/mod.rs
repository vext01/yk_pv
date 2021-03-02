//! Tests for the code generator (compiling TIR traces to native code).

use crate::helpers::{add6, add_some};
use libc;
use libc::{abs, getuid};
use paste::paste;
use ykshim_client::{compile_tir_trace, compile_trace, start_tracing, TirTrace, TracingKind};

mod reg_alloc;

#[test]
fn simple() {
    struct InterpCtx(u8);

    #[interp_step]
    #[inline(never)]
    fn simple(io: &mut InterpCtx) {
        let x = 13;
        io.0 = x;
    }

    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    simple(&mut InterpCtx(0));
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 13);
}

#[inline(never)]
fn farg(i: u8) -> u8 {
    i
}

#[test]
fn function_call_simple() {
    struct InterpCtx(u8);

    #[interp_step]
    #[inline(never)]
    fn fcall(io: &mut InterpCtx) {
        io.0 = farg(13);
        let _z = farg(14);
    }

    let mut io = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    fcall(&mut io);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 13);
}

#[test]
fn function_call_nested() {
    struct InterpCtx(u8);

    fn fnested3(i: u8, _j: u8) -> u8 {
        let c = i;
        c
    }

    fn fnested2(i: u8) -> u8 {
        fnested3(i, 10)
    }

    #[interp_step]
    fn fnested(io: &mut InterpCtx) {
        io.0 = fnested2(20);
    }

    let mut io = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    fnested(&mut io);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 20);
}

// A trace which contains a call to something which we don't have SIR for should emit a TIR
// call operation.
/// Execute a trace which calls a symbol accepting no arguments, but which does return a value.
#[test]
fn exec_call_symbol_no_args() {
    struct InterpCtx(u32);
    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        io.0 = unsafe { getuid() };
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let mut args = InterpCtx(0);
    let ct = compile_trace(sir_trace).unwrap();
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(ctx.0, args.0);
}

/// Execute a trace which calls a symbol accepting arguments and returns a value.
#[test]
fn exec_call_symbol_with_arg() {
    struct InterpCtx(i32);
    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        io.0 = unsafe { abs(io.0) };
    }

    let mut ctx = InterpCtx(-56);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let mut args = InterpCtx(-56);
    let ct = compile_trace(sir_trace).unwrap();
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(ctx.0, args.0);
}

/// The same as `exec_call_symbol_args_with_rv`, just using a constant argument.
#[test]
fn exec_call_symbol_with_const_arg() {
    struct InterpCtx(i32);
    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        io.0 = unsafe { abs(-123) };
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(ctx.0, args.0);
}

#[test]
fn exec_call_symbol_with_many_args() {
    struct InterpCtx(u64);
    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        io.0 = unsafe { add6(1, 2, 3, 4, 5, 6) };
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(ctx.0, 21);
    assert_eq!(ctx.0, args.0);
}

#[test]
fn exec_call_symbol_with_many_args_some_ignored() {
    struct InterpCtx(u64);
    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        io.0 = unsafe { add_some(1, 2, 3, 4, 5) };
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 7);
    assert_eq!(args.0, ctx.0);
}

#[test]
fn ext_call_and_spilling() {
    struct InterpCtx(u64);

    #[interp_step]
    fn ext_call(io: &mut InterpCtx) {
        let a = 1;
        let b = 2;
        let c = 3;
        let d = 4;
        let e = 5;
        // When calling `add_some` argument `a` is loaded from a register, while the remaining
        // arguments are loaded from the stack.
        let expect = unsafe { add_some(a, b, c, d, e) };
        io.0 = expect;
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    ext_call(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(ctx.0, 7);
    assert_eq!(ctx.0, args.0);
}

/// Generates a test for a binary operation.
macro_rules! mk_binop_test {
    ($name: ident, $op: tt, $type: ident, $arg1: expr, $arg2: expr, $expect: expr) => {
        paste! {
            #[test]
            fn [<$name _ $type>]() {
                dbg!(&$arg1, &$arg2);
                #[derive(Eq, PartialEq, Debug)]
                struct BinopCtx {
                    arg1: $type,
                    arg2: $type,
                    res: $type,
                }

                impl BinopCtx {
                    fn new(arg1: $type, arg2: $type, res: $type) -> Self {
                        Self { arg1, arg2, res }
                    }
                }

                #[interp_step]
                fn interp_step(ctx: &mut BinopCtx) {
                    ctx.res = ctx.arg1 $op ctx.arg2;
                }

                let mut ctx = BinopCtx::new($arg1, $arg2, 0);
                #[cfg(tracermode = "hw")]
                let th = start_tracing(TracingKind::HardwareTracing);
                #[cfg(tracermode = "sw")]
                let th = start_tracing(TracingKind::SoftwareTracing);
                interp_step(&mut ctx);
                let sir_trace = th.stop_tracing().unwrap();
                let ct = compile_trace(sir_trace).unwrap();

                let mut args = BinopCtx::new($arg1, $arg2, 0);
                assert!(unsafe { ct.execute(&mut args).is_null() });
                assert_eq!(args, BinopCtx::new($arg1, $arg2, $expect));
            }
        }
    };
}

/// Generates binary operation tests for all unsigned types.
/// Since all types are tested, numeric operands must fit in a u8.
macro_rules! mk_binop_tests_unsigned {
    ($name: ident, $op: tt, $arg1: expr, $arg2: expr, $expect: expr) => {
        mk_binop_test!($name, $op, u8, $arg1, $arg2, $expect);
        mk_binop_test!($name, $op, u16, $arg1, $arg2, $expect);
        mk_binop_test!($name, $op, u32, $arg1, $arg2, $expect);
        mk_binop_test!($name, $op, u64, $arg1, $arg2, $expect);
        // FIXME u128 hits unreachable code.
    };
}

/// Generates binary operation tests for all signed types.
/// Since all types are tested, numeric operands must fit in an i8.
macro_rules! mk_binop_tests_signed {
    ($name: ident, $op: tt, $arg1: expr, $arg2: expr, $expect: expr) => {
        mk_binop_test!($name, $op, i8, $arg1, $arg2, $expect);
        mk_binop_test!($name, $op, i16, $arg1, $arg2, $expect);
        mk_binop_test!($name, $op, i32, $arg1, $arg2, $expect);
        mk_binop_test!($name, $op, i64, $arg1, $arg2, $expect);
        // FIXME i128 hits unreachable code.
    };
}

mk_binop_tests_unsigned!(binop_add1, +, 0, 0, 0);
mk_binop_tests_signed!(binop_add2, +, 0, 0, 0);
mk_binop_tests_unsigned!(binop_add3, +, 1, 1, 2);
mk_binop_tests_signed!(binop_add4, +, 1, 1, 2);
mk_binop_tests_unsigned!(binop_add5, +, 253, 2, 255);
mk_binop_tests_signed!(binop_add6, +, 125, 2, 127);
mk_binop_test!(binop_add7, +, u16, u16::MAX - 7, 7, u16::MAX);
mk_binop_test!(binop_add8, +, u32, u32::MAX - 14, 14, u32::MAX);
mk_binop_test!(binop_add9, +, u64, u64::MAX - 100, 100, u64::MAX);
mk_binop_test!(binop_add10, +, i16, i16::MAX - 7, 7, i16::MAX);
mk_binop_test!(binop_add11, +, i32, i32::MAX - 14, 14, i32::MAX);
mk_binop_test!(binop_add13, +, i64, i64::MAX - 100, 100, i64::MAX);

mk_binop_tests_unsigned!(binop_sub1, -, 0, 0, 0);
mk_binop_tests_signed!(binop_sub2, -, 0, 0, 0);
mk_binop_tests_unsigned!(binop_sub3, -, 1, 0, 1);
mk_binop_tests_signed!(binop_sub4, -, 1, 0, 1);
mk_binop_tests_signed!(binop_sub5, -, 0, 1, -1);
mk_binop_tests_signed!(binop_sub6, -, -120, 8, -128);
mk_binop_tests_signed!(binop_sub7, -, -1, -1, 0);
mk_binop_test!(binop_sub8, -, u16, u16::MAX, 7, u16::MAX - 7);
mk_binop_test!(binop_sub9, -, u32, u32::MAX, 8, u32::MAX - 8);
mk_binop_test!(binop_sub10, -, u64, u64::MAX, 33, u64::MAX - 33);
mk_binop_test!(binop_sub11, -, i16, i16::MAX, 7, i16::MAX - 7);
mk_binop_test!(binop_sub12, -, i32, i32::MAX, 8, i32::MAX - 8);
mk_binop_test!(binop_sub13, -, i64, i64::MAX, 33, i64::MAX - 33);

// FIXME implement and test signed multiplication.
mk_binop_tests_unsigned!(binop_mul1, *, 0, 0, 0);
mk_binop_tests_unsigned!(binop_mul2, *, 10, 10, 100);
mk_binop_tests_unsigned!(binop_mul3, *, 15, 15, 225);
mk_binop_test!(binop_mul4, *, u16, 510, 8, 4080);
mk_binop_test!(binop_mul5, *, u32, 131072, 8, 1048576);
mk_binop_test!(binop_mul5, *, u64, 8589934592u64, 8, 68719476736);

// FIXME implement and test signed division.
mk_binop_tests_unsigned!(binop_div1, /, 1, 1, 1);
mk_binop_tests_unsigned!(binop_div2, /, 2, 1, 2);
mk_binop_tests_unsigned!(binop_div3, /, 252, 4, 63);
mk_binop_test!(binop_div4, /, u16, 4080, 8, 510);
mk_binop_test!(binop_div5, /, u32, 1048576, 8, 131072);
mk_binop_test!(binop_div6, /, u64, 68719476736u64, 8, 8589934592);

#[test]
fn binop_add_overflow() {
    #[derive(Eq, PartialEq, Debug)]
    struct InterpCtx(u8, u8);

    #[interp_step]
    fn interp_stepx(io: &mut InterpCtx) {
        io.1 = io.0 + 1;
    }

    let mut ctx = InterpCtx(254, 0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_stepx(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    assert_eq!(ctx.1, 255);
    let ct = compile_trace(sir_trace).unwrap();

    // Executing a trace with no overflow shouldn't fail any guards.
    let mut args = InterpCtx(10, 0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args, InterpCtx(10, 11));

    // Executing a trace *with* overflow will fail a guard.
    let mut args = InterpCtx(255, 5);
    assert!(!unsafe { ct.execute(&mut args).is_null() });
}

#[test]
fn binop_other() {
    #[derive(Eq, PartialEq, Debug)]
    struct InterpCtx(u64, u64, u64);

    #[interp_step]
    fn interp_stepx(io: &mut InterpCtx) {
        io.2 = io.0 * 3 - 5;
        io.1 = io.2 / 2;
    }

    let mut ctx = InterpCtx(5, 2, 0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_stepx(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(5, 2, 0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args, InterpCtx(5, 5, 10));
}

#[test]
fn ref_deref_simple() {
    #[derive(Debug)]
    struct InterpCtx(u64);

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        let mut x = 9;
        let y = &mut x;
        *y = 10;
        io.0 = *y;
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 10);
}

#[test]
fn ref_deref_double() {
    #[derive(Debug)]
    struct InterpCtx(u64);

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        let mut x = 9;
        let y = &mut &mut x;
        **y = 4;
        io.0 = x;
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 4);
}

#[test]
fn ref_deref_double_and_field() {
    #[derive(Debug)]
    struct InterpCtx(u64);

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        let five = 5;
        let mut s = (4u64, &five);
        let y = &mut s;
        io.0 = *y.1;
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 5);
}

#[test]
fn ref_deref_stack() {
    struct InterpCtx(u64);

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        let _a = 1;
        let _b = 2;
        let _c = 3;
        let _d = 4;
        let _e = 5;
        let _f = 6;
        let mut x = 9;
        let y = &mut x;
        *y = 10;
        let z = *y;
        io.0 = z
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 10);
}

/// Dereferences a variable that lives on the stack and stores it in a register.
#[test]
fn deref_stack_to_register() {
    fn deref1(arg: u64) -> u64 {
        let a = &arg;
        return *a;
    }

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        let _a = 1;
        let _b = 2;
        let _c = 3;
        let f = 6;
        io.0 = deref1(f);
    }

    struct InterpCtx(u64);
    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 6);
}

#[test]
fn deref_register_to_stack() {
    struct InterpCtx(u64);

    fn deref2(arg: u64) -> u64 {
        let a = &arg;
        let _b = 2;
        let _c = 3;
        let _d = 4;
        return *a;
    }

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        let f = 6;
        io.0 = deref2(f);
    }

    // This test dereferences a variable that lives on the stack and stores it in a register.
    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args) }.is_null());
    assert_eq!(args.0, 6);
}

#[test]
fn do_not_trace() {
    struct InterpCtx(u8);

    #[do_not_trace]
    fn dont_trace_this(a: u8) -> u8 {
        let b = 2;
        let c = a + b;
        c
    }

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        io.0 = dont_trace_this(io.0);
    }

    let mut ctx = InterpCtx(1);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let tir_trace = TirTrace::new(&sir_trace);

    let ct = compile_tir_trace(tir_trace).unwrap();
    let mut args = InterpCtx(1);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 3);
}

#[test]
fn do_not_trace_stdlib() {
    struct InterpCtx<'a>(&'a mut Vec<u64>);

    #[interp_step]
    fn dont_trace_stdlib(io: &mut InterpCtx) {
        io.0.push(3);
    }

    let mut vec: Vec<u64> = Vec::new();
    let mut ctx = InterpCtx(&mut vec);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    dont_trace_stdlib(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut argv: Vec<u64> = Vec::new();
    let mut args = InterpCtx(&mut argv);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(argv.len(), 1);
    assert_eq!(argv[0], 3);
}

#[test]
fn projection_chain() {
    #[derive(Debug)]
    struct InterpCtx((usize, u8, usize), u8, S, usize);

    #[derive(Debug, PartialEq)]
    struct S {
        x: usize,
        y: usize,
    }

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        io.1 = (io.0).1;
        io.3 = io.2.y;
    }

    let s = S { x: 5, y: 6 };
    let t = (1, 2, 3);
    let mut ctx = InterpCtx(t, 0u8, s, 0usize);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();

    let t2 = (1, 2, 3);
    let s2 = S { x: 5, y: 6 };
    let mut args = InterpCtx(t2, 0u8, s2, 0usize);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, (1usize, 2u8, 3usize));
    assert_eq!(args.1, 2u8);
    assert_eq!(args.2, S { x: 5, y: 6 });
    assert_eq!(args.3, 6);
}

#[test]
fn projection_lhs() {
    struct InterpCtx((u8, u8), u8);

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        (io.0).1 = io.1;
    }

    let t = (1u8, 2u8);
    let mut ctx = InterpCtx(t, 3u8);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let t2 = (1u8, 2u8);
    let mut args = InterpCtx(t2, 3u8);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!((args.0).1, 3);
}

#[test]
fn array() {
    struct InterpCtx<'a>(&'a mut [u8; 3], u8);

    #[interp_step]
    #[inline(never)]
    fn array(io: &mut InterpCtx) {
        let z = io.0[1];
        io.1 = z;
    }

    let mut a = [3, 4, 5];
    let mut ctx = InterpCtx(&mut a, 0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    array(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    assert_eq!(ctx.1, 4);
    let ct = compile_trace(sir_trace).unwrap();
    let mut a2 = [3, 4, 5];
    let mut args = InterpCtx(&mut a2, 0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.1, 4);
}

#[test]
fn array_nested() {
    struct InterpCtx<'a>(&'a mut [[u8; 3]; 2], u8);

    #[interp_step]
    #[inline(never)]
    fn array(io: &mut InterpCtx) {
        let z = io.0[1][2];
        io.1 = z;
    }

    let mut a = [[3, 4, 5], [6, 7, 8]];
    let mut ctx = InterpCtx(&mut a, 0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    array(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    assert_eq!(ctx.1, 8);
    let ct = compile_trace(sir_trace).unwrap();
    let mut a2 = [[3, 4, 5], [6, 7, 8]];
    let mut args = InterpCtx(&mut a2, 0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.1, 8);
}

#[test]
fn array_nested_mad() {
    struct S([u16; 4]);
    struct InterpCtx<'a>(&'a mut [S; 3], u16);

    #[interp_step]
    #[inline(never)]
    fn array(io: &mut InterpCtx) {
        let z = io.0[2].0[2];
        io.1 = z;
    }

    let mut a = [S([3, 4, 5, 6]), S([7, 8, 9, 10]), S([11, 12, 13, 14])];
    let mut ctx = InterpCtx(&mut a, 0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    array(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    assert_eq!(ctx.1, 13);
    let ct = compile_trace(sir_trace).unwrap();
    let mut a2 = [S([3, 4, 5, 6]), S([7, 8, 9, 10]), S([11, 12, 13, 14])];
    let mut args = InterpCtx(&mut a2, 0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.1, 13);
}

/// Test codegen of field access on a struct ref on the right-hand side.
#[test]
fn rhs_struct_ref_field() {
    struct InterpCtx(u8);

    #[interp_step]
    fn add1(io: &mut InterpCtx) {
        io.0 = io.0 + 1
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    add1(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();

    let mut args = InterpCtx(10);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 11);
}

/// Test codegen of indexing a struct ref on the left-hand side.
#[test]
fn mut_lhs_struct_ref() {
    struct InterpCtx(u8);

    #[interp_step]
    fn set100(io: &mut InterpCtx) {
        io.0 = 100;
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    set100(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();

    let mut args = InterpCtx(10);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 100);
}

/// Test codegen of copying something which doesn't fit in a register.
#[test]
fn place_larger_than_reg() {
    #[derive(Debug, Eq, PartialEq)]
    struct S(u64, u64, u64);
    struct InterpCtx(S);

    #[interp_step]
    fn ten(io: &mut InterpCtx) {
        io.0 = S(10, 10, 10);
    }

    let mut ctx = InterpCtx(S(0, 0, 0));
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    ten(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    assert_eq!(ctx.0, S(10, 10, 10));

    let mut args = InterpCtx(S(1, 1, 1));
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, S(10, 10, 10));
}

#[test]
fn array_slice_index() {
    struct InterpCtx<'a>(&'a [u8], u8);

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        io.1 = io.0[2];
    }

    let a = [1, 2, 3];
    let mut ctx = InterpCtx(&a, 0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(&a, 0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.1, 3);
}

// Only `interp_step` annotated functions and their callees should remain after trace trimming.
#[test]
fn trim_junk() {
    struct InterpCtx(u8);

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        io.0 += 1;
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    ctx.0 = 0; // Should get trimmed.
    interp_step(&mut ctx);
    ctx.0 = 0; // Should get trimmed
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();

    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 3);
}

#[test]
fn comparison() {
    struct InterpCtx(u8, bool);

    fn checks(i: u8) -> bool {
        let a = i == 0;
        let b = i > 1;
        let c = i < 1;
        if a && b || c {
            true
        } else {
            false
        }
    }

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        let x = checks(io.0);
        io.1 = x;
    }

    let mut ctx = InterpCtx(0, false);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0, false);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.1, true);
}

#[test]
fn guard() {
    struct InterpCtx(u8, u8);

    fn guard(i: u8) -> u8 {
        if i != 3 {
            9
        } else {
            10
        }
    }

    #[interp_step]
    fn interp_step(io: &mut InterpCtx) {
        let x = guard(io.0);
        io.1 = x;
    }

    let mut ctx = InterpCtx(std::hint::black_box(|i| i)(0), 0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0, 0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.1, 9);
    // Execute the trace with a context that causes a guard to fail.
    let mut args = InterpCtx(3, 0);
    let ptr = unsafe { ct.execute(&mut args) };
    assert!(!ptr.is_null());
}

#[test]
fn matching() {
    struct InterpCtx(u8);

    #[interp_step]
    #[inline(never)]
    fn matchthis(io: &mut InterpCtx) {
        let x = match io.0 {
            1 => 2,
            2 => 3,
            _ => 0,
        };
        io.0 = x;
    }

    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    matchthis(&mut InterpCtx(1));
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(1);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 2);
}

#[test]
fn cast() {
    struct InterpCtx(u16, u8);

    #[interp_step]
    #[inline(never)]
    fn matchthis(io: &mut InterpCtx) {
        let y = match io.1 as char {
            'a' => 1,
            'b' => 2,
            _ => 3,
        };
        io.0 = y;
    }

    let mut io = InterpCtx(0, 97);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    matchthis(&mut io);
    let sir_trace = th.stop_tracing().unwrap();
    assert_eq!(io.0, 1);
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0, 97);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 1);
}

#[test]
fn vec_add() {
    struct InterpCtx {
        ptr: usize,
        cells: Vec<u8>,
    }

    #[interp_step]
    #[inline(never)]
    fn vec_add(io: &mut InterpCtx) {
        io.cells[io.ptr] = io.cells[io.ptr].wrapping_add(1);
    }

    let cells = vec![0, 1, 2];
    let mut io = InterpCtx { ptr: 1, cells };
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    vec_add(&mut io);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let cells = vec![1, 2, 3];
    let mut args = InterpCtx { ptr: 1, cells };
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.cells, vec![1, 3, 3]);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.cells, vec![1, 4, 3]);
}

/// Check that calling a `do_not_trace` annotated function from within a regular (but
/// non-interp-step) function works.
#[test]
fn nested_do_not_trace() {
    #[do_not_trace]
    fn one() -> usize {
        1
    }

    fn call_one() -> usize {
        one()
    }

    struct InterpCtx(usize);

    #[interp_step]
    #[inline(never)]
    fn interp_step(io: &mut InterpCtx) {
        io.0 = call_one();
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 1);
}

#[test]
fn recursive_do_not_trace() {
    #[do_not_trace]
    fn rec(i: u8) -> u8 {
        let mut j = i;
        if i < 10 {
            j = rec(i + 1);
        }
        j
    }

    struct InterpCtx(u8);

    #[interp_step]
    #[inline(never)]
    fn interp_step(io: &mut InterpCtx) {
        io.0 = rec(1);
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 10);
}

#[test]
fn mut_recursive_do_not_trace() {
    fn rec2(i: u8) -> u8 {
        rec(i + 1)
    }

    #[do_not_trace]
    fn rec(i: u8) -> u8 {
        let mut j = i;
        if i < 10 {
            j = rec2(i);
        }
        j
    }

    struct InterpCtx(u8);

    #[interp_step]
    #[inline(never)]
    fn interp_step(io: &mut InterpCtx) {
        io.0 = rec(1);
    }

    let mut ctx = InterpCtx(0);
    #[cfg(tracermode = "hw")]
    let th = start_tracing(TracingKind::HardwareTracing);
    #[cfg(tracermode = "sw")]
    let th = start_tracing(TracingKind::SoftwareTracing);
    interp_step(&mut ctx);
    let sir_trace = th.stop_tracing().unwrap();
    let ct = compile_trace(sir_trace).unwrap();
    let mut args = InterpCtx(0);
    assert!(unsafe { ct.execute(&mut args).is_null() });
    assert_eq!(args.0, 10);
}
