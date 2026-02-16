import { type BreezClient, verifyMessage } from '@breeztech/breez-sdk-spark'

const exampleSignMessage = async (client: BreezClient) => {
  // ANCHOR: sign-message
  // Set to true to get a compact signature rather than a DER
  const compact = true

  const signMessageResponse = await client.message.sign({
    message: '<message to sign>',
    compact
  })

  const signature = signMessageResponse.signature
  const pubkey = signMessageResponse.pubkey

  console.log(`Pubkey: ${pubkey}`)
  console.log(`Signature: ${signature}`)
  // ANCHOR_END: sign-message
}

const exampleCheckMessage = async () => {
  // ANCHOR: check-message
  const checkMessageResponse = verifyMessage({
    message: '<message>',
    pubkey: '<pubkey of signer>',
    signature: '<message signature>'
  })
  const isValid = checkMessageResponse.isValid

  console.log(`Signature valid: ${isValid}`)
  // ANCHOR_END: check-message
}
