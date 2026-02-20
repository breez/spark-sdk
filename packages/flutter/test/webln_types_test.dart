import 'package:flutter_test/flutter_test.dart';
import 'package:breez_sdk_spark_flutter/src/webln/types.dart';

void main() {
  group('LnurlType', () {
    test('has pay, withdraw, and auth values', () {
      expect(LnurlType.values, containsAll([LnurlType.pay, LnurlType.withdraw, LnurlType.auth]));
      expect(LnurlType.values.length, 3);
    });
  });

  group('LnurlRequest', () {
    test('creates pay request with all fields', () {
      const request = LnurlRequest(
        type: LnurlType.pay,
        domain: 'example.com',
        minAmountSats: 1000,
        maxAmountSats: 100000,
        metadata: '[["text/plain", "test"]]',
      );

      expect(request.type, LnurlType.pay);
      expect(request.domain, 'example.com');
      expect(request.minAmountSats, 1000);
      expect(request.maxAmountSats, 100000);
      expect(request.metadata, '[["text/plain", "test"]]');
      expect(request.defaultDescription, isNull);
    });

    test('creates withdraw request with all fields', () {
      const request = LnurlRequest(
        type: LnurlType.withdraw,
        domain: 'service.com',
        minAmountSats: 100,
        maxAmountSats: 50000,
        defaultDescription: 'Withdrawal',
      );

      expect(request.type, LnurlType.withdraw);
      expect(request.domain, 'service.com');
      expect(request.minAmountSats, 100);
      expect(request.maxAmountSats, 50000);
      expect(request.defaultDescription, 'Withdrawal');
      expect(request.metadata, isNull);
    });

    test('creates auth request with minimal fields', () {
      const request = LnurlRequest(
        type: LnurlType.auth,
        domain: 'auth.example.com',
      );

      expect(request.type, LnurlType.auth);
      expect(request.domain, 'auth.example.com');
      expect(request.minAmountSats, isNull);
      expect(request.maxAmountSats, isNull);
    });
  });

  group('LnurlUserResponse', () {
    test('creates approved response with amount', () {
      const response = LnurlUserResponse(
        approved: true,
        amountSats: 5000,
        comment: 'Thanks!',
      );

      expect(response.approved, true);
      expect(response.amountSats, 5000);
      expect(response.comment, 'Thanks!');
    });

    test('creates approved response without optional fields', () {
      const response = LnurlUserResponse(approved: true);

      expect(response.approved, true);
      expect(response.amountSats, isNull);
      expect(response.comment, isNull);
    });

    test('rejected constant is pre-defined', () {
      const response = LnurlUserResponse.rejected;

      expect(response.approved, false);
      expect(response.amountSats, isNull);
      expect(response.comment, isNull);
    });
  });

  group('WebLnErrorCode', () {
    test('has all expected error codes', () {
      expect(WebLnErrorCode.userRejected, 'USER_REJECTED');
      expect(WebLnErrorCode.providerNotEnabled, 'PROVIDER_NOT_ENABLED');
      expect(WebLnErrorCode.unsupportedMethod, 'UNSUPPORTED_METHOD');
      expect(WebLnErrorCode.insufficientFunds, 'INSUFFICIENT_FUNDS');
      expect(WebLnErrorCode.invalidParams, 'INVALID_PARAMS');
      expect(WebLnErrorCode.internalError, 'INTERNAL_ERROR');
    });
  });
}
