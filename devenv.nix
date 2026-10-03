{ pkgs, ... }:

{
  languages.rust = {
    enable = true;
    channel = "stable";
    components = [ "rustc" "cargo" "clippy" "rustfmt" "rust-analyzer" ];
  };

  scripts.build.exec = ''
    set -euo pipefail

    ROOT=$(git rev-parse --show-toplevel)
    cd "$ROOT"

    cargo fmt --all
    cargo clippy --workspace
    cargo test --workspace
    cargo build --workspace --release
  '';
}
