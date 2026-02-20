/**
 * Breez Spark WebLN Provider
 *
 * Implements the WebLN spec for Breez Spark SDK.
 * This provider is injected into WebViews and communicates with
 * the native SDK through platform-specific bridges.
 */

import {
  type GetInfoResponse,
  type KeysendArgs,
  type LnurlResponse,
  type RequestInvoiceArgs,
  type RequestInvoiceResponse,
  type SendPaymentResponse,
  type SignMessageResponse,
  type WebLnProvider,
  type WebLnRequest,
  type WebLnResponse,
  WebLnError,
  WebLnErrorCodes,
} from './types';
import {
  detectBridge,
  generateRequestId,
  postMessage,
  RequestTracker,
} from './bridge';

export class BreezSparkWebLnProvider implements WebLnProvider {
  private enabled = false;
  private tracker = new RequestTracker();

  /**
   * Requests permission to use WebLN from the user.
   * Must be called before any other WebLN method.
   */
  async enable(): Promise<void> {
    if (this.enabled) {
      return;
    }

    const bridgeType = detectBridge();
    if (bridgeType === 'none') {
      throw new WebLnError(
        'WebLN provider not available - no native bridge detected',
        WebLnErrorCodes.INTERNAL_ERROR
      );
    }

    const id = generateRequestId();
    const request: WebLnRequest = {
      id,
      method: 'enable',
      params: {
        domain: window.location.origin,
      },
    };

    const promise = this.tracker.create<void>(id);
    postMessage(request);

    await promise;
    this.enabled = true;
  }

  /**
   * Returns information about the connected node.
   */
  async getInfo(): Promise<GetInfoResponse> {
    this.ensureEnabled();

    const id = generateRequestId();
    const request: WebLnRequest = {
      id,
      method: 'getInfo',
      params: {},
    };

    const promise = this.tracker.create<GetInfoResponse>(id);
    postMessage(request);

    return promise;
  }

  /**
   * Sends a payment for a BOLT11 invoice.
   */
  async sendPayment(paymentRequest: string): Promise<SendPaymentResponse> {
    this.ensureEnabled();

    if (!paymentRequest || typeof paymentRequest !== 'string') {
      throw new WebLnError(
        'Invalid payment request',
        WebLnErrorCodes.INVALID_PARAMS
      );
    }

    const id = generateRequestId();
    const request: WebLnRequest = {
      id,
      method: 'sendPayment',
      params: { paymentRequest },
    };

    const promise = this.tracker.create<SendPaymentResponse>(id);
    postMessage(request);

    return promise;
  }

  /**
   * Keysend is not supported by Spark.
   * Always throws UNSUPPORTED_METHOD error.
   */
  async keysend(_args: KeysendArgs): Promise<SendPaymentResponse> {
    throw new WebLnError(
      'Keysend is not supported by Spark',
      WebLnErrorCodes.UNSUPPORTED_METHOD
    );
  }

  /**
   * Creates a new invoice.
   */
  async makeInvoice(args?: RequestInvoiceArgs): Promise<RequestInvoiceResponse> {
    this.ensureEnabled();

    const id = generateRequestId();
    const request: WebLnRequest = {
      id,
      method: 'makeInvoice',
      params: (args ?? {}) as Record<string, unknown>,
    };

    const promise = this.tracker.create<RequestInvoiceResponse>(id);
    postMessage(request);

    return promise;
  }

  /**
   * Signs a message with the node's key.
   */
  async signMessage(message: string): Promise<SignMessageResponse> {
    this.ensureEnabled();

    if (!message || typeof message !== 'string') {
      throw new WebLnError('Invalid message', WebLnErrorCodes.INVALID_PARAMS);
    }

    const id = generateRequestId();
    const request: WebLnRequest = {
      id,
      method: 'signMessage',
      params: { message },
    };

    const promise = this.tracker.create<SignMessageResponse>(id);
    postMessage(request);

    return promise;
  }

  /**
   * Verifies a signed message.
   */
  async verifyMessage(signature: string, message: string): Promise<void> {
    this.ensureEnabled();

    if (!signature || typeof signature !== 'string') {
      throw new WebLnError('Invalid signature', WebLnErrorCodes.INVALID_PARAMS);
    }

    if (!message || typeof message !== 'string') {
      throw new WebLnError('Invalid message', WebLnErrorCodes.INVALID_PARAMS);
    }

    const id = generateRequestId();
    const request: WebLnRequest = {
      id,
      method: 'verifyMessage',
      params: { signature, message },
    };

    const promise = this.tracker.create<void>(id);
    postMessage(request);

    return promise;
  }

  /**
   * Handles LNURL operations (pay, withdraw, auth).
   */
  async lnurl(lnurlString: string): Promise<LnurlResponse> {
    this.ensureEnabled();

    if (!lnurlString || typeof lnurlString !== 'string') {
      throw new WebLnError('Invalid LNURL', WebLnErrorCodes.INVALID_PARAMS);
    }

    const id = generateRequestId();
    const request: WebLnRequest = {
      id,
      method: 'lnurl',
      params: { lnurl: lnurlString },
    };

    const promise = this.tracker.create<LnurlResponse>(id);
    postMessage(request);

    return promise;
  }

  /**
   * Handles responses from the native bridge.
   * Called by the native code to return results.
   */
  _handleResponse(response: WebLnResponse): void {
    this.tracker.resolve(response);
  }

  /**
   * Checks if the provider has been enabled.
   */
  private ensureEnabled(): void {
    if (!this.enabled) {
      throw new WebLnError(
        'Provider not enabled. Call enable() first.',
        WebLnErrorCodes.PROVIDER_NOT_ENABLED
      );
    }
  }
}
