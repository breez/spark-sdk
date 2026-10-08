# The operator the integration tests and regtest/local run: Lightspark's current
# code, which is not public, from Breez's private mirror
# (github.com/breez/spark-breez). The mirror builds and publishes it as a
# private image, see its breez/README.md. spark-so.dockerfile builds the public
# code, for when Lightspark publishes it again.
ARG USER=so
# A commit on the mirror's breez branch. migrations-private.dockerfile must pin
# the same commit.
ARG VERSION=736e059a1f65d1af18fcde61416aa50ae29de2b5

FROM ghcr.io/breez/spark-operator:${VERSION} AS operator


FROM debian:bookworm-20250721-slim AS final

ARG USER

RUN adduser --disabled-password \
            --home "/data" \
            --gecos "" \
            "$USER"

RUN apt-get update -qq && \
    apt-get install -qq -y --no-install-recommends \
        postgresql-client \
        libzmq3-dev \
        sed \
        openssl && \
    rm -rf /var/lib/apt/lists/*

COPY entrypoint.sh /
RUN chmod +x /entrypoint.sh

RUN mkdir -p /data/ && chown -R $USER:$USER /data/
RUN mkdir -p /config/ && chown -R $USER:$USER /config/

USER $USER

# This operator serves gRPC, grpc-web and the operator-to-operator services on
# ports of their own. Clients use gRPC on 8535, as with the public operator.
ENV OPERATOR_LISTENER_FLAGS="-grpc-port 8535 -http-port 8536 -internal-grpc-port 8537"
EXPOSE 8535

COPY so.config.yaml /config/
COPY --from=operator /usr/local/bin/spark-operator /bin/operator
COPY --from=operator /usr/local/bin/spark-frost-signer /bin/

ENTRYPOINT ["/entrypoint.sh"]
