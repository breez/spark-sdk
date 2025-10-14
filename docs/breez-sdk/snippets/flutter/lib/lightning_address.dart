import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';
import 'helper.dart';

Config configureLightningAddress() {
  // ANCHOR: config-lightning-address
  final config = defaultConfig(network: Network.mainnet)
      .copyWith(
        apiKey: 'your-api-key',
        lnurlDomain: 'yourdomain.com'
      );
  // ANCHOR_END: config-lightning-address
  return config;
}

Future<bool> checkLightningAddressAvailability(BreezSdk sdk) async {
  final username = 'myusername';
  
  // ANCHOR: check-lightning-address
  final request = CheckLightningAddressRequest(
    username: username,
  );
  
  final available = await sdk.checkLightningAddressAvailable(request: request);
  // ANCHOR_END: check-lightning-address
  return available;
}

Future<(String, String)> registerLightningAddress(BreezSdk sdk) async {
  final username = 'myusername';
  final description = 'My Lightning Address';
  // ANCHOR: register-lightning-address
  final request = RegisterLightningAddressRequest(
    username: username,
    description: description,
  );
  
  final addressInfo = await sdk.registerLightningAddress(request: request);
  final lightningAddress = addressInfo.lightningAddress;
  final lnurl = addressInfo.lnurl;
  // ANCHOR_END: register-lightning-address
  return (lightningAddress, lnurl);
}

Future<(String, String, String, String)> getLightningAddress(BreezSdk sdk) async {
  // ANCHOR: get-lightning-address
  final addressInfoOpt = await sdk.getLightningAddress();
  
  if (addressInfoOpt == null) {
    throw Exception("No Lightning Address registered for this user.");
  }
 
  final lightningAddress = addressInfoOpt.lightningAddress;
  final username = addressInfoOpt.username;
  final description = addressInfoOpt.description;
  final lnurl = addressInfoOpt.lnurl;
  // ANCHOR_END: get-lightning-address
  return (lightningAddress, username, description, lnurl);
}

Future<void> deleteLightningAddress(BreezSdk sdk) async {
  // ANCHOR: delete-lightning-address
  await sdk.deleteLightningAddress();
  // ANCHOR_END: delete-lightning-address
}
