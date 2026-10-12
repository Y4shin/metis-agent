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
    # The SvelteKit frontend under apps/ui builds inside this shell too.
    pkgs.nodejs
  ];
}
