//! The crate's own minimal executor: one worker thread per pending call, with a
//! park/unpark waker.
//!
//! The trait is async and the ABI is callback based, so the adapter needs
//! something to drive a capability future after `invoke` has already returned
//! `Pending`. That something is normally owned by `metis-plugin-sdk`; this is
//! the crate's deliberately tiny stand-in, because `plugin-contract` carries no
//! async runtime (D3).
//!
//! It is two functions and one wrapper:
//!
//! - [`block_on`] drives a future on the calling thread.
//! - [`spawn`] runs a future on a fresh worker thread and hands back its handle,
//!   so the adapter can join every worker at `shutdown` before the host unloads
//!   the library (D12).
//! - [`catch_panic`] wraps a future so a panic in its `poll` becomes an `Err`
//!   instead of unwinding the worker thread, which would leave a pending call
//!   without a completion (D7).

use std::any::Any;
use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, JoinHandle, Thread};

/// What a capability future's panic was caught as.
pub type Panic = Box<dyn Any + Send + 'static>;

struct ThreadWaker(Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Runs a future to completion on the calling thread.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            // Unpark tokens persist, so a wake that races with this park is
            // never lost. Park may also return spuriously; the loop is safe.
            Poll::Pending => thread::park(),
        }
    }
}

/// Runs a future on a fresh worker thread and hands back its handle.
///
/// Callers that may panic should wrap the future in [`catch_panic`] first: a
/// panic that unwound this thread would leave whatever the future was going to
/// do undone.
pub fn spawn<F>(future: F) -> JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    thread::spawn(move || block_on(future))
}

/// Wraps a future so a panic in its `poll` becomes an `Err`.
///
/// A worker thread is a boundary: the host is waiting for a completion, so a
/// panic inside a capability turns into an `Internal` completion instead of a
/// thread that dies without answering (D7).
pub struct CatchPanic<F> {
    inner: F,
}

/// Wraps `future` so a panic in its `poll` becomes an `Err`.
pub fn catch_panic<F: Future>(future: F) -> CatchPanic<F> {
    CatchPanic { inner: future }
}

impl<F: Future> Future for CatchPanic<F> {
    type Output = Result<F::Output, Panic>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: `inner` is structurally pinned here and never moved, so the
        // projection keeps whatever pinning guarantee the caller had.
        let inner = unsafe { self.map_unchecked_mut(|this| &mut this.inner) };
        match catch_unwind(AssertUnwindSafe(|| inner.poll(context))) {
            Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
            Ok(Poll::Pending) => Poll::Pending,
            Err(panic) => Poll::Ready(Err(panic)),
        }
    }
}
