import { type Wallet } from '@breeztech/breez-sdk-spark'

const exampleGetUserSettings = async (wallet: Wallet) => {
  // ANCHOR: get-user-settings
  const userSettings = await wallet.settings.get()
  console.log(`User settings: ${JSON.stringify(userSettings)}`)
  // ANCHOR_END: get-user-settings
}

const exampleUpdateUserSettings = async (wallet: Wallet) => {
  // ANCHOR: update-user-settings
  const sparkPrivateModeEnabled = true
  await wallet.settings.update({
    sparkPrivateModeEnabled
  })
  // ANCHOR_END: update-user-settings
}
