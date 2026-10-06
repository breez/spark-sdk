import {
  TokenAllowanceLimit,
  TokenAllowanceRole,
  type BreezSdk
} from '@breeztech/breez-sdk-spark-react-native'

const exampleCreateTokenAllowance = async (sdk: BreezSdk) => {
  // ANCHOR: create-token-allowance
  const response = await sdk.createTokenAllowance({
    spenderPublicKey: '<spender identity public key>',
    tokenIdentifier: '<token identifier>',
    maxPerPayment: new TokenAllowanceLimit.Amount({ amount: BigInt(5_000_000) }),
    maxTotal: new TokenAllowanceLimit.Amount({ amount: BigInt(100_000_000) }),
    expiryTime: BigInt(1_798_761_600),
    allowedRecipients: []
  })
  console.log(`Allowance id: ${response.allowance.id}`)
  // ANCHOR_END: create-token-allowance
}

const exampleListTokenAllowances = async (sdk: BreezSdk) => {
  // ANCHOR: list-token-allowances
  const response = await sdk.listTokenAllowances({
    role: TokenAllowanceRole.Owner,
    counterpartyPublicKey: undefined,
    tokenIdentifier: undefined,
    includeInactive: false,
    offset: undefined,
    limit: undefined
  })
  for (const allowance of response.allowances) {
    console.log(`${allowance.id}: spent ${allowance.spentAmount}`)
  }
  // ANCHOR_END: list-token-allowances
}

const exampleRevokeTokenAllowance = async (sdk: BreezSdk) => {
  // ANCHOR: revoke-token-allowance
  await sdk.revokeTokenAllowance({ allowanceId: '<allowance id>' })
  // ANCHOR_END: revoke-token-allowance
}

const examplePullPayment = async (sdk: BreezSdk) => {
  // ANCHOR: pull-payment
  const prepareResponse = await sdk.preparePullPayment({
    payerPublicKey: '<payer identity public key>',
    tokenIdentifier: '<token identifier>',
    receivers: [{ amount: BigInt(5_000_000), receiverPublicKey: undefined }]
  })
  console.log(`Pulling ${prepareResponse.amount}`)

  const response = await sdk.pullPayment({ prepareResponse })
  console.log(`Pull transaction: ${response.txHash}`)
  // ANCHOR_END: pull-payment
}
