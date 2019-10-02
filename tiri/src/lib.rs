// Copyright 2019 King's College London.
// Created by the Software Development Team <http://soft-dev.org/>.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Tiri -- TIR trace interpreter.
//!
//! No effort has been made to make this fast.

#![feature(exclusive_range_pattern)]
#![feature(test)]
extern crate test;

use yktrace::tir::{Guard, Statement, TirOp, TirTrace, LocalIndex, Rvalue, Constant, Local};
use std::convert::TryFrom;
use std::collections::{hash_map, HashMap};

const DUMP_CONTEXT: usize = 4;

/// Mutable interpreter state.
struct InterpState {
    /// The next position in the trace to interpret.
    pc: usize,
    /// Local variable store.
    locals: HashMap<u32, Constant>,
}

impl InterpState {
    fn new() -> Self {
        Self {
            pc: 0,
            locals: HashMap::new(),
        }
    }
}

/// The interpreter itself.
/// The struct itself holds only immutable program information.
pub struct Interp<'t> {
    trace: &'t TirTrace,
}

impl<'t> Interp<'t> {
    /// Create a new interpreter, using the TIR found in the `.yk_tir` section of the binary `bin`.
    pub fn new(trace: &'t TirTrace) -> Self {
        Self { trace }
    }

    /// Start interpreting the trace.
    pub fn run(&self) {
        let mut state = InterpState::new();

        // The main interpreter loop.
        loop {
            let op = self.trace.op(state.pc);
            self.dump(&state);
            match op {
                TirOp::Statement(stmt) => self.interp_stmt(&mut state, stmt),
                TirOp::Guard(grd) => self.interp_guard(&mut state, grd),
            }
        }
    }

    /// Prints diagnostic information about the interpreter state.
    /// Used for debugging.
    fn dump(&self, state: &InterpState) {
        // Dump the code.
        let start = match state.pc {
            0..DUMP_CONTEXT => 0,
            _ => state.pc - DUMP_CONTEXT,
        };
        let end = state.pc + DUMP_CONTEXT;

        eprintln!("[Begin Interpreter State Dump]");
        eprintln!("     pc: {}\n", state.pc);
        for idx in start..end {
            let op = self.trace.op(idx);
            let pc_str = if idx == state.pc {
                "->"
            } else {
                "  "
            };

            eprintln!("  {} {}: {}", pc_str, idx, op);
        }
        eprintln!();

        // Dump the locals.
        for (idx, val) in &state.locals {
            eprintln!("     ${}: {}", idx, val);
        }
        eprintln!("[End Interpreter State Dump]\n");
    }

    /// Interpret the specified statement.
    fn interp_stmt(&self, state: &mut InterpState, stmt: &Statement) {
        match stmt {
            Statement::Assign(var, rval) => {
                state.locals.insert(var.idx(), self.eval_rvalue(state, rval));
                state.pc += 1;
            },
            _ => panic!("unhandled statement: {}", stmt),
        }
    }

    fn eval_rvalue(&self, state: &InterpState, rval: &Rvalue) -> Constant {
        match rval {
            Rvalue::Constant(c) => c.clone(),
            Rvalue::Local(l) => self.local(state, l),
            _ => panic!("unimplemented rvalue eval"),
        }
    }

    fn local(&self, state: &InterpState, l: &Local) -> Constant {
        match state.locals.get(&l.idx()) {
            Some(c) => c.clone(),
            None => panic!("uninitialised read from ${}", l.idx()),
        }
    }

    /// Interpret the specified terminator.
    fn interp_guard(&self, state: &mut InterpState, _guard: &Guard) {
        unimplemented!();
    }
}

#[cfg(test)]
mod tests {
    use super::Interp;
    use test::black_box;
    use yktrace::{start_tracing, tir::TirTrace, TracingKind};

    // Some work to trace.
    #[inline(never)]
    fn work(x: usize, y: usize) -> usize {
        let mut res = 0;
        while res < y {
            res += x;
        }
        res
    }

    #[test]
    fn interp_simple_trace() {
        let tracer = start_tracing(Some(TracingKind::SoftwareTracing));
        let res = work(black_box(3), black_box(13));
        let sir_trace = tracer.stop_tracing().unwrap();
        assert_eq!(res, 15);


        //use yktrace::debug::print_sir_trace;
        //print_sir_trace(sir_trace.as_ref(), false);

        let tir_trace = TirTrace::new(sir_trace.as_ref()).unwrap();
        assert!(tir_trace.len() > 0);

        let interp = Interp::new(&tir_trace);
        interp.run();
    }
}
