// GraphQL query strings
use super::fragments::{
    COOP_EXIT_FEE_ESTIMATE_FIELDS, CURRENCY_AMOUNT_FIELDS, FEE_ESTIMATE_FIELDS,
    LIGHTNING_INVOICE_FIELDS, REQUEST_BASE_FIELDS, SWAP_LEAF_FIELDS, TRANSFER_FIELDS,
    with_fragments,
};

/// LightningSendFeeEstimate query
const LIGHTNING_SEND_FEE_ESTIMATE: &str = r#"
query LightningSendFeeEstimate(
  $encoded_invoice: String!
  amount_sats: Long
) {
  lightning_send_fee_estimate(input: {
    encoded_invoice: $encoded_invoice,
    amount_sats: $amount_sats
  }) {
    fee_estimate {
      ...FeeEstimateFields
    }
  }
}
"#;

pub fn lightning_send_fee_estimate() -> String {
    with_fragments(
        LIGHTNING_SEND_FEE_ESTIMATE,
        &[FEE_ESTIMATE_FIELDS, CURRENCY_AMOUNT_FIELDS],
    )
}

/// CoopExitFeeEstimate query
const COOP_EXIT_FEE_ESTIMATE: &str = r#"
query CoopExitFeeEstimate(
  $leaf_external_ids: [UUID!]!
  $withdrawal_address: String!
) {
  coop_exit_fee_estimates(input: {
    leaf_external_ids: $leaf_external_ids,
    withdrawal_address: $withdrawal_address
  }) {
    speed_fast {
      ...CoopExitFeeEstimateFields
    }
    speed_medium {
      ...CoopExitFeeEstimateFields
    }
    speed_slow {
      ...CoopExitFeeEstimateFields
    }
  }
}
"#;

pub fn coop_exit_fee_estimate() -> String {
    with_fragments(
        COOP_EXIT_FEE_ESTIMATE,
        &[COOP_EXIT_FEE_ESTIMATE_FIELDS, CURRENCY_AMOUNT_FIELDS],
    )
}

/// LeavesSwapFeeEstimate query
const LEAVES_SWAP_FEE_ESTIMATE: &str = r#"
query LeavesSwapFeeEstimate(
  $total_amount_sats: Int!
) {
  leaves_swap_fee_estimate(input: {
    total_amount_sats: $total_amount_sats
  }) {
    fee_estimate {
      ...FeeEstimateFields
    }
  }
}
"#;

pub fn leaves_swap_fee_estimate() -> String {
    with_fragments(
        LEAVES_SWAP_FEE_ESTIMATE,
        &[FEE_ESTIMATE_FIELDS, CURRENCY_AMOUNT_FIELDS],
    )
}

/// GetClaimDepositQuote query
const GET_CLAIM_DEPOSIT_QUOTE: &str = r#"
query StaticDepositQuote(
  $transaction_id: String!
  $output_index: Int!
  $network: BitcoinNetwork!
) {
  static_deposit_quote(input: {
    transaction_id: $transaction_id,
    output_index: $output_index,
    network: $network
  }) {
    transaction_id
    output_index
    network
    credit_amount_sats
    signature
  }
}
"#;

pub fn get_claim_deposit_quote() -> String {
    GET_CLAIM_DEPOSIT_QUOTE.to_string()
}

/// Transfer query
const GET_TRANSFER: &str = r#"
query Transfer($transfer_spark_id: UUID!) {
  transfer(transfer_spark_id: $transfer_spark_id) {
    ...TransferFields
  }
}
"#;

pub fn get_transfer() -> String {
    with_fragments(GET_TRANSFER, &[TRANSFER_FIELDS, CURRENCY_AMOUNT_FIELDS])
}

/// UserRequest query - used for different types of user requests
const USER_REQUEST: &str = r#"
query UserRequest($request_id: ID!) {
  user_request(request_id: $request_id) {
    ... on LightningReceiveRequest {
      ...RequestBaseFields
      invoice {
        ...LightningInvoiceFields
      }
      transfer {
        ...TransferFields
      }
      payment_preimage
    }
    ... on LightningSendRequest {
      ...RequestBaseFields
      encoded_invoice
      fee {
        ...CurrencyAmountFields
      }
      idempotency_key
      transfer {
        ...TransferFields
      }
      payment_preimage
    }
    ... on CoopExitRequest {
      ...RequestBaseFields
      fee {
        ...CurrencyAmountFields
      }
      l1_broadcast_fee {
        ...CurrencyAmountFields
      }
      expires_at
      raw_connector_transaction
      raw_coop_exit_transaction
      coop_exit_txid
      transfer {
        ...TransferFields
      }
    }
    ... on LeavesSwapRequest {
      ...RequestBaseFields
      total_amount {
        ...CurrencyAmountFields
      }
      target_amount {
        ...CurrencyAmountFields
      }
      fee {
        ...CurrencyAmountFields
      }
      inbound_transfer {
        ...TransferFields
      }
      outbound_transfer {
        ...TransferFields
      }
      expires_at
      swap_leaves {
        ...SwapLeafFields
      }
    }
  }
}
"#;

pub fn user_request() -> String {
    with_fragments(
        USER_REQUEST,
        &[
            LIGHTNING_INVOICE_FIELDS,
            SWAP_LEAF_FIELDS,
            REQUEST_BASE_FIELDS,
            TRANSFER_FIELDS,
            CURRENCY_AMOUNT_FIELDS,
        ],
    )
}
