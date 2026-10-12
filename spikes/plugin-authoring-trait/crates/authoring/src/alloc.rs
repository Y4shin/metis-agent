//! D6 on the plugin side, in one place: the adapter allocates every buffer the
//! plugin hands to the host, and `release_buffer` frees it.
//!
//! The trait never allocates and never sees a `MetisBuffer`, which is the one
//! part of the translation that leaks nothing at all.

use std::collections::HashMap;
use std::mem;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use spike_abi::*;

static LIVE: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static FREES: AtomicUsize = AtomicUsize::new(0);
static VIOLATIONS: AtomicUsize = AtomicUsize::new(0);

fn live() -> &'static Mutex<HashMap<usize, usize>> {
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn violation(what: &str) {
    VIOLATIONS.fetch_add(1, Ordering::SeqCst);
    eprintln!("spike-trait: buffer violation: {what}");
}

/// Allocates a plugin-owned buffer (D6).
pub fn plugin_alloc(bytes: &[u8]) -> MetisBuffer {
    let mut vec: Vec<u8> = Vec::with_capacity(bytes.len());
    vec.extend_from_slice(bytes);
    let ptr = vec.as_mut_ptr();
    let len = vec.len();
    let cap = vec.capacity();
    mem::forget(vec);
    live().lock().unwrap().insert(ptr as usize, cap);
    ALLOCS.fetch_add(1, Ordering::SeqCst);
    MetisBuffer { ptr, len, cap, owner: METIS_BUFFER_OWNER_PLUGIN, reserved: 0 }
}

/// `allocs`, `frees`, still-live buffers, violations.
pub(crate) fn accounting() -> (usize, usize, usize, usize) {
    (
        ALLOCS.load(Ordering::SeqCst),
        FREES.load(Ordering::SeqCst),
        live().lock().unwrap().len(),
        VIOLATIONS.load(Ordering::SeqCst),
    )
}

/// The plugin table's `release_buffer` entry point.
pub unsafe extern "C" fn release_buffer(buffer: MetisBuffer) -> MetisStatus {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if buffer.owner != METIS_BUFFER_OWNER_PLUGIN {
            violation("release_buffer with a buffer this plugin did not allocate");
            return MetisStatus::InvalidArgument;
        }
        if buffer.ptr.is_null() {
            // The empty buffer is the null buffer; nothing was allocated.
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
            violation("release_buffer with a changed length or capacity");
        }
        // SAFETY: the pointer came from `plugin_alloc`, the capacity is the one
        // recorded then, and the map no longer holds it, so this drop is once.
        drop(unsafe { Vec::from_raw_parts(buffer.ptr, buffer.len, cap) });
        FREES.fetch_add(1, Ordering::SeqCst);
        MetisStatus::Ok
    }));
    result.unwrap_or(MetisStatus::Internal)
}
