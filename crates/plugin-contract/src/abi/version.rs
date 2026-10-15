//! The version gate: reading a table only after checking the version and the
//! size that is supposed to cover it.
//!
//! Two things are checked before any entry is read, and they are separate on
//! purpose. The *version* refuses a table this build cannot interpret: a
//! different major, or a minor older than the one this build knows, which is
//! what refuses a plugin whose entries are in a superseded order instead of
//! reading them through this one. The *size* refuses a table too short to cover
//! an entry this build needs, which is the presence rule the whole ABI rests
//! on.
//!
//! The minor check is additive: a table reporting the minor this build knows or
//! a newer one is accepted, because additive change inside major 1 is
//! append-only by `size`, so a newer minor's entries are appended after the last
//! entry this build reads and are never reached. A minor older than this
//! build's is refused, because the entry order itself changed and the entries
//! this build reads would be read at the wrong offsets. `found >= known` is the
//! whole rule; the retired pre-release minor 0 stays refused by it.
//!
//! The order of the two checks is part of the contract: the version is checked
//! first, so a table of the wrong version is refused before its bytes are
//! interpreted at all, whatever size it reports.

use core::mem::size_of;

use super::{
    METIS_ABI_MAJOR, METIS_ABI_MINOR, MetisPluginTableV1, MetisTablePrefix, metis_read_prefix,
};

/// Why a plugin table was refused.
///
/// The variants exist so a caller can tell a plugin built for another ABI
/// apart from a plugin whose table is simply too short, and so a per-capability
/// gate can report the first without treating it as corruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MetisAbiError {
    /// The table pointer was null, or its `size` did not cover the version
    /// prefix, so nothing could be read at all.
    Unreadable,
    /// The table's `abi_major` is not the major this build speaks.
    MajorMismatch {
        /// The major this build speaks.
        expected: u32,
        /// The major the table reported.
        found: u32,
    },
    /// The table's `abi_minor` is older than the one this build knows.
    ///
    /// Additive tolerance, in one direction only: a table reporting this
    /// build's minor or a newer one is accepted, and a table reporting an older
    /// one is refused, because the older table's entries are in a superseded
    /// order and reading them through this build's order would interpret the
    /// wrong bytes.
    MinorMismatch {
        /// The minor this build knows.
        known: u32,
        /// The minor the table reported, which is older.
        found: u32,
    },
    /// The table's `size` does not cover an entry this build requires.
    EntryMissing {
        /// The first entry the reported `size` does not reach.
        entry: &'static str,
    },
}

impl core::fmt::Display for MetisAbiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MetisAbiError::Unreadable => write!(f, "the plugin table is null or too short to read"),
            MetisAbiError::MajorMismatch { expected, found } => write!(
                f,
                "the plugin table reports ABI major {found}, and this build speaks major {expected}"
            ),
            MetisAbiError::MinorMismatch { known, found } => write!(
                f,
                "the plugin table reports ABI minor {found}, and this build knows minor {known}"
            ),
            MetisAbiError::EntryMissing { entry } => write!(
                f,
                "the plugin table's size does not cover the {entry} entry"
            ),
        }
    }
}

impl std::error::Error for MetisAbiError {}

/// The entries of the plugin table in slot order, with the `size` that covers
/// each one.
///
/// The list is the presence rule spelled out: an entry is there exactly when
/// the reported `size` reaches its slot.
/// One pointer slot, which is also one `size_t` on the supported targets: the
/// C translation unit asserts that `sizeof(size_t) == sizeof(void *)`, so the
/// two are written as one number here rather than assumed equal in silence.
const SLOT: usize = size_of::<*const core::ffi::c_void>();

const PLUGIN_ENTRIES: [(&str, usize); 8] = [
    ("init_plugin", size_of::<MetisTablePrefix>() + SLOT),
    ("destroy_instance", size_of::<MetisTablePrefix>() + 2 * SLOT),
    (
        "list_capabilities",
        size_of::<MetisTablePrefix>() + 3 * SLOT,
    ),
    ("invoke_future", size_of::<MetisTablePrefix>() + 4 * SLOT),
    ("cancel", size_of::<MetisTablePrefix>() + 5 * SLOT),
    ("release_buffer", size_of::<MetisTablePrefix>() + 6 * SLOT),
    ("shutdown", size_of::<MetisTablePrefix>() + 7 * SLOT),
    ("service_complete", size_of::<MetisTablePrefix>() + 8 * SLOT),
];

/// Reads and versions a plugin table.
///
/// The checks run in this order, and each one only touches bytes the previous
/// one made safe: the prefix (which carries `size`, `abi_major` and
/// `abi_minor`) is read first, then the version fields, then the entries.
///
/// Only the entries this build's layout carries are read. A table that reports a
/// longer `size`, such as a plugin built by a later revision of this ABI, is
/// accepted: the entries past this build's last one are appended and are never
/// touched, because nothing here names them. A table that reports a shorter
/// `size` than an entry this build needs is refused with the name of the first
/// entry the reported size does not cover.
///
/// # Safety
///
/// `table` must point at a live, immutable table that stays alive for `'a`.
/// The returned reference is only valid past the entries the reported `size`
/// was checked to cover.
pub unsafe fn metis_read_table<'a>(
    table: *const MetisPluginTableV1,
) -> Result<&'a MetisPluginTableV1, MetisAbiError> {
    // SAFETY: the caller vouches for the pointer, and `metis_read_prefix` reads
    // only the prefix and only after checking the reported `size` covers it.
    let prefix = unsafe { metis_read_prefix(table) }.ok_or(MetisAbiError::Unreadable)?;
    if prefix.abi_major != METIS_ABI_MAJOR {
        return Err(MetisAbiError::MajorMismatch {
            expected: METIS_ABI_MAJOR,
            found: prefix.abi_major,
        });
    }
    if prefix.abi_minor < METIS_ABI_MINOR {
        return Err(MetisAbiError::MinorMismatch {
            known: METIS_ABI_MINOR,
            found: prefix.abi_minor,
        });
    }
    for (entry, needed) in PLUGIN_ENTRIES {
        if prefix.size < needed {
            return Err(MetisAbiError::EntryMissing { entry });
        }
    }
    // SAFETY: the size checks above cover every field of the table, so the
    // whole struct may be read. A long table is fine: only the fields named
    // above are read, and every one of them is inside the reported size.
    Ok(unsafe { &*table })
}
