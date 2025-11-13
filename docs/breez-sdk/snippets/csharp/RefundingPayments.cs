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
                    if (deposit.claimError is DepositClaimError.DepositClaimFeeExceeded exceeded)
                    {
                        Console.WriteLine($"Claim failed: Fee exceeded. Max: {exceeded.maxFee}, " +
                                        $"Actual: {exceeded.actualFee}");
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

        async Task ClaimDeposit(BreezSdk sdk)
        {
            // ANCHOR: claim-deposit
            var txid = "your_deposit_txid";
            var vout = 0U;

            // Set a higher max fee to retry claiming
            var maxFee = new Fee.Fixed(amount: 5_000);

            var request = new ClaimDepositRequest(txid: txid, vout: vout, maxFee: maxFee);

            var response = await sdk.ClaimDeposit(request: request);
            Console.WriteLine($"Deposit claimed successfully. Payment: {response.payment}");
            // ANCHOR_END: claim-deposit
        }

        async Task RefundDeposit(BreezSdk sdk)
        {
            // ANCHOR: refund-deposit
            var txid = "your_deposit_txid";
            var vout = 0U;
            var destinationAddress = "bc1qexample...";  // Your Bitcoin address

            // Set the fee for the refund transaction
            var fee = new Fee.Fixed(amount: 500);

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
    }
}
