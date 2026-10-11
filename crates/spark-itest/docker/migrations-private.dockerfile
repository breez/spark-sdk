# Pinned to the same commit as spark-so-private.dockerfile: these define the
# schema the operator built from that commit expects.
ARG VERSION=736e059a1f65d1af18fcde61416aa50ae29de2b5

FROM ghcr.io/breez/spark-operator:${VERSION} AS operator


FROM arigaio/atlas:0.36.0 AS final

COPY --from=operator /opt/spark/migrations/ /migrations/
