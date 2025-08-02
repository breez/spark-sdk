ARG VERSION=428a71c0f13175a3b1ffff0f5f31eba042d7d537
ARG REPOSITORY=https://github.com/buildonspark/spark.git

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


FROM arigaio/atlas AS final

COPY --from=downloader /source/spark/so/ent/migrate/migrations/ /migrations/
