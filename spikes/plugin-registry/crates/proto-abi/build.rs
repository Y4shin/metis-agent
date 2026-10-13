//! Compiles the C translation unit that static-asserts the ABI layout (lane 3).
//!
//! `c/layout_static_asserts.c` includes the committed generated header and
//! refuses to compile when its sizes, alignments or offsets drift from the
//! pointer-size formulas in `src/layout.rs`. `tests/layout.rs` then compares the
//! same numbers as measured by the C compiler with Rust's `size_of`,
//! `align_of` and `offset_of`, so both sides of the agreement are checked by
//! every build.
//!
//! `-std=c11` because `_Static_assert` and `_Alignof` are C11, `-Wall -Wextra`
//! for the same warning set the landed crate uses, and `-Werror` so any
//! diagnostic the generated header produces fails the build instead of being
//! printed and ignored.

fn main() {
    println!("cargo::rerun-if-changed=c/layout_static_asserts.c");
    println!("cargo::rerun-if-changed=include/metis_plugin_abi.h");

    cc::Build::new()
        .file("c/layout_static_asserts.c")
        .include("include")
        // `_FORTIFY_SOURCE` reaches this TU from the toolchain environment, and
        // it warns unless an optimisation level is set, so pin one. The layout
        // asserts do not care about the level; the build does care that no
        // warning is turned into an error by `-Werror`.
        .opt_level(2)
        .flag_if_supported("-std=c11")
        .flag_if_supported("-Wall")
        .flag_if_supported("-Wextra")
        .flag_if_supported("-Werror")
        .compile("proto_abi_layout");
}
