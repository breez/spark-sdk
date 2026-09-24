# The docker-compose environment as native processes under process-compose.
# State lives in SPARK_LOCAL_DIR, ./.spark-local by default, and every port can
# be set in the environment.
{ pkgs, packages, mempool }:

let
  inherit (pkgs) lib;

  scripts = ../scripts;
  soConfig = ../../../crates/spark-itest/docker/so.config.yaml;

  dkgMinAvailableKeys = 12000;
  dkgBatchSize = 2000;

  ports = {
    BITCOIND_RPC_PORT = 18443;
    BITCOIND_P2P_PORT = 18444;
    BITCOIND_ZMQ_PORT = 28332;
    OPERATOR_0_PORT = 8535;
    OPERATOR_1_PORT = 8536;
    OPERATOR_2_PORT = 8537;
    SSP_PORT = 59049;
    SSP_INTERNAL_PORT = 59050;
    LNURL_PORT = 8080;
    DATA_SYNC_PORT = 8081;
    DATA_SYNC_WEB_PORT = 8082;
    LDK_P2P_PORT = 9735;
    LDK_GRPC_PORT = 3536;
    LDK_ALICE_P2P_PORT = 9736;
    LDK_ALICE_GRPC_PORT = 3537;
    POSTGRES_PORT = 25432;
    ELECTRS_PORT = 3002;
    ELECTRS_ELECTRUM_PORT = 60401;
    ELECTRS_MONITORING_PORT = 24224;
    MEMPOOL_PORT = 8090;
    MEMPOOL_API_PORT = 8999;
  };

  tools = [
    pkgs.bash
    pkgs.coreutils
    pkgs.gnused
    pkgs.gnugrep
    pkgs.gawk
    pkgs.curl
    pkgs.jq
    pkgs.openssl
    pkgs.unixtools.xxd
    pkgs.bitcoind
    pkgs.postgresql_16
    pkgs.atlas
    pkgs.nginx
    pkgs.process-compose
    packages.spark-operator
    packages.spark-frost-signer
    packages.ldk-server
    packages.sspd
    packages.lnurl
    packages.data-sync
    packages.mempool-electrs
    mempool.backend
  ];

  bitcoinCli = ''bitcoin-cli -regtest -rpcport="$BITCOIND_RPC_PORT" -rpcuser="$BITCOIND_RPC_USER" -rpcpassword="$BITCOIND_RPC_PASSWORD"'';
  psql = ''psql -h 127.0.0.1 -p "$POSTGRES_PORT" -U postgres'';

  # A shell command from its words: process-compose mangles line continuations.
  words = lib.concatStringsSep " ";

  # Commands are shell scripts, left for the shell to expand.
  process = attrs: {
    disable_env_expansion = true;
    availability.restart = "on_failure";
    shutdown.signal = 15;
  } // attrs;

  oneShot = attrs: attrs // { availability.restart = "exit_on_failure"; };

  healthy = names: lib.genAttrs names (_: { condition = "process_healthy"; });
  completed = names: lib.genAttrs names (_: { condition = "process_completed_successfully"; });
  started = names: lib.genAttrs names (_: { condition = "process_started"; });

  probe = command: {
    exec.command = command;
    period_seconds = 2;
    failure_threshold = 900;
  };

  operator = index: process {
    command = words [
      "SO_CONFIG=${soConfig}"
      "SPARK_OPERATOR_INDEX=${toString index}"
      "SPARK_OPERATOR_KEY=${lib.concatStrings (lib.replicate 32 "0${toString (index + 1)}")}"
      ''OPERATOR_PORT="$OPERATOR_${toString index}_PORT"''
      "DKG_MIN_AVAILABLE_KEYS=${toString dkgMinAvailableKeys}"
      "DKG_BATCH_SIZE=${toString dkgBatchSize}"
      "${./operator.sh}"
    ];
    depends_on = healthy [ "bitcoind" ] // completed [ "init" "migrations" ];
    readiness_probe = probe (words [
      "POSTGRES_HOST=127.0.0.1"
      "POSTGRES_USER=postgres"
      "POSTGRES_PASSWORD=postgres"
      "SPARK_OPERATOR_INDEX=${toString index}"
      ''OPERATOR_ADDRESS="127.0.0.1:$OPERATOR_${toString index}_PORT"''
      "DKG_MIN_AVAILABLE_KEYS=${toString dkgMinAvailableKeys}"
      "${scripts}/operator-ready.sh"
    ]);
  };

  nginxConfig = pkgs.writeText "mempool-nginx.conf" ''
    daemon off;
    pid nginx.pid;
    error_log stderr;
    events {}
    http {
      include ${pkgs.nginx}/conf/mime.types;
      access_log off;
      client_body_temp_path tmp/body;
      proxy_temp_path tmp/proxy;
      fastcgi_temp_path tmp/fastcgi;
      uwsgi_temp_path tmp/uwsgi;
      scgi_temp_path tmp/scgi;
      server {
        listen @BIND_ADDRESS@:@MEMPOOL_PORT@;
        location /api/v1/ws {
          proxy_pass http://127.0.0.1:@MEMPOOL_API_PORT@;
          proxy_http_version 1.1;
          proxy_set_header Upgrade $http_upgrade;
          proxy_set_header Connection "upgrade";
        }
        location /api/v1/ {
          proxy_pass http://127.0.0.1:@MEMPOOL_API_PORT@/api/v1/;
        }
        location /api/ {
          proxy_pass http://127.0.0.1:@ELECTRS_PORT@/;
        }
        location /resources/ {
          root ${mempool.frontend};
        }
        location / {
          root ${mempool.frontend}/en-US;
          try_files $uri $uri/ /index.html;
        }
      }
    }
  '';

  # mempool-config.sh fills these in, unquoting the ports.
  mempoolConfig = {
    MEMPOOL = {
      NETWORK = "regtest";
      BACKEND = "esplora";
      HTTP_PORT = "@MEMPOOL_API_PORT@";
      CACHE_DIR = "@CACHE_DIR@";
    };
    CORE_RPC = {
      HOST = "127.0.0.1";
      PORT = "@BITCOIND_RPC_PORT@";
      USERNAME = "@BITCOIND_RPC_USER@";
      PASSWORD = "@BITCOIND_RPC_PASSWORD@";
    };
    ESPLORA.REST_API_URL = "http://127.0.0.1:@ELECTRS_PORT@";
    # With a database, the first start fetches the mining pool list from GitHub
    # and exits when it cannot.
    DATABASE.ENABLED = false;
    STATISTICS.ENABLED = false;
    FIAT_PRICE.ENABLED = false;
  };

  config = {
    version = "0.5";
    log_level = "info";
    processes = {
      init = oneShot (process {
        # The operators parse a peer's address as a URL, which an IP with a port
        # fails.
        command = words [
          ''OPERATOR_ADDRESSES="localhost:$OPERATOR_0_PORT localhost:$OPERATOR_1_PORT localhost:$OPERATOR_2_PORT"''
          ''OPERATOR_PUBLIC_PORTS="$OPERATOR_0_PORT $OPERATOR_1_PORT $OPERATOR_2_PORT"''
          ''SSP_PUBLIC_PORT="$SSP_PORT"''
          ''SSP_ADDRESS="127.0.0.1:$SSP_PORT"''
          ''LDK_BITCOIND_RPC_ADDRESS="127.0.0.1:$BITCOIND_RPC_PORT"''
          ''LDK_SSP_SEED_DIR="$SPARK_LOCAL_DIR/ldk"''
          ''LDK_GRPC_ADDRESS="127.0.0.1:$LDK_GRPC_PORT"''
          ''LDK_LISTEN_PORT="$LDK_P2P_PORT"''
          ''LDK_ALICE_SEED_DIR="$SPARK_LOCAL_DIR/ldk-alice"''
          ''LDK_ALICE_GRPC_ADDRESS="127.0.0.1:$LDK_ALICE_GRPC_PORT"''
          ''LDK_ALICE_LISTEN_PORT="$LDK_ALICE_P2P_PORT"''
          "${scripts}/init.sh"
        ];
      });

      bitcoind = process {
        command = ''
          mkdir -p "$SPARK_LOCAL_DIR/bitcoind"
          ${words [
            "exec bitcoind -regtest"
            ''-datadir="$SPARK_LOCAL_DIR/bitcoind"''
            "-printtoconsole -server -txindex -addresstype=bech32 -fallbackfee=0.00000253"
            ''-bind=127.0.0.1 -port="$BITCOIND_P2P_PORT" -rpcport="$BITCOIND_RPC_PORT"''
            ''-rpcuser="$BITCOIND_RPC_USER" -rpcpassword="$BITCOIND_RPC_PASSWORD"''
            "-rpcbind=127.0.0.1 -rpcallowip=127.0.0.1"
            ''-zmqpubrawblock="tcp://127.0.0.1:$BITCOIND_ZMQ_PORT"''
          ]}
        '';
        readiness_probe = probe "${bitcoinCli} getblockchaininfo";
      };

      miner = process {
        command = "${scripts}/miner.sh";
        depends_on = healthy [ "bitcoind" ];
      };

      postgres = process {
        command = ''
          data="$SPARK_LOCAL_DIR/postgres"
          if [ ! -f "$data/PG_VERSION" ]; then
            initdb -D "$data" -U postgres --auth=trust >/dev/null
          fi
          ${words [
            ''exec postgres -D "$data" -p "$POSTGRES_PORT"''
            "-c listen_addresses=127.0.0.1 -c unix_socket_directories="
            "-c max_connections=300"
          ]}
        '';
        readiness_probe = probe ''pg_isready -h 127.0.0.1 -p "$POSTGRES_PORT" -U postgres'';
      };

      migrations = oneShot (process {
        command = ''
          set -e
          for db in sparkoperator_0 sparkoperator_1 sparkoperator_2 sspd lnurl; do
            ${psql} -tAc "SELECT 1 FROM pg_database WHERE datname = '$db'" | grep -q 1 ||
              ${psql} -c "CREATE DATABASE $db"
          done
          for i in 0 1 2; do
            ${words [
              ''atlas migrate apply --dir "file://${packages.spark-migrations}"''
              ''--url "postgres://postgres:postgres@127.0.0.1:$POSTGRES_PORT/sparkoperator_$i?sslmode=disable"''
            ]}
          done
        '';
        depends_on = healthy [ "postgres" ];
      });

      spark-so-0 = operator 0;
      spark-so-1 = operator 1;
      spark-so-2 = operator 2;

      ldk-server = process {
        command = ''exec ldk-server "$LOCAL_DIR/ldk-ssp.toml"'';
        depends_on = healthy [ "bitcoind" ] // completed [ "init" ];
        readiness_probe = probe ''test -f "$SPARK_LOCAL_DIR/ldk/tls.crt"'';
      };

      # The node the SSP's holds a channel with, so a Lightning payment can
      # leave and enter the environment.
      ldk-alice = process {
        command = ''exec ldk-server "$LOCAL_DIR/ldk-alice.toml"'';
        depends_on = healthy [ "bitcoind" ] // completed [ "init" ];
        readiness_probe = probe ''test -f "$SPARK_LOCAL_DIR/ldk-alice/tls.crt"'';
      };

      lightning = oneShot (process {
        command = words [
          ''SSP_NODE_ADDRESS="127.0.0.1:$LDK_P2P_PORT"''
          ''SSP_BASE_URL="127.0.0.1:$LDK_GRPC_PORT"''
          ''SSP_TLS_CERT="$SPARK_LOCAL_DIR/ldk/tls.crt"''
          "${scripts}/lightning.sh"
        ];
        depends_on = healthy [ "ldk-server" "ldk-alice" ] // started [ "miner" ];
      });

      sspd = process {
        command = words [
          ''SSPD_DB_URL="postgres://postgres:postgres@127.0.0.1:$POSTGRES_PORT/sspd"''
          ''SSPD_BITCOIND_RPC_PASSWORD="$BITCOIND_RPC_PASSWORD"''
          ''SSPD_WALLET_SEED="$SSP_WALLET_SEED"''
          ''SSPD_LDK_SERVER_API_KEY="$LDK_SERVER_API_KEY"''
          ''SSPD_LDK_SERVER_INVOICE_SIGNING_KEY="$LDK_SERVER_NODE_SECRET_KEY"''
          ''exec sspd --config="$LOCAL_DIR/sspd.toml"''
          ''--address="$BIND_ADDRESS:$SSP_PORT"''
          ''--internal-address="127.0.0.1:$SSP_INTERNAL_PORT"''
          "--network=regtest --auto-migrate --chain-poll-interval-seconds=1"
          ''--bitcoind-rpc-address="http://127.0.0.1:$BITCOIND_RPC_PORT"''
          ''--bitcoind-rpc-user="$BITCOIND_RPC_USER"''
          ''--leaves-per-denomination="$LEAVES_PER_DENOMINATION"''
          ''--max-denomination-power="$MAX_DENOMINATION_POWER"''
          ''--ldk-server-url="127.0.0.1:$LDK_GRPC_PORT"''
          ''--ldk-server-cert-path="$SPARK_LOCAL_DIR/ldk/tls.crt"''
        ];
        depends_on = healthy [ "spark-so-0" "spark-so-1" "spark-so-2" "ldk-server" ];
      };

      # Serves the lightning addresses wallets register with it, taking their
      # invoices from the SSP.
      lnurl = process {
        command = words [
          "BREEZ_LNURL_NETWORK=regtest"
          ''BREEZ_LNURL_SPARK_CONFIG="$LOCAL_DIR/spark-config-internal.json"''
          ''BREEZ_LNURL_DB_URL="postgres://postgres:postgres@127.0.0.1:$POSTGRES_PORT/lnurl"''
          "BREEZ_LNURL_AUTO_MIGRATE=true"
          "exec lnurl --scheme=http"
          ''--address="$BIND_ADDRESS:$LNURL_PORT"''
          # A wallet signs the address it reaches the server by, which is
          # PUBLIC_HOST from another device and loopback on this one.
          ''--domains="$PUBLIC_HOST:$LNURL_PORT,127.0.0.1:$LNURL_PORT,localhost:$LNURL_PORT"''
          ''--ssp-auth-seed="$LNURL_AUTH_SEED"''
          # A local environment is registered against as often as its test needs.
          "--max-registrations-per-day=0"
        ];
        depends_on = started [ "sspd" ] // completed [ "init" ];
        readiness_probe = probe ''curl -sf -o /dev/null "http://127.0.0.1:$LNURL_PORT/health"'';
      };

      # Keeps a wallet's data in step across the devices it runs on.
      data-sync = process {
        command = ''
          dir="$SPARK_LOCAL_DIR/data-sync"
          mkdir -p "$dir"
          ${words [
            ''GRPC_LISTEN_ADDRESS="$BIND_ADDRESS:$DATA_SYNC_PORT"''
            ''GRPC_WEB_LISTEN_ADDRESS="$BIND_ADDRESS:$DATA_SYNC_WEB_PORT"''
            ''SQLITE_DIR_PATH="$dir" exec data-sync''
          ]}
        '';
        readiness_probe = probe ''curl -sf -o /dev/null "http://127.0.0.1:$DATA_SYNC_WEB_PORT/"'';
      };

      funder = process {
        command = "${scripts}/funder.sh";
        depends_on = started [ "sspd" ];
        readiness_probe = probe ''test -f "$LOCAL_DIR/ready"'';
      };

      electrs = process {
        command = words [
          ''exec electrs --network=regtest --daemon-rpc-addr="127.0.0.1:$BITCOIND_RPC_PORT"''
          ''--cookie="$BITCOIND_RPC_USER:$BITCOIND_RPC_PASSWORD" --jsonrpc-import''
          ''--db-dir="$SPARK_LOCAL_DIR/electrs" --http-addr="127.0.0.1:$ELECTRS_PORT"''
          ''--electrum-rpc-addr="127.0.0.1:$ELECTRS_ELECTRUM_PORT"''
          ''--monitoring-addr="127.0.0.1:$ELECTRS_MONITORING_PORT"''
        ];
        depends_on = healthy [ "bitcoind" ];
      };

      mempool-api = process {
        command = ''
          MEMPOOL_TEMPLATE=${pkgs.writeText "mempool-config.json" (builtins.toJSON mempoolConfig)} ${./mempool-config.sh}
          MEMPOOL_CONFIG_FILE="$SPARK_LOCAL_DIR/mempool/mempool-config.json" exec mempool-backend
        '';
        depends_on = started [ "electrs" ];
      };

      mempool = process {
        command = ''
          dir="$SPARK_LOCAL_DIR/nginx"
          mkdir -p "$dir/tmp"
          ${words [
            ''sed -e "s|@BIND_ADDRESS@|$BIND_ADDRESS|"''
            ''-e "s|@MEMPOOL_PORT@|$MEMPOOL_PORT|"''
            ''-e "s|@MEMPOOL_API_PORT@|$MEMPOOL_API_PORT|"''
            ''-e "s|@ELECTRS_PORT@|$ELECTRS_PORT|"''
            ''${nginxConfig} >"$dir/nginx.conf"''
          ]}
          exec nginx -e stderr -p "$dir" -c "$dir/nginx.conf"
        '';
        depends_on = started [ "mempool-api" ];
      };

      # Reports what the others are doing until a wallet can be served.
      ready = oneShot (process {
        command = words [
          ''SPARK_CONFIG_PATH="$LOCAL_DIR/spark-config.json"''
          ''OPERATOR_PUBLIC_PORTS="$OPERATOR_0_PORT $OPERATOR_1_PORT $OPERATOR_2_PORT"''
          "DKG_MIN_AVAILABLE_KEYS=${toString dkgMinAvailableKeys}"
          "POSTGRES_HOST=127.0.0.1 POSTGRES_USER=postgres POSTGRES_PASSWORD=postgres"
          ''BITCOIN_CLI="${bitcoinCli}"''
          ''SSP_CLI="ssp-cli --grpc-uri http://127.0.0.1:$SSP_INTERNAL_PORT"''
          ''SSP_NODE_CLI="ldk-server-cli --config $LOCAL_DIR/ldk-ssp.toml"''
          ''ALICE_CLI="ldk-server-cli --config $LOCAL_DIR/ldk-alice.toml"''
          "${scripts}/ready.sh"
        ];
        depends_on = healthy [ "postgres" ];
      });
    };
  };

  configFile = pkgs.writeText "process-compose.json" (builtins.toJSON config);

  exportDefault = name: value: ''export ${name}="''${${name}:-${toString value}}"'';
in
pkgs.writeShellApplication {
  name = "spark-local";
  runtimeInputs = tools;
  text = ''
    set -a
    # shellcheck disable=SC1091
    . ${../.env}
    set +a
    export SPARK_LOCAL_DIR="''${SPARK_LOCAL_DIR:-$PWD/.spark-local}"
    export LOCAL_DIR="$SPARK_LOCAL_DIR/local"
    export BIND_ADDRESS="''${BIND_ADDRESS:-127.0.0.1}"
    export PUBLIC_HOST="''${PUBLIC_HOST:-127.0.0.1}"
    export LEAVES_PER_DENOMINATION="''${LEAVES_PER_DENOMINATION:-8}"
    export MAX_DENOMINATION_POWER="''${MAX_DENOMINATION_POWER:-16}"
    export BLOCK_INTERVAL_SECONDS="''${BLOCK_INTERVAL_SECONDS:-5}"
    export PC_PORT_NUM="''${PC_PORT_NUM:-18080}"
    ${lib.concatStringsSep "\n    " (lib.mapAttrsToList exportDefault ports)}
    export BITCOIND_RPC_URL="http://127.0.0.1:$BITCOIND_RPC_PORT"
    export SSP_GRPC_URI="http://127.0.0.1:$SSP_INTERNAL_PORT"
    export CHAIN_API_URL="http://127.0.0.1:$MEMPOOL_PORT/api"
    export LNURL_URL="http://127.0.0.1:$LNURL_PORT"
    export DATA_SYNC_WEB_URL="http://127.0.0.1:$DATA_SYNC_WEB_PORT"

    command="''${1:-up}"
    [ $# -gt 0 ] && shift
    case "$command" in
      up)
        mkdir -p "$SPARK_LOCAL_DIR"
        exec process-compose up --config ${configFile} "$@"
        ;;
      down)
        exec process-compose down
        ;;
      reset)
        rm -rf "$SPARK_LOCAL_DIR"
        ;;
      config)
        cat "$LOCAL_DIR/spark-config.json"
        ;;
      fund)
        exec ${scripts}/fund.sh "$@"
        ;;
      mine)
        exec ${scripts}/mine.sh "$@"
        ;;
      *)
        echo "usage: spark-local [up|down|reset|config|fund <address> <sats>|mine <blocks>]" >&2
        exit 1
        ;;
    esac
  '';
}
