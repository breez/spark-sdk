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
    sparkPrivateModeEnabled,
    stableBalanceActiveLabel: undefined
  })
  // ANCHOR_END: update-user-settings
}

const exampleActivateStableBalance = async (sdk: BreezSdk) => {
  // ANCHOR: activate-stable-balance
  await sdk.updateUserSettings({
    stableBalanceActiveLabel: { type: 'set', label: 'USDB' }
  })
  // ANCHOR_END: activate-stable-balance
}

const exampleDeactivateStableBalance = async (sdk: BreezSdk) => {
  // ANCHOR: deactivate-stable-balance
  await sdk.updateUserSettings({
    stableBalanceActiveLabel: { type: 'unset' }
  })
  // ANCHOR_END: deactivate-stable-balance
}
