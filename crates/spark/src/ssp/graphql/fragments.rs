// GraphQL fragment definitions

/// Fragment for currency amount fields
pub const CURRENCY_AMOUNT_FIELDS: &str = r#"
fragment CurrencyAmountFields on CurrencyAmount {
  original_value
  original_unit
  preferred_currency_unit
  preferred_currency_value_rounded
  preferred_currency_value_approx
}
"#;

/// Fragment for transfer fields
pub const TRANSFER_FIELDS: &str = r#"
fragment TransferFields on Transfer {
  total_amount {
    ...CurrencyAmountFields
  }
  spark_id
}
"#;

/// Fragment for common request fields
pub const REQUEST_BASE_FIELDS: &str = r#"
fragment RequestBaseFields on UserRequest {
  id
  created_at
  updated_at
  network
  status
}
"#;

/// Fragment for lightning invoice fields
pub const LIGHTNING_INVOICE_FIELDS: &str = r#"
fragment LightningInvoiceFields on LightningInvoice {
  encoded_invoice
  bitcoin_network
  payment_hash
  amount {
    ...CurrencyAmountFields
  }
  created_at
  expires_at
  memo
}
"#;

/// Fragment for lightning receive request fields
pub const LIGHTNING_RECEIVE_REQUEST_FIELDS: &str = r#"
fragment LightningReceiveRequestFields on LightningReceiveRequest {
  ...RequestBaseFields
  invoice {
    ...LightningInvoiceFields
  }
  transfer {
    ...TransferFields
  }
  payment_preimage
}
"#;

/// Fragment for lightning send request fields
pub const LIGHTNING_SEND_REQUEST_FIELDS: &str = r#"
fragment LightningSendRequestFields on LightningSendRequest {
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
"#;

/// Fragment for coop exit request fields
pub const COOP_EXIT_REQUEST_FIELDS: &str = r#"
fragment CoopExitRequestFields on CoopExitRequest {
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
"#;

/// Fragment for swap leaf fields
pub const SWAP_LEAF_FIELDS: &str = r#"
fragment SwapLeafFields on SwapLeaf {
  leaf_id
  raw_unsigned_refund_transaction
  adaptor_signed_signature
}
"#;

/// Fragment for leaves swap request fields
pub const LEAVES_SWAP_REQUEST_FIELDS: &str = r#"
fragment LeavesSwapRequestFields on LeavesSwapRequest {
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
"#;

/// Fragment for fee estimate fields
pub const FEE_ESTIMATE_FIELDS: &str = r#"
fragment FeeEstimateFields on CurrencyAmount {
  ...CurrencyAmountFields
}
"#;

/// Fragment for coop exit fee estimate fields
pub const COOP_EXIT_FEE_ESTIMATE_FIELDS: &str = r#"
fragment CoopExitFeeEstimateFields on CoopExitFeeEstimate {
  user_fee {
    ...CurrencyAmountFields
  }
  l1_broadcast_fee {
    ...CurrencyAmountFields
  }
}
"#;

/// Create a full GraphQL operation with fragments
pub fn with_fragments(query: &str, fragments: &[&str]) -> String {
    let mut result = String::from(query);
    for fragment in fragments {
        result.push_str(fragment);
    }
    result
}
