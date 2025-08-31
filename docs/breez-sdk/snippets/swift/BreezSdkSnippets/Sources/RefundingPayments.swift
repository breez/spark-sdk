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
            case .depositClaimFeeExceeded(let tx, let vout, let maxFee, let actualFee):
                print("Claim failed: Fee exceeded. Max: \(maxFee), Actual: \(actualFee)")
            case .missingUtxo(let tx, let vout):
                print("Claim failed: UTXO not found")
            case .generic(let message):
                print("Claim failed: \(message)")
            }
        }
    }
    // ANCHOR_END: list-unclaimed-deposits
}

func claimDeposit(sdk: BreezSdk) async throws {
    // ANCHOR: claim-deposit
    let txid = "your_deposit_txid"
    let vout: UInt32 = 0
    
    // Set a higher max fee to retry claiming
    let maxFee = Fee.absolute(feeSat: 5000) // 5000 sats
    
    let request = ClaimDepositRequest(
        txid: txid,
        vout: vout,
        maxFee: maxFee
    )
    
    let response = try await sdk.claimDeposit(request: request)
    print("Deposit claimed successfully. Payment: \(response.payment)")
    // ANCHOR_END: claim-deposit
}

func refundDeposit(sdk: BreezSdk) async throws {
    // ANCHOR: refund-deposit
    let txid = "your_deposit_txid"
    let vout: UInt32 = 0
    let destinationAddress = "bc1qexample..." // Your Bitcoin address
    // Set the fee for the refund transaction
    let fee = Fee.absolute(feeSat: 500) // 500 sats
    
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
