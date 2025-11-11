import { type BreezSdk } from '@breeztech/breez-sdk-spark'

const exampleGetUserSettings = async (sdk: BreezSdk) => {
  // ANCHOR: get-user-settings
  const userSettings = await sdk.getUserSettings()
  console.log(`User settings: ${JSON.stringify(userSettings)}`)
  // ANCHOR_END: get-user-settings
}

const exampleUpdateUserSettings = async (sdk: BreezSdk) => {
  // ANCHOR: update-user-settings
  const sparkPrivateModeEnabled = true
  await sdk.updateUserSettings({
    sparkPrivateModeEnabled
  })
  // ANCHOR_END: update-user-settings
}
