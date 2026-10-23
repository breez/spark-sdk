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
  /** The server's public origin, such as `https://api.example.com`. The URL a payment commits to is built from it. */
  publicOrigin: string
  /** The price in sats over Lightning, or a function computing it from the request. */
  priceSats: number | ((request: Request) => number | Promise<number>)
  /** Request headers the payment also commits to, such as `authorization`. */
  boundHeaders?: string[]
  /** What is being paid for, shown to the payer. */
  description?: string
  /** The resource's MIME type. */
  mimeType?: string
  /** How long a challenge can be paid, in seconds. Defaults to 300. */
  maxTimeoutSecs?: number
}

export type X402ChargeResult =
  | { paid: true; paymentResponse: string }
  | { paid: false; response: Response }

/**
 * Charges for a request: settles the payment it carries, or returns the 402
 * (or, for a payment that can't be read, 400) response to send instead. Serve
 * the request only when `paid` is true, with a `PAYMENT-RESPONSE` header set
 * to `paymentResponse` and `Access-Control-Expose-Headers` passed through
 * `exposeX402Headers`. Don't call it for CORS preflight (`OPTIONS`) requests.
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
 * Hono middleware charging for every route it is mounted on. CORS preflight
 * requests pass through to the app's CORS handling.
 */
export function x402Hono(
  x402: X402Charger,
  options: X402ChargeOptions
): (c: any, next: () => Promise<void>) => Promise<Response | void>

/**
 * Express middleware charging for every route it is mounted on. The payment
 * commits to the raw body bytes, so mount `express.raw({ type: '*\/*' })`
 * before it on routes with a body. CORS preflight requests pass through to
 * the app's CORS handling.
 */
export function x402Express(
  x402: X402Charger,
  options: X402ChargeOptions
): (req: any, res: any, next: (err?: unknown) => void) => void
