import { type BreezClient } from '@breeztech/breez-sdk-spark'

const exampleGetUserSettings = async (client: BreezClient) => {
  // ANCHOR: get-user-settings
  const userSettings = await client.settings.get()
  console.log(`User settings: ${JSON.stringify(userSettings)}`)
  // ANCHOR_END: get-user-settings
}

const exampleUpdateUserSettings = async (client: BreezClient) => {
  // ANCHOR: update-user-settings
  const sparkPrivateModeEnabled = true
  await client.settings.update({
    sparkPrivateModeEnabled
  })
  // ANCHOR_END: update-user-settings
}
