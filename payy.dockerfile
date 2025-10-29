FROM rust:1.88-slim-trixie AS build

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y \
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

RUN rustup override set 1.88.0
RUN rustup component add rustfmt

COPY ./payy /app

WORKDIR /app/pkg

RUN SKIP_GUEST_BUILD=1 cargo build --release

# our final base
FROM rust:1.88-slim-trixie

# copy the build artifact from the build stage
# TODO: think about paths

WORKDIR /app

COPY --from=build /app/target/release/burn-substitutor .
COPY --from=build /app/target/release/generate_key .
COPY --from=build /app/target/release/vk_hash .
COPY --from=build /app/target/release/node .
COPY --from=build /app/config-prod.toml config.toml

EXPOSE 8091
EXPOSE 5000

ENTRYPOINT ["sh", "-c", "./node"]
