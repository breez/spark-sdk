import {
  type BreezSdk,
  type X402,
  X402PaymentAmount,
  X402Request
} from '@breeztech/breez-sdk-spark-react-native'

const getX402 = (sdk: BreezSdk) => {
  // ANCHOR: get-x402
  const x402 = sdk.getX402()
  // ANCHOR_END: get-x402
  return x402
}

const payHttpRequest = async (x402: X402, paymentRequiredHeader: string) => {
  // ANCHOR: pay-http-request
  // The request that returned 402, exactly as it will be sent again
  const request = new X402Request.Http({
    method: 'GET',
    url: 'https://api.example.com/article/A',
    headers: [{ name: 'accept', value: 'application/json' }],
    body: undefined
  })
  const response = await x402.pay({
    // The PAYMENT-REQUIRED header of the 402 response
    paymentRequired: paymentRequiredHeader,
    request,
    maxAmount: new X402PaymentAmount.Lightning({ amountSats: BigInt(100) }),
    conversionOptions: undefined,
    completionTimeoutSecs: undefined
  })

  if (response.paymentSignature !== undefined) {
    // Send the request again with a PAYMENT-SIGNATURE header
    console.log(`PAYMENT-SIGNATURE: ${response.paymentSignature}`)
  } else {
    // Still in flight: paying the same challenge again picks it up
    console.log(`Payment status: ${response.status}`)
  }
  // ANCHOR_END: pay-http-request
}

const payMcpToolCall = async (x402: X402, structuredContent: string) => {
  // ANCHOR: pay-mcp-tool-call
  const request = new X402Request.Mcp({
    server: 'https://api.example.com/mcp',
    // The tools/call params, exactly as they will be sent again
    params: '{"name":"get_article","arguments":{"article":"A"}}'
  })
  const response = await x402.pay({
    // The structuredContent of the tool result
    paymentRequired: structuredContent,
    request,
    maxAmount: new X402PaymentAmount.Lightning({ amountSats: BigInt(100) }),
    conversionOptions: undefined,
    completionTimeoutSecs: undefined
  })

  if (response.paymentSignature !== undefined) {
    // Call the tool again with this JSON in params._meta["x402/payment"]
    console.log(`Payment: ${response.paymentSignature}`)
  }
  // ANCHOR_END: pay-mcp-tool-call
}
