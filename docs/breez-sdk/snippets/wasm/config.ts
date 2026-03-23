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

const exampleConfigureSparkConfig = async () => {
  // ANCHOR: spark-config
  const config = defaultConfig('mainnet')

  // Connect to a custom Spark environment
  config.sparkConfig = {
    coordinatorIdentifier: '0000000000000000000000000000000000000000000000000000000000000001',
    threshold: 2,
    signingOperators: [
      {
        id: 0,
        identifier: '0000000000000000000000000000000000000000000000000000000000000001',
        address: 'https://0.spark.example.com',
        identityPublicKey: '03acd9a5a88db102730ff83dee69d69088cc4c9d93bbee893e90fd5051b7da9651'
      },
      {
        id: 1,
        identifier: '0000000000000000000000000000000000000000000000000000000000000002',
        address: 'https://1.spark.example.com',
        identityPublicKey: '02d2d103cacb1d6355efeab27637c74484e2a7459e49110c3fe885210369782e23'
      },
      {
        id: 2,
        identifier: '0000000000000000000000000000000000000000000000000000000000000003',
        address: 'https://2.spark.example.com',
        identityPublicKey: '0350f07ffc21bfd59d31e0a7a600e2995273938444447cb9bc4c75b8a895dbb853'
      }
    ],
    sspConfig: {
      baseUrl: 'https://api.example.com',
      identityPublicKey: '02e0b8d42c5d3b5fe4c5beb6ea796ab3bc8aaf28a3d3195407482c67e0b58228a5'
    },
    expectedWithdrawBondSats: 10_000,
    expectedWithdrawRelativeBlockLocktime: 1_000
  }
  // ANCHOR_END: spark-config
  console.log('Config:', config)
}

export {
  exampleConfigureSdk,
  exampleConfigurePrivateEnabledDefault,
  exampleConfigureOptimizationConfiguration,
  exampleConfigureStableBalance,
  exampleConfigureSparkConfig
}
