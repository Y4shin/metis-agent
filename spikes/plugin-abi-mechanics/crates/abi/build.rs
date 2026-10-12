//! Compiles the C translation unit that static-asserts the ABI layout (D13).
//!
//! The C file includes the committed `cbindgen` header and refuses to compile
//! when the header's sizes, alignments or offsets drift from the literals in
//! `src/layout.rs`. `tests/c_layout.rs` then compares the exported `SPIKE_*`
//! symbols with the Rust values and drives the C-authored function table.

fn main() {
    println!("cargo:rerun-if-changed=c/layout_static_asserts.c");
    println!("cargo:rerun-if-changed=include/metis_plugin_abi.h");

    cc::Build::new()
        .file("c/layout_static_asserts.c")
        .include("include")
        .flag_if_supported("-std=c11")
        .flag_if_supported("-Wall")
        .flag_if_supported("-Wextra")
        .compile("spike_c_layout");
}
