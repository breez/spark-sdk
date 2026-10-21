# Pinned to the same commit as spark-so-private.dockerfile: these define the
# schema the operator built from that commit expects.
ARG VERSION=bb5711ef810849fde471923cc35fc618041020db

FROM ghcr.io/breez/spark-operator:${VERSION} AS operator


FROM arigaio/atlas:0.36.0 AS final

COPY --from=operator /opt/spark/migrations/ /migrations/
