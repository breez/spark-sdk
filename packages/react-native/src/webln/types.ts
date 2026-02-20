/**
 * WebLN types for React Native
 * @module webln/types
 */

/**
 * Callback for enable requests.
 * Called when a website requests WebLN access.
 *
 * @param domain - The domain requesting access
 * @returns Promise resolving to `true` to allow access, `false` to deny
 */
export type OnEnableRequest = (domain: string) => Promise<boolean>;

/**
 * Callback for payment requests.
 * Called when a website requests to send a payment.
 *
 * @param invoice - The BOLT11 invoice
 * @param amountSats - The amount in satoshis
 * @returns Promise resolving to `true` to approve payment, `false` to reject
 */
export type OnPaymentRequest = (
  invoice: string,
  amountSats: number
) => Promise<boolean>;

/**
 * Callback for LNURL requests.
 * Called when a website initiates an LNURL flow.
 *
 * @param request - The LNURL request details
 * @returns Promise resolving to user's response with approval and optional amount/comment
 */
export type OnLnurlRequest = (request: LnurlRequest) => Promise<LnurlUserResponse>;

/**
 * Represents the type of LNURL request.
 * - `pay`: LNURL-pay request
 * - `withdraw`: LNURL-withdraw request
 * - `auth`: LNURL-auth request
 */
export type LnurlType = 'pay' | 'withdraw' | 'auth';

/**
 * Represents an LNURL request that needs user approval.
 * Passed to the `onLnurlRequest` callback.
 */
export interface LnurlRequest {
  /** The type of LNURL request */
  type: LnurlType;
  /** The domain of the LNURL service */
  domain: string;
  /** Minimum amount in sats (for pay/withdraw requests) */
  minAmountSats?: number;
  /** Maximum amount in sats (for pay/withdraw requests) */
  maxAmountSats?: number;
  /** LNURL metadata JSON string (for pay requests) */
  metadata?: string;
  /** Default description (for withdraw requests) */
  defaultDescription?: string;
}

/**
 * Represents the user's response to an LNURL request.
 * Returned from the `onLnurlRequest` callback.
 */
export interface LnurlUserResponse {
  /** Whether the user approved the request */
  approved: boolean;
  /** Amount in sats selected by the user (for pay/withdraw) */
  amountSats?: number;
  /** Optional comment (for LNURL-pay) */
  comment?: string;
}

/** WebLN error codes returned to JavaScript. */
export const WebLnErrorCode = {
  userRejected: 'USER_REJECTED',
  providerNotEnabled: 'PROVIDER_NOT_ENABLED',
  insufficientFunds: 'INSUFFICIENT_FUNDS',
  invalidParams: 'INVALID_PARAMS',
  unsupportedMethod: 'UNSUPPORTED_METHOD',
  internalError: 'INTERNAL_ERROR',
} as const;

export type WebLnErrorCodeType =
  (typeof WebLnErrorCode)[keyof typeof WebLnErrorCode];

/** WebLn request from JavaScript */
export interface WebLnRequest {
  id: string;
  method: string;
  params: Record<string, unknown>;
}

/** WebLn response to JavaScript */
export interface WebLnResponse {
  id: string;
  success: boolean;
  result?: Record<string, unknown>;
  error?: string;
}
