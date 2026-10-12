//! Regenerates (and, with `--check`, verifies) the committed cbindgen header.
//!
//! `repr(C)` Rust in `spike-abi` is the source of truth (D2); this is the same
//! shape the real `crates/plugin-contract` would use, including the CI check
//! that regeneration produces no diff.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let abi_crate = manifest.parent().expect("crates dir").join("abi");
    let header_path = abi_crate.join("include").join("metis_plugin_abi.h");

    let generated = match cbindgen::Builder::new()
        .with_crate(&abi_crate)
        .with_config(cbindgen::Config::from_file(abi_crate.join("cbindgen.toml")).expect("cbindgen.toml"))
        .generate()
    {
        Ok(bindings) => {
            let mut buffer: Vec<u8> = Vec::new();
            bindings.write(&mut buffer);
            String::from_utf8(buffer).expect("cbindgen writes UTF-8")
        }
        Err(err) => {
            eprintln!("cbindgen failed: {err}");
            return ExitCode::FAILURE;
        }
    };

    if std::env::args().any(|arg| arg == "--check") {
        return check(&header_path, &generated);
    }

    if let Some(dir) = header_path.parent() {
        std::fs::create_dir_all(dir).expect("create include dir");
    }
    std::fs::write(&header_path, &generated).expect("write header");
    println!("wrote {}", header_path.display());
    ExitCode::SUCCESS
}

fn check(header_path: &Path, generated: &str) -> ExitCode {
    match std::fs::read_to_string(header_path) {
        Ok(committed) if committed == generated => {
            println!("{} is up to date", header_path.display());
            ExitCode::SUCCESS
        }
        Ok(_) => {
            eprintln!("{} differs from the generated header:", header_path.display());
            print!("{generated}");
            ExitCode::from(2)
        }
        Err(err) => {
            eprintln!("cannot read {}: {err}", header_path.display());
            ExitCode::FAILURE
        }
    }
}
