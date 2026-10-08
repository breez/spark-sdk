using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class Errors
    {
        async Task HandleErrors(BreezSdk sdk, PrepareSendPaymentRequest request)
        {
            // ANCHOR: handle-errors
            try
            {
                var prepareResponse = await sdk.PrepareSendPayment(request: request);
                Console.WriteLine($"Payment prepared: {prepareResponse.paymentMethod}");
            }
            catch (SdkException.InsufficientFunds)
            {
                Console.WriteLine("Not enough funds for this payment");
            }
            catch (SdkException.CrossChainDisabled e)
            {
                Console.WriteLine($"Cross-chain payments are not enabled, see {e.docsUrl}");
            }
            catch (SdkException e)
            {
                Console.WriteLine($"Failed to prepare the payment: {e.Message}");
            }
            // ANCHOR_END: handle-errors
        }
    }
}
