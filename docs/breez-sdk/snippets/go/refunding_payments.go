package example

import (
	"errors"
	"fmt"
	"log"

	"github.com/breez/breez-sdk-spark-go/breez_sdk_spark"
)

func ListUnclaimedDeposits(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: list-unclaimed-deposits
	request := breez_sdk_spark.ListUnclaimedDepositsRequest{}
	response, err := sdk.ListUnclaimedDeposits(request)

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
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
					switch fee := (*claimErr.MaxFee).(type) {
					case breez_sdk_spark.FeeFixed:
						maxFeeStr = fmt.Sprintf("%v sats", fee.Amount)
					case breez_sdk_spark.FeeRate:
						maxFeeStr = fmt.Sprintf("%v sats/vByte", fee.SatPerVbyte)
					}
				}
				log.Printf("Max claim fee exceeded. Max: %v, Required: %v sats or %v sats/vByte", maxFeeStr, claimErr.RequiredFeeSats, claimErr.RequiredFeeRateSatPerVbyte)
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
			requiredFee := exceeded.RequiredFeeSats

			// Show UI to user with the required fee and get approval
			userApproved := true // Replace with actual user approval logic

			if userApproved {
				maxFee := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeFixed{Amount: requiredFee})
				claimRequest := breez_sdk_spark.ClaimDepositRequest{
					Txid:   deposit.Txid,
					Vout:   deposit.Vout,
					MaxFee: &maxFee,
				}
				_, err := sdk.ClaimDeposit(claimRequest)
				if err != nil {
					var sdkErr *breez_sdk_spark.SdkError
					if errors.As(err, &sdkErr) {
						// Handle SdkError - can inspect specific variants if needed
						// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
					}
					return err
				}
			}
		}
	}
	// ANCHOR_END: handle-fee-exceeded
	return nil
}

func RefundDeposit(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: refund-deposit
	txid := "<your_deposit_txid>"
	vout := uint32(0)
	destinationAddress := "bc1qexample..." // Your Bitcoin address

	// Set the fee for the refund transaction using the half-hour feerate
	recommendedFees, err := sdk.RecommendedFees()
	if err != nil {
		return err
	}
	fee := breez_sdk_spark.Fee(breez_sdk_spark.FeeRate{SatPerVbyte: recommendedFees.HalfHourFee})
	// or using a fixed amount
	//fee := breez_sdk_spark.Fee(breez_sdk_spark.FeeFixed{Amount: 500})
	//
	// Important: The total fee must be at least 194 sats to ensure the
	// transaction can be relayed by the Bitcoin network. If the fee is
	// lower, the refund request will be rejected.

	request := breez_sdk_spark.RefundDepositRequest{
		Txid:               txid,
		Vout:               vout,
		DestinationAddress: destinationAddress,
		Fee:                fee,
	}
	response, err := sdk.RefundDeposit(request)

	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return err
	}

	log.Print("Refund transaction created:")
	log.Printf("Transaction ID: %v", response.TxId)
	log.Printf("Transaction hex: %v", response.TxHex)
	// ANCHOR_END: refund-deposit
	return nil
}

func SetMaxFeeToRecommendedFees() error {
	// ANCHOR: set-max-fee-to-recommended-fees
	// Create the default config
	config := breez_sdk_spark.DefaultConfig(breez_sdk_spark.NetworkMainnet)
	apiKey := "<breez api key>"
	config.ApiKey = &apiKey

	// Set the maximum fee to the fastest network recommended fee at the time of claim
	// with a leeway of 1 sats/vbyte
	networkRecommendedInterface := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeNetworkRecommended{LeewaySatPerVbyte: 1})
	config.MaxDepositClaimFee = &networkRecommendedInterface
	// ANCHOR_END: set-max-fee-to-recommended-fees
	log.Printf("Config: %v", config)
	return nil
}

func CustomClaimLogic(sdk *breez_sdk_spark.BreezSdk, deposit breez_sdk_spark.DepositInfo) error {
	// ANCHOR: custom-claim-logic
	if claimErr := *deposit.ClaimError; claimErr != nil {
		if exceeded, ok := claimErr.(breez_sdk_spark.DepositClaimErrorMaxDepositClaimFeeExceeded); ok {
			requiredFeeRate := exceeded.RequiredFeeRateSatPerVbyte

			recommendedFees, err := sdk.RecommendedFees()
			if err != nil {
				return err
			}

			if requiredFeeRate <= recommendedFees.FastestFee {
				maxFee := breez_sdk_spark.MaxFee(breez_sdk_spark.MaxFeeRate{SatPerVbyte: requiredFeeRate})
				claimRequest := breez_sdk_spark.ClaimDepositRequest{
					Txid:   deposit.Txid,
					Vout:   deposit.Vout,
					MaxFee: &maxFee,
				}
				_, err := sdk.ClaimDeposit(claimRequest)
				if err != nil {
					var sdkErr *breez_sdk_spark.SdkError
					if errors.As(err, &sdkErr) {
						// Handle SdkError - can inspect specific variants if needed
						// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
					}
					return err
				}
			}
		}
	}
	// ANCHOR_END: custom-claim-logic
	return nil
}

func RecommendedFees(sdk *breez_sdk_spark.BreezSdk) error {
	// ANCHOR: recommended-fees
	response, err := sdk.RecommendedFees()
	if err != nil {
		var sdkErr *breez_sdk_spark.SdkError
		if errors.As(err, &sdkErr) {
			// Handle SdkError - can inspect specific variants if needed
			// e.g., switch on sdkErr variant for InsufficientFunds, NetworkError, etc.
		}
		return err
	}
	log.Printf("Fastest fee: %v sats/vByte", response.FastestFee)
	log.Printf("Half-hour fee: %v sats/vByte", response.HalfHourFee)
	log.Printf("Hour fee: %v sats/vByte", response.HourFee)
	log.Printf("Economy fee: %v sats/vByte", response.EconomyFee)
	log.Printf("Minimum fee: %v sats/vByte", response.MinimumFee)
	// ANCHOR_END: recommended-fees
	return nil
}
