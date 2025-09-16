import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Config configureLightningAddress() {
  // ANCHOR: config-lightning-address
  final config = defaultConfig(network: Network.bitcoin)
      .copyWith(
        apiKey: 'your-api-key',
        lnurlDomain: 'yourdomain.com'
      );
  // ANCHOR_END: config-lightning-address
  return config;
}

Future<void> checkLightningAddressAvailability(BreezSdk sdk) async {
  final username = 'myusername';
  
  // ANCHOR: check-lightning-address
  final request = CheckLightningAddressRequest(
    username: username,
  );
  
  final available = await sdk.checkLightningAddressAvailable(request: request);
  // ANCHOR_END: check-lightning-address
}

Future<void> registerLightningAddress(BreezSdk sdk) async {
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
}

Future<void> getLightningAddress(BreezSdk sdk) async {
  // ANCHOR: get-lightning-address
  final addressInfoOpt = await sdk.getLightningAddress();
  
  if (addressInfoOpt != null) {
    final lightningAddress = addressInfoOpt.lightningAddress;
    final username = addressInfoOpt.username;
    final description = addressInfoOpt.description;
    final lnurl = addressInfoOpt.lnurl;
  }
  // ANCHOR_END: get-lightning-address
}

Future<void> deleteLightningAddress(BreezSdk sdk) async {
  // ANCHOR: delete-lightning-address
  await sdk.deleteLightningAddress();
  // ANCHOR_END: delete-lightning-address
}
