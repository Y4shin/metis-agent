//! The generated message types, mounted under `crate::proto`.
//!
//! Every package the contract declares gets one file per tree, because both
//! plugins run with `file_per_package`. The module tree below is handwritten;
//! the types are not.

pub mod metis {
    pub mod v1 {
        // Raw buffa output writes an explicit `Default` impl where a derive
        // would do.
        #![allow(clippy::derivable_impls)]

        include!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/generated/buffa/metis.v1.rs"
        ));
    }
}
