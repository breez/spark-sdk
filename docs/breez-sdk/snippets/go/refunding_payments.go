package example

import (
	"fmt"
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
			case breez_sdk_spark.DepositClaimErrorMaxDepositClaimFeeExceeded:
				maxFeeStr := "none"
				if claimErr.MaxFee != nil {
					maxFeeStr = fmt.Sprintf("%v sats", *claimErr.MaxFee)
				}
				log.Printf("Max claim fee exceeded. Max: %v, Required: %v sats", maxFeeStr, claimErr.RequiredFee)
			case breez_sdk_spark.DepositClaimErrorMissingUtxo:
				log.Print("UTXO not found when claiming deposit")
			case breez_sdk_spark.DepositClaimErrorGeneric:
				log.Printf("Claim failed: %v", claimErr.Message)
			}
		}
	}
	// ANCHOR_END: list-unclaimed-deposits
	return nil
}

func HandleFeeExceeded(sdk *breez_sdk_spark.BreezSdk, deposit breez_sdk_spark.DepositInfo) error {
	// ANCHOR: handle-fee-exceeded
	if claimErr := *deposit.ClaimError; claimErr != nil {
		if exceeded, ok := claimErr.(breez_sdk_spark.DepositClaimErrorMaxDepositClaimFeeExceeded); ok {
			requiredFee := exceeded.RequiredFee

			// Show UI to user with the required fee and get approval
			userApproved := true // Replace with actual user approval logic

			if userApproved {
				maxFee := breez_sdk_spark.Fee(breez_sdk_spark.FeeFixed{Amount: requiredFee})
				claimRequest := breez_sdk_spark.ClaimDepositRequest{
					Txid:   deposit.Txid,
					Vout:   deposit.Vout,
					MaxFee: &maxFee,
				}
				_, err := sdk.ClaimDeposit(claimRequest)
				if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
					return err
				}
			}
		}
	}
	// ANCHOR_END: handle-fee-exceeded
	return nil
}

func ClaimDeposit(sdk *breez_sdk_spark.BreezSdk) (*breez_sdk_spark.Payment, error) {
	// ANCHOR: claim-deposit
	txid := "<your_deposit_txid>"
	vout := uint32(0)

	// Set a higher max fee to retry claiming
	maxFee := breez_sdk_spark.Fee(breez_sdk_spark.FeeFixed{Amount: 5000})

	request := breez_sdk_spark.ClaimDepositRequest{
		Txid:   txid,
		Vout:   vout,
		MaxFee: &maxFee,
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

	// Set the fee for the refund transaction using a rate
	fee := breez_sdk_spark.Fee(breez_sdk_spark.FeeRate{SatPerVbyte: 5})
	// or using a fixed amount
	//fee := breez_sdk_spark.Fee(breez_sdk_spark.FeeFixed{Amount: 500})

	request := breez_sdk_spark.RefundDepositRequest{
		Txid:               txid,
		Vout:               vout,
		DestinationAddress: destinationAddress,
		Fee:                fee,
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

func RecommendedFees(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: recommended-fees
	response, err := sdk.RecommendedFees()
	if sdkErr := err.(*breez_sdk_spark.SdkError); sdkErr != nil {
		return err
	}
	log.Printf("Fastest fee: %v sats", response.FastestFee)
	log.Printf("Half-hour fee: %v sats", response.HalfHourFee)
	log.Printf("Hour fee: %v sats", response.HourFee)
	log.Printf("Economy fee: %v sats", response.EconomyFee)
	log.Printf("Minimum fee: %v sats", response.MinimumFee)
	// ANCHOR_END: recommended-fees
	return nil
}
