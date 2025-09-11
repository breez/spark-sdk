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
RUN cargo build --release --bin lnurl


FROM debian:bookworm-slim AS final

RUN apt-get update -qq && \
    apt-get install -qq -y --no-install-recommends \
        openssl && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/lnurl /usr/local/bin

ENTRYPOINT ["/usr/local/bin/lnurl"]
