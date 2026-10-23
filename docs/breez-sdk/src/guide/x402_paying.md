# Paying with x402

The SDK pays [x402](x402.md) challenges over Lightning, for both HTTP requests and MCP tool calls. To charge for your own resources instead, see [Charging with x402](x402_charging.md).

## Paying for an HTTP request

A server that requires payment answers with status `402` and a `PAYMENT-REQUIRED` header. Pass that header to {{#name pay}} together with the request it answered. The challenge commits to the exact request, so describe it as it will be sent again: the method, the absolute URL, the headers and the raw body. Include headers your HTTP client adds on its own, such as `Accept`. A server can require them, and a challenge that doesn't match the request is refused.

Before anything is paid, the challenge is checked: it must be for this request, from the server's advertised payee, unexpired, and cost no more than {{#name max_amount}}. Give the most you'll spend over Lightning, routing fee included, as {{#enum X402PaymentAmount::Lightning}}. Once paid, send the request again with a `PAYMENT-SIGNATURE` header set to {{#name payment_signature}}.

{{#tabs x402:pay-http-request}}

## Paying for an MCP tool call

An MCP server returns the challenge in the `structuredContent` of a tool result, and as text in `content[0]`. Pass that JSON to {{#name pay}} together with the tool call's `params`. For a tool call, {{#name payment_signature}} is JSON: call the tool again with it under `params._meta["x402/payment"]`.

{{#tabs x402:pay-mcp-tool-call}}

## Pending payments

If the payment hasn't completed within {{#name completion_timeout_secs}} (30 seconds by default), {{#name pay}} returns {{#enum X402PaymentStatus::Pending}}. Paying the same challenge again picks up that payment instead of paying twice. Don't send the request again to retry: the server answers with a new challenge, and paying it would pay a second time.

## Paying from a token balance

To pay from a token balance such as USDB, set {{#name conversion_options}} and the tokens are converted to sats before paying. With [stable balance](stable_balance.md) active, this happens automatically when the sats balance is too low. {{#name max_amount}} limits the Lightning payment, while the conversion's cost is limited by its slippage.

## Refused challenges

A challenge that fails a check is refused with {{#enum SdkError::InvalidInput}}, and nothing is paid. The message starts with the x402 reason, such as `invalid_exact_lnbtc_invoice_expired` or `invalid_exact_lnbtc_request_mismatch`.
