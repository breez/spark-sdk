import type { Wallet, ConversionOptions, PaymentIntent } from '@breeztech/breez-sdk-spark'

const exampleFetchTokenBalances = async (wallet: Wallet) => {
  // ANCHOR: fetch-token-balances
  const info = await wallet.getInfo({
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

const exampleFetchTokenMetadata = async (wallet: Wallet) => {
  // ANCHOR: fetch-token-metadata
  const response = await wallet.tokens.metadata({
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

const exampleReceiveTokenPaymentSparkInvoice = async (wallet: Wallet) => {
  // ANCHOR: receive-token-payment-spark-invoice
  const tokenIdentifier = '<token identifier>'
  const optionalDescription = '<invoice description>'
  const optionalAmount = '5000'
  // Optionally set the expiry UNIX timestamp in seconds
  const optionalExpiryTimeSeconds = 1716691200
  const optionalSenderPublicKey = '<sender public key>'

  const result = await wallet.receive({
    paymentType: 'sparkInvoice',
    tokenIdentifier,
    description: optionalDescription,
    amount: optionalAmount,
    expiry: optionalExpiryTimeSeconds,
    senderPublicKey: optionalSenderPublicKey
  })

  const destination = result.destination
  console.log(`Payment request: ${destination}`)
  const receiveFeeSats = result.fee
  console.log(`Fees: ${receiveFeeSats} token base units`)
  // ANCHOR_END: receive-token-payment-spark-invoice
}

const exampleSendTokenPayment = async (wallet: Wallet) => {
  // ANCHOR: send-token-payment
  const destination = '<spark address or invoice>'
  // Token identifier must match the invoice in case it specifies one.
  const tokenIdentifier = '<token identifier>'
  // Set the amount of tokens you wish to send (in token base units).
  const amount = BigInt(1_000)

  const intent = await wallet.createPayment(destination, {
    amount,
    tokenIdentifier
  })

  // If the fees are acceptable, continue to send the token payment
  const fee = intent.fee
  if (fee.type === 'spark') {
    console.log(`Token ID: ${intent.tokenIdentifier}`)
    console.log(`Fees: ${fee.fee} token base units`)
  }

  // Send the token payment
  const sendResponse = await intent.confirm()
  const payment = sendResponse.payment
  console.log(`Payment: ${JSON.stringify(payment)}`)
  // ANCHOR_END: send-token-payment
}

const exampleFetchConversionLimits = async (wallet: Wallet) => {
  // ANCHOR: fetch-conversion-limits
  // Fetch limits for converting Bitcoin to a token
  const fromBitcoinResponse = await wallet.tokens.swapLimits({
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
  const toBitcoinResponse = await wallet.tokens.swapLimits({
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

const examplePrepareSendPaymentTokenConversion = async (wallet: Wallet) => {
  // ANCHOR: prepare-send-payment-with-conversion
  const destination = '<spark address or invoice>'
  // Token identifier must match the invoice in case it specifies one.
  const tokenIdentifier = '<token identifier>'
  // Set the amount of tokens you wish to send (in token base units).
  const amount = BigInt(1_000)
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

  const intent = await wallet.createPayment(destination, {
    amount,
    tokenIdentifier,
    conversionOptions
  })

  // If the fees are acceptable, continue to send the token payment
  if (intent.conversionEstimate !== undefined) {
    const conversionEstimate = intent.conversionEstimate
    console.log(`Estimated conversion amount: ${conversionEstimate.amount} sats`)
    console.log(`Estimated conversion fee: ${conversionEstimate.fee} sats`)
  }
  // ANCHOR_END: prepare-send-payment-with-conversion
}
