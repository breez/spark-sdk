FROM rust:slim-bookworm AS builder

RUN apt-get update -qq && \
    apt-get install -qq -y --no-install-recommends \
        libprotobuf-dev \
        libssl-dev \
        pkg-config \
        protobuf-compiler
        
# Note these files are relative to the workspace root.
RUN mkdir -p /app/crates/breez-sdk/lnurl/
COPY . /app
WORKDIR /app/crates/breez-sdk/lnurl
ARG CARGO_FEATURES=""
RUN if [ -z "$CARGO_FEATURES" ]; then \
        cargo build --release --bin lnurl; \
    else \
        cargo build --release --bin lnurl --features "$CARGO_FEATURES"; \
    fi


FROM debian:bookworm-slim AS final

RUN apt-get update -qq && \
    apt-get install -qq -y --no-install-recommends \
        ca-certificates \
        openssl && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/crates/breez-sdk/lnurl/target/release/lnurl /usr/local/bin

ENTRYPOINT ["/usr/local/bin/lnurl"]
