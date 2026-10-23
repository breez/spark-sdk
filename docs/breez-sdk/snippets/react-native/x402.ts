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

const createChallenge = async (x402: X402) => {
  // ANCHOR: create-challenge
  // The unpaid request, with its URL built from your public origin
  const request = new X402Request.Http({
    method: 'GET',
    url: 'https://api.example.com/article/A',
    headers: [{ name: 'accept', value: 'application/json' }],
    body: undefined
  })
  const response = await x402.createChallenge({
    request,
    // Headers that change what is served, so the payment covers them too
    boundNames: ['accept'],
    amounts: [new X402PaymentAmount.Lightning({ amountSats: BigInt(21) })],
    maxTimeoutSecs: undefined,
    description: 'An article',
    mimeType: 'application/json'
  })

  // Respond with status 402 and this PAYMENT-REQUIRED header
  console.log(`PAYMENT-REQUIRED: ${response.paymentRequired}`)
  // ANCHOR_END: create-challenge
}

const settlePayment = async (x402: X402, paymentSignatureHeader: string) => {
  // ANCHOR: settle-payment
  // The paid request, exactly as received
  const request = new X402Request.Http({
    method: 'GET',
    url: 'https://api.example.com/article/A',
    headers: [{ name: 'accept', value: 'application/json' }],
    body: undefined
  })
  const response = await x402.settle({
    // The PAYMENT-SIGNATURE header of the request
    payment: paymentSignatureHeader,
    request,
    boundNames: ['accept']
  })

  // Send the PAYMENT-RESPONSE header either way
  console.log(`PAYMENT-RESPONSE: ${response.paymentResponse}`)
  if (response.success) {
    // Serve the resource
  } else {
    // Respond with status 402 and a new challenge
    console.log(`Payment refused: ${response.errorReason ?? ''}`)
  }
  // ANCHOR_END: settle-payment
}
