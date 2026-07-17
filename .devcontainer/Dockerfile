FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive

# mingw-w64 provides the x86_64-w64-mingw32 linker/toolchain used to cross-compile
# the hook DLL and the Tauri app to Windows (this project only ever ships Windows
# binaries — the hook is injected into a Windows game process).
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential curl git ca-certificates pkg-config \
    gcc-mingw-w64-x86-64 g++-mingw-w64-x86-64 && \
    rm -rf /var/lib/apt/lists/*

# winres (used by src-hook/build.rs to embed a Windows resource) shells out to the
# unprefixed "windres" name, so alias it to the mingw cross tool.
RUN ln -s /usr/bin/x86_64-w64-mingw32-windres /usr/bin/windres

# Node 20 to match CI (.github/workflows/ci.yaml uses actions/setup-node@v4 node-version 20).
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get install -y --no-install-recommends nodejs && \
    rm -rf /var/lib/apt/lists/*

# Ubuntu 24.04 ships with user 'ubuntu' already created with UID 1000.
USER ubuntu
WORKDIR /workspace

# Toolchain channel is pinned in rust-toolchain.toml (nightly-2024-05-04); install
# that exact toolchain (not "stable") so the Windows target lands on the one
# cargo will actually select once invoked inside /workspace.
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain nightly-2024-05-04 && \
    ~/.cargo/bin/rustup target add x86_64-pc-windows-gnu --toolchain nightly-2024-05-04 && \
    ~/.cargo/bin/rustup component add clippy rustfmt --toolchain nightly-2024-05-04

ENV PATH="/home/ubuntu/.cargo/bin:${PATH}"

# Point cargo at the mingw linker for the Windows target.
RUN mkdir -p ~/.cargo && \
    printf '[target.x86_64-pc-windows-gnu]\nlinker = "x86_64-w64-mingw32-gcc"\nar = "x86_64-w64-mingw32-gcc-ar"\n' >> ~/.cargo/config.toml
