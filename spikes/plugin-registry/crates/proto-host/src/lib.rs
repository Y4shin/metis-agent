//! PROTOTYPE (throwaway). Not product code.
//!
//! The daemon-side half: the two adapters behind the seam, the typed method
//! families, the benchmark, and the host binary.
//!
//! Module map:
//!
//! - [`crate::abi_adapter`] wraps a cdylib loaded through the landed
//!   `plugin-contract` ABI, driving it with the landed `HostAdapter`.
//! - [`crate::native`] is the in-process Rust plugin, behind the same seam.
//! - [`crate::typed`] is the tool and provider method families above the seam.
//! - [`crate::types`] is the host's own copy of the payload types.
//! - [`crate::bench`] measures what the registry costs per call.
//!
//! The registry itself lives in the `proto-registry` crate, behind its
//! `registry` feature. `proto-host` enables that feature; `proto-registry`
//! alone builds without it.
//!
//! Round one's `seam.rs` and `registry.rs` are kept on disk unmodified, as the
//! record of that round, but are no longer crate modules: the seam and the
//! registry moved to `proto-registry`. See `FINDINGS2.md`.

pub mod abi_adapter;
pub mod bench;
pub mod future_bench;
pub mod future_driver;
pub mod native;
pub mod typed;
pub mod types;
pub mod versioning;
