# Token allowances

A token allowance lets another wallet pull tokens from your wallet, within limits you set. The wallet that grants the allowance is the owner. The wallet that pulls is the spender.

An allowance names the spender by its identity public key, and sets:

- the token
- a limit on each payment and a limit on all payments together
- an expiry
- an optional list of recipients the spender may pay

The owner can revoke an allowance at any time. An allowance doesn't reserve funds: a pull fails if the owner's balance is too low. In the owner's history, a pull shows up as an ordinary outgoing payment.

Granting and revoking need the device clock to be within about a minute of the network's time.

## Granting an allowance

{{#tabs token_allowances:create-token-allowance}}

Both limits must be given: to leave one open, set it to {{#enum TokenAllowanceLimit::Unlimited}}.

Only one allowance can be active for the same spender and token. Granting another returns {{#enum TokenAllowanceErrorReason::AlreadyActive}}: list your allowances to find the active one. To change the limits, revoke the allowance and grant a new one.

## Listing allowances

Use {{#enum TokenAllowanceRole::Owner}} for allowances your wallet granted and {{#enum TokenAllowanceRole::Spender}} for allowances granted to it.

{{#tabs token_allowances:list-token-allowances}}

The spent amount counts a pull as soon as it is sent, including pulls that later fail, so it can be higher than what was actually pulled.

## Revoking an allowance

{{#tabs token_allowances:revoke-token-allowance}}

A revocation takes effect within seconds. A pull that hasn't completed can still fail after a revocation, so treat a pull as paid only once it completes.

## Pulling a payment

A spender pulls in two steps. {{#name prepare_pull_payment}} finds the allowance, checks its limits and builds the transfer from the payer's tokens. {{#name pull_payment}} signs it and sends it.

{{#tabs token_allowances:pull-payment}}

Leave a receiver's public key unset to pull into your own wallet. One pull can pay several receivers. When the allowance has a recipient list, every receiver must be on it, your own wallet included. The per-payment limit applies to the total.

### Retrying

A pull counts as paid once {{#name pull_payment}} returns it with the {{#name status}} {{#enum PaymentStatus::Completed}}. A {{#enum PaymentStatus::Pending}} pull is accepted but can still fail: call {{#name pull_payment}} again with the same prepare response to refresh it.

If {{#name pull_payment}} fails with {{#enum SdkError::SparkError}} or {{#enum SdkError::NetworkError}}, call it again with the same prepare response; the operators recognize a pull they've already seen, so this never charges the payer twice. Keep calling until the pull completes, fails with {{#enum SdkError::TokenAllowance}}, or 10 minutes have passed since its {{#name expiry_time}}, because a pull the operators accepted can still complete for a few minutes after that time. Prepare a new pull only after that.

{{#enum TokenAllowanceErrorReason::PreparedPullStale}} means the operators never accepted the pull and never will: the tokens it was prepared with were spent by another payment, or it wasn't sent before its window closed. Prepare a new pull right away. Don't call {{#name pull_payment}} twice at the same time with the same prepare response, because the second call can report a pull the first one is still sending as stale.

## Errors

Token allowance errors return {{#enum SdkError::TokenAllowance}} with a {{#name reason}} that says why the request was refused, for example {{#enum TokenAllowanceErrorReason::OverPerPaymentLimit}}, {{#enum TokenAllowanceErrorReason::OverTotalLimit}}, {{#enum TokenAllowanceErrorReason::RecipientNotAllowed}}, {{#enum TokenAllowanceErrorReason::Revoked}} or {{#enum TokenAllowanceErrorReason::PayerInsufficientFunds}}. {{#enum TokenAllowanceErrorReason::PreparedPullStale}} means the pull can't go through anymore (see [Retrying](#retrying)). {{#enum TokenAllowanceErrorReason::NotEnabled}} means token allowances aren't available on this network.

In JavaScript, errors arrive as text: the reason's name is part of the error message.

## Signing pulls on the client

To have the spender's key approve each pull on a separate device or signing service, see [Client signing](client_signing.md#pulls).
