import { type BreezClient, defaultConfig } from '@breeztech/breez-sdk-spark'

const configureLightningAddress = () => {
  // ANCHOR: config-lightning-address
  const config = defaultConfig('mainnet')
  config.apiKey = 'your-api-key'
  config.lnurlDomain = 'yourdomain.com'
  // ANCHOR_END: config-lightning-address
  return config
}

const exampleCheckLightningAddressAvailability = async (client: BreezClient) => {
  const username = 'myusername'

  // ANCHOR: check-lightning-address
  const available = await client.lightningAddress.isAvailable(username)
  // ANCHOR_END: check-lightning-address
}

const exampleRegisterLightningAddress = async (client: BreezClient) => {
  const username = 'myusername'
  const description = 'My Lightning Address'

  // ANCHOR: register-lightning-address
  const request = {
    username,
    description
  }

  const addressInfo = await client.lightningAddress.register(request)
  const lightningAddress = addressInfo.lightningAddress
  const lnurlUrl = addressInfo.lnurl.url
  const lnurlBech32 = addressInfo.lnurl.bech32
  // ANCHOR_END: register-lightning-address
}

const exampleGetLightningAddress = async (client: BreezClient) => {
  // ANCHOR: get-lightning-address
  const addressInfoOpt = await client.lightningAddress.get()

  if (addressInfoOpt != null) {
    const lightningAddress = addressInfoOpt.lightningAddress
    const username = addressInfoOpt.username
    const description = addressInfoOpt.description
    const lnurlUrl = addressInfoOpt.lnurl.url
    const lnurlBech32 = addressInfoOpt.lnurl.bech32
  }
  // ANCHOR_END: get-lightning-address
}

const exampleDeleteLightningAddress = async (client: BreezClient) => {
  // ANCHOR: delete-lightning-address
  await client.lightningAddress.delete()
  // ANCHOR_END: delete-lightning-address
}

const exampleAccessSenderComment = async (client: BreezClient) => {
  const paymentId = '<payment id>'
  const response = await client.payments.get(paymentId)
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

const exampleAccessNostrZap = async (client: BreezClient) => {
  const paymentId = '<payment id>'
  const response = await client.payments.get(paymentId)
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
