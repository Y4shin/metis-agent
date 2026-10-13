//! D6 on the plugin side, in one place.
//!
//! The adapter allocates every buffer the plugin hands to the host (a result, a
//! progress payload, an event, the capability list), and `release_buffer` frees
//! exactly those. Ownership is what makes the rule checkable at runtime: the
//! allocator keeps the capacity of each live buffer and refuses anything it did
//! not allocate, a pointer it has already released, or a length that changed in
//! flight.
//!
//! The ledger this module keeps is the plugin's side of the ownership check: the
//! fixture reports it through the host's `log` callback at teardown, and the
//! harness compares it with its own count of released buffers.

use std::collections::HashMap;
use std::mem;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::abi::{METIS_BUFFER_OWNER_PLUGIN, MetisBuffer, MetisStatus};

/// The plugin-side ledger: what the adapter allocated, released, still holds,
/// and how many completions it suppressed after a successful cancel (D6, D12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accounting {
    /// Buffers the adapter allocated.
    pub allocs: usize,
    /// Buffers released through the table's `release_buffer`.
    pub frees: usize,
    /// Buffers allocated and not yet released.
    pub live: usize,
    /// Ownership violations, which are always zero in a correct run.
    pub violations: usize,
    /// Completions suppressed because the plugin gave up on a cancelled call.
    pub suppressed: usize,
}

impl Accounting {
    /// The one-line ledger a plugin can log, which the harness parses.
    pub fn line(self) -> String {
        format!(
            "allocs={} frees={} live={} violations={} suppressed={}",
            self.allocs, self.frees, self.live, self.violations, self.suppressed
        )
    }
}

/// Buffers this plugin allocated, keyed by pointer and valued by capacity.
fn live() -> &'static Mutex<HashMap<usize, usize>> {
    static LIVE: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static VIOLATIONS: AtomicUsize = AtomicUsize::new(0);
static SUPPRESSED: AtomicUsize = AtomicUsize::new(0);

/// The current ledger.
pub fn accounting() -> Accounting {
    Accounting {
        allocs: ALLOCS.load(Ordering::SeqCst),
        frees: FREES.load(Ordering::SeqCst),
        live: live().lock().unwrap().len(),
        violations: VIOLATIONS.load(Ordering::SeqCst),
        suppressed: SUPPRESSED.load(Ordering::SeqCst),
    }
}

/// Records an ownership violation, which a correct run never has.
pub(crate) fn violation(what: &str) {
    VIOLATIONS.fetch_add(1, Ordering::SeqCst);
    eprintln!("plugin-contract: buffer violation: {what}");
}

/// Counts a completion the adapter suppressed after a successful cancel (D12).
pub(crate) fn note_suppressed() {
    SUPPRESSED.fetch_add(1, Ordering::SeqCst);
}

/// Copies bytes into a buffer this plugin owns (D6).
pub(crate) fn plugin_alloc(bytes: &[u8]) -> MetisBuffer {
    let mut vec: Vec<u8> = Vec::with_capacity(bytes.len());
    vec.extend_from_slice(bytes);
    let ptr = vec.as_mut_ptr();
    let len = vec.len();
    let cap = vec.capacity();
    mem::forget(vec);
    live().lock().unwrap().insert(ptr as usize, cap);
    ALLOCS.fetch_add(1, Ordering::SeqCst);
    MetisBuffer {
        ptr,
        len,
        cap,
        owner: METIS_BUFFER_OWNER_PLUGIN,
        reserved: 0,
    }
}

/// The table's `release_buffer` entry point.
///
/// # Safety
///
/// `buffer` must be a value the host received from this plugin, unchanged: the
/// allocator rebuilds the exact `Vec` from the recorded capacity, so a buffer
/// whose length or capacity changed in flight is reported and dropped with its
/// original length.
pub unsafe extern "C" fn release_buffer(buffer: MetisBuffer) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if buffer.owner != METIS_BUFFER_OWNER_PLUGIN {
            violation("release_buffer with a buffer this plugin did not allocate");
            return MetisStatus::InvalidArgument;
        }
        if buffer.ptr.is_null() {
            // The empty buffer is the null buffer: nothing was allocated.
            return MetisStatus::Ok;
        }
        let cap = match live().lock().unwrap().remove(&(buffer.ptr as usize)) {
            Some(cap) => cap,
            None => {
                violation("release_buffer with an unknown or already released pointer");
                return MetisStatus::NotFound;
            }
        };
        if cap != buffer.cap || buffer.len > buffer.cap {
            violation("release_buffer with a buffer whose length or capacity changed");
        }
        // SAFETY: the pointer came from `plugin_alloc`, the capacity is the one
        // recorded then, and the map no longer holds it, so this drop is once.
        drop(unsafe { Vec::from_raw_parts(buffer.ptr, buffer.len, cap) });
        FREES.fetch_add(1, Ordering::SeqCst);
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}
