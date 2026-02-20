import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import {
  detectBridge,
  postMessage,
  generateRequestId,
  RequestTracker,
} from './bridge';

describe('bridge', () => {
  describe('detectBridge', () => {
    beforeEach(() => {
      // Reset window properties
      delete (window as any).BreezSparkWebLn;
      delete (window as any).ReactNativeWebView;
      delete (window as any).flutter_inappwebview;
    });

    it('returns "none" when no bridge is available', () => {
      expect(detectBridge()).toBe('none');
    });

    it('detects Android/iOS bridge', () => {
      (window as any).BreezSparkWebLn = { postMessage: vi.fn() };
      expect(detectBridge()).toBe('android_ios');
    });

    it('detects React Native bridge', () => {
      (window as any).ReactNativeWebView = { postMessage: vi.fn() };
      expect(detectBridge()).toBe('react_native');
    });

    it('detects Flutter bridge', () => {
      (window as any).flutter_inappwebview = { callHandler: vi.fn() };
      expect(detectBridge()).toBe('flutter');
    });

    it('prioritizes Android/iOS over React Native', () => {
      (window as any).BreezSparkWebLn = { postMessage: vi.fn() };
      (window as any).ReactNativeWebView = { postMessage: vi.fn() };
      expect(detectBridge()).toBe('android_ios');
    });
  });

  describe('postMessage', () => {
    beforeEach(() => {
      delete (window as any).BreezSparkWebLn;
      delete (window as any).ReactNativeWebView;
      delete (window as any).flutter_inappwebview;
    });

    it('throws when no bridge is available', () => {
      const request = { id: '123', method: 'getInfo', params: {} };
      expect(() => postMessage(request)).toThrow('No native bridge available');
    });

    it('posts to Android/iOS bridge', () => {
      const mockPostMessage = vi.fn();
      (window as any).BreezSparkWebLn = { postMessage: mockPostMessage };

      const request = { id: '123', method: 'getInfo', params: {} };
      postMessage(request);

      expect(mockPostMessage).toHaveBeenCalledWith(JSON.stringify(request));
    });

    it('posts to React Native bridge', () => {
      const mockPostMessage = vi.fn();
      (window as any).ReactNativeWebView = { postMessage: mockPostMessage };

      const request = { id: '123', method: 'sendPayment', params: { paymentRequest: 'lnbc...' } };
      postMessage(request);

      expect(mockPostMessage).toHaveBeenCalledWith(JSON.stringify(request));
    });

    it('posts to Flutter bridge', () => {
      const mockCallHandler = vi.fn();
      (window as any).flutter_inappwebview = { callHandler: mockCallHandler };

      const request = { id: '123', method: 'enable', params: { domain: 'example.com' } };
      postMessage(request);

      expect(mockCallHandler).toHaveBeenCalledWith('BreezSparkWebLn', JSON.stringify(request));
    });
  });

  describe('generateRequestId', () => {
    it('generates unique IDs', () => {
      const ids = new Set<string>();
      for (let i = 0; i < 100; i++) {
        ids.add(generateRequestId());
      }
      expect(ids.size).toBe(100);
    });

    it('generates IDs with timestamp prefix', () => {
      const id = generateRequestId();
      const timestampPart = id.split('-')[0];
      expect(Number(timestampPart)).toBeLessThanOrEqual(Date.now());
      expect(Number(timestampPart)).toBeGreaterThan(Date.now() - 1000);
    });
  });

  describe('RequestTracker', () => {
    let tracker: RequestTracker;

    beforeEach(() => {
      tracker = new RequestTracker();
      vi.useFakeTimers();
    });

    afterEach(() => {
      vi.useRealTimers();
    });

    it('creates and resolves a request', async () => {
      const promise = tracker.create<{ data: string }>('req-1');

      tracker.resolve({
        id: 'req-1',
        success: true,
        result: { data: 'test' },
      });

      const result = await promise;
      expect(result).toEqual({ data: 'test' });
    });

    it('rejects a request on error', async () => {
      const promise = tracker.create('req-1');

      tracker.resolve({
        id: 'req-1',
        success: false,
        error: 'Something went wrong',
      });

      await expect(promise).rejects.toThrow('Something went wrong');
    });

    it('times out after default timeout', async () => {
      const promise = tracker.create('req-1');

      vi.advanceTimersByTime(60001);

      await expect(promise).rejects.toThrow('timed out');
    });

    it('times out after custom timeout', async () => {
      const promise = tracker.create('req-1', 5000);

      vi.advanceTimersByTime(5001);

      await expect(promise).rejects.toThrow('timed out');
    });

    it('clears timeout on successful resolve', async () => {
      const promise = tracker.create<void>('req-1');

      tracker.resolve({
        id: 'req-1',
        success: true,
      });

      await promise;

      // Advancing time should not cause issues
      vi.advanceTimersByTime(70000);
    });

    it('returns false when resolving unknown request', () => {
      const result = tracker.resolve({
        id: 'unknown',
        success: true,
      });

      expect(result).toBe(false);
    });

    it('clears all pending requests', async () => {
      const promise1 = tracker.create('req-1');
      const promise2 = tracker.create('req-2');

      tracker.clear();

      await expect(promise1).rejects.toThrow('cancelled');
      await expect(promise2).rejects.toThrow('cancelled');
    });
  });
});
