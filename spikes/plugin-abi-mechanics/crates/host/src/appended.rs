//! Checks the append-only reading rule for the function table (D8).
//!
//! `MetisPluginTableNext` is a spike-local hypothesis for the next revision of
//! the table: one entry appended after `shutdown`. It is deliberately not in
//! `spike-abi`, so the committed header only contains the real version 1
//! surface.

use core::mem::size_of;

use spike_abi::*;

pub type MetisSetLogLevelFn = unsafe extern "C" fn(level: i32) -> MetisStatus;

/// A version 1 table with one entry appended. Because the appended field lands
/// exactly at `size_of::<MetisPluginTableV1>()`, nesting reproduces the layout
/// that appending would produce.
#[repr(C)]
pub struct MetisPluginTableNext {
    pub v1: MetisPluginTableV1,
    pub set_log_level: MetisSetLogLevelFn,
}

/// Entry points an older host knows: everything up to `destroy_instance`.
/// A host may only read fields it knows, and only when `size` covers them.
#[repr(C)]
pub struct MetisTablePrefixOldHost {
    pub size: usize,
    pub abi_major: u32,
    pub abi_minor: u32,
    pub create_instance: MetisCreateInstanceFn,
    pub destroy_instance: MetisDestroyInstanceFn,
}

/// Reads the appended entry, but only when the plugin's reported size covers
/// it. When the plugin is older, nothing past its table is read.
///
/// # Safety
///
/// `table` must come from an entry point of ABI major 1.
pub unsafe fn read_appended(table: *const MetisPluginTableV1) -> Option<MetisSetLogLevelFn> {
    let prefix = unsafe { metis_read_prefix(table) }?;
    if prefix.size < size_of::<MetisPluginTableNext>() {
        return None;
    }
    let next = unsafe { &*(table as *const MetisPluginTableNext) };
    Some(next.set_log_level)
}

/// The layout identity that makes prefix reading sound: the appended field
/// lands exactly at `size_of::<MetisPluginTableV1>()`.
const _: () = assert!(size_of::<MetisPluginTableNext>() == size_of::<MetisPluginTableV1>() + size_of::<usize>());
