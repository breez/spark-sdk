import { describe, it, expect, beforeEach, vi } from 'vitest';
import { BreezSparkWebLnProvider } from './provider';
import { WebLnError, WebLnErrorCodes } from './types';

describe('BreezSparkWebLnProvider', () => {
  let provider: BreezSparkWebLnProvider;
  let mockPostMessage: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    provider = new BreezSparkWebLnProvider();
    mockPostMessage = vi.fn();

    // Setup Android/iOS bridge
    (window as any).BreezSparkWebLn = { postMessage: mockPostMessage };

    // Mock window.location.origin
    Object.defineProperty(window, 'location', {
      value: { origin: 'https://example.com' },
      writable: true,
    });
  });

  describe('enable', () => {
    it('sends enable request with domain', async () => {
      const enablePromise = provider.enable();

      // Get the request that was sent
      expect(mockPostMessage).toHaveBeenCalledTimes(1);
      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);

      expect(request.method).toBe('enable');
      expect(request.params.domain).toBe('https://example.com');

      // Simulate native response
      provider._handleResponse({
        id: request.id,
        success: true,
        result: {},
      });

      await enablePromise;
    });

    it('does not send request if already enabled', async () => {
      // First enable
      const firstPromise = provider.enable();
      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      provider._handleResponse({ id: request.id, success: true });
      await firstPromise;

      mockPostMessage.mockClear();

      // Second enable should be no-op
      await provider.enable();
      expect(mockPostMessage).not.toHaveBeenCalled();
    });

    it('throws when no bridge is available', async () => {
      delete (window as any).BreezSparkWebLn;

      await expect(provider.enable()).rejects.toThrow('no native bridge detected');
    });
  });

  describe('getInfo', () => {
    it('throws if not enabled', async () => {
      await expect(provider.getInfo()).rejects.toThrow('Provider not enabled');
    });

    it('sends getInfo request when enabled', async () => {
      // Enable first
      const enablePromise = provider.enable();
      let request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      provider._handleResponse({ id: request.id, success: true });
      await enablePromise;

      mockPostMessage.mockClear();

      // Call getInfo
      const infoPromise = provider.getInfo();

      expect(mockPostMessage).toHaveBeenCalledTimes(1);
      request = JSON.parse(mockPostMessage.mock.calls[0][0]);

      expect(request.method).toBe('getInfo');
      expect(request.params).toEqual({});

      // Simulate response
      provider._handleResponse({
        id: request.id,
        success: true,
        result: {
          node: { pubkey: 'abc123', alias: '' },
          methods: ['getInfo', 'sendPayment'],
        },
      });

      const result = await infoPromise;
      expect(result.node.pubkey).toBe('abc123');
      expect(result.methods).toContain('sendPayment');
    });
  });

  describe('sendPayment', () => {
    beforeEach(async () => {
      const enablePromise = provider.enable();
      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      provider._handleResponse({ id: request.id, success: true });
      await enablePromise;
      mockPostMessage.mockClear();
    });

    it('sends payment request with invoice', async () => {
      const paymentPromise = provider.sendPayment('lnbc1000...');

      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      expect(request.method).toBe('sendPayment');
      expect(request.params.paymentRequest).toBe('lnbc1000...');

      provider._handleResponse({
        id: request.id,
        success: true,
        result: { preimage: 'abc123preimage' },
      });

      const result = await paymentPromise;
      expect(result.preimage).toBe('abc123preimage');
    });

    it('throws on invalid payment request', async () => {
      await expect(provider.sendPayment('')).rejects.toThrow('Invalid payment request');
      await expect(provider.sendPayment(null as any)).rejects.toThrow('Invalid payment request');
    });

    it('handles USER_REJECTED error', async () => {
      const paymentPromise = provider.sendPayment('lnbc1000...');

      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      provider._handleResponse({
        id: request.id,
        success: false,
        error: 'USER_REJECTED',
      });

      await expect(paymentPromise).rejects.toThrow('USER_REJECTED');
    });
  });

  describe('keysend', () => {
    it('always throws UNSUPPORTED_METHOD', async () => {
      try {
        await provider.keysend({ destination: 'abc', amount: 1000 });
        expect.fail('Should have thrown');
      } catch (error) {
        expect(error).toBeInstanceOf(WebLnError);
        expect((error as WebLnError).code).toBe(WebLnErrorCodes.UNSUPPORTED_METHOD);
      }
    });
  });

  describe('makeInvoice', () => {
    beforeEach(async () => {
      const enablePromise = provider.enable();
      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      provider._handleResponse({ id: request.id, success: true });
      await enablePromise;
      mockPostMessage.mockClear();
    });

    it('sends makeInvoice request with args', async () => {
      const invoicePromise = provider.makeInvoice({ amount: 1000, defaultMemo: 'Test' });

      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      expect(request.method).toBe('makeInvoice');
      expect(request.params.amount).toBe(1000);
      expect(request.params.defaultMemo).toBe('Test');

      provider._handleResponse({
        id: request.id,
        success: true,
        result: { paymentRequest: 'lnbc1000...' },
      });

      const result = await invoicePromise;
      expect(result.paymentRequest).toBe('lnbc1000...');
    });

    it('sends makeInvoice request without args', async () => {
      const invoicePromise = provider.makeInvoice();

      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      expect(request.method).toBe('makeInvoice');
      expect(request.params).toEqual({});

      provider._handleResponse({
        id: request.id,
        success: true,
        result: { paymentRequest: 'lnbc...' },
      });

      await invoicePromise;
    });
  });

  describe('signMessage', () => {
    beforeEach(async () => {
      const enablePromise = provider.enable();
      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      provider._handleResponse({ id: request.id, success: true });
      await enablePromise;
      mockPostMessage.mockClear();
    });

    it('sends signMessage request', async () => {
      const signPromise = provider.signMessage('Hello, World!');

      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      expect(request.method).toBe('signMessage');
      expect(request.params.message).toBe('Hello, World!');

      provider._handleResponse({
        id: request.id,
        success: true,
        result: { message: 'Hello, World!', signature: 'sig123' },
      });

      const result = await signPromise;
      expect(result.signature).toBe('sig123');
    });

    it('throws on invalid message', async () => {
      await expect(provider.signMessage('')).rejects.toThrow('Invalid message');
    });
  });

  describe('verifyMessage', () => {
    beforeEach(async () => {
      const enablePromise = provider.enable();
      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      provider._handleResponse({ id: request.id, success: true });
      await enablePromise;
      mockPostMessage.mockClear();
    });

    it('sends verifyMessage request', async () => {
      const verifyPromise = provider.verifyMessage('sig123', 'Hello');

      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      expect(request.method).toBe('verifyMessage');
      expect(request.params.signature).toBe('sig123');
      expect(request.params.message).toBe('Hello');

      provider._handleResponse({
        id: request.id,
        success: true,
      });

      await verifyPromise;
    });

    it('throws on invalid signature', async () => {
      await expect(provider.verifyMessage('', 'Hello')).rejects.toThrow('Invalid signature');
    });

    it('throws on invalid message', async () => {
      await expect(provider.verifyMessage('sig123', '')).rejects.toThrow('Invalid message');
    });
  });

  describe('lnurl', () => {
    beforeEach(async () => {
      const enablePromise = provider.enable();
      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      provider._handleResponse({ id: request.id, success: true });
      await enablePromise;
      mockPostMessage.mockClear();
    });

    it('sends lnurl request', async () => {
      const lnurlPromise = provider.lnurl('lnurl1dp68gurn8ghj7...');

      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);
      expect(request.method).toBe('lnurl');
      expect(request.params.lnurl).toBe('lnurl1dp68gurn8ghj7...');

      provider._handleResponse({
        id: request.id,
        success: true,
        result: { status: 'OK' },
      });

      const result = await lnurlPromise;
      expect(result.status).toBe('OK');
    });

    it('throws on invalid lnurl', async () => {
      await expect(provider.lnurl('')).rejects.toThrow('Invalid LNURL');
    });
  });

  describe('_handleResponse', () => {
    it('handles success response', async () => {
      const enablePromise = provider.enable();
      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);

      provider._handleResponse({
        id: request.id,
        success: true,
        result: {},
      });

      await enablePromise;
    });

    it('handles error response', async () => {
      const enablePromise = provider.enable();
      const request = JSON.parse(mockPostMessage.mock.calls[0][0]);

      provider._handleResponse({
        id: request.id,
        success: false,
        error: 'USER_REJECTED',
      });

      await expect(enablePromise).rejects.toThrow('USER_REJECTED');
    });
  });
});
