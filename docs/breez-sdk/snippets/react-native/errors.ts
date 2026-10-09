import {
  type BreezSdk,
  type PrepareSendPaymentRequest,
  SdkError
} from '@breeztech/breez-sdk-spark-react-native'

const exampleHandleErrors = async (sdk: BreezSdk, request: PrepareSendPaymentRequest) => {
  // ANCHOR: handle-errors
  try {
    const prepareResponse = await sdk.prepareSendPayment(request)
    console.log(`Payment prepared: ${prepareResponse.paymentMethod.tag}`)
  } catch (error) {
    if (SdkError.InsufficientFunds.instanceOf(error)) {
      console.log('Not enough funds for this payment')
    } else if (SdkError.CrossChainDisabled.instanceOf(error)) {
      console.log(`Cross-chain payments are not enabled, see ${error.inner.docsUrl}`)
    } else {
      console.log(`Failed to prepare the payment: ${String(error)}`)
    }
  }
  // ANCHOR_END: handle-errors
}
