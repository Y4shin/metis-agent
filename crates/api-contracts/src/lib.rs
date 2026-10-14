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
//! loop, the Unix, TCP, and Windows named-pipe listener front-ends, and the
//! connector shapes the skeleton, the `client` crate, and the daemon effort
//! share.
//!
//! `proxy` is the reverse-proxy rule ADR 0004 chose: the proxies an operator
//! named, the forwarded headers read from them and from nobody else, and the
//! plaintext warnings that follow from the exposure the daemon can see.
//!
//! `identity` is the declaration a client carries as request metadata on every
//! call and the version gate the daemon applies to it.
//!
//! `auth` is the credential's wire vocabulary, `keychain` is the platform's
//! keychain, reached through its command line tool, and `time` is the stamp
//! format the contract's `received_at` field asks for.
//!
//! `request` is the identifier a caller declares on a call it may want to stop
//! and the echo that returns it, and `cancel` is the registry of the work such a
//! call has in flight. The daemon and the client's in-process service share both,
//! so a stream is cancellable the same way whichever one serves it.

pub mod auth;
pub mod cancel;
pub mod connect;
pub mod events;
pub mod identity;
pub mod keychain;
pub mod proto;
pub mod proxy;
pub mod request;
pub mod time;
pub mod transport;
