# Dockerfile.aztec-rust
FROM citrea

# Set non-interactive mode to avoid tzdata prompts
ENV DEBIAN_FRONTEND=noninteractive

# Install system dependencies for Rust and development
RUN apt-get update && apt-get install -y \
    jq \
    nano \
    curl \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    cmake \
    clang \
    libc6-dev \
    libgflags-dev \
    libsnappy-dev \
    zlib1g-dev \
    libbz2-dev \
    liblz4-dev \
    libzstd-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

RUN curl -L https://raw.githubusercontent.com/noir-lang/noirup/refs/heads/main/install | bash
RUN . /root/.bashrc && noirup

RUN curl -L https://raw.githubusercontent.com/AztecProtocol/aztec-packages/refs/heads/master/barretenberg/bbup/install | bash
RUN . /root/.bashrc && bbup -v 1.0.0-nightly.20250723

ENV PATH="$PATH:/usr/src/noir/noir-repo/target/release:/usr/src/barretenberg/cpp/build/bin"

# Create a workspace directory
WORKDIR /app
COPY ./payy .

# Set bash as entrypoint with login shell to ensure profile is sourced
ENTRYPOINT ["/bin/bash", "--login"]

# Default command is interactive shell
CMD ["-i"]

# Build metadata
LABEL maintainer="Payy Development Team"
LABEL description="Aztec Protocol base image with Rust 1.88.0 and Payy development environment"
LABEL version="1.0"