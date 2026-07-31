FROM rust:slim-bookworm AS builder

RUN apt-get update -qq && \
    apt-get install -qq -y --no-install-recommends \
        libprotobuf-dev \
        libssl-dev \
        pkg-config \
        protobuf-compiler
        
# The build context is the workspace root.
COPY . /app
WORKDIR /app
ARG CARGO_FEATURES=""
RUN if [ -z "$CARGO_FEATURES" ]; then \
        cargo build --release -p lnurl; \
    else \
        cargo build --release -p lnurl --features "$CARGO_FEATURES"; \
    fi


FROM debian:bookworm-slim AS final

RUN apt-get update -qq && \
    apt-get install -qq -y --no-install-recommends \
        ca-certificates \
        openssl && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/lnurl /usr/local/bin

ENTRYPOINT ["/usr/local/bin/lnurl"]
