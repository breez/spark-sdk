import { type BreezSdk, defaultConfig } from '@breeztech/breez-sdk-spark'

const configureLightningAddress = () => {
  // ANCHOR: config-lightning-address
  const config = defaultConfig('mainnet')
  config.apiKey = 'your-api-key'
  config.lnurlDomain = 'yourdomain.com'
  // ANCHOR_END: config-lightning-address
  return config
}

const exampleCheckLightningAddressAvailability = async (sdk: BreezSdk) => {
  const username = 'myusername'

  // ANCHOR: check-lightning-address
  const request = {
    username
  }

  const available = await sdk.checkLightningAddressAvailable(request)
  // ANCHOR_END: check-lightning-address
}

const exampleRegisterLightningAddress = async (sdk: BreezSdk) => {
  const username = 'myusername'
  const description = 'My Lightning Address'

  // ANCHOR: register-lightning-address
  const request = {
    username,
    description,
    transfer: undefined
  }

  const addressInfo = await sdk.registerLightningAddress(request)
  const lightningAddress = addressInfo.lightningAddress
  const lnurlUrl = addressInfo.lnurl.url
  const lnurlBech32 = addressInfo.lnurl.bech32
  // ANCHOR_END: register-lightning-address
}

const exampleGetLightningAddress = async (sdk: BreezSdk) => {
  // ANCHOR: get-lightning-address
  const addressInfoOpt = await sdk.getLightningAddress()

  if (addressInfoOpt != null) {
    const lightningAddress = addressInfoOpt.lightningAddress
    const username = addressInfoOpt.username
    const description = addressInfoOpt.description
    const lnurlUrl = addressInfoOpt.lnurl.url
    const lnurlBech32 = addressInfoOpt.lnurl.bech32
  }
  // ANCHOR_END: get-lightning-address
}

// Run on the *current owner's* wallet. Produces the authorization that the
// new owner needs to take over the username in a single atomic call.
const exampleSignLightningAddressTransfer = async (
  currentOwnerSdk: BreezSdk,
  currentOwnerPubkey: string,
  newOwnerPubkey: string
) => {
  const username = 'myusername'

  // ANCHOR: sign-lightning-address-transfer
  // `username` must be lowercased and trimmed.
  // pubkeys are hex-encoded secp256k1 compressed (via getInfo().identityPubkey).
  const message = `transfer:${currentOwnerPubkey}-${username}-${newOwnerPubkey}`
  const signed = await currentOwnerSdk.signMessage({
    message,
    compact: false
  })

  const transfer = {
    pubkey: signed.pubkey,
    signature: signed.signature
  }
  // ANCHOR_END: sign-lightning-address-transfer
  return transfer
}

// Run on the *new owner's* wallet with the authorization received
// out-of-band from the current owner.
const exampleRegisterLightningAddressViaTransfer = async (
  newOwnerSdk: BreezSdk,
  transfer: { pubkey: string, signature: string }
) => {
  const username = 'myusername'
  const description = 'My Lightning Address'

  // ANCHOR: register-lightning-address-transfer
  const request = {
    username,
    description,
    transfer
  }

  const addressInfo = await newOwnerSdk.registerLightningAddress(request)
  // ANCHOR_END: register-lightning-address-transfer
}

const exampleDeleteLightningAddress = async (sdk: BreezSdk) => {
  // ANCHOR: delete-lightning-address
  await sdk.deleteLightningAddress()
  // ANCHOR_END: delete-lightning-address
}

const exampleAccessSenderComment = async (sdk: BreezSdk) => {
  const paymentId = '<payment id>'
  const response = await sdk.getPayment({ paymentId })
  const payment = response.payment

  // ANCHOR: access-sender-comment
  // Check if this is a lightning payment with LNURL receive metadata
  if (payment.details?.type === 'lightning') {
    const metadata = payment.details.lnurlReceiveMetadata

    // Access the sender comment if present
    if (metadata?.senderComment != null) {
      console.log('Sender comment:', metadata.senderComment)
    }
  }
  // ANCHOR_END: access-sender-comment
}

const exampleAccessNostrZap = async (sdk: BreezSdk) => {
  const paymentId = '<payment id>'
  const response = await sdk.getPayment({ paymentId })
  const payment = response.payment

  // ANCHOR: access-nostr-zap
  // Check if this is a lightning payment with LNURL receive metadata
  if (payment.details?.type === 'lightning') {
    const metadata = payment.details.lnurlReceiveMetadata

    // Access the Nostr zap request if present
    if (metadata?.nostrZapRequest != null) {
      // The nostrZapRequest is a JSON string containing the Nostr event (kind 9734)
      console.log('Nostr zap request:', metadata.nostrZapRequest)
    }

    // Access the Nostr zap receipt if present
    if (metadata?.nostrZapReceipt != null) {
      // The nostrZapReceipt is a JSON string containing the Nostr event (kind 9735)
      console.log('Nostr zap receipt:', metadata.nostrZapReceipt)
    }
  }
  // ANCHOR_END: access-nostr-zap
}
