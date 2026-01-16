using Breez.Sdk.Spark;

namespace BreezSdkSnippets;

public static class LnurlAuth
{
    public static async Task ParseLnurlAuth(BreezSdk sdk)
    {
        // ANCHOR: parse-lnurl-auth
        // LNURL-auth URL from a service
        // Can be in the form:
        // - lnurl1... (bech32 encoded)
        // - https://service.com/lnurl-auth?tag=login&k1=...
        var lnurlAuthUrl = "lnurl1...";

        var inputType = await sdk.Parse(lnurlAuthUrl);
        if (inputType is InputType.LnurlAuth lnurlAuth)
        {
            var requestData = lnurlAuth.v1;
            Console.WriteLine($"Domain: {requestData.domain}");
            Console.WriteLine($"Action: {requestData.action}");

            // Show domain to user and ask for confirmation
            // This is important for security
        }
        // ANCHOR_END: parse-lnurl-auth
    }

    public static async Task Authenticate(BreezSdk sdk, LnurlAuthRequestDetails requestData)
    {
        // ANCHOR: lnurl-auth
        // Perform LNURL authentication
        var result = await sdk.LnurlAuth(requestData);

        if (result is LnurlCallbackStatus.Ok)
        {
            Console.WriteLine("Authentication successful");
        }
        else if (result is LnurlCallbackStatus.ErrorStatus errorStatus)
        {
            Console.WriteLine($"Authentication failed: {errorStatus.errorDetails.reason}");
        }
        // ANCHOR_END: lnurl-auth
    }
}
