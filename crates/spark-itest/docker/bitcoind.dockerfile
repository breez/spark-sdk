FROM debian:bookworm-slim

ARG BITCOIN_VERSION=29.0
ARG TARGETARCH

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl gnupg && \
    rm -rf /var/lib/apt/lists/*

# Fetch bitcoind, verifying the release tarball against the official
# SHA256SUMS. Primary source is bitcoincore.org; the GitHub release mirror is
# used as a fallback when the primary is unreachable.
RUN case "${TARGETARCH}" in \
      amd64)  ARCH=x86_64-linux-gnu ;; \
      arm64)  ARCH=aarch64-linux-gnu ;; \
      *)      echo "Unsupported arch: ${TARGETARCH}" && exit 1 ;; \
    esac && \
    TARBALL="bitcoin-${BITCOIN_VERSION}-${ARCH}.tar.gz" && \
    PRIMARY="https://bitcoincore.org/bin/bitcoin-core-${BITCOIN_VERSION}" && \
    FALLBACK="https://github.com/bitcoin/bitcoin/releases/download/v${BITCOIN_VERSION}" && \
    fetch() { \
      curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors "$1" -o "$3" || \
      curl -fsSL --retry 5 --retry-delay 3 --retry-all-errors "$2" -o "$3"; \
    } && \
    fetch "${PRIMARY}/SHA256SUMS" "${FALLBACK}/SHA256SUMS" /tmp/SHA256SUMS && \
    fetch "${PRIMARY}/${TARBALL}" "${FALLBACK}/${TARBALL}" "/tmp/${TARBALL}" && \
    cd /tmp && \
    grep " ${TARBALL}\$" SHA256SUMS | sha256sum -c - && \
    tar -xzf "${TARBALL}" && \
    install -m 0755 "/tmp/bitcoin-${BITCOIN_VERSION}/bin/bitcoind" /usr/local/bin/ && \
    install -m 0755 "/tmp/bitcoin-${BITCOIN_VERSION}/bin/bitcoin-cli" /usr/local/bin/ && \
    rm -rf /tmp/bitcoin* /tmp/SHA256SUMS && \
    bitcoind --version | head -1

RUN mkdir -p /root/.bitcoin

EXPOSE 8332 28332

ENTRYPOINT ["bitcoind"]
