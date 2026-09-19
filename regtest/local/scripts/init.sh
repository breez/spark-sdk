#!/bin/sh
# Writes the certificates and configuration the services start from, into
# LOCAL_DIR. Safe to run again: the CA is kept, so a wallet configured against
# it stays valid, and the server certificate is only reissued when the host
# names it covers change.
#
# OPERATOR_ADDRESSES: host:port per operator, as operators and the SSP reach it.
# OPERATOR_PUBLIC_PORTS: the port per operator on PUBLIC_HOST, for wallets.
# SSP_ADDRESS: host:port the SSP's API is reached at from inside the
# environment, PUBLIC_HOST and SSP_PUBLIC_PORT by default.
# TLS_EXTRA_HOSTS: more names or IPs a wallet may reach the operators by.
# OUT_DIR: where the wallet-facing config is written, LOCAL_DIR by default.
# LDK_SSP_SEED_DIR, LDK_ALICE_SEED_DIR: when set, the two Lightning nodes'
# storage, seeded with their keys, and their configs are written too.
set -eu

. "$(dirname "$0")/lib.sh"

: "${LOCAL_DIR:?}" "${OPERATOR_ADDRESSES:?}" "${OPERATOR_PUBLIC_PORTS:?}"
: "${OPERATOR_IDENTITY_PUBLIC_KEYS:?}" "${SSP_IDENTITY_PUBLIC_KEY:?}"
: "${PUBLIC_HOST:=127.0.0.1}" "${SSP_PUBLIC_PORT:=59049}" "${TLS_EXTRA_HOSTS:=}"
: "${SSP_ADDRESS:=$PUBLIC_HOST:$SSP_PUBLIC_PORT}"

certs="$LOCAL_DIR/certs"
mkdir -p "$certs"

if [ ! -f "$certs/ca.crt" ]; then
  log "creating the operators' certificate authority"
  cat >"$certs/ca.cnf" <<EOF
[req]
distinguished_name = dn
prompt = no
x509_extensions = ca
[dn]
CN = Spark local regtest CA
[ca]
basicConstraints = critical,CA:TRUE
keyUsage = critical,keyCertSign,cRLSign
subjectKeyIdentifier = hash
EOF
  openssl req -x509 -new -config "$certs/ca.cnf" -newkey ec \
    -pkeyopt ec_paramgen_curve:prime256v1 -nodes -days 3650 \
    -keyout "$certs/ca.key" -out "$certs/ca.crt" 2>/dev/null
  rm -f "$certs/server.crt"
fi

# Wallets on an Android emulator reach the host as 10.0.2.2.
hosts="localhost 127.0.0.1 10.0.2.2 $PUBLIC_HOST $TLS_EXTRA_HOSTS"
for address in $OPERATOR_ADDRESSES; do
  hosts="$hosts ${address%:*}"
done
hosts=$(printf '%s\n' $hosts | sort -u | tr '\n' ' ')

if [ ! -f "$certs/server.crt" ] || [ "$(cat "$certs/hosts" 2>/dev/null)" != "$hosts" ]; then
  log "issuing the operators' certificate for $hosts"
  san=""
  for host in $hosts; do
    case "$host" in
      *[!0-9.]*) entry="DNS:$host" ;;
      *) entry="IP:$host" ;;
    esac
    san="${san:+$san,}$entry"
  done
  cat >"$certs/server.ext" <<EOF
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature
extendedKeyUsage = serverAuth,clientAuth
subjectAltName = $san
EOF
  openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
    -subj "/CN=spark-operator" -keyout "$certs/server.key" \
    -out "$certs/server.csr" 2>/dev/null
  openssl x509 -req -in "$certs/server.csr" -CA "$certs/ca.crt" \
    -CAkey "$certs/ca.key" -CAcreateserial -days 3650 \
    -extfile "$certs/server.ext" -out "$certs/server.crt" 2>/dev/null
  # The operators run as their own user.
  chmod 644 "$certs/server.key"
  echo "$hosts" >"$certs/hosts"
fi

ca_pem=$(cat "$certs/ca.crt")

# Positional lists, walked in step: the operator index, its address, port and key.
set -- $OPERATOR_ADDRESSES
index=0
operators_json="[]"
sspd_operators=""
sdk_operators="[]"
internal_operators="[]"
for public_port in $OPERATOR_PUBLIC_PORTS; do
  address=$1
  shift
  key=$(echo "$OPERATOR_IDENTITY_PUBLIC_KEYS" | cut -d' ' -f$((index + 1)))
  identifier=$(printf '%064x' $((index + 1)))
  operators_json=$(printf '%s\n' "$operators_json" | jq -c \
    --argjson id "$index" --arg address "$address" \
    --arg external "$PUBLIC_HOST:$public_port" --arg key "$key" \
    --arg cert "$certs/ca.crt" \
    '. + [{id: $id, address: $address, external_address: $external,
           identity_public_key: $key, cert_path: $cert}]')
  sspd_operators="$sspd_operators$(printf '[[operators]]\nid = %s\nidentifier = "%s"\naddress = "https://%s"\nidentity_public_key = "%s"\nca_cert_pem = """\n%s\n"""\n\n' \
    "$index" "$identifier" "$address" "$key" "$ca_pem")
"
  sdk_operators=$(printf '%s\n' "$sdk_operators" | jq -c \
    --argjson id "$index" --arg identifier "$identifier" \
    --arg address "https://$PUBLIC_HOST:$public_port" --arg key "$key" \
    --arg ca "$ca_pem" \
    '. + [{id: $id, identifier: $identifier, address: $address,
           identity_public_key: $key, ca_cert_pem: $ca}]')
  internal_operators=$(printf '%s\n' "$internal_operators" | jq -c \
    --argjson id "$index" --arg identifier "$identifier" \
    --arg address "https://$address" --arg key "$key" \
    --arg ca "$ca_pem" \
    '. + [{id: $id, identifier: $identifier, address: $address,
           identity_public_key: $key, ca_cert_pem: $ca}]')
  index=$((index + 1))
done

printf '%s\n' "$operators_json" | jq . >"$LOCAL_DIR/operators.json"
printf '%s' "$sspd_operators" >"$LOCAL_DIR/sspd.toml"

# Matches the SDK's SparkConfig, which parse_spark_config takes as it is.
spark_config() {
  jq -n --argjson operators "$1" --arg ssp_url "$2" \
    --arg ssp_key "$SSP_IDENTITY_PUBLIC_KEY" \
    '{
      coordinator_identifier: $operators[0].identifier,
      threshold: 2,
      signing_operators: $operators,
      ssp_config: {
        base_url: $ssp_url,
        identity_public_key: $ssp_key,
        schema_endpoint: "graphql/spark/rc"
      },
      expected_withdraw_bond_sats: 10000,
      expected_withdraw_relative_block_locktime: 1000
    }'
}

mkdir -p "${OUT_DIR:=$LOCAL_DIR}"
spark_config "$sdk_operators" "http://$PUBLIC_HOST:$SSP_PUBLIC_PORT" \
  >"$OUT_DIR/spark-config.json"
# For the environment's own services, which reach each other by their names.
spark_config "$internal_operators" "http://$SSP_ADDRESS" \
  >"$LOCAL_DIR/spark-config-internal.json"

# A Lightning node's config, and the keys it takes as its own. `dir` is its
# storage as the node sees it, `seed_dir` as this script does, which differ when
# each runs in a container of its own.
ldk_node() {
  name=$1 dir=$2 seed_dir=$3 mnemonic=$4 api_key=$5 grpc_address=$6 listen_port=$7 tls_host=$8
  cat >"$LOCAL_DIR/ldk-$name.toml" <<EOF
[node]
network = "regtest"
listening_addresses = ["0.0.0.0:$listen_port"]
grpc_service_address = "$grpc_address"
alias = "spark-local-$name"

[storage.disk]
dir_path = "$dir"

[log]
level = "Info"

[bitcoind]
rpc_address = "$LDK_BITCOIND_RPC_ADDRESS"
rpc_user = "$BITCOIND_RPC_USER"
rpc_password = "$BITCOIND_RPC_PASSWORD"

[tls]
hosts = ["$tls_host"]
EOF
  mkdir -p "$seed_dir/regtest"
  if [ ! -f "$seed_dir/keys_mnemonic" ]; then
    echo "$mnemonic" >"$seed_dir/keys_mnemonic"
    chmod 600 "$seed_dir/keys_mnemonic"
  fi
  if [ ! -f "$seed_dir/regtest/api_key" ]; then
    # ldk-server stores the key as raw bytes and authenticates with their hex.
    echo "$api_key" | xxd -r -p >"$seed_dir/regtest/api_key"
    chmod 400 "$seed_dir/regtest/api_key"
  fi
}

if [ -n "${LDK_SSP_SEED_DIR:-}" ]; then
  : "${LDK_BITCOIND_RPC_ADDRESS:?}" "${LDK_SERVER_MNEMONIC:?}" "${LDK_SERVER_API_KEY:?}"
  : "${ALICE_MNEMONIC:?}" "${ALICE_API_KEY:?}" "${LDK_ALICE_SEED_DIR:?}"
  : "${LDK_SSP_DIR:=$LDK_SSP_SEED_DIR}" "${LDK_ALICE_DIR:=$LDK_ALICE_SEED_DIR}"
  : "${LDK_GRPC_ADDRESS:=127.0.0.1:3536}" "${LDK_ALICE_GRPC_ADDRESS:=127.0.0.1:3537}"
  : "${LDK_LISTEN_PORT:=9735}" "${LDK_ALICE_LISTEN_PORT:=9736}"
  : "${LDK_TLS_HOST:=localhost}" "${LDK_ALICE_TLS_HOST:=localhost}"
  ldk_node ssp "$LDK_SSP_DIR" "$LDK_SSP_SEED_DIR" "$LDK_SERVER_MNEMONIC" "$LDK_SERVER_API_KEY" \
    "$LDK_GRPC_ADDRESS" "$LDK_LISTEN_PORT" "$LDK_TLS_HOST"
  ldk_node alice "$LDK_ALICE_DIR" "$LDK_ALICE_SEED_DIR" "$ALICE_MNEMONIC" "$ALICE_API_KEY" \
    "$LDK_ALICE_GRPC_ADDRESS" "$LDK_ALICE_LISTEN_PORT" "$LDK_ALICE_TLS_HOST"
fi

log "configuration written to $LOCAL_DIR"
