//! The generated Connect stubs, mounted under `crate::connect`.
//!
//! `crate::proto` is the mount point the generated stubs name, because the
//! connect plugin runs with `buffa_module=crate::proto`.

pub mod metis {
    pub mod v1 {
        // The generated fallback dispatcher emits single-arm `match` blocks
        // when the service has few methods.
        #![allow(clippy::match_single_binding)]

        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/generated/connect/metis.v1.rs"
        ));
    }
}
