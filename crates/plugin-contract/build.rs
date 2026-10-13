//! Compiles the C translation unit that static-asserts the ABI layout (D13).
//!
//! `c/layout_static_asserts.c` includes the committed generated header and
//! refuses to compile when its sizes, alignments or offsets drift from the
//! pointer-size formulas in `src/abi/layout.rs`. `tests/c_layout.rs` then
//! compares the same numbers as measured by the C compiler with Rust's
//! `size_of`, `align_of` and `offset_of`, so both sides of the agreement are
//! checked by every build, and CI's build and test steps carry the C compiler
//! check without a step of its own.

fn main() {
    println!("cargo::rerun-if-changed=c/layout_static_asserts.c");
    println!("cargo::rerun-if-changed=include/metis_plugin_abi.h");

    cc::Build::new()
        .file("c/layout_static_asserts.c")
        .include("include")
        // `_Static_assert` and `_Alignof` need C11.
        .flag_if_supported("-std=c11")
        .flag_if_supported("-Wall")
        .flag_if_supported("-Wextra")
        .compile("plugin_c_layout");
}
