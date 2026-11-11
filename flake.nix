# firefight dev shell: Rust + Flutter (Linux desktop and Android control deck)
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
      config.android_sdk.accept_license = true;
    };

    # Android SDK for the control deck APK; versions match Flutter's defaults
    # (flutter.compileSdkVersion / targetSdkVersion / ndkVersion), plus platform 35 and
    # CMake 3.22.1 which the path_provider plugin (via jni) builds with. The NDK and CMake
    # come from Nix because Gradle's own downloads are not patched to run on NixOS.
    androidBuildTools = "36.0.0";
    androidSdk = pkgs.androidenv.composeAndroidPackages {
      platformVersions = [ "35" "36" ];
      buildToolsVersions = [ androidBuildTools ];
      includeNDK = true;
      ndkVersions = [ "28.2.13676358" ];
      cmakeVersions = [ "3.22.1" ];
      includeSystemImages = false;
      includeEmulator = false;
    };
  in
  {
    # Run: `nix run .#emulator`
    # Boots a local Android emulator (KVM accelerated) for trying the control deck; then
    # `cd app && flutter run` from the dev shell installs and starts the app on it. The
    # virtual device persists in ~/.local/share/firefight-emulator between runs. Inside the
    # emulator the host machine, and so a local firefight-server, is reachable at 10.0.2.2.
    packages.emulator = pkgs.androidenv.emulateApp {
      name = "firefight-emulator";
      platformVersion = "35";
      abiVersion = "x86_64";
      systemImageType = "default";            # plain AOSP image, no Google apps
      deviceName = "firefight";
      # Expanded by the launcher script at runtime, so the device survives between runs.
      androidUserHome = "$HOME/.local/share/firefight-emulator";
      androidAvdFlags = "--device pixel_6";
      configOptions = {
        "hw.keyboard" = "yes";                # type with the host keyboard
        "hw.ramSize" = "2048";
      };
    };
    apps.emulator = {
      type = "app";
      program = "${self.packages.${system}.emulator}/bin/run-test-emulator";
    };

    devShells.default = pkgs.mkShell {

      nativeBuildInputs = with pkgs; [
        bashInteractive                   # Solve for normal shell operation
        just                              # Build recipes in the justfile

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
        alsa-lib                          # Audio output for the agent's sound actions

        # Flutter dependencies
        flutter                           # Flutter support and its CLI
        cmake                             # Flutter Linux desktop build system
        ninja                             # Flutter Linux desktop build backend
        gtk3                              # Flutter Linux desktop embedder toolkit
        libx11                            # tray_manager/window_manager native code (nativeapi)
        libxi                             # tray_manager/window_manager native code (nativeapi)

        # Android build dependencies
        jdk17                             # Gradle / Android Gradle Plugin runtime
        androidSdk.androidsdk             # Android SDK (platform, build-tools, platform-tools)
      ];

      # Set flutter and dart SDK locations to get correct versions
      FLUTTER_ROOT = "${pkgs.flutter}";
      DART_ROOT = "${pkgs.flutter}/bin/cache/dart-sdk";

      # Set the rust source path for rust-analyzer to be happy
      RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";

      shellHook = ''
        export PATH="$HOME/.cargo/bin:$PATH"

        # The Nix store is read-only, so Android tools can't write license acceptance files
        # there. Build a writable SDK directory that symlinks the Nix SDK components but
        # has a real (writable) licenses/ directory.
        NIX_SDK="${androidSdk.androidsdk}/libexec/android-sdk"
        WRITABLE_SDK="$HOME/.local/share/android-sdk"
        if [ "$(readlink "$WRITABLE_SDK/.nix-sdk-source" 2>/dev/null)" != "$NIX_SDK" ]; then
          echo "Updating Android SDK wrapper at $WRITABLE_SDK..."
          rm -rf "$WRITABLE_SDK"
          mkdir -p "$WRITABLE_SDK"
          for item in "$NIX_SDK"/*; do
            name=$(basename "$item")
            [ "$name" != "licenses" ] && ln -sf "$item" "$WRITABLE_SDK/$name"
          done
          ln -sf "$NIX_SDK" "$WRITABLE_SDK/.nix-sdk-source"
        fi
        [ -L "$WRITABLE_SDK/licenses" ] && rm "$WRITABLE_SDK/licenses"
        mkdir -p "$WRITABLE_SDK/licenses"
        # Canonical license hashes checked by sdkmanager / flutter doctor
        printf "24333f8a63b6825ea9c5514f83c2829b004d1fee\n8933bad161af4178b1185d1a37fbf41ea5269c55\nd56f5187479451eabf01fb78af6dfcb131a6481e" \
          > "$WRITABLE_SDK/licenses/android-sdk-license"
        printf "84831b9409646a918e30573bab4c9c91346d8abd\nd975f751698a77b662f1254ddbeed3901e976f5a" \
          > "$WRITABLE_SDK/licenses/android-sdk-preview-license"
        export ANDROID_HOME="$WRITABLE_SDK"
        export ANDROID_SDK_ROOT="$WRITABLE_SDK"
        export JAVA_HOME="${pkgs.jdk17}"
        export PATH="$WRITABLE_SDK/platform-tools:$PATH"
        # Dart build hooks (e.g. nativeapi's, behind tray_manager/window_manager) run with a filtered
        # environment that drops PKG_CONFIG_PATH, so give them a pkg-config that restores this
        # shell's search path.
        PKGCFG_BIN="$HOME/.cache/firefight-devshell/bin"
        mkdir -p "$PKGCFG_BIN"
        REAL_PKG_CONFIG="$(command -v pkg-config)"
        if [ "$REAL_PKG_CONFIG" != "$PKGCFG_BIN/pkg-config" ]; then
          printf '#!/bin/sh\nexport PKG_CONFIG_PATH="%s''${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"\nexec %s "$@"\n' \
            "$PKG_CONFIG_PATH" "$REAL_PKG_CONFIG" > "$PKGCFG_BIN/pkg-config"
          chmod +x "$PKGCFG_BIN/pkg-config"
        fi
        export PATH="$PKGCFG_BIN:$PATH"

        # Gradle downloads a dynamically linked aapt2 that cannot run on NixOS; use the SDK's.
        export GRADLE_OPTS="-Dorg.gradle.project.android.aapt2FromMavenOverride=$WRITABLE_SDK/build-tools/${androidBuildTools}/aapt2"

        echo -e "firefight dev shell:\n - $(rustc --version)\n - $(flutter --version 2>/dev/null | head -n1)"
      '';
    };
  });
}
