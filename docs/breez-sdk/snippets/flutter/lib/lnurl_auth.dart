import 'package:breez_sdk_spark/breez_sdk_spark.dart';

Future<void> parseLnurlAuth(BreezSdk sdk) async {
  // ANCHOR: parse-lnurl-auth
  // LNURL-auth URL from a service
  // Can be in the form:
  // - lnurl1... (bech32 encoded)
  // - https://service.com/lnurl-auth?tag=login&k1=...
  String lnurlAuthUrl = "lnurl1...";

  InputType inputType = await sdk.parse(lnurlAuthUrl);
  if (inputType is InputType_LnurlAuth) {
    LnurlAuthRequestDetails requestData = inputType.data;
    print("Domain: ${requestData.domain}");
    print("Action: ${requestData.action}");

    // Show domain to user and ask for confirmation
    // This is important for security
  }
  // ANCHOR_END: parse-lnurl-auth
}

Future<void> authenticate(BreezSdk sdk, LnurlAuthRequestDetails requestData) async {
  // ANCHOR: lnurl-auth
  // Perform LNURL authentication
  LnurlCallbackStatus result = await sdk.lnurlAuth(requestData: requestData);

  if (result is LnurlCallbackStatus_Ok) {
    print("Authentication successful");
  } else if (result is LnurlCallbackStatus_ErrorStatus) {
    print("Authentication failed: ${result.data.reason}");
  }
  // ANCHOR_END: lnurl-auth
}
