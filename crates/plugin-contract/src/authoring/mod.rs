//! The authoring surface: what a Rust plugin author implements, and the adapter
//! that translates between it and the C ABI.
//!
//! This module is behind the `authoring` feature, off by default, so
//! `plugin-runtime` can depend on the primitives, the payload envelope and the
//! capability declaration alone. Turning it on adds `schemars`, which derives
//! the inline capability schemas an author's payload types imply. The
//! declaration those schemas fill in lives in `crate::capability`, ungated,
//! because both sides of the wire read the same shape.
//!
//! The shape of the translation:
//!
//! - [`Plugin`] is what an author implements: one capability-name-routed
//!   `call`, a capability declaration, a fallible asynchronous constructor, and
//!   a cancellation hook. Nothing in it names a buffer, a status, a callback, or
//!   a payload header.
//! - [`table`] is the whole plugin side as one `const fn`, which the plugin
//!   instantiates into a static and exports through `metis_plugin_entry_v1`.
//! - [`future`] is the plugin's side of the future seam: it wraps the author's
//!   future in the host's waker and contains a panic in both `poll` and `drop`.
//!   Both the initialisation entry and every capability go through it.
//!
//! There is no executor in this module and no thread: the host polls, and this
//! side answers. An author surface that needs to wait asks for a host service
//! through [`Call::service_request`] and is answered through the host's
//! `service_complete`.

mod alloc;
mod call;
mod future;
mod plugin_adapter;
mod schema;
mod traits;

pub use alloc::{Accounting, accounting};
pub use call::{Call, CancellationToken, Cancelled, Host, LogLevel, ServiceAnswer};
// The declaration is not feature-gated: both sides of the wire read it.
pub use crate::capability::{CapabilityDescriptor, CapabilityKind, CapabilityList};
pub use plugin_adapter::table;
pub use schema::schema_of;
pub use traits::{HostContext, Initialisation, Plugin, PluginError, Request, Response};

/// The schema derive an author reaches for when it publishes inline capability
/// schemas, re-exported so a plugin needs no direct dependency on it.
pub use schemars;
