//! The authoring surface: what a Rust plugin author implements, and the adapter
//! that translates between it and the C ABI.
//!
//! This module is behind the `authoring` feature, off by default, so
//! `plugin-runtime` can depend on the primitives and the payload envelope
//! alone. Turning it on adds the serde pair a plugin's request and response
//! bodies are serialised with, and `schemars` for the inline capability
//! schemas.
//!
//! The shape of the translation:
//!
//! - [`Plugin`] is what an author implements: one capability-name-routed
//!   `call`, a capability declaration, an instance lifecycle pair, and a
//!   cancellation hook. Nothing in it names a buffer, a status, a callback, or a
//!   payload header.
//! - [`table`] is the whole plugin side as one `const fn`, which the plugin
//!   instantiates into a static and exports through `metis_plugin_entry_v1`.
//! - [`future`] is the plugin's side of the future seam: it wraps the author's
//!   future in the host's waker and contains a panic in both `poll` and `drop`.
//!
//! There is no executor in this module and no thread: the host polls, and this
//! side answers. An author surface that needs to wait asks for a host service
//! through [`Call::service_request`] and is answered through the host's
//! `service_complete`.

mod alloc;
mod call;
mod capability;
mod future;
mod plugin_adapter;
mod traits;

pub use alloc::{Accounting, accounting};
pub use call::{Call, CancellationToken, Cancelled, Host, LogLevel, ServiceAnswer};
pub use capability::{CapabilityDescriptor, CapabilityKind, CapabilityList};
pub use plugin_adapter::table;
pub use traits::{Plugin, PluginError, Request, Response};

/// The schema derive an author reaches for when it publishes inline capability
/// schemas, re-exported so a plugin needs no direct dependency on it.
pub use schemars;
