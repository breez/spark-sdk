/**
 * WebLN type definitions for Breez Spark SDK
 * Based on WebLN spec: https://www.webln.guide/
 */

// Request/Response types for native bridge communication
export interface WebLnRequest {
  id: string;
  method: string;
  params: Record<string, unknown>;
}

export interface WebLnResponse {
  id: string;
  success: boolean;
  result?: unknown;
  error?: string;
}

// WebLN spec types
export interface GetInfoResponse {
  node: {
    pubkey: string;
    alias: string;
  };
  methods: string[];
}

export interface SendPaymentResponse {
  preimage: string;
}

export interface KeysendArgs {
  destination: string;
  amount: string | number;
  customRecords?: Record<string, string>;
}

export interface RequestInvoiceArgs {
  amount?: string | number;
  defaultAmount?: string | number;
  minimumAmount?: string | number;
  maximumAmount?: string | number;
  defaultMemo?: string;
}

export interface RequestInvoiceResponse {
  paymentRequest: string;
}

export interface SignMessageResponse {
  message: string;
  signature: string;
}

// LNURL types
export interface LnurlResponse {
  status: 'OK' | 'ERROR';
  reason?: string;
}

// Error types
export class WebLnError extends Error {
  constructor(
    message: string,
    public code: string
  ) {
    super(message);
    this.name = 'WebLnError';
  }
}

export const WebLnErrorCodes = {
  USER_REJECTED: 'USER_REJECTED',
  PROVIDER_NOT_ENABLED: 'PROVIDER_NOT_ENABLED',
  UNSUPPORTED_METHOD: 'UNSUPPORTED_METHOD',
  INSUFFICIENT_FUNDS: 'INSUFFICIENT_FUNDS',
  INVALID_PARAMS: 'INVALID_PARAMS',
  INTERNAL_ERROR: 'INTERNAL_ERROR',
} as const;

// Window augmentation for native bridges
declare global {
  interface Window {
    webln?: WebLnProvider;
    BreezSparkWebLn?: {
      postMessage: (message: string) => void;
    };
    webkit?: {
      messageHandlers?: {
        BreezSparkWebLn?: {
          postMessage: (message: string) => void;
        };
      };
    };
    ReactNativeWebView?: {
      postMessage: (message: string) => void;
    };
    flutter_inappwebview?: {
      callHandler: (name: string, message: string) => void;
    };
  }
}

// WebLN Provider interface
export interface WebLnProvider {
  enable(): Promise<void>;
  getInfo(): Promise<GetInfoResponse>;
  sendPayment(paymentRequest: string): Promise<SendPaymentResponse>;
  keysend(args: KeysendArgs): Promise<SendPaymentResponse>;
  makeInvoice(args?: RequestInvoiceArgs): Promise<RequestInvoiceResponse>;
  signMessage(message: string): Promise<SignMessageResponse>;
  verifyMessage(signature: string, message: string): Promise<void>;
  lnurl(lnurlString: string): Promise<LnurlResponse>;
}
