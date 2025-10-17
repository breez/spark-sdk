# $USER name to be used in the `final` image
ARG USER=so
ARG VERSION=003289e7c35982f589ea0e99e973abab1f2c3af3
ARG REPOSITORY=https://github.com/buildonspark/spark.git

FROM debian:bookworm-20250721-slim AS downloader

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


FROM golang:1.25.1-bookworm AS operator-builder

# Install required dependencies for building
RUN apt-get update -qq && \
    apt-get install -y --no-install-recommends \
    libzmq3-dev

WORKDIR /app/spark
COPY --from=downloader /source/spark/go.mod /source/spark/go.sum ./
RUN go mod download

WORKDIR /app
COPY --from=downloader /source/ ./

WORKDIR /app/spark
RUN go build ./bin/operator


FROM rust:1.89.0-bookworm AS signer-builder

RUN apt-get update -qq && \
    apt-get install -qq -y --no-install-recommends \
        protobuf-compiler \
        libprotobuf-dev

WORKDIR /app
COPY --from=downloader /source/signer/ ./
WORKDIR /app/spark-frost-signer
RUN cargo install --path .


FROM debian:bookworm-20250721-slim AS final

ARG USER

LABEL maintainer="Jesse de Wit (@JssDWt)"

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

# Operator grpc port
EXPOSE 8535

COPY so.config.yaml /config/
COPY --from=operator-builder /app/spark/operator /bin/
COPY --from=signer-builder /usr/local/cargo/bin/spark-frost-signer /bin/

ENTRYPOINT ["/entrypoint.sh"]
