# Pinned to the same commit as spark-so-private.dockerfile: these define the
# schema the operator built from that commit expects.
ARG VERSION=729d4e711647d47e72f822754be1b0be10314e58

FROM ghcr.io/breez/spark-operator:${VERSION} AS operator


FROM arigaio/atlas:0.36.0 AS final

COPY --from=operator /opt/spark/migrations/ /migrations/
