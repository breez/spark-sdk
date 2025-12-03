import BreezSdkSpark

func listUnclaimedDeposits(sdk: BreezSdk) async throws {
    // ANCHOR: list-unclaimed-deposits
    let request = ListUnclaimedDepositsRequest()
    let response = try await sdk.listUnclaimedDeposits(request: request)

    for deposit in response.deposits {
        print("Unclaimed deposit: \(deposit.txid):\(deposit.vout)")
        print("Amount: \(deposit.amountSats) sats")

        if let claimError = deposit.claimError {
            switch claimError {
            case .maxDepositClaimFeeExceeded(
                let tx, let vout, let maxFee, let requiredFeeSats, let requiredFeeRateSatPerVbyte):
                let maxFeeStr: String
                if let maxFee = maxFee {
                    switch maxFee {
                    case .fixed(let amount):
                        maxFeeStr = "\(amount) sats"
                    case .rate(let satPerVbyte):
                        maxFeeStr = "\(satPerVbyte) sats/vByte"
                    }
                } else {
                    maxFeeStr = "none"
                }
                print(
                    "Max claim fee exceeded. Max: \(maxFeeStr), Required: \(requiredFeeSats) sats or \(requiredFeeRateSatPerVbyte) sats/vByte"
                )
            case .missingUtxo(let tx, let vout):
                print("UTXO not found when claiming deposit")
            case .generic(let message):
                print("Claim failed: \(message)")
            }
        }
    }
    // ANCHOR_END: list-unclaimed-deposits
}

func handleFeeExceeded(sdk: BreezSdk, deposit: DepositInfo) async throws {
    // ANCHOR: handle-fee-exceeded
    if case .maxDepositClaimFeeExceeded(_, _, _, let requiredFeeSats, _) = deposit.claimError {
        // Show UI to user with the required fee and get approval
        let userApproved = true  // Replace with actual user approval logic

        if userApproved {
            let claimRequest = ClaimDepositRequest(
                txid: deposit.txid,
                vout: deposit.vout,
                maxFee: MaxFee.fixed(amount: requiredFeeSats)
            )
            try await sdk.claimDeposit(request: claimRequest)
        }
    }
    // ANCHOR_END: handle-fee-exceeded
}

func refundDeposit(sdk: BreezSdk) async throws {
    // ANCHOR: refund-deposit
    let txid = "your_deposit_txid"
    let vout: UInt32 = 0
    let destinationAddress = "bc1qexample..."  // Your Bitcoin address

    // Set the fee for the refund transaction using a rate
    let fee = Fee.rate(satPerVbyte: 5)  // 5 sats per vbyte
    // or using a fixed amount
    //let fee = Fee.fixed(amount: 500) // 500 sats

    let request = RefundDepositRequest(
        txid: txid,
        vout: vout,
        destinationAddress: destinationAddress,
        fee: fee
    )

    let response = try await sdk.refundDeposit(request: request)
    print("Refund transaction created:")
    print("Transaction ID: \(response.txId)")
    print("Transaction hex: \(response.txHex)")
    // ANCHOR_END: refund-deposit
}

func setMaxFeeToRecommendedFees() async throws {
    // ANCHOR: set-max-fee-to-recommended-fees
    // Create the default config
    var config = defaultConfig(network: Network.mainnet)
    config.apiKey = "<breez api key>"

    // Set the maximum fee to the fastest network recommended fee at the time of claim
    // with a leeway of 1 sats/vbyte
    config.maxDepositClaimFee = MaxFee.networkRecommended(leewaySatPerVbyte: 1)
    // ANCHOR_END: set-max-fee-to-recommended-fees
    print("Config: \(config)")
}

func recommendedFees(sdk: BreezSdk) async throws {
    // ANCHOR: recommended-fees
    let response = try await sdk.recommendedFees()
    print("Fastest fee: \(response.fastestFee) sats/vByte")
    print("Half-hour fee: \(response.halfHourFee) sats/vByte")
    print("Hour fee: \(response.hourFee) sats/vByte")
    print("Economy fee: \(response.economyFee) sats/vByte")
    print("Minimum fee: \(response.minimumFee) sats/vByte")
    // ANCHOR_END: recommended-fees
}
