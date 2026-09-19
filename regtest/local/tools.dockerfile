# The scripts that set up and keep the environment running. `sspd` and
# `ldk_server` are those images, passed in as build contexts, for their CLIs.
FROM debian:bookworm-slim

RUN apt-get update -qq && \
    apt-get install -qq -y --no-install-recommends \
        ca-certificates \
        curl \
        jq \
        openssl \
        postgresql-client \
        xxd && \
    rm -rf /var/lib/apt/lists/*

COPY --from=sspd /usr/local/bin/ssp-cli /usr/local/bin/ssp-cli
COPY --from=ldk_server /usr/local/bin/ldk-server-cli /usr/local/bin/ldk-server-cli
COPY scripts/ /scripts/
