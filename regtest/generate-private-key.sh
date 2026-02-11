#!/bin/bash
set -e

# Function to check if required commands are installed
check_dependencies() {
  local missing_deps=()
  for cmd in openssl xxd bc mktemp; do
    if ! command -v $cmd &> /dev/null; then
      missing_deps+=("$cmd")
    fi
  done
  
  # Check if OpenSSL supports the required algorithms
  if ! openssl list -digest-algorithms | grep -q "RIPEMD160"; then
    echo "Warning: Your OpenSSL installation doesn't support RIPEMD160."
    echo "This script may not work correctly."
  fi
  
  if [ ${#missing_deps[@]} -ne 0 ]; then
    echo "Error: The following dependencies are missing: ${missing_deps[*]}"
    echo "Please install them and try again."
    exit 1
  fi
}

# Generate a random 32-byte private key
generate_private_key() {
  openssl rand -hex 32
}

# Convert private key to WIF format for regtest
private_key_to_wif() {
  local private_key=$1
  
  # Prepend 0xef for regtest private key (0x80 for mainnet)
  local extended="ef${private_key}01" # 01 suffix for compressed keys
  
  # Calculate double SHA256 checksum (first 4 bytes)
  local checksum=$(echo -n "$extended" | xxd -r -p | openssl dgst -sha256 -binary | openssl dgst -sha256 -binary | xxd -p -l 4)
  
  # Final extended key with checksum
  local extended_with_checksum="${extended}${checksum}"
  
  # Convert to base58
  local wif=$(echo -n "$extended_with_checksum" | xxd -r -p | base58_encode)
  
  echo "$wif"
}

private_key_to_public_key() {
  local private_key=$1

  # Build a proper SEC1 EC private key in DER format for secp256k1
  # The structure is:
  #   SEQUENCE {
  #     INTEGER 1 (version)
  #     OCTET STRING (32 bytes private key)
  #     [0] OID 1.3.132.0.10 (secp256k1)
  #   }

  local temp_dir=$(mktemp -d)
  local der_file="$temp_dir/privkey.der"
  local pem_file="$temp_dir/privkey.pem"

  # secp256k1 OID in DER: 06 05 2b 81 04 00 0a
  # Build the DER structure manually:
  # 30 (SEQUENCE) + length + contents

  # Version: 02 01 01
  local version="020101"

  # Private key octet string: 04 20 + 32 bytes
  local priv_octet="0420${private_key}"

  # Context tag [0] with secp256k1 OID: a0 07 06 05 2b 81 04 00 0a
  local curve_oid="a00706052b8104000a"

  # Calculate total length of inner contents
  local inner_hex="${version}${priv_octet}${curve_oid}"
  local inner_len=$((${#inner_hex} / 2))

  # Format length (assuming < 128 bytes, which it is)
  local len_hex=$(printf '%02x' $inner_len)

  # Full DER: 30 + length + contents
  local full_der="30${len_hex}${inner_hex}"

  # Write DER file
  echo -n "$full_der" | xxd -r -p > "$der_file"

  # Convert to PEM
  echo "-----BEGIN EC PRIVATE KEY-----" > "$pem_file"
  base64 < "$der_file" >> "$pem_file"
  echo "-----END EC PRIVATE KEY-----" >> "$pem_file"

  # Extract compressed public key using OpenSSL
  local public_key=$(openssl ec -in "$pem_file" -pubout -conv_form compressed 2>/dev/null | \
    openssl ec -pubin -outform DER 2>/dev/null | \
    tail -c 33 | xxd -p -c 66 | tr -d '\n')

  # Clean up
  rm -rf "$temp_dir"

  echo "$public_key"
}

# Generate regtest address from private key
generate_address() {
  local private_key=$1
  local public_key=$(private_key_to_public_key "$private_key")
  local address_type=${2:-"p2wpkh"} # Default to p2wpkh (native segwit)
  
  # Get RIPEMD160 hash of the SHA256 of the public key (HASH160)
  local pubkey_hash=$(echo -n "$public_key" | xxd -r -p | openssl dgst -sha256 -binary | openssl dgst -ripemd160 -binary | xxd -p -c 40 | tr -d '\n')
  
  case "$address_type" in
    "p2pkh")
      # For regtest, prepend 0x6f for P2PKH addresses (mainnet is 0x00)
      local extended="6f$pubkey_hash"
      local checksum=$(echo -n "$extended" | xxd -r -p | openssl dgst -sha256 -binary | openssl dgst -sha256 -binary | xxd -p -l 4)
      local address=$(echo -n "${extended}${checksum}" | xxd -r -p | base58_encode)
      echo "$address"
      ;;
    "p2sh")
      # For P2SH in regtest (0xc4 prefix, mainnet is 0x05)
      # Create P2WPKH redeem script: 0014<pubkey_hash>
      local script="0014$pubkey_hash"
      local script_hash=$(echo -n "$script" | xxd -r -p | openssl dgst -sha256 -binary | openssl dgst -ripemd160 -binary | xxd -p -c 40)
      local extended="c4$script_hash"
      local checksum=$(echo -n "$extended" | xxd -r -p | openssl dgst -sha256 -binary | openssl dgst -sha256 -binary | xxd -p -l 4)
      local address=$(echo -n "${extended}${checksum}" | xxd -r -p | base58_encode)
      echo "$address"
      ;;
    "p2wpkh")
      # Implement Bech32 encoding properly according to BIP173
      # For regtest, the human readable part is "bcrt"
      local hrp="bcrt"
      
      # The version byte for segwit v0 is 0
      local segwit_version=0
      
      # Segwit data - the 20-byte pubkey hash (HASH160)
      local data="$pubkey_hash"
      
      # This is the Bech32 character set
      local CHARSET="qpzry9x8gf2tvdw0s3jn54khce6mua7l"
      
      # 1. Create the witness program by appending segwit version and pubkey hash
      # For segwit v0, convert the pubkey hash to 5-bit words
      
      # First calculate how many 5-bit values we need
      local data_bytes=$((${#data} / 2)) # Each byte is 2 hex chars
      
      # Convert pubkey hash from 8-bit to 5-bit values
      local five_bit_data=""
      local acc=0
      local bits=0
      
      # Convert each byte (2 hex chars) of the pubkey hash
      for ((i=0; i<${#data}; i+=2)); do
        local value=$((16#${data:i:2}))
        acc=$((acc << 8 | value))
        bits=$((bits + 8))
        
        # Extract 5-bit chunks
        while ((bits >= 5)); do
          bits=$((bits - 5))
          local v=$((acc >> bits & 31))
          five_bit_data="${five_bit_data}${v} "
        done
      done
      
      # Handle remaining bits
      if ((bits > 0)); then
        local v=$((acc << (5 - bits) & 31))
        five_bit_data="${five_bit_data}${v} "
      fi
      
      # Create an array of 5-bit integers - including the witness version
      local -a values=($segwit_version $five_bit_data)
      
      # 2. Expand the HRP into values for checksum calculation
      local expanded_hrp=""
      for ((i=0; i<${#hrp}; i++)); do
        expanded_hrp="${expanded_hrp}$(($(printf '%d' "'${hrp:i:1}") >> 5)) "
      done
      expanded_hrp="${expanded_hrp}0 " # Add separator
      
      for ((i=0; i<${#hrp}; i++)); do
        expanded_hrp="${expanded_hrp}$(($(printf '%d' "'${hrp:i:1}") & 31)) "
      done
      
      # 3. Create data for checksum calculation
      local checksum_values="${expanded_hrp}"
      for value in "${values[@]}"; do
        checksum_values="${checksum_values}${value} "
      done
      # Add zero padding for the checksum
      checksum_values="${checksum_values}0 0 0 0 0 0 "
      
      # 4. Calculate Bech32 checksum using the polynomial
      local polymod=1
      local -a GENERATOR=(0x3b6a57b2 0x26508e6d 0x1ea119fa 0x3d4233dd 0x2a1462b3)
      
      for value in $checksum_values; do
        local top=$((polymod >> 25))
        polymod=$(( (polymod & 0x1ffffff) << 5 ^ value ))
        
        for ((j=0; j<5; j++)); do
          if (( ((top >> j) & 1) )); then
            polymod=$((polymod ^ ${GENERATOR[j]}))
          fi
        done
      done
      
      # Final XOR with 1
      polymod=$((polymod ^ 1))
      
      # 5. Convert the checksum to 6 5-bit values
      local -a checksum_values=()
      for ((i=0; i<6; i++)); do
        checksum_values[i]=$(( (polymod >> (5 * (5 - i))) & 31 ))
      done
      
      # 6. Build the address string
      local address="${hrp}1" # Start with HRP and separator
      
      # Add data part (witness program)
      for value in "${values[@]}"; do
        address="${address}${CHARSET:value:1}"
      done
      
      # Add checksum
      for value in "${checksum_values[@]}"; do
        address="${address}${CHARSET:value:1}"
      done
      
      echo "$address"
      ;;
    *)
      echo "Unsupported address type: $address_type"
      exit 1
      ;;
  esac
}

# Real Base58 encoding implementation
base58_encode() {
  local input=$(xxd -p -c 256)  # Get hexadecimal input
  local alphabet="123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
  local result=""
  
  # Count leading zeros in the input (bytes)
  local leading_zeros=0
  local i=0
  while [ $i -lt ${#input} ] && [ "${input:i:2}" = "00" ]; do
    ((leading_zeros++))
    i=$((i+2))
  done
  
  # Prepend a "1" for each leading zero byte
  for (( i=0; i<leading_zeros; i++ )); do
    result="${result}1"
  done
  
  # Convert hex input to decimal using bc
  # Uppercase the input and properly format the bc command
  local decimal=$(echo "ibase=16; $(echo $input | tr '[:lower:]' '[:upper:]')" | bc)
  
  # Special case for zero
  if [ "$decimal" = "0" ]; then
    echo "$result"
    return
  fi
  
  # Convert from decimal to base58 string
  local base58=""
  while [ "$decimal" != "0" ]; do
    # Calculate remainder when dividing by 58
    local remainder=$(echo "$decimal % 58" | bc)
    # Prepend the corresponding character
    base58="${alphabet:$remainder:1}$base58"
    # Integer division by 58
    decimal=$(echo "$decimal / 58" | bc)
  done
  
  # Combine leading 1's with the converted base58 string
  echo "${result}${base58}"
}

# Bech32 helper functions
bech32_hrp_expand() {
  local hrp="$1"
  local result=""
  
  # Expand HRP chars to 5-bit values
  for (( i=0; i<${#hrp}; i++ )); do
    # Get ASCII value of char
    local c=$(printf '%d' "'${hrp:i:1}")
    # First set of values are the high bits (shifted right by 5)
    result="${result}$(printf '%x' $(( c >> 5 )))"
  done
  
  # Add separator (0)
  result="${result}0"
  
  # Second set of values are the low bits (& 31)
  for (( i=0; i<${#hrp}; i++ )); do
    local c=$(printf '%d' "'${hrp:i:1}")
    result="${result}$(printf '%x' $(( c & 31 )))"
  done
  
  echo "$result"
}

# Convert from 8-bit bytes to 5-bit values (for bech32)
convert_bits() {
  local data="$1"
  local from_bits=8
  local to_bits=5
  local pad=1
  local value=0
  local bits=0
  local max_v=$(( (1 << to_bits) - 1 ))
  local result=""
  
  # Process hex data in pairs (bytes)
  for (( i=0; i<${#data}; i+=2 )); do
    # Convert hex to decimal
    local byte=$(( 16#${data:i:2} ))
    
    # Accumulate bits
    value=$(( (value << from_bits) | byte ))
    bits=$(( bits + from_bits ))
    
    # Extract complete to_bits chunks
    while (( bits >= to_bits )); do
      bits=$(( bits - to_bits ))
      result="${result}$(printf '%x' $(( (value >> bits) & max_v )))"
    done
  done
  
  # Handle remaining bits with proper padding
  if (( pad && bits > 0 )); then
    result="${result}$(printf '%x' $(( (value << (to_bits - bits)) & max_v )))"
  fi
  
  echo "$result"
}

# Calculate bech32 checksum - implements the polymod function from BIP173
bech32_polymod() {
  local values="$1"
  # These are the generator coefficients for bech32, specified in BIP173
  local generator=(0x3b6a57b2 0x26508e6d 0x1ea119fa 0x3d4233dd 0x2a1462b3)
  local chk=1
  
  # Process each value
  for (( i=0; i<${#values}; i++ )); do
    local v=$(( 16#${values:i:1} ))
    if [ $v -lt 0 ] || [ $v -ge 32 ]; then
      return 1
    fi
    
    # Extract high bits for generator XOR
    local top=$(( chk >> 25 ))
    
    # Shift and XOR with input value
    chk=$(( ((chk & 0x1ffffff) << 5) ^ v ))
    
    # Apply generator polynomial where needed
    for (( j=0; j<5; j++ )); do
      if (( ((top >> j) & 1) != 0 )); then
        chk=$(( chk ^ ${generator[$j]} ))
      fi
    done
  done
  
  # Final XOR with 1 as per BIP173
  echo $(( chk ^ 1 ))
}

# Encode data to bech32 format
bech32_create_checksum() {
  local hrp="$1"
  local data="$2"
  local hrp_expanded=$(bech32_hrp_expand "$hrp")
  local values="${hrp_expanded}${data}00000"
  local polymod=$(bech32_polymod "$values")
  local result=""
  
  for (( i=0; i<6; i++ )); do
    result="${result}$(printf '%x' $(( (polymod >> (5 * (5 - i))) & 31 )))"
  done
  
  echo "$result"
}

# Main bech32 encoding function for segwit addresses
bech32_encode() {
  local hrp="$1"       # Human readable part (bcrt for regtest)
  local data="$2"      # Witness version + data (all in 5-bit format)
  local charset="qpzry9x8gf2tvdw0s3jn54khce6mua7l"
  
  # Calculate checksum
  local checksum=$(bech32_create_checksum "$hrp" "$data")
  
  # Convert to Bech32 format
  local result="${hrp}1"  # HRP + separator (1)
  
  # Encode the data (witness version + pubkey hash in 5-bit format)
  for (( i=0; i<${#data}; i++ )); do
    local val=$(( 16#${data:i:1} ))
    if [ $val -lt 0 ] || [ $val -ge 32 ]; then
      return 1
    fi
    result="${result}${charset:$val:1}"
  done
  
  # Encode the checksum
  for (( i=0; i<${#checksum}; i++ )); do
    local val=$(( 16#${checksum:i:1} ))
    if [ $val -lt 0 ] || [ $val -ge 32 ]; then
      return 1
    fi
    result="${result}${charset:$val:1}"
  done
  
  echo "$result"
}

# Main execution
check_dependencies

echo "Generating Bitcoin private key for regtest network"
private_key=$(generate_private_key)
echo "Private key (hex): $private_key"

wif=$(private_key_to_wif "$private_key")
echo "Private key (WIF): $wif"

public_key=$(private_key_to_public_key "$private_key")
echo "Public key (hex): $public_key"

address_p2wpkh=$(generate_address "$private_key" "p2wpkh")
echo "Regtest address (p2wpkh): $address_p2wpkh"

echo "Use this private key and address for testing purposes only."
