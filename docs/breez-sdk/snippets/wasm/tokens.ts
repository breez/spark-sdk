import type { BreezClient, ConversionOptions, PaymentIntent } from '@breeztech/breez-sdk-spark'

const exampleFetchTokenBalances = async (client: BreezClient) => {
  // ANCHOR: fetch-token-balances
  const info = await client.getInfo({
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

const exampleFetchTokenMetadata = async (client: BreezClient) => {
  // ANCHOR: fetch-token-metadata
  const response = await client.tokens.metadata({
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

const exampleReceiveTokenPaymentSparkInvoice = async (client: BreezClient) => {
  // ANCHOR: receive-token-payment-spark-invoice
  const tokenIdentifier = '<token identifier>'
  const optionalDescription = '<invoice description>'
  const optionalAmountTokenUnits = '5000'
  // Optionally set the expiry UNIX timestamp in seconds
  const optionalExpiryTimeSeconds = 1716691200
  const optionalSenderPublicKey = '<sender public key>'

  const result = await client.receive({
    paymentType: 'sparkInvoice',
    tokenIdentifier,
    description: optionalDescription,
    amountTokenUnits: optionalAmountTokenUnits,
    expiry: optionalExpiryTimeSeconds,
    senderPublicKey: optionalSenderPublicKey
  })

  const destination = result.destination
  console.log(`Payment request: ${destination}`)
  const receiveFeeTokenUnits = result.feeTokenUnits
  console.log(`Fees: ${receiveFeeTokenUnits} token base units`)
  // ANCHOR_END: receive-token-payment-spark-invoice
}

const exampleSendTokenPayment = async (client: BreezClient) => {
  // ANCHOR: send-token-payment
  const destination = '<spark address or invoice>'
  // Token identifier must match the invoice in case it specifies one.
  const tokenIdentifier = '<token identifier>'
  // Set the amount of tokens you wish to send (in token base units).
  const amountTokenUnits = '1000'

  const payment = await client.preparePayment(destination, {
    amountTokenUnits,
    tokenIdentifier
  })

  // If the fees are acceptable, continue to send the token payment
  const fee = payment.fee
  if (fee.type === 'sparkToken') {
    console.log(`Token ID: ${payment.tokenIdentifier}`)
    console.log(`Fees: ${fee.feeTokenUnits} token base units`)
  }

  // Send the token payment
  const sendResponse = await payment.send()
  const confirmedPayment = sendResponse.payment
  console.log(`Payment: ${JSON.stringify(confirmedPayment)}`)
  // ANCHOR_END: send-token-payment
}

const exampleFetchConversionLimits = async (client: BreezClient) => {
  // ANCHOR: fetch-conversion-limits
  // Fetch limits for converting Bitcoin to a token
  const fromBitcoinResponse = await client.tokens.swapLimits({
    conversionType: { type: 'fromBitcoin' },
    tokenIdentifier: '<token identifier>'
  })

  if (fromBitcoinResponse.minFromAmount !== undefined) {
    console.log(`Minimum BTC to convert: ${fromBitcoinResponse.minFromAmount} sats`)
  }
  if (fromBitcoinResponse.minToAmount !== undefined) {
    console.log(`Minimum tokens to receive: ${fromBitcoinResponse.minToAmount} base units`)
  }

  // Fetch limits for converting a token to Bitcoin
  const toBitcoinResponse = await client.tokens.swapLimits({
    conversionType: {
      type: 'toBitcoin',
      fromTokenIdentifier: '<token identifier>'
    },
    tokenIdentifier: undefined
  })

  if (toBitcoinResponse.minFromAmount !== undefined) {
    console.log(`Minimum tokens to convert: ${toBitcoinResponse.minFromAmount} base units`)
  }
  if (toBitcoinResponse.minToAmount !== undefined) {
    console.log(`Minimum BTC to receive: ${toBitcoinResponse.minToAmount} sats`)
  }
  // ANCHOR_END: fetch-conversion-limits
}

const examplePrepareSendPaymentTokenConversion = async (client: BreezClient) => {
  // ANCHOR: prepare-send-payment-with-conversion
  const destination = '<spark address or invoice>'
  // Token identifier must match the invoice in case it specifies one.
  const tokenIdentifier = '<token identifier>'
  // Set the amount of tokens you wish to send (in token base units).
  const amountTokenUnits = '1000'
  // Set to use Bitcoin funds to pay via conversion
  const optionalMaxSlippageBps = 50
  const optionalCompletionTimeoutSecs = 30
  const conversionOptions: ConversionOptions = {
    conversionType: {
      type: 'fromBitcoin'
    },
    maxSlippageBps: optionalMaxSlippageBps,
    completionTimeoutSecs: optionalCompletionTimeoutSecs
  }

  const payment = await client.preparePayment(destination, {
    amountTokenUnits,
    tokenIdentifier,
    conversionOptions
  })

  // If the fees are acceptable, continue to send the token payment
  if (payment.conversionEstimate !== undefined) {
    const conversionEstimate = payment.conversionEstimate
    console.log(`Estimated conversion amount: ${conversionEstimate.amount} sats`)
    console.log(`Estimated conversion fee: ${conversionEstimate.fee} sats`)
  }
  // ANCHOR_END: prepare-send-payment-with-conversion
}
