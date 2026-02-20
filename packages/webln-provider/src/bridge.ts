/**
 * Platform-agnostic bridge for native communication
 */

import type { WebLnRequest, WebLnResponse } from './types';

export type BridgeType = 'android_ios' | 'react_native' | 'flutter' | 'none';

/**
 * Detects which native bridge is available
 */
export function detectBridge(): BridgeType {
  if (typeof window === 'undefined') {
    return 'none';
  }

  // Android (addJavascriptInterface) or iOS WKWebView (addScriptMessageHandler)
  if (
    window.BreezSparkWebLn?.postMessage ||
    window.webkit?.messageHandlers?.BreezSparkWebLn?.postMessage
  ) {
    return 'android_ios';
  }

  if (window.ReactNativeWebView?.postMessage) {
    return 'react_native';
  }

  if (window.flutter_inappwebview?.callHandler) {
    return 'flutter';
  }

  return 'none';
}

/**
 * Sends a message to the native bridge
 */
export function postMessage(request: WebLnRequest): void {
  const message = JSON.stringify(request);
  const bridgeType = detectBridge();

  switch (bridgeType) {
    case 'android_ios':
      if (window.BreezSparkWebLn?.postMessage) {
        window.BreezSparkWebLn.postMessage(message);
      } else {
        window.webkit!.messageHandlers!.BreezSparkWebLn!.postMessage(message);
      }
      break;
    case 'react_native':
      window.ReactNativeWebView!.postMessage(message);
      break;
    case 'flutter':
      window.flutter_inappwebview!.callHandler('BreezSparkWebLn', message);
      break;
    case 'none':
      throw new Error('No native bridge available');
  }
}

/**
 * Generates a unique request ID
 */
export function generateRequestId(): string {
  return `${Date.now()}-${Math.random().toString(36).slice(2, 11)}`;
}

/**
 * Pending request tracker
 */
export interface PendingRequest<T = unknown> {
  resolve: (value: T) => void;
  reject: (error: Error) => void;
  timeoutId?: ReturnType<typeof setTimeout>;
}

export class RequestTracker {
  private pending = new Map<string, PendingRequest>();
  private defaultTimeout = 60000; // 60 seconds

  /**
   * Creates a tracked request that returns a promise
   */
  create<T>(id: string, timeout?: number): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const timeoutMs = timeout ?? this.defaultTimeout;

      const timeoutId = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`Request ${id} timed out after ${timeoutMs}ms`));
      }, timeoutMs);

      this.pending.set(id, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timeoutId,
      });
    });
  }

  /**
   * Resolves a pending request with a response
   */
  resolve(response: WebLnResponse): boolean {
    const request = this.pending.get(response.id);
    if (!request) {
      return false;
    }

    this.pending.delete(response.id);

    if (request.timeoutId) {
      clearTimeout(request.timeoutId);
    }

    if (response.success) {
      request.resolve(response.result);
    } else {
      request.reject(new Error(response.error ?? 'Unknown error'));
    }

    return true;
  }

  /**
   * Clears all pending requests
   */
  clear(): void {
    for (const [id, request] of this.pending) {
      if (request.timeoutId) {
        clearTimeout(request.timeoutId);
      }
      request.reject(new Error('Request cancelled'));
    }
    this.pending.clear();
  }
}
