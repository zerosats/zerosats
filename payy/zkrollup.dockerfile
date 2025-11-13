# Build binary
FROM rust:1-bookworm AS workspace

ARG SCCACHE_GCS_BUCKET
ARG SCCACHE_GCS_KEY_PREFIX

RUN rustup component add rustfmt

RUN apt-get update && apt-get install -y \
    curl jq \
    build-essential \
    pkg-config \
    libssl-dev \
    git \
    cmake \
    clang \
    libglib2.0-dev \
    libpq-dev \
    libprotobuf-dev \
    libc6-dev \
    libgflags-dev \
    libsnappy-dev \
    libc6 libstdc++6 \
    zlib1g-dev \
    libbz2-dev \
    liblz4-dev \
    libzstd-dev \
    ninja-build \
    python3 \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Ensure the toolchain specified in rust-toolchain.toml is installed
RUN rustup show

# Set `SYSROOT` to a dummy path (default is /usr) because pkg-config-rs *always*
# links those located in that path dynamically but we want static linking, c.f.
# https://github.com/rust-lang/pkg-config-rs/blob/54325785816695df031cef3b26b6a9a203bbc01b/src/lib.rs#L613
ENV SYSROOT=/dummy


# Conditional sccache setup: Only if bucket and key are provided
RUN --mount=type=secret,id=gcs_sa_key_base64,required=false \
    if [ -n "$SCCACHE_GCS_BUCKET" ] && [ -f /run/secrets/gcs_sa_key_base64 ]; then \
    cat /run/secrets/gcs_sa_key_base64 | base64 -d > /gcs_key.json && \
    wget https://github.com/mozilla/sccache/releases/download/v0.10.0/sccache-v0.10.0-x86_64-unknown-linux-musl.tar.gz && \
    tar -xzf sccache-v0.10.0-x86_64-unknown-linux-musl.tar.gz && \
    mv sccache-v0.10.0-x86_64-unknown-linux-musl/sccache /usr/local/cargo/bin/sccache && \
    rm -rf sccache-v0.10.0-x86_64-unknown-linux-musl sccache-v0.10.0-x86_64-unknown-linux-musl.tar.gz && \
    chmod +x /usr/local/cargo/bin/sccache; \
    fi
ENV SCCACHE_GCS_KEY_PATH=/gcs_key.json
ENV SCCACHE_GCS_BUCKET=$SCCACHE_GCS_BUCKET
ENV SCCACHE_GCS_KEY_PREFIX=$SCCACHE_GCS_KEY_PREFIX
ENV SCCACHE_GCS_RW_MODE=READ_WRITE


WORKDIR /build


FROM workspace AS tester

SHELL ["/bin/bash", "--login", "-c"]

RUN curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.39.7/install.sh | bash
RUN nvm install 20 \
    && ln -s "$(which node)" /usr/bin/node \
    && ln -s "$(which npm)" /usr/bin/npm \
    && npm install --global yarn \
    && ln -s "$(which yarn)" /usr/bin/yarn

# Download and install barretenberg
RUN wget https://github.com/AztecProtocol/aztec-packages/releases/download/v1.0.0-nightly.20250723/barretenberg-amd64-linux.tar.gz -O barretenberg.tar.gz && \
    tar -xzf barretenberg.tar.gz && \
    mv bb /usr/local/bin/bb && \
    rm barretenberg.tar.gz

# bb requires a recent glibcxx version
# Enable backports and pull libstdc++ 13.x (exports GLIBCXX_3.4.31)
# also installs jq, some bb commands require jq
RUN echo 'deb http://deb.debian.org/debian testing main' \
    >  /etc/apt/sources.list.d/testing.list         && \
    echo 'APT::Default-Release "stable";'               \
    >  /etc/apt/apt.conf.d/99defaultrelease         && \
    apt-get update                                      && \
    # pull only the two runtime libs from testing
    DEBIAN_FRONTEND=noninteractive \
    apt-get install -y -t testing clang libclang-dev libc6 libstdc++6 build-essential jq

SHELL ["sh", "-c"]

ARG RELEASE=1
ENV RELEASE=$RELEASE

COPY rust-toolchain.toml ./

COPY . .

# Run tests as part of RUN, not CMD, because prebuilding and running tests is tricky
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    chmod +x ./docker/test.sh && \
    if [ -f /gcs_key.json ]; then \
    echo "Using sccache with GCS for tests"; \
    RUSTC_WRAPPER=/usr/local/cargo/bin/sccache exec ./docker/test.sh && \
    sccache --show-stats && \
    sccache --stop-server; \
    else \
    echo "Skipping sccache for tests"; \
    exec ./docker/test.sh; \
    fi

CMD ["sh", "-c", "echo 'This image is not meant to be run, only built.' && exit 1"]


# Build binary
FROM workspace AS builder

ARG RELEASE=1

COPY rust-toolchain.toml ./


COPY rust-toolchain.toml ./
COPY .cargo/config.toml .cargo/config.toml
COPY Cargo.lock ./
COPY Cargo.toml ./
COPY pkg ./pkg

# Remove app package as its not needed
RUN sed 's|, "app/packages/react-native-rust-bridge/cpp/rustbridge"||g' Cargo.toml > Cargo.toml.tmp \
    && mv Cargo.toml.tmp Cargo.toml

# Add fixtures
COPY citrea/artifacts/contracts ./citrea/artifacts/contracts
COPY fixtures/params ./fixtures/params
COPY fixtures/programs ./fixtures/programs
COPY fixtures/keys ./fixtures/keys

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    build_flags=$([ "$RELEASE" = "1" ] && echo "--release"); \
    if [ -f /gcs_key.json ]; then \
    echo "Using sccache with GCS"; \
    if RUSTC_WRAPPER=/usr/local/cargo/bin/sccache cargo build --bin node --bin payy-cli ${build_flags}; then \
    sccache --show-stats && \
    sccache --stop-server; \
    else \
    status=$?; \
    echo "sccache-backed build failed (exit ${status}); retrying without sccache"; \
    sccache --stop-server || true; \
    cargo build --bin node --bin payy-cli ${build_flags}; \
    fi; \
    else \
    echo "Skipping sccache (missing required vars)"; \
    cargo build --bin node --bin payy-cli ${build_flags}; \
    fi

RUN cp /build/target/$([ "$RELEASE" = "1" ] && echo "release" || echo "debug")/node /build/target
RUN cp /build/target/$([ "$RELEASE" = "1" ] && echo "release" || echo "debug")/payy-cli /build/target


# Runtime stage - can be used for both node and prover mode
FROM debian:bookworm-slim as runtime

ENV ROOT_DIR /polybase
WORKDIR $ROOT_DIR

USER root

RUN groupadd -g 1001 --system spaceman && \
    useradd -u 1001 --system --gid spaceman --home "$ROOT_DIR" spaceman && \
    chown -R spaceman:spaceman "$ROOT_DIR"

RUN apt update && apt install -y curl nano libpq-dev postgresql wget tar curl

# Download and install barretenberg
RUN wget https://github.com/AztecProtocol/aztec-packages/releases/download/v1.0.0-nightly.20250723/barretenberg-amd64-linux.tar.gz -O barretenberg.tar.gz && \
    tar -xzf barretenberg.tar.gz && \
    mv bb /usr/local/bin/bb && \
    rm barretenberg.tar.gz

# Enable backports and pull libstdc++ 13.x (exports GLIBCXX_3.4.31)
# also installs jq, some bb commands require jq
RUN echo 'deb http://deb.debian.org/debian testing main' \
    >  /etc/apt/sources.list.d/testing.list         && \
    echo 'APT::Default-Release "stable";'               \
    >  /etc/apt/apt.conf.d/99defaultrelease         && \
    apt-get update                                      && \
    # pull only the two runtime libs from testing
    DEBIAN_FRONTEND=noninteractive \
    apt-get install -y -t testing libclang-dev  libc6 libstdc++6 jq

# Create directories and set permissions for spaceman user
# Create directories for both node and prover modes
RUN mkdir -p /tmp /.bb-crs /polybase/.polybase-prover/db /polybase/.polybase-prover/smirk && \
    chown spaceman:spaceman /tmp /.bb-crs /polybase/.polybase-prover /polybase/.polybase-prover/db /polybase/.polybase-prover/smirk && \
    chmod 755 /tmp /.bb-crs /polybase/.polybase-prover /polybase/.polybase-prover/db /polybase/.polybase-prover/smirk

ARG WAIT_SECONDS=0

ENV WAIT_SECONDS=$WAIT_SECONDS

RUN echo '#!/bin/bash\n\
    if [ "$WAIT_SECONDS" -gt 0 ]; then\n\
    echo "Waiting $WAIT_SECONDS seconds before starting..."\n\
    sleep $WAIT_SECONDS\n\
    fi\n\
    exec "$@"' > /entrypoint-wrapper.sh && chmod +x /entrypoint-wrapper.sh

USER spaceman

COPY --from=builder /build/target/node /usr/bin/node
COPY --from=builder /build/target/payy-cli /usr/bin/payy-cli

COPY config-prod.toml ./config.toml

STOPSIGNAL SIGTERM

# Expose both potential ports (node and prover)
EXPOSE 8080 8091 8092

# TODO: re-enable healthcheck once we have RPC
# HEALTHCHECK --interval=5s --timeout=5s --retries=3 CMD \
#     curl -f http://localhost:8080/v0/health || exit 1

# Default entrypoint for node mode
# For prover mode, override CMD in docker-compose.yml
ENTRYPOINT ["/entrypoint-wrapper.sh", "/usr/bin/node"]

VOLUME [ "$ROOT_DIR" ]
