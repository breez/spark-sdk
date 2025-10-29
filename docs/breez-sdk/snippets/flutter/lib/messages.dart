import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<SignMessageResponse> signMessage(BreezSdk sdk) async {
  // ANCHOR: sign-message
  // Set to true to get a compact signature rather than a DER
  bool optionalCompact = true;

  SignMessageRequest signMessageRequest = SignMessageRequest(
    message: "<message to sign>",
    compact: optionalCompact,
  );

  SignMessageResponse signMessageResponse = await sdk.signMessage(
    request: signMessageRequest,
  );

  String signature = signMessageResponse.signature;
  String pubkey = signMessageResponse.pubkey;

  print("Pubkey: $pubkey");
  print("Signature: $signature");
  // ANCHOR_END: sign-message
  return signMessageResponse;
}

Future<CheckMessageResponse> checkMessage(BreezSdk sdk) async {
  // ANCHOR: check-message
  CheckMessageRequest checkMessageRequest = CheckMessageRequest(
    message: "<message>",
    pubkey: "<pubkey of signer>",
    signature: "<message signature>",
  );

  CheckMessageResponse checkMessageResponse = await sdk.checkMessage(
    request: checkMessageRequest,
  );

  bool isValid = checkMessageResponse.isValid;

  print("Signature valid: $isValid");
  // ANCHOR_END: check-message
  return checkMessageResponse;
}
