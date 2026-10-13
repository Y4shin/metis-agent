//! The plugin boundary: the `repr(C)` primitives, the payload envelope, and the
//! generated C header.
//!
//! `repr(C)` Rust in this crate is the source of truth for the ABI. The
//! committed header at `include/metis_plugin_abi.h` is generated from it by
//! `scripts/regenerate-abi-header.sh`, and CI fails when regeneration produces
//! a diff. `crate::abi` is the whole C ABI surface and the only module
//! `cbindgen` sees; `crate::payload` holds the one-byte envelope that rides
//! inside `MetisBuffer`, whose constants are part of the header.
//!
//! This crate carries no async runtime and no serde in its C ABI surface: a
//! plugin boundary is a byte boundary, and everything above it (serialisation
//! shapes, the authoring trait) lives behind the `authoring` feature.
//!
//! The revision this crate carries is the approved plugin contract: a future is
//! the only call path (`invoke` and the host `complete` callback are gone), the
//! entry order changed under the unchanged symbol and major so `abi_minor` is 1
//! and a reader refuses a minor it does not know, and neither vtable carries a
//! `size` field. A wrong-size plugin vtable is that plugin's own bug and is not
//! detectable at load; the residual is recorded rather than paid for on every
//! poll.
//!
//! The Rust authoring surface (the routed trait and the future adapter) is
//! behind the `authoring` feature, off by default, so the plugin runtime
//! depends on the primitives and the payload envelope alone.

pub mod abi;

#[cfg(feature = "authoring")]
pub mod authoring;

pub mod payload;
