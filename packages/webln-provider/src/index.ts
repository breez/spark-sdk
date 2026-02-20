/**
 * Breez Spark WebLN Provider
 *
 * This module is bundled and injected into WebViews to provide WebLN support.
 * When loaded, it automatically creates a provider instance and attaches it to window.webln.
 */

export * from './types';
export * from './bridge';
export { BreezSparkWebLnProvider } from './provider';

import { BreezSparkWebLnProvider } from './provider';
import type { WebLnResponse } from './types';

// Create the provider instance
const provider = new BreezSparkWebLnProvider();

// Attach to window.webln
if (typeof window !== 'undefined') {
  window.webln = provider;

  // Expose response handler globally for native bridges to call
  (window as unknown as Record<string, unknown>).__breezSparkWebLnHandleResponse =
    (response: WebLnResponse) => {
      provider._handleResponse(response);
    };
}

export default provider;
