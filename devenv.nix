{
  pkgs,
  ...
}:

{
  languages.rust = {
    enable = true;
    # Use the current nightly toolchain (rust-overlay). Omitting `version` picks
    # the latest nightly, which devenv.lock pins for reproducibility.
    channel = "nightly";
  };

  packages = [
    pkgs.git
    # `buf` generates the wire contract and runs the contract gates in CI. The
    # plugins it runs are BSR remote plugins, so nothing else is installed:
    # no `protoc`, no local plugin binary, no JVM.
    pkgs.buf
    # `cbindgen` turns the `repr(C)` Rust in `crates/plugin-contract` into the
    # committed C header; `scripts/regenerate-abi-header.sh` runs it. The nixpkgs
    # attribute is `rust-cbindgen` (0.29.4, the version the spine measured), and
    # the binary it installs is `cbindgen`.
    pkgs.rust-cbindgen
    # The SvelteKit frontend under apps/ui builds inside this shell too.
    pkgs.nodejs
  ];
}
