//! ykpack -- Serialiser and deserialiser for carrying data from compile-time to run-time.
//!
//! This crate allows ykrustc to serialise various compile-time information for later
//! deserialisation by the Yorick runtime.
//!
//! The encoder and decoder API is structured in such a way that each item -- or "Pack" -- can be
//! streamed to/from the serialised format one item at a time. This helps to reduce memory
//! consumption.
//!
//! The MIR data is serialised in the msgpack format in the following form:
//!
//!  -----------
//!  pack_0:             \
//!  ...                  - Packs.
//!  pack_n              /
//!  sentinel           -- End of packs marker.
//!  -----------
//!
//!  Where each pack_i is an instance of `Some(Pack)` and the sentinel is a `None`.
//!
//!  The version field is automatically written and checked by the `Encoder` and `Decoder`
//!  respectively.

#![feature(yk_swt)]

mod decode;
mod encode;
mod types;

pub use decode::Decoder;
pub use encode::Encoder;
pub use types::*;

/// The prefix used in `DILabel` names for blocks.
pub const BLOCK_LABEL_PREFIX: &str = "__YK_BLK";

// ELF sections with this prefix contain SIR.
pub const SIR_SECTION_PREFIX: &str = ".yksir_";

#[cfg(test)]
mod tests {
    use super::{
        BasicBlock, BinOp, Body, Constant, ConstantInt, Decoder, Encoder, Local, Operand, Pack,
        Place, PlaceBase, PlaceProjection, Rvalue, Statement, Terminator, UnsignedInt,
    };
    use fallible_iterator::{self, FallibleIterator};
    use std::io::{Cursor, Seek, SeekFrom};

    // Get a cursor to serialise to and deserialise from. For real, we'd be reading from a file,
    // but for tests we use a vector of bytes.
    fn get_curs() -> Cursor<Vec<u8>> {
        let buf: Vec<u8> = Vec::new();
        Cursor::new(buf)
    }

    // Rewind a cursor to the beginning.
    fn rewind_curs(curs: &mut Cursor<Vec<u8>>) {
        curs.seek(SeekFrom::Start(0)).unwrap();
    }

    // Makes some sample stuff to round trip test.
    fn get_sample_packs() -> Vec<Pack> {
        let dummy_term = Terminator::Goto(10);

        let stmts1_b1 = vec![Statement::Nop; 16];
        let stmts1_b2 = vec![Statement::Nop; 3];
        let blocks1 = vec![
            BasicBlock::new(stmts1_b1, dummy_term.clone()),
            BasicBlock::new(stmts1_b2, dummy_term.clone()),
        ];
        let sir1 = Pack::Body(Body {
            symbol_name: String::from("symbol1"),
            blocks: blocks1,
            flags: 0,
        });

        let stmts2_b1 = vec![Statement::Nop; 7];
        let stmts2_b2 = vec![Statement::Nop; 200];
        let stmts2_b3 = vec![Statement::Nop; 1];
        let blocks2 = vec![
            BasicBlock::new(stmts2_b1, dummy_term.clone()),
            BasicBlock::new(stmts2_b2, dummy_term.clone()),
            BasicBlock::new(stmts2_b3, dummy_term.clone()),
        ];
        let sir2 = Pack::Body(Body {
            symbol_name: String::from("symbol2"),
            blocks: blocks2,
            flags: 0,
        });

        vec![sir1, sir2]
    }

    // Check serialising and deserialising works for zero packs.
    #[test]
    fn test_empty() {
        let mut curs = get_curs();

        let enc = Encoder::from(&mut curs);
        enc.done().unwrap();

        rewind_curs(&mut curs);
        let mut dec = Decoder::from(&mut curs);
        assert!(dec.next().unwrap().is_none());
    }

    // Check a typical serialising and deserialising session.
    #[test]
    fn test_basic() {
        let inputs = get_sample_packs();
        let mut curs = get_curs();

        let mut enc = Encoder::from(&mut curs);
        for md in &inputs {
            enc.serialise(md.clone()).unwrap();
        }
        enc.done().unwrap();

        rewind_curs(&mut curs);
        let dec = Decoder::from(&mut curs);

        // Obtain two fallible iterators, so we can zip them.
        let expect_iter = fallible_iterator::convert(inputs.into_iter().map(|e| Ok(e)));

        let mut itr = dec.zip(expect_iter);
        while let Some((got, expect)) = itr.next().unwrap() {
            assert_eq!(expect, got);
        }
    }

    #[test]
    fn test_text_dump() {
        let stmts_t1_b0 = vec![
            Statement::Assign(Local(0), Rvalue::from(Local(1))),
            Statement::Assign(
                Local(2),
                Rvalue::Use(Operand::Place(Place {
                    base: PlaceBase::Local(Local(3)),
                    projections: vec![PlaceProjection::Field(4)],
                })),
            ),
            Statement::Assign(
                Local(4),
                Rvalue::Use(Operand::Constant(Constant::Int(ConstantInt::UnsignedInt(
                    UnsignedInt::U8(10),
                )))),
            ),
            Statement::Nop,
        ];
        let term_t1_b0 = Terminator::Goto(20);
        let stmts_t1_b1 = vec![
            Statement::Assign(Local(5), Rvalue::from(Local(6))),
            Statement::Assign(Local(5), Rvalue::from(Local(4))),
            Statement::Assign(
                Local(7),
                Rvalue::BinaryOp(BinOp::Add, Operand::from(Local(8)), Operand::from(Local(9))),
            ),
            Statement::Assign(
                Local(7),
                Rvalue::BinaryOp(
                    BinOp::Sub,
                    Operand::from(Local(9)),
                    Operand::from(Local(10)),
                ),
            ),
            Statement::Assign(
                Local(11),
                Rvalue::Use(Operand::Constant(Constant::Int(ConstantInt::u8_from_bits(
                    0,
                )))),
            ),
        ];
        let term_t1_b1 = Terminator::Goto(50);

        let blocks_t1 = vec![
            BasicBlock::new(stmts_t1_b0, term_t1_b0),
            BasicBlock::new(stmts_t1_b1, term_t1_b1),
        ];

        let sirs = vec![
            Pack::Body(Body {
                symbol_name: String::from("symbol1"),
                blocks: blocks_t1,
                flags: 0,
            }),
            Pack::Body(Body {
                symbol_name: String::from("symbol2"),
                blocks: vec![BasicBlock::new(
                    vec![Statement::Unimplemented(String::from("abc"))],
                    Terminator::Unreachable,
                )],
                flags: 0,
            }),
        ];

        let mut got = String::new();
        for pack in sirs {
            got.push_str(&format!("{}", pack));
        }
        let got_lines = got.split("\n");

        let expect = "[Begin SIR for symbol1]\n\
  flags: 0
    bb0:
        $0 = $1
        $2 = ($3).4
        $4 = U8(10)
        nop
        goto bb20
    bb1:
        $5 = $6
        $5 = $4
        $7 = add($8, $9)
        $7 = sub($9, $10)
        $11 = U8(0)
        goto bb50
[End SIR for symbol1]
[Begin SIR for symbol2]
  flags: 0
    bb0:
        unimplemented_stmt: abc
        unreachable
[End SIR for symbol2]\n";

        let expect_lines = expect.split("\n");

        assert_eq!(got_lines.clone().count(), expect_lines.clone().count());
        for (got, expect) in got_lines.zip(expect_lines) {
            assert_eq!(got.trim(), expect.trim());
        }
    }
}
