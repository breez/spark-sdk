import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> getUserSettings(BreezSdk sdk) async {
  // ANCHOR: get-user-settings
  final userSettings = await sdk.getUserSettings();
  print('User settings: $userSettings');
  // ANCHOR_END: get-user-settings
}

Future<void> updateUserSettings(BreezSdk sdk) async {
  // ANCHOR: update-user-settings
  final bool enableSparkPrivateMode = true;

  await sdk.updateUserSettings(
      request: UpdateUserSettingsRequest(
          enableSparkPrivateMode: enableSparkPrivateMode));
  // ANCHOR_END: update-user-settings
}
