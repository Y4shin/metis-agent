//! A test-only library whose table reports numbers this build did not write.
//!
//! A library compiled by this repository reports this build's own ABI and this
//! build's own layout, so the reader's version and size rules cannot be shown to
//! it. This file is not compiled as part of any target: the test reads it,
//! substitutes `SYMBOL`, `SIZE`, `MAJOR` and `MINOR`, and compiles the result
//! with `rustc`, so the reader is shown a table from a plugin built elsewhere:
//! one whose minor is older or newer than this build's, one whose reported size
//! stops before an entry this build needs, and one whose major differs.
//!
//! The layout here is the test's own reading of the contract rather than a use
//! of any type in this tree, which is the point: the numbers must not come from
//! the reader. Every slot carries the entry function's own address, because no
//! test here calls an entry and a slot has to be a readable pointer either way,
//! and the last slot is the entry a newer plugin appends after this build's
//! layout.

#![allow(dead_code)]

#[repr(C)]
struct Table {
    size: usize,
    abi_major: u32,
    abi_minor: u32,
    entries: [extern "C" fn() -> *const Table; 9],
}

static TABLE: Table = Table {
    size: SIZE,
    abi_major: MAJOR,
    abi_minor: MINOR,
    entries: [SYMBOL; 9],
};

#[unsafe(no_mangle)]
pub extern "C" fn SYMBOL() -> *const Table {
    &raw const TABLE
}
