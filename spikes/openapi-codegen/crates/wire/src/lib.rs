//! Handwritten glue for the openapi-codegen spike's primary alternative
//! (`openapi-generator` for both sides).
//!
//! Everything in this crate is code that the generators did **not** produce: the
//! SSE server side and the conversions between the generated server DTOs and the
//! generated client DTOs. The shared HTTP glue (Unix-socket serving, transports,
//! SSE client) lives in `spike-shared`.

pub mod convert;
pub mod sse_server;

pub use spike_shared::{serve_uds, sse_client, transport};
