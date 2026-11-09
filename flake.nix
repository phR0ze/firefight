# firefight dev shell: Rust + Flutter (Linux desktop only)
# - Mirrors apps.dev.rust and apps.dev.flutter from nixos-config
# - https://nixos.wiki/wiki/Rust
# - https://nixos.wiki/wiki/Flutter
#
# nativeBuildInputs vs buildInputs vs packages
# - for mkShells they are all treated the same
# - [longer explanation](https://discourse.nixos.org/t/use-buildinputs-or-nativebuildinputs-for-nix-shell/8464)
{
  inputs = {
    # nixos-unstable from 2026.11.03, matches nixos-config's nixpkgs-unstable
    nixpkgs.url = "github:nixos/nixpkgs/1d060086d60cb76a40cf37109f979d88a5bd6396";
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = { self, nixpkgs, flake-utils, ... }: flake-utils.lib.eachSystem [ "x86_64-linux" "aarch64-linux" ] (system: let
    pkgs = import nixpkgs {
      inherit system;
      config.allowUnfree = true;
    };
  in
  {
    devShells.default = pkgs.mkShell {

      nativeBuildInputs = with pkgs; [
        bashInteractive                   # Solve for normal shell operation

        # LLVM tooling
        clang                             # A C language family frontend for LLVM
        lldb                              # Next gen high-performance debugger for Rust
        llvmPackages.bintools             # Use lld instead of ld

        # Rust dependencies
        cargo                             # Rust project dependency management tooling
        rustc                             # Rust compiler
        rust-analyzer                     # Rust LSP
        rustfmt                           # Rust code formatter
        clippy                            # Rust linter

        # Native build requirements
        gnumake                           # A tool to control the generation of non-source files from sources
        pkg-config                        # System dependency path resolution
        openssl                           # Cryptographic implementation of the SSL and TLS protocols

        # Flutter dependencies
        flutter                           # Flutter support and its CLI
        cmake                             # Flutter Linux desktop build system
        ninja                             # Flutter Linux desktop build backend
        gtk3                              # Flutter Linux desktop embedder toolkit
      ];

      # Set flutter and dart SDK locations to get correct versions
      FLUTTER_ROOT = "${pkgs.flutter}";
      DART_ROOT = "${pkgs.flutter}/bin/cache/dart-sdk";

      # Set the rust source path for rust-analyzer to be happy
      RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";

      shellHook = ''
        export PATH="$HOME/.cargo/bin:$PATH"
        echo "firefight dev shell: $(rustc --version), $(flutter --version 2>/dev/null | head -n1)"
      '';
    };
  });
}
