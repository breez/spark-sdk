/// WebLn types for Flutter
library;

/// Represents the type of LNURL request.
enum LnurlType {
  /// LNURL-pay request
  pay,

  /// LNURL-withdraw request
  withdraw,

  /// LNURL-auth request
  auth,
}

/// Represents an LNURL request that needs user approval.
/// Passed to the [OnLnurlRequest] callback.
class LnurlRequest {
  /// Type of LNURL operation
  final LnurlType type;

  /// Domain of the LNURL service
  final String domain;

  /// Minimum amount in sats (for pay/withdraw)
  final int? minAmountSats;

  /// Maximum amount in sats (for pay/withdraw)
  final int? maxAmountSats;

  /// LNURL metadata (for pay)
  final String? metadata;

  /// Default description (for withdraw)
  final String? defaultDescription;

  const LnurlRequest({
    required this.type,
    required this.domain,
    this.minAmountSats,
    this.maxAmountSats,
    this.metadata,
    this.defaultDescription,
  });
}

/// Represents the user's response to an LNURL request.
/// Returned from the [OnLnurlRequest] callback.
class LnurlUserResponse {
  /// Whether the user approved the operation
  final bool approved;

  /// Amount in sats (for pay/withdraw)
  final int? amountSats;

  /// Optional comment (for pay)
  final String? comment;

  const LnurlUserResponse({required this.approved, this.amountSats, this.comment});

  /// Creates a rejected response
  static const LnurlUserResponse rejected = LnurlUserResponse(approved: false);
}

/// WebLN error codes returned to JavaScript.
class WebLnErrorCode {
  static const String userRejected = 'USER_REJECTED';
  static const String providerNotEnabled = 'PROVIDER_NOT_ENABLED';
  static const String unsupportedMethod = 'UNSUPPORTED_METHOD';
  static const String insufficientFunds = 'INSUFFICIENT_FUNDS';
  static const String invalidParams = 'INVALID_PARAMS';
  static const String internalError = 'INTERNAL_ERROR';
}
