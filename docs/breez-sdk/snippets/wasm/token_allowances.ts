import type { BreezSdk } from '@breeztech/breez-sdk-spark'

const exampleCreateTokenAllowance = async (sdk: BreezSdk) => {
  // ANCHOR: create-token-allowance
  const response = await sdk.createTokenAllowance({
    spenderPublicKey: '<spender identity public key>',
    tokenIdentifier: '<token identifier>',
    maxPerPayment: { type: 'amount', amount: '5000000' },
    maxTotal: { type: 'amount', amount: '100000000' },
    expiryTime: 1_798_761_600,
    allowedRecipients: []
  })
  console.log(`Allowance id: ${response.allowance.id}`)
  // ANCHOR_END: create-token-allowance
}

const exampleListTokenAllowances = async (sdk: BreezSdk) => {
  // ANCHOR: list-token-allowances
  const response = await sdk.listTokenAllowances({ role: 'owner', includeInactive: false })
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
    receivers: [{ amount: '5000000' }]
  })
  console.log(`Pulling ${prepareResponse.amount}`)

  const response = await sdk.pullPayment({ prepareResponse })
  console.log(`Pull transaction: ${response.txHash}`)
  // ANCHOR_END: pull-payment
}
