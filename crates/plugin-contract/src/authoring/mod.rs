//! The authoring surface: what a Rust plugin author implements, and the two
//! adapters that translate between it and the C ABI (D3, D15).
//!
//! This module is behind the `authoring` feature, off by default, so
//! `plugin-runtime` can depend on the primitives and the payload envelope
//! alone. Turning it on adds the serde pair the plugin's own request and
//! response types are serialised with.
//!
//! The shape of the translation:
//!
//! - [`Plugin`] is what an author implements: one async method per capability
//!   arm, an instance lifecycle pair, and a cancellation method. Nothing in it
//!   names a buffer, a status, a callback, or a payload header.
//! - [`table`] is the whole plugin side as one `const fn`, which the plugin
//!   instantiates into a static and exports through `metis_plugin_entry_v1`.
//! - [`HostAdapter`] is the host side: it owns the callback table, registers a
//!   pending record before a call, and awaits the completion the plugin's
//!   callback delivers.
//! - [`executor`] is the crate's own minimal executor, because a trait path
//!   that always returns `Pending` needs somewhere to drive the future and the
//!   crate carries no async runtime.
//!
//! Two things the ABI forces on the author anyway, and the trait documents
//! rather than hides: `Send + Sync + 'static` on the plugin and `+ Send` on
//! every capability future, because the adapter moves each call onto a worker
//! thread; and a per-call context, because the ABI is asynchronous and out of
//! band.

pub mod executor;

mod alloc;
mod call;
mod capability;
mod host_adapter;
mod plugin_adapter;
mod traits;

pub use alloc::{Accounting, accounting};
pub use call::{Call, CancellationToken, Cancelled, Host, LogLevel};
pub use capability::{CapabilityDescriptor, CapabilityKind, CapabilityList, Shape};
pub use host_adapter::{Await, Completed, HostAdapter};
pub use plugin_adapter::table;
pub use traits::{Plugin, PluginError};
