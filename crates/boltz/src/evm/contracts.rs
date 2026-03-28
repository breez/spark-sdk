use alloy_primitives::{Address, FixedBytes, U256};
use alloy_sol_types::{SolCall, SolValue, sol};

use crate::api::types::QuoteCalldata;
use crate::error::BoltzError;

// ─── Router contract (boltz-core v2, VERSION = 2) ────────────────────────

sol! {
    /// Cooperative ERC20 claim data. The `v/r/s` fields are the `ERC20Swap`
    /// cooperative claim EIP-712 signature (signed by the gas signer / claimAddress).
    struct Erc20Claim {
        bytes32 preimage;
        uint256 amount;
        address tokenAddress;
        address refundAddress;
        uint256 timelock;
        uint8 v;
        bytes32 r;
        bytes32 s;
    }

    /// A single call for the Router to execute (DEX swap calls).
    /// NOTE: Boltz encode API returns `{to, value, data}` but the Router contract
    /// uses `{target, value, callData}`. Use `Call::from_quote_calldata` to map.
    struct Call {
        address target;
        uint256 value;
        bytes callData;
    }

    /// Same-chain: claim tBTC + DEX swap + sweep output token to `destination`.
    /// The trailing v/r/s is the Router EIP-712 Claim signature.
    function claimERC20Execute(
        Erc20Claim calldata claim,
        Call[] calldata calls,
        address token,
        uint256 minAmountOut,
        address destination,
        uint8 v,
        bytes32 r,
        bytes32 s
    );

    /// OFT send parameters for cross-chain bridging via `LayerZero`.
    struct SendData {
        uint32 dstEid;
        bytes32 to;
        bytes extraOptions;
        bytes composeMsg;
        bytes oftCmd;
    }

    /// Authorization for cross-chain Router.claimERC20ExecuteOft.
    struct ClaimSendAuthorization {
        uint256 minAmountLd;
        uint256 lzTokenFee;
        address refundAddress;
        uint8 v;
        bytes32 r;
        bytes32 s;
    }

    /// Cross-chain: claim + DEX swap + OFT bridge to another chain.
    function claimERC20ExecuteOft(
        Erc20Claim calldata claim,
        Call[] calldata calls,
        address token,
        address oft,
        SendData calldata sendData,
        ClaimSendAuthorization calldata auth
    );

    // ─── ERC20Swap contract ──────────────────────────────────────────────

    /// Direct claim (non-Router fallback). Anyone can call; tokens go to claimAddress.
    function claim(
        bytes32 preimage,
        uint256 amount,
        address tokenAddress,
        address claimAddress,
        address refundAddress,
        uint256 timelock
    );

    /// `ERC20Swap` version — used for EIP-712 domain (currently returns 6).
    function version() external view returns (uint64);

    // ─── ERC20 ───────────────────────────────────────────────────────────

    function transfer(address to, uint256 amount) returns (bool);
    function balanceOf(address account) returns (uint256);

    // ─── Router read functions ───────────────────────────────────────────

    function TYPEHASH_SEND_DATA() external view returns (bytes32);

    // ─── OFT Contract (LayerZero USDT0) ──────────────────────────────

    struct OftSendParam {
        uint32 dstEid;
        bytes32 to;
        uint256 amountLD;
        uint256 minAmountLD;
        bytes extraOptions;
        bytes composeMsg;
        bytes oftCmd;
    }

    struct OftLimit {
        uint256 minAmountLD;
        uint256 maxAmountLD;
    }

    struct OftReceipt {
        uint256 amountSentLD;
        uint256 amountReceivedLD;
    }

    struct OftFeeDetail {
        int256 feeAmountLD;
        string description;
    }

    struct MessagingFee {
        uint256 nativeFee;
        uint256 lzTokenFee;
    }

    function quoteOFT(OftSendParam calldata sendParam)
        external view
        returns (OftLimit, OftFeeDetail[], OftReceipt);

    function quoteSend(OftSendParam calldata sendParam, bool payInLzToken)
        external view
        returns (MessagingFee);
}

/// Convert a Boltz encode API `QuoteCalldata` to a Router `Call`.
/// Maps: `to` -> `target`, `data` -> `callData`.
pub fn quote_calldata_to_call(qc: &QuoteCalldata) -> Result<Call, BoltzError> {
    let target = parse_address(&qc.to)?;
    let value = parse_u256(&qc.value)?;
    let call_data = parse_hex_bytes(&qc.data)?;

    Ok(Call {
        target,
        value,
        callData: call_data.into(),
    })
}

/// Encode `claimERC20Execute` calldata for same-chain delivery.
#[expect(clippy::too_many_arguments)]
pub fn encode_claim_erc20_execute(
    claim: &Erc20Claim,
    calls: &[Call],
    token: Address,
    min_amount_out: U256,
    destination: Address,
    router_sig_v: u8,
    router_sig_r: [u8; 32],
    router_sig_s: [u8; 32],
) -> Vec<u8> {
    let call = claimERC20ExecuteCall {
        claim: claim.clone(),
        calls: calls.to_vec(),
        token,
        minAmountOut: min_amount_out,
        destination,
        v: router_sig_v,
        r: router_sig_r.into(),
        s: router_sig_s.into(),
    };
    call.abi_encode()
}

/// Encode `claimERC20ExecuteOft` calldata for cross-chain delivery.
pub fn encode_claim_erc20_execute_oft(
    claim: &Erc20Claim,
    calls: &[Call],
    token: Address,
    oft: Address,
    send_data: &SendData,
    auth: &ClaimSendAuthorization,
) -> Vec<u8> {
    let call = claimERC20ExecuteOftCall {
        claim: claim.clone(),
        calls: calls.to_vec(),
        token,
        oft,
        sendData: send_data.clone(),
        auth: auth.clone(),
    };
    call.abi_encode()
}

/// Encode `version()` calldata for reading `ERC20Swap` version.
pub fn encode_version_call() -> Vec<u8> {
    versionCall {}.abi_encode()
}

/// Decode `version()` return value.
pub fn decode_version_return(data: &[u8]) -> Result<u64, BoltzError> {
    let decoded = <(u64,)>::abi_decode(data).map_err(|e| BoltzError::Evm {
        reason: format!("Failed to decode version return: {e}"),
        tx_hash: None,
    })?;
    Ok(decoded.0)
}

/// Encode `balanceOf(address)` calldata.
pub fn encode_balance_of(account: Address) -> Vec<u8> {
    balanceOfCall { account }.abi_encode()
}

/// Decode `balanceOf` return value.
pub fn decode_balance_of(data: &[u8]) -> Result<U256, BoltzError> {
    let decoded = <(U256,)>::abi_decode(data).map_err(|e| BoltzError::Evm {
        reason: format!("Failed to decode balanceOf return: {e}"),
        tx_hash: None,
    })?;
    Ok(decoded.0)
}

/// Encode the direct `claim()` calldata (non-Router fallback).
pub fn encode_direct_claim(
    preimage: [u8; 32],
    amount: U256,
    token_address: Address,
    claim_address: Address,
    refund_address: Address,
    timelock: U256,
) -> Vec<u8> {
    let call = claimCall {
        preimage: preimage.into(),
        amount,
        tokenAddress: token_address,
        claimAddress: claim_address,
        refundAddress: refund_address,
        timelock,
    };
    call.abi_encode()
}

/// Encode `TYPEHASH_SEND_DATA()` calldata.
pub fn encode_typehash_send_data_call() -> Vec<u8> {
    TYPEHASH_SEND_DATACall {}.abi_encode()
}

/// Decode `TYPEHASH_SEND_DATA()` return value.
pub fn decode_typehash_send_data(data: &[u8]) -> Result<[u8; 32], BoltzError> {
    let decoded = <(FixedBytes<32>,)>::abi_decode(data).map_err(|e| BoltzError::Evm {
        reason: format!("Failed to decode TYPEHASH_SEND_DATA return: {e}"),
        tx_hash: None,
    })?;
    Ok(decoded.0.into())
}

// ─── OFT helpers ─────────────────────────────────────────────────────────

/// Build an `OftSendParam` for quoting. `extraOptions`, `composeMsg`, and `oftCmd`
/// are empty (no native drops — disabled for USDT0, matching the web app).
pub fn build_oft_send_param(
    dst_eid: u32,
    recipient: Address,
    amount_ld: U256,
    min_amount_ld: U256,
) -> OftSendParam {
    OftSendParam {
        dstEid: dst_eid,
        to: address_to_bytes32(recipient),
        amountLD: amount_ld,
        minAmountLD: min_amount_ld,
        extraOptions: vec![].into(),
        composeMsg: vec![].into(),
        oftCmd: vec![].into(),
    }
}

/// Left-pad a 20-byte EVM address to 32 bytes (`bytes32`), as required by OFT `to` field.
pub fn address_to_bytes32(addr: Address) -> FixedBytes<32> {
    let mut bytes = [0u8; 32];
    bytes[12..32].copy_from_slice(addr.as_slice());
    FixedBytes::from(bytes)
}

/// Encode `quoteOFT(OftSendParam)` calldata.
pub fn encode_quote_oft(send_param: &OftSendParam) -> Vec<u8> {
    quoteOFTCall {
        sendParam: send_param.clone(),
    }
    .abi_encode()
}

/// Decode `quoteOFT` return value: `(OftLimit, OftFeeDetail[], OftReceipt)`.
pub fn decode_quote_oft_return(
    data: &[u8],
) -> Result<(OftLimit, Vec<OftFeeDetail>, OftReceipt), BoltzError> {
    let decoded = quoteOFTCall::abi_decode_returns(data).map_err(|e| BoltzError::Evm {
        reason: format!("Failed to decode quoteOFT return: {e}"),
        tx_hash: None,
    })?;
    Ok((decoded._0, decoded._1, decoded._2))
}

/// Encode `quoteSend(OftSendParam, bool)` calldata.
pub fn encode_quote_send(send_param: &OftSendParam, pay_in_lz_token: bool) -> Vec<u8> {
    quoteSendCall {
        sendParam: send_param.clone(),
        payInLzToken: pay_in_lz_token,
    }
    .abi_encode()
}

/// Decode `quoteSend` return value: `MessagingFee`.
pub fn decode_quote_send_return(data: &[u8]) -> Result<MessagingFee, BoltzError> {
    let decoded = quoteSendCall::abi_decode_returns(data).map_err(|e| BoltzError::Evm {
        reason: format!("Failed to decode quoteSend return: {e}"),
        tx_hash: None,
    })?;
    Ok(MessagingFee {
        nativeFee: decoded.nativeFee,
        lzTokenFee: decoded.lzTokenFee,
    })
}

/// Compute the EIP-712 struct hash for `SendData`.
///
/// `hash = keccak256(abi.encode(TYPEHASH, dstEid, to, keccak256(extraOptions), keccak256(composeMsg), keccak256(oftCmd)))`
pub fn hash_send_data(typehash: [u8; 32], send_data: &SendData) -> [u8; 32] {
    use alloy_primitives::keccak256;

    let extra_options_hash = keccak256(send_data.extraOptions.as_ref());
    let compose_msg_hash = keccak256(send_data.composeMsg.as_ref());
    let oft_cmd_hash = keccak256(send_data.oftCmd.as_ref());

    let encoded = (
        FixedBytes::<32>::from(typehash),
        U256::from(send_data.dstEid),
        send_data.to,
        extra_options_hash,
        compose_msg_hash,
        oft_cmd_hash,
    )
        .abi_encode();

    keccak256(&encoded).into()
}

// ─── Helpers ─────────────────────────────────────────────────────────────

/// Parse a hex address string (with or without 0x prefix) into an `Address`.
pub fn parse_address(hex_str: &str) -> Result<Address, BoltzError> {
    let clean = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(clean).map_err(|e| BoltzError::Evm {
        reason: format!("Invalid address hex '{hex_str}': {e}"),
        tx_hash: None,
    })?;
    if bytes.len() != 20 {
        return Err(BoltzError::Evm {
            reason: format!("Address must be 20 bytes, got {}", bytes.len()),
            tx_hash: None,
        });
    }
    Ok(Address::from_slice(&bytes))
}

/// Parse a decimal or hex string into a `U256`.
pub fn parse_u256(s: &str) -> Result<U256, BoltzError> {
    if let Some(hex_str) = s.strip_prefix("0x") {
        U256::from_str_radix(hex_str, 16)
    } else {
        U256::from_str_radix(s, 10)
    }
    .map_err(|e| BoltzError::Evm {
        reason: format!("Invalid U256 value '{s}': {e}"),
        tx_hash: None,
    })
}

/// Parse hex-encoded bytes (with or without 0x prefix).
pub fn parse_hex_bytes(hex_str: &str) -> Result<Vec<u8>, BoltzError> {
    let clean = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    hex::decode(clean).map_err(|e| BoltzError::Evm {
        reason: format!("Invalid hex bytes '{hex_str}': {e}"),
        tx_hash: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_sol_types::SolCall;

    #[test]
    fn test_parse_address() {
        let addr = parse_address("0xaB6B467FC443Ca37a8E5aA11B04ea29434688d61").unwrap();
        let expected_bytes = hex::decode("aB6B467FC443Ca37a8E5aA11B04ea29434688d61").unwrap();
        assert_eq!(addr.as_slice(), &expected_bytes);
    }

    #[test]
    fn test_parse_address_no_prefix() {
        let addr = parse_address("aB6B467FC443Ca37a8E5aA11B04ea29434688d61").unwrap();
        let expected_bytes = hex::decode("aB6B467FC443Ca37a8E5aA11B04ea29434688d61").unwrap();
        assert_eq!(addr.as_slice(), &expected_bytes);
    }

    #[test]
    fn test_parse_address_invalid_length() {
        let result = parse_address("0x1234");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_u256_decimal() {
        let val = parse_u256("1000000000000000000").unwrap();
        assert_eq!(val, U256::from(1_000_000_000_000_000_000u64));
    }

    #[test]
    fn test_parse_u256_hex() {
        let val = parse_u256("0xde0b6b3a7640000").unwrap();
        assert_eq!(val, U256::from(1_000_000_000_000_000_000u64));
    }

    #[test]
    fn test_parse_u256_zero() {
        let val = parse_u256("0").unwrap();
        assert_eq!(val, U256::ZERO);
    }

    #[test]
    fn test_quote_calldata_to_call() {
        let qc = QuoteCalldata {
            to: "0xaB6B467FC443Ca37a8E5aA11B04ea29434688d61".to_string(),
            value: "0".to_string(),
            data: "0xabcdef".to_string(),
        };
        let call = quote_calldata_to_call(&qc).unwrap();
        assert_eq!(
            call.target,
            parse_address("0xaB6B467FC443Ca37a8E5aA11B04ea29434688d61").unwrap()
        );
        assert_eq!(call.value, U256::ZERO);
        assert_eq!(call.callData.as_ref(), &[0xab, 0xcd, 0xef]);
    }

    #[test]
    fn test_version_call_selector() {
        let encoded = encode_version_call();
        // function selector for `version()` = keccak256("version()")[..4]
        // = 0x54fd4d50
        assert_eq!(&encoded[..4], &[0x54, 0xfd, 0x4d, 0x50]);
    }

    #[test]
    fn test_decode_version() {
        // ABI-encode uint64 value 6
        let encoded = U256::from(6).abi_encode();
        let version = decode_version_return(&encoded).unwrap();
        assert_eq!(version, 6);
    }

    #[test]
    fn test_balance_of_call_selector() {
        let addr = parse_address("0x0000000000000000000000000000000000000001").unwrap();
        let encoded = encode_balance_of(addr);
        // function selector for `balanceOf(address)` = 0x70a08231
        assert_eq!(&encoded[..4], &[0x70, 0xa0, 0x82, 0x31]);
    }

    #[test]
    fn test_decode_balance_of() {
        let encoded = U256::from(1_000_000u64).abi_encode();
        let balance = decode_balance_of(&encoded).unwrap();
        assert_eq!(balance, U256::from(1_000_000u64));
    }

    #[test]
    fn test_direct_claim_call_selector() {
        let encoded = encode_direct_claim(
            [0u8; 32],
            U256::from(100u64),
            Address::ZERO,
            Address::ZERO,
            Address::ZERO,
            U256::from(1000u64),
        );
        // function selector for claim(bytes32,uint256,address,address,address,uint256)
        // = keccak256("claim(bytes32,uint256,address,address,address,uint256)")[..4]
        let selector = &encoded[..4];
        let expected_selector = &claimCall::SELECTOR;
        assert_eq!(selector, expected_selector);
    }

    #[test]
    fn test_claim_erc20_execute_encodes() {
        let claim = Erc20Claim {
            preimage: [1u8; 32].into(),
            amount: U256::from(100_000_000_000_000u64),
            tokenAddress: parse_address("0x6c84a8f1c29108F47a79964b5Fe888D4f4D0dE40").unwrap(),
            refundAddress: parse_address("0x0000000000000000000000000000000000000002").unwrap(),
            timelock: U256::from(12345u64),
            v: 27,
            r: [2u8; 32].into(),
            s: [3u8; 32].into(),
        };

        let calls = vec![Call {
            target: parse_address("0x0000000000000000000000000000000000000003").unwrap(),
            value: U256::ZERO,
            callData: vec![0xab, 0xcd].into(),
        }];

        let token = parse_address("0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9").unwrap();
        let min_amount_out = U256::from(71_000_000u64);
        let destination = parse_address("0x0000000000000000000000000000000000000004").unwrap();

        let encoded = encode_claim_erc20_execute(
            &claim,
            &calls,
            token,
            min_amount_out,
            destination,
            28,
            [4u8; 32],
            [5u8; 32],
        );

        // Verify it starts with the correct function selector
        let expected_selector = &claimERC20ExecuteCall::SELECTOR;
        assert_eq!(&encoded[..4], expected_selector);
        // Encoded data should be non-trivial (claim struct + dynamic calls array + trailing params)
        assert!(encoded.len() > 500);
    }

    #[test]
    fn test_claim_erc20_execute_oft_encodes() {
        let claim = Erc20Claim {
            preimage: [1u8; 32].into(),
            amount: U256::from(100_000_000_000_000u64),
            tokenAddress: Address::ZERO,
            refundAddress: Address::ZERO,
            timelock: U256::from(100u64),
            v: 27,
            r: [0u8; 32].into(),
            s: [0u8; 32].into(),
        };

        let send_data = SendData {
            dstEid: 30101,
            to: [0xaa; 32].into(),
            extraOptions: vec![].into(),
            composeMsg: vec![].into(),
            oftCmd: vec![].into(),
        };

        let auth = ClaimSendAuthorization {
            minAmountLd: U256::from(1000u64),
            lzTokenFee: U256::ZERO,
            refundAddress: Address::ZERO,
            v: 28,
            r: [0u8; 32].into(),
            s: [0u8; 32].into(),
        };

        let encoded = encode_claim_erc20_execute_oft(
            &claim,
            &[],
            Address::ZERO,
            Address::ZERO,
            &send_data,
            &auth,
        );

        let expected_selector = &claimERC20ExecuteOftCall::SELECTOR;
        assert_eq!(&encoded[..4], expected_selector);
        assert!(encoded.len() > 200);
    }

    #[test]
    fn test_typehash_send_data_call_selector() {
        let encoded = encode_typehash_send_data_call();
        let expected_selector = &TYPEHASH_SEND_DATACall::SELECTOR;
        assert_eq!(&encoded[..4], expected_selector);
    }

    #[test]
    fn test_parse_hex_bytes() {
        let bytes = parse_hex_bytes("0xabcdef").unwrap();
        assert_eq!(bytes, vec![0xab, 0xcd, 0xef]);

        let bytes_no_prefix = parse_hex_bytes("abcdef").unwrap();
        assert_eq!(bytes_no_prefix, vec![0xab, 0xcd, 0xef]);

        let empty = parse_hex_bytes("0x").unwrap();
        assert!(empty.is_empty());
    }

    // ─── OFT tests ───────────────────────────────────────────────────

    #[test]
    fn test_address_to_bytes32() {
        let addr = parse_address("0xaB6B467FC443Ca37a8E5aA11B04ea29434688d61").unwrap();
        let b32 = address_to_bytes32(addr);
        // First 12 bytes should be zero-padding
        assert_eq!(&b32[..12], &[0u8; 12]);
        // Last 20 bytes should be the address
        assert_eq!(&b32[12..], addr.as_slice());
    }

    #[test]
    fn test_address_to_bytes32_zero() {
        let b32 = address_to_bytes32(Address::ZERO);
        assert_eq!(b32, FixedBytes::<32>::ZERO);
    }

    #[test]
    fn test_quote_oft_call_selector() {
        let send_param = build_oft_send_param(30101, Address::ZERO, U256::ZERO, U256::ZERO);
        let encoded = encode_quote_oft(&send_param);
        let expected_selector = &quoteOFTCall::SELECTOR;
        assert_eq!(&encoded[..4], expected_selector);
    }

    #[test]
    fn test_quote_send_call_selector() {
        let send_param = build_oft_send_param(30101, Address::ZERO, U256::ZERO, U256::ZERO);
        let encoded = encode_quote_send(&send_param, false);
        let expected_selector = &quoteSendCall::SELECTOR;
        assert_eq!(&encoded[..4], expected_selector);
    }

    #[test]
    fn test_hash_send_data_deterministic() {
        let typehash = [1u8; 32];
        let send_data = SendData {
            dstEid: 30101,
            to: [0xaa; 32].into(),
            extraOptions: vec![].into(),
            composeMsg: vec![].into(),
            oftCmd: vec![].into(),
        };

        let hash1 = hash_send_data(typehash, &send_data);
        let hash2 = hash_send_data(typehash, &send_data);
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, [0u8; 32]);
    }

    #[test]
    fn test_hash_send_data_different_eid() {
        let typehash = [1u8; 32];
        let sd1 = SendData {
            dstEid: 30101,
            to: [0xaa; 32].into(),
            extraOptions: vec![].into(),
            composeMsg: vec![].into(),
            oftCmd: vec![].into(),
        };
        let sd2 = SendData {
            dstEid: 30111,
            ..sd1.clone()
        };
        assert_ne!(
            hash_send_data(typehash, &sd1),
            hash_send_data(typehash, &sd2)
        );
    }

    #[test]
    fn test_build_oft_send_param() {
        let addr = parse_address("0xaB6B467FC443Ca37a8E5aA11B04ea29434688d61").unwrap();
        let sp = build_oft_send_param(30111, addr, U256::from(1000u64), U256::from(900u64));
        assert_eq!(sp.dstEid, 30111);
        assert_eq!(&sp.to[12..], addr.as_slice());
        assert_eq!(sp.amountLD, U256::from(1000u64));
        assert_eq!(sp.minAmountLD, U256::from(900u64));
        assert!(sp.extraOptions.is_empty());
        assert!(sp.composeMsg.is_empty());
        assert!(sp.oftCmd.is_empty());
    }
}
