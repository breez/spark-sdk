import type { Payment, PayX402Request, PayX402Response } from '../breez_sdk_spark_wasm.js'

/** The part of the SDK's `X402` object the wrapper uses. */
export interface X402Payer {
  pay(request: PayX402Request): Promise<PayX402Response>
}

export interface X402FetchOptions {
  /** The most to spend on one request in sats over Lightning, routing fee included. */
  maxAmountSats: number
  /** The most to spend across all requests made with this fetch. Unset means no limit. */
  budgetSats?: number
  /**
   * Hosts to pay, as in `URL.host`, checked against the URL that answered with
   * the challenge. Unset pays any host.
   */
  allowedHosts?: string[]
  /** How long each payment attempt waits to complete. Defaults to 30 seconds. */
  completionTimeoutSecs?: number
  /** Times to pay the same challenge again while its payment is pending. Defaults to 3. */
  pendingRetries?: number
  /**
   * Times to send the paid request again while the server answers 402 without
   * a new challenge, meaning the payment may still settle. Defaults to 5.
   */
  settleRetries?: number
  /** How long to wait before each of those, in milliseconds. Defaults to 2000. */
  settleRetryDelayMs?: number
}

/** Thrown when a payment is still in flight after the configured retries. */
export class X402PaymentPendingError extends Error {
  readonly payment: Payment
}

/**
 * Wraps `fetch` so that a 402 response carrying an x402 Lightning challenge is
 * paid and the request is sent again with the payment. Responses that aren't
 * an x402 challenge, are for a host not in `allowedHosts`, or would exceed the
 * budget are returned unchanged.
 */
export function withX402(
  fetchFn: typeof fetch,
  x402: X402Payer,
  options: X402FetchOptions
): typeof fetch
