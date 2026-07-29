import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> getUserSettings(BreezSdk sdk) async {
  // ANCHOR: get-user-settings
  final userSettings = await sdk.getUserSettings();
  print('User settings: $userSettings');
  // ANCHOR_END: get-user-settings
}

Future<void> updateUserSettings(BreezSdk sdk) async {
  // ANCHOR: update-user-settings
  await sdk.updateUserSettings(
      request: UpdateUserSettingsRequest(
          sparkPrivateModeEnabled: true,
          stableBalanceActiveLabel: null,
          sparkMasterIdentityPublicKey: SparkMasterIdentityPublicKey_Set(
              publicKey: "<hex encoded public key>")));
  // ANCHOR_END: update-user-settings
}

Future<void> activateStableBalance(BreezSdk sdk) async {
  // ANCHOR: activate-stable-balance
  await sdk.updateUserSettings(
      request: UpdateUserSettingsRequest(
          stableBalanceActiveLabel: StableBalanceActiveLabel_Set(label: "USDB")));
  // ANCHOR_END: activate-stable-balance
}

Future<void> deactivateStableBalance(BreezSdk sdk) async {
  // ANCHOR: deactivate-stable-balance
  await sdk.updateUserSettings(
      request: UpdateUserSettingsRequest(
          stableBalanceActiveLabel: StableBalanceActiveLabel_Unset()));
  // ANCHOR_END: deactivate-stable-balance
}
