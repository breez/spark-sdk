import { WebLnErrorCode, type LnurlRequest, type LnurlUserResponse } from '../types';

describe('WebLnErrorCode', () => {
  it('has all expected error codes', () => {
    expect(WebLnErrorCode.userRejected).toBe('USER_REJECTED');
    expect(WebLnErrorCode.providerNotEnabled).toBe('PROVIDER_NOT_ENABLED');
    expect(WebLnErrorCode.insufficientFunds).toBe('INSUFFICIENT_FUNDS');
    expect(WebLnErrorCode.invalidParams).toBe('INVALID_PARAMS');
    expect(WebLnErrorCode.unsupportedMethod).toBe('UNSUPPORTED_METHOD');
    expect(WebLnErrorCode.internalError).toBe('INTERNAL_ERROR');
  });
});

describe('LnurlRequest type', () => {
  it('can create pay request with all fields', () => {
    const request: LnurlRequest = {
      type: 'pay',
      domain: 'example.com',
      minAmountSats: 1000,
      maxAmountSats: 100000,
      metadata: '[["text/plain", "test"]]',
    };

    expect(request.type).toBe('pay');
    expect(request.domain).toBe('example.com');
    expect(request.minAmountSats).toBe(1000);
    expect(request.maxAmountSats).toBe(100000);
    expect(request.metadata).toBe('[["text/plain", "test"]]');
  });

  it('can create withdraw request with all fields', () => {
    const request: LnurlRequest = {
      type: 'withdraw',
      domain: 'service.com',
      minAmountSats: 100,
      maxAmountSats: 50000,
      defaultDescription: 'Withdrawal',
    };

    expect(request.type).toBe('withdraw');
    expect(request.domain).toBe('service.com');
    expect(request.minAmountSats).toBe(100);
    expect(request.maxAmountSats).toBe(50000);
    expect(request.defaultDescription).toBe('Withdrawal');
  });

  it('can create auth request with minimal fields', () => {
    const request: LnurlRequest = {
      type: 'auth',
      domain: 'auth.example.com',
    };

    expect(request.type).toBe('auth');
    expect(request.domain).toBe('auth.example.com');
    expect(request.minAmountSats).toBeUndefined();
  });
});

describe('LnurlUserResponse type', () => {
  it('can create approved response with amount', () => {
    const response: LnurlUserResponse = {
      approved: true,
      amountSats: 5000,
      comment: 'Thanks!',
    };

    expect(response.approved).toBe(true);
    expect(response.amountSats).toBe(5000);
    expect(response.comment).toBe('Thanks!');
  });

  it('can create rejected response', () => {
    const response: LnurlUserResponse = {
      approved: false,
    };

    expect(response.approved).toBe(false);
    expect(response.amountSats).toBeUndefined();
  });
});
