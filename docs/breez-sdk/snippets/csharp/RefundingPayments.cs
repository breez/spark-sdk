using Breez.Sdk.Spark;

namespace BreezSdkSnippets
{
    class RefundingPayments
    {
        async Task ListUnclaimedDeposits(BreezSdk sdk)
        {
            // ANCHOR: list-unclaimed-deposits
            var request = new ListUnclaimedDepositsRequest();
            var response = await sdk.ListUnclaimedDeposits(request: request);

            foreach (var deposit in response.deposits)
            {
                Console.WriteLine($"Unclaimed deposit: {deposit.txid}:{deposit.vout}");
                Console.WriteLine($"Amount: {deposit.amountSats} sats");

                if (deposit.claimError != null)
                {
                    if (deposit.claimError is DepositClaimError.MaxDepositClaimFeeExceeded exceeded)
                    {
                        var maxFeeStr = "none";
                        if (exceeded.maxFee != null)
                        {
                            if (exceeded.maxFee is Fee.Fixed fixedFee)
                            {
                                maxFeeStr = $"{fixedFee.amount} sats";
                            }
                            else if (exceeded.maxFee is Fee.Rate rateFee)
                            {
                                maxFeeStr = $"{rateFee.satPerVbyte} sats/vByte";
                            }
                        }
                        Console.WriteLine($"Claim failed: Fee exceeded. Max: {maxFeeStr}, " +
                                        $"Required: {exceeded.requiredFeeSats} sats or {exceeded.requiredFeeRateSatPerVbyte} sats/vByte");
                    }
                    else if (deposit.claimError is DepositClaimError.MissingUtxo)
                    {
                        Console.WriteLine("Claim failed: UTXO not found");
                    }
                    else if (deposit.claimError is DepositClaimError.Generic generic)
                    {
                        Console.WriteLine($"Claim failed: {generic.message}");
                    }
                }
            }
            // ANCHOR_END: list-unclaimed-deposits
        }

        async Task HandleFeeExceeded(BreezSdk sdk, DepositInfo deposit)
        {
            // ANCHOR: handle-fee-exceeded
            if (deposit.claimError is DepositClaimError.MaxDepositClaimFeeExceeded exceeded)
            {
                var requiredFee = exceeded.requiredFeeSats;

                // Show UI to user with the required fee and get approval
                var userApproved = true; // Replace with actual user approval logic

                if (userApproved)
                {
                    var claimRequest = new ClaimDepositRequest(
                        txid: deposit.txid,
                        vout: deposit.vout,
                        maxFee: new MaxFee.Fixed(amount: requiredFee)
                    );
                    await sdk.ClaimDeposit(request: claimRequest);
                }
            }
            // ANCHOR_END: handle-fee-exceeded
        }

        async Task RefundDeposit(BreezSdk sdk)
        {
            // ANCHOR: refund-deposit
            var txid = "your_deposit_txid";
            var vout = 0U;
            var destinationAddress = "bc1qexample...";  // Your Bitcoin address

            // Set the fee for the refund transaction using a rate
            var fee = new Fee.Rate(satPerVbyte: 5);
            // or using a fixed amount
            //var fee = new Fee.Fixed(amount: 500);

            var request = new RefundDepositRequest(
                txid: txid,
                vout: vout,
                destinationAddress: destinationAddress,
                fee: fee
            );

            var response = await sdk.RefundDeposit(request: request);
            Console.WriteLine("Refund transaction created:");
            Console.WriteLine($"Transaction ID: {response.txId}");
            Console.WriteLine($"Transaction hex: {response.txHex}");
            // ANCHOR_END: refund-deposit
        }

        void SetMaxFeeToRecommendedFees()
        {
            // ANCHOR: set-max-fee-to-recommended-fees
            // Create the default config
            var config = BreezSdkSparkMethods.DefaultConfig(Network.Mainnet) with
            {
                apiKey = "<breez api key>"
            };

            // Set the maximum fee to the fastest network recommended fee at the time of claim
            // with a leeway of 1 sats/vbyte
            config = config with { maxDepositClaimFee = new MaxFee.NetworkRecommended(leewaySatPerVbyte: 1) };
            // ANCHOR_END: set-max-fee-to-recommended-fees
            Console.WriteLine($"Config: {config}");
        }

        async Task RecommendedFees(BreezSdk sdk)
        {
            // ANCHOR: recommended-fees
            var response = await sdk.RecommendedFees();
            Console.WriteLine($"Fastest fee: {response.fastestFee} sats/vByte");
            Console.WriteLine($"Half-hour fee: {response.halfHourFee} sats/vByte");
            Console.WriteLine($"Hour fee: {response.hourFee} sats/vByte");
            Console.WriteLine($"Economy fee: {response.economyFee} sats/vByte");
            Console.WriteLine($"Minimum fee: {response.minimumFee} sats/vByte");
        }
        // ANCHOR_END: recommended-fees
    }
}
