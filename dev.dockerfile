ARG CITREA_BASE_IMAGE=satsbridge/ciphera:citrea
FROM ${CITREA_BASE_IMAGE}

ARG BB_VERSION=1.0.0-nightly.20250723

ENV DEBIAN_FRONTEND=noninteractive

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
    libc++-dev

# Install Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.88.0

RUN curl -fsSL https://deb.nodesource.com/setup_22.x -o nodesource_setup.sh
RUN bash nodesource_setup.sh

RUN apt-get update && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

RUN curl -L https://raw.githubusercontent.com/noir-lang/noirup/refs/heads/main/install | bash
RUN . /root/.bashrc && noirup -v 1.0.0-beta.14

COPY ./ciphera/web/binaries/linux64/barretenberg-amd64-linux.tar.gz barretenberg.tar.gz
RUN tar -xzf barretenberg.tar.gz && \
    mv bb /usr/local/bin/bb && \
    rm barretenberg.tar.gz

ENV PATH="/root/.cargo/bin:/usr/local/bin:$PATH"

# Set bash as entrypoint with login shell to ensure profile is sourced
ENTRYPOINT ["/bin/bash", "--login"]

# Default command is interactive shell
CMD ["-i"]

# Build metadata
LABEL maintainer="Ciphera Development Team"
LABEL description="Aztec Protocol base image with Rust 1.88.0 and Ciphera development environment"
LABEL version="1.0"
