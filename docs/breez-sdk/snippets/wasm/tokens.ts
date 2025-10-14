import { type BreezSdk, type PrepareSendPaymentResponse } from '@breeztech/breez-sdk-spark'

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

const exampleSendTokenPayment = async (sdk: BreezSdk) => {
  // ANCHOR: send-token-payment
  const paymentRequest = '<spark address>'
  // The token identifier (e.g., asset ID or token contract)
  const tokenIdentifier = '<token identifier>'
  // Set the amount of tokens you wish to send
  const amount = BigInt(1_000)

  const prepareResponse = await sdk.prepareSendPayment({
    paymentRequest,
    amount,
    tokenIdentifier
  })

  // If the fees are acceptable, continue to send the token payment
  if (prepareResponse.paymentMethod.type === 'sparkAddress') {
    console.log(`Token ID: ${prepareResponse.paymentMethod.tokenIdentifier}`)
    console.log(`Fees: ${prepareResponse.paymentMethod.fee} sats`)
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
