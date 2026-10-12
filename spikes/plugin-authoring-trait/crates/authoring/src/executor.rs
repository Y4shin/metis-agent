//! A minimal async executor for the spike.
//!
//! The ABI is callback based and the trait is async, so the adapter needs
//! something to drive a future after `invoke` has already returned pending.
//! That something is normally owned by `metis-plugin-sdk`; this is its
//! throwaway stand-in, deliberately tiny (one thread per call, park/unpark) and
//! not a scheduler.

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, JoinHandle, Thread};
use std::time::{Duration, Instant};

struct ThreadWaker(Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}

/// Runs a future to completion on the current thread.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            // Unpark tokens persist, so a wake that races with this park is
            // never lost. Park may also return spuriously; the loop is safe.
            Poll::Pending => thread::park(),
        }
    }
}

/// Runs a future on a fresh worker thread and hands back the handle, so the
/// adapter can join it at shutdown before the host unloads the library.
pub fn spawn<F>(future: F) -> JoinHandle<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    thread::spawn(move || block_on(future))
}

/// A timer future: it never blocks the executor thread, a timer thread wakes
/// the task instead.
pub struct Delay {
    deadline: Instant,
    timer: Option<JoinHandle<()>>,
}

/// Resolves after `duration`.
pub fn sleep(duration: Duration) -> Delay {
    Delay { deadline: Instant::now() + duration, timer: None }
}

impl Future for Delay {
    type Output = ();

    fn poll(mut self: std::pin::Pin<&mut Self>, context: &mut Context<'_>) -> Poll<()> {
        let this = self.as_mut().get_mut();
        if Instant::now() >= this.deadline {
            return Poll::Ready(());
        }
        if this.timer.is_none() {
            let waker = context.waker().clone();
            let deadline = this.deadline;
            this.timer = Some(thread::spawn(move || {
                let now = Instant::now();
                if deadline > now {
                    thread::sleep(deadline - now);
                }
                waker.wake();
            }));
        }
        Poll::Pending
    }
}
