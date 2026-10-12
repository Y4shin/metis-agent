//! The per-request context handed to every capability method.
//!
//! This type exists only because the ABI is asynchronous and out of band. A
//! request id, a cancellation flag shared with a `cancel` call on another
//! thread, and a progress route are ABI concepts that a plain Rust trait method
//! would not need. See the findings, bend 2.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::traits::PluginError;

/// A raw pointer the adapter vouches for. D12 says callbacks may come from any
/// plugin thread, so the host table has to travel to worker threads; the
/// adapter asserts that the host keeps it alive for the instance's lifetime.
///
/// The pointer is only reachable through `get`, so a closure or async block that
/// uses the host table captures the whole `SendPtr` (which is `Send`) and not the
/// bare `*const T` field (which is not).
pub(crate) struct SendPtr<T>(*const T);

// Manual impls: the derived ones would demand `T: Clone` / `T: Copy`, which the
// ABI types are not.
impl<T> Clone for SendPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for SendPtr<T> {}

unsafe impl<T> Send for SendPtr<T> {}
unsafe impl<T> Sync for SendPtr<T> {}

impl<T> SendPtr<T> {
    pub(crate) fn new(ptr: *const T) -> Self {
        SendPtr(ptr)
    }

    pub(crate) fn get(&self) -> *const T {
        self.0
    }
}

pub(crate) struct CallInner {
    id: u64,
    cancelled: AtomicBool,
    progress: Box<dyn Fn(&[u8]) + Send + Sync + 'static>,
}

/// Context for one `invoke` call.
///
/// Cheap to clone, because the adapter keeps one clone in its cancellation
/// registry so that a later `cancel` can reach the same call.
#[derive(Clone)]
pub struct Call(Arc<CallInner>);

impl Call {
    pub(crate) fn new(id: u64, progress: impl Fn(&[u8]) + Send + Sync + 'static) -> Self {
        Call(Arc::new(CallInner {
            id,
            cancelled: AtomicBool::new(false),
            progress: Box::new(progress),
        }))
    }

    /// The ABI request id for this call.
    pub fn id(&self) -> u64 {
        self.0.id
    }

    /// Whether a `cancel` for this request has already been accepted.
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::SeqCst)
    }

    pub(crate) fn mark_cancelled(&self) {
        self.0.cancelled.store(true, Ordering::SeqCst);
    }

    /// Emits a progress payload through the host `progress` callback.
    ///
    /// The body is JSON; the D4 header, the plugin allocation and the release
    /// rule stay inside the adapter, so the author never sees a `MetisBuffer`.
    pub fn progress<T: Serialize>(&self, body: &T) -> Result<(), PluginError> {
        let bytes = serde_json::to_vec(body)
            .map_err(|err| PluginError::Internal(format!("progress payload: {err}")))?;
        (self.0.progress)(&bytes);
        Ok(())
    }
}
