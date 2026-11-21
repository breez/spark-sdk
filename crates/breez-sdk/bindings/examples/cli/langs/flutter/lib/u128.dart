import 'package:breez_sdk_spark_flutter/breez_sdk_spark_dart.dart';

/// Parses a u128 amount (token amounts and payment amounts), rejecting text
/// that is not a whole number.
U128 parseU128(String value) => BigInt.parse(value);

/// Like [parseU128], but returns `null` for text that is not a whole number.
U128? tryParseU128(String value) => BigInt.tryParse(value);

/// A u128 amount as a [BigInt], for arithmetic and comparisons.
BigInt u128ToBigInt(U128 value) => value;
