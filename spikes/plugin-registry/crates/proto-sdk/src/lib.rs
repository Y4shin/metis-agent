//! PROTOTYPE (throwaway). Not product code.
//!
//! The SDK side of candidate A's executor seam.
//!
//! [`proto_registry::Executor`] is the primitives-side seam. This crate is the
//! concrete implementation the plugin owner injects:
//!
//! - [`PoolExecutor`]: a bounded worker pool with a task queue, created once
//!   and reused. Workers wait on a queue condvar; the futures they drive park
//!   and unpark through the [`proto_registry::block_on`] waker, so a task that
//!   wakes itself from another thread never spins.
//! - [`ThreadPerCallExecutor`]: the existing per-call `thread::spawn` kept as
//!   the fallback, behind the same trait, so the two are measured in one
//!   binary.
//!
//! The pool's worker count and idle policy are configuration, because the
//! sizing question ("N plugins times pool size") is the point of the prototype.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use proto_registry::{DrainOutcome, Executor, SpawnRefused, Task, block_on};

/// How an idle worker behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdlePolicy {
    /// A worker waits on the queue and stays for the pool's whole life, so the
    /// steady state is `workers` threads per pool.
    Stay,
    /// A worker exits after `after` with an empty queue. The next `spawn`
    /// starts a worker again (lazy re-init), which is where the re-init race
    /// lives: a worker that has decided to exit must re-check the queue under
    /// the same lock a submit uses, or it can retire just as a task is queued
    /// and lose it.
    Timeout(Duration),
}

struct PoolState {
    queue: VecDeque<Task>,
    workers: usize,
    busy: usize,
    draining: bool,
    shutdown: bool,
    completed: usize,
    handles: Vec<JoinHandle<()>>,
}

struct PoolShared {
    state: Mutex<PoolState>,
    /// A worker waits here for work; a submit notifies one.
    worker_signal: Condvar,
    /// A `drain` caller waits here for the queue and the running count to empty.
    finished: Condvar,
    max_workers: usize,
    idle: IdlePolicy,
}

/// A bounded worker pool behind the primitives-side [`Executor`] seam.
pub struct PoolExecutor {
    shared: Arc<PoolShared>,
}

impl PoolExecutor {
    /// Builds a pool with `workers` workers, started now and reused.
    pub fn new(workers: usize, idle: IdlePolicy) -> Self {
        let workers = workers.max(1);
        let shared = Arc::new(PoolShared {
            state: Mutex::new(PoolState {
                queue: VecDeque::new(),
                workers: 0,
                busy: 0,
                draining: false,
                shutdown: false,
                completed: 0,
                handles: Vec::new(),
            }),
            worker_signal: Condvar::new(),
            finished: Condvar::new(),
            max_workers: workers,
            idle,
        });
        let mut state = shared.state.lock().unwrap();
        for _ in 0..workers {
            PoolShared::start_worker(&shared, &mut state);
        }
        drop(state);
        PoolExecutor { shared }
    }

    /// How many workers are alive right now. With [`IdlePolicy::Timeout`] this
    /// drops while the pool is idle and rises again on the next submit.
    pub fn worker_count(&self) -> usize {
        self.shared.state.lock().unwrap().workers
    }

    /// The pool's configured ceiling.
    pub fn max_workers(&self) -> usize {
        self.shared.max_workers
    }
}

impl PoolShared {
    /// Starts one worker. The caller holds the state lock, so the new worker's
    /// `workers += 1` cannot interleave with a submit's decision to start one.
    fn start_worker(self: &Arc<Self>, state: &mut PoolState) {
        let shared = Arc::clone(self);
        let handle = thread::spawn(move || Self::worker_loop(shared));
        state.handles.push(handle);
        state.workers += 1;
    }

    fn worker_loop(shared: Arc<Self>) {
        loop {
            let task = {
                let mut state = shared.state.lock().unwrap();
                loop {
                    if let Some(task) = state.queue.pop_front() {
                        state.busy += 1;
                        break Some(task);
                    }
                    if state.draining || state.shutdown {
                        state.workers -= 1;
                        shared.finished.notify_all();
                        return;
                    }
                    match shared.idle {
                        IdlePolicy::Stay => {
                            state = shared.worker_signal.wait(state).unwrap();
                        }
                        IdlePolicy::Timeout(after) => {
                            let (guard, timeout) =
                                shared.worker_signal.wait_timeout(state, after).unwrap();
                            state = guard;
                            if timeout.timed_out()
                                && state.queue.is_empty()
                                && !state.draining
                                && !state.shutdown
                            {
                                // Re-checked the queue under the lock and it was
                                // still empty, so exiting here cannot lose a
                                // task a submit queued while this worker was
                                // waking up.
                                state.workers -= 1;
                                shared.finished.notify_all();
                                return;
                            }
                        }
                    }
                }
            };
            let Some(task) = task else { continue };
            block_on(task);
            let mut state = shared.state.lock().unwrap();
            state.busy -= 1;
            state.completed += 1;
            shared.finished.notify_all();
        }
    }
}

impl Executor for PoolExecutor {
    fn spawn(&self, task: Task) -> Result<(), SpawnRefused> {
        let mut state = self.shared.state.lock().unwrap();
        if state.draining || state.shutdown {
            return Err(SpawnRefused::Draining);
        }
        state.queue.push_back(task);
        // An idle worker is woken; if every worker is busy and the pool is
        // under its ceiling, one more is started. With `IdlePolicy::Timeout`
        // and `workers == 0`, this is the lazy re-init.
        if state.workers == 0
            || (state.busy >= state.workers && state.workers < self.shared.max_workers)
        {
            PoolShared::start_worker(&self.shared, &mut state);
        }
        self.shared.worker_signal.notify_one();
        Ok(())
    }

    fn drain(&self, deadline: Option<Duration>) -> DrainOutcome {
        let shared = &self.shared;
        let mut state = shared.state.lock().unwrap();
        state.draining = true;
        shared.worker_signal.notify_all();
        let deadline_at = deadline.map(|after| Instant::now() + after);
        let mut timed_out = false;
        while state.busy > 0 || !state.queue.is_empty() {
            match deadline_at {
                Some(at) => {
                    let now = Instant::now();
                    if now >= at {
                        timed_out = true;
                        break;
                    }
                    let (guard, _) = shared.finished.wait_timeout(state, at - now).unwrap();
                    state = guard;
                }
                None => {
                    state = shared.finished.wait(state).unwrap();
                }
            }
        }
        let still_running = state.busy;
        // Queued futures are dropped outside the lock, so a guard can release
        // its plugin buffer without re-entering the pool.
        let dropped_tasks: Vec<Task> = if timed_out {
            state.queue.drain(..).collect()
        } else {
            Vec::new()
        };
        let dropped = dropped_tasks.len();
        let completed = state.completed;
        shared.worker_signal.notify_all();
        drop(state);
        drop(dropped_tasks);
        DrainOutcome {
            completed,
            dropped,
            still_running,
            timed_out,
        }
    }

    fn is_draining(&self) -> bool {
        self.shared.state.lock().unwrap().draining
    }
}

impl Drop for PoolExecutor {
    fn drop(&mut self) {
        // Never let a worker outlive the executor: the executor lives in the
        // plugin, so a worker still inside plugin code after the instance is
        // gone is a use-after-unload. The first drain is unbounded on purpose:
        // a deadline cannot preempt a running task.
        let _ = self.drain(None);
        let handles: Vec<JoinHandle<()>> = {
            let mut state = self.shared.state.lock().unwrap();
            state.shutdown = true;
            self.shared.worker_signal.notify_all();
            std::mem::take(&mut state.handles)
        };
        for handle in handles {
            let _ = handle.join();
        }
    }
}

struct ThreadCallState {
    handles: Vec<JoinHandle<()>>,
    draining: bool,
}

/// The existing per-call `thread::spawn`, kept as the fallback behind the same
/// seam so the two executors are measured against each other in one binary.
pub struct ThreadPerCallExecutor {
    state: Mutex<ThreadCallState>,
}

impl Default for ThreadPerCallExecutor {
    fn default() -> Self {
        ThreadPerCallExecutor::new()
    }
}

impl ThreadPerCallExecutor {
    pub fn new() -> Self {
        ThreadPerCallExecutor {
            state: Mutex::new(ThreadCallState {
                handles: Vec::new(),
                draining: false,
            }),
        }
    }

    /// How many per-call workers are alive (joined workers are not removed
    /// until `drain`, because a join block here would serialize call setup).
    pub fn outstanding(&self) -> usize {
        self.state.lock().unwrap().handles.len()
    }
}

impl Executor for ThreadPerCallExecutor {
    fn spawn(&self, task: Task) -> Result<(), SpawnRefused> {
        let mut state = self.state.lock().unwrap();
        if state.draining {
            return Err(SpawnRefused::Draining);
        }
        let handle = thread::spawn(move || block_on(task));
        state.handles.push(handle);
        Ok(())
    }

    fn drain(&self, deadline: Option<Duration>) -> DrainOutcome {
        let started = Instant::now();
        let handles = {
            let mut state = self.state.lock().unwrap();
            state.draining = true;
            std::mem::take(&mut state.handles)
        };
        let mut completed = 0;
        for handle in handles {
            let _ = handle.join();
            completed += 1;
        }
        let timed_out = deadline.map(|after| started.elapsed() > after).unwrap_or(false);
        DrainOutcome {
            completed,
            dropped: 0,
            still_running: 0,
            timed_out,
        }
    }

    fn is_draining(&self) -> bool {
        self.state.lock().unwrap().draining
    }
}
