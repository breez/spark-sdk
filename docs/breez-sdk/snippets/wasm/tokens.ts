import type { BreezSdk, PrepareSendPaymentResponse, PrepareConvertTokenResponse } from '@breeztech/breez-sdk-spark'

const exampleFetchTokenBalances = async (sdk: BreezSdk) => {
  // ANCHOR: fetch-token-balances
  const info = await sdk.getInfo({
    // ensureSynced: true will ensure the SDK is synced with the Spark network
    // before returning the balance
    ensureSynced: false
  })

  // Token balances are a map of token identifier to balance
  const tokenBalances = info.tokenBalances
  for (const [tokenId, tokenBalance] of Object.entries(tokenBalances)) {
    console.log(`Token ID: ${tokenId}`)
    console.log(`Balance: ${tokenBalance.balance}`)
    console.log(`Name: ${tokenBalance.tokenMetadata.name}`)
    console.log(`Ticker: ${tokenBalance.tokenMetadata.ticker}`)
    console.log(`Decimals: ${tokenBalance.tokenMetadata.decimals}`)
  }
  // ANCHOR_END: fetch-token-balances
}

const exampleFetchTokenMetadata = async (sdk: BreezSdk) => {
  // ANCHOR: fetch-token-metadata
  const response = await sdk.getTokensMetadata({
    tokenIdentifiers: ['<token identifier 1>', '<token identifier 2>']
  })

  const tokensMetadata = response.tokensMetadata
  for (const tokenMetadata of tokensMetadata) {
    console.log(`Token ID: ${tokenMetadata.identifier}`)
    console.log(`Name: ${tokenMetadata.name}`)
    console.log(`Ticker: ${tokenMetadata.ticker}`)
    console.log(`Decimals: ${tokenMetadata.decimals}`)
    console.log(`Max Supply: ${tokenMetadata.maxSupply}`)
    console.log(`Is Freezable: ${tokenMetadata.isFreezable}`)
  }
  // ANCHOR_END: fetch-token-metadata
}

const exampleReceiveTokenPaymentSparkInvoice = async (sdk: BreezSdk) => {
  // ANCHOR: receive-token-payment-spark-invoice
  const tokenIdentifier = '<token identifier>'
  const optionalDescription = '<invoice description>'
  const optionalAmount = BigInt(5_000)
  const optionalExpiryTimeSeconds = 1716691200
  const optionalSenderPublicKey = '<sender public key>'

  const response = await sdk.receivePayment({
    paymentMethod: {
      type: 'sparkInvoice',
      tokenIdentifier,
      description: optionalDescription,
      amount: optionalAmount,
      expiryTime: optionalExpiryTimeSeconds,
      senderPublicKey: optionalSenderPublicKey
    }
  })

  const paymentRequest = response.paymentRequest
  console.log(`Payment request: ${paymentRequest}`)
  const receiveFeeSats = response.fee
  console.log(`Fees: ${receiveFeeSats} token base units`)
  // ANCHOR_END: receive-token-payment-spark-invoice
}

const exampleSendTokenPayment = async (sdk: BreezSdk) => {
  // ANCHOR: send-token-payment
  const paymentRequest = '<spark address or invoice>'
  // Token identifier must match the invoice in case it specifies one.
  const tokenIdentifier = '<token identifier>'
  // Set the amount of tokens you wish to send.
  const optionalAmount = BigInt(1_000)

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    amount: optionalAmount,
    tokenIdentifier
  })

  // If the fees are acceptable, continue to send the token payment
  if (prepareResponse.paymentMethod.type === 'sparkAddress') {
    console.log(`Token ID: ${prepareResponse.paymentMethod.tokenIdentifier}`)
    console.log(`Fees: ${prepareResponse.paymentMethod.fee} token base units`)
  }
  if (prepareResponse.paymentMethod.type === 'sparkInvoice') {
    console.log(`Token ID: ${prepareResponse.paymentMethod.tokenIdentifier}`)
    console.log(`Fees: ${prepareResponse.paymentMethod.fee} token base units`)
  }

  // Send the token payment
  const sendResponse = await sdk.sendPayment({
    prepareResponse,
    options: undefined
  })
  const payment = sendResponse.payment
  console.log(`Payment: ${JSON.stringify(payment)}`)
  // ANCHOR_END: send-token-payment
}

const examplePrepareConvertTokenToBitcoin = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-convert-token-to-bitcoin
  const tokenIdentifier = '<token identifier>'
  // Amount in token base units
  const amount = BigInt(10_000_000)

  const prepareResponse = await sdk.prepareConvertToken({
    convertType: {
      type: 'toBitcoin',
      fromTokenIdentifier: tokenIdentifier
    },
    amount
  })

  const estimatedReceiveAmount = prepareResponse.estimatedReceiveAmount
  const fee = prepareResponse.fee
  console.log(`Estimated receive amount: ${estimatedReceiveAmount} sats`)
  console.log(`Fee: ${fee} token base units`)
  // ANCHOR_END: prepare-convert-token-to-bitcoin
}

const examplePrepareConvertTokenFromBitcoin = async (sdk: BreezSdk) => {
  // ANCHOR: prepare-convert-token-from-bitcoin
  const tokenIdentifier = '<token identifier>'
  // Amount in satoshis
  const amount = BigInt(10_000)

  const prepareResponse = await sdk.prepareConvertToken({
    convertType: {
      type: 'fromBitcoin',
      toTokenIdentifier: tokenIdentifier
    },
    amount
  })

  const estimatedReceiveAmount = prepareResponse.estimatedReceiveAmount
  const fee = prepareResponse.fee
  console.log(`Estimated receive amount: ${estimatedReceiveAmount} token base units`)
  console.log(`Fee: ${fee} sats`)
  // ANCHOR_END: prepare-convert-token-from-bitcoin
}

const exampleConvertToken = async (sdk: BreezSdk, prepareResponse: PrepareConvertTokenResponse) => {
  // ANCHOR: convert-token
  // Set the maximum slippage to 1% in basis points
  const optionalMaxSlippageBps = 100

  const response = await sdk.convertToken({
    prepareResponse,
    maxSlippageBps: optionalMaxSlippageBps
  })

  const sentPayment = response.sentPayment
  const receivedPayment = response.receivedPayment
  console.log(`Sent Payment: ${JSON.stringify(sentPayment)}`)
  console.log(`Received Payment: ${JSON.stringify(receivedPayment)}`)
  // ANCHOR_END: convert-token
}
