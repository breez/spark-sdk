package example

import (
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func ListUnclaimedDeposits(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: list-unclaimed-deposits
	request := breez_sdk_spark.ListUnclaimedDepositsRequest{}
	response, err := sdk.ListUnclaimedDeposits(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	for _, deposit := range response.Deposits {
		log.Printf("Unclaimed Deposit: %v:%v", deposit.Txid, deposit.Vout)
		log.Printf("Amount: %v sats", deposit.AmountSats)

		if claimErr := *deposit.ClaimError; claimErr != nil {
			switch claimErr := claimErr.(type) {
			case breez_sdk_spark.DepositClaimErrorDepositClaimFeeExceeded:
				log.Printf("Max claim fee exceeded. Max: %v, Actual: %v sats", claimErr.MaxFee, claimErr.ActualFee)
			case breez_sdk_spark.DepositClaimErrorDepositClaimFeeNotSet:
				log.Printf("Max claim fee not set. Actual: %v sats", claimErr.ActualFee)
			case breez_sdk_spark.DepositClaimErrorMissingUtxo:
				log.Print("UTXO not found when claiming deposit")
			case breez_sdk_spark.DepositClaimErrorGeneric:
				log.Printf("Claim failed: %v", claimErr.Message)
			case nil:
				log.Printf("No claim error")
			}
		}
	}
	// ANCHOR_END: list-unclaimed-deposits
	return nil
}

func ClaimDeposit(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: claim-deposit
	txid := "<your_deposit_txid>"
	vout := uint32(0)

	request := breez_sdk_spark.ClaimDepositRequest{
		Txid: txid,
		Vout: vout,
	}
	response, err := sdk.ClaimDeposit(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return nil, err
	}

	payment := response.Payment
	// ANCHOR_END: claim-deposit
	return &payment, nil
}

func RefundDeposit(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: refund-deposit
	txid := "<your_deposit_txid>"
	vout := uint32(0)
	destinationAddress := "bc1qexample..." // Your Bitcoin address

	request := breez_sdk_spark.RefundDepositRequest{
		Txid:               txid,
		Vout:               vout,
		DestinationAddress: destinationAddress,
	}
	response, err := sdk.RefundDeposit(request)

	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}

	log.Print("Refund transaction created:")
	log.Printf("Transaction ID: %v", response.TxId)
	log.Printf("Transaction hex: %v", response.TxHex)
	// ANCHOR_END: refund-deposit
	return nil
}
