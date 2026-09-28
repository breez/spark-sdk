# Pinned to the same commit as spark-so.dockerfile: these define the schema the
# operator built from that commit expects.
ARG VERSION=42afde75a5197ea8ffffbd29d3a5f5425823f4c4
ARG REPOSITORY=https://github.com/breez/spark.git

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
