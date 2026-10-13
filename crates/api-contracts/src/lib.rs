//! The Metis wire contract: generated Protobuf messages and Connect stubs.
//!
//! `proto` carries the generated message types and `connect` the generated
//! service trait, register helper, and client, both produced from
//! `contracts/proto/metis/v1/` by the pinned BSR plugins in the root
//! `buf.gen.yaml`.
//!
//! The generated trees under `generated/` are committed and mounted, never
//! edited: `src/proto.rs` and `src/connect.rs` are the handwritten module trees
//! the generated files are `include!`d into. Regenerate with
//! `bash scripts/regenerate-contracts.sh`; see `contracts/README.md`.
//!
//! `transport` is the one handwritten transport in the repository: the serve
//! loop, the Unix listener front-end, and the connector shape the skeleton, the
//! `client` crate, and the daemon effort share.

pub mod connect;
pub mod proto;
pub mod transport;
