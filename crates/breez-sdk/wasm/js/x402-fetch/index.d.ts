import type { Payment, PayX402Request, PayX402Response } from '../breez_sdk_spark_wasm.js'

/** The part of the SDK's `X402` object the wrapper uses. */
export interface X402Payer {
  pay(request: PayX402Request): Promise<PayX402Response>
}

export interface X402FetchOptions {
  /**
   * The most to spend on one request in whole sats over Lightning, routing fee
   * included. A challenge that would cost more is refused with an error.
   */
  maxAmountSats: number
  /**
   * The most to spend across all requests made with this fetch, in whole sats.
   * A payment that is still pending, or fails after it may have been sent,
   * counts too. Once it is spent, challenges are returned unpaid. A challenge
   * that costs more than what remains is refused with an error. Unset means
   * no limit.
   */
  budgetSats?: number
  /**
   * Hosts to pay, as in `URL.host`, checked against the URL that answered with
   * the challenge. Unset pays any host.
   */
  allowedHosts?: string[]
  /**
   * Pay challenges from plain `http://` URLs on other machines too. The proof
   * of payment is a bearer token, which anyone on the network path could then
   * read and spend. Defaults to false, which pays over `http://` only on
   * `localhost`, `127.0.0.0/8` and `[::1]`, and returns other such challenges
   * unpaid.
   */
  allowHttp?: boolean
  /** How long each payment attempt waits to complete. Defaults to 30 seconds. */
  completionTimeoutSecs?: number
  /**
   * Times to wait on the payment again while it is pending, each up to
   * `completionTimeoutSecs`. Defaults to 3.
   */
  pendingRetries?: number
  /**
   * Times to send the paid request again while the server answers 402 saying
   * the payment may still settle (`settlement_pending`). Defaults to 5.
   */
  settleRetries?: number
  /** How long to wait before each of those, in milliseconds. Defaults to 2000. */
  settleRetryDelayMs?: number
}

/**
 * Thrown when a payment may have been made but the request hasn't been served
 * with it: the payment is still pending after the configured retries, the
 * server hasn't confirmed it or answered with an error such as a 500 without
 * settling it, or something failed after it may have been sent (see `cause`).
 * Nothing is paid again. To resume, pay `payRequest` again if
 * `paymentSignature` is unset, which picks up the same payment, then send the
 * request to `url` with the signature in the `PAYMENT-SIGNATURE` header. A
 * server accepts the payment only until shortly after its invoice expires (60
 * seconds by default).
 *
 * `payRequest`, `payment` and `paymentSignature` hold the request's headers
 * and the proof of payment, so they are left out when the error is logged or
 * serialized.
 */
export class X402PaymentPendingError extends Error {
  /** The URL the payment is for. */
  readonly url: string
  readonly payRequest: PayX402Request
  /** Unset when the payment's outcome is unknown. */
  readonly payment?: Payment
  /** Set once the payment completed. */
  readonly paymentSignature?: string
  /** The server's error answer to the paid request, with its body unread. */
  readonly response?: Response
}

/**
 * Thrown when the server answers the paid request with a 402 that doesn't say
 * the payment may still settle (a new challenge, or a refusal in
 * `PAYMENT-RESPONSE`), or with a 400 because it couldn't read the payment. A
 * 400 whose `PAYMENT-RESPONSE` reports the payment settled is the resource's
 * own answer, and is returned instead. The payment was made and counts against
 * the budget, and isn't made again.
 *
 * `payRequest`, `payment` and `paymentSignature` hold the request's headers
 * and the proof of payment, so they are left out when the error is logged or
 * serialized.
 */
export class X402PaymentRefusedError extends Error {
  /** The URL the payment is for. */
  readonly url: string
  readonly payRequest: PayX402Request
  readonly payment: Payment
  readonly paymentSignature: string
  /** The `errorReason` in the response's `PAYMENT-RESPONSE`, when it has one. */
  readonly errorReason?: string
  /** The server's 402 or 400, with its body unread. */
  readonly response: Response
}

/**
 * Wraps `fetch` so that a 402 response carrying an x402 Lightning challenge is
 * paid and the request is sent again with the payment. Responses that aren't
 * an x402 challenge, offer no Lightning payment on this network, are for a
 * host not in `allowedHosts` or a plain `http://` URL `allowHttp` excludes, or
 * arrive once the budget is spent are returned unchanged.
 *
 * The payment commits to the request's method, URL, body and the headers the
 * server binds, as sent. fetch adds `accept: *\/*` when the request has none,
 * and the payment does too. Set any other header the server binds on the
 * request itself, including ones the runtime adds, such as Node.js's
 * `accept-language`.
 *
 * A request that already carries a `PAYMENT-SIGNATURE` header is sent as is,
 * and a challenge in answer to it is returned unpaid, so resuming a payment
 * through this fetch never pays twice.
 *
 * A challenge reached through a redirect is paid only for a GET or HEAD that
 * stayed on the same origin, and is returned unchanged otherwise: request its
 * URL directly to pay it. The paid request doesn't follow redirects, so the
 * payment goes only to the server that charged, and a redirect it answers with
 * is returned as the response.
 *
 * Options in `init` that aren't part of a `Request`, such as undici's
 * `dispatcher` or Next.js's `next`, are passed to `fetchFn` with both requests.
 */
export function withX402(
  fetchFn: typeof fetch,
  x402: X402Payer,
  options: X402FetchOptions
): typeof fetch
