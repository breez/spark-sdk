FROM debian:bookworm-slim

ARG BITCOIN_VERSION=29.0
ARG TARGETARCH

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl gnupg && \
    rm -rf /var/lib/apt/lists/*

# Map Docker arch to Bitcoin Core archive name
RUN case "${TARGETARCH}" in \
      amd64)  ARCH=x86_64-linux-gnu ;; \
      arm64)  ARCH=aarch64-linux-gnu ;; \
      *)      echo "Unsupported arch: ${TARGETARCH}" && exit 1 ;; \
    esac && \
    curl -fsSL "https://bitcoincore.org/bin/bitcoin-core-${BITCOIN_VERSION}/bitcoin-${BITCOIN_VERSION}-${ARCH}.tar.gz" \
      -o /tmp/bitcoin.tar.gz && \
    tar -xzf /tmp/bitcoin.tar.gz -C /tmp && \
    install -m 0755 /tmp/bitcoin-${BITCOIN_VERSION}/bin/bitcoind /usr/local/bin/ && \
    install -m 0755 /tmp/bitcoin-${BITCOIN_VERSION}/bin/bitcoin-cli /usr/local/bin/ && \
    rm -rf /tmp/bitcoin* && \
    bitcoind --version | head -1

RUN mkdir -p /root/.bitcoin

EXPOSE 8332 28332

ENTRYPOINT ["bitcoind"]
