// Copyright 2019 King's College London.
// Created by the Software Development Team <http://soft-dev.org/>.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Conceptually this module takes an ordered collection of SIR block locations and converts it
//! into a Tracing IR (TIR) Trace using the SIR found in the `.yk_sir` section of the currently
//! running executable.

use super::SirTrace;
use elf;
use fallible_iterator::FallibleIterator;
use std::{collections::HashMap, convert::TryFrom, env, io::Cursor};
#[cfg(debug_assertions)]
use ykpack::{BasicBlockIndex, Terminator};
use ykpack::{Body, Decoder, DefId, Pack, Statement};

// The SIR Map lets us look up a SIR body from the SIR DefId.
// The map is unique to the executable binary being traced (i.e. shared for all threads).
lazy_static! {
    static ref SIR_MAP: HashMap<DefId, Body> = {
        let ef = elf::File::open_path(env::current_exe().unwrap()).unwrap();
        let sec = ef.get_section(".yk_sir").expect("Can't find SIR section");
        let mut curs = Cursor::new(&sec.data);
        let mut dec = Decoder::from(&mut curs);

        let mut sir_map = HashMap::new();
        while let Some(pack) = dec.next().unwrap() {
            let Pack::Body(body) = pack;
            sir_map.insert(body.def_id.clone(), body);
        }
        sir_map
    };
}

/// A TIR trace is conceptually a straight-line path through the SIR with guarded speculation.
#[derive(Debug)]
pub struct TirTrace {
    ops: Vec<TirOp>
}

impl TirTrace {
    /// Create a TirTrace from a SirTrace.
    /// Returns Err if a DefId is encountered for which no SIR is available. In the error case, the
    /// trace built thus-far is returned inside the error.
    ///
    /// FIXME: This is not the final "intended" API. Normally we wouldn't care about the trace
    /// built thus-far in an error case, but since we have no sensible way to stop the tracer at
    /// the right time, inevitably traces can shoot off into code which has no SIR. In turn this
    /// invalidates the trace. We only return the trace built so far in the interim.
    pub(crate) fn new(trace: &'_ dyn SirTrace) -> Result<Self, Self> {
        let mut ops = Vec::new();
        let num_locs = trace.len();

        for blk_idx in 0..num_locs {
            let loc = trace.loc(blk_idx);
            eprintln!("-----------------");
            dbg!(loc);
            let body = match SIR_MAP.get(&DefId::from_sir_loc(loc)) {
                Some(b) => b,
                None => {
                    eprintln!("^^NO SIR!!!");
                    ops.push(TirOp::Unknown);
                    continue;
                }
            };
            let shadow_bb_idx_usize = usize::try_from(loc.bb_idx()).unwrap();
            // Here we use an invariant of the MIR transform to find the user block for a shadow
            // block. In the blocks vector, first come N shadow blocks, then come N corresponding
            // user blocks. The debug assertion checks the invariant holds by looking at where the
            // shadow block returns to after the call to the trace recorder.
            let user_bb_idx_usize = body.blocks.len() / 2 + shadow_bb_idx_usize;
            eprintln!("{}", body.blocks[user_bb_idx_usize]);
            #[cfg(debug_assertions)]
            match body.blocks[shadow_bb_idx_usize].term {
                Terminator::Call { ret_bb, .. } => debug_assert!(
                    ret_bb == Some(BasicBlockIndex::try_from(user_bb_idx_usize).unwrap())
                ),
                _ => panic!("shadow invariant doesn't hold")
            }

            // When adding statements to the trace, we clone them (rather than referencing the
            // statements in the SIR_MAP) so that we have the freedom to mutate them later.
            ops.extend(
                body.blocks[user_bb_idx_usize]
                    .stmts
                    .iter()
                    .cloned()
                    .map(TirOp::Statement)
            );

            // FIXME: Convert the block terminator to a guard if necessary.
        }
        Ok(Self { ops })
    }

    /// Return the length of the trace measure in operations.
    pub fn len(&self) -> usize {
        self.ops.len()
    }
}

/// A TIR operation. A collection of these makes a TIR trace.
#[derive(Debug)]
pub enum TirOp {
    Statement(Statement), // FIXME guards
    // FIXME: I hope we can remove this in the future.
    /// We were unable to find the SIR for a block.
    Unknown,

}

#[cfg(test)]
mod tests {
    // Some work to trace.
    #[inline(never)]
    fn work(x: usize, y: usize) -> usize {
        let mut res = 0;
        while res < y {
            res += x;
        }
        res
    }

    use crate::{start_tracing, TirTrace, TracingKind};
    use test::black_box;

    #[test]
    fn nonempty_tir_trace() {
        let tracer = start_tracing(Some(TracingKind::SoftwareTracing));
        let res = black_box(work(black_box(3), black_box(13)));
        let sir_trace = tracer.t_impl.stop_tracing().unwrap();
        let tir_trace = TirTrace::new(&*sir_trace).unwrap();
        assert_eq!(res, 15);
        dbg!(tir_trace.len());
        assert!(tir_trace.len() > 0);
    }
}
