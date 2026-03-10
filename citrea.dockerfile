FROM debian:trixie-slim

ARG CITREA_VERSION=v2.1.0
ARG TARGETARCH

WORKDIR /

# Install minimal runtime deps + curl for downloading
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Download prebuilt Citrea binary
RUN ARCH="$TARGETARCH" && \
    if [ -z "$ARCH" ]; then ARCH="$(dpkg --print-architecture)"; fi && \
    case "$ARCH" in \
        amd64|arm64) ;; \
        x86_64) ARCH=amd64 ;; \
        aarch64) ARCH=arm64 ;; \
        *) echo "Unsupported arch: $ARCH" >&2; exit 1 ;; \
    esac && \
    curl -fSL "https://github.com/chainwayxyz/citrea/releases/download/${CITREA_VERSION}/citrea-${CITREA_VERSION}-linux-${ARCH}" \
    -o /citrea && chmod +x /citrea

# Download mock configs and genesis
RUN mkdir -p /configs/mock /genesis/mock && \
    BASE="https://raw.githubusercontent.com/chainwayxyz/citrea/${CITREA_VERSION}/resources" && \
    for f in sequencer_rollup_config.toml sequencer_config.toml; do \
        curl -fSL "$BASE/configs/mock/$f" -o "/configs/mock/$f"; \
    done && \
    for f in accounts.json evm.json l2_block_rule_enforcer.json; do \
        curl -fSL "$BASE/genesis/mock/$f" -o "/genesis/mock/$f"; \
    done

EXPOSE 12345

ENTRYPOINT ["sh", "-c", "./citrea --dev --da-layer mock --rollup-config-path /configs/mock/sequencer_rollup_config.toml --sequencer /configs/mock/sequencer_config.toml --genesis-paths /genesis/mock/"]
