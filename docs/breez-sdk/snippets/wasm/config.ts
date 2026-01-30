import { defaultConfig } from '@breeztech/breez-sdk-spark'

const exampleConfigureSdk = async () => {
  // ANCHOR: max-deposit-claim-fee
  // Create the default config
  const config = defaultConfig('mainnet')
  config.apiKey = '<breez api key>'

  // Disable automatic claiming
  config.maxDepositClaimFee = undefined

  // Set a maximum feerate of 10 sat/vB
  config.maxDepositClaimFee = { type: 'rate', satPerVbyte: 10 }

  // Set a maximum fee of 1000 sat
  config.maxDepositClaimFee = { type: 'fixed', amount: 1000 }

  // Set the maximum fee to the fastest network recommended fee at the time of claim
  // with a leeway of 1 sats/vbyte
  config.maxDepositClaimFee = { type: 'networkRecommended', leewaySatPerVbyte: 1 }
  // ANCHOR_END: max-deposit-claim-fee
  console.log('Config:', config)
}

const exampleConfigurePrivateEnabledDefault = async () => {
  // ANCHOR: private-enabled-default
  // Disable Spark private mode by default
  const config = defaultConfig('mainnet')
  config.privateEnabledDefault = false
  // ANCHOR_END: private-enabled-default
  console.log('Config:', config)
}

const exampleConfigureOptimizationConfiguration = async () => {
  // ANCHOR: optimization-configuration
  const config = defaultConfig('mainnet')
  config.optimizationConfig = { autoEnabled: true, multiplicity: 1 }
  // ANCHOR_END: optimization-configuration
  console.log('Config:', config)
}

const exampleConfigureStableBalance = async () => {
  // ANCHOR: stable-balance-config
  const config = defaultConfig('mainnet')

  // Enable stable balance with auto-conversion to a specific token
  config.stableBalanceConfig = {
    tokenIdentifier: '<token_identifier>',
    thresholdSats: 10_000,
    maxSlippageBps: 100,
    reservedSats: 1_000
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
