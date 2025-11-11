import type {
  BreezSdk,
  BreezIssuerSdk,
  TokenMetadata,
  Payment
} from '@breeztech/breez-sdk-spark-react-native'

const getIssuerSdk = (sdk: BreezSdk) => {
  // ANCHOR: get-issuer-sdk
  const issuerSdk = sdk.getIssuerSdk()
  // ANCHOR_END: get-issuer-sdk
}

const createToken = async (issuerSdk: BreezIssuerSdk): Promise<TokenMetadata> => {
  // ANCHOR: create-token
  const tokenMetadata = await issuerSdk.createIssuerToken({
    name: 'My Token',
    ticker: 'MTK',
    decimals: 2,
    isFreezable: false,
    maxSupply: BigInt(1_000_000)
  })
  console.debug(`Token identifier: ${tokenMetadata.identifier}`)
  // ANCHOR_END: create-token
  return tokenMetadata
}

const mintToken = async (issuerSdk: BreezIssuerSdk): Promise<Payment> => {
  // ANCHOR: mint-token
  const payment = await issuerSdk.mintIssuerToken({
    amount: BigInt(1_000)
  })
  // ANCHOR_END: mint-token
  return payment
}

const burnToken = async (issuerSdk: BreezIssuerSdk): Promise<Payment> => {
  // ANCHOR: burn-token
  const payment = await issuerSdk.burnIssuerToken({
    amount: BigInt(1_000)
  })
  // ANCHOR_END: burn-token
  return payment
}

const getTokenMetadata = async (issuerSdk: BreezIssuerSdk): Promise<TokenMetadata> => {
  // ANCHOR: get-token-metadata
  const tokenBalance = await issuerSdk.getIssuerTokenBalance()
  console.debug(`Token balance: ${tokenBalance.balance}`)

  const tokenMetadata = await issuerSdk.getIssuerTokenMetadata()
  console.debug(`Token ticker: ${tokenMetadata.ticker}`)
  // ANCHOR_END: get-token-metadata
  return tokenMetadata
}

const freezeToken = async (issuerSdk: BreezIssuerSdk): Promise<void> => {
  // ANCHOR: freeze-token
  const sparkAddress = '<spark address>'
  // Freeze the tokens held at the specified Spark address
  const freezeResponse = await issuerSdk.freezeIssuerToken({
    address: sparkAddress
  })

  // To unfreeze the tokens, use the following:
  const unfreezeResponse = await issuerSdk.unfreezeIssuerToken({
    address: sparkAddress
  })
  // ANCHOR_END: freeze-token
}
