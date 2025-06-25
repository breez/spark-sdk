// GraphQL query strings

/// LightningSendFeeEstimate query
pub const LIGHTNING_SEND_FEE_ESTIMATE: &str = r#"
query LightningSendFeeEstimate($encoded_invoice: String!) {
  lightning_send_fee_estimate(encoded_invoice: $encoded_invoice) {
    fee_sats
    amount_sats
    total_sats
  }
}
"#;

/// CoopExitFeeEstimate query
pub const COOP_EXIT_FEE_ESTIMATE: &str = r#"
query CoopExitFeeEstimates($leaf_external_ids: [String!]!, $withdrawal_address: String!) {
  coop_exit_fee_estimates(leaf_external_ids: $leaf_external_ids, withdrawal_address: $withdrawal_address) {
    request_fee_sats
    exit_fee_sats
    min_withdrawal_sats
  }
}
"#;

/// LeavesSwapFeeEstimate query
pub const LEAVES_SWAP_FEE_ESTIMATE: &str = r#"
query LeavesSwapFeeEstimate($total_amount_sats: Int!) {
  leaves_swap_fee_estimate(total_amount_sats: $total_amount_sats) {
    fee_sats
    min_per_leaf_amount_sats
  }
}
"#;

/// GetClaimDepositQuote query
pub const GET_CLAIM_DEPOSIT_QUOTE: &str = r#"
query StaticDepositQuote($transaction_id: String!, $output_index: Int!, $network: BitcoinNetwork!) {
  static_deposit_quote(transaction_id: $transaction_id, output_index: $output_index, network: $network) {
    transaction_id
    output_index
    deposit_value_sats
    credit_value_sats
    fee_sats
  }
}
"#;

/// UserRequest query - used for different types of user requests
pub const USER_REQUEST: &str = r#"
query UserRequest($request_id: ID!) {
  user_request(request_id: $request_id) {
    id
    status
    ... on LightningReceiveRequest {
      invoice {
        encoded_invoice
        payment_hash
        amount_sats
        memo
        expiry_timestamp
      }
    }
    ... on LightningSendRequest {
      encoded_invoice
      payment_hash
      amount_sats
      fee_sats
      preimage
    }
    ... on CoopExitRequest {
      leaf_external_ids
      withdrawal_address
      total_amount_sats
      request_fee_sats
      exit_fee_sats
      exit_speed
    }
    ... on LeavesSwapRequest {
      total_amount_sats
      target_amount_sats
      fee_sats
      user_leaves
      adaptor_pubkey
    }
  }
}
"#;
