import { type BreezSdk } from '@breeztech/breez-sdk-spark'

const exampleSignMessage = async (sdk: BreezSdk) => {
  // ANCHOR: sign-message
  // Set to true to get a compact signature rather than a DER
  const optionalCompact = true

  const signMessageResponse = await sdk.signMessage({
    message: '<message to sign>',
    compact: optionalCompact
  })

  const signature = signMessageResponse.signature
  const pubkey = signMessageResponse.pubkey

  console.log(`Pubkey: ${pubkey}`)
  console.log(`Signature: ${signature}`)
  // ANCHOR_END: sign-message
}

const exampleCheckMessage = async (sdk: BreezSdk) => {
  // ANCHOR: check-message
  const checkMessageResponse = await sdk.checkMessage({
    message: '<message>',
    pubkey: '<pubkey of signer>',
    signature: '<message signature>'
  })
  const isValid = checkMessageResponse.isValid

  console.log(`Signature valid: ${isValid}`)
  // ANCHOR_END: check-message
}
