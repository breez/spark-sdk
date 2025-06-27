// GraphQL mutation strings
use super::fragments::{
    COOP_EXIT_REQUEST_FIELDS, CURRENCY_AMOUNT_FIELDS, LEAVES_SWAP_REQUEST_FIELDS,
    LIGHTNING_INVOICE_FIELDS, LIGHTNING_RECEIVE_REQUEST_FIELDS, LIGHTNING_SEND_REQUEST_FIELDS,
    REQUEST_BASE_FIELDS, SWAP_LEAF_FIELDS, TRANSFER_FIELDS, with_fragments,
};

/// GetChallenge mutation
const GET_CHALLENGE: &str = r#"
mutation GetChallenge(
  $public_key: PublicKey!
) {
  get_challenge(input: {
    public_key: $public_key
  }) {
    protected_challenge
  }
}
"#;

pub fn get_challenge() -> String {
    GET_CHALLENGE.to_string()
}

/// VerifyChallenge mutation
const VERIFY_CHALLENGE: &str = r#"
mutation VerifyChallenge(
  $protected_challenge: String!
  $signature: String!
  $identity_public_key: PublicKey!
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

pub fn verify_challenge() -> String {
    VERIFY_CHALLENGE.to_string()
}

/// RequestLightningReceive mutation
const REQUEST_LIGHTNING_RECEIVE: &str = r#"
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
      ...LightningReceiveRequestFields
    }
  }
}
"#;

pub fn request_lightning_receive() -> String {
    with_fragments(
        REQUEST_LIGHTNING_RECEIVE,
        &[
            LIGHTNING_RECEIVE_REQUEST_FIELDS,
            LIGHTNING_INVOICE_FIELDS,
            REQUEST_BASE_FIELDS,
            TRANSFER_FIELDS,
            CURRENCY_AMOUNT_FIELDS,
        ],
    )
}

/// RequestLightningSend mutation
const REQUEST_LIGHTNING_SEND: &str = r#"
mutation RequestLightningSend(
  $encoded_invoice: String!
  $idempotency_key: String!
  $amount_sats: Long
) {
  request_lightning_send(input: {
    encoded_invoice: $encoded_invoice,
    idempotency_key: $idempotency_key,
    amount_sats: $amount_sats
  }) {
    request {
      ...LightningSendRequestFields
    }
  }
}
"#;

pub fn request_lightning_send() -> String {
    with_fragments(
        REQUEST_LIGHTNING_SEND,
        &[
            LIGHTNING_SEND_REQUEST_FIELDS,
            REQUEST_BASE_FIELDS,
            TRANSFER_FIELDS,
            CURRENCY_AMOUNT_FIELDS,
        ],
    )
}

/// RequestCoopExit mutation
const REQUEST_COOP_EXIT: &str = r#"
mutation RequestCoopExit(
  $leaf_external_ids: [UUID!]!
  $withdrawal_address: String!
  $idempotency_key: String!
  $exit_speed: ExitSpeed!
) {
  request_coop_exit(input: {
    leaf_external_ids: $leaf_external_ids, 
    withdrawal_address: $withdrawal_address, 
    idempotency_key: $idempotency_key,
    exit_speed: $exit_speed
  }) {
    request {
      ...CoopExitRequestFields
    }
  }
}
"#;

pub fn request_coop_exit() -> String {
    with_fragments(
        REQUEST_COOP_EXIT,
        &[
            COOP_EXIT_REQUEST_FIELDS,
            REQUEST_BASE_FIELDS,
            TRANSFER_FIELDS,
            CURRENCY_AMOUNT_FIELDS,
        ],
    )
}

/// CompleteCoopExit mutation
const COMPLETE_COOP_EXIT: &str = r#"
mutation CompleteCoopExit(
  $user_outbound_transfer_external_id: UUID!
  $coop_exit_request_id: ID!
) {
  complete_coop_exit(input: {
    user_outbound_transfer_external_id: $user_outbound_transfer_external_id,
    coop_exit_request_id: $coop_exit_request_id
  }) {
    request {
      ...CoopExitRequestFields
    }
  }
}
"#;

pub fn complete_coop_exit() -> String {
    with_fragments(
        COMPLETE_COOP_EXIT,
        &[
            COOP_EXIT_REQUEST_FIELDS,
            REQUEST_BASE_FIELDS,
            TRANSFER_FIELDS,
            CURRENCY_AMOUNT_FIELDS,
        ],
    )
}

/// RequestLeavesSwap mutation
const REQUEST_LEAVES_SWAP: &str = r#"
mutation RequestLeavesSwap(
  $adaptor_pubkey: PublicKey!
  $total_amount_sats: Int!
  $target_amount_sats: Int!
  $fee_sats: Int!
  $user_leaves: [UserLeafInput!]!
  $idempotency_key: String!
) {
  request_leaves_swap(input: {
    adaptor_pubkey: $adaptor_pubkey,
    total_amount_sats: $total_amount_sats,
    target_amount_sats: $target_amount_sats, 
    fee_sats: $fee_sats,
    user_leaves: $user_leaves,
    idempotency_key: $idempotency_key
  }) {
    request {
      ...LeavesSwapRequestFields
    }
  }
}
"#;

pub fn request_leaves_swap() -> String {
    with_fragments(
        REQUEST_LEAVES_SWAP,
        &[
            LEAVES_SWAP_REQUEST_FIELDS,
            SWAP_LEAF_FIELDS,
            REQUEST_BASE_FIELDS,
            TRANSFER_FIELDS,
            CURRENCY_AMOUNT_FIELDS,
        ],
    )
}

/// CompleteLeavesSwap mutation
const COMPLETE_LEAVES_SWAP: &str = r#"
mutation CompleteLeavesSwap(
  $adaptor_secret_key: String!
  $user_outbound_transfer_external_id: UUID!
  $leaves_swap_request_id: ID!
) {
  complete_leaves_swap(input: {
    adaptor_secret_key: $adaptor_secret_key,
    user_outbound_transfer_external_id: $user_outbound_transfer_external_id,
    leaves_swap_request_id: $leaves_swap_request_id
  }) {
    request {
      ...LeavesSwapRequestFields
    }
  }
}
"#;

pub fn complete_leaves_swap() -> String {
    with_fragments(
        COMPLETE_LEAVES_SWAP,
        &[
            LEAVES_SWAP_REQUEST_FIELDS,
            SWAP_LEAF_FIELDS,
            REQUEST_BASE_FIELDS,
            TRANSFER_FIELDS,
            CURRENCY_AMOUNT_FIELDS,
        ],
    )
}

/// ClaimStaticDeposit mutation
const CLAIM_STATIC_DEPOSIT: &str = r#"
mutation ClaimStaticDeposit(
  $transaction_id: String!
  $output_index: Int!
  $network: BitcoinNetwork!
  $request_type: ClaimStaticDepositRequestType!
  $credit_amount_sats: Int
  $deposit_secret_key: String!
  $signature: String!
  $quote_signature: String!
) {
  claim_static_deposit(input: {
    transaction_id: $transaction_id,
    output_index: $output_index,
    network: $network,
    request_type: $request_type,
    credit_amount_sats: $credit_amount_sats,
    deposit_secret_key: $deposit_secret_key,
    signature: $signature,
    quote_signature: $quote_signature
  }) {
    transfer_id
  }
}
"#;

pub fn claim_static_deposit() -> String {
    CLAIM_STATIC_DEPOSIT.to_string()
}
