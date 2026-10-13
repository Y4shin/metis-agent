//! The plugin boundary: the `repr(C)` primitives, the payload envelope, and the
//! generated C header.
//!
//! `repr(C)` Rust in this crate is the source of truth for the ABI (D2). The
//! committed header at `include/metis_plugin_abi.h` is generated from it by
//! `scripts/regenerate-abi-header.sh`, and CI fails when regeneration produces
//! a diff. `crate::abi` is the whole C ABI surface and the only module
//! `cbindgen` sees; `crate::payload` holds the one-byte envelope that rides
//! inside `MetisBuffer`, deliberately outside the generated header.
//!
//! This crate carries no async runtime and no serde in its C ABI surface: a
//! plugin boundary is a byte boundary, and everything above it (serialisation
//! shapes, the authoring trait) lives elsewhere.

pub mod abi;
pub mod payload;
