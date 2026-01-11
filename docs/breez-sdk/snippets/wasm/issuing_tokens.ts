import {
  type Payment,
  type TokenMetadata,
  type BreezSdk,
  type TokenIssuer,
  type Seed,
  defaultConfig,
  SdkBuilder,
  KeySetType
} from '@breeztech/breez-sdk-spark'
import { Network } from 'node:inspector'

// Init stub
const init = async () => { }

const getTokenIssuer = (sdk: BreezSdk) => {
  // ANCHOR: get-token-issuer
  const tokenIssuer = sdk.getTokenIssuer()
  // ANCHOR_END: get-token-issuer
  return tokenIssuer
}

const createToken = async (tokenIssuer: TokenIssuer): Promise<TokenMetadata> => {
  // ANCHOR: create-token
  const tokenMetadata = await tokenIssuer.createIssuerToken({
    name: 'My Token',
    ticker: 'MTK',
    decimals: 6,
    isFreezable: false,
    maxSupply: BigInt(1_000_000)
  })
  console.debug(`Token identifier: ${tokenMetadata.identifier}`)
  // ANCHOR_END: create-token
  return tokenMetadata
}

const createTokenWithCustomAccountNumber = async () => {
  // ANCHOR: custom-account-number
  await init()

  const accountNumber = 21

  const mnemonic = '<mnemonic words>'
  const seed: Seed = { type: 'mnemonic', mnemonic, passphrase: undefined }
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'
  let builder = SdkBuilder.new(config, seed)
  builder = await builder.withDefaultStorage('./.data')

  // Set the account number for the SDK
  builder = builder.withKeySet({
    keySetType: 'default',
    useAddressIndex: false,
    accountNumber
  })

  const sdk = await builder.build()
  // ANCHOR_END: custom-account-number
}

const mintToken = async (tokenIssuer: TokenIssuer): Promise<Payment> => {
  // ANCHOR: mint-token
  const payment = await tokenIssuer.mintIssuerToken({
    amount: BigInt(1_000)
  })
  // ANCHOR_END: mint-token
  return payment
}

const burnToken = async (tokenIssuer: TokenIssuer): Promise<Payment> => {
  // ANCHOR: burn-token
  const payment = await tokenIssuer.burnIssuerToken({
    amount: BigInt(1_000)
  })
  // ANCHOR_END: burn-token
  return payment
}

const getTokenMetadata = async (tokenIssuer: TokenIssuer): Promise<TokenMetadata> => {
  // ANCHOR: get-token-metadata
  const tokenBalance = await tokenIssuer.getIssuerTokenBalance()
  console.debug(`Token balance: ${tokenBalance.balance}`)

  const tokenMetadata = await tokenIssuer.getIssuerTokenMetadata()
  console.debug(`Token ticker: ${tokenMetadata.ticker}`)
  // ANCHOR_END: get-token-metadata
  return tokenMetadata
}

const freezeToken = async (tokenIssuer: TokenIssuer): Promise<void> => {
  // ANCHOR: freeze-token
  const sparkAddress = '<spark address>'
  // Freeze the tokens held at the specified Spark address
  const freezeResponse = await tokenIssuer.freezeIssuerToken({
    address: sparkAddress
  })

  // Unfreeze the tokens held at the specified Spark address
  const unfreezeResponse = await tokenIssuer.unfreezeIssuerToken({
    address: sparkAddress
  })
  // ANCHOR_END: freeze-token
}
