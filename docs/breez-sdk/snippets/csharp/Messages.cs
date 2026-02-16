using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class Messages
    {
        async Task SignMessage(BreezClient client)
        {
            // ANCHOR: sign-message
            var message = "<message to sign>";
            // Set to true to get a compact signature rather than a DER
            var compact = true;
            var signMessageRequest = new SignMessageRequest(
                message: message,
                compact: compact
            );
            var signMessageResponse = await client.SignMessage(request: signMessageRequest);

            var signature = signMessageResponse.signature;
            var pubkey = signMessageResponse.pubkey;

            Console.WriteLine($"Pubkey: {pubkey}");
            Console.WriteLine($"Signature: {signature}");
            // ANCHOR_END: sign-message
        }

        async Task CheckMessage(BreezClient client)
        {
            // ANCHOR: check-message
            var message = "<message>";
            var pubkey = "<pubkey of signer>";
            var signature = "<message signature>";
            var checkMessageRequest = new CheckMessageRequest(
                message: message,
                pubkey: pubkey,
                signature: signature
            );
            var checkMessageResponse = await client.CheckMessage(request: checkMessageRequest);

            var isValid = checkMessageResponse.isValid;

            Console.WriteLine($"Signature valid: {isValid}");
            // ANCHOR_END: check-message
        }
    }
}
