

#!/bin/bash

# Script to call a GraphQL endpoint with a query and variables
# This script can take 2-4 inputs: address, amount_sats, [username, password]
# Username and password can also be provided via environment variables:
# SPARK_FAUCET_USERNAME and SPARK_FAUCET_PASSWORD

# Check if the minimum required arguments are provided
if [ "$#" -lt 2 ]; then
    echo "Usage: $0 <address> <amount_sats> [username] [password]"
    echo "Example: $0 bcrt1abcdefgh12345 10000 myuser mypassword"
    echo ""
    echo "Alternatively, you can set these environment variables:"
    echo "SPARK_FAUCET_USERNAME - for the username"
    echo "SPARK_FAUCET_PASSWORD - for the password"
    exit 1
fi

# Assign input arguments to variables
ADDRESS="$1"
AMOUNT_SATS="$2"

# Check for username in arguments or environment variables
if [ "$#" -ge 3 ]; then
    # Username provided as argument
    USERNAME="$3"
else
    # Check environment variable
    if [ -n "$SPARK_FAUCET_USERNAME" ]; then
        USERNAME="$SPARK_FAUCET_USERNAME"
    else
        echo "Error: Username must be provided as the third argument or set as SPARK_FAUCET_USERNAME environment variable"
        exit 1
    fi
fi

# Check for password in arguments or environment variables
if [ "$#" -ge 4 ]; then
    # Password provided as argument
    PASSWORD="$4"
else
    # Check environment variable
    if [ -n "$SPARK_FAUCET_PASSWORD" ]; then
        PASSWORD="$SPARK_FAUCET_PASSWORD"
    else
        echo "Error: Password must be provided as the fourth argument or set as SPARK_FAUCET_PASSWORD environment variable"
        exit 1
    fi
fi

# URL for the GraphQL endpoint
URL="https://api.loadtest.dev.sparkinfra.net/graphql/spark/rc"

# GraphQL query
QUERY='mutation RequestRegtestFunds($address: String!, $amount_sats: Long!) {
  request_regtest_funds(input: {address: $address, amount_sats: $amount_sats}) {
    transaction_hash
  }
}'

# Create JSON payload with query and variables
JSON_PAYLOAD=$(cat <<EOF
{
  "operationName": "RequestRegtestFunds",
  "query": $(echo "$QUERY" | jq -Rs .),
  "variables": {
    "address": "$ADDRESS",
    "amount_sats": $AMOUNT_SATS
  }
}
EOF
)

echo "Requesting $AMOUNT_SATS sats for address $ADDRESS"

RESPONSE=$(curl -s -X POST "$URL" \
  -H "Content-Type: application/json" \
  -H "Accept: application/json" \
  -u "$USERNAME:$PASSWORD" \
  -d "$JSON_PAYLOAD")

# Check if curl request was successful
if [ $? -ne 0 ]; then
    echo "Error: Failed to connect to the GraphQL endpoint"
    exit 1
fi

# Print the response
if echo "$RESPONSE" | jq . > /dev/null 2>&1; then    
    # Extract transaction hash if available and response is valid JSON
    TXID=$(echo "$RESPONSE" | jq -r '.data.request_regtest_funds.transaction_hash // "Not available"')

    if [ "$TXID" != "Not available" ] && [ "$TXID" != "null" ]; then
        echo "Transaction ID: $TXID"
    else
        echo "Failed to get transaction ID. Check response for errors."
        exit 1
    fi
else
    echo "Received non-JSON response. Cannot process further."
    echo "Raw Response:"
    echo "$RESPONSE"
    exit 1
fi
