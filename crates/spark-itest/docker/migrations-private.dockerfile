# Pinned to the same commit as spark-so-private.dockerfile: these define the
# schema the operator built from that commit expects.
ARG VERSION=35b57a32e14a5a0f8567967f47a742c50ccf0953

FROM ghcr.io/breez/spark-operator:${VERSION} AS operator


FROM arigaio/atlas:0.36.0 AS final

COPY --from=operator /opt/spark/migrations/ /migrations/
