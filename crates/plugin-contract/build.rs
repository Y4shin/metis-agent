//! Compiles the C translation unit that static-asserts the ABI layout (D13).
//!
//! `c/layout_static_asserts.c` includes the committed generated header and
//! refuses to compile when its sizes, alignments or offsets drift from the
//! pointer-size formulas in `src/abi/layout.rs`. `tests/c_layout.rs` then
//! compares the same numbers as measured by the C compiler with Rust's
//! `size_of`, `align_of` and `offset_of`, so both sides of the agreement are
//! checked by every build, and CI's build and test steps carry the C compiler
//! check without a step of its own.
//!
//! The translation unit is written in the C89 subset, because `cl.exe`
//! compiles `.c` files in its legacy C mode and the recorded MSVC invocation
//! carries no `/std:` flag (measured:
//! `docs/bugs/archive/plugin-contract-c-static-assert-msvc.md`). That property
//! cannot be checked by a host build, which compiles the file in this host's
//! default mode: `scripts/check-c-layout-asserts.sh` holds it instead, by
//! compiling the unit in strict ISO C89 under gcc and clang with warnings
//! denied, and by compiling a deliberately broken copy to prove that a
//! mismatch still fails the build.

fn main() {
    println!("cargo::rerun-if-changed=c/layout_static_asserts.c");
    println!("cargo::rerun-if-changed=include/metis_plugin_abi.h");

    let mut build = cc::Build::new();
    build
        .file("c/layout_static_asserts.c")
        .include("include")
        // Pinned for gcc and clang. The C89 subset the unit stays inside is
        // what `cl.exe` compiles it as, since that compiler does not take this
        // flag's spelling and therefore stays in its legacy C mode.
        .flag_if_supported("-std=c11");

    // The MSVC invocation denies warnings. It is the one invocation no
    // compiler here can reproduce, so it carries the strictest bar it can be
    // given; `cc` asks MSVC for `-W4` by default, and the `-Wall` this build
    // script used to add reached `cl.exe` as its all-warnings level, which is
    // not what gcc's `-Wall` means and is not a level to deny warnings at.
    //
    // gcc and clang are not denied warnings here: `cc` compiles at `-O0`
    // (measured: at `-O0` this host's glibc promotes its own fortify warning to
    // an error under `-Werror`, which would fail every debug build).
    // `scripts/check-c-layout-asserts.sh` holds those two compilers to a
    // warnings-denied compile of the same file instead, which is where that bar
    // belongs.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        build.warnings_into_errors(true);
    }

    build.compile("plugin_c_layout");
}
