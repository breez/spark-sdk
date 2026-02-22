ARG VERSION=2776c7533d889c153a97ebe512ae4cf9581f1e9e
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


FROM arigaio/atlas:0.36.0 AS final

COPY --from=downloader /source/spark/so/ent/migrate/migrations/ /migrations/
