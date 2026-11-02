import type {
  CreateX402ChallengeRequest,
  CreateX402ChallengeResponse,
  SettleX402Request,
  SettleX402Response
} from '../breez_sdk_spark_wasm.js'

/** The part of the SDK's `X402` object the middleware uses. */
export interface X402Charger {
  createChallenge(request: CreateX402ChallengeRequest): Promise<CreateX402ChallengeResponse>
  settle(request: SettleX402Request): Promise<SettleX402Response>
}

export interface X402ChargeOptions {
  /**
   * The server's public origin, such as `https://api.example.com`. The URL a
   * payment commits to is built from it. An origin with a path throws a
   * `TypeError`.
   */
  publicOrigin: string
  /**
   * The price in sats over Lightning, or a function computing it from the
   * request. A payment quoting a price other than the current one is refused.
   */
  priceSats: number | ((request: Request) => number | Promise<number>)
  /** Request headers the payment also commits to, such as `authorization`. */
  boundHeaders?: string[]
  /** What is being paid for, shown to the payer. */
  description?: string
  /** The resource's MIME type. */
  mimeType?: string
  /** How long a challenge can be paid, in seconds. Defaults to 300. */
  maxTimeoutSecs?: number
  /**
   * The largest request body to read, in bytes. A request whose body is
   * larger is answered with 413 and isn't charged. Reading stops as soon as
   * the body passes the limit. Defaults to 1 MiB (1048576).
   */
  maxBodyBytes?: number
}

export type X402ChargeResult =
  | { paid: true; paymentResponse: string }
  | { paid: false; response: Response }

/**
 * Charges for a request: settles the payment it carries, or returns the
 * response to send instead. That is a 402 with a new challenge, a 402 without
 * one while the payment may still settle, a 400 for a payment that can't be
 * read or a URL that can't be paid for, a 413 for a body over `maxBodyBytes`,
 * a 500 when the payment couldn't be settled for a fault on this server, or a
 * 204 for a CORS preflight, which is never charged. The 402, 400, 413 and 500
 * responses carry `Cache-Control: no-store`.
 *
 * Serve the request only when `paid` is true, with a `PAYMENT-RESPONSE` header
 * set to `paymentResponse`, `Access-Control-Expose-Headers` passed through
 * `exposeX402Headers`, and `Cache-Control` marked `private` (in place of any
 * `public`), so shared caches don't serve the paid response to others. Call it
 * before anything reads the request body. The 204 for a preflight carries no
 * CORS headers, so answer preflights with CORS handling before calling it.
 */
export function requirePayment(
  request: Request,
  x402: X402Charger,
  options: X402ChargeOptions
): Promise<X402ChargeResult>

/**
 * Adds `PAYMENT-REQUIRED` and `PAYMENT-RESPONSE` to an
 * `Access-Control-Expose-Headers` value, so cross-origin browser code can read
 * them.
 */
export function exposeX402Headers(existing?: string | null): string

/**
 * Hono middleware charging for every request that reaches it, whatever its
 * method, as `requirePayment` does. Paid responses get `private` merged into
 * their `Cache-Control`, in place of any `public`. Mount it before middleware
 * that reads the request body, and after CORS handling: it answers a CORS
 * preflight with 204 without charging, but adds no CORS headers.
 */
export function x402Hono(
  x402: X402Charger,
  options: X402ChargeOptions
): (c: any, next: () => Promise<void>) => Promise<Response | void>

/**
 * Express middleware charging for every request that reaches it, whatever its
 * method, as `requirePayment` does. Paid responses get `private` merged into
 * their `Cache-Control`, in place of any `public`.
 *
 * The payment commits to the body bytes as sent. Mounted before any body
 * parser, it reads them itself and sets `req.body` to them, and later body
 * parsers skip the request. Behind a parser, have it keep the bytes in
 * `req.rawBody`, as with
 * `express.json({ verify: (req, res, buf) => { req.rawBody = buf }, inflate: false })`,
 * or use `express.raw({ type: () => true, inflate: false })`. A body that was
 * parsed without them, or inflated, is refused with an error.
 *
 * Mount CORS handling before it: it answers a CORS preflight with 204 without
 * charging, but adds no CORS headers. A request target that URL parsing would
 * rewrite is answered with 400.
 */
export function x402Express(
  x402: X402Charger,
  options: X402ChargeOptions
): (req: any, res: any, next: (err?: unknown) => void) => void
