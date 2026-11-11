# firefight build recipes. Each runs inside the flake's dev shell, entering it when needed.

set shell := ["bash", "-euo", "pipefail", "-c"]

# Already inside `nix develop`? Then run commands directly.
dev := if env("IN_NIX_SHELL", "") != "" { "" } else { "nix develop " + justfile_directory() + " --command" }

# Throwaway server data dir, agent config and run state for `just run`
data := "/tmp/ff-test"

# List the recipes
default:
    @just --list

# Build release artifacts: `linux` (default) for the server, agent and Linux app; `mobile` for the Android APK
build target="linux":
    {{ if target == "linux" { "just _build-linux" } else if target == "mobile" { "just _build-mobile" } else { error("unknown build target '" + target + "', expected linux or mobile") } }}

_build-linux:
    {{ dev }} cargo build --release
    cd app && {{ dev }} flutter build linux --release

# Signed with the debug key, for sideloading
_build-mobile:
    {{ dev }} just _apk
    mkdir -p build
    cp app/build/app/outputs/flutter-apk/app-release.apk build/firefight.apk
    @echo "APK: build/firefight.apk (install with: adb install build/firefight.apk)"

# The tray plugins only run in the Linux client display (see app/lib/main.dart), but their native
# library, cnativeapi, has a build hook that compiles its C++ core with the NDK for every Android ABI.
# The APK build swaps in a copy of the package without the hook: its Dart bindings still compile,
# and nothing on Android ever loads the library. pubspec.lock is restored afterwards.
[working-directory: 'app']
_apk:
    #!/usr/bin/env bash
    set -euo pipefail
    version=$(awk '/^  cnativeapi:/ { found = 1 } found && $1 == "version:" { gsub(/"/, "", $2); print $2; exit }' pubspec.lock)
    src="${PUB_CACHE:-$HOME/.pub-cache}/hosted/pub.dev/cnativeapi-$version"
    [ -d "$src" ] || flutter pub get >/dev/null
    stub=build/android-stubs/cnativeapi
    rm -rf "$stub" && mkdir -p "$stub"
    cp -r "$src/lib" "$stub/"
    chmod -R u+w "$stub"
    # A path dependency can't be part of the package's upstream workspace.
    sed '/^resolution: workspace/d' "$src/pubspec.yaml" > "$stub/pubspec.yaml"

    cp pubspec.lock build/pubspec.lock.orig
    restore() {
        rm -f pubspec_overrides.yaml
        mv build/pubspec.lock.orig pubspec.lock
        flutter pub get >/dev/null
    }
    trap restore EXIT
    printf 'dependency_overrides:\n  cnativeapi:\n    path: %s\n' "$stub" > pubspec_overrides.yaml
    # Android only installs an APK over the existing app when its version code is higher, so
    # each build takes minutes since the epoch (always rising, and far below Android's limit).
    flutter build apk --release --build-number=$(( $(date +%s) / 60 ))

# Run a component locally: server, deck, agent, client or mobile. Restarts it if already running.
run target:
    #!/usr/bin/env bash
    set -euo pipefail

    if [ -z "${IN_NIX_SHELL:-}" ]; then
      exec nix develop "{{justfile_directory()}}" --command just run "{{target}}"
    fi

    case "{{target}}" in
      server)
        cmd=(cargo run -p firefight-server -- --data-dir "{{data}}")
        ;;
      deck)
        token="{{data}}/admin_token"
        if [ ! -f "$token" ]; then
          echo "no admin token in {{data}} yet; start the server first: just run server" >&2
          exit 1
        fi
        cd app
        cmd=(flutter run -d linux -a --mode=server -a --server=localhost:7878 -a "--token=$(cat "$token")")
        ;;
      agent)
        cmd=(cargo run -p firefight-agent -- --server localhost:7878 --name "Test box" --config "{{data}}/agent.toml")
        ;;
      client)
        cd app
        cmd=(flutter run -d linux -a --mode=client)
        ;;
      mobile)
        # The emulator is slow to boot, so it is left running between restarts.
        device=$(adb devices | awk '/^emulator-/ { print $1; exit }')
        if [ -z "$device" ]; then
          nix run .#emulator
          device=$(adb devices | awk '/^emulator-/ { print $1; exit }')
        fi
        echo "inside the emulator the server is at 10.0.2.2:7878"
        cd app
        cmd=(flutter run -d "$device")
        ;;
      *)
        echo "unknown target '{{target}}'; expected server, deck, agent, client or mobile" >&2
        exit 1
        ;;
    esac

    # Stop a previous run of this target, along with everything it started (e.g. the
    # binary under cargo, the app under flutter). The environment marker guards against
    # the recorded pid having been reused by an unrelated process.
    marker="FIREFIGHT_RUN={{target}}"
    pidfile="{{data}}/run/{{target}}.pid"
    mkdir -p "$(dirname "$pidfile")"
    if [ -f "$pidfile" ]; then
      old=$(cat "$pidfile")
      if tr '\0' '\n' < "/proc/$old/environ" 2>/dev/null | grep -qx "$marker"; then
        tree() { echo "$1"; for c in $(pgrep -P "$1"); do tree "$c"; done; }
        alive() { for p; do kill -0 "$p" 2>/dev/null && return 0; done; return 1; }
        pids=$(tree "$old")
        echo "stopping the running {{target}} (pid $old)"
        kill $pids 2>/dev/null || true
        for _ in $(seq 50); do alive $pids || break; sleep 0.1; done
        kill -9 $pids 2>/dev/null || true
      fi
    fi

    export FIREFIGHT_RUN="{{target}}"
    echo $$ > "$pidfile"
    exec "${cmd[@]}"
