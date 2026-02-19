import type { BreezSdk } from '@breeztech/breez-sdk-spark'

// ANCHOR: create-invoice
const createInvoiceExample = async (sdk: BreezSdk): Promise<void> => {
  // Create a Lightning invoice for 1000 sats
  const result = await sdk.createInvoice({
    amountSats: 1000,
    description: 'Coffee payment',
    expirySecs: 3600
  })
  console.log(`Invoice: ${result.bolt11}`)
  console.log(`Fee: ${result.feeSats} sats`)
}
// ANCHOR_END: create-invoice

// ANCHOR: create-spark-invoice
const createSparkInvoiceExample = async (sdk: BreezSdk): Promise<void> => {
  // Create a Spark invoice for 500 sats
  const result = await sdk.createSparkInvoice({
    amount: '500',
    description: 'Spark payment'
  })
  console.log(`Spark invoice: ${result.invoice}`)
  console.log(`Fee: ${result.fee}`)
}
// ANCHOR_END: create-spark-invoice

// ANCHOR: get-bitcoin-address
const getBitcoinAddressExample = async (sdk: BreezSdk): Promise<void> => {
  const result = await sdk.getBitcoinAddress()
  console.log(`Deposit address: ${result.address}`)
}
// ANCHOR_END: get-bitcoin-address

// ANCHOR: get-spark-address
const getSparkAddressExample = async (sdk: BreezSdk): Promise<void> => {
  const result = await sdk.getSparkAddress()
  console.log(`Spark address: ${result.address}`)
}
// ANCHOR_END: get-spark-address

export {
  createInvoiceExample,
  createSparkInvoiceExample,
  getBitcoinAddressExample,
  getSparkAddressExample
}
