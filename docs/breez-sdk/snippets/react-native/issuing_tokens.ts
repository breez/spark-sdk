import {
  defaultConfig,
  type BreezSdk,
  Network,
  Seed,
  type TokenIssuer,
  type TokenMetadata,
  type Payment,
  KeySetType,
  SdkBuilder,
  type KeySetConfig
} from '@breeztech/breez-sdk-spark-react-native'
import RNFS from 'react-native-fs'

const getTokenIssuer = (sdk: BreezSdk) => {
  // ANCHOR: get-token-issuer
  const tokenIssuer = sdk.getTokenIssuer()
  // ANCHOR_END: get-token-issuer
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
  const accountNumber = 21

  const mnemonic = '<mnemonics words>'
  const seed = new Seed.Mnemonic({ mnemonic, passphrase: undefined })
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'
  const builder = new SdkBuilder(config, seed)
  await builder.withDefaultStorage(`${RNFS.DocumentDirectoryPath}/data`)

  // Set the account number for the SDK
  const keySetConfig: KeySetConfig = {
    keySetType: KeySetType.Default,
    useAddressIndex: false,
    accountNumber: accountNumber
  }
  await builder.withKeySet(keySetConfig)

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

  // To unfreeze the tokens, use the following:
  const unfreezeResponse = await tokenIssuer.unfreezeIssuerToken({
    address: sparkAddress
  })
  // ANCHOR_END: freeze-token
}
