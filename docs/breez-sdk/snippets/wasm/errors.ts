import {
  type BreezSdk,
  type PrepareSendPaymentRequest,
  type SdkError
} from '@breeztech/breez-sdk-spark'

const exampleHandleErrors = async (sdk: BreezSdk, request: PrepareSendPaymentRequest) => {
  // ANCHOR: handle-errors
  try {
    const prepareResponse = await sdk.prepareSendPayment(request)
    console.log(`Payment prepared: ${prepareResponse.paymentMethod.type}`)
  } catch (error) {
    // The SDK throws Error objects. Some also carry a docsUrl linking to the guide.
    const sdkError = error as SdkError
    console.log(`Failed to prepare the payment: ${sdkError.message}`)
    if (sdkError.docsUrl !== undefined) {
      console.log(`Learn more: ${sdkError.docsUrl}`)
    }
  }
  // ANCHOR_END: handle-errors
}
