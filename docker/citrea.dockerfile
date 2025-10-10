FROM rust:1.88 AS build
WORKDIR /app


# Install system dependencies
RUN apt-get update && apt-get -y upgrade && \
    apt-get install -y libclang-dev pkg-config && \
    apt-get install protobuf-compiler -y && apt-get install -y curl git && \
    apt-get install cmake -y


RUN git clone https://github.com/chainwayxyz/citrea.git --single-branch --branch erce/filter-changes /app

RUN rustup override set 1.88.0
# Build the project
RUN SKIP_GUEST_BUILD=1 cargo build --release --bin citrea

# our final base
FROM rust:1.88

# copy the build artifact from the build stage
COPY --from=build /app/target/release/citrea .

EXPOSE 12345

ENTRYPOINT ["sh", "-c", "./target/release/citrea --dev --da-layer mock --rollup-config-path ./resources/configs/mock/sequencer_rollup_config.toml --sequencer ./resources/configs/mock/sequencer_config.toml --genesis-paths resources/genesis/mock/"]
