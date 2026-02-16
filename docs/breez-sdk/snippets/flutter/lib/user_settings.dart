import 'package:breez_sdk_spark_flutter/breez_sdk_spark.dart';

Future<void> getUserSettings(BreezClient client) async {
  // ANCHOR: get-user-settings
  final userSettings = await client.settings().get();
  print('User settings: $userSettings');
  // ANCHOR_END: get-user-settings
}

Future<void> updateUserSettings(BreezClient client) async {
  // ANCHOR: update-user-settings
  final bool sparkPrivateModeEnabled = true;

  await client.settings().update(
      request: UpdateUserSettingsRequest(
          sparkPrivateModeEnabled: sparkPrivateModeEnabled));
  // ANCHOR_END: update-user-settings
}
