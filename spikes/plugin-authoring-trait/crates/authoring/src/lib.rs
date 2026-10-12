//! Throwaway authoring layer for the plugin ABI.
//!
//! Spike-only code for
//! `docs/tasks/metis-foundations/tasks/plugin-authoring-trait-spike/task.md`.
//! It holds the throwaway authoring trait, the per-request context, a minimal
//! async executor, and the hand-written translation in both directions:
//! `plugin_adapter` turns `Plugin` into `MetisPluginTableV1`, and
//! `host_adapter` turns `MetisHostApiV1` into an awaitable call.
//!
//! The ABI primitives themselves are the committed ones from the mechanics
//! spike (`spike-abi`, `spike-payload`), depended on by path. Nothing here
//! re-declares them.

#![allow(clippy::missing_safety_doc)]

mod alloc;
mod call;
pub mod executor;
pub mod host_adapter;
pub mod payload;
pub mod plugin_adapter;
mod traits;

pub use call::Call;
pub use traits::{Plugin, PluginError};
