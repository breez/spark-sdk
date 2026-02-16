import { type BreezClient } from '@breeztech/breez-sdk-spark-react-native'

const exampleSignMessage = async (client: BreezClient) => {
  // ANCHOR: sign-message
  // Set to true to get a compact signature rather than a DER
  const compact = true

  const signMessageResponse = await client.signMessage({
    message: '<message to sign>',
    compact
  })

  const signature = signMessageResponse.signature
  const pubkey = signMessageResponse.pubkey

  console.log(`Pubkey: ${pubkey}`)
  console.log(`Signature: ${signature}`)
  // ANCHOR_END: sign-message
}

const exampleCheckMessage = async (client: BreezClient) => {
  // ANCHOR: check-message
  const checkMessageResponse = await client.checkMessage({
    message: '<message>',
    pubkey: '<pubkey of signer>',
    signature: '<message signature>'
  })
  const isValid = checkMessageResponse.isValid

  console.log(`Signature valid: ${isValid}`)
  // ANCHOR_END: check-message
}
