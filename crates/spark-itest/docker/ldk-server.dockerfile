# Pinned to a commit of the breez/ldk-server fork, for what the SSP needs and
# upstream lacks:
# - min_final_cltv_expiry_delta on Bolt11ReceiveForHash
# - the claimable amount on the PaymentClaimable event
# - the claim deadline and claimable amount on a stored payment
# - a PaymentSendingFailed error code
# - paying an invoice the node issued itself, which is how a Lightning payment
#   between two users of the same SSP settles
# - refusing to pay an invoice whose payment the node still tracks
# Part of this lives in ldk-node and rust-lightning, which the fork's Cargo patch
# takes from the breez forks' `ssp` branches.
ARG VERSION=88d40268025ca8fcb72ff118a8483e5958edcddc
ARG REPOSITORY=https://github.com/breez/ldk-server.git

FROM debian:bookworm-slim AS downloader

ARG VERSION
ARG REPOSITORY

WORKDIR /source/

RUN apt-get update -qq && \
    apt-get install -qq -y --no-install-recommends \
        ca-certificates \
        git

RUN git init && \
    git remote add origin "$REPOSITORY" && \
    git fetch --depth 1 origin "$VERSION" && \
    git checkout FETCH_HEAD

FROM rust:1.85 AS builder

WORKDIR /app
COPY --from=downloader /source/ .
RUN cargo build --release --locked -p ldk-server -p ldk-server-cli

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ldk-server /usr/local/bin/ldk-server
COPY --from=builder /app/target/release/ldk-server-cli /usr/local/bin/ldk-server-cli

ENV LDK_SERVER_NODE_GRPC_SERVICE_ADDRESS=0.0.0.0:3536

EXPOSE 9735 3536

ENTRYPOINT ["ldk-server"]
