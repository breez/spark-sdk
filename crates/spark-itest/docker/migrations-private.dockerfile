# Pinned to the same commit as spark-so-private.dockerfile: these define the
# schema the operator built from that commit expects.
ARG VERSION=988deadddfbe0e791c168e137cbaf39d36394962

FROM ghcr.io/breez/spark-operator:${VERSION} AS operator


FROM arigaio/atlas:0.36.0 AS final

COPY --from=operator /opt/spark/migrations/ /migrations/
