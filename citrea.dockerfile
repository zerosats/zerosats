FROM rust:1.88-slim-trixie AS build
WORKDIR /app


# Install system dependencies
RUN apt-get update && apt-get -y upgrade && \
    apt-get install -y g++ libclang-dev pkg-config \
    protobuf-compiler curl git libssl-dev \
    cmake


RUN git clone https://github.com/chainwayxyz/citrea.git --single-branch --branch erce/filter-changes /app

RUN rustup override set 1.88.0
RUN rustup component add rustfmt
# Build the project
RUN SKIP_GUEST_BUILD=1 cargo build --release --bin citrea

# our final base
FROM rust:1.88-slim-trixie

# copy the build artifact from the build stage
# TODO: think about paths

COPY --from=build /app/target/release/citrea .
COPY --from=build /app/resources .

EXPOSE 12345

ENTRYPOINT ["sh", "-c", "./citrea --dev --da-layer mock --rollup-config-path ./configs/mock/sequencer_rollup_config.toml --sequencer ./configs/mock/sequencer_config.toml --genesis-paths ./genesis/mock/"]
