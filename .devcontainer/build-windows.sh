#!/usr/bin/env bash
# Cross-compiles the hook DLL, the Tauri app, and the injector for Windows from
# inside the devcontainer. Mirrors the two cargo invocations from
# .github/workflows/ci.yaml, but on native Windows CI both land in
# target/release/ for free because host == target; here we build for
# x86_64-pc-windows-gnu and copy hook.dll into the path src-tauri/build.rs
# expects (../target/release/hook.dll) before the main build, since that copy
# is what tauri_build's resource bundling relies on existing. The final
# workspace-wide build also produces target/$TARGET/release/injector.exe.
set -euo pipefail

cd "$(dirname "$0")/.."

TARGET=x86_64-pc-windows-gnu
EXTRA_ARGS=("$@")

cargo build --release --package hook --target "$TARGET" "${EXTRA_ARGS[@]}"
mkdir -p target/release
cp "target/$TARGET/release/hook.dll" target/release/hook.dll

cargo build --release --target "$TARGET" "${EXTRA_ARGS[@]}"
