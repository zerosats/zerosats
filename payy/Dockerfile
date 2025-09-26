# Dockerfile.aztec-rust
FROM aztecprotocol/aztec

# Set non-interactive mode to avoid tzdata prompts
ENV DEBIAN_FRONTEND=noninteractive

# Install system dependencies for Rust and development
RUN apt-get update && apt-get install -y \
    curl \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    cmake \
    clang \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Install Rust 1.88.0 specifically
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
    --default-toolchain 1.88.0 \
    --profile default \
    --component clippy,rustfmt \
    -y

# Set up Rust environment for all subsequent commands
ENV CARGO_HOME="/root/.cargo"
ENV RUSTUP_HOME="/root/.rustup"
ENV PATH="/root/.cargo/bin:${PATH}"

# Verify Rust installation and show versions
RUN rustc --version && \
    cargo --version && \
    rustup --version

ENV PATH="$PATH:/usr/src/noir/noir-repo/target/release:/usr/src/barretenberg/cpp/build/bin"

# Create a workspace directory
WORKDIR /workspace

# Set bash as entrypoint with login shell to ensure profile is sourced
ENTRYPOINT ["/bin/bash", "--login"]

# Default command is interactive shell
CMD ["-i"]

# Build metadata
LABEL maintainer="Payy Development Team"
LABEL description="Aztec Protocol base image with Rust 1.88.0 and Payy development environment"
LABEL version="1.0"