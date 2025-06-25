// GraphQL mutation strings

/// GetChallenge mutation
pub const GET_CHALLENGE: &str = r#"
mutation GetChallenge(
  $public_key: String!
) {
  get_challenge(input: {
    public_key: $public_key
  }) {
    protected_challenge
  }
}
"#;

/// VerifyChallenge mutation
pub const VERIFY_CHALLENGE: &str = r#"
mutation VerifyChallenge(
  $protected_challenge: String!
  $signature: String!
  $identity_public_key: String!
  $provider: Provider
) {
  verify_challenge(input: {
    protected_challenge: $protected_challenge,
    signature: $signature, 
    identity_public_key: $identity_public_key,
    provider: $provider
  }) {
    session_token
    valid_until
  }
}
"#;

/// RequestLightningReceive mutation
pub const REQUEST_LIGHTNING_RECEIVE: &str = r#"
mutation RequestLightningReceive(
  $network: BitcoinNetwork!
  $amount_sats: Long!
  $payment_hash: Hash32!
  $expiry_secs: Int
  $memo: String
  $include_spark_address: Boolean
  $receiver_identity_pubkey: PublicKey
  $description_hash: Hash32
) {
  request_lightning_receive(
    input: {
      network: $network
      amount_sats: $amount_sats
      payment_hash: $payment_hash
      expiry_secs: $expiry_secs
      memo: $memo
      include_spark_address: $include_spark_address
      receiver_identity_pubkey: $receiver_identity_pubkey
      description_hash: $description_hash
    }
  ) {
    request {
      id
      status
      invoice {
        encoded_invoice
        payment_hash
        amount_sats
        memo
        expiry_timestamp
      }
    }
  }
}
"#;

/// RequestLightningSend mutation
pub const REQUEST_LIGHTNING_SEND: &str = r#"
mutation RequestLightningSend($encoded_invoice: String!, $idempotency_key: String!) {
  request_lightning_send(encoded_invoice: $encoded_invoice, idempotency_key: $idempotency_key) {
    request {
      id
      status
      encoded_invoice
      payment_hash
      amount_sats
      fee_sats
      preimage
    }
  }
}
"#;

/// RequestCoopExit mutation
pub const REQUEST_COOP_EXIT: &str = r#"
mutation RequestCoopExit(
  $leaf_external_ids: [String!]!, 
  $withdrawal_address: String!, 
  $idempotency_key: String!,
  $exit_speed: ExitSpeed!
) {
  request_coop_exit(
    leaf_external_ids: $leaf_external_ids, 
    withdrawal_address: $withdrawal_address, 
    idempotency_key: $idempotency_key,
    exit_speed: $exit_speed
  ) {
    request {
      id
      status
      leaf_external_ids
      withdrawal_address
      total_amount_sats
      request_fee_sats
      exit_fee_sats
      exit_speed
    }
  }
}
"#;

/// CompleteCoopExit mutation
pub const COMPLETE_COOP_EXIT: &str = r#"
mutation CompleteCoopExit($user_outbound_transfer_external_id: String!, $coop_exit_request_id: String!) {
  complete_coop_exit(user_outbound_transfer_external_id: $user_outbound_transfer_external_id, coop_exit_request_id: $coop_exit_request_id) {
    request {
      id
      status
      leaf_external_ids
      withdrawal_address
      total_amount_sats
      request_fee_sats
      exit_fee_sats
      exit_speed
    }
  }
}
"#;

/// RequestLeavesSwap mutation
pub const REQUEST_LEAVES_SWAP: &str = r#"
mutation RequestLeavesSwap(
  $adaptor_pubkey: String!,
  $total_amount_sats: Int!,
  $target_amount_sats: Int!,
  $fee_sats: Int!,
  $user_leaves: [String!]!,
  $idempotency_key: String!
) {
  request_leaves_swap(
    adaptor_pubkey: $adaptor_pubkey,
    total_amount_sats: $total_amount_sats,
    target_amount_sats: $target_amount_sats, 
    fee_sats: $fee_sats,
    user_leaves: $user_leaves,
    idempotency_key: $idempotency_key
  ) {
    request {
      id
      status
      total_amount_sats
      target_amount_sats
      fee_sats
      user_leaves
      adaptor_pubkey
    }
  }
}
"#;

/// CompleteLeavesSwap mutation
pub const COMPLETE_LEAVES_SWAP: &str = r#"
mutation CompleteLeavesSwap(
  $adaptor_secret_key: String!,
  $user_outbound_transfer_external_id: String!,
  $leaves_swap_request_id: String!
) {
  complete_leaves_swap(
    adaptor_secret_key: $adaptor_secret_key,
    user_outbound_transfer_external_id: $user_outbound_transfer_external_id,
    leaves_swap_request_id: $leaves_swap_request_id
  ) {
    request {
      id
      status
      total_amount_sats
      target_amount_sats
      fee_sats
      user_leaves
      adaptor_pubkey
    }
  }
}
"#;

/// ClaimStaticDeposit mutation
pub const CLAIM_STATIC_DEPOSIT: &str = r#"
mutation ClaimStaticDeposit(
  $transaction_id: String!,
  $output_index: Int!,
  $network: BitcoinNetwork!,
  $request_type: ClaimStaticDepositRequestType!,
  $credit_amount_sats: Int!,
  $deposit_secret_key: String!,
  $signature: String!,
  $quote_signature: String!
) {
  claim_static_deposit(
    transaction_id: $transaction_id,
    output_index: $output_index,
    network: $network,
    request_type: $request_type,
    credit_amount_sats: $credit_amount_sats,
    deposit_secret_key: $deposit_secret_key,
    signature: $signature,
    quote_signature: $quote_signature
  ) {
    deposit_value_sats
    credit_value_sats
    fee_sats
    static_deposit_address
  }
}
"#;
