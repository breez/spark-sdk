import {
  defaultConfig,
  Network,
  MaxFee,
  OptimizationConfig,
  StableBalanceConfig
} from '@breeztech/breez-sdk-spark-react-native'

const exampleConfigureSdk = () => {
  // ANCHOR: max-deposit-claim-fee
  // Create the default config
  const config = defaultConfig(Network.Mainnet)
  config.apiKey = '<breez api key>'

  // Disable automatic claiming
  config.maxDepositClaimFee = undefined

  // Set a maximum feerate of 10 sat/vB
  config.maxDepositClaimFee = new MaxFee.Rate({ satPerVbyte: BigInt(10) })

  // Set a maximum fee of 1000 sat
  config.maxDepositClaimFee = new MaxFee.Fixed({ amount: BigInt(1000) })

  // Set the maximum fee to the fastest network recommended fee at the time of claim
  // with a leeway of 1 sats/vbyte
  config.maxDepositClaimFee = new MaxFee.NetworkRecommended({ leewaySatPerVbyte: BigInt(1) })
  // ANCHOR_END: max-deposit-claim-fee
  console.log('Config:', config)
}

const exampleConfigurePrivateEnabledDefault = () => {
  // ANCHOR: private-enabled-default
  // Disable Spark private mode by default
  const config = defaultConfig(Network.Mainnet)
  config.privateEnabledDefault = false
  // ANCHOR_END: private-enabled-default
  console.log('Config:', config)
}

const exampleConfigureOptimizationConfiguration = () => {
  // ANCHOR: optimization-configuration
  const config = defaultConfig(Network.Mainnet)
  config.optimizationConfig = { autoEnabled: true, multiplicity: 1 }
  // ANCHOR_END: optimization-configuration
  console.log('Config:', config)
}

const exampleConfigureStableBalance = () => {
  // ANCHOR: stable-balance-config
  const config = defaultConfig(Network.Mainnet)

  // Enable stable balance with auto-conversion to a specific token
  config.stableBalanceConfig = {
    tokenIdentifier: '<token_identifier>',
    thresholdSats: BigInt(10_000),
    maxSlippageBps: 100,
    reservedSats: BigInt(1_000)
  }
  // ANCHOR_END: stable-balance-config
  console.log('Config:', config)
}

export {
  exampleConfigureSdk,
  exampleConfigurePrivateEnabledDefault,
  exampleConfigureOptimizationConfiguration,
  exampleConfigureStableBalance
}
