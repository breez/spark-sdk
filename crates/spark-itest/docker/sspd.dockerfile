# Built from the working tree: the build context is the repository root.
FROM rust:1.88 AS builder

RUN apt-get update -qq && \
    apt-get install -qq -y --no-install-recommends \
        protobuf-compiler \
        libprotobuf-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .
RUN cargo build --release --locked -p sspd -p ssp-cli

FROM debian:bookworm-slim

RUN apt-get update -qq && \
    apt-get install -qq -y --no-install-recommends \
        ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sspd /usr/local/bin/sspd
COPY --from=builder /app/target/release/ssp-cli /usr/local/bin/ssp-cli

# The GraphQL API and the internal gRPC API.
EXPOSE 8080 59050

ENTRYPOINT ["sspd"]
